//! 离线授权的密码学原语：Ed25519 验签 / HMAC-SHA256 / SHA-256 / Base64 / hex
//!
//! 设计要点：
//! - 授权码使用非对称签名（Ed25519）。私钥仅开发者持有，公钥编译进程序，
//!   程序可验证授权码真伪但无法伪造。
//! - 试用状态使用 HMAC-SHA256 做完整性校验，密钥由“机器指纹 + 内置胡椒”派生，
//!   换机器或改文件均会校验失败。

use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey, SIGNATURE_LENGTH};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// ⚠️ 内嵌的开发者公钥（32 字节）。由 keygen `gen-keys` 生成，私钥离线保管。
/// 从 keygen/public_key.bin 二进制文件在编译时嵌入。
const DEVELOPER_PUBLIC_KEY: [u8; 32] = *include_bytes!("../../keygen/public_key.bin");

/// 内置胡椒（混淆），用于派生 HMAC 密钥，抬高本地篡改门槛。
/// 非真正机密：被提取也只能伪造试用状态，不能伪造授权码。
const HMAC_PEPPER: &[u8] = b"umya-excel-v1-s3cr3t-pepper-CHANGE-ME";

/// SHA-256 摘要（小写十六进制）
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex(&hasher.finalize())
}

/// 用机器指纹派生 HMAC 密钥
fn derive_hmac_key(machine_fingerprint: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(HMAC_PEPPER);
    hasher.update(machine_fingerprint);
    let out = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out);
    key
}

/// 计算消息的 HMAC（小写十六进制）
pub fn hmac_hex(machine_fingerprint: &[u8], msg: &[u8]) -> String {
    let key = derive_hmac_key(machine_fingerprint);
    let mut mac = HmacSha256::new_from_slice(&key).expect("HMAC accepts any key length");
    mac.update(msg);
    hex(&mac.finalize().into_bytes())
}

/// 校验 HMAC（常量时间比较，防时序攻击）
pub fn hmac_verify(machine_fingerprint: &[u8], msg: &[u8], expected_hex: &str) -> bool {
    let actual = hmac_hex(machine_fingerprint, msg);
    if actual.len() != expected_hex.len() {
        return false;
    }
    let diff = actual
        .as_bytes()
        .iter()
        .zip(expected_hex.as_bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b));
    diff == 0
}

/// 用内嵌公钥验证 Ed25519 签名（授权码防伪造的核心）
pub fn ed25519_verify(msg: &[u8], sig_bytes: &[u8]) -> bool {
    if sig_bytes.len() != SIGNATURE_LENGTH {
        return false;
    }
    let Ok(vk) = VerifyingKey::from_bytes(&DEVELOPER_PUBLIC_KEY) else {
        return false;
    };
    let Ok(sig) = Signature::from_slice(sig_bytes) else {
        return false;
    };
    vk.verify(msg, &sig).is_ok()
}

/// Base64 编码（与 [`b64_decode`] 配对的公共工具，keygen 等场景使用）
#[allow(dead_code)]
pub fn b64_encode(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// Base64 解码
pub fn b64_decode(s: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(s).ok()
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

// ---------------------------------------------------------------------------
// AES-256-GCM 加密导出（--license 用）
// ---------------------------------------------------------------------------

/// 导出用加密密钥上下文标签（与 HMAC_PEPPER 不同，避免密钥复用）
const EXPORT_LABEL: &[u8] = b"umya-excel-license-export-v1";

/// 从机器指纹派生 AES-256 加密密钥（32 字节）。
///
/// 使用与 `derive_hmac_key` 相似的 SHA-256 派生，但加入不同的上下文标签，
/// 确保加密密钥与 HMAC 密钥完全不同。
fn derive_export_key(machine_fingerprint: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(EXPORT_LABEL);
    hasher.update(HMAC_PEPPER);
    hasher.update(machine_fingerprint);
    let out = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out);
    key
}

/// AES-256-GCM 加密。返回 `base64(nonce[12] || ciphertext || tag[16])`。
///
/// - `plaintext`：要加密的明文字节
/// - `machine_fingerprint`：机器指纹（派生密钥用，确保绑机）
///
/// 每次调用生成随机 nonce，同一明文产出不同密文。
pub fn aes256gcm_encrypt(plaintext: &[u8], machine_fingerprint: &[u8]) -> Option<String> {
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

    let key = derive_export_key(machine_fingerprint);
    let cipher = Aes256Gcm::new_from_slice(&key).ok()?;
    // 生成 12 字节随机 nonce
    let mut nonce_bytes = [0u8; 12];
    if getrandom::getrandom(&mut nonce_bytes).is_err() {
        return None;
    }
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, plaintext).ok()?;

    // 拼接 nonce + ciphertext_with_tag
    let mut out = Vec::with_capacity(12 + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);

    Some(base64::engine::general_purpose::STANDARD.encode(&out))
}

