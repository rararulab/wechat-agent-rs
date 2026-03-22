use crate::api::WeixinApiClient;
use crate::storage::CDN_BASE_URL;
use aes::Aes128;
use cipher::{BlockDecryptMut as _, BlockEncryptMut as _, KeyInit};
use block_padding::Pkcs7;
use reqwest::Client;
use serde_json::Value;
use std::path::{Path, PathBuf};

type Aes128EcbEnc = ecb::Encryptor<Aes128>;
type Aes128EcbDec = ecb::Decryptor<Aes128>;

const MEDIA_DIR: &str = "/tmp/weixin-agent/media";

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

    let key: [u8; 16] = rand::random();
    let aes_key_hex = hex::encode(key);
    let encrypted = encrypt_aes_ecb(&key, &data);

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

    let client = Client::new();
    client.put(upload_url).body(encrypted).send().await?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aes_ecb_roundtrip() {
        let key = [0x42u8; 16];
        let plaintext = b"Hello, WeChat media encryption!";
        let encrypted = encrypt_aes_ecb(&key, plaintext);
        let decrypted = decrypt_aes_ecb(&key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext.to_vec());
    }
}
