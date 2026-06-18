//! 本地存储：试用状态 + 授权码，带 HMAC 校验与多位置冗余（文件 + 注册表）。
//!
//! 文件内容（两行）：
//! ```text
//! f=<first_run_day>|l=<last_run_day>|r=<rollback_count>|mac=<hex>
//! <授权码或空>
//! ```
//! 注册表（Windows，`HKCU\Software\MyExcel`）存同样两份。
//!
//! 加载时对每份存储做 HMAC 校验，失败视为篡改；多份有效副本合并取
//! `min(first_run_day)` / `max(last_run_day)`，抵抗删除/回拨绕过。

use std::path::PathBuf;

#[cfg(windows)]
fn reg_path() -> String {
    format!("Software\\{}", crate::license::fingerprint::registry_uuid())
}
const TRIAL_FILENAME: &str = "license.dat";

/// 试用状态
#[derive(Clone, Debug)]
pub struct TrialState {
    /// 首次启动日（试用起点）
    pub first_run_day: u64,
    /// 高水位：已观测到的最大 day（防回拨核心，只增不减）
    pub last_run_day: u64,
    /// 累计检测到的时钟回拨次数
    pub rollback_count: u32,
    /// HMAC(机器指纹, body)
    pub mac: String,
}

impl TrialState {
    /// 参与 HMAC 的明文（不含 mac 自身）
    fn body(&self) -> String {
        format!("f={}|l={}|r={}", self.first_run_day, self.last_run_day, self.rollback_count)
    }

    /// 计算并填充 mac
    pub fn sign(&mut self, machine_fp: &[u8]) {
        self.mac = super::crypto::hmac_hex(machine_fp, self.body().as_bytes());
    }

    /// 校验 mac
    pub fn verify(&self, machine_fp: &[u8]) -> bool {
        super::crypto::hmac_verify(machine_fp, self.body().as_bytes(), &self.mac)
    }
}

/// 加载结果
pub struct LoadResult {
    /// 合并后的试用状态（无则 None）
    pub trial: Option<TrialState>,
    /// 存储的授权码原始字符串（尚未验签，由 LicenseManager 验证）
    pub license_raw: Option<String>,
    /// 是否检测到“存在但验签失败”的存储（篡改信号）
    pub tampered: bool,
}

fn primary_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".MyExcel").join(TRIAL_FILENAME)
}

/// 解析并校验一行试用状态
fn parse_trial_line(line: &str, machine_fp: &[u8]) -> Option<TrialState> {
    let mut first = None;
    let mut last = None;
    let mut roll = None;
    let mut mac = String::new();
    for part in line.split('|') {
        let (k, v) = part.split_once('=')?;
        match k {
            "f" => first = v.parse().ok(),
            "l" => last = v.parse().ok(),
            "r" => roll = v.parse().ok(),
            "mac" => mac = v.to_string(),
            _ => {}
        };
    }
    let t = TrialState {
        first_run_day: first?,
        last_run_day: last?,
        rollback_count: roll?,
        mac,
    };
    if t.verify(machine_fp) {
        Some(t)
    } else {
        None
    }
}

/// 读取注册表（Windows）
#[cfg(windows)]
fn read_registry() -> Option<(String, String)> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey(&reg_path()).ok()?;
    let t: String = key.get_value("Trial").ok()?;
    let l: String = key.get_value("License").unwrap_or_default();
    Some((t, l))
}