/// AES-256-GCM 解密。接受 `base64(nonce || ciphertext || tag)` 格式。
///
/// 成功返回明文，失败（密钥不匹配 / 篡改 / 格式错误）返回 `None`。
pub fn aes256gcm_decrypt(encoded: &str, machine_fingerprint: &[u8]) -> Option<Vec<u8>> {
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

    let data = b64_decode(encoded)?;
    if data.len() < 12 + 16 {
        // nonce(12) + 至少 tag(16)
        return None;
    }

    let key = derive_export_key(machine_fingerprint);
    let cipher = Aes256Gcm::new_from_slice(&key).ok()?;
    let nonce = Nonce::from_slice(&data[..12]);
    cipher.decrypt(nonce, &data[12..]).ok()
}

// ---------------------------------------------------------------------------
// 分位置（per-location）AES-256-GCM：内部多存储点用，密文按位置差异化
// ---------------------------------------------------------------------------

/// 分位置加密的上下文标签（与 [`EXPORT_LABEL`]、[`HMAC_PEPPER`] 互不相同，
/// 避免三套密钥派生路径之间的密钥复用）。
const LOCATION_LABEL: &[u8] = b"umya-excel-license-store-v1";

/// 从机器指纹 + 存储位置 tag 派生 AES-256 密钥（32 字节）。
///
/// 每个 tag 对应不同密钥：同一明文在不同存储点产出不同密文，
/// 且无法把 A 点的密文搬到 B 点解密（抗重定位 / 抗按内容批量定位）。
fn derive_location_key(machine_fingerprint: &[u8], location_tag: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(LOCATION_LABEL);
    hasher.update(HMAC_PEPPER);
    hasher.update(machine_fingerprint);
    hasher.update(location_tag);
    let out = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out);
    key
}

/// 分位置 AES-256-GCM 加密。返回 `base64(nonce[12] || ciphertext || tag[16])`。
///
/// - `plaintext`：明文字节
/// - `machine_fingerprint`：机器指纹（绑机）
/// - `location_tag`：存储位置标识（决定派生密钥）
///
/// 每次随机 nonce → 同明文多次/多点产出不同密文；不同 tag → 不同密钥。
pub fn aes256gcm_encrypt_for(
    plaintext: &[u8],
    machine_fingerprint: &[u8],
    location_tag: &[u8],
) -> Option<String> {
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

    let key = derive_location_key(machine_fingerprint, location_tag);
    let cipher = Aes256Gcm::new_from_slice(&key).ok()?;
    let mut nonce_bytes = [0u8; 12];
    if getrandom::getrandom(&mut nonce_bytes).is_err() {
        return None;
    }
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, plaintext).ok()?;

    let mut out = Vec::with_capacity(12 + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);

    Some(base64::engine::general_purpose::STANDARD.encode(&out))
}

