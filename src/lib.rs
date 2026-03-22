/// HTTP client for the `WeChat` iLink Bot API.
pub mod api;
/// Login and startup orchestration.
pub mod bot;
/// Error types used throughout the SDK.
pub mod errors;
/// Media upload, download, and AES encryption utilities.
pub mod media;
/// Data models for chat requests, responses, and agent traits.
pub mod models;
/// Long-polling message loop and message processing.
pub mod runtime;
/// Local filesystem persistence for account credentials and state.
pub mod storage;

pub use bot::{login, start};
pub use errors::{Error, Result};
pub use models::{
    Agent, ChatRequest, ChatResponse, IncomingMedia, LoginOptions, MediaType, OutgoingMedia,
    OutgoingMediaType, StartOptions,
};
