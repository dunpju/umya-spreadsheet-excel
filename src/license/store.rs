//! 本地存储：试用状态 + 授权码，AES-256-GCM 加密 + HMAC 校验 + **多点分散冗余**。
//!
//! 设计目标：让"删除存储来重置试用"和"按内容批量定位并删除全部存储"都难以奏效。
//!
//! # 存储点（best-effort，单点失败不影响其余；缺失点不算篡改）
//!
//! | tag | 位置 | 说明 |
//! |---|---|---|
//! | `home` | `~/.MyExcel/license.dat` | 既有路径，向后兼容 |
//! | `config` | `{config_dir}/{dir_uuid(config)}/state.dat` | 新增（如 `%APPDATA%\{dir_uuid(config)}\`） |
//! | `local` | `{data_local_dir}/{dir_uuid(local)}/cache.bin` | 新增（如 `%LOCALAPPDATA%\{dir_uuid(local)}\`） |
//! | `regmain` | 注册表 `HKCU\Software\{uuid}\Data` | 既有（仅 Windows） |
//! | `regclsid` | 注册表 `HKCU\Software\Classes\CLSID\{uuid}\Data` | 新增分支（仅 Windows） |
//!
//! # 差异化加密
//!
//! 每个存储点用**分位置密钥**加密（[`super::crypto::aes256gcm_encrypt_for`]，
//! 密钥 = `SHA256(LOCATION_LABEL || PEPPER || 机器指纹 || tag)`，随机 nonce），
//! 因此各点密文**互不相同**，且无法把 A 点密文搬到 B 点解密（抗重定位 / 抗按内容批量定位）。
//!
//! # 明文格式（加密前，两行）
//!
//! ```text
//! f=<first_run_day>|l=<last_run_day>|r=<rollback_count>|mac=<hex>|loc=<tag>|mani=<manifest>
//! <授权码或空>
//! ```
//!
//! - `mac` 仍是 HMAC over `f|l|r`（[`TrialState::body`] 不变），仅覆盖试用核心字段；
//! - `loc`（位置 tag）与 `mani`（清单哈希）追加在后，**不**进 HMAC，但被 AES-GCM 整体认证 → 防篡改；
//! - `mani = sha256(排序后的全部 tag)`，每个二进制版本固定；用于区分"当前版本"与"旧版/遗留"blob。
//!
//! # 加载与交叉校验
//!
//! 每点先尝试分位置解密，失败再尝试无 tag 旧版解密（兼容升级前数据），最后兜底按明文解析。
//! 合并取 `min(first_run_day)` / `max(last_run_day)` / `max(rollback_count)`。
//! 缺失点不算篡改；当前版本 blob 之间 `first_run_day` 或 license 不一致 → 篡改（[`cross_validate`]）。
//! 参见 [`load`] / [`save`] / [`cross_validate`] 的文档。

use std::path::PathBuf;

#[cfg(windows)]
fn reg_path() -> String {
    format!("Software\\{}", crate::license::fingerprint::registry_uuid())
}

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
    /// 参与 HMAC 的明文（不含 mac 自身，也不含 loc/mani）
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
    /// 是否需要**主动自愈**：非篡改、且已有 trial，但有存储点（含 `LicenseBlob`）缺失。
    /// 调用方据此立即 `save()` 重写全部存储点，补齐冗余。
    pub needs_heal: bool,
}

// ===========================================================================
// 存储点抽象
// ===========================================================================

/// 一个可读写的分散存储点。`tag` 是稳定的位置标识，兼作加密盐。
trait Store {
    /// 稳定位置标识（加密盐 / 清单成员）
    fn tag(&self) -> &'static str;
    /// 读取原始密文；`None` 表示不存在 / 不可读（**不算**篡改）。
    fn read(&self) -> Option<String>;
    /// best-effort 写入，错误被忽略（与既有实现一致）。
    fn write(&self, data: &str);
}

/// 文件系统存储点
struct FileStore {
    tag: &'static str,
    path: PathBuf,
}

impl Store for FileStore {
    fn tag(&self) -> &'static str {
        self.tag
    }
    fn read(&self) -> Option<String> {
        let s = std::fs::read_to_string(&self.path).ok()?;
        if s.trim().is_empty() {
            None
        } else {
            Some(s)
        }
    }
    fn write(&self, data: &str) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&self.path, data);
    }
}

/// 注册表存储点（`HKCU` 下，无需管理员权限）
#[cfg(windows)]
struct RegStore {
    tag: &'static str,
    subkey: String,
}

