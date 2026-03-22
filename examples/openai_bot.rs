use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use base64::Engine;
use reqwest::Client;
use serde_json::{Value, json};
use snafu::ResultExt;
use wechat_agent_rs::{
    Agent, ChatRequest, ChatResponse, LoginOptions, StartOptions,
    errors::{HttpSnafu, IoSnafu},
    login, start,
};

struct OpenAIAgent {
    client:        Client,
    base_url:      String,
    api_key:       String,
    model:         String,
    system_prompt: String,
    histories:     Mutex<HashMap<String, Vec<Value>>>,
}

impl OpenAIAgent {
    fn new() -> Self {
        Self {
            client:        Client::new(),
            base_url:      std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
            api_key:       std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY required"),
            model:         std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o".into()),
            system_prompt: std::env::var("SYSTEM_PROMPT")
                .unwrap_or_else(|_| "You are a helpful assistant.".into()),
            histories:     Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl Agent for OpenAIAgent {
    async fn chat(&self, request: ChatRequest) -> wechat_agent_rs::Result<ChatResponse> {
        let user_content = if let Some(ref media) = request.media {
            match media.media_type {
                wechat_agent_rs::MediaType::Image => {
                    let data = std::fs::read(&media.file_path).context(IoSnafu)?;
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                    json!([
                        {"type": "text", "text": request.text},
                        {"type": "image_url", "image_url": {"url": format!("data:{};base64,{b64}", media.mime_type)}}
                    ])
                }
                _ => {
                    json!(format!(
                        "{}\n[Attachment: {} ({})]",
                        request.text,
                        media.file_name.as_deref().unwrap_or("file"),
                        media.mime_type
                    ))
                }
            }
        } else {
            json!(request.text)
        };

        // Build messages while holding the lock, then drop it before await
        let messages = {
            let mut histories = self.histories.lock().unwrap();
            let history = histories
                .entry(request.conversation_id.clone())
                .or_default();

            history.push(json!({"role": "user", "content": user_content}));

            if history.len() > 50 {
                history.drain(0..history.len() - 50);
            }

            let mut messages = vec![json!({"role": "system", "content": self.system_prompt})];
            messages.extend(history.iter().cloned());
            drop(histories);
            messages
        };

        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&json!({
                "model": self.model,
                "messages": messages,
            }))
            .send()
            .await
            .context(HttpSnafu)?
            .json::<Value>()
            .await
            .context(HttpSnafu)?;

        let reply = resp["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("(no response)")
            .to_string();

        // Re-acquire lock to store assistant reply
        self.histories
            .lock()
            .unwrap()
            .entry(request.conversation_id)
            .or_default()
            .push(json!({"role": "assistant", "content": &reply}));

        Ok(ChatResponse {
            text:  Some(reply),
            media: None,
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let account_id = match wechat_agent_rs::storage::get_account_ids() {
        Ok(ids) if !ids.is_empty() => ids[0].clone(),
        _ => login(LoginOptions::default()).await?,
    };

    println!("Using account: {account_id}");

    let agent = Arc::new(OpenAIAgent::new());
    start(
        agent,
        StartOptions {
            account_id: Some(account_id),
        },
    )
    .await?;

    Ok(())
}