/// 分位置 AES-256-GCM 解密。与 [`aes256gcm_encrypt_for`] 配对，
/// 必须用**相同的 location_tag** 才能解密成功。
pub fn aes256gcm_decrypt_for(
    encoded: &str,
    machine_fingerprint: &[u8],
    location_tag: &[u8],
) -> Option<Vec<u8>> {
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

    let data = b64_decode(encoded)?;
    if data.len() < 12 + 16 {
        return None;
    }

    let key = derive_location_key(machine_fingerprint, location_tag);
    let cipher = Aes256Gcm::new_from_slice(&key).ok()?;
    let nonce = Nonce::from_slice(&data[..12]);
    cipher.decrypt(nonce, &data[12..]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64_roundtrip() {
        let data: &[u8] = b"hello \x00\xff license";
        assert_eq!(b64_decode(&b64_encode(data)), Some(data.to_vec()));
    }

    #[test]
    fn hmac_sign_and_verify() {
        let fp: &[u8] = b"fingerprint-bytes";
        let msg: &[u8] = b"message body";
        let mac = hmac_hex(fp, msg);
        assert!(hmac_verify(fp, msg, &mac), "valid mac should verify");
        assert!(!hmac_verify(fp, b"tampered", &mac), "tampered msg rejected");
        assert!(!hmac_verify(b"other-fp", msg, &mac), "different fingerprint rejected");
    }

    #[test]
    fn ed25519_rejects_garbage() {
        // 全零 / 随便造的 64 字节都不应通过验签
        assert!(!ed25519_verify(b"msg", &[0u8; SIGNATURE_LENGTH]));
        // 长度不对直接拒绝
        assert!(!ed25519_verify(b"msg", &[0u8; 10]));
    }

    #[test]
    fn aes256gcm_roundtrip() {
        let fp: &[u8] = b"test-fingerprint-bytes";
        let plaintext = b"f=20622|l=20622|r=0|mac=abc123";
        let encoded = aes256gcm_encrypt(plaintext, fp).expect("encrypt should succeed");
        let decoded = aes256gcm_decrypt(&encoded, fp).expect("decrypt should succeed");
        assert_eq!(decoded, plaintext);
    }

    #[test]
    fn aes256gcm_wrong_key_fails() {
        let fp1: &[u8] = b"machine-A";
        let fp2: &[u8] = b"machine-B";
        let plaintext = b"secret data";
        let encoded = aes256gcm_encrypt(plaintext, fp1).expect("encrypt");
        assert!(aes256gcm_decrypt(&encoded, fp2).is_none(), "wrong machine key must fail");
    }

    #[test]
    fn aes256gcm_tampered_fails() {
        let fp: &[u8] = b"test-fp";
        let plaintext = b"secret data";
        let encoded = aes256gcm_encrypt(plaintext, fp).expect("encrypt");
        // 篡改 base64 串中的字符
        let bytes = encoded.into_bytes();
        if bytes.len() > 10 {
            let mut tampered = bytes;
            tampered[10] = if tampered[10] == b'A' { b'B' } else { b'A' };
            let tampered_str = String::from_utf8(tampered).unwrap_or_default();
            assert!(
                aes256gcm_decrypt(&tampered_str, fp).is_none(),
                "tampered ciphertext must not decrypt successfully"
            );
        }
    }

    #[test]
    fn aes256gcm_for_roundtrip() {
        let fp: &[u8] = b"test-fingerprint-bytes";
        let plaintext = b"f=20622|l=20622|r=0|mac=abc123|loc=home|mani=zzz";
        let encoded = aes256gcm_encrypt_for(plaintext, fp, b"home").expect("encrypt_for");
        let decoded = aes256gcm_decrypt_for(&encoded, fp, b"home").expect("decrypt_for");
        assert_eq!(decoded, plaintext);
    }

    #[test]
    fn aes256gcm_for_tag_isolation() {
        // home 点的密文不能用 config 的 tag 解密（密钥不同）→ 抗重定位
        let fp: &[u8] = b"machine-fp";
        let plaintext = b"secret trial state";
        let encoded = aes256gcm_encrypt_for(plaintext, fp, b"home").expect("encrypt_for home");
        assert!(
            aes256gcm_decrypt_for(&encoded, fp, b"config").is_none(),
            "blob encrypted under 'home' must not decrypt under 'config'"
        );
    }

    #[test]
    fn aes256gcm_for_distinct_ciphertext_per_call() {
        // 同明文、同 tag 两次加密，因随机 nonce 应产出不同密文
        let fp: &[u8] = b"machine-fp";
        let plaintext = b"identical plaintext";
        let a = aes256gcm_encrypt_for(plaintext, fp, b"home").expect("encrypt a");
        let b = aes256gcm_encrypt_for(plaintext, fp, b"home").expect("encrypt b");
        assert_ne!(a, b, "two encryptions of same plaintext must differ (random nonce)");
    }

    #[test]
    fn aes256gcm_for_distinct_ciphertext_per_tag() {
        // 同明文、不同 tag 应产出不同密文
        let fp: &[u8] = b"machine-fp";
        let plaintext = b"identical plaintext";
        let home = aes256gcm_encrypt_for(plaintext, fp, b"home").expect("encrypt home");
        let config = aes256gcm_encrypt_for(plaintext, fp, b"config").expect("encrypt config");
        assert_ne!(home, config, "ciphertext must differ per location tag");
    }

    #[test]
    fn aes256gcm_for_distinct_from_legacy() {
        // 分位置密文应与无 tag 旧版密文不同（不同标签 / 不同派生）
        let fp: &[u8] = b"machine-fp";
        let plaintext = b"identical plaintext";
        let legacy = aes256gcm_encrypt(plaintext, fp).expect("legacy encrypt");
        let tagged = aes256gcm_encrypt_for(plaintext, fp, b"home").expect("tagged encrypt");
        assert_ne!(legacy, tagged, "tagged ciphertext must differ from legacy");
        // 旧版密文不能被分位置解密（兜底时需走无 tag 路径）
        assert!(
            aes256gcm_decrypt_for(&legacy, fp, b"home").is_none(),
            "legacy blob must not decrypt via tagged path"
        );
    }
}