#[cfg(windows)]
impl Store for RegStore {
    fn tag(&self) -> &'static str {
        self.tag
    }
    fn read(&self) -> Option<String> {
        use winreg::enums::*;
        use winreg::RegKey;
        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(&self.subkey).ok()?;
        let v: String = key.get_value("Data").ok()?;
        if v.trim().is_empty() {
            None
        } else {
            Some(v)
        }
    }
    fn write(&self, data: &str) {
        use winreg::enums::*;
        use winreg::RegKey;
        if let Ok((key, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(&self.subkey) {
            let value = data.to_string();
            let _ = key.set_value("Data", &value);
        }
    }
}

/// 构造全部存储点（best-effort；`dirs` 返回 `None` 的点直接跳过，不入列表）。
///
/// 注意：`tag` 集合必须与 [`all_store_tags`] 完全一致，否则 [`expected_manifest`] 会与实际写入点不匹配。
fn all_stores() -> Vec<Box<dyn Store>> {
    let mut stores: Vec<Box<dyn Store>> = Vec::new();

    if let Some(home) = dirs::home_dir() {
        stores.push(Box::new(FileStore {
            tag: "home",
            path: home.join(".MyExcel").join("license.dat"),
        }));
    }
    if let Some(cfg) = dirs::config_dir() {
        stores.push(Box::new(FileStore {
            tag: "config",
            path: cfg.join(crate::license::fingerprint::dir_uuid("config")).join("state.dat"),
        }));
    }
    if let Some(loc) = dirs::data_local_dir() {
        stores.push(Box::new(FileStore {
            tag: "local",
            path: loc.join(crate::license::fingerprint::dir_uuid("local")).join("cache.bin"),
        }));
    }

    #[cfg(windows)]
    {
        let uuid = crate::license::fingerprint::registry_uuid();
        stores.push(Box::new(RegStore {
            tag: "regmain",
            subkey: format!("Software\\{uuid}"),
        }));
        stores.push(Box::new(RegStore {
            tag: "regclsid",
            // CLSID 惯例：大写 + 花括号（如 …\CLSID\{71445FAC-…}）。仅此点用 clsid 形式，
            // regmain 仍用小写无花括号的 {uuid}。路径不影响加密密钥（密钥由 tag 派生）。
            subkey: format!(
                "Software\\Classes\\CLSID\\{}",
                crate::license::fingerprint::registry_uuid_clsid()
            ),
        }));
    }

    stores
}

/// 全部存储点的 tag 集合（**不**访问文件系统/注册表；仅供清单哈希计算）。
/// 必须与 [`all_stores`] 实际构造的 tag 集合一致。
fn all_store_tags() -> Vec<&'static str> {
    let mut tags = vec!["home", "config", "local"];
    #[cfg(windows)]
    tags.extend(["regmain", "regclsid"]);
    tags
}

/// 清单哈希 = `sha256(排序后的全部 tag 用 | 连接)`。编译期常量，每个二进制版本固定。
/// 用于区分"当前版本"blob（参与严格校验）与"旧版/遗留"blob（信任、跳过严格校验）。
fn expected_manifest() -> String {
    let mut tags = all_store_tags();
    tags.sort_unstable();
    super::crypto::sha256_hex(tags.join("|").as_bytes())
}

// ===========================================================================
// 明文构造 / 记录解析
// ===========================================================================

/// 构造某存储点的明文（加密前）。`mac` 仅覆盖 `f|l|r`；`loc`/`mani` 由 GCM 认证。
fn build_plaintext(trial: &TrialState, license_raw: &Option<String>, tag: &str) -> String {
    let trial_line = format!(
        "f={}|l={}|r={}|mac={}|loc={}|mani={}",
        trial.first_run_day,
        trial.last_run_day,
        trial.rollback_count,
        trial.mac,
        tag,
        expected_manifest(),
    );
    format!("{}\n{}\n", trial_line, license_raw.clone().unwrap_or_default())
}

/// 解析解密后的一段文本，拆出试用状态、授权码、以及位置/清单元信息。
struct Record {
    /// `None` 表示 trial_line 为空，或解析/HMAC 失败
    trial: Option<TrialState>,
    license: Option<String>,
    /// 记录在 blob 内的位置 tag（旧版 blob 无此字段 → `None`）
    loc: Option<String>,
    /// 记录在 blob 内的清单哈希（旧版 blob 无此字段 → `None`）
    mani: Option<String>,
}

fn parse_record(text: &str, machine_fp: &[u8]) -> Record {
    let mut lines = text.lines();
    let trial_line = lines.next().unwrap_or("");
    let lic_line = lines.next().unwrap_or("");

    let mut loc = None;
    let mut mani = None;
    for part in trial_line.split('|') {
        if let Some((k, v)) = part.split_once('=') {
            match k {
                "loc" => loc = Some(v.to_string()),
                "mani" => mani = Some(v.to_string()),
                _ => {}
            }
        }
    }

    let trial = if trial_line.is_empty() {
        None
    } else {
        parse_trial_line(trial_line, machine_fp)
    };
    let license = if lic_line.is_empty() {
        None
    } else {
        Some(lic_line.to_string())
    };

    Record { trial, license, loc, mani }
}

/// 解析并校验一行试用状态（`mac` 失败返回 `None`）。
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

