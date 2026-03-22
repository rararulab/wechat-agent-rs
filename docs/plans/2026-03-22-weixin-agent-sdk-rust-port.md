# Weixin Agent SDK — Rust Port Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Port the Python `weixin-agent-sdk` (by frostming) to idiomatic Rust, preserving the same top-level API shape: `Agent` trait, `login()`, `start()`.

**Architecture:** Async-first using `tokio` + `reqwest`. The SDK is a library crate (`lib.rs`) with two examples (`echo_bot`, `openai_bot`). The `Agent` trait replaces Python's `Protocol`. Long-polling loop, AES-ECB media encryption, and file-based credential storage are all ported 1:1.

**Tech Stack:** `tokio`, `reqwest`, `serde`/`serde_json`, `aes`/`ecb`/`block-padding` (RustCrypto), `qrcode`, `mime_guess`, `tempfile`, `thiserror`, `async-trait`

---

## Module Mapping (Python → Rust)

| Python module | Rust module | Purpose |
|---|---|---|
| `models.py` | `models.rs` | Data types + `Agent` trait |
| `api.py` | `api.rs` | HTTP client for WeChat iLink Bot API |
| `storage.py` | `storage.rs` | File-based credential persistence |
| `media.py` | `media.rs` | AES-ECB encrypt/decrypt, CDN upload/download |
| `runtime.py` | `runtime.rs` | Message polling loop + dispatch |
| `bot.py` | `bot.rs` | `login()` and `start()` entry points |
| `__init__.py` | `lib.rs` | Public re-exports |

---

### Task 1: Project Scaffolding & Dependencies

**Files:**
- Modify: `Cargo.toml`
- Create: `src/lib.rs`
- Create: `src/models.rs`
- Create: `src/errors.rs`

**Step 1: Set up Cargo.toml with all dependencies**

```toml
[package]
name = "weixin-agent-sdk"
version = "0.1.0"
edition = "2024"
description = "Rust SDK for WeChat Agent (iLink Bot) — ported from frostming/weixin-agent-sdk"
license = "MIT"

[dependencies]
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", features = ["json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
aes = "0.8"
ecb = "0.1"
block-padding = "0.3"
cipher = "0.4"
qrcode = "0.14"
mime_guess = "2"
tempfile = "3"
thiserror = "2"
async-trait = "0.1"
rand = "0.9"
hex = "0.4"
dirs = "6"
tracing = "0.1"
uuid = { version = "1", features = ["v4"] }

[dev-dependencies]
tokio-test = "0.4"

[[example]]
name = "echo_bot"
path = "examples/echo_bot.rs"

[[example]]
name = "openai_bot"
path = "examples/openai_bot.rs"
```

**Step 2: Create `src/errors.rs` with error types**

```rust
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
```

**Step 3: Create `src/models.rs` with data types and Agent trait**

```rust
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

pub type LogFn = Box<dyn Fn(&str) + Send + Sync>;
```

**Step 4: Create `src/lib.rs` with re-exports**

```rust
pub mod api;
pub mod bot;
pub mod errors;
pub mod media;
pub mod models;
pub mod runtime;
pub mod storage;

pub use errors::{Error, Result};
pub use models::*;
pub use bot::{login, start};
```

**Step 5: Create stub modules so it compiles**

Create empty `src/api.rs`, `src/bot.rs`, `src/media.rs`, `src/runtime.rs`, `src/storage.rs` with just `// TODO` comments.

**Step 6: Run `cargo check`**

Run: `cargo check`
Expected: Compiles (with warnings about unused imports)

**Step 7: Commit**

```bash
git add -A && git commit -m "feat: project scaffolding with deps, models, and error types"
```

---

### Task 2: Storage Module

**Files:**
- Implement: `src/storage.rs`

**Step 1: Implement file-based credential storage**

Port `storage.py`. Key constants:
- `DEFAULT_BASE_URL = "https://ilinkai.weixin.qq.com"`
- `CDN_BASE_URL = "https://novac2c.cdn.weixin.qq.com/c2c"`
- Storage root: `~/.openclaw/openclaw-weixin/`

