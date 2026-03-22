use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Trait that application code implements to handle incoming chat messages.
///
/// The SDK calls [`Agent::chat`] for every incoming message and sends the
/// returned [`ChatResponse`] back to the `WeChat` user.
#[async_trait]
pub trait Agent: Send + Sync {
    /// Processes an incoming chat request and returns a response.
    async fn chat(&self, request: ChatRequest) -> crate::Result<ChatResponse>;
}

/// An incoming chat message delivered to the agent.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    /// Unique identifier for the conversation / user.
    pub conversation_id: String,
    /// The text body of the message (may be empty when media-only).
    pub text:            String,
    /// Optional media attachment included with the message.
    pub media:           Option<IncomingMedia>,
}

/// The agent's reply to be sent back to the user.
#[derive(Debug, Clone, Default)]
pub struct ChatResponse {
    /// Optional text content of the reply.
    pub text:  Option<String>,
    /// Optional media attachment to include in the reply.
    pub media: Option<OutgoingMedia>,
}

/// A media file received from a `WeChat` user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingMedia {
    /// The kind of media (image, audio, video, or generic file).
    pub media_type: MediaType,
    /// Local filesystem path to the downloaded and decrypted file.
    pub file_path:  String,
    /// MIME type of the file.
    pub mime_type:  String,
    /// Original file name, if available.
    pub file_name:  Option<String>,
}

/// A media file to be sent to a `WeChat` user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutgoingMedia {
    /// The kind of media to send.
    pub media_type: OutgoingMediaType,
    /// URL from which the SDK will download the file before uploading to
    /// `WeChat`.
    pub url:        String,
    /// Optional file name hint for the uploaded file.
    pub file_name:  Option<String>,
}

/// Classification of an incoming media attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    /// A still image (JPEG, PNG, etc.).
    Image,
    /// An audio recording or voice message.
    Audio,
    /// A video clip.
    Video,
    /// A generic file attachment.
    File,
}

/// Classification of an outgoing media attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutgoingMediaType {
    /// A still image.
    Image,
    /// A video clip.
    Video,
    /// A generic file.
    File,
}

/// Options for the QR-code login flow.
#[derive(Debug, Clone, Default)]
pub struct LoginOptions {
    /// Override the default API base URL.
    pub base_url: Option<String>,
}

/// Options for starting the message-polling loop.
#[derive(Debug, Clone, Default)]
pub struct StartOptions {
    /// Account ID to use; if `None`, the first saved account is used.
    pub account_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_response_default() {
        let resp = ChatResponse::default();
        assert!(resp.text.is_none());
        assert!(resp.media.is_none());
    }

    #[test]
    fn test_media_type_serde() {
        for variant in [
            MediaType::Image,
            MediaType::Audio,
            MediaType::Video,
            MediaType::File,
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            let deserialized: MediaType = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn test_outgoing_media_type_serde() {
        for variant in [
            OutgoingMediaType::Image,
            OutgoingMediaType::Video,
            OutgoingMediaType::File,
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            let deserialized: OutgoingMediaType = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, variant);
        }
    }
}
