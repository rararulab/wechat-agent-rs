use std::{path::Path, sync::Arc};

use serde_json::Value;
use snafu::ResultExt;
use tokio::sync::Mutex;
use tracing::{error, warn};

use crate::{
    api::WeixinApiClient,
    errors::{HttpSnafu, IoSnafu},
    media::{download_media, upload_media},
    models::{Agent, ChatRequest, IncomingMedia, MediaType},
    storage,
};

/// Strips `Markdown` formatting from text, returning a plain-text
/// approximation.
pub fn markdown_to_plain_text(text: &str) -> String {
    let mut result = text.to_string();
    let code_block_re = regex_lite::Regex::new(r"(?s)```[\s\S]*?```").unwrap();
    result = code_block_re.replace_all(&result, "").to_string();
    let inline_code_re = regex_lite::Regex::new(r"`[^`]+`").unwrap();
    result = inline_code_re.replace_all(&result, "").to_string();
    let img_re = regex_lite::Regex::new(r"!\[([^\]]*)\]\([^)]+\)").unwrap();
    result = img_re.replace_all(&result, "$1").to_string();
    let link_re = regex_lite::Regex::new(r"\[([^\]]+)\]\([^)]+\)").unwrap();
    result = link_re.replace_all(&result, "$1").to_string();
    let bold_re = regex_lite::Regex::new(r"\*\*([^*]+)\*\*").unwrap();
    result = bold_re.replace_all(&result, "$1").to_string();
    let italic_re = regex_lite::Regex::new(r"\*([^*]+)\*").unwrap();
    result = italic_re.replace_all(&result, "$1").to_string();
    let strike_re = regex_lite::Regex::new(r"~~([^~]+)~~").unwrap();
    result = strike_re.replace_all(&result, "$1").to_string();
    let table_sep_re = regex_lite::Regex::new(r"\|[-:| ]+\|").unwrap();
    result = table_sep_re.replace_all(&result, "").to_string();
    result.replace('|', " ").trim().to_string()
}