```rust
use crate::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
pub const CDN_BASE_URL: &str = "https://novac2c.cdn.weixin.qq.com/c2c";

fn storage_root() -> PathBuf {
    dirs::home_dir()
        .expect("no home directory")
        .join(".openclaw")
        .join("openclaw-weixin")
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AccountData {
    pub token: String,
    #[serde(rename = "savedAt")]
    pub saved_at: String,
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    #[serde(rename = "userId")]
    pub user_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AccountConfig {
    #[serde(default)]
    pub route_tag: Option<String>,
}

pub fn get_account_ids() -> Result<Vec<String>> {
    let path = storage_root().join("accounts.json");
    if !path.exists() {
        return Ok(vec![]);
    }
    let data = std::fs::read_to_string(&path)?;
    let ids: Vec<String> = serde_json::from_str(&data)?;
    Ok(ids)
}

pub fn save_account_ids(ids: &[String]) -> Result<()> {
    let path = storage_root().join("accounts.json");
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, serde_json::to_string_pretty(ids)?)?;
    Ok(())
}

pub fn get_account_data(account_id: &str) -> Result<AccountData> {
    let path = storage_root().join("accounts").join(format!("{account_id}.json"));
    let data = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&data)?)
}

pub fn save_account_data(account_id: &str, data: &AccountData) -> Result<()> {
    let path = storage_root().join("accounts").join(format!("{account_id}.json"));
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, serde_json::to_string_pretty(data)?)?;
    Ok(())
}

pub fn get_updates_buf(account_id: &str) -> Option<String> {
    let path = storage_root().join("get_updates_buf").join(format!("{account_id}.txt"));
    std::fs::read_to_string(&path).ok()
}

pub fn save_updates_buf(account_id: &str, buf: &str) -> Result<()> {
    let path = storage_root().join("get_updates_buf").join(format!("{account_id}.txt"));
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, buf)?;
    Ok(())
}

pub fn get_account_config(account_id: &str) -> Option<AccountConfig> {
    let path = storage_root().join("config").join(format!("{account_id}.json"));
    let data = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data).ok()
}
```

**Step 2: Run `cargo check`**

Expected: Compiles

**Step 3: Commit**

```bash
git add src/storage.rs && git commit -m "feat: implement file-based credential storage"
```

---

### Task 3: API Client

**Files:**
- Implement: `src/api.rs`

**Step 1: Implement the WeChat API client**

Port `api.py`. The client wraps all HTTP calls to the iLink Bot API.

```rust
use crate::errors::{Error, Result};
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

const SESSION_EXPIRED_ERRCODE: i64 = -14;

pub struct WeixinApiClient {
    client: Client,
    base_url: String,
    token: String,
    route_tag: Option<String>,
}

impl WeixinApiClient {
    pub fn new(base_url: &str, token: &str, route_tag: Option<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            route_tag,
        }
    }

    pub fn set_token(&mut self, token: &str) {
        self.token = token.to_string();
    }

    fn headers(&self) -> reqwest::header::HeaderMap {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("authorizationtype"),
            HeaderValue::from_static("ilink_bot_token"),
        );
        headers.insert(
            reqwest::header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.token)).unwrap(),
        );
        // Random X-WECHAT-UIN between 1000000000 and 9999999999
        let uin: u64 = rand::random::<u64>() % 9_000_000_000 + 1_000_000_000;
        headers.insert(
            HeaderName::from_static("x-wechat-uin"),
            HeaderValue::from_str(&uin.to_string()).unwrap(),
        );
        if let Some(ref tag) = self.route_tag {
            headers.insert(
                HeaderName::from_static("skroutetag"),
                HeaderValue::from_str(tag).unwrap(),
            );
        }
        headers
    }

    async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        self.post_with_timeout(path, body, Duration::from_secs(30)).await
    }

    async fn post_with_timeout(&self, path: &str, body: &Value, timeout: Duration) -> Result<Value> {
        let url = format!("{}/{}", self.base_url, path);
        let resp = self
            .client
            .post(&url)
            .headers(self.headers())
            .json(body)
            .timeout(timeout)
            .send()
            .await?
            .json::<Value>()
            .await?;

        if let Some(code) = resp.get("errcode").and_then(|v| v.as_i64()) {
            if code == SESSION_EXPIRED_ERRCODE {
                return Err(Error::SessionExpired);
            }
            if code != 0 {
                let msg = resp
                    .get("errmsg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error")
                    .to_string();
                return Err(Error::Api { code, message: msg });
            }
        }
        Ok(resp)
    }

    pub async fn fetch_qr_code(&self) -> Result<Value> {
        self.post("ilink/bot/get_bot_qrcode", &serde_json::json!({})).await
    }

    pub async fn get_qr_code_status(&self, qrcode_id: &str) -> Result<Value> {
        self.post(
            "ilink/bot/get_qrcode_status",
            &serde_json::json!({ "qrcode_id": qrcode_id }),
        )
        .await
    }

    pub async fn get_updates(&self, buf: Option<&str>) -> Result<Value> {
        let mut body = serde_json::json!({});
        if let Some(b) = buf {
            body["get_updates_buf"] = Value::String(b.to_string());
        }
        self.post_with_timeout("ilink/bot/getupdates", &body, Duration::from_secs(40))
            .await
    }

    pub async fn send_text_message(
        &self,
        to_user_id: &str,
        context_token: &str,
        text: &str,
    ) -> Result<Value> {
        let body = serde_json::json!({
            "to_user_id": to_user_id,
            "context_token": context_token,
            "item_list": [{
                "type": 0,
                "body": text
            }]
        });
        self.post("ilink/bot/sendmessage", &body).await
    }

    pub async fn send_media_message(
        &self,
        to_user_id: &str,
        context_token: &str,
        text: Option<&str>,
        file_info: &Value,
    ) -> Result<Value> {
        let mut item_list = vec![];
        if let Some(t) = text {
            item_list.push(serde_json::json!({ "type": 0, "body": t }));
        }
        item_list.push(file_info.clone());
        let body = serde_json::json!({
            "to_user_id": to_user_id,
            "context_token": context_token,
            "item_list": item_list
        });
        self.post("ilink/bot/sendmessage", &body).await
    }

    pub async fn send_typing(&self, to_user_id: &str, context_token: &str) -> Result<Value> {
        let body = serde_json::json!({
            "to_user_id": to_user_id,
            "context_token": context_token,
        });
        self.post("ilink/bot/sendtyping", &body).await
    }

    pub async fn get_upload_url(&self, file_name: &str, file_size: u64) -> Result<Value> {
        let body = serde_json::json!({
            "file_name": file_name,
            "file_size": file_size,
        });
        self.post("ilink/bot/getuploadurl", &body).await
    }
}
```

