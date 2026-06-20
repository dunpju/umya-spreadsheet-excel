//! 离线授权对外门面：`LicenseManager` + `LicenseStatus` + `ActivateError`。
//!
//! 典型用法：
//! ```ignore
//! let mut lic = LicenseManager::load();        // 启动加载
//! let status = lic.status(license::time::today_epoch_day());
//! if status.is_blocking() { /* 弹激活窗 */ }
//! lic.activate(code, today)?;                   // 用户激活
//! lic.checkpoint(today);                        // 正常运行时推进高水位
//! ```

pub mod crypto;
pub mod fingerprint;
pub mod payload;
pub mod store;
pub mod time;

use payload::{LicensePayload, PRODUCT_ID, TRIAL_DAYS};
use store::TrialState;

/// 时钟回拨容忍天数（跨时区 / 夏令时微小抖动）
const ROLLBACK_TOLERANCE_DAYS: u64 = 2;

/// 授权状态：启动时由 `LicenseManager` 计算，UI 据此决定是否放行
#[derive(Debug, Clone, PartialEq)]
pub enum LicenseStatus {
    /// 试用中，剩余天数
    Trial { days_left: i64 },
    /// 试用期已结束，必须激活
    TrialExpired,
    /// 已激活；`None` = 永久，`Some` = 剩余天数
    Licensed { days_left: Option<i64> },
    /// 授权已到期（限时授权），需续期
    LicensedExpired,
    /// 检测到篡改（签名失败 / 时钟回拨 / 存储冲突），锁定
    Tampered,
}

impl LicenseStatus {
    /// 是否拦截：到期 / 篡改等需要弹激活窗、阻止使用的状态
    pub fn is_blocking(&self) -> bool {
        matches!(
            self,
            Self::TrialExpired | Self::LicensedExpired | Self::Tampered
        )
    }
}

/// 激活失败原因
#[derive(Debug, Clone, PartialEq)]
pub enum ActivateError {
    /// 授权码格式不对
    Format,
    /// 签名校验失败（伪造 / 损坏）
    InvalidSig,
    /// 机器码不匹配（授权码不是给本机的）
    MachineMismatch,
    /// 产品标识不匹配
    ProductMismatch,
    /// 签发即已过期
    AlreadyExpired,
}

impl ActivateError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::Format => "授权码格式不正确",
            Self::InvalidSig => "授权码无效（签名校验失败）",
            Self::MachineMismatch => "授权码与当前机器不匹配",
            Self::ProductMismatch => "授权码产品不匹配",
            Self::AlreadyExpired => "授权码已过期，请联系开发者续期",
        }
    }
}

pub struct LicenseManager {
    /// 用户可见机器码（发给开发者）
    machine_code: String,
    /// 机器指纹（HMAC 密钥派生用）
    machine_fp: Vec<u8>,
    /// 已验证的授权负载
    license: Option<LicensePayload>,
    /// 授权码原始字符串（持久化用）
    license_raw: Option<String>,
    /// 试用状态
    trial: Option<TrialState>,
    /// 是否检测到存储篡改
    tampered: bool,
}

impl LicenseManager {
    /// 启动时调用：读取本地状态、校验签名、检测时钟回拨、得到状态
    pub fn load() -> Self {
        let machine_code = fingerprint::machine_code();
        let machine_fp = fingerprint::fingerprint_bytes();
        let today = time::today_epoch_day();

        let lr = store::load(&machine_fp);
        let license = lr
            .license_raw
            .as_ref()
            .and_then(|s| verify_license(s, &machine_code));

        let trial = match lr.trial {
            Some(mut t) => {
                // 时钟回拨检测（仅计数，不下调高水位）
                if t.last_run_day > today + ROLLBACK_TOLERANCE_DAYS {
                    t.rollback_count = t.rollback_count.saturating_add(1);
                }
                // 主动自愈：非篡改且部分存储点缺失（含 LicenseBlob）→ 立即重写全部存储点补齐冗余。
                // 堵掉"同天反复删点逐步侵蚀冗余"的窗口（否则要等到次日跨天 checkpoint 才补）。
                if lr.needs_heal {
                    t.sign(&machine_fp);
                    store::save(&t, &lr.license_raw, &machine_fp);
                }
                Some(t)
            }
            None => {
                if !lr.tampered {
                    // 首次运行：初始化试用状态并落盘
                    let mut t = TrialState {
                        first_run_day: today,
                        last_run_day: today,
                        rollback_count: 0,
                        mac: String::new(),
                    };
                    t.sign(&machine_fp);
                    store::save(&t, &lr.license_raw, &machine_fp);
                    Some(t)
                } else {
                    None
                }
            }
        };

        Self {
            machine_code,
            machine_fp,
            license,
            license_raw: lr.license_raw,
            trial,
            tampered: lr.tampered,
        }
    }

