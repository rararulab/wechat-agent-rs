use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use snafu::ResultExt;

use crate::{
    Result,
    errors::{IoSnafu, JsonSnafu},
};

/// Default `WeChat` iLink API base URL.
pub const DEFAULT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";

/// Base URL for downloading encrypted media from the `WeChat` CDN.
pub const CDN_BASE_URL: &str = "https://novac2c.cdn.weixin.qq.com/c2c";

/// Resolves the state directory by checking env vars in priority order:
/// `$OPENCLAW_STATE_DIR` -> `$CLAWDBOT_STATE_DIR` -> `~/.openclaw`.
fn state_dir() -> PathBuf {
    std::env::var("OPENCLAW_STATE_DIR")
        .or_else(|_| std::env::var("CLAWDBOT_STATE_DIR"))
        .map_or_else(
            |_| {
                dirs::home_dir()
                    .expect("no home directory")
                    .join(".openclaw")
            },
            PathBuf::from,
        )
}

fn storage_root() -> PathBuf {
    state_dir().join("openclaw-weixin")
}

/// Normalizes an account ID: trims whitespace, lowercases, and replaces
/// `@` and `.` with `-`.
pub fn normalize_account_id(id: &str) -> String {
    id.trim().to_lowercase().replace(['@', '.'], "-")
}

/// Derives the raw account ID from a normalized one.
///
/// For now this is an identity function — reverse mapping is best-effort.
pub fn derive_raw_account_id(id: &str) -> String {
    id.to_string()
}

/// Persisted authentication credentials for a single `WeChat` account.
#[derive(Debug, Serialize, Deserialize)]
pub struct AccountData {
    /// Bearer token used to authenticate API requests.
    pub token:    String,
    /// ISO-8601 timestamp of when this data was saved.
    #[serde(rename = "savedAt")]
    pub saved_at: String,
    /// API base URL for this account.
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    /// The iLink user ID associated with this account.
    #[serde(rename = "userId")]
    pub user_id:  String,
}

/// Per-account configuration loaded from local storage.
#[derive(Debug, Serialize, Deserialize)]
pub struct AccountConfig {
    /// Optional routing tag sent as a header on every API request.
    #[serde(default)]
    pub route_tag: Option<String>,
}

/// Returns the list of saved account IDs from local storage.
pub fn get_account_ids() -> Result<Vec<String>> {
    let path = storage_root().join("accounts.json");
    if !path.exists() {
        return Ok(vec![]);
    }
    let data = std::fs::read_to_string(&path).context(IoSnafu)?;
    let ids: Vec<String> = serde_json::from_str(&data).context(JsonSnafu)?;
    Ok(ids)
}

/// Persists the given list of account IDs to local storage.
pub fn save_account_ids(ids: &[String]) -> Result<()> {
    let path = storage_root().join("accounts.json");
    std::fs::create_dir_all(path.parent().expect("accounts.json must have a parent dir"))
        .context(IoSnafu)?;
    let json = serde_json::to_string_pretty(ids).context(JsonSnafu)?;
    std::fs::write(&path, json).context(IoSnafu)?;
    Ok(())
}

/// Loads the saved credentials for the given account.
pub fn get_account_data(account_id: &str) -> Result<AccountData> {
    let path = storage_root()
        .join("accounts")
        .join(format!("{account_id}.json"));
    let data = std::fs::read_to_string(&path).context(IoSnafu)?;
    serde_json::from_str(&data).context(JsonSnafu)
}

/// Saves credentials for the given account to local storage.
///
/// On Unix systems, the file is set to mode 600 (owner read/write only).
pub fn save_account_data(account_id: &str, data: &AccountData) -> Result<()> {
    let path = storage_root()
        .join("accounts")
        .join(format!("{account_id}.json"));
    std::fs::create_dir_all(path.parent().expect("account file must have a parent dir"))
        .context(IoSnafu)?;
    let json = serde_json::to_string_pretty(data).context(JsonSnafu)?;
    std::fs::write(&path, &json).context(IoSnafu)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&path, perms).context(IoSnafu)?;
    }

    Ok(())
}

/// JSON wrapper for the sync buffer, matching the Python SDK format.
#[derive(Debug, Serialize, Deserialize)]
struct SyncBuf {
    get_updates_buf: String,
}

/// Returns the saved long-poll continuation buffer for the given account.
///
/// Reads from `accounts/{id}.sync.json` as a JSON object with a
/// `get_updates_buf` field, matching the Python SDK format.
pub fn get_updates_buf(account_id: &str) -> Option<String> {
    let path = storage_root()
        .join("accounts")
        .join(format!("{account_id}.sync.json"));
    let data = std::fs::read_to_string(&path).ok()?;
    let buf: SyncBuf = serde_json::from_str(&data).ok()?;
    Some(buf.get_updates_buf)
}