**Step 2: Run `cargo check`**

Expected: Compiles

**Step 3: Commit**

```bash
git add src/api.rs && git commit -m "feat: implement WeChat iLink Bot API client"
```

---

### Task 4: Media Module (AES-ECB Encryption + CDN)

**Files:**
- Implement: `src/media.rs`

**Step 1: Write a unit test for AES-ECB encrypt/decrypt roundtrip**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aes_ecb_roundtrip() {
        let key = [0x42u8; 16];
        let plaintext = b"Hello, WeChat media encryption!";
        let encrypted = encrypt_aes_ecb(&key, plaintext);
        let decrypted = decrypt_aes_ecb(&key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }
}
```

**Step 2: Implement AES-ECB encryption helpers**

```rust
use aes::Aes128;
use aes::cipher::{BlockEncryptMut, BlockDecryptMut, KeyInit};
use block_padding::Pkcs7;

type Aes128EcbEnc = ecb::Encryptor<Aes128>;
type Aes128EcbDec = ecb::Decryptor<Aes128>;

pub fn encrypt_aes_ecb(key: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let enc = Aes128EcbEnc::new(key.into());
    enc.encrypt_padded_vec_mut::<Pkcs7>(data)
}

pub fn decrypt_aes_ecb(key: &[u8; 16], data: &[u8]) -> crate::Result<Vec<u8>> {
    let dec = Aes128EcbDec::new(key.into());
    dec.decrypt_padded_vec_mut::<Pkcs7>(data)
        .map_err(|e| crate::Error::Encryption(e.to_string()))
}

pub fn parse_aes_key(hex_key: &str) -> crate::Result<[u8; 16]> {
    let bytes = hex::decode(hex_key)
        .map_err(|e| crate::Error::Encryption(format!("invalid hex key: {e}")))?;
    bytes
        .try_into()
        .map_err(|_| crate::Error::Encryption("AES key must be 16 bytes".into()))
}
```

**Step 3: Implement media download and upload functions**

```rust
use crate::api::WeixinApiClient;
use crate::storage::CDN_BASE_URL;
use reqwest::Client;
use serde_json::Value;
use std::path::{Path, PathBuf};

const MEDIA_DIR: &str = "/tmp/weixin-agent/media";

pub fn media_type_id(t: &str) -> u32 {
    match t {
        "image" => 1,
        "video" => 2,
        "file" => 3,
        "voice" | "audio" => 4,
        _ => 3,
    }
}

