use std::sync::Arc;

use serde_json::Value;
use snafu::ResultExt;
use tokio::sync::Mutex;
use tracing::{error, warn};

use crate::{
    api::WeixinApiClient,
    errors::{HttpSnafu, IoSnafu},
    media,
    models::{Agent, ChatRequest, IncomingMedia, OutgoingMediaType},
    storage,
};

/// Message item type: text (1-based, aligned with Python SDK).
const MESSAGE_ITEM_TEXT: u64 = 1;
/// Message item type: image.
const MESSAGE_ITEM_IMAGE: u64 = 2;
/// Message item type: voice.
const MESSAGE_ITEM_VOICE: u64 = 3;
/// Message item type: file.
const MESSAGE_ITEM_FILE: u64 = 4;
/// Message item type: video.
const MESSAGE_ITEM_VIDEO: u64 = 5;
/// Typing indicator status: currently typing.
const TYPING_STATUS_TYPING: u8 = 1;
/// Typing indicator status: cancel typing.
const TYPING_STATUS_CANCEL: u8 = 2;

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
///
/// Handles text items (type=1) with optional quoted messages in `ref_msg`,
/// and voice items (type=3) via transcription.
pub fn body_from_item_list(item_list: &[Value]) -> String {
    let mut parts = vec![];
    for item in item_list {
        let item_type = item["type"].as_u64().unwrap_or(0);
        match item_type {
            MESSAGE_ITEM_TEXT => {
                if let Some(body) = item["body"].as_str() {
                    parts.push(body.to_string());
                }
                if let Some(ref_msg) = item.get("ref_msg")
                    && let Some(ref_items) = ref_msg["item_list"].as_array()
                {
                    let ref_text = body_from_item_list(ref_items);
                    if !ref_text.is_empty() {
                        parts.push(format!("[Quoted: {ref_text}]"));
                    }
                }
            }
            MESSAGE_ITEM_VOICE => {
                if let Some(trans) = item["voice_transcription_body"].as_str() {
                    parts.push(trans.to_string());
                }
            }
            _ => {}
        }
    }
    parts.join("\n")
}

/// Finds the highest-priority media item in the given item list.
///
/// Priority: IMAGE > VIDEO > FILE > VOICE (only when no text) > `ref_msg` media.
/// Returns the item JSON and its type code.
fn find_media_item(item_list: &[Value], has_text: bool) -> Option<(Value, u64)> {
    // Check for image, video, file in priority order
    for &target in &[MESSAGE_ITEM_IMAGE, MESSAGE_ITEM_VIDEO, MESSAGE_ITEM_FILE] {
        for item in item_list {
            if item["type"].as_u64() == Some(target) {
                return Some((item.clone(), target));
            }
        }
    }
    // Voice only when there is no text body
    if !has_text {
        for item in item_list {
            if item["type"].as_u64() == Some(MESSAGE_ITEM_VOICE) {
                return Some((item.clone(), MESSAGE_ITEM_VOICE));
            }
        }
    }
    // Recurse into quoted/referenced messages
    for item in item_list {
        if let Some(ref_msg) = item.get("ref_msg")
            && let Some(ref_items) = ref_msg["item_list"].as_array()
            && let Some(found) = find_media_item(ref_items, has_text)
        {
            return Some(found);
        }
    }
    None
}

