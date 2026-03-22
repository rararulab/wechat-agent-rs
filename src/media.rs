use std::path::{Path, PathBuf};

use aes::Aes128;
use block_padding::Pkcs7;
use cipher::{BlockDecryptMut as _, BlockEncryptMut as _, KeyInit};
use ecb;
use reqwest::Client;
use serde_json::Value;
use snafu::ResultExt;

use crate::{
    api::WeixinApiClient,
    errors::{ApiSnafu, EncryptionSnafu, HttpSnafu, IoSnafu},
    storage::CDN_BASE_URL,
};

type Aes128EcbEnc = ecb::Encryptor<Aes128>;
type Aes128EcbDec = ecb::Decryptor<Aes128>;

const MEDIA_DIR: &str = "/tmp/weixin-agent/media";

/// Encrypts `data` using AES-128 in ECB mode with PKCS7 padding.
pub fn encrypt_aes_ecb(key: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let enc = Aes128EcbEnc::new(key.into());
    enc.encrypt_padded_vec_mut::<Pkcs7>(data)
}

/// Decrypts `data` using AES-128 in ECB mode with PKCS7 padding.
pub fn decrypt_aes_ecb(key: &[u8; 16], data: &[u8]) -> crate::Result<Vec<u8>> {
    let dec = Aes128EcbDec::new(key.into());
    dec.decrypt_padded_vec_mut::<Pkcs7>(data).map_err(|e| {
        EncryptionSnafu {
            reason: e.to_string(),
        }
        .build()
    })
}

/// Parses a hex-encoded AES-128 key string into a 16-byte array.
pub fn parse_aes_key(hex_key: &str) -> crate::Result<[u8; 16]> {
    let bytes = hex::decode(hex_key).map_err(|e| {
        EncryptionSnafu {
            reason: format!("invalid hex key: {e}"),
        }
        .build()
    })?;
    bytes.try_into().map_err(|_| {
        EncryptionSnafu {
            reason: "AES key must be 16 bytes".to_owned(),
        }
        .build()
    })
}

/// Downloads and decrypts a media file from the `WeChat` CDN.
///
/// Returns the local filesystem path where the decrypted file was saved.
pub async fn download_media(
    file_key: &str,
    aes_key_hex: &str,
    file_name: Option<&str>,
) -> crate::Result<PathBuf> {
    let key = parse_aes_key(aes_key_hex)?;
    let url = format!("{CDN_BASE_URL}/{file_key}");
    let client = Client::new();
    let encrypted_bytes = client
        .get(&url)
        .send()
        .await
        .context(HttpSnafu)?
        .bytes()
        .await
        .context(HttpSnafu)?;
    let decrypted = decrypt_aes_ecb(&key, &encrypted_bytes)?;

    let dir = Path::new(MEDIA_DIR);
    std::fs::create_dir_all(dir).context(IoSnafu)?;

    let name = file_name.unwrap_or("download");
    let path = dir.join(format!("{}_{}", uuid::Uuid::new_v4(), name));
    std::fs::write(&path, &decrypted).context(IoSnafu)?;
    Ok(path)
}

/// Encrypts and uploads a local file to the `WeChat` CDN.
///
/// Returns a JSON object containing the `filekey`, `aes_key`, and metadata
/// needed to reference the uploaded file in a message.
pub async fn upload_media(api_client: &WeixinApiClient, file_path: &Path) -> crate::Result<Value> {
    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let data = std::fs::read(file_path).context(IoSnafu)?;
    let file_size = data.len() as u64;

    let key: [u8; 16] = rand::random();
    let aes_key_hex = hex::encode(key);
    let encrypted = encrypt_aes_ecb(&key, &data);

    let upload_info = api_client.get_upload_url(file_name, file_size).await?;
    let upload_url = upload_info["data"]["upload_url"].as_str().ok_or_else(|| {
        ApiSnafu {
            code:    -1_i64,
            message: "no upload_url in response".to_owned(),
        }
        .build()
    })?;
    let file_key = upload_info["data"]["file_key"]
        .as_str()
        .unwrap_or("")
        .to_string();

    let client = Client::new();
    client
        .put(upload_url)
        .body(encrypted)
        .send()
        .await
        .context(HttpSnafu)?;

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

    #[test]
    fn test_aes_ecb_empty_data() {
        let key = [0xAAu8; 16];
        let plaintext = b"";
        let encrypted = encrypt_aes_ecb(&key, plaintext);
        let decrypted = decrypt_aes_ecb(&key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext.to_vec());
    }

    #[test]
    fn test_aes_ecb_large_data() {
        let key = [0xBBu8; 16];
        let plaintext = vec![0x42u8; 10 * 1024];
        let encrypted = encrypt_aes_ecb(&key, &plaintext);
        let decrypted = decrypt_aes_ecb(&key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_aes_ecb_block_aligned() {
        let key = [0xCCu8; 16];
        let plaintext = [0xDDu8; 16];
        let encrypted = encrypt_aes_ecb(&key, &plaintext);
        let decrypted = decrypt_aes_ecb(&key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext.to_vec());
    }

    #[test]
    fn test_parse_aes_key_valid() {
        let hex_key = "00112233445566778899aabbccddeeff";
        let key = parse_aes_key(hex_key).unwrap();
        assert_eq!(
            key,
            [
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff
            ]
        );
    }

    #[test]
    fn test_parse_aes_key_invalid_hex() {
        let result = parse_aes_key("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, crate::Error::Encryption { .. }),
            "expected Encryption error, got: {err}"
        );
    }

    #[test]
    fn test_parse_aes_key_wrong_length() {
        // 24 hex chars = 12 bytes, not 16
        let result = parse_aes_key("aabbccddeeff00112233aabb");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, crate::Error::Encryption { .. }),
            "expected Encryption error, got: {err}"
        );
    }

    #[test]
    fn test_decrypt_invalid_data() {
        let key = [0xEEu8; 16];
        let garbage = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        let result = decrypt_aes_ecb(&key, &garbage);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, crate::Error::Encryption { .. }),
            "expected Encryption error, got: {err}"
        );
    }
}