/// Extracts the text body from a `WeChat` `item_list` JSON array.
pub fn body_from_item_list(item_list: &[Value]) -> String {
    let mut parts = vec![];
    for item in item_list {
        let item_type = item["type"].as_u64().unwrap_or(0);
        match item_type {
            0 => {
                if let Some(body) = item["body"].as_str() {
                    parts.push(body.to_string());
                }
            }
            5 => {
                if let Some(trans) = item["voice_transcription_body"].as_str() {
                    parts.push(trans.to_string());
                }
            }
            7 => {
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

/// Runs the long-polling message loop, dispatching each message to `agent`.
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
                if let Some(new_buf) = resp["get_updates_buf"].as_str() {
                    buf = Some(new_buf.to_string());
                    let _ = storage::save_updates_buf(account_id, new_buf);
                }
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
                break;
            }
            Err(crate::Error::Http { ref source }) if source.is_timeout() => {}
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

    if let Some(echo_text) = text.strip_prefix("/echo ") {
        api_client
            .lock()
            .await
            .send_text_message(to_user_id, context_token, echo_text)
            .await?;
        return Ok(());
    }

    let _ = api_client
        .lock()
        .await
        .send_typing(to_user_id, context_token)
        .await;

    let incoming_media = extract_media_from_items(&item_list).await;

    let request = ChatRequest {
        conversation_id: to_user_id.to_string(),
        text,
        media: incoming_media,
    };

    let response = agent.chat(request).await?;

    let client = api_client.lock().await;
    if let Some(ref media) = response.media {
        let http_client = reqwest::Client::new();
        let media_bytes = http_client
            .get(&media.url)
            .send()
            .await
            .context(HttpSnafu)?
            .bytes()
            .await
            .context(HttpSnafu)?;
        let tmp_dir = Path::new("/tmp/weixin-agent/media");
        std::fs::create_dir_all(tmp_dir).context(IoSnafu)?;
        let file_name = media.file_name.as_deref().unwrap_or("file");
        let tmp_path = tmp_dir.join(format!("{}_{file_name}", uuid::Uuid::new_v4()));
        std::fs::write(&tmp_path, &media_bytes).context(IoSnafu)?;

        let uploaded = upload_media(&client, &tmp_path).await?;

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
        drop(client);
    } else if let Some(text) = &response.text {
        let plain = markdown_to_plain_text(text);
        client
            .send_text_message(to_user_id, context_token, &plain)
            .await?;
        drop(client);
    }

    Ok(())
}

async fn extract_media_from_items(item_list: &[Value]) -> Option<IncomingMedia> {
    for item in item_list {
        let item_type = item["type"].as_u64().unwrap_or(0);
        if matches!(item_type, 1..=5) {
            let file_key = item["body"]["filekey"]
                .as_str()
                .or_else(|| item["filekey"].as_str())?;
            let aes_key = item["body"]["aes_key"]
                .as_str()
                .or_else(|| item["aes_key"].as_str())?;
            let file_name = item["body"]["file_name"]
                .as_str()
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

#[cfg(test)]
mod tests {
    use super::*;

    // -- markdown_to_plain_text tests --

    #[test]
    fn test_strip_code_blocks() {
        let input = "before\n```rust\nfn main() {}\n```\nafter";
        let result = markdown_to_plain_text(input);
        assert_eq!(result, "before\n\nafter");
    }

    #[test]
    fn test_strip_inline_code() {
        let input = "use `println!` macro";
        let result = markdown_to_plain_text(input);
        assert_eq!(result, "use  macro");
    }

    #[test]
    fn test_strip_bold() {
        let input = "this is **bold** text";
        let result = markdown_to_plain_text(input);
        assert_eq!(result, "this is bold text");
    }

    #[test]
    fn test_strip_italic() {
        let input = "this is *italic* text";
        let result = markdown_to_plain_text(input);
        assert_eq!(result, "this is italic text");
    }

    #[test]
    fn test_strip_strikethrough() {
        let input = "this is ~~deleted~~ text";
        let result = markdown_to_plain_text(input);
        assert_eq!(result, "this is deleted text");
    }

    #[test]
    fn test_strip_links() {
        let input = "click [here](https://example.com) now";
        let result = markdown_to_plain_text(input);
        assert_eq!(result, "click here now");
    }

    #[test]
    fn test_strip_images() {
        let input = "see ![my image](https://example.com/img.png) above";
        let result = markdown_to_plain_text(input);
        assert_eq!(result, "see my image above");
    }

    #[test]
    fn test_strip_tables() {
        let input = "| Name | Age |\n|------|-----|\n| Alice | 30 |";
        let result = markdown_to_plain_text(input);
        assert!(
            !result.contains('|'),
            "pipes should be removed, got: {result}"
        );
        assert!(
            !result.contains("---"),
            "table separator should be removed, got: {result}"
        );
    }

    #[test]
    fn test_plain_text_passthrough() {
        let input = "Hello, this is plain text.";
        let result = markdown_to_plain_text(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_mixed_markdown() {
        let input = "**bold** and *italic* and [link](http://x.com)";
        let result = markdown_to_plain_text(input);
        assert_eq!(result, "bold and italic and link");
    }

    // -- body_from_item_list tests --

    #[test]
    fn test_text_item() {
        let items = vec![serde_json::json!({"type": 0, "body": "hello world"})];
        let result = body_from_item_list(&items);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_voice_transcription() {
        let items = vec![serde_json::json!({
            "type": 5,
            "voice_transcription_body": "transcribed text"
        })];
        let result = body_from_item_list(&items);
        assert_eq!(result, "transcribed text");
    }

    #[test]
    fn test_quoted_message() {
        let items = vec![serde_json::json!({
            "type": 7,
            "ref_item_list": [{"type": 0, "body": "original message"}]
        })];
        let result = body_from_item_list(&items);
        assert_eq!(result, "> original message");
    }

    #[test]
    fn test_multiple_items() {
        let items = vec![
            serde_json::json!({"type": 0, "body": "first"}),
            serde_json::json!({"type": 0, "body": "second"}),
        ];
        let result = body_from_item_list(&items);
        assert_eq!(result, "first\nsecond");
    }

    #[test]
    fn test_empty_list() {
        let items: Vec<Value> = vec![];
        let result = body_from_item_list(&items);
        assert_eq!(result, "");
    }

    #[test]
    fn test_unknown_type() {
        let items = vec![serde_json::json!({"type": 99, "body": "ignored"})];
        let result = body_from_item_list(&items);
        assert_eq!(result, "");
    }
}
