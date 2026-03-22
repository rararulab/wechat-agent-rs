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
