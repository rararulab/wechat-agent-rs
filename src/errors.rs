use snafu::Snafu;

/// Errors that can occur when interacting with the `WeChat` Agent SDK.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    /// An HTTP request failed.
    #[snafu(display("HTTP error: {source}"))]
    Http { source: reqwest::Error },

    /// JSON serialization or deserialization failed.
    #[snafu(display("JSON error: {source}"))]
    Json { source: serde_json::Error },

    /// A filesystem I/O operation failed.
    #[snafu(display("IO error: {source}"))]
    Io { source: std::io::Error },

    /// The `WeChat` API returned a non-zero error code.
    #[snafu(display("API error (code {code}): {message}"))]
    Api {
        /// The numeric error code from the API.
        code:    i64,
        /// The human-readable error message.
        message: String,
    },

    /// The current session has expired and requires re-authentication.
    #[snafu(display("Session expired"))]
    SessionExpired,

    /// The login QR code has expired before being scanned.
    #[snafu(display("QR code expired"))]
    QrCodeExpired,

    /// The login flow failed for the given reason.
    #[snafu(display("Login failed: {reason}"))]
    LoginFailed {
        /// Description of why the login failed.
        reason: String,
    },

    /// No saved account was found in local storage.
    #[snafu(display("No account found"))]
    NoAccount,

    /// An AES encryption or decryption operation failed.
    #[snafu(display("Encryption error: {reason}"))]
    Encryption {
        /// Description of the encryption failure.
        reason: String,
    },
}

/// A convenience alias for `Result<T, Error>`.
pub type Result<T> = std::result::Result<T, Error>;
