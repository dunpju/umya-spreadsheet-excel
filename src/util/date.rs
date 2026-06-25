//! 日期工具（不依赖 chrono）
//!
//! 从 `main.rs` 抽离复用：基于 Unix epoch 天数换算年月日。
//! license 模块也复用本模块，避免重复实现与 chrono 依赖。

/// 将 Unix 天数（自 1970-01-01 起）转换为 (年, 月, 日)
pub fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let leap = is_leap(year);
    let month_days: [u64; 12] = if leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 1u64;
    for &md in &month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }
    (year, month, days + 1)
}

/// 是否闰年
pub fn is_leap(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// 当前时间戳（14 位）：`yyyymmddhhmmss`（年月日时分秒）。
///
/// 基于 Unix epoch 秒数换算，复用 [`days_to_ymd`] 计算日期部分，再由日内剩余秒数
/// 推导时分秒，全程不依赖 chrono（与 `license` / `viewer` 的换算口径一致，均为 UTC）。
/// 用于导入文件备份命名（`原文件名_yyyymmddhhmmss.ext`）等需要精确到秒的场景。
pub fn now_timestamp14() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, m, d) = days_to_ymd(secs / 86400);
    let day_secs = secs % 86400;
    let h = day_secs / 3600;
    let min = (day_secs % 3600) / 60;
    let s = day_secs % 60;
    format!("{:04}{:02}{:02}{:02}{:02}{:02}", y, m, d, h, min, s)
}
