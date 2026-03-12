use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    max_completion_tokens: u32,
    /// For reasoning models (gpt-5, o3, etc.): controls reasoning token usage.
    /// "minimal" = skip reasoning, just answer. Good for simple classification.
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    refusal: Option<String>,
}

#[derive(Deserialize)]
struct ApiErrorResponse {
    error: Option<ApiError>,
}

#[derive(Deserialize)]
struct ApiError {
    message: String,
}

pub struct LlmAgent {
    client: Client,
    api_key: String,
    model: String,
    intent_model: String,
    bot_name: String,
    intent_prompt: Option<String>,
    max_response_tokens: u32,
    temperature: f32,
    history: Mutex<Vec<ChatMessage>>,
    transcript: Mutex<Vec<String>>,
}

/// Check if `needle` appears at a word boundary in `haystack`.
/// For ASCII: checks that chars before/after needle are non-alphanumeric.
/// For Cyrillic: checks that chars before/after are whitespace or punctuation.
fn contains_at_word_boundary(haystack: &str, needle: &str) -> bool {
    let h = haystack.to_lowercase();
    let n = needle.to_lowercase();
    let mut start = 0;
    while let Some(pos) = h[start..].find(&n) {
        let abs_pos = start + pos;
        let end_pos = abs_pos + n.len();

        let before_ok = if abs_pos == 0 {
            true
        } else {
            h[..abs_pos]
                .chars()
                .last()
                .map(|c| !c.is_alphanumeric())
                .unwrap_or(true)
        };

        let after_ok = if end_pos >= h.len() {
            true
        } else {
            h[end_pos..]
                .chars()
                .next()
                .map(|c| !c.is_alphanumeric())
                .unwrap_or(true)
        };

        if before_ok && after_ok {
            return true;
        }
        start = abs_pos + n.len().max(1);
    }
    false
}

