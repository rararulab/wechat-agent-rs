use std::path::{Path, PathBuf};

use aes::Aes128;
use base64::Engine as _;
use block_padding::Pkcs7;
use cipher::{BlockDecryptMut as _, BlockEncryptMut as _, KeyInit};
use ecb;
use reqwest::Client;
use serde_json::Value;
use snafu::ResultExt;

use crate::{
    api::WeixinApiClient,
    errors::{ApiSnafu, EncryptionSnafu, HttpSnafu, IoSnafu},
    models::MediaType,
    storage::CDN_BASE_URL,
};

type Aes128EcbEnc = ecb::Encryptor<Aes128>;
type Aes128EcbDec = ecb::Decryptor<Aes128>;

const MEDIA_DIR: &str = "/tmp/weixin-agent/media";

/// Upload media type: image.
pub const UPLOAD_MEDIA_IMAGE: u8 = 1;
/// Upload media type: video.
pub const UPLOAD_MEDIA_VIDEO: u8 = 2;
/// Upload media type: file.
pub const UPLOAD_MEDIA_FILE: u8 = 3;
/// Upload media type: voice.
pub const UPLOAD_MEDIA_VOICE: u8 = 4;

const MAX_UPLOAD_RETRIES: u8 = 3;

/// Result of uploading a media file to the CDN.
pub struct UploadResult {
    /// The encrypted query parameter for constructing download URLs.
    pub encrypt_query_param: String,
    /// The AES key as base64-encoded hex string.
    pub aes_key: String,
    /// The original file name.
    pub file_name: String,
    /// The original file size in bytes.
    pub file_size: u64,
}

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

/// Parses an AES-128 key from a hex string or base64-encoded string.
///
/// Supports three formats:
/// - Direct 32-character hex string (decodes to 16 bytes)
/// - Base64-encoded 16 raw bytes
/// - Base64-encoded 32-character hex string (decoded recursively)
pub fn parse_aes_key(key_str: &str) -> crate::Result<[u8; 16]> {
    // Try direct hex decode first (32-char hex string)
    if key_str.len() == 32
        && let Ok(bytes) = hex::decode(key_str)
        && let Ok(arr) = <[u8; 16]>::try_from(bytes.as_slice())
    {
        return Ok(arr);
    }
    // Try base64 decode
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(key_str)
        .map_err(|e| {
            EncryptionSnafu {
                reason: format!("invalid key encoding: {e}"),
            }
            .build()
        })?;
    if decoded.len() == 16 {
        return decoded.try_into().map_err(|_| {
            EncryptionSnafu {
                reason: "AES key must be 16 bytes".to_owned(),
            }
            .build()
        });
    }
    if decoded.len() == 32 {
        let hex_str = std::str::from_utf8(&decoded).map_err(|e| {
            EncryptionSnafu {
                reason: format!("invalid hex in base64: {e}"),
            }
            .build()
        })?;
        // Recurse to hex-decode the inner string
        return parse_aes_key(hex_str);
    }
    Err(EncryptionSnafu {
        reason: format!("unexpected key length: {}", decoded.len()),
    }
    .build())
}

/// Calculates the AES-ECB padded ciphertext size for a given plaintext size.
pub const fn aes_ecb_padded_size(size: u64) -> u64 {
    ((size / 16) + 1) * 16
}

/// Downloads and decrypts a media file from the `WeChat` CDN.
///
/// Uses the `encrypted_query_param` URL pattern to construct the download URL.
/// Returns the local filesystem path where the decrypted file was saved.
pub async fn download_media(
    encrypt_query_param: &str,
    aes_key_str: &str,
    file_name: Option<&str>,
    subdir: &str,
) -> crate::Result<PathBuf> {
    let key = parse_aes_key(aes_key_str)?;
    let encoded = urlencoding::encode(encrypt_query_param);
    let url = format!("{CDN_BASE_URL}/download?encrypted_query_param={encoded}");
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

    let dir = Path::new(MEDIA_DIR).join(subdir);
    std::fs::create_dir_all(&dir).context(IoSnafu)?;

    let name = file_name.unwrap_or("download");
    let path = dir.join(format!("{}_{}", uuid::Uuid::new_v4(), name));
    std::fs::write(&path, &decrypted).context(IoSnafu)?;
    Ok(path)
}

