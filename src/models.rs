use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[async_trait]
pub trait Agent: Send + Sync {
    async fn chat(&self, request: ChatRequest) -> crate::Result<ChatResponse>;
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub conversation_id: String,
    pub text: String,
    pub media: Option<IncomingMedia>,
}

#[derive(Debug, Clone, Default)]
pub struct ChatResponse {
    pub text: Option<String>,
    pub media: Option<OutgoingMedia>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingMedia {
    pub media_type: MediaType,
    pub file_path: String,
    pub mime_type: String,
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutgoingMedia {
    pub media_type: OutgoingMediaType,
    pub url: String,
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    Image,
    Audio,
    Video,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutgoingMediaType {
    Image,
    Video,
    File,
}

#[derive(Debug, Clone, Default)]
pub struct LoginOptions {
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct StartOptions {
    pub account_id: Option<String>,
}
