use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Serialize, Deserialize, Clone)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChatMessage,
}

pub struct LlmAgent {
    client: Client,
    api_key: String,
    model: String,
    bot_name: String,
    trigger_phrase: String,
    history: Mutex<Vec<ChatMessage>>,
    transcript: Mutex<Vec<String>>,
}

impl LlmAgent {
    pub fn new(api_key: &str, model: &str, bot_name: &str, trigger_phrase: &str) -> Self {
        let system_msg = format!(
            "You are {}, an AI meeting assistant in a Google Meet call.\n\
             You respond when participants address you directly.\n\
             Keep responses concise (1-3 sentences).\n\
             IMPORTANT: Respond ONLY in English or Ukrainian. NEVER respond in Russian.",
            bot_name
        );

        Self {
            client: Client::new(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            bot_name: bot_name.to_string(),
            trigger_phrase: trigger_phrase.to_string(),
            history: Mutex::new(vec![ChatMessage {
                role: "system".to_string(),
                content: system_msg,
            }]),
            transcript: Mutex::new(Vec::new()),
        }
    }

    pub fn add_transcript(&self, text: &str) {
        let mut t = self.transcript.lock().unwrap();
        t.push(text.to_string());
        if t.len() > 50 {
            let drain = t.len() - 50;
            t.drain(..drain);
        }
    }

    /// Uses GPT-4o-mini to detect if the speaker is addressing the bot.
    /// Returns Some(question) if they are, None otherwise.
    /// Has a 5-second timeout to avoid blocking the transcription loop.
    pub async fn should_respond(&self, text: &str) -> Option<String> {
        let trimmed = text.trim();
        if trimmed.len() < 5 {
            return None;
        }

        let recent = {
            let t = self.transcript.lock().unwrap();
            let len = t.len();
            let start = if len > 5 { len - 5 } else { 0 };
            t[start..].join("\n")
        };

        let prompt = format!(
            "You are an intent detector for a meeting bot named \"{}\".\n\
             The trigger phrase is \"{}\" but speech recognition often misheard it \
             (e.g. \"hey what\", \"hi buddy\", \"high board\", \"hey boss\", etc.).\n\n\
             Given the transcript line below, determine if the speaker is addressing the bot.\n\
             Consider:\n\
             - Any phrase that sounds like \"{}\" (even badly transcribed)\n\
             - Directly mentioning the bot by name\n\
             - Asking a question clearly directed at an AI assistant\n\n\
             Recent meeting context:\n{}\n\n\
             New transcript line: \"{}\"\n\n\
             Respond with EXACTLY one of:\n\
             - \"YES: <the question they're asking>\" if they are addressing the bot\n\
             - \"NO\" if they are not addressing the bot\n\n\
             Examples:\n\
             - \"Hey boss, what is 2 plus 2?\" -> YES: what is 2 plus 2?\n\
             - \"High board, can you summarize?\" -> YES: can you summarize?\n\
             - \"I think we should schedule a meeting\" -> NO\n\
             - \"Bot, help me\" -> YES: help me",
            self.bot_name, self.trigger_phrase, self.trigger_phrase, recent, trimmed
        );

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.chat_once("gpt-4o-mini", &prompt, 0.0, 60),
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

    /// Respond to a question using the configured model (GPT-4o) with conversation history.
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

        let req = ChatRequest {
            model: self.model.clone(),
            messages,
            temperature: 0.7,
            max_tokens: 150,
        };

        let resp: ChatResponse = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .bearer_auth(&self.api_key)
            .json(&req)
            .send()
            .await?
            .json()
            .await?;

        let answer = resp
            .choices
            .first()
            .map(|c| c.message.content.trim().to_string())
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

        self.chat_once(&self.model, &prompt, 0.7, 300).await
    }

    async fn chat_once(
        &self,
        model: &str,
        prompt: &str,
        temp: f32,
        max_tokens: u32,
    ) -> anyhow::Result<String> {
        let req = ChatRequest {
            model: model.to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: prompt.to_string(),
            }],
            temperature: temp,
            max_tokens,
        };

        let resp: ChatResponse = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .bearer_auth(&self.api_key)
            .json(&req)
            .send()
            .await?
            .json()
            .await?;

        Ok(resp
            .choices
            .first()
            .map(|c| c.message.content.trim().to_string())
            .unwrap_or_default())
    }
}