/// 尝试对存储内容做**无 tag** AES-256-GCM 解密（旧版/遗留格式兜底）。
///
/// 成功返回解密后的明文字符串；失败（非加密格式 / 被篡改 / 错误机器）返回 `None`。
fn try_decrypt(content: &str, machine_fp: &[u8]) -> Option<String> {
    let bytes = super::crypto::aes256gcm_decrypt(content.trim(), machine_fp)?;
    std::str::from_utf8(&bytes).ok().map(String::from)
}

// ===========================================================================
// 导出 blob（--license CLI 用，保持无 tag）
// ===========================================================================

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

/// 从注册表读取已保存的加密导出字符串（`LicenseBlob` 值）。
/// 亦用于自愈判定：`LicenseBlob` 缺失时触发重写。
#[cfg(windows)]
pub fn read_export_blob() -> Option<String> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey(&reg_path()).ok()?;
    key.get_value("LicenseBlob").ok()
}

// ===========================================================================
// --license 解密（兼容导出 blob 与各存储点密文）
// ===========================================================================

/// 解密 `--license` 入参，并统一输出为**导出格式**
/// （`f=<first_run_day>|l=<last_run_day>|r=<剩余天数>|mac=<指纹哈希>`），便于技术支持阅读。
///
/// **多存储位置兼容**：入参的加密字符串可能来自两种来源——
/// - `LicenseBlob` 导出（`save()` 写入注册表，**无 tag** 导出密钥）；
/// - 任一存储点的内部密文（`home` / `config` / `local` / `regmain` / `regclsid`，**分位置密钥**）。
///
/// 每个存储位置的密钥各不相同（见 [`super::crypto::aes256gcm_encrypt_for`]），故本函数依次尝试
/// 「无 tag 导出密钥」+「各存储点分位置密钥」，**任一成功即解密**，不受来源位置影响。
/// 存储点密文解密后是内部格式（带 `mac`/`loc`/`mani`），会被 [`normalize_for_display`]
/// 重新格式化为导出格式，使输出与来源无关。
///
/// 返回 `None` 表示所有密钥都解不开（串损坏 / 非本机生成 / 格式错误）。
pub fn decrypt_for_display(encoded: &str, machine_fp: &[u8]) -> Option<String> {
    let trimmed = encoded.trim();

    // 依次尝试：无 tag 导出密钥 → 各存储点分位置密钥（短路于第一个成功）
    let bytes = super::crypto::aes256gcm_decrypt(trimmed, machine_fp).or_else(|| {
        all_store_tags().iter().find_map(|tag| {
            super::crypto::aes256gcm_decrypt_for(trimmed, machine_fp, tag.as_bytes())
        })
    })?;

    let text = std::str::from_utf8(&bytes).ok()?;
    Some(normalize_for_display(text, machine_fp))
}

/// 把解密后的明文统一为导出格式：能解析成内部 trial + license 就用 [`build_export_blob`]
/// 重新格式化（存储点密文的情况）；否则（已是导出格式或无法解析）原样返回。
fn normalize_for_display(text: &str, machine_fp: &[u8]) -> String {
    let rec = parse_record(text, machine_fp);
    if let Some(t) = rec.trial {
        // 内部存储格式（带 mac/loc/mani + 第二行 license）→ 导出格式
        return build_export_blob(&t, &rec.license, machine_fp);
    }
    // 已是导出格式（或无法解析）→ 原样（去尾换行）返回
    text.trim_end().to_string()
}

// ===========================================================================
// 交叉校验（纯函数，便于单测）
// ===========================================================================

/// 交叉校验各存储点记录的一致性。返回 `true` 表示检测到篡改信号。
///
/// 规则（**保守，防误锁**）：
/// - 仅"当前版本"blob（`mani == manifest`）参与；旧版/遗留 blob（无 `mani` 或不匹配）排除；
/// - ≥2 条当前 blob 的 `first_run_day` 出现 ≥2 个不同值 → 篡改（正常运行各点值完全相同）；
/// - ≥2 条当前 blob 带不同的非空 license → 篡改；
/// - **`last_run_day` 与 `rollback_count` 的不一致永远不算篡改**（volatile 高水位/计数器，
///   中断保存会产生合理差异；靠 `max` 合并即可，HMAC 已保证单条完整性）。
fn cross_validate(records: &[(&str, &Record)], manifest: &str) -> bool {
    let current: Vec<&Record> = records
        .iter()
        .filter(|(_, r)| r.mani.as_deref() == Some(manifest))
        .map(|(_, r)| *r)
        .collect();
    if current.len() < 2 {
        return false;
    }

    // first_run_day 不一致 → 篡改
    let mut firsts: Vec<u64> = current
        .iter()
        .filter_map(|r| r.trial.as_ref())
        .map(|t| t.first_run_day)
        .collect();
    firsts.sort_unstable();
    firsts.dedup();
    if firsts.len() >= 2 {
        return true;
    }

    // 非空 license 不一致 → 篡改
    let mut lics: Vec<&str> = current
        .iter()
        .filter_map(|r| r.license.as_deref().filter(|s| !s.is_empty()))
        .collect();
    lics.sort_unstable();
    lics.dedup();
    if lics.len() >= 2 {
        return true;
    }

    false
}