/// 加载并合并所有存储
pub fn load(machine_fp: &[u8]) -> LoadResult {
    let mut trials: Vec<TrialState> = Vec::new();
    let mut license_raw: Option<String> = None;
    let mut tampered = false;

    // —— 文件 ——
    let path = primary_path();
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            let mut lines = content.lines();
            let trial_line = lines.next().unwrap_or("");
            let lic_line = lines.next().unwrap_or("");
            if !trial_line.is_empty() {
                match parse_trial_line(trial_line, machine_fp) {
                    Some(t) => trials.push(t),
                    None => tampered = true,
                }
            }
            if !lic_line.is_empty() {
                license_raw = Some(lic_line.to_string());
            }
        }
    }

    // —— 注册表（Windows） ——
    #[cfg(windows)]
    if let Some((t_line, l_line)) = read_registry() {
        if !t_line.is_empty() {
            match parse_trial_line(&t_line, machine_fp) {
                Some(t) => trials.push(t),
                None => tampered = true,
            }
        }
        if !l_line.is_empty() && license_raw.is_none() {
            license_raw = Some(l_line);
        }
    }

    // —— 合并：min(first_run_day) / max(last_run_day) / max(rollback_count) ——
    let trial = if trials.is_empty() {
        None
    } else {
        let first = trials.iter().map(|t| t.first_run_day).min().unwrap();
        let last = trials.iter().map(|t| t.last_run_day).max().unwrap();
        let roll = trials.iter().map(|t| t.rollback_count).max().unwrap();
        let mut merged = TrialState {
            first_run_day: first,
            last_run_day: last,
            rollback_count: roll,
            mac: String::new(),
        };
        merged.sign(machine_fp);
        Some(merged)
    };

    LoadResult {
        trial,
        license_raw,
        tampered,
    }
}

/// 保存：同时写文件 + 注册表（Windows）
pub fn save(trial: &TrialState, license_raw: &Option<String>, machine_fp: &[u8]) {
    let trial_line = format!(
        "f={}|l={}|r={}|mac={}",
        trial.first_run_day, trial.last_run_day, trial.rollback_count, trial.mac
    );
    let lic_line = license_raw.clone().unwrap_or_default();

    // 文件
    let path = primary_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let content = format!("{}\n{}\n", trial_line, lic_line);
    let _ = std::fs::write(&path, &content);

    // 注册表（Windows）
    #[cfg(windows)]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok((key, _)) = hkcu.create_subkey(&reg_path()) {
            let _ = key.set_value("Trial", &trial_line);
            let _ = key.set_value("License", &lic_line);
        }
    }

    // machine_fp 仅用于未来扩展（如按指纹命名文件），当前保存无需它
    let _ = machine_fp;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_of(t: &TrialState) -> String {
        format!(
            "f={}|l={}|r={}|mac={}",
            t.first_run_day, t.last_run_day, t.rollback_count, t.mac
        )
    }

    #[test]
    fn trial_sign_parse_roundtrip() {
        let fp: &[u8] = b"machine-fp";
        let mut t = TrialState {
            first_run_day: 100,
            last_run_day: 150,
            rollback_count: 2,
            mac: String::new(),
        };
        t.sign(fp);
        let parsed = parse_trial_line(&line_of(&t), fp).expect("should parse + verify");
        assert_eq!(parsed.first_run_day, 100);
        assert_eq!(parsed.last_run_day, 150);
        assert_eq!(parsed.rollback_count, 2);
    }

    #[test]
    fn trial_tampered_field_rejected() {
        let fp: &[u8] = b"machine-fp";
        let mut t = TrialState {
            first_run_day: 100,
            last_run_day: 150,
            rollback_count: 2,
            mac: String::new(),
        };
        t.sign(fp);
        // 篡改 last_run_day，HMAC 不再匹配
        let mut bad = line_of(&t);
        bad = bad.replace("l=150", "l=999");
        assert!(parse_trial_line(&bad, fp).is_none(), "tampered trial must fail HMAC");
    }

    #[test]
    fn trial_wrong_machine_rejected() {
        let mut t = TrialState {
            first_run_day: 100,
            last_run_day: 150,
            rollback_count: 0,
            mac: String::new(),
        };
        t.sign(b"machine-A");
        // 换机器指纹 → 派生的 HMAC 密钥不同 → 校验失败
        assert!(parse_trial_line(&line_of(&t), b"machine-B").is_none());
    }
}
