use eframe::NativeOptions;
use std::backtrace::Backtrace;
use std::io::Write;
use std::path::PathBuf;

mod excel;
mod gui;

use gui::viewer::ExcelViewer;

/// 将崩溃信息（panic 信息 + 调用栈）写入日志文件，并弹窗提示用户
fn handle_panic(info: &std::panic::PanicHookInfo) {
    // 收集崩溃信息
    let payload = info.payload();
    let message = if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "Unknown panic payload".to_string()
    };

    let location = info.location().map(|loc| {
        format!("{}:{}:{}", loc.file(), loc.line(), loc.column())
    }).unwrap_or_else(|| "unknown location".to_string());

    let backtrace = Backtrace::capture();

    let log_content = format!(
        "===== 程序崩溃 =====\n\
         时间: {}\n\
         位置: {}\n\
         信息: {}\n\
         \n\
         调用栈:\n\
         {}\n",
        chrono_free_timestamp(),
        location,
        message,
        backtrace,
    );

    // 写入日志文件（与 exe 同目录下，按日期命名如 crash-20260609.log）
    let (year, month, day) = days_to_ymd(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() / 86400);
    let log_filename = format!("crash-{:04}{:02}{:02}.log", year, month, day);
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let log_path = exe_dir.join(&log_filename);

    let log_display = log_path.display().to_string();
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .and_then(|mut f| {
            writeln!(f, "{}", log_content)?;
            Ok(())
        });

    // 弹窗提示
    let msg = format!(
        "程序发生崩溃，日志已保存至：\n{}\n\n崩溃信息：{}",
        log_display, message
    );
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("程序崩溃")
        .set_description(&msg)
        .show();
}

/// 获取简易时间戳（不依赖 chrono crate）
fn chrono_free_timestamp() -> String {
    let now = std::time::SystemTime::now();
    let duration = now.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs();
    // 简易计算日期时间（不处理闰秒等边缘情况）
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;
    // 计算年月日（从 1970-01-01 起）
    let (year, month, day) = days_to_ymd(days);
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC", year, month, day, hours, minutes, seconds)
}

/// 将 Unix 天数转换为年月日
fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year { break; }
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
        if days < md { break; }
        days -= md;
        month += 1;
    }
    (year, month, days + 1)
}

fn is_leap(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn main() -> eframe::Result<()> {
    // 注册崩溃处理：捕获 panic 并记录调用栈到 crash.log
    std::panic::set_hook(Box::new(handle_panic));

    // 启用调用栈捕获（release 模式下默认不捕获）
    std::env::set_var("RUST_BACKTRACE", "1");

    let options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "My Excel",
        options,
        Box::new(|_cc| Ok(Box::new(ExcelViewer::new()))),
    )
}