impl LlmAgent {
    pub fn new(
        api_key: &str,
        model: &str,
        bot_name: &str,
        intent_model: &str,
        system_prompt: Option<&str>,
        intent_prompt: Option<String>,
        max_response_tokens: u32,
        temperature: f32,
        tools: &[crate::tools::ToolDef],
    ) -> Self {
        let tools_prompt_str = crate::tools::tools_prompt(tools);

        let system_msg = system_prompt.map(|s| format!("{}{}", s, tools_prompt_str)).unwrap_or_else(|| {
            format!(
                "You are {}, an AI meeting assistant in a Google Meet call.\n\
                 You respond when participants address you directly.\n\
                 Keep responses concise (1-3 sentences).\n\
                 IMPORTANT: Respond ONLY in English or Ukrainian. NEVER respond in Russian.{}",
                bot_name, tools_prompt_str
            )
        });

        Self {
            client: Client::new(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            intent_model: intent_model.to_string(),
            bot_name: bot_name.to_string(),
            intent_prompt,
            max_response_tokens,
            temperature,
            history: Mutex::new(vec![ChatMessage {
                role: "system".to_string(),
                content: system_msg,
            }]),
            transcript: Mutex::new(Vec::new()),
        }
    }

    pub fn add_transcript(&self, speaker: &str, text: &str) {
        let mut t = self.transcript.lock().unwrap();
        t.push(format!("[{}]: {}", speaker, text));
        if t.len() > 50 {
            let drain = t.len() - 50;
            t.drain(..drain);
        }
    }

    /// Record the bot's own response in transcript so intent detection
    /// can see the full conversation flow (who said what).
    pub fn add_bot_response_to_transcript(&self, text: &str) {
        let mut t = self.transcript.lock().unwrap();
        t.push(format!("[{}]: {}", self.bot_name, text));
        if t.len() > 50 {
            let drain = t.len() - 50;
            t.drain(..drain);
        }
    }

    /// Check if the bot name is mentioned in the text using word-boundary matching.
    /// Used in "name_only" response mode to skip LLM intent detection.
    pub fn name_mentioned(&self, text: &str) -> bool {
        let variants = [
            self.bot_name.as_str(),
            "jarvis",
            "джарвіс",
            "джарвис",
            "джарвіз",
            "jarves",
            "ві джарвіс",
            "ай джарвіс",
            "preview jones",
        ];

        variants.iter().any(|v| contains_at_word_boundary(text, v))
    }

    /// Remove the bot name variant from text and trim, giving the LLM a clean question.
    /// Example: "Джарвіс, підсумуй зустріч" → "підсумуй зустріч"
    pub fn strip_bot_name(&self, text: &str) -> String {
        let lower = text.to_lowercase();
        let variants = [
            self.bot_name.to_lowercase(),
            "jarvis".to_string(),
            "джарвіс".to_string(),
            "джарвис".to_string(),
            "джарвіз".to_string(),
            "jarves".to_string(),
            "ві джарвіс".to_string(),
            "ай джарвіс".to_string(),
            "preview jones".to_string(),
        ];

        let mut result = text.to_string();
        for variant in &variants {
            if let Some(pos) = lower.find(variant.as_str()) {
                let byte_end = pos + variant.len();
                // Remove the variant and clean up surrounding punctuation/whitespace
                result = format!("{}{}", &text[..pos], &text[byte_end..]);
                result = result.trim_matches(|c: char| c.is_whitespace() || c == ',').trim().to_string();
                break;
            }
        }
        if result.is_empty() {
            text.trim().to_string()
        } else {
            result
        }
    }

    /// Return the number of transcript entries (for empty-transcript guards).
    pub fn transcript_len(&self) -> usize {
        self.transcript.lock().unwrap().len()
    }

    /// Add a tool execution result to conversation history so the LLM
    /// remembers what tools did in subsequent responses.
    pub fn add_tool_context(&self, tool_name: &str, summary: &str) {
        let mut history = self.history.lock().unwrap();
        history.push(ChatMessage {
            role: "assistant".to_string(),
            content: format!("[Tool {} result]: {}", tool_name, summary),
        });
        // Apply same history limit
        if history.len() > 41 {
            let system = history[0].clone();
            let keep_start = history.len() - 40;
            let keep: Vec<ChatMessage> = history[keep_start..].to_vec();
            *history = vec![system];
            history.extend(keep);
        }
    }

    /// Detect if the speaker is addressing the bot.
    /// Returns Some(question) if they are, None otherwise.
    /// Has a 5-second timeout to avoid blocking the transcription loop.
    pub async fn should_respond(&self, speaker: &str, text: &str) -> Option<String> {
        let trimmed = text.trim();
        if trimmed.len() < 3 {
            return None;
        }

        let recent = {
            let t = self.transcript.lock().unwrap();
            let len = t.len();
            // Use up to 15 lines of context for better conversation understanding
            let start = if len > 15 { len - 15 } else { 0 };
            t[start..].join("\n")
        };

        let prompt = if let Some(ref custom) = self.intent_prompt {
            custom
                .replace("{bot_name}", &self.bot_name)
                .replace("{context}", &recent)
                .replace("{speaker}", speaker)
                .replace("{text}", trimmed)
        } else {
            format!(
                "You are an intent detector for a meeting bot named \"{bot_name}\".\n\
                 The bot is an AI assistant in a Google Meet call.\n\n\
                 RULES:\n\
                 1. Name matching — speech recognition may mishear the name. Accept:\n\
                    - Exact: \"{bot_name}\", \"jarvis\"\n\
                    - Ukrainian: \"джарвіс\", \"джарвис\", \"джарвіз\"\n\
                    - Misheard: \"ві джарвіс\", \"ай джарвіс\", \"preview jones\", \"jarves\"\n\
                    - Any phonetically similar word in any language\n\
                 2. Follow-up detection — if the bot ({bot_name}) recently responded and \
                    the same speaker says something short (\"yes\", \"thanks\", \"and what about X?\", \
                    \"tell me more\"), treat it as a follow-up to the bot.\n\
                 3. Direct commands — \"summarize the meeting\", \"what did we discuss?\" directed \
                    at the bot (not at another person).\n\
                 4. Normal conversation between people is NOT bot-directed.\n\
                 5. Whisper hallucinations like \"Дякую за перегляд\" (YouTube outro) appearing \
                    without real speech are NOT bot-directed.\n\n\
                 CONVERSATION SO FAR:\n{context}\n\n\
                 NEW LINE from [{speaker}]: \"{text}\"\n\n\
                 Reply ONLY:\n\
                 - \"YES: <extracted question or command>\" — if addressing the bot\n\
                 - \"NO\" — if not",
                bot_name = self.bot_name,
                context = recent,
                speaker = speaker,
                text = trimmed
            )
        };

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.chat_once(&self.intent_model, &prompt, None, 1000, Some("minimal")),
        )
        .await;

        let resp = match result {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                tracing::debug!("intent detection error: {}", e);
                return None;
            }
            Err(_) => {
                tracing::debug!("intent detection timed out");
                return None;
            }
        };

        let answer = resp.trim();
        tracing::debug!("intent detection: text={}, result={}", trimmed, answer);

