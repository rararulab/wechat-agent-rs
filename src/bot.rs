use std::sync::Arc;

use snafu::OptionExt;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::{
    api::WeixinApiClient,
    errors::{LoginFailedSnafu, NoAccountSnafu, QrCodeExpiredSnafu, Result},
    models::{LoginOptions, StartOptions},
    runtime::monitor_weixin,
    storage::{self, DEFAULT_BASE_URL},
};

/// Maximum number of QR code refreshes before giving up.
const MAX_QR_REFRESHES: u8 = 3;

/// Total deadline for the entire login flow (seconds).
const LOGIN_DEADLINE_SECS: u64 = 480;

/// Performs an interactive QR-code login and persists the resulting
/// credentials.
///
/// Uses a two-level loop matching the Python SDK: the outer loop refreshes the
/// QR code (up to 3 times), the inner loop polls scan status, and a global
/// 480-second deadline aborts the whole flow.
///
/// Returns the account ID on success.
pub async fn login(options: LoginOptions) -> Result<String> {
    let base_url = options.base_url.as_deref().unwrap_or(DEFAULT_BASE_URL);
    let client = WeixinApiClient::new(base_url, "", None);
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_secs(LOGIN_DEADLINE_SECS);
    let mut refresh_count = 0u8;

    loop {
        let qr_resp = client.fetch_qr_code().await?;
        // Response fields may be at root or nested under "data"
        let qrcode = qr_resp["qrcode"]
            .as_str()
            .or_else(|| qr_resp["data"]["qrcode"].as_str())
            .context(LoginFailedSnafu {
                reason: "no qrcode",
            })?;
        let qrcode_url = qr_resp["qrcode_url"]
            .as_str()
            .or_else(|| qr_resp["data"]["qrcode_url"].as_str())
            .unwrap_or(qrcode);

        let qr = qrcode::QrCode::new(qrcode_url.as_bytes()).map_err(|e| {
            LoginFailedSnafu {
                reason: format!("QR generation failed: {e}"),
            }
            .build()
        })?;
        let image = qr
            .render::<char>()
            .quiet_zone(true)
            .module_dimensions(2, 1)
            .build();
        println!("{image}");
        println!("Scan the QR code above with WeChat to login");

        loop {
            if tokio::time::Instant::now() > deadline {
                return Err(QrCodeExpiredSnafu.build());
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            let status_resp = client.get_qr_code_status(qrcode).await?;
            // Status field may be at root or nested under "data"
            let status = status_resp["status"]
                .as_str()
                .or_else(|| status_resp["data"]["status"].as_str())
                .unwrap_or("unknown");

            match status {
                "wait" => {}
                "scanned" | "scaned" => {
                    info!("QR code scanned, waiting for confirmation...");
                }
                "expired" => {
                    refresh_count += 1;
                    if refresh_count >= MAX_QR_REFRESHES {
                        return Err(QrCodeExpiredSnafu.build());
                    }
                    warn!(
                        "QR code expired, refreshing ({refresh_count}/{MAX_QR_REFRESHES})..."
                    );
                    break; // break inner loop to refresh QR in outer loop
                }
                "confirmed" => {
                    // Confirmed fields may be at root or nested under "data"
                    let data = if status_resp.get("bot_token").is_some() {
                        &status_resp
                    } else {
                        &status_resp["data"]
                    };
                    let token = data["bot_token"].as_str().context(LoginFailedSnafu {
                        reason: "no bot_token",
                    })?;
                    let bot_id =
                        data["ilink_bot_id"]
                            .as_str()
                            .context(LoginFailedSnafu {
                                reason: "no ilink_bot_id",
                            })?;
                    let base = data["baseurl"].as_str().unwrap_or(base_url);
                    let user_id = data["ilink_user_id"].as_str().unwrap_or("");

                    let raw_id = bot_id.strip_prefix("ilink_bot_").unwrap_or(bot_id);
                    let account_id = storage::normalize_account_id(raw_id);

                    let account_data = storage::AccountData {
                        token:    token.to_string(),
                        saved_at: chrono::Utc::now().to_rfc3339(),
                        base_url: base.to_string(),
                        user_id:  user_id.to_string(),
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
}

/// Starts the long-polling message loop for the given agent.
///
/// If no `account_id` is specified in `options`, the first saved account is
/// used.
pub async fn start(agent: Arc<dyn crate::models::Agent>, options: StartOptions) -> Result<()> {
    let account_id = if let Some(id) = options.account_id {
        id
    } else {
        let ids = storage::get_account_ids()?;
        ids.into_iter()
            .next()
            .ok_or_else(|| NoAccountSnafu.build())?
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
