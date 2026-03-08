use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    #[serde(rename = "type")]
    pub tool_type: String,
    pub description: String,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    pub parameters: HashMap<String, String>,
    #[serde(default)]
    pub body_template: Option<serde_json::Value>,
    #[serde(default)]
    pub prompt_template: Option<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
}

/// Result of executing a tool
pub struct ToolResult {
    pub tool_name: String,
    pub success: bool,
    pub output: String,
}

/// Build a description of all available tools for the LLM system prompt
pub fn tools_prompt(tools: &[ToolDef]) -> String {
    if tools.is_empty() {
        return String::new();
    }

    let mut prompt = String::from(
        "\n\nYou have access to the following tools. When you decide to use a tool, \
         respond with EXACTLY this format on a single line:\n\
         TOOL: tool_name | param1=value1 | param2=value2\n\n\
         IMPORTANT: Parameter values must NOT contain the \" | \" sequence.\n\n\
         Available tools:\n",
    );

    for tool in tools {
        prompt.push_str(&format!("\n- **{}** (type: {}): {}\n", tool.name, tool.tool_type, tool.description));
        prompt.push_str("  Parameters:\n");
        for (name, desc) in &tool.parameters {
            prompt.push_str(&format!("    - {}: {}\n", name, desc));
        }
    }

    prompt.push_str(
        "\nIf no tool is needed, respond normally. \
         Only use a tool when the user's request clearly matches a tool's purpose.\n",
    );

    prompt
}

/// Parse a TOOL: response from the LLM
/// Format: "TOOL: tool_name | param1=value1 | param2=value2"
pub fn parse_tool_call(response: &str) -> Option<(String, HashMap<String, String>)> {
    let trimmed = response.trim();

    // Check if any line starts with "TOOL:"
    for line in trimmed.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("TOOL:") {
            // Split on " | " (with spaces) to avoid breaking values containing bare "|"
            let parts: Vec<&str> = rest.split(" | ").map(|s| s.trim()).collect();
            if parts.is_empty() {
                return None;
            }

            let tool_name = parts[0].trim().to_string();
            let mut params = HashMap::new();

            for part in &parts[1..] {
                if let Some((key, value)) = part.split_once('=') {
                    params.insert(key.trim().to_string(), value.trim().to_string());
                }
            }

            return Some((tool_name, params));
        }
    }

    None
}

/// Substitute {param} placeholders in a JSON value
fn substitute_json(value: &serde_json::Value, params: &HashMap<String, String>) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => {
            let mut result = s.clone();
            for (key, val) in params {
                result = result.replace(&format!("{{{}}}", key), val);
            }
            serde_json::Value::String(result)
        }
        serde_json::Value::Object(map) => {
            let new_map: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), substitute_json(v, params)))
                .collect();
            serde_json::Value::Object(new_map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(|v| substitute_json(v, params)).collect())
        }
        other => other.clone(),
    }
}

/// Substitute {param} placeholders in a string
fn substitute_string(template: &str, params: &HashMap<String, String>) -> String {
    let mut result = template.to_string();
    for (key, val) in params {
        result = result.replace(&format!("{{{}}}", key), val);
    }
    result
}

/// Resolve environment variable references in a string.
/// Supports `${ENV_VAR_NAME}` syntax anywhere in the value.
fn resolve_env_vars(value: &str) -> String {
    let mut result = value.to_string();
    // Find and replace all ${VAR_NAME} patterns
    while let Some(start) = result.find("${") {
        if let Some(end) = result[start..].find('}') {
            let var_name = &result[start + 2..start + end];
            let var_value = std::env::var(var_name).unwrap_or_default();
            result = format!("{}{}{}", &result[..start], var_value, &result[start + end + 1..]);
        } else {
            break;
        }
    }
    result
}

/// Truncate a string at a safe UTF-8 boundary
fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Check if a URL targets a private/internal network address (SSRF protection)
fn is_private_url(url_str: &str) -> bool {
    let parsed = match url::Url::parse(url_str) {
        Ok(u) => u,
        Err(_) => return true, // Block unparseable URLs
    };

    let host = match parsed.host_str() {
        Some(h) => h,
        None => return true,
    };

    // Block localhost variants
    if host == "localhost" || host == "127.0.0.1" || host == "::1" || host == "0.0.0.0" {
        return true;
    }

    // Block private IP ranges
    if let Ok(ip) = host.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => {
                v4.is_private()           // 10.x, 172.16-31.x, 192.168.x
                || v4.is_loopback()       // 127.x
                || v4.is_link_local()     // 169.254.x (cloud metadata)
                || v4.is_unspecified()    // 0.0.0.0
            }
            IpAddr::V6(v6) => {
                v6.is_loopback() || v6.is_unspecified()
            }
        };
    }

    // Block metadata endpoints
    if host == "metadata.google.internal" || host.ends_with(".internal") {
        return true;
    }

    false
}

