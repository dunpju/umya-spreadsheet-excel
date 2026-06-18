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
pub const DEVELOPER_PUBLIC_KEY: [u8; 32] = [
    0x08, 0x27, 0xd0, 0x78, 0x55, 0xcd, 0x5f, 0x5b, 0x45, 0x79, 0x23, 0x28, 0x3c, 0xe4, 0x95, 0x15,
    0x35, 0x64, 0x23, 0x05, 0xf1, 0x80, 0xb4, 0xff, 0x98, 0xc8, 0x58, 0x55, 0x75, 0x87, 0x8e, 0x94,
];

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
}