    /// 计算当前授权状态
    pub fn status(&self, today: u64) -> LicenseStatus {
        // 已激活优先
        if let Some(lic) = &self.license {
            if lic.is_expired(today) {
                return LicenseStatus::LicensedExpired;
            }
            let days_left = if lic.expires_day == payload::EXPIRES_NEVER {
                None
            } else {
                Some(lic.expires_day as i64 - today as i64)
            };
            return LicenseStatus::Licensed { days_left };
        }

        // 无有效授权
        if self.tampered {
            return LicenseStatus::Tampered;
        }

        match &self.trial {
            None => {
                // 既无 trial 也无 license 且非篡改：load 时本应初始化 trial，
                // 到此说明 home 目录不可写等异常，保守按到期处理
                LicenseStatus::TrialExpired
            }
            Some(t) => {
                // 时钟回拨：高水位远超当前时间 → 锁定
                if t.last_run_day > today + ROLLBACK_TOLERANCE_DAYS {
                    return LicenseStatus::Tampered;
                }
                let expire_day = t.first_run_day + TRIAL_DAYS;
                if today >= expire_day {
                    LicenseStatus::TrialExpired
                } else {
                    LicenseStatus::Trial {
                        days_left: expire_day as i64 - today as i64,
                    }
                }
            }
        }
    }

    /// 用户输入授权码 → 验签 → 比对机器码 → 落盘激活
    pub fn activate(&mut self, license_str: &str, today: u64) -> Result<LicensePayload, ActivateError> {
        let s = license_str.trim();
        let payload = verify_license(s, &self.machine_code).ok_or_else(|| {
            // 区分 Format / InvalidSig / Mismatch：先做粗判
            let (p_b64, sig_b64) = match s.split_once('.') {
                Some(pair) => pair,
                None => return ActivateError::Format,
            };
            let payload_text = match crypto::b64_decode(p_b64) {
                Some(b) => b,
                None => return ActivateError::Format,
            };
            let sig = match crypto::b64_decode(sig_b64) {
                Some(b) => b,
                None => return ActivateError::Format,
            };
            if !crypto::ed25519_verify(&payload_text, &sig) {
                return ActivateError::InvalidSig;
            }
            let text = match std::str::from_utf8(&payload_text) {
                Ok(t) => t,
                Err(_) => return ActivateError::Format,
            };
            match LicensePayload::parse(text) {
                None => ActivateError::Format,
                Some(p) => {
                    if p.product != PRODUCT_ID {
                        ActivateError::ProductMismatch
                    } else {
                        ActivateError::MachineMismatch
                    }
                }
            }
        })?;

        if payload.is_expired(today) {
            return Err(ActivateError::AlreadyExpired);
        }

        self.license = Some(payload.clone());
        self.license_raw = Some(s.to_string());

        // 持久化（确保 trial 存在，否则新建一个再带 license 落盘）
        let trial_owned = self.trial.clone().unwrap_or_else(|| {
            let mut t = TrialState {
                first_run_day: today,
                last_run_day: today,
                rollback_count: 0,
                mac: String::new(),
            };
            t.sign(&self.machine_fp);
            t
        });
        store::save(&trial_owned, &self.license_raw, &self.machine_fp);
        self.trial = Some(trial_owned);

        Ok(payload)
    }

