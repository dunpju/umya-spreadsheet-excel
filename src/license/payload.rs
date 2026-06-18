//! 授权负载：被 Ed25519 签名的内容。
//!
//! 采用手工文本编码（固定字段顺序 + 分隔符），保证签名/验签字节完全一致，
//! 不依赖 serde-derive，与项目现有风格一致。

/// 产品标识，防止别的产品授权码串用
pub const PRODUCT_ID: &str = "umya-excel";

/// 试用期天数
pub const TRIAL_DAYS: u64 = 0;

/// 0 表示永久授权
pub const EXPIRES_NEVER: u64 = 0;

/// 负载版本，便于将来升级编码格式
#[allow(dead_code)]
pub const LICENSE_VERSION: u32 = 1;

#[derive(Clone, Debug)]
pub struct LicensePayload {
    /// 负载版本
    pub version: u32,
    /// 产品标识
    pub product: String,
    /// 绑定的机器码（与本地指纹比对）
    pub machine: String,
    /// 签发日（epoch 天）
    pub issued_day: u64,
    /// 到期日（epoch 天），0 = 永久
    pub expires_day: u64,
    /// 版本/功能位，如 "pro"
    pub edition: String,
    /// 客户名（可选，便于核对）
    pub customer: String,
}

impl LicensePayload {
    /// 规范化明文：签名与编码都基于这串字节。
    /// 关键：行序、分隔符固定，绝不能在生成后改变格式（否则旧授权码全部失效）。
    #[allow(dead_code)]
    pub fn to_text(&self) -> String {
        format!(
            "v={}\np={}\nm={}\ni={}\ne={}\ned={}\nc={}\n",
            self.version,
            self.product,
            self.machine,
            self.issued_day,
            self.expires_day,
            self.edition,
            self.customer,
        )
    }

    /// 解析明文（验签通过后调用）。任一关键字段缺失返回 None。
    pub fn parse(text: &str) -> Option<Self> {
        let mut p = LicensePayload {
            version: 0,
            product: String::new(),
            machine: String::new(),
            issued_day: 0,
            expires_day: 0,
            edition: String::new(),
            customer: String::new(),
        };
        for line in text.lines() {
            let (k, v) = line.split_once('=')?;
            match k {
                "v" => p.version = v.parse().ok()?,
                "p" => p.product = v.to_string(),
                "m" => p.machine = v.to_string(),
                "i" => p.issued_day = v.parse().ok()?,
                "e" => p.expires_day = v.parse().ok()?,
                "ed" => p.edition = v.to_string(),
                "c" => p.customer = v.to_string(),
                _ => {}
            }
        }
        if p.product.is_empty() || p.machine.is_empty() {
            return None;
        }
        Some(p)
    }

    /// 是否已到期（基于当前 epoch 天）
    pub fn is_expired(&self, today: u64) -> bool {
        self.expires_day != EXPIRES_NEVER && today >= self.expires_day
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> LicensePayload {
        LicensePayload {
            version: LICENSE_VERSION,
            product: PRODUCT_ID.to_string(),
            machine: "ABCD-EFGH-IJKL-MNOP".to_string(),
            issued_day: 20_000,
            expires_day: EXPIRES_NEVER,
            edition: "pro".to_string(),
            customer: "Acme".to_string(),
        }
    }

    #[test]
    fn text_parse_roundtrip() {
        let p = sample();
        let p2 = LicensePayload::parse(&p.to_text()).expect("parse");
        assert_eq!(p2.version, p.version);
        assert_eq!(p2.product, p.product);
        assert_eq!(p2.machine, p.machine);
        assert_eq!(p2.expires_day, EXPIRES_NEVER);
        assert_eq!(p2.customer, p.customer);
    }

    #[test]
    fn perpetual_never_expires() {
        let p = sample();
        assert!(!p.is_expired(u64::MAX));
    }

    #[test]
    fn expiry_boundary() {
        let mut p = sample();
        p.expires_day = 200;
        assert!(!p.is_expired(199));
        assert!(p.is_expired(200));
    }

    #[test]
    fn parse_rejects_incomplete() {
        assert!(LicensePayload::parse("v=1\n").is_none());
        assert!(LicensePayload::parse("").is_none());
    }
}