/// Extracts the `encrypt_query_param` and `aes_key` from a media sub-item JSON node.
fn extract_media_fields<'a>(
    sub_item: &'a Value,
    type_name: &str,
) -> crate::Result<(&'a str, &'a str)> {
    let eqp = sub_item["media"]["encrypt_query_param"]
        .as_str()
        .ok_or_else(|| {
            ApiSnafu {
                code:    -1_i64,
                message: format!("missing encrypt_query_param for {type_name}"),
            }
            .build()
        })?;
    let key = sub_item["media"]["aes_key"].as_str().ok_or_else(|| {
        ApiSnafu {
            code:    -1_i64,
            message: format!("missing aes key for {type_name}"),
        }
        .build()
    })?;
    Ok((eqp, key))
}

/// Downloads media from an incoming message item, handling per-type field structures.
///
/// Extracts the appropriate fields based on `item_type`:
/// - IMAGE (type=2): `image_item.media.encrypt_query_param` + hex/base64 aes key
/// - VOICE (type=3): `voice_item.media.encrypt_query_param` + `voice_item.media.aes_key`
/// - FILE (type=4): `file_item.media.encrypt_query_param` + `file_item.media.aes_key` + file name
/// - VIDEO (type=5): `video_item.media.encrypt_query_param` + `video_item.media.aes_key`
///
/// Returns `(path, media_type, mime_type, file_name)`.
pub async fn download_media_from_item(
    item: &Value,
    item_type: u64,
) -> crate::Result<(PathBuf, MediaType, String, Option<String>)> {
    let (encrypt_query_param, aes_key_str, file_name, media_type, subdir) = match item_type {
        2 => {
            let image_item = &item["image_item"];
            let eqp = image_item["media"]["encrypt_query_param"]
                .as_str()
                .ok_or_else(|| {
                    ApiSnafu {
                        code:    -1_i64,
                        message: "missing encrypt_query_param for image".to_owned(),
                    }
                    .build()
                })?;
            // Image: try hex key first (aeskey field), then base64 (media.aes_key)
            let key = image_item["aeskey"]
                .as_str()
                .or_else(|| image_item["media"]["aes_key"].as_str())
                .ok_or_else(|| {
                    ApiSnafu {
                        code:    -1_i64,
                        message: "missing aes key for image".to_owned(),
                    }
                    .build()
                })?;
            (eqp, key, None, MediaType::Image, "image")
        }
        3 => {
            let (eqp, key) = extract_media_fields(&item["voice_item"], "voice")?;
            (eqp, key, None, MediaType::Audio, "voice")
        }
        4 => {
            let (eqp, key) = extract_media_fields(&item["file_item"], "file")?;
            let fname = item["file_item"]["file_name"].as_str().map(String::from);
            (eqp, key, fname, MediaType::File, "file")
        }
        5 => {
            let (eqp, key) = extract_media_fields(&item["video_item"], "video")?;
            (eqp, key, None, MediaType::Video, "video")
        }
        _ => {
            return Err(ApiSnafu {
                code:    -1_i64,
                message: format!("unsupported media item_type: {item_type}"),
            }
            .build());
        }
    };

    let file_name_ref = file_name.as_deref();
    let path = download_media(encrypt_query_param, aes_key_str, file_name_ref, subdir).await?;
    let mime = mime_guess::from_path(&path)
        .first_or_octet_stream()
        .to_string();
    Ok((path, media_type, mime, file_name))
}

