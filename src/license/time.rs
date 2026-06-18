//! 时间工具：当前 epoch 天数（复用 SystemTime，无 chrono）

/// 当前自 1970-01-01 起的天数
pub fn today_epoch_day() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86400
}

/// epoch 天数 → 友好日期串（如 2026-06-18），用于 UI 显示到期日
#[allow(dead_code)]
pub fn day_to_ymd_string(day: u64) -> String {
    let (y, m, d) = crate::util::date::days_to_ymd(day);
    format!("{:04}-{:02}-{:02}", y, m, d)
}
