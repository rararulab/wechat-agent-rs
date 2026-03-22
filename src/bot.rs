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

    let qr_resp = client.fetch_qr_code().await?;
    let qrcode_url = qr_resp["data"]["qrcode_url"]
        .as_str()
        .ok_or_else(|| Error::LoginFailed("no qrcode_url".into()))?;
    let qrcode_id = qr_resp["data"]["qrcode_id"]
        .as_str()
        .ok_or_else(|| Error::LoginFailed("no qrcode_id".into()))?;

    let qr = qrcode::QrCode::new(qrcode_url.as_bytes())
        .map_err(|e| Error::LoginFailed(format!("QR generation failed: {e}")))?;
    let image = qr
        .render::<char>()
        .quiet_zone(true)
        .module_dimensions(2, 1)
        .build();
    println!("{image}");
    println!("Scan the QR code above with WeChat to login");

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
                let base = data["baseurl"].as_str().unwrap_or(base_url);
                let user_id = data["ilink_user_id"].as_str().unwrap_or("");

                let account_id = bot_id
                    .strip_prefix("ilink_bot_")
                    .unwrap_or(bot_id)
                    .to_string();

                let account_data = storage::AccountData {
                    token: token.to_string(),
                    saved_at: chrono::Utc::now().to_rfc3339(),
                    base_url: base.to_string(),
                    user_id: user_id.to_string(),
                };
                storage::save_account_data(&account_id, &account_data)?;

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
