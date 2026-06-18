// 使用 Windows GUI 子系统：双击运行时不再弹出黑色控制台窗口（仅 Windows 生效）。
// 副作用：程序默认没有控制台，println!/eprintln! 无输出；命令行场景
// （--uuid / --license）改用 console_print 附加到父进程控制台。
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use eframe::NativeOptions;
use std::backtrace::Backtrace;
use std::io::Write;
use std::path::PathBuf;

mod excel;
mod gui;
mod util;
mod license;

use gui::viewer::ExcelViewer;
use util::date::days_to_ymd;

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

/// 输出一段文本到控制台。
///
/// GUI 子系统下程序默认没有控制台，println! 不会显示。这里先尝试附加到
/// 父进程的控制台（从终端运行 --uuid / --license 时有效），再写入 CONOUT$。
/// 附加失败（如双击运行）则静默忽略。输出均为 ASCII，无控制台编码问题。
#[cfg(windows)]
fn console_print(msg: &str) {
    use std::io::Write;
    extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } != 0 {
        if let Ok(mut out) = std::fs::OpenOptions::new().write(true).open("CONOUT$") {
            let _ = out.write_all(msg.as_bytes());
        }
    }
}

#[cfg(not(windows))]
fn console_print(msg: &str) {
    use std::io::Write;
    let _ = std::io::stdout().write_all(msg.as_bytes());
}

fn main() -> eframe::Result<()> {
    // CLI: --uuid 打印本机注册表路径 UUID 后退出
    if std::env::args().any(|a| a == "--uuid") {
        console_print(&format!("{}\n", license::fingerprint::registry_uuid()));
        std::process::exit(0);
    }
    // CLI: --license 'encrypted_string' 解密并输出授权状态信息后退出
    {
        let args: Vec<String> = std::env::args().collect();
        if let Some(pos) = args.iter().position(|a| a == "--license") {
            if pos + 1 < args.len() {
                let encrypted = &args[pos + 1];
                let machine_fp = license::fingerprint::fingerprint_bytes();
                match license::crypto::aes256gcm_decrypt(encrypted, &machine_fp) {
                    Some(plaintext) => match std::str::from_utf8(&plaintext) {
                        Ok(text) => {
                            console_print(&format!("{}\n", text));
                            std::process::exit(0);
                        }
                        Err(_) => {
                            console_print("Error: decrypted data is not valid UTF-8\n");
                            std::process::exit(1);
                        }
                    },
                    None => {
                        console_print("Error: invalid or tampered license string, or wrong machine\n");
                        std::process::exit(1);
                    }
                }
            } else {
                console_print("Error: --license requires an argument\n");
                std::process::exit(1);
            }
        }
    }
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
