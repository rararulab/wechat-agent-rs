use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("API error (code {code}): {message}")]
    Api { code: i64, message: String },

    #[error("Session expired")]
    SessionExpired,

    #[error("QR code expired")]
    QrCodeExpired,

    #[error("Login failed: {0}")]
    LoginFailed(String),

    #[error("No account found")]
    NoAccount,

    #[error("Encryption error: {0}")]
    Encryption(String),
}

pub type Result<T> = std::result::Result<T, Error>;