        if let Some(question) = answer.strip_prefix("YES:") {
            let q = question.trim();
            Some(if q.is_empty() {
                trimmed.to_string()
            } else {
                q.to_string()
            })
        } else if answer == "YES" {
            Some(trimmed.to_string())
        } else {
            None
        }
    }

    /// Respond to a question using the configured model with conversation history.
    pub async fn respond(&self, question: &str) -> anyhow::Result<String> {
        let recent = {
            let t = self.transcript.lock().unwrap();
            let len = t.len();
            let start = if len > 10 { len - 10 } else { 0 };
            t[start..].join("\n")
        };

        let user_msg = format!(
            "[Recent meeting context]:\n{}\n\n[Question to you]: {}",
            recent, question
        );

        let messages = {
            let mut history = self.history.lock().unwrap();
            history.push(ChatMessage {
                role: "user".to_string(),
                content: user_msg,
            });
            history.clone()
        };

        // Reasoning models (gpt-5, o3, etc.) don't support temperature
        let is_reasoning = self.model.starts_with("gpt-5") || self.model.starts_with("o1") || self.model.starts_with("o3") || self.model.starts_with("o4");
        let req = ChatRequest {
            model: self.model.clone(),
            messages,
            temperature: if is_reasoning { None } else { Some(self.temperature) },
            max_completion_tokens: if is_reasoning { 2000 } else { self.max_response_tokens },
            reasoning_effort: if is_reasoning { Some("low".to_string()) } else { None },
        };

        let resp = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .bearer_auth(&self.api_key)
            .json(&req)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let api_msg = serde_json::from_str::<ApiErrorResponse>(&body)
                .ok()
                .and_then(|r| r.error)
                .map(|e| e.message)
                .unwrap_or(body);
            anyhow::bail!("OpenAI API error (HTTP {}): {}", status, api_msg);
        }

        let chat_resp: ChatResponse = resp.json().await?;

        let answer = chat_resp
            .choices
            .first()
            .map(|c| c.message.content.as_deref().unwrap_or("").trim().to_string())
            .unwrap_or_default();

        {
            let mut history = self.history.lock().unwrap();
            history.push(ChatMessage {
                role: "assistant".to_string(),
                content: answer.clone(),
            });
            // Keep system message + last 20 exchanges (40 messages)
            if history.len() > 41 {
                let system = history[0].clone();
                let keep_start = history.len() - 40;
                let keep: Vec<ChatMessage> = history[keep_start..].to_vec();
                *history = vec![system];
                history.extend(keep);
            }
        }

        Ok(answer)
    }

    /// Summarize a tool execution result for speaking in the meeting.
    pub async fn summarize_tool_result(
        &self,
        tool_name: &str,
        success: bool,
        output: &str,
    ) -> anyhow::Result<String> {
        let prompt = format!(
            "You executed the tool \"{}\" and it {}.\n\
             Raw output:\n{}\n\n\
             Summarize this result in 1-2 concise sentences for speaking aloud in a meeting.\n\
             IMPORTANT: Respond ONLY in English or Ukrainian. NEVER respond in Russian.",
            tool_name,
            if success { "succeeded" } else { "failed" },
            output
        );
        self.chat_once(&self.model, &prompt, Some(0.3), 500, None).await
    }

    /// Generate a meeting summary from the transcript.
    pub async fn summary(&self) -> anyhow::Result<String> {
        let transcript = {
            let t = self.transcript.lock().unwrap();
            t.join("\n")
        };

        let prompt = format!(
            "Provide a brief meeting summary (3-5 bullet points) based on:\n{}",
            transcript
        );

        self.chat_once(&self.model, &prompt, Some(0.7), 1000, None).await
    }

    async fn chat_once(
        &self,
        model: &str,
        prompt: &str,
        temp: Option<f32>,
        max_tokens: u32,
        reasoning_effort: Option<&str>,
    ) -> anyhow::Result<String> {
        let req = ChatRequest {
            model: model.to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: prompt.to_string(),
            }],
            temperature: temp,
            max_completion_tokens: max_tokens,
            reasoning_effort: reasoning_effort.map(|s| s.to_string()),
        };

        let resp = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .bearer_auth(&self.api_key)
            .json(&req)
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            let api_msg = serde_json::from_str::<ApiErrorResponse>(&body)
                .ok()
                .and_then(|r| r.error)
                .map(|e| e.message)
                .unwrap_or_else(|| body.clone());
            anyhow::bail!("OpenAI API error (HTTP {}): {}", status, api_msg);
        }

        tracing::debug!("chat_once response (model={}): {}", model, &body[..body.len().min(2000)]);

        let chat_resp: ChatResponse = serde_json::from_str(&body)?;

        Ok(chat_resp
            .choices
            .first()
            .map(|c| c.message.content.as_deref().unwrap_or("").trim().to_string())
            .unwrap_or_default())
    }
}