/// Saves the long-poll continuation buffer for the given account.
///
/// Writes to `accounts/{id}.sync.json` as a JSON object with a
/// `get_updates_buf` field, matching the Python SDK format.
pub fn save_updates_buf(account_id: &str, buf: &str) -> Result<()> {
    let path = storage_root()
        .join("accounts")
        .join(format!("{account_id}.sync.json"));
    std::fs::create_dir_all(path.parent().expect("sync file must have a parent dir"))
        .context(IoSnafu)?;
    let wrapper = SyncBuf {
        get_updates_buf: buf.to_string(),
    };
    let json = serde_json::to_string_pretty(&wrapper).context(JsonSnafu)?;
    std::fs::write(&path, json).context(IoSnafu)?;
    Ok(())
}

/// Global config file structure matching the Python SDK's `openclaw.json`.
#[derive(Debug, Deserialize)]
struct GlobalConfig {
    #[serde(default)]
    channels: HashMap<String, ChannelConfig>,
}

/// Channel-level configuration within the global config.
#[derive(Debug, Deserialize)]
struct ChannelConfig {
    #[serde(default)]
    accounts: HashMap<String, AccountSettingsInConfig>,
}

/// Per-account settings within a channel config.
#[derive(Debug, Deserialize)]
struct AccountSettingsInConfig {
    #[serde(default, rename = "routeTag")]
    route_tag: Option<String>,
}

/// Loads the optional per-account configuration from the global
/// `{state_dir}/openclaw.json` file, reading
/// `.channels["openclaw-weixin"].accounts["{raw_id}"].routeTag`.
pub fn get_account_config(account_id: &str) -> Option<AccountConfig> {
    let path = state_dir().join("openclaw.json");
    let data = std::fs::read_to_string(&path).ok()?;
    let config: GlobalConfig = serde_json::from_str(&data).ok()?;
    let raw_id = derive_raw_account_id(account_id);
    let route_tag = config
        .channels
        .get("openclaw-weixin")?
        .accounts
        .get(&raw_id)?
        .route_tag
        .clone();
    Some(AccountConfig { route_tag })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_account_data_serde() {
        let data = AccountData {
            token:    "tok_abc".to_string(),
            saved_at: "2025-01-01T00:00:00Z".to_string(),
            base_url: "https://example.com".to_string(),
            user_id:  "user_123".to_string(),
        };
        let json = serde_json::to_string(&data).unwrap();
        let deserialized: AccountData = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.token, data.token);
        assert_eq!(deserialized.saved_at, data.saved_at);
        assert_eq!(deserialized.base_url, data.base_url);
        assert_eq!(deserialized.user_id, data.user_id);
    }

    #[test]
    fn test_account_config_serde() {
        let config = AccountConfig {
            route_tag: Some("tag_xyz".to_string()),
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: AccountConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.route_tag, Some("tag_xyz".to_string()));
    }

    #[test]
    fn test_account_config_default_route_tag() {
        let json = r"{}";
        let config: AccountConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.route_tag, None);
    }

    #[test]
    fn test_normalize_account_id() {
        assert_eq!(
            normalize_account_id("  MyBot@Test.Com  "),
            "mybot-test-com"
        );
        assert_eq!(normalize_account_id("simple"), "simple");
        assert_eq!(normalize_account_id("A.B@C"), "a-b-c");
    }

    #[test]
    fn test_derive_raw_account_id() {
        assert_eq!(derive_raw_account_id("mybot"), "mybot");
    }

    #[test]
    fn test_sync_buf_json_format() {
        let wrapper = SyncBuf {
            get_updates_buf: "some-buf-value".to_string(),
        };
        let json = serde_json::to_string(&wrapper).unwrap();
        let deserialized: SyncBuf = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.get_updates_buf, "some-buf-value");
    }

    #[test]
    fn test_global_config_parsing() {
        let json = r#"{
            "channels": {
                "openclaw-weixin": {
                    "accounts": {
                        "mybot": {
                            "routeTag": "tag-123"
                        }
                    }
                }
            }
        }"#;
        let config: GlobalConfig = serde_json::from_str(json).unwrap();
        let route_tag = config
            .channels
            .get("openclaw-weixin")
            .unwrap()
            .accounts
            .get("mybot")
            .unwrap()
            .route_tag
            .as_ref()
            .unwrap();
        assert_eq!(route_tag, "tag-123");
    }
}
