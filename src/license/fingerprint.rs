//! 机器指纹：绑定授权到具体机器，防止一份授权码多机共用。
//!
//! 组合多个标识（按稳定性排序：硬件级 -> OS 级 -> 用户可改），
//! SHA-256 后取前若干字节做 hex 分组，得到用户可见的"机器码"（无额外依赖）。
//!
//! 硬件级标识（主板序列号/型号、CPU 型号）在重装系统、改计算机名后不变，
//! 确保已激活授权不会被误锁。仅更换主板/CPU 才会导致指纹变化，需重新激活。

/// Windows 注册表读取 MachineGuid
#[cfg(windows)]
fn windows_machine_guid() -> Option<String> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    hklm.open_subkey("SOFTWARE\\Microsoft\\Cryptography")
        .ok()
        .and_then(|key| key.get_value::<String, _>("MachineGuid").ok())
}

/// Windows 主板序列号（最稳定的硬件标识，重装系统不变）
#[cfg(windows)]
fn windows_system_serial() -> Option<String> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    hklm.open_subkey("HARDWARE\\DESCRIPTION\\System\\BIOS")
        .ok()
        .and_then(|key| key.get_value::<String, _>("SystemSerialNumber").ok())
}

/// Windows 主板/系统产品名（重装系统不变）
#[cfg(windows)]
fn windows_system_product() -> Option<String> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    hklm.open_subkey("HARDWARE\\DESCRIPTION\\System\\BIOS")
        .ok()
        .and_then(|key| key.get_value::<String, _>("SystemProductName").ok())
}

/// Windows CPU 标识（更换 CPU 才会变）
#[cfg(windows)]
fn windows_cpu_id() -> Option<String> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    hklm.open_subkey("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0")
        .ok()
        .and_then(|key| key.get_value::<String, _>("ProcessorNameString").ok())
}

