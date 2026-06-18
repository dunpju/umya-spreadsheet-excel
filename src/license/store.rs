//! 本地存储：试用状态 + 授权码，带 AES-256-GCM 加密 + HMAC 校验 + 多位置冗余。
//!
//! 文件内容（加密后为单行 base64，解密后两行）：
//! ```text
//! f=<first_run_day>|l=<last_run_day>|r=<rollback_count>|mac=<hex>
//! <授权码或空>
//! ```
//! 注册表（Windows，`HKCU\Software\{uuid}`）存加密副本（`Data` 值），
//! 同时保留明文 `Trial`/`License` 值以兼容旧版本。
//!
//! 加载时先尝试 AES-256-GCM 解密；失败则按明文解析（兼容升级前数据）。
//! 对每份存储做 HMAC 校验，失败视为篡改；多份有效副本合并取
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
    /// 是否检测到"存在但验签失败"的存储（篡改信号）
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

/// 从解密后的两行文本中提取试用状态和授权码
fn parse_content_lines(text: &str, machine_fp: &[u8]) -> (Option<TrialState>, Option<String>, bool) {
    let mut lines = text.lines();
    let trial_line = lines.next().unwrap_or("");
    let lic_line = lines.next().unwrap_or("");

    let mut trial = None;
    let mut tamp = false;

    if !trial_line.is_empty() {
        match parse_trial_line(trial_line, machine_fp) {
            Some(t) => trial = Some(t),
            None => tamp = true,
        }
    }

    let lic = if !lic_line.is_empty() {
        Some(lic_line.to_string())
    } else {
        None
    };
    (trial, lic, tamp)
}

/// 尝试对存储内容做 AES-256-GCM 解密。
///
/// 成功返回解密后的明文字符串；失败（非加密格式 / 被篡改 / 错误机器）返回 `None`。
fn try_decrypt(content: &str, machine_fp: &[u8]) -> Option<String> {
    let bytes = super::crypto::aes256gcm_decrypt(content.trim(), machine_fp)?;
    std::str::from_utf8(&bytes).ok().map(String::from)
}

/// 构建加密导出的明文（供 `--license` 显示用）。
///
/// 格式：`f={first_run_day}|l={last_run_day}|r={remaining_days}|mac={sha256(machine_fp)}`
pub fn build_export_blob(
    trial: &TrialState,
    license_raw: &Option<String>,
    machine_fp: &[u8],
) -> String {
    let fp_hash = super::crypto::sha256_hex(machine_fp);
    let today = super::time::today_epoch_day();

    let remaining = match license_raw {
        Some(raw) if !raw.is_empty() => {
            raw.split_once('.')
                .and_then(|(p_b64, _)| super::crypto::b64_decode(p_b64))
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .and_then(|text| super::payload::LicensePayload::parse(&text))
                .map(|p| {
                    if p.expires_day == super::payload::EXPIRES_NEVER {
                        0u64
                    } else {
                        let rem = p.expires_day as i64 - today as i64;
                        if rem < 0 { 0 } else { rem as u64 }
                    }
                })
                .unwrap_or(0)
        }
        _ => {
            let trial_expire = trial.first_run_day + super::payload::TRIAL_DAYS;
            let rem = trial_expire as i64 - today as i64;
            if rem < 0 { 0 } else { rem as u64 }
        }
    };

    format!(
        "f={}|l={}|r={}|mac={}",
        trial.first_run_day, trial.last_run_day, remaining, fp_hash
    )
}

/// 从注册表读取已保存的加密导出字符串（`LicenseBlob` 值）
#[cfg(windows)]
#[allow(dead_code)]
pub fn read_export_blob() -> Option<String> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey(&reg_path()).ok()?;
    key.get_value("LicenseBlob").ok()
}

/// 读取注册表旧版明文格式（`Trial` + `License` 值，兼容升级前数据）
#[cfg(windows)]
#[allow(dead_code)]
fn read_registry() -> Option<(String, String)> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey(&reg_path()).ok()?;
    let t: String = key.get_value("Trial").ok()?;
    let l: String = key.get_value("License").unwrap_or_default();
    Some((t, l))
}

/// 读取注册表新版加密格式（`Data` 值）
#[cfg(windows)]
fn read_registry_encrypted() -> Option<String> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey(&reg_path()).ok()?;
    key.get_value("Data").ok()
}

