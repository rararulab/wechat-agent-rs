use std::{future::Future, pin::Pin, sync::Arc};

use wechat_agent_rs::{Agent, ChatRequest, ChatResponse, LoginOptions, StartOptions, login, start};

struct EchoAgent;

impl Agent for EchoAgent {
    fn chat(
        &self,
        request: ChatRequest,
    ) -> Pin<Box<dyn Future<Output = wechat_agent_rs::Result<ChatResponse>> + Send + '_>> {
        Box::pin(async move {
            Ok(ChatResponse {
                text:  Some(format!("You said: {}", request.text)),
                media: None,
            })
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

    let agent = Arc::new(EchoAgent);
    start(
        agent,
        StartOptions {
            account_id: Some(account_id),
        },
    )
    .await?;

    Ok(())
}