    /// 每次正常运行后调用：推进高水位并持久化（文件 + 注册表）。
    ///
    /// 注意：本方法可能每帧被调用，故**仅当天数推进或检测到回拨时才落盘**，
    /// 避免正常运行时每秒几十次写文件 / 写注册表。
    pub fn checkpoint(&mut self, today: u64) -> bool {
        let Some(t) = self.trial.as_mut() else {
            return false;
        };
        let day_advanced = today > t.last_run_day;
        let rollback = t.last_run_day > today + ROLLBACK_TOLERANCE_DAYS;
        if day_advanced {
            t.last_run_day = today;
        }
        if rollback {
            t.rollback_count = t.rollback_count.saturating_add(1);
        }
        if !day_advanced && !rollback {
            return false; // 状态未变化，不落盘
        }
        t.sign(&self.machine_fp);
        store::save(t, &self.license_raw, &self.machine_fp);
        true
    }

    /// 用户可见机器码
    pub fn machine_code(&self) -> &str {
        &self.machine_code
    }

    /// 当前授权负载（已激活时）
    #[allow(dead_code)]
    pub fn license(&self) -> Option<&LicensePayload> {
        self.license.as_ref()
    }
}

/// 校验授权码字符串：签名 + 产品 + 机器码。通过返回负载（不含到期检查）。
fn verify_license(s: &str, machine_code: &str) -> Option<LicensePayload> {
    let s = s.trim();
    let (p_b64, sig_b64) = s.split_once('.')?;
    let payload_text = crypto::b64_decode(p_b64)?;
    let sig = crypto::b64_decode(sig_b64)?;
    if !crypto::ed25519_verify(&payload_text, &sig) {
        return None;
    }
    let text = std::str::from_utf8(&payload_text).ok()?;
    let p = LicensePayload::parse(text)?;
    if p.product != PRODUCT_ID {
        return None;
    }
    if p.machine != machine_code {
        return None;
    }
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 由 keygen `sign TEST-MACHINE-0001 0 TestCustomer` 生成（对应本 crate 内嵌公钥）。
    // 注意：该机器码是测试专用假码，不会与真实机器冲突，也不会触发真实激活。
    const TEST_LICENSE: &str = "dj0xCnA9dW15YS1leGNlbAptPVRFU1QtTUFDSElORS0wMDAxCmk9MjA2MjIKZT0wCmVkPXBybwpjPVRlc3RDdXN0b21lcgo=.ylkD01m4TO92M/syID43V3ZBr7WL1sF97HA/j0aBgvSCrKN/G4UI/8NZGWQ5x28xt4/s4lFxKCEnzIXmUmdhBw==";
    const TEST_MACHINE: &str = "TEST-MACHINE-0001";

    /// E2E：keygen（私钥）签发 ↔ app（内嵌公钥）验签 —— 核心安全属性
    #[test]
    fn e2e_license_verifies() {
        let p = verify_license(TEST_LICENSE, TEST_MACHINE).expect("license should verify");
        assert_eq!(p.product, "umya-excel");
        assert_eq!(p.machine, TEST_MACHINE);
        assert_eq!(p.customer, "TestCustomer");
        assert_eq!(p.expires_day, 0); // 永久
    }

    #[test]
    fn e2e_wrong_machine_rejected() {
        assert!(verify_license(TEST_LICENSE, "WRONG-MACHINE").is_none());
    }

    #[test]
    fn e2e_tampered_signature_rejected() {
        // 篡改签名段首字节，验签应失败
        let (payload, sig) = TEST_LICENSE.split_once('.').unwrap();
        let mut sig_bytes: Vec<u8> = sig.bytes().collect();
        sig_bytes[0] = if sig_bytes[0] == b'A' { b'B' } else { b'A' };
        let tampered = format!(
            "{}.{}",
            payload,
            String::from_utf8(sig_bytes).unwrap()
        );
        assert!(verify_license(&tampered, TEST_MACHINE).is_none());
    }

    #[test]
    fn e2e_malformed_rejected() {
        assert!(verify_license("not-a-license", TEST_MACHINE).is_none());
        assert!(verify_license("aaaa.bbbb", TEST_MACHINE).is_none());
    }
}