/// 加载并合并所有存储
pub fn load(machine_fp: &[u8]) -> LoadResult {
    let mut trials: Vec<TrialState> = Vec::new();
    let mut license_raw: Option<String> = None;
    let mut tampered = false;

    // —— 文件 ——
    let path = primary_path();
    if path.exists() {
        if let Ok(raw) = std::fs::read_to_string(&path) {
            // 先尝试 AES-256-GCM 解密（新版加密格式），失败则按明文解析（兼容旧版）
            let text = try_decrypt(&raw, machine_fp).unwrap_or(raw);
            let (t, l, tamp) = parse_content_lines(&text, machine_fp);
            if let Some(t) = t {
                trials.push(t);
            }
            if l.is_some() {
                license_raw = l;
            }
            if tamp {
                tampered = true;
            }
        }
    }

    // —— 注册表（Windows） ——
    #[cfg(windows)]
    if let Some(data) = read_registry_encrypted() {
        let text = try_decrypt(&data, machine_fp).unwrap_or(data);
        let (t, l, tamp) = parse_content_lines(&text, machine_fp);
        if let Some(t) = t {
            trials.push(t);
        }
        if l.is_some() && license_raw.is_none() {
            license_raw = l;
        }
        if tamp {
            tampered = true;
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

/// 保存：加密后同时写文件 + 注册表（Windows）+ 导出 blob
pub fn save(trial: &TrialState, license_raw: &Option<String>, machine_fp: &[u8]) {
    let trial_line = format!(
        "f={}|l={}|r={}|mac={}",
        trial.first_run_day, trial.last_run_day, trial.rollback_count, trial.mac
    );
    let lic_line = license_raw.clone().unwrap_or_default();
    let plaintext = format!("{}\n{}\n", trial_line, lic_line);

    // 加密内部存储内容
    let encrypted = super::crypto::aes256gcm_encrypt(plaintext.as_bytes(), machine_fp);

    // 生成加密导出字符串（供 --license 显示用）
    let export_blob = {
        let export_text = build_export_blob(trial, license_raw, machine_fp);
        super::crypto::aes256gcm_encrypt(export_text.as_bytes(), machine_fp)
    };

    // 文件：写入加密内容
    let path = primary_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Some(ref enc) = encrypted {
        let _ = std::fs::write(&path, enc);
    }

    // 注册表（Windows）
    #[cfg(windows)]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok((key, _)) = hkcu.create_subkey(&reg_path()) {
            // 新版：加密 Data 值
            if let Some(ref enc) = encrypted {
                let _ = key.set_value("Data", enc);
            }
            // 加密导出（供 --license 使用）
            if let Some(ref blob) = export_blob {
                let _ = key.set_value("LicenseBlob", blob);
            }
        }
    }
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
        assert!(parse_trial_line(&line_of(&t), b"machine-B").is_none());
    }

    #[test]
    fn encrypted_file_roundtrip() {
        let fp: &[u8] = b"test-machine-fp";
        let mut t = TrialState {
            first_run_day: 20622,
            last_run_day: 20622,
            rollback_count: 0,
            mac: String::new(),
        };
        t.sign(fp);
        let lic = Some("test-license-code".to_string());

        // 模拟 save：加密
        let trial_line = line_of(&t);
        let plaintext = format!("{}\n{}\n", trial_line, lic.as_deref().unwrap_or(""));
        let encrypted = super::super::crypto::aes256gcm_encrypt(plaintext.as_bytes(), fp)
            .expect("encrypt should succeed");

        // 模拟 load：解密并解析
        let decrypted = try_decrypt(&encrypted, fp).expect("decrypt should succeed");
        let (parsed_t, parsed_l, tamp) = parse_content_lines(&decrypted, fp);

        assert!(!tamp);
        let parsed_t = parsed_t.expect("trial should parse");
        assert_eq!(parsed_t.first_run_day, 20622);
        assert_eq!(parsed_t.last_run_day, 20622);
        assert_eq!(parsed_l, lic);
    }

    #[test]
    fn encrypted_file_tampered_rejected() {
        let fp: &[u8] = b"test-machine-fp";
        let plaintext = "f=100|l=150|r=0|mac=bad\n\n";
        let encrypted = super::super::crypto::aes256gcm_encrypt(plaintext.as_bytes(), fp)
            .expect("encrypt");

        // 篡改加密串
        let mut bytes = encrypted.into_bytes();
        bytes[5] = if bytes[5] == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(bytes).unwrap_or_default();

        // 解密应失败
        assert!(
            try_decrypt(&tampered, fp).is_none(),
            "tampered encrypted file must fail to decrypt"
        );
    }

    #[test]
    fn plaintext_fallback_works() {
        let fp: &[u8] = b"test-machine-fp";
        let mut t = TrialState {
            first_run_day: 100,
            last_run_day: 150,
            rollback_count: 0,
            mac: String::new(),
        };
        t.sign(fp);
        let content = format!("{}\ntest-license\n", line_of(&t));

        // try_decrypt 对明文应返回 None（不是有效加密串）
        assert!(try_decrypt(&content, fp).is_none());

        // 但 parse_content_lines 能正确解析明文
        let (parsed_t, parsed_l, tamp) = parse_content_lines(&content, fp);
        assert!(!tamp);
        assert!(parsed_t.is_some());
        assert_eq!(parsed_l, Some("test-license".to_string()));
    }
}
