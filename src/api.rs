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