/// Builds a per-type outgoing media item JSON for `send_message`.
fn build_media_send_item(
    upload: &media::UploadResult,
    outgoing_type: OutgoingMediaType,
) -> Value {
    let media_obj = serde_json::json!({
        "encrypt_query_param": upload.encrypt_query_param,
        "aes_key": upload.aes_key,
        "encrypt_type": 1,
    });
    match outgoing_type {
        OutgoingMediaType::Video => serde_json::json!({
            "type": MESSAGE_ITEM_VIDEO,
            "video_item": {"media": media_obj, "video_size": upload.file_size}
        }),
        OutgoingMediaType::Image => serde_json::json!({
            "type": MESSAGE_ITEM_IMAGE,
            "image_item": {"media": media_obj, "mid_size": upload.file_size}
        }),
        OutgoingMediaType::File => serde_json::json!({
            "type": MESSAGE_ITEM_FILE,
            "file_item": {"media": media_obj, "file_name": upload.file_name, "len": upload.file_size}
        }),
    }
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
                warn!("Session expired, sleeping 1 hour before exit");
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
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

/// Downloads an outgoing media file from its URL, uploads to CDN, and sends it.
async fn send_outgoing_media(
    client: &WeixinApiClient,
    outgoing_media: &crate::models::OutgoingMedia,
    to_user_id: &str,
    context_token: &str,
) -> crate::Result<()> {
    let http_client = reqwest::Client::new();
    let media_bytes = http_client
        .get(&outgoing_media.url)
        .send()
        .await
        .context(HttpSnafu)?
        .bytes()
        .await
        .context(HttpSnafu)?;
    let tmp_dir = std::path::Path::new("/tmp/weixin-agent/media/upload");
    std::fs::create_dir_all(tmp_dir).context(IoSnafu)?;
    let file_name = outgoing_media.file_name.as_deref().unwrap_or("file");
    let tmp_path = tmp_dir.join(format!("{}_{file_name}", uuid::Uuid::new_v4()));
    std::fs::write(&tmp_path, &media_bytes).context(IoSnafu)?;
    let upload_media_type = match outgoing_media.media_type {
        OutgoingMediaType::Image => media::UPLOAD_MEDIA_IMAGE,
        OutgoingMediaType::Video => media::UPLOAD_MEDIA_VIDEO,
        OutgoingMediaType::File => media::UPLOAD_MEDIA_FILE,
    };
    let uploaded = media::upload_media(client, &tmp_path, upload_media_type, to_user_id).await?;
    let media_item = build_media_send_item(&uploaded, outgoing_media.media_type);
    client
        .send_message(to_user_id, context_token, &[media_item])
        .await?;
    Ok(())
}

async fn process_message(
    api_client: Arc<Mutex<WeixinApiClient>>,
    agent: Arc<dyn Agent>,
    msg: &Value,
) -> crate::Result<()> {
    let item_list = msg["item_list"].as_array().cloned().unwrap_or_default();
    let from_user_id = msg["from_user_id"].as_str().unwrap_or("");
    let context_token = msg["context_token"].as_str().unwrap_or("");
    let text = body_from_item_list(&item_list);

    // Slash commands
    if let Some(echo_text) = text.strip_prefix("/echo ") {
        let item = serde_json::json!({"type": MESSAGE_ITEM_TEXT, "body": echo_text});
        api_client
            .lock()
            .await
            .send_message(from_user_id, context_token, &[item])
            .await?;
        return Ok(());
    }

    // Typing indicator
    let _ = api_client
        .lock()
        .await
        .send_typing(from_user_id, context_token, TYPING_STATUS_TYPING)
        .await;

    // Extract and download media
    let has_text = !text.is_empty();
    let incoming_media =
        if let Some((media_item, media_type)) = find_media_item(&item_list, has_text) {
            match media::download_media_from_item(&media_item, media_type).await {
                Ok((path, mt, mime, fname)) => Some(IncomingMedia {
                    media_type: mt,
                    file_path: path.to_string_lossy().to_string(),
                    mime_type: mime,
                    file_name: fname,
                }),
                Err(e) => {
                    warn!("Failed to download media: {e}");
                    None
                }
            }
        } else {
            None
        };

    let request = ChatRequest {
        conversation_id: from_user_id.to_string(),
        text,
        media: incoming_media,
    };

    // Call agent -- on error, send error text to user then propagate
    let response = match agent.chat(request).await {
        Ok(resp) => resp,
        Err(e) => {
            error!("Agent error: {e}");
            let client = api_client.lock().await;
            let err_item = serde_json::json!({
                "type": MESSAGE_ITEM_TEXT,
                "body": format!("Error: {e}")
            });
            let _ = client
                .send_message(from_user_id, context_token, &[err_item])
                .await;
            let _ = client
                .send_typing(from_user_id, context_token, TYPING_STATUS_CANCEL)
                .await;
            drop(client);
            return Err(e);
        }
    };

    let client = api_client.lock().await;

    // Send text and media as SEPARATE messages (aligned with Python SDK)
    if let Some(ref outgoing_media) = response.media {
        if let Some(ref resp_text) = response.text {
            let plain = markdown_to_plain_text(resp_text);
            let text_item = serde_json::json!({"type": MESSAGE_ITEM_TEXT, "body": plain});
            let _ = client
                .send_message(from_user_id, context_token, &[text_item])
                .await;
        }
        send_outgoing_media(&client, outgoing_media, from_user_id, context_token).await?;
    } else if let Some(ref resp_text) = response.text {
        let plain = markdown_to_plain_text(resp_text);
        let text_item = serde_json::json!({"type": MESSAGE_ITEM_TEXT, "body": plain});
        client
            .send_message(from_user_id, context_token, &[text_item])
            .await?;
    }

    // Cancel typing
    let _ = client
        .send_typing(from_user_id, context_token, TYPING_STATUS_CANCEL)
        .await;
    drop(client);
    Ok(())
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
        let items = vec![serde_json::json!({"type": 1, "body": "hello world"})];
        let result = body_from_item_list(&items);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_voice_transcription() {
        let items = vec![serde_json::json!({
            "type": 3,
            "voice_transcription_body": "transcribed text"
        })];
        let result = body_from_item_list(&items);
        assert_eq!(result, "transcribed text");
    }

    #[test]
    fn test_quoted_message() {
        let items = vec![serde_json::json!({
            "type": 1,
            "body": "reply",
            "ref_msg": {"item_list": [{"type": 1, "body": "original message"}]}
        })];
        let result = body_from_item_list(&items);
        assert_eq!(result, "reply\n[Quoted: original message]");
    }

    #[test]
    fn test_multiple_items() {
        let items = vec![
            serde_json::json!({"type": 1, "body": "first"}),
            serde_json::json!({"type": 1, "body": "second"}),
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

    // -- find_media_item tests --

    #[test]
    fn test_find_media_image_priority() {
        let items = vec![
            serde_json::json!({"type": MESSAGE_ITEM_FILE, "file_item": {}}),
            serde_json::json!({"type": MESSAGE_ITEM_IMAGE, "image_item": {}}),
        ];
        let (_, t) = find_media_item(&items, false).unwrap();
        assert_eq!(t, MESSAGE_ITEM_IMAGE);
    }

    #[test]
    fn test_find_media_voice_skipped_when_text() {
        let items = vec![serde_json::json!({"type": MESSAGE_ITEM_VOICE})];
        assert!(find_media_item(&items, true).is_none());
    }

    #[test]
    fn test_find_media_voice_when_no_text() {
        let items = vec![serde_json::json!({"type": MESSAGE_ITEM_VOICE})];
        let (_, t) = find_media_item(&items, false).unwrap();
        assert_eq!(t, MESSAGE_ITEM_VOICE);
    }

    #[test]
    fn test_find_media_in_ref_msg() {
        let items = vec![serde_json::json!({
            "type": MESSAGE_ITEM_TEXT,
            "body": "look at this",
            "ref_msg": {
                "item_list": [{"type": MESSAGE_ITEM_IMAGE, "image_item": {}}]
            }
        })];
        let (_, t) = find_media_item(&items, true).unwrap();
        assert_eq!(t, MESSAGE_ITEM_IMAGE);
    }

    #[test]
    fn test_find_media_none() {
        let items = vec![serde_json::json!({"type": MESSAGE_ITEM_TEXT, "body": "hi"})];
        assert!(find_media_item(&items, false).is_none());
    }

    // -- build_media_send_item tests --

    #[test]
    fn test_build_image_send_item() {
        let upload = media::UploadResult {
            encrypt_query_param: "eqp".to_string(),
            aes_key: "key".to_string(),
            file_name: "img.png".to_string(),
            file_size: 1024,
        };
        let item = build_media_send_item(&upload, OutgoingMediaType::Image);
        assert_eq!(item["type"], MESSAGE_ITEM_IMAGE);
        assert!(item["image_item"]["media"]["encrypt_query_param"].is_string());
    }

    #[test]
    fn test_build_video_send_item() {
        let upload = media::UploadResult {
            encrypt_query_param: "eqp".to_string(),
            aes_key: "key".to_string(),
            file_name: "vid.mp4".to_string(),
            file_size: 2048,
        };
        let item = build_media_send_item(&upload, OutgoingMediaType::Video);
        assert_eq!(item["type"], MESSAGE_ITEM_VIDEO);
        assert!(item["video_item"]["media"]["encrypt_query_param"].is_string());
    }

    #[test]
    fn test_build_file_send_item() {
        let upload = media::UploadResult {
            encrypt_query_param: "eqp".to_string(),
            aes_key: "key".to_string(),
            file_name: "doc.pdf".to_string(),
            file_size: 4096,
        };
        let item = build_media_send_item(&upload, OutgoingMediaType::File);
        assert_eq!(item["type"], MESSAGE_ITEM_FILE);
        assert_eq!(item["file_item"]["file_name"], "doc.pdf");
        assert_eq!(item["file_item"]["len"], 4096);
    }
}