/// Encrypts and uploads a local file to the `WeChat` CDN.
///
/// Follows the Python SDK upload flow:
/// 1. Read file, compute raw size + MD5
/// 2. Generate random filekey (16 bytes hex) and AES key (16 bytes)
/// 3. Request a pre-signed upload URL from the API
/// 4. AES-ECB encrypt the file data
/// 5. POST encrypted data to the CDN
/// 6. Extract `x-encrypted-param` header from response
/// 7. Return [`UploadResult`] with the encrypted query param and metadata
///
/// Retries up to 3 times on non-4xx failures.
pub async fn upload_media(
    api_client: &WeixinApiClient,
    file_path: &Path,
    media_type: u8,
    to_user_id: &str,
) -> crate::Result<UploadResult> {
    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let data = std::fs::read(file_path).context(IoSnafu)?;
    let raw_size = data.len() as u64;
    let raw_md5 = format!("{:x}", md5::compute(&data));

    // Generate random filekey and AES key
    let filekey_bytes: [u8; 16] = rand::random();
    let filekey = hex::encode(filekey_bytes);
    let aes_key: [u8; 16] = rand::random();
    let aes_key_hex = hex::encode(aes_key);

    let file_size = aes_ecb_padded_size(raw_size);

    let upload_info = api_client
        .get_upload_url(
            &filekey,
            media_type,
            to_user_id,
            raw_size,
            &raw_md5,
            file_size,
            &aes_key_hex,
        )
        .await?;
    let upload_url = upload_info["data"]["upload_url"]
        .as_str()
        .ok_or_else(|| {
            ApiSnafu {
                code:    -1_i64,
                message: "no upload_url in response".to_owned(),
            }
            .build()
        })?;

    let encrypted = encrypt_aes_ecb(&aes_key, &data);
    let client = Client::new();

    // Retry loop: up to MAX_UPLOAD_RETRIES attempts, no retry on 4xx
    let mut last_err = None;
    for _ in 0..MAX_UPLOAD_RETRIES {
        let resp = client
            .post(upload_url)
            .header("Content-Type", "application/octet-stream")
            .body(encrypted.clone())
            .send()
            .await;

        match resp {
            Ok(response) => {
                let status = response.status();
                if status.is_client_error() {
                    return Err(ApiSnafu {
                        code:    i64::from(status.as_u16()),
                        message: format!("CDN upload failed with {status}"),
                    }
                    .build());
                }
                if !status.is_success() {
                    last_err = Some(
                        ApiSnafu {
                            code:    i64::from(status.as_u16()),
                            message: format!("CDN upload failed with {status}"),
                        }
                        .build(),
                    );
                    continue;
                }
                // Extract x-encrypted-param header
                let encrypt_query_param = response
                    .headers()
                    .get("x-encrypted-param")
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| {
                        ApiSnafu {
                            code:    -1_i64,
                            message: "missing x-encrypted-param header in CDN response".to_owned(),
                        }
                        .build()
                    })?
                    .to_string();

                // Base64-encode the hex key string for the result
                let aes_key_b64 =
                    base64::engine::general_purpose::STANDARD.encode(aes_key_hex.as_bytes());

                return Ok(UploadResult {
                    encrypt_query_param,
                    aes_key: aes_key_b64,
                    file_name: file_name.to_string(),
                    file_size: raw_size,
                });
            }
            Err(e) => {
                last_err = Some(crate::Error::Http { source: e });
            }
        }
    }

    Err(last_err.unwrap_or_else(|| {
        ApiSnafu {
            code:    -1_i64,
            message: "upload failed after retries".to_owned(),
        }
        .build()
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
    fn test_parse_aes_key_direct_hex() {
        let key = parse_aes_key("00112233445566778899aabbccddeeff").unwrap();
        assert_eq!(
            key,
            [
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc,
                0xdd, 0xee, 0xff
            ]
        );
    }

    #[test]
    fn test_parse_aes_key_base64_hex() {
        let b64 =
            base64::engine::general_purpose::STANDARD.encode("00112233445566778899aabbccddeeff");
        let key = parse_aes_key(&b64).unwrap();
        assert_eq!(
            key,
            [
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc,
                0xdd, 0xee, 0xff
            ]
        );
    }

    #[test]
    fn test_parse_aes_key_base64_raw() {
        let raw_key = [0x42u8; 16];
        let b64 = base64::engine::general_purpose::STANDARD.encode(raw_key);
        let key = parse_aes_key(&b64).unwrap();
        assert_eq!(key, raw_key);
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

    #[test]
    fn test_aes_ecb_padded_size() {
        assert_eq!(aes_ecb_padded_size(0), 16);
        assert_eq!(aes_ecb_padded_size(1), 16);
        assert_eq!(aes_ecb_padded_size(16), 32);
        assert_eq!(aes_ecb_padded_size(17), 32);
    }
}