pub fn media_type_from_id(id: u32) -> &'static str {
    match id {
        1 => "image",
        2 => "video",
        4 => "audio",
        _ => "file",
    }
}

pub async fn download_media(
    file_key: &str,
    aes_key_hex: &str,
    file_name: Option<&str>,
) -> crate::Result<PathBuf> {
    let key = parse_aes_key(aes_key_hex)?;
    let url = format!("{CDN_BASE_URL}/{file_key}");
    let client = Client::new();
    let encrypted_bytes = client.get(&url).send().await?.bytes().await?;
    let decrypted = decrypt_aes_ecb(&key, &encrypted_bytes)?;

    let dir = Path::new(MEDIA_DIR);
    std::fs::create_dir_all(dir)?;

    let name = file_name.unwrap_or("download");
    let path = dir.join(format!("{}_{}", uuid::Uuid::new_v4(), name));
    std::fs::write(&path, &decrypted)?;
    Ok(path)
}

pub async fn upload_media(
    api_client: &WeixinApiClient,
    file_path: &Path,
) -> crate::Result<Value> {
    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let data = std::fs::read(file_path)?;
    let file_size = data.len() as u64;

    // Generate random AES key
    let key: [u8; 16] = rand::random();
    let aes_key_hex = hex::encode(key);
    let encrypted = encrypt_aes_ecb(&key, &data);

    // Get upload URL from API
    let upload_info = api_client.get_upload_url(file_name, file_size).await?;
    let upload_url = upload_info["data"]["upload_url"]
        .as_str()
        .ok_or_else(|| crate::Error::Api {
            code: -1,
            message: "no upload_url in response".into(),
        })?;
    let file_key = upload_info["data"]["file_key"]
        .as_str()
        .unwrap_or("")
        .to_string();

    // Upload encrypted data
    let client = Client::new();
    client
        .put(upload_url)
        .body(encrypted)
        .send()
        .await?;

    let mime = mime_guess::from_path(file_path)
        .first_or_octet_stream()
        .to_string();

    Ok(serde_json::json!({
        "filekey": file_key,
        "aes_key": aes_key_hex,
        "file_name": file_name,
        "file_size": file_size,
        "mime_type": mime,
    }))
}
```

**Step 4: Run `cargo test`**

Expected: AES roundtrip test passes

**Step 5: Commit**

```bash
git add src/media.rs && git commit -m "feat: implement AES-ECB media encryption and CDN upload/download"
```

---

### Task 5: Runtime (Message Polling Loop)

**Files:**
- Implement: `src/runtime.rs`

**Step 1: Implement message text extraction and markdown stripping**

```rust
use serde_json::Value;

/// Strip markdown formatting from text for plain WeChat messages
pub fn markdown_to_plain_text(text: &str) -> String {
    let mut result = text.to_string();
    // Remove code blocks
    let code_block_re = regex_lite::Regex::new(r"(?s)```[\s\S]*?```").unwrap();
    result = code_block_re.replace_all(&result, "").to_string();
    // Remove inline code
    let inline_code_re = regex_lite::Regex::new(r"`[^`]+`").unwrap();
    result = inline_code_re.replace_all(&result, "").to_string();
    // Remove images
    let img_re = regex_lite::Regex::new(r"!\[([^\]]*)\]\([^)]+\)").unwrap();
    result = img_re.replace_all(&result, "$1").to_string();
    // Remove links, keep text
    let link_re = regex_lite::Regex::new(r"\[([^\]]+)\]\([^)]+\)").unwrap();
    result = link_re.replace_all(&result, "$1").to_string();
    // Remove bold/italic/strikethrough
    let bold_re = regex_lite::Regex::new(r"\*\*([^*]+)\*\*").unwrap();
    result = bold_re.replace_all(&result, "$1").to_string();
    let italic_re = regex_lite::Regex::new(r"\*([^*]+)\*").unwrap();
    result = italic_re.replace_all(&result, "$1").to_string();
    let strike_re = regex_lite::Regex::new(r"~~([^~]+)~~").unwrap();
    result = strike_re.replace_all(&result, "$1").to_string();
    // Remove table formatting
    let table_sep_re = regex_lite::Regex::new(r"\|[-:| ]+\|").unwrap();
    result = table_sep_re.replace_all(&result, "").to_string();
    result.replace('|', " ").trim().to_string()
}