/// Execute a tool and return the result
pub async fn execute_tool(
    tool: &ToolDef,
    params: &HashMap<String, String>,
    client: &Client,
) -> ToolResult {
    match tool.tool_type.as_str() {
        "curl" => execute_curl(tool, params, client).await,
        "claude-code" => execute_claude_code(tool, params).await,
        _ => ToolResult {
            tool_name: tool.name.clone(),
            success: false,
            output: format!("Unknown tool type: {}", tool.tool_type),
        },
    }
}

async fn execute_curl(
    tool: &ToolDef,
    params: &HashMap<String, String>,
    client: &Client,
) -> ToolResult {
    let url = match &tool.url {
        Some(u) => substitute_string(u, params),
        None => {
            return ToolResult {
                tool_name: tool.name.clone(),
                success: false,
                output: "Tool missing 'url' field".to_string(),
            };
        }
    };

    // SSRF protection: block requests to private/internal addresses
    if is_private_url(&url) {
        return ToolResult {
            tool_name: tool.name.clone(),
            success: false,
            output: format!("Blocked request to private/internal URL: {}", url),
        };
    }

    let method = tool.method.as_deref().unwrap_or("GET");

    tracing::info!("[tools] executing curl: {} {}", method, url);

    let mut req = match method.to_uppercase().as_str() {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "PATCH" => client.patch(&url),
        "DELETE" => client.delete(&url),
        _ => {
            return ToolResult {
                tool_name: tool.name.clone(),
                success: false,
                output: format!("Unsupported HTTP method: {}", method),
            };
        }
    };

    // Add headers (with env var resolution)
    if let Some(headers) = &tool.headers {
        for (key, value) in headers {
            let substituted = substitute_string(value, params);
            let resolved = resolve_env_vars(&substituted);
            req = req.header(key, resolved);
        }
    }

    // Add body from template
    if let Some(body_template) = &tool.body_template {
        let body = substitute_json(body_template, params);
        req = req.json(&body);
    }

    // 30-second timeout for HTTP requests
    match tokio::time::timeout(
        std::time::Duration::from_secs(30),
        req.send(),
    )
    .await
    {
        Ok(Ok(response)) => {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let truncated = if body.len() > 2000 {
                format!("{}... (truncated)", truncate_utf8(&body, 2000))
            } else {
                body
            };
            ToolResult {
                tool_name: tool.name.clone(),
                success: status.is_success(),
                output: format!("HTTP {} — {}", status, truncated),
            }
        }
        Ok(Err(e)) => ToolResult {
            tool_name: tool.name.clone(),
            success: false,
            output: format!("HTTP request failed: {}", e),
        },
        Err(_) => ToolResult {
            tool_name: tool.name.clone(),
            success: false,
            output: "HTTP request timed out (30s limit)".to_string(),
        },
    }
}

async fn execute_claude_code(
    tool: &ToolDef,
    params: &HashMap<String, String>,
) -> ToolResult {
    let prompt_template = match &tool.prompt_template {
        Some(t) => t,
        None => {
            return ToolResult {
                tool_name: tool.name.clone(),
                success: false,
                output: "Tool missing 'prompt_template' field".to_string(),
            };
        }
    };

    let prompt = substitute_string(prompt_template, params);

    tracing::info!("[tools] executing claude-code: {}", prompt);

    let mut cmd = tokio::process::Command::new("claude");
    cmd.arg("-p").arg(&prompt);
    cmd.arg("--output-format").arg("text");

    // Only use working_directory from the static tool config, never from LLM-provided params
    if let Some(dir) = &tool.working_directory {
        // Resolve env vars but NOT LLM params — prevents path traversal
        let dir = resolve_env_vars(dir);
        let path = std::path::Path::new(&dir);
        if path.is_absolute() && path.exists() && path.is_dir() {
            cmd.current_dir(&dir);
        } else {
            tracing::warn!("[tools] invalid working_directory, ignoring: {}", dir);
        }
    }

    match tokio::time::timeout(
        std::time::Duration::from_secs(300), // 5 minute timeout
        cmd.output(),
    )
    .await
    {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = if stderr.is_empty() {
                stdout.to_string()
            } else {
                format!("{}\n{}", stdout, stderr)
            };
            let truncated = if combined.len() > 3000 {
                format!("{}... (truncated)", truncate_utf8(&combined, 3000))
            } else {
                combined
            };
            ToolResult {
                tool_name: tool.name.clone(),
                success: output.status.success(),
                output: truncated,
            }
        }
        Ok(Err(e)) => ToolResult {
            tool_name: tool.name.clone(),
            success: false,
            output: format!("Failed to run claude: {}. Is claude CLI installed?", e),
        },
        Err(_) => ToolResult {
            tool_name: tool.name.clone(),
            success: false,
            output: "Claude Code execution timed out (5 min limit)".to_string(),
        },
    }
}
