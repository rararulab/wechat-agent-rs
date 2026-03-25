use std::time::Duration;

use base64::Engine as _;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::Client;
use serde_json::Value;
use snafu::ResultExt;

use crate::errors::{ApiSnafu, HttpSnafu, Result, SessionExpiredSnafu};

const SESSION_EXPIRED_ERRCODE: i64 = -14;
const DEFAULT_LONG_POLL_TIMEOUT: Duration = Duration::from_secs(35);
const DEFAULT_API_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_CONFIG_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_ILINK_BOT_TYPE: &str = "3";
const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");

/// HTTP client wrapper for the `WeChat` iLink Bot API.
///
/// Handles authentication headers, request signing, and automatic
/// session-expiry detection on every response.
pub struct WeixinApiClient {
    client:    Client,
    base_url:  String,
    token:     String,
    route_tag: Option<String>,
}

impl WeixinApiClient {
    /// Creates a new API client targeting `base_url` with the given bearer
    /// `token`.
    pub fn new(base_url: &str, token: &str, route_tag: Option<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            route_tag,
        }
    }

    /// Replaces the bearer token used for subsequent requests.
    pub fn set_token(&mut self, token: &str) { self.token = token.to_string(); }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        headers.insert(
            HeaderName::from_static("authorizationtype"),
            HeaderValue::from_static("ilink_bot_token"),
        );
        // base64-encoded random u32 to match Python SDK behaviour
        let uin: u32 = rand::random();
        let uin_b64 = base64::engine::general_purpose::STANDARD.encode(uin.to_le_bytes());
        headers.insert(
            HeaderName::from_static("x-wechat-uin"),
            HeaderValue::from_str(&uin_b64).expect("valid base64"),
        );
        // Only add auth if token is non-empty
        if !self.token.is_empty() {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", self.token))
                    .expect("valid token"),
            );
        }
        if let Some(ref tag) = self.route_tag {
            headers.insert(
                HeaderName::from_static("skroutetag"),
                HeaderValue::from_str(tag).expect("valid route tag"),
            );
        }
        headers
    }

    /// Checks the API response for error codes (`errcode` or `ret`).
    #[allow(clippy::unused_self)]
    fn check_response(&self, resp: &Value) -> Result<()> {
        let code = resp
            .get("errcode")
            .and_then(Value::as_i64)
            .or_else(|| resp.get("ret").and_then(Value::as_i64));
        if let Some(code) = code {
            if code == SESSION_EXPIRED_ERRCODE {
                return Err(SessionExpiredSnafu.build());
            }
            if code != 0 {
                let msg = resp
                    .get("errmsg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error")
                    .to_string();
                return Err(ApiSnafu { code, message: msg }.build());
            }
        }
        Ok(())
    }

    async fn post_with_timeout(
        &self,
        path: &str,
        body: &Value,
        timeout: Duration,
    ) -> Result<Value> {
        let url = format!("{}/{}", self.base_url, path);
        // Inject base_info with SDK version into every POST payload
        let mut payload = body.clone();
        if let Some(obj) = payload.as_object_mut() {
            obj.insert(
                "base_info".to_string(),
                serde_json::json!({"channel_version": SDK_VERSION}),
            );
        }
        let resp = self
            .client
            .post(&url)
            .headers(self.headers())
            .json(&payload)
            .timeout(timeout)
            .send()
            .await
            .context(HttpSnafu)?
            .json::<Value>()
            .await
            .context(HttpSnafu)?;

        self.check_response(&resp)?;
        Ok(resp)
    }

    /// Sends a GET request with query parameters and optional extra headers.
    async fn get_with_timeout(
        &self,
        path: &str,
        query: &[(&str, &str)],
        extra_headers: Option<HeaderMap>,
        timeout: Duration,
    ) -> Result<Value> {
        let url = format!("{}/{}", self.base_url, path);
        let mut req = self
            .client
            .get(&url)
            .headers(self.headers())
            .query(query)
            .timeout(timeout);
        if let Some(h) = extra_headers {
            req = req.headers(h);
        }
        let resp = req
            .send()
            .await
            .context(HttpSnafu)?
            .json::<Value>()
            .await
            .context(HttpSnafu)?;
        self.check_response(&resp)?;
        Ok(resp)
    }

    /// Requests a new login QR code from the API.
    pub async fn fetch_qr_code(&self) -> Result<Value> {
        self.get_with_timeout(
            "ilink/bot/get_bot_qrcode",
            &[("bot_type", DEFAULT_ILINK_BOT_TYPE)],
            None,
            DEFAULT_API_TIMEOUT,
        )
        .await
    }

    /// Polls the current scan status for the given `qrcode`.
    pub async fn get_qr_code_status(&self, qrcode: &str) -> Result<Value> {
        let mut extra = HeaderMap::new();
        extra.insert(
            HeaderName::from_static("ilink-app-clientversion"),
            HeaderValue::from_static("1"),
        );
        self.get_with_timeout(
            "ilink/bot/get_qrcode_status",
            &[("qrcode", qrcode)],
            Some(extra),
            DEFAULT_LONG_POLL_TIMEOUT,
        )
        .await
    }

    /// Long-polls for new incoming messages, optionally resuming from `buf`.
    pub async fn get_updates(&self, buf: Option<&str>) -> Result<Value> {
        let mut body = serde_json::json!({});
        if let Some(b) = buf {
            body["get_updates_buf"] = Value::String(b.to_string());
        }
        self.post_with_timeout("ilink/bot/getupdates", &body, DEFAULT_LONG_POLL_TIMEOUT)
            .await
    }

    /// Sends a message with the given item list to `to_user_id`.
    pub async fn send_message(
        &self,
        to_user_id: &str,
        context_token: &str,
        item_list: &[Value],
    ) -> Result<Value> {
        let body = serde_json::json!({
            "to_user_id": to_user_id,
            "context_token": context_token,
            "item_list": item_list,
        });
        self.post_with_timeout("ilink/bot/sendmessage", &body, DEFAULT_API_TIMEOUT)
            .await
    }

    /// Sends a typing indicator to `to_user_id`.
    pub async fn send_typing(
        &self,
        to_user_id: &str,
        context_token: &str,
        typing_status: u8,
    ) -> Result<Value> {
        let body = serde_json::json!({
            "to_user_id": to_user_id,
            "context_token": context_token,
            "typing_status": typing_status,
        });
        self.post_with_timeout("ilink/bot/sendtyping", &body, DEFAULT_CONFIG_TIMEOUT)
            .await
    }

    /// Requests a pre-signed upload URL with full media metadata.
    #[allow(clippy::too_many_arguments)]
    pub async fn get_upload_url(
        &self,
        filekey: &str,
        media_type: u8,
        to_user_id: &str,
        rawsize: u64,
        rawfilemd5: &str,
        filesize: u64,
        aeskey: &str,
    ) -> Result<Value> {
        let body = serde_json::json!({
            "filekey": filekey,
            "media_type": media_type,
            "to_user_id": to_user_id,
            "rawsize": rawsize,
            "rawfilemd5": rawfilemd5,
            "filesize": filesize,
            "no_need_thumb": true,
            "aeskey": aeskey,
        });
        self.post_with_timeout("ilink/bot/getuploadurl", &body, DEFAULT_API_TIMEOUT)
            .await
    }

    /// Fetches the bot configuration (e.g. typing ticket).
    pub async fn get_config(&self) -> Result<Value> {
        self.post_with_timeout("ilink/bot/getconfig", &serde_json::json!({}), DEFAULT_CONFIG_TIMEOUT)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_new() {
        let client = WeixinApiClient::new("https://example.com/", "tok_123", None);
        assert_eq!(client.base_url, "https://example.com");
        assert_eq!(client.token, "tok_123");
        assert!(client.route_tag.is_none());
    }

    #[test]
    fn test_client_set_token() {
        let mut client = WeixinApiClient::new("https://example.com", "old_token", None);
        assert_eq!(client.token, "old_token");
        client.set_token("new_token");
        assert_eq!(client.token, "new_token");
    }

    #[test]
    fn test_headers_contain_base64_uin() {
        let client = WeixinApiClient::new("https://example.com", "tok", None);
        let headers = client.headers();
        let uin = headers
            .get("x-wechat-uin")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(base64::engine::general_purpose::STANDARD.decode(uin).is_ok());
    }

    #[test]
    fn test_headers_have_content_type() {
        let client = WeixinApiClient::new("https://example.com", "tok", None);
        let headers = client.headers();
        assert_eq!(
            headers.get("content-type").unwrap().to_str().unwrap(),
            "application/json"
        );
    }

    #[test]
    fn test_headers_skip_auth_when_empty_token() {
        let client = WeixinApiClient::new("https://example.com", "", None);
        let headers = client.headers();
        assert!(headers.get("authorization").is_none());
    }

    #[test]
    fn test_check_response_ok() {
        let client = WeixinApiClient::new("https://example.com", "tok", None);
        let resp = serde_json::json!({"errcode": 0, "errmsg": "ok"});
        assert!(client.check_response(&resp).is_ok());
    }

    #[test]
    fn test_check_response_session_expired() {
        let client = WeixinApiClient::new("https://example.com", "tok", None);
        let resp = serde_json::json!({"errcode": -14});
        let err = client.check_response(&resp).unwrap_err();
        assert!(
            matches!(err, crate::Error::SessionExpired),
            "expected SessionExpired, got: {err:?}"
        );
    }

    #[test]
    fn test_check_response_api_error() {
        let client = WeixinApiClient::new("https://example.com", "tok", None);
        let resp = serde_json::json!({"errcode": 42, "errmsg": "bad request"});
        let err = client.check_response(&resp).unwrap_err();
        assert!(
            matches!(err, crate::Error::Api { code: 42, .. }),
            "expected Api error with code 42, got: {err:?}"
        );
    }

    #[test]
    fn test_check_response_ret_field() {
        let client = WeixinApiClient::new("https://example.com", "tok", None);
        let resp = serde_json::json!({"ret": -14});
        let err = client.check_response(&resp).unwrap_err();
        assert!(
            matches!(err, crate::Error::SessionExpired),
            "expected SessionExpired via ret field, got: {err:?}"
        );
    }

    #[test]
    fn test_check_response_no_code() {
        let client = WeixinApiClient::new("https://example.com", "tok", None);
        let resp = serde_json::json!({"data": "something"});
        assert!(client.check_response(&resp).is_ok());
    }
}