/// Extract text body from a WeChat message item_list
pub fn body_from_item_list(item_list: &[Value]) -> String {
    let mut parts = vec![];
    for item in item_list {
        let item_type = item["type"].as_u64().unwrap_or(0);
        match item_type {
            0 => {
                // Text item
                if let Some(body) = item["body"].as_str() {
                    parts.push(body.to_string());
                }
            }
            5 => {
                // Voice with transcription
                if let Some(trans) = item["voice_transcription_body"].as_str() {
                    parts.push(trans.to_string());
                }
            }
            7 => {
                // Quoted/referenced message
                if let Some(ref_list) = item["ref_item_list"].as_array() {
                    let ref_text = body_from_item_list(ref_list);
                    if !ref_text.is_empty() {
                        parts.push(format!("> {ref_text}"));
                    }
                }
            }
            _ => {}
        }
    }
    parts.join("\n")
}
```

**Step 2: Implement the main monitor loop and message processor**

```rust
use crate::api::WeixinApiClient;
use crate::media::{download_media, media_type_from_id, upload_media};
use crate::models::{Agent, ChatRequest, IncomingMedia, MediaType};
use crate::storage;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

pub async fn monitor_weixin(
    api_client: Arc<Mutex<WeixinApiClient>>,
    agent: Arc<dyn Agent>,
    account_id: &str,
) {
    let mut consecutive_errors = 0u32;
    let mut buf = storage::get_updates_buf(account_id);

    loop {
        let result = {
            let client = api_client.lock().await;
            client.get_updates(buf.as_deref()).await
        };

        match result {
            Ok(resp) => {
                consecutive_errors = 0;
                // Update buffer
                if let Some(new_buf) = resp["get_updates_buf"].as_str() {
                    buf = Some(new_buf.to_string());
                    let _ = storage::save_updates_buf(account_id, new_buf);
                }
                // Process messages
                if let Some(messages) = resp["msg_list"].as_array() {
                    for msg in messages {
                        let agent = agent.clone();
                        let api = api_client.clone();
                        let msg = msg.clone();
                        tokio::spawn(async move {
                            if let Err(e) = process_message(api, agent, &msg).await {
                                error!("Error processing message: {e}");
                            }
                        });
                    }
                }
            }
            Err(crate::Error::SessionExpired) => {
                warn!("Session expired, need to re-login");
                // In the Python version, this re-authenticates the token.
                // For now, we log and break. The caller should handle re-auth.
                break;
            }
            Err(crate::Error::Http(ref e)) if e.is_timeout() => {
                // Timeout is normal for long-polling, just retry
                continue;
            }
            Err(e) => {
                consecutive_errors += 1;
                error!("Error getting updates ({consecutive_errors}): {e}");
                if consecutive_errors >= 3 {
                    warn!("Too many consecutive errors, backing off 30s");
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    consecutive_errors = 0;
                } else {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
        }
    }
}

async fn process_message(
    api_client: Arc<Mutex<WeixinApiClient>>,
    agent: Arc<dyn Agent>,
    msg: &Value,
) -> crate::Result<()> {
    let item_list = msg["item_list"].as_array().cloned().unwrap_or_default();
    let to_user_id = msg["to_user_id"].as_str().unwrap_or("");
    let context_token = msg["context_token"].as_str().unwrap_or("");

    let text = body_from_item_list(&item_list);

    // Check for built-in commands
    if text.starts_with("/echo ") {
        let echo_text = &text[6..];
        let client = api_client.lock().await;
        client.send_text_message(to_user_id, context_token, echo_text).await?;
        return Ok(());
    }

    // Send typing indicator
    {
        let client = api_client.lock().await;
        let _ = client.send_typing(to_user_id, context_token).await;
    }

    // Download incoming media if present
    let incoming_media = extract_media_from_items(&item_list).await;

    // Build request and call agent
    let request = ChatRequest {
        conversation_id: to_user_id.to_string(),
        text,
        media: incoming_media,
    };

    let response = agent.chat(request).await?;

    // Send response
    let client = api_client.lock().await;
    if let Some(ref media) = response.media {
        // Download the outgoing media URL, upload encrypted, send
        let http_client = reqwest::Client::new();
        let media_bytes = http_client.get(&media.url).send().await?.bytes().await?;
        let tmp_dir = Path::new("/tmp/weixin-agent/media");
        std::fs::create_dir_all(tmp_dir)?;
        let file_name = media.file_name.as_deref().unwrap_or("file");
        let tmp_path = tmp_dir.join(format!("{}_{file_name}", uuid::Uuid::new_v4()));
        std::fs::write(&tmp_path, &media_bytes)?;

        let uploaded = upload_media(&*client, &tmp_path).await?;

        let media_type_id = match media.media_type {
            crate::models::OutgoingMediaType::Image => 1,
            crate::models::OutgoingMediaType::Video => 2,
            crate::models::OutgoingMediaType::File => 3,
        };

        let file_info = serde_json::json!({
            "type": media_type_id,
            "body": uploaded,
        });

        client
            .send_media_message(
                to_user_id,
                context_token,
                response.text.as_deref(),
                &file_info,
            )
            .await?;
    } else if let Some(ref text) = response.text {
        let plain = markdown_to_plain_text(text);
        client.send_text_message(to_user_id, context_token, &plain).await?;
    }

    Ok(())
}

async fn extract_media_from_items(item_list: &[Value]) -> Option<IncomingMedia> {
    for item in item_list {
        let item_type = item["type"].as_u64().unwrap_or(0);
        // Types 1=image, 2=video, 3=file, 4/5=voice
        if matches!(item_type, 1 | 2 | 3 | 4 | 5) {
            let file_key = item["body"]["filekey"].as_str()
                .or_else(|| item["filekey"].as_str())?;
            let aes_key = item["body"]["aes_key"].as_str()
                .or_else(|| item["aes_key"].as_str())?;
            let file_name = item["body"]["file_name"].as_str()
                .or_else(|| item["file_name"].as_str());

            if let Ok(path) = download_media(file_key, aes_key, file_name).await {
                let media_type = match item_type {
                    1 => MediaType::Image,
                    2 => MediaType::Video,
                    4 | 5 => MediaType::Audio,
                    _ => MediaType::File,
                };
                let mime = mime_guess::from_path(&path)
                    .first_or_octet_stream()
                    .to_string();
                return Some(IncomingMedia {
                    media_type,
                    file_path: path.to_string_lossy().to_string(),
                    mime_type: mime,
                    file_name: file_name.map(String::from),
                });
            }
        }
    }
    None
}
```

**Step 3: Add `regex-lite` dependency to Cargo.toml**

Add: `regex-lite = "0.1"`

**Step 4: Run `cargo check`**

Expected: Compiles

**Step 5: Commit**

```bash
git add src/runtime.rs Cargo.toml && git commit -m "feat: implement message polling loop and message processor"
```

---

### Task 6: Bot Module (login + start)

**Files:**
- Implement: `src/bot.rs`

**Step 1: Implement login flow with QR code**

```rust
use crate::api::WeixinApiClient;
use crate::errors::{Error, Result};
use crate::models::{LoginOptions, StartOptions};
use crate::runtime::monitor_weixin;
use crate::storage::{self, DEFAULT_BASE_URL};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

pub async fn login(options: LoginOptions) -> Result<String> {
    let base_url = options.base_url.as_deref().unwrap_or(DEFAULT_BASE_URL);
    let client = WeixinApiClient::new(base_url, "", None);

    // Fetch QR code
    let qr_resp = client.fetch_qr_code().await?;
    let qrcode_url = qr_resp["data"]["qrcode_url"]
        .as_str()
        .ok_or_else(|| Error::LoginFailed("no qrcode_url".into()))?;
    let qrcode_id = qr_resp["data"]["qrcode_id"]
        .as_str()
        .ok_or_else(|| Error::LoginFailed("no qrcode_id".into()))?;

    // Print QR code to terminal
    let qr = qrcode::QrCode::new(qrcode_url.as_bytes())
        .map_err(|e| Error::LoginFailed(format!("QR generation failed: {e}")))?;
    let image = qr
        .render::<char>()
        .quiet_zone(true)
        .module_dimensions(2, 1)
        .build();
    println!("{image}");
    println!("Scan the QR code above with WeChat to login");

    // Poll for status
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let status_resp = client.get_qr_code_status(qrcode_id).await?;
        let status = status_resp["data"]["status"]
            .as_str()
            .unwrap_or("unknown");

        match status {
            "wait" => continue,
            "scaned" => {
                info!("QR code scanned, waiting for confirmation...");
            }
            "expired" => {
                return Err(Error::QrCodeExpired);
            }
            "confirmed" => {
                let data = &status_resp["data"];
                let token = data["bot_token"]
                    .as_str()
                    .ok_or_else(|| Error::LoginFailed("no bot_token".into()))?;
                let bot_id = data["ilink_bot_id"]
                    .as_str()
                    .ok_or_else(|| Error::LoginFailed("no ilink_bot_id".into()))?;
                let base = data["baseurl"]
                    .as_str()
                    .unwrap_or(base_url);
                let user_id = data["ilink_user_id"]
                    .as_str()
                    .unwrap_or("");

                // Normalize account ID
                let account_id = bot_id
                    .strip_prefix("ilink_bot_")
                    .unwrap_or(bot_id)
                    .to_string();

                // Save credentials
                let account_data = storage::AccountData {
                    token: token.to_string(),
                    saved_at: chrono::Utc::now().to_rfc3339(),
                    base_url: base.to_string(),
                    user_id: user_id.to_string(),
                };
                storage::save_account_data(&account_id, &account_data)?;

                // Update account list
                let mut ids = storage::get_account_ids().unwrap_or_default();
                if !ids.contains(&account_id) {
                    ids.push(account_id.clone());
                    storage::save_account_ids(&ids)?;
                }

                info!("Login successful! Account ID: {account_id}");
                return Ok(account_id);
            }
            other => {
                warn!("Unknown QR status: {other}");
            }
        }
    }
}

pub async fn start(agent: Arc<dyn crate::models::Agent>, options: StartOptions) -> Result<()> {
    // Resolve account
    let account_id = match options.account_id {
        Some(id) => id,
        None => {
            let ids = storage::get_account_ids()?;
            ids.into_iter().next().ok_or(Error::NoAccount)?
        }
    };

    let account_data = storage::get_account_data(&account_id)?;
    let config = storage::get_account_config(&account_id);
    let route_tag = config.and_then(|c| c.route_tag);

    let api_client = Arc::new(Mutex::new(WeixinApiClient::new(
        &account_data.base_url,
        &account_data.token,
        route_tag,
    )));

    info!("Starting message loop for account {account_id}...");
    monitor_weixin(api_client, agent, &account_id).await;

    Ok(())
}
```

**Step 2: Add `chrono` dependency to Cargo.toml**

Add: `chrono = "0.4"`

**Step 3: Run `cargo check`**

Expected: Compiles

**Step 4: Commit**

```bash
git add src/bot.rs Cargo.toml && git commit -m "feat: implement login and start entry points"
```

---

### Task 7: Update lib.rs + Fix Compilation

**Files:**
- Modify: `src/lib.rs`

**Step 1: Update lib.rs with final re-exports**

```rust
pub mod api;
pub mod bot;
pub mod errors;
pub mod media;
pub mod models;
pub mod runtime;
pub mod storage;

pub use bot::{login, start};
pub use errors::{Error, Result};
pub use models::{
    Agent, ChatRequest, ChatResponse, IncomingMedia, LoginOptions, MediaType, OutgoingMedia,
    OutgoingMediaType, StartOptions,
};
```

**Step 2: Run `cargo build` — fix any compilation errors**

**Step 3: Commit**

```bash
git add -A && git commit -m "feat: finalize lib.rs re-exports, fix compilation"
```

---

### Task 8: Echo Bot Example

**Files:**
- Create: `examples/echo_bot.rs`

**Step 1: Write the echo bot example**

```rust
use async_trait::async_trait;
use std::sync::Arc;
use weixin_agent_sdk::{Agent, ChatRequest, ChatResponse, LoginOptions, StartOptions, login, start};

struct EchoAgent;

#[async_trait]
impl Agent for EchoAgent {
    async fn chat(&self, request: ChatRequest) -> weixin_agent_sdk::Result<ChatResponse> {
        Ok(ChatResponse {
            text: Some(format!("You said: {}", request.text)),
            media: None,
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // Login if no account exists
    let account_id = match weixin_agent_sdk::storage::get_account_ids() {
        Ok(ids) if !ids.is_empty() => ids[0].clone(),
        _ => login(LoginOptions::default()).await?,
    };

    println!("Using account: {account_id}");

    let agent = Arc::new(EchoAgent);
    start(agent, StartOptions {
        account_id: Some(account_id),
    })
    .await?;

    Ok(())
}
```

**Step 2: Add dev/example dependencies to Cargo.toml**

```toml
[dependencies]
# ... existing deps ...
anyhow = "1"
tracing-subscriber = "0.3"
```

**Step 3: Run `cargo check --example echo_bot`**

Expected: Compiles

**Step 4: Commit**

```bash
git add examples/echo_bot.rs Cargo.toml && git commit -m "feat: add echo_bot example"
```

---

### Task 9: OpenAI Bot Example

**Files:**
- Create: `examples/openai_bot.rs`

**Step 1: Write the OpenAI bot example**

```rust
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use weixin_agent_sdk::{Agent, ChatRequest, ChatResponse, LoginOptions, StartOptions, login, start};

struct OpenAIAgent {
    client: Client,
    base_url: String,
    api_key: String,
    model: String,
    system_prompt: String,
    histories: Mutex<HashMap<String, Vec<Value>>>,
}

impl OpenAIAgent {
    fn new() -> Self {
        Self {
            client: Client::new(),
            base_url: std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
            api_key: std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY required"),
            model: std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o".into()),
            system_prompt: std::env::var("SYSTEM_PROMPT")
                .unwrap_or_else(|_| "You are a helpful assistant.".into()),
            histories: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl Agent for OpenAIAgent {
    async fn chat(&self, request: ChatRequest) -> weixin_agent_sdk::Result<ChatResponse> {
        let mut histories = self.histories.lock().unwrap();
        let history = histories
            .entry(request.conversation_id.clone())
            .or_insert_with(Vec::new);

        // Add user message
        let user_content = if let Some(ref media) = request.media {
            match media.media_type {
                weixin_agent_sdk::MediaType::Image => {
                    let data = std::fs::read(&media.file_path)?;
                    let b64 = base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        &data,
                    );
                    json!([
                        {"type": "text", "text": request.text},
                        {"type": "image_url", "image_url": {"url": format!("data:{};base64,{b64}", media.mime_type)}}
                    ])
                }
                _ => {
                    json!(format!("{}\n[Attachment: {} ({})]", request.text, media.file_name.as_deref().unwrap_or("file"), media.mime_type))
                }
            }
        } else {
            json!(request.text)
        };

        history.push(json!({"role": "user", "content": user_content}));

        // Keep history bounded
        if history.len() > 50 {
            history.drain(0..history.len() - 50);
        }

        // Build messages
        let mut messages = vec![json!({"role": "system", "content": self.system_prompt})];
        messages.extend(history.iter().cloned());

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
            .map_err(weixin_agent_sdk::Error::Http)?
            .json::<Value>()
            .await
            .map_err(weixin_agent_sdk::Error::Http)?;

        let reply = resp["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("(no response)")
            .to_string();

        history.push(json!({"role": "assistant", "content": &reply}));

        Ok(ChatResponse {
            text: Some(reply),
            media: None,
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let account_id = match weixin_agent_sdk::storage::get_account_ids() {
        Ok(ids) if !ids.is_empty() => ids[0].clone(),
        _ => login(LoginOptions::default()).await?,
    };

    println!("Using account: {account_id}");

    let agent = Arc::new(OpenAIAgent::new());
    start(agent, StartOptions {
        account_id: Some(account_id),
    })
    .await?;

    Ok(())
}
```

**Step 2: Add `base64` dependency**

Add: `base64 = "0.22"`

**Step 3: Run `cargo check --example openai_bot`**

Expected: Compiles

**Step 4: Commit**

```bash
git add examples/openai_bot.rs Cargo.toml && git commit -m "feat: add openai_bot example"
```

---

### Task 10: Final Polish & Verify

**Step 1: Run `cargo build --all-targets`**

Expected: Everything compiles

**Step 2: Run `cargo test`**

Expected: AES roundtrip test passes

**Step 3: Run `cargo clippy`**

Fix any lints

**Step 4: Final commit**

```bash
git add -A && git commit -m "chore: fix clippy lints and finalize Rust port"
```

---

## Summary of Rust Dependencies

| Crate | Maps to Python |
|---|---|
| `tokio` | `asyncio` |
| `reqwest` | `httpx` |
| `serde` / `serde_json` | dataclasses + json |
| `aes` / `ecb` / `block-padding` | `cryptography` (AES-ECB) |
| `qrcode` | `qrcode` |
| `mime_guess` | mime detection |
| `async-trait` | `Protocol` |
| `thiserror` | custom exceptions |
| `tracing` | logging |
| `dirs` | `~` home dir resolution |
| `chrono` | datetime |
| `hex` | hex encoding |
| `uuid` | unique file names |
| `rand` | random AES keys |
| `regex-lite` | markdown stripping |
| `base64` | base64 encoding (example) |
| `anyhow` | example error handling |