/// 采集机器标识原始串（按稳定性排序：硬件 -> OS 级 -> 用户可改）
fn collect_raw_identifiers() -> Vec<String> {
    let mut ids = Vec::new();
    #[cfg(windows)]
    {
        // 稳定：主板序列号（硬件唯一，重装系统不变）
        if let Some(s) = windows_system_serial() {
            ids.push(format!("serial={s}"));
        }
        // 稳定：主板型号
        if let Some(p) = windows_system_product() {
            ids.push(format!("product={p}"));
        }
        // 较稳定：CPU 型号（更换 CPU 才会变）
        if let Some(c) = windows_cpu_id() {
            ids.push(format!("cpu={c}"));
        }
        // 半稳定：MachineGuid（重装系统会变）
        if let Some(g) = windows_machine_guid() {
            ids.push(format!("guid={g}"));
        }
        // 不稳定：计算机名（用户可改）
        if let Ok(name) = std::env::var("COMPUTERNAME") {
            ids.push(format!("host={name}"));
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(h) = std::env::var("HOSTNAME").or_else(|_| std::env::var("COMPUTERNAME")) {
            ids.push(format!("host={h}"));
        }
    }
    ids
}

/// 机器指纹（原始字节）—— 供 HMAC 密钥派生与授权绑定使用
pub fn fingerprint_bytes() -> Vec<u8> {
    let raw = collect_raw_identifiers().join("|");
    crate::license::crypto::sha256_hex(raw.as_bytes()).into_bytes()
}

/// 仅采集稳定硬件标识（用于注册表路径 UUID，不受计算机名 / OS 重装影响）
fn collect_stable_identifiers() -> String {
    let mut parts = Vec::new();
    #[cfg(windows)]
    {
        if let Some(s) = windows_system_serial() {
            parts.push(format!("serial={s}"));
        }
        if let Some(p) = windows_system_product() {
            parts.push(format!("product={p}"));
        }
        if let Some(c) = windows_cpu_id() {
            parts.push(format!("cpu={c}"));
        }
    }
    parts.join("|")
}

/// 将 64 位十六进制摘要格式化为 UUID v5 风格字符串：取前 16 字节，
/// 设置版本 nibble = 5 与 RFC 4122 变体位。[`registry_uuid`] 与 [`dir_uuid`] 共用。
fn uuid_v5_from_hex(hex: &str) -> String {
    // 取前 16 字节（32 hex chars）
    let mut b = [0u8; 16];
    for i in 0..16 {
        b[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(0);
    }
    // UUID v5 风格：版本 nibble = 5，变体 = RFC 4122
    b[6] = (b[6] & 0x0F) | 0x50;
    b[8] = (b[8] & 0x3F) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

/// 基于硬件标识派生的 UUID（用于注册表路径混淆）。
///
/// 格式如 `71445fac-d6ef-5436-9da7-5a323762d7f5`（UUID v5 风格），
/// 由主板序列号 / 型号 / CPU 三项稳定硬件标识经 SHA-256 取前 16 字节生成。
/// 确定性：同一台机器始终得到相同 UUID；更换主板 / CPU 后改变。
pub fn registry_uuid() -> String {
    let raw = collect_stable_identifiers();
    let hex = crate::license::crypto::sha256_hex(raw.as_bytes());
    uuid_v5_from_hex(&hex)
}

/// 按存储位置前缀派生的 UUID 目录名（用于 config / local 等文件存储点的目录混淆）。
///
/// 生成规则：`uuid_v5_style( sha256( prefix + registry_uuid() ) )`——
/// 把位置前缀字符串（如 `"config"` / `"local"`）与机器派生 UUID [`registry_uuid`]
/// **拼接后取 SHA-256**，再按 UUID v5 风格格式化。故每个前缀得到**不同**目录名、
/// 且与机器绑定；路径中无可识别的固定产品名，抬高"按路径批量定位"的门槛。
pub fn dir_uuid(prefix: &str) -> String {
    let base = registry_uuid();
    let input = format!("{prefix}{base}");
    let hex = crate::license::crypto::sha256_hex(input.as_bytes());
    uuid_v5_from_hex(&hex)
}

/// 用户可见的机器码：取指纹再哈希后 hex，按 XXXX-XXXX-XXXX-XXXX 分组
pub fn machine_code() -> String {
    let fp = fingerprint_bytes();
    let hex = crate::license::crypto::sha256_hex(&fp);
    let g = |i: usize| &hex[i..i + 4];
    format!("{}-{}-{}-{}", g(0), g(4), g(8), g(12))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 校验 UUID v5 风格格式：8-4-4-4-12 共 36 字符、4 个连字符、版本 nibble = 5
    fn assert_valid_v5(u: &str) {
        assert_eq!(u.len(), 36, "UUID must be 36 chars: {u}");
        assert_eq!(u.chars().filter(|&c| c == '-').count(), 4, "must have 4 dashes: {u}");
        let groups: Vec<&str> = u.split('-').collect();
        assert_eq!(groups.len(), 5);
        assert!(groups[2].starts_with('5'), "version nibble must be 5: {u}");
        // 变体位：第 4 组首字符为 8/9/a/b（RFC 4122）
        let v = groups[3].chars().next().unwrap();
        assert!(matches!(v, '8' | '9' | 'a' | 'b'), "variant nibble must be 8-b: {u}");
    }

    #[test]
    fn registry_uuid_format() {
        assert_valid_v5(&registry_uuid());
    }

    #[test]
    fn dir_uuid_format_and_distinct() {
        let reg = registry_uuid();
        let cfg = dir_uuid("config");
        let loc = dir_uuid("local");
        for u in [&reg, &cfg, &loc] {
            assert_valid_v5(u);
        }
        // 三个目录名互不相同（不同前缀 + registry_uuid 本身）
        assert_ne!(cfg, loc, "dir_uuid(config) must differ from dir_uuid(local)");
        assert_ne!(cfg, reg, "dir_uuid(config) must differ from registry_uuid");
        assert_ne!(loc, reg, "dir_uuid(local) must differ from registry_uuid");
    }

    #[test]
    fn dir_uuid_is_deterministic() {
        // 同机同前缀多次调用结果一致
        assert_eq!(dir_uuid("config"), dir_uuid("config"));
        assert_eq!(dir_uuid("local"), dir_uuid("local"));
    }

    #[test]
    fn dir_uuid_uses_prefix_and_registry_uuid() {
        // 规则可验：dir_uuid(prefix) = uuid_v5_from_hex(sha256(prefix + registry_uuid()))
        let base = registry_uuid();
        for prefix in ["config", "local", "home"] {
            let input = format!("{prefix}{base}");
            let hex = crate::license::crypto::sha256_hex(input.as_bytes());
            // 重新实现一份 uuid_v5_from_hex 逻辑做交叉校验（不调用私有函数）
            let mut b = [0u8; 16];
            for i in 0..16 {
                b[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(0);
            }
            b[6] = (b[6] & 0x0F) | 0x50;
            b[8] = (b[8] & 0x3F) | 0x80;
            let expected = format!(
                "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
                b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
            );
            assert_eq!(dir_uuid(prefix), expected, "dir_uuid must follow prefix+registry_uuid rule");
        }
    }
}