// ===========================================================================
// load / save（签名与既有调用点兼容：load(fp) / save(trial, license_raw, fp)）
// ===========================================================================

/// 加载并合并所有存储点，交叉校验一致性，并标记是否需要**主动自愈**（[`LoadResult::needs_heal`]）。
pub fn load(machine_fp: &[u8]) -> LoadResult {
    let mut lr = load_from(all_stores(), machine_fp);
    // LicenseBlob 是与内部存储点并列的第 6 处（仅 Windows）；缺失也触发自愈
    #[cfg(windows)]
    if !lr.tampered && read_export_blob().is_none() {
        lr.needs_heal = true;
    }
    lr
}

/// 从给定存储点集合加载（可注入 mock，便于单测自愈逻辑）。
fn load_from(stores: Vec<Box<dyn Store>>, machine_fp: &[u8]) -> LoadResult {
    let manifest = expected_manifest();
    let expected = stores.len();
    let mut records: Vec<(&'static str, Record)> = Vec::new();
    let mut tampered = false;

    for store in stores {
        let tag = store.tag();
        let Some(raw) = store.read() else {
            continue; // 不存在 / 不可读 → 非篡改，跳过
        };

        // 三级解密兜底：分位置 → 无 tag 旧版 AES → 原始明文（pre-encryption 时代）
        let text = super::crypto::aes256gcm_decrypt_for(raw.trim(), machine_fp, tag.as_bytes())
            .and_then(|b| String::from_utf8(b).ok())
            .or_else(|| try_decrypt(&raw, machine_fp))
            .unwrap_or_else(|| raw.trim().to_string());

        let rec = parse_record(&text, machine_fp);

        // trial_line 非空却解析/HMAC 失败 → 篡改信号
        let trial_line_present = text.lines().next().map_or(false, |l| !l.is_empty());
        if trial_line_present && rec.trial.is_none() {
            tampered = true;
        }
        // 仅"当前版本"blob 做严格位置校验：loc 必须与读取它的存储点 tag 一致（抗搬迁）
        if rec.mani.as_deref() == Some(manifest.as_str()) && rec.loc.as_deref() != Some(tag) {
            tampered = true;
        }

        records.push((tag, rec));
    }

    // 合并：min(first_run_day) / max(last_run_day) / max(rollback_count)
    let valid_trials: Vec<&TrialState> =
        records.iter().filter_map(|(_, r)| r.trial.as_ref()).collect();
    let trial = if valid_trials.is_empty() {
        None
    } else {
        let first = valid_trials.iter().map(|t| t.first_run_day).min().unwrap();
        let last = valid_trials.iter().map(|t| t.last_run_day).max().unwrap();
        let roll = valid_trials.iter().map(|t| t.rollback_count).max().unwrap();
        let mut merged = TrialState {
            first_run_day: first,
            last_run_day: last,
            rollback_count: roll,
            mac: String::new(),
        };
        merged.sign(machine_fp);
        Some(merged)
    };

    let license_raw = records.iter().find_map(|(_, r)| r.license.clone());

    let cv_input: Vec<(&str, &Record)> = records.iter().map(|(t, r)| (*t, r)).collect();
    if cross_validate(&cv_input, &manifest) {
        tampered = true;
    }

    // 主动自愈判定：非篡改 且 内部存储点有缺失（present < expected）。
    // LicenseBlob 的缺失在 [`load`] 包装层额外置位。
    let present = records.len();
    let needs_heal = healing_needed(present, expected, tampered);

    LoadResult {
        trial,
        license_raw,
        tampered,
        needs_heal,
    }
}

/// 是否需要主动自愈补写：非篡改，且内部存储点有缺失（`present < expected`）。
///
/// 篡改时**永不**自愈（避免覆盖篡改证据 / 被攻击者借机迫使重写）。
fn healing_needed(stores_present: usize, stores_expected: usize, tampered: bool) -> bool {
    !tampered && stores_present < stores_expected
}

/// 保存：向**所有**存储点写入**分位置差异化密文**；LicenseBlob 导出保持无 tag（供 `--license`）。
pub fn save(trial: &TrialState, license_raw: &Option<String>, machine_fp: &[u8]) {
    for store in all_stores() {
        let tag = store.tag();
        let plaintext = build_plaintext(trial, license_raw, tag);
        if let Some(enc) = super::crypto::aes256gcm_encrypt_for(
            plaintext.as_bytes(),
            machine_fp,
            tag.as_bytes(),
        ) {
            store.write(&enc); // best-effort；各点密文互不相同（tag + 随机 nonce）
        }
    }

    // LicenseBlob 导出（无 tag）—— 写入 regmain，与既有 --license CLI 兼容
    #[cfg(windows)]
    {
        let export_text = build_export_blob(trial, license_raw, machine_fp);
        if let Some(blob) = super::crypto::aes256gcm_encrypt(export_text.as_bytes(), machine_fp) {
            use winreg::enums::*;
            use winreg::RegKey;
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            if let Ok((key, _)) = hkcu.create_subkey(&reg_path()) {
                let _ = key.set_value("LicenseBlob", &blob);
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

    fn signed_trial(first: u64, last: u64, roll: u32, fp: &[u8]) -> TrialState {
        let mut t = TrialState {
            first_run_day: first,
            last_run_day: last,
            rollback_count: roll,
            mac: String::new(),
        };
        t.sign(fp);
        t
    }

    #[test]
    fn trial_sign_parse_roundtrip() {
        let fp: &[u8] = b"machine-fp";
        let t = signed_trial(100, 150, 2, fp);
        let parsed = parse_trial_line(&line_of(&t), fp).expect("should parse + verify");
        assert_eq!(parsed.first_run_day, 100);
        assert_eq!(parsed.last_run_day, 150);
        assert_eq!(parsed.rollback_count, 2);
    }

    #[test]
    fn trial_tampered_field_rejected() {
        let fp: &[u8] = b"machine-fp";
        let t = signed_trial(100, 150, 2, fp);
        // 篡改 last_run_day，HMAC 不再匹配
        let mut bad = line_of(&t);
        bad = bad.replace("l=150", "l=999");
        assert!(parse_trial_line(&bad, fp).is_none(), "tampered trial must fail HMAC");
    }

    #[test]
    fn trial_wrong_machine_rejected() {
        let t = signed_trial(100, 150, 0, b"machine-A");
        assert!(parse_trial_line(&line_of(&t), b"machine-B").is_none());
    }

    #[test]
    fn encrypted_file_roundtrip() {
        let fp: &[u8] = b"test-machine-fp";
        let t = signed_trial(20622, 20622, 0, fp);
        let lic = Some("test-license-code".to_string());

        // 模拟 save：构造 home 点明文，用无 tag 加密（与 try_decrypt 兜底路径配对做 roundtrip）
        let plaintext = build_plaintext(&t, &lic, "home");
        let encrypted = super::super::crypto::aes256gcm_encrypt(plaintext.as_bytes(), fp)
            .expect("tag-less encrypt for roundtrip");

        // 模拟 load：无 tag 解密（兜底路径）并解析
        let decrypted = try_decrypt(&encrypted, fp).expect("decrypt should succeed");
        let rec = parse_record(&decrypted, fp);

        assert!(rec.trial.is_some(), "trial should parse");
        let parsed_t = rec.trial.unwrap();
        assert_eq!(parsed_t.first_run_day, 20622);
        assert_eq!(parsed_t.last_run_day, 20622);
        assert_eq!(rec.license, lic);
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
        let t = signed_trial(100, 150, 0, fp);
        let content = format!("{}\ntest-license\n", line_of(&t));

        // try_decrypt 对明文应返回 None（不是有效加密串）
        assert!(try_decrypt(&content, fp).is_none());

        // 但 parse_record 能直接解析明文
        let rec = parse_record(&content, fp);
        assert!(rec.trial.is_some());
        assert_eq!(rec.license, Some("test-license".to_string()));
    }

    // ----- 新增：多点 + 差异化加密 + 交叉校验 -----

    #[test]
    fn differentiated_ciphertext_per_store() {
        let fp: &[u8] = b"machine-fp";
        let t = signed_trial(100, 150, 0, fp);

        // 同一 trial，不同 tag 加密 → 密文不同
        let home_pt = build_plaintext(&t, &None, "home");
        let cfg_pt = build_plaintext(&t, &None, "config");
        let home_enc = super::super::crypto::aes256gcm_encrypt_for(home_pt.as_bytes(), fp, b"home")
            .expect("encrypt home");
        let cfg_enc = super::super::crypto::aes256gcm_encrypt_for(cfg_pt.as_bytes(), fp, b"config")
            .expect("encrypt config");
        assert_ne!(home_enc, cfg_enc, "ciphertext must differ per store");

        // 且明文内嵌的 loc 字段也不同
        assert!(home_pt.contains("loc=home"));
        assert!(cfg_pt.contains("loc=config"));
    }

    #[test]
    fn cross_validate_first_run_disagreement_is_tamper() {
        let fp: &[u8] = b"machine-fp";
        let manifest = expected_manifest();
        let mk = |first: u64, loc: &str| {
            let t = signed_trial(first, 150, 0, fp);
            let line = format!(
                "f={}|l={}|r={}|mac={}|loc={}|mani={}",
                t.first_run_day, t.last_run_day, t.rollback_count, t.mac, loc, manifest
            );
            let text = format!("{line}\n\n");
            (loc.to_string(), parse_record(&text, fp))
        };
        let records = vec![mk(100, "home"), mk(200, "config")];
        let cv: Vec<(&str, &Record)> = records.iter().map(|(t, r)| (t.as_str(), r)).collect();
        assert!(cross_validate(&cv, &manifest), "first_run_day disagreement => tamper");
    }

    #[test]
    fn cross_validate_rollback_disagreement_is_not_tamper() {
        let fp: &[u8] = b"machine-fp";
        let manifest = expected_manifest();
        let mk = |roll: u32, loc: &str| {
            let t = signed_trial(100, 150, roll, fp);
            let line = format!(
                "f={}|l={}|r={}|mac={}|loc={}|mani={}",
                t.first_run_day, t.last_run_day, t.rollback_count, t.mac, loc, manifest
            );
            let text = format!("{line}\n\n");
            (loc.to_string(), parse_record(&text, fp))
        };
        // 仅 rollback_count 不同（last_run 也不同）→ 不算篡改
        let records = vec![mk(0, "home"), mk(5, "config")];
        let cv: Vec<(&str, &Record)> = records.iter().map(|(t, r)| (t.as_str(), r)).collect();
        assert!(!cross_validate(&cv, &manifest), "rollback/last_run disagreement must NOT be tamper");
    }

    #[test]
    fn cross_validate_legacy_record_excluded() {
        let fp: &[u8] = b"machine-fp";
        let manifest = expected_manifest();
        // 一条当前版本（first=100），一条遗留（无 mani，first=999）→ 遗留排除 → 不算篡改
        let cur_t = signed_trial(100, 150, 0, fp);
        let cur_line = format!(
            "f={}|l={}|r={}|mac={}|loc=home|mani={}",
            cur_t.first_run_day, cur_t.last_run_day, cur_t.rollback_count, cur_t.mac, manifest
        );
        let cur = parse_record(&format!("{cur_line}\n\n"), fp);

        let leg_t = signed_trial(999, 999, 0, fp);
        let leg_line = line_of(&leg_t); // 旧格式：无 loc/mani
        let leg = parse_record(&format!("{leg_line}\n\n"), fp);

        let records = vec![("home".to_string(), cur), ("config".to_string(), leg)];
        let cv: Vec<(&str, &Record)> = records.iter().map(|(t, r)| (t.as_str(), r)).collect();
        assert!(!cross_validate(&cv, &manifest), "legacy record must be excluded from cross-validation");
    }

    #[test]
    fn legacy_fallback_decrypts_old_blob() {
        let fp: &[u8] = b"machine-fp";
        let t = signed_trial(20622, 20630, 0, fp);
        // 旧版无 tag 明文（升级前的格式），用无 tag 加密
        let legacy_pt = format!("{}\nmy-license\n", line_of(&t));
        let legacy_enc = super::super::crypto::aes256gcm_encrypt(legacy_pt.as_bytes(), fp)
            .expect("legacy encrypt");

        // 分位置解密应失败
        assert!(
            super::super::crypto::aes256gcm_decrypt_for(&legacy_enc, fp, b"home").is_none(),
            "legacy blob must not decrypt via tagged path"
        );
        // 无 tag 兜底应成功
        let text = try_decrypt(&legacy_enc, fp).expect("legacy fallback decrypt");
        let rec = parse_record(&text, fp);
        assert!(rec.trial.is_some());
        assert_eq!(rec.mani, None, "legacy blob has no manifest");
        assert_eq!(rec.license, Some("my-license".to_string()));
    }

    #[test]
    fn partial_deletion_does_not_reset_trial() {
        // 模拟 5 点中只有 2 点在（值一致）、3 点缺失 → load 语义：
        // 通过 cross_validate 验证"幸存的 2 条当前版本记录一致 → 不算篡改"，
        // 且合并出的高水位来自幸存点（不重置）。
        let fp: &[u8] = b"machine-fp";
        let manifest = expected_manifest();
        let t = signed_trial(20622, 20640, 0, fp);
        let mk = |loc: &str| {
            let line = format!(
                "f={}|l={}|r={}|mac={}|loc={}|mani={}",
                t.first_run_day, t.last_run_day, t.rollback_count, t.mac, loc, manifest
            );
            (loc.to_string(), parse_record(&format!("{line}\n\n"), fp))
        };
        let survivors = vec![mk("home"), mk("regmain")]; // 另 3 点缺失
        let cv: Vec<(&str, &Record)> = survivors.iter().map(|(t, r)| (t.as_str(), r)).collect();
        assert!(!cross_validate(&cv, &manifest), "consistent survivors must not be tamper");
        // 高水位（last_run_day=20640）来自幸存点
        assert_eq!(survivors[0].1.trial.as_ref().unwrap().last_run_day, 20640);
    }

    #[test]
    fn relocation_rejected() {
        let fp: &[u8] = b"machine-fp";
        // home 点密文无法用 config 的 tag 解密
        let pt = b"some trial plaintext";
        let home_enc = super::super::crypto::aes256gcm_encrypt_for(pt, fp, b"home").expect("enc");
        assert!(
            super::super::crypto::aes256gcm_decrypt_for(&home_enc, fp, b"config").is_none(),
            "relocated blob must not decrypt under wrong tag"
        );
        // 且 load 对 loc 与存储点 tag 不一致的当前版本 blob 标 tamper
        let manifest = expected_manifest();
        let t = signed_trial(100, 150, 0, fp);
        // 故意构造 loc=home 的 blob，却"从 config 点读出"
        let line = format!(
            "f={}|l={}|r={}|mac={}|loc=home|mani={}",
            t.first_run_day, t.last_run_day, t.rollback_count, t.mac, manifest
        );
        let rec = parse_record(&format!("{line}\n\n"), fp);
        assert_eq!(rec.loc.as_deref(), Some("home"));
        assert_ne!(rec.loc.as_deref(), Some("config"), "loc mismatches the store it was read from");
    }

    // ----- 主动自愈 -----

    #[test]
    fn healing_needed_predicate() {
        // 非篡改 + 有缺失 → 需要自愈
        assert!(healing_needed(4, 5, false));
        assert!(healing_needed(0, 3, false));
        // 全部在场 + 非篡改 → 不需要
        assert!(!healing_needed(5, 5, false));
        assert!(!healing_needed(3, 3, false));
        // 篡改 → 永不自愈
        assert!(!healing_needed(4, 5, true));
        assert!(!healing_needed(0, 5, true));
    }

    /// 内存 mock 存储点：不碰真实文件系统 / 注册表即可测试 load_from
    use std::cell::RefCell;
    struct MemStore {
        tag: &'static str,
        data: RefCell<Option<String>>,
    }
    impl Store for MemStore {
        fn tag(&self) -> &'static str {
            self.tag
        }
        fn read(&self) -> Option<String> {
            self.data.borrow().clone()
        }
        fn write(&self, d: &str) {
            *self.data.borrow_mut() = Some(d.to_string());
        }
    }

    /// 用真实加密路径构造一份合法"当前版本"blob（loc 与 tag 一致）
    fn mem_blob(tag: &'static str, t: &TrialState, fp: &[u8], manifest: &str) -> String {
        let line = format!(
            "f={}|l={}|r={}|mac={}|loc={}|mani={}",
            t.first_run_day, t.last_run_day, t.rollback_count, t.mac, tag, manifest
        );
        let pt = format!("{line}\n\n");
        super::super::crypto::aes256gcm_encrypt_for(pt.as_bytes(), fp, tag.as_bytes())
            .expect("encrypt_for")
    }

    #[test]
    fn load_from_no_heal_when_all_present() {
        let fp: &[u8] = b"machine-fp";
        let manifest = expected_manifest();
        let t = signed_trial(100, 150, 0, fp);
        let stores: Vec<Box<dyn Store>> = vec![
            Box::new(MemStore { tag: "home", data: RefCell::new(Some(mem_blob("home", &t, fp, &manifest))) }),
            Box::new(MemStore { tag: "config", data: RefCell::new(Some(mem_blob("config", &t, fp, &manifest))) }),
            Box::new(MemStore { tag: "local", data: RefCell::new(Some(mem_blob("local", &t, fp, &manifest))) }),
        ];
        let lr = load_from(stores, fp);
        assert!(lr.trial.is_some());
        assert!(!lr.tampered);
        assert!(!lr.needs_heal, "all stores present => no heal");
    }

    #[test]
    fn load_from_heals_when_store_missing() {
        let fp: &[u8] = b"machine-fp";
        let manifest = expected_manifest();
        let t = signed_trial(100, 150, 0, fp);
        // config 点缺失（read None）
        let stores: Vec<Box<dyn Store>> = vec![
            Box::new(MemStore { tag: "home", data: RefCell::new(Some(mem_blob("home", &t, fp, &manifest))) }),
            Box::new(MemStore { tag: "config", data: RefCell::new(None) }),
            Box::new(MemStore { tag: "local", data: RefCell::new(Some(mem_blob("local", &t, fp, &manifest))) }),
        ];
        let lr = load_from(stores, fp);
        assert!(lr.trial.is_some(), "trial reconstructed from survivors");
        assert!(!lr.tampered, "missing store is NOT tamper");
        assert!(lr.needs_heal, "one store missing => needs_heal");
    }

    #[test]
    fn load_from_no_heal_when_tampered() {
        let fp: &[u8] = b"machine-fp";
        let manifest = expected_manifest();
        let t = signed_trial(100, 150, 0, fp);
        // home 点缺失；config 点放一个"被搬迁"的 blob（loc=home 却在 config 点）→ tamper
        let bad_line = format!(
            "f={}|l={}|r={}|mac={}|loc=home|mani={}",
            t.first_run_day, t.last_run_day, t.rollback_count, t.mac, manifest
        );
        let bad_blob = super::super::crypto::aes256gcm_encrypt_for(
            format!("{bad_line}\n\n").as_bytes(),
            fp,
            b"config",
        )
        .expect("encrypt_for");
        let stores: Vec<Box<dyn Store>> = vec![
            Box::new(MemStore { tag: "home", data: RefCell::new(None) }),
            Box::new(MemStore { tag: "config", data: RefCell::new(Some(bad_blob)) }),
        ];
        let lr = load_from(stores, fp);
        assert!(lr.tampered, "relocated blob => tamper");
        assert!(!lr.needs_heal, "tampered => never heal");
    }

    // ----- --license 多存储位置兼容解密 -----

    #[test]
    fn decrypt_for_display_accepts_store_ciphertext() {
        let fp: &[u8] = b"machine-fp";
        let t = signed_trial(20622, 20630, 0, fp);
        // local 点的内部格式密文（即 cache.bin 中的内容）
        let store_pt = build_plaintext(&t, &None, "local");
        let local_enc = super::super::crypto::aes256gcm_encrypt_for(store_pt.as_bytes(), fp, b"local")
            .expect("encrypt_for local");

        // --license 应能解密（无论来自哪个存储点），并输出导出格式
        let out = decrypt_for_display(&local_enc, fp).expect("should decrypt store ciphertext");
        assert!(out.starts_with("f=20622|l=20630|"), "export format: {out}");
        assert!(out.contains("mac="));
        // 导出格式不含内部字段 loc/mani（已规范化）
        assert!(!out.contains("loc="));
        assert!(!out.contains("mani="));
    }

    #[test]
    fn decrypt_for_display_accepts_each_store_tag() {
        let fp: &[u8] = b"machine-fp";
        let t = signed_trial(100, 150, 0, fp);
        // 每个存储点的密文都应能被 --license 解密
        for tag in ["home", "config", "local", "regmain", "regclsid"] {
            let pt = build_plaintext(&t, &None, tag);
            let enc = super::super::crypto::aes256gcm_encrypt_for(pt.as_bytes(), fp, tag.as_bytes())
                .expect("encrypt_for");
            let out = decrypt_for_display(&enc, fp)
                .unwrap_or_else(|| panic!("should decrypt {tag} ciphertext"));
            assert!(out.starts_with("f=100|l=150|"), "[{tag}] export format: {out}");
        }
    }

    #[test]
    fn decrypt_for_display_accepts_export_blob() {
        let fp: &[u8] = b"machine-fp";
        let t = signed_trial(20622, 20630, 0, fp);
        // 导出 blob（无 tag，LicenseBlob 格式）
        let export_text = build_export_blob(&t, &None, fp);
        let blob_enc = super::super::crypto::aes256gcm_encrypt(export_text.as_bytes(), fp)
            .expect("tag-less encrypt");
        let out = decrypt_for_display(&blob_enc, fp).expect("should decrypt export blob");
        assert_eq!(out, export_text, "export blob round-trips unchanged");
    }

    #[test]
    fn decrypt_for_display_rejects_garbage_and_wrong_machine() {
        let fp: &[u8] = b"machine-fp";
        // 非法串
        assert!(decrypt_for_display("not-a-valid-blob", fp).is_none());
        // 用错误机器指纹加密的 local 密文（绑机）→ 解不开
        let other: &[u8] = b"other-machine";
        let t = signed_trial(100, 150, 0, other);
        let enc = super::super::crypto::aes256gcm_encrypt_for(
            build_plaintext(&t, &None, "local").as_bytes(),
            other,
            b"local",
        )
        .expect("encrypt_for");
        assert!(decrypt_for_display(&enc, fp).is_none(), "wrong-machine blob must fail");
    }

    /// 真实注册表往返：用 CLSID 风格子键（大写 + 花括号，与生产 regclsid 同格式）验证
    /// 「写入的密文能从同一路径读回」——即改 UUID 格式后 regclsid 读密文功能正常。
    #[cfg(windows)]
    #[test]
    fn regstore_roundtrip_under_clsid_subkey() {
        use winreg::enums::*;
        use winreg::RegKey;
        let sub = format!(
            "Software\\Classes\\CLSID\\{}__rt_test",
            crate::license::fingerprint::registry_uuid_clsid()
        );
        let store = RegStore { tag: "regclsid", subkey: sub.clone() };
        let payload = "Y2lwaGVydGV4dA=="; // 任意密文形态
        store.write(payload);
        let back = store.read();
        // 清理临时键（无论断言是否通过都先清理）
        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey(&sub);
        assert_eq!(
            back.as_deref(),
            Some(payload),
            "RegStore must round-trip (read==write) under CLSID-style subkey"
        );
    }
}
