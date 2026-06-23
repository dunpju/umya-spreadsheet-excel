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
mod shortcut;

use gui::viewer::ExcelViewer;
use util::date::days_to_ymd;

/// 加载窗口图标（标题栏 + Windows 任务栏），返回 egui::IconData。
///
/// 复用 `image` crate（png feature）将内嵌的精确 256px `assets/icon-v3-256.png`
/// 解码为 RGBA，封装为 `egui::IconData`，再经 `ViewportBuilder::with_icon` 设置；
/// eframe 在 Windows 上据此调用 `WM_SETICON`，标题栏与任务栏图标同源。
/// 与 `build.rs`（winresource 把 icon.ico 嵌入 .exe 资源）互补：后者覆盖
/// 资源管理器 / 跳转列表 / 运行前任务栏，本函数覆盖运行时窗口图标。
/// 解码失败时返回 None（仅无图标，不影响功能，不阻塞启动）。
fn load_window_icon() -> Option<egui::IconData> {
    const ICON_PNG: &[u8] = include_bytes!("../assets/icon-v3-256.png");
    let img = image::load_from_memory(ICON_PNG).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    })
}

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

/// 输出一段文本到控制台（仅 `diagnostic` feature 构建编译，与 §6 的三个诊断子命令同进退）。
///
/// GUI 子系统下程序默认没有控制台，println! 不会显示。这里先尝试附加到
/// 父进程的控制台（从终端运行 --uuid / --stores / --license 时有效），再写入 CONOUT$。
/// 附加失败（如双击运行）则静默忽略。输出均为 ASCII，无控制台编码问题。
#[cfg(all(windows, feature = "diagnostic"))]
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

#[cfg(all(not(windows), feature = "diagnostic"))]
fn console_print(msg: &str) {
    use std::io::Write;
    let _ = std::io::stdout().write_all(msg.as_bytes());
}

#[cfg(feature = "diagnostic")]
/// 构建 `--stores` 输出：列出本机实际解析后的全部存储点路径。
///
/// 对应 `docs/gui/widgets/license.md` §6.1「存储位置」的全部 5 个存储点（home / config / local /
/// regmain / regclsid），把文档占位符（`~` / `{config_dir}` / `{data_local_dir}` /
/// `{uuid}` / `{dir_uuid(...)}`）解析为当前系统的实际绝对路径后输出（非占位符字面量）。
/// 路径派生原语与 `license::store::all_stores()` 一致，故输出即程序实际读写的路径。
/// 标签沿用 doc 中的 tag 名（ASCII，避开控制台编码问题），统一 `{:<9}` 左对齐路径列。
#[cfg(feature = "diagnostic")]
fn format_store_paths() -> String {
    let mut out = String::new();

    // —— 文件存储点（best-effort；dirs 返回 None 时打印兜底文案）——
    // home：用户主目录下的 license 文件（~/.MyExcel/license.dat）
    match dirs::home_dir() {
        Some(home) => {
            let path = home.join(".MyExcel").join("license.dat");
            out.push_str(&format!("{:<9} {}\n", "home:", path.display()));
        }
        None => out.push_str(&format!("{:<9} (unavailable: home directory not found)\n", "home:")),
    }
    // config：配置目录下的分散点（{config_dir}/{dir_uuid(config)}/state.dat）
    match dirs::config_dir() {
        Some(cfg) => {
            let path = cfg
                .join(license::fingerprint::dir_uuid("config"))
                .join("state.dat");
            out.push_str(&format!("{:<9} {}\n", "config:", path.display()));
        }
        None => out.push_str(&format!("{:<9} (unavailable: config directory not found)\n", "config:")),
    }
    // local：本地应用数据缓存（{data_local_dir}/{dir_uuid(local)}/cache.bin）
    match dirs::data_local_dir() {
        Some(loc) => {
            let path = loc
                .join(license::fingerprint::dir_uuid("local"))
                .join("cache.bin");
            out.push_str(&format!("{:<9} {}\n", "local:", path.display()));
        }
        None => out.push_str(&format!("{:<9} (unavailable: local data directory not found)\n", "local:")),
    }

    // —— 注册表存储点（仅 Windows；非 Windows 无此两点）——
    #[cfg(windows)]
    {
        // regmain：HKCU\Software\{uuid}
        out.push_str(&format!(
            "{:<9} HKCU\\Software\\{}\n",
            "regmain:",
            license::fingerprint::registry_uuid()
        ));
        // regclsid：HKCU\Software\Classes\CLSID\{大写UUID}（CLSID 惯例：大写 + 花括号）
        out.push_str(&format!(
            "{:<9} HKCU\\Software\\Classes\\CLSID\\{}\n",
            "regclsid:",
            license::fingerprint::registry_uuid_clsid()
        ));
    }
    #[cfg(not(windows))]
    {
        out.push_str(&format!("{:<9} (N/A on non-Windows)\n", "regmain:"));
        out.push_str(&format!("{:<9} (N/A on non-Windows)\n", "regclsid:"));
    }

    out
}

fn main() -> eframe::Result<()> {
    // 诊断 CLI 子命令（--uuid / --stores / --license）：仅 `diagnostic` feature 构建编译。
    // 这些命令会暴露本机存储路径 / UUID / 授权状态，公开发布版（build.bat）默认不带此 feature，
    // 以免逆向者 `strings` 出 "--stores" 等字面量并借其一次性枚举全部存储点（见 docs/main.md §6）。
    #[cfg(feature = "diagnostic")]
    {
        // CLI: --uuid 打印本机注册表路径 UUID 后退出
        if std::env::args().any(|a| a == "--uuid") {
            console_print(&format!("{}\n", license::fingerprint::registry_uuid()));
            std::process::exit(0);
        }
        // CLI: --stores 打印本机实际解析后的全部存储点路径（home / config / local / regmain / regclsid）后退出
        if std::env::args().any(|a| a == "--stores") {
            console_print(&format_store_paths());
            std::process::exit(0);
        }
        // CLI: --license 'encrypted_string' 解密并输出授权状态信息后退出
        {
            let args: Vec<String> = std::env::args().collect();
            if let Some(pos) = args.iter().position(|a| a == "--license") {
                if pos + 1 < args.len() {
                    let encrypted = &args[pos + 1];
                    let machine_fp = license::fingerprint::fingerprint_bytes();
                    // 兼容多种来源：LicenseBlob 导出（无 tag）或任一存储点的分位置密文
                    // （home/config/local/regmain/regclsid）。详见 store::decrypt_for_display。
                    match license::store::decrypt_for_display(encrypted, &machine_fp) {
                        Some(text) => {
                            console_print(&format!("{}\n", text));
                            std::process::exit(0);
                        }
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
    }
    // 注册崩溃处理：捕获 panic 并记录调用栈到 crash.log
    std::panic::set_hook(Box::new(handle_panic));

    // 启用调用栈捕获（release 模式下默认不捕获）
    std::env::set_var("RUST_BACKTRACE", "1");

    // 确保桌面存在指向本程序的 my-excel.lnk 快捷方式（缺失才创建；best-effort，失败不阻塞启动）。
    // 仅 Windows 生效，非 Windows 平台为空操作。
    shortcut::ensure_desktop_shortcut();

    let mut viewport = egui::ViewportBuilder::default().with_inner_size([1200.0, 800.0]);
    // 设置窗口图标（Windows 上同时作用于标题栏与任务栏）；解码失败则回退默认图标。
    if let Some(icon) = load_window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "My Excel",
        options,
        Box::new(|_cc| Ok(Box::new(ExcelViewer::new()))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `--stores` 必须把占位符（`~` / `{config_dir}` / `{data_local_dir}` / `{uuid}` /
    /// `{dir_uuid(...)}`）解析为当前系统的**实际绝对路径**，而非输出占位符字面量
    ///（5 个存储点：home / config / local / regmain / regclsid，见 docs/gui/widgets/license.md §6.1）。
    #[cfg(feature = "diagnostic")]
    #[test]
    fn format_store_paths_resolves_real_paths() {
        let out = format_store_paths();

        // 文件存储点行均存在
        let home_line = out.lines().find(|l| l.starts_with("home:"));
        let config_line = out.lines().find(|l| l.starts_with("config:"));
        let local_line = out.lines().find(|l| l.starts_with("local:"));
        assert!(home_line.is_some(), "home line present:\n{out}");
        assert!(config_line.is_some(), "config line present:\n{out}");
        assert!(local_line.is_some(), "local line present:\n{out}");

        // home 行：解析为实际主目录（非 `~` 占位符）
        if let Some(home) = dirs::home_dir() {
            let line = home_line.unwrap();
            assert!(
                line.contains(&home.display().to_string()),
                "home resolves to real home dir, not placeholder: {line}"
            );
            assert!(!line.contains('~'), "no literal ~ placeholder: {line}");
        }

        // config 行：嵌入实际的 dir_uuid(config)，解析为实际 config 目录，以 state.dat 结尾
        if let Some(cfg) = dirs::config_dir() {
            let line = config_line.unwrap();
            assert!(
                line.contains(&cfg.display().to_string()),
                "config resolves to real config dir, not placeholder: {line}"
            );
            assert!(
                line.contains(&license::fingerprint::dir_uuid("config")),
                "config embeds real dir_uuid(\"config\"): {line}"
            );
            assert!(line.ends_with("state.dat"), "config ends with state.dat: {line}");
            assert!(!line.contains("{dir_uuid"), "no literal {{dir_uuid}} placeholder: {line}");
        }

        // local 行：嵌入实际的 dir_uuid(local)，以 cache.bin 结尾（非占位符字面量）
        let line = local_line.unwrap();
        assert!(
            line.contains(&license::fingerprint::dir_uuid("local")),
            "local embeds real dir_uuid(\"local\"): {line}"
        );
        assert!(line.ends_with("cache.bin"), "local ends with cache.bin: {line}");
        assert!(
            !line.contains("{dir_uuid"),
            "no literal {{dir_uuid(...)}} placeholder: {line}"
        );

        // 注册表存储点（仅 Windows）：regmain / regclsid 均嵌入实际派生 UUID
        #[cfg(windows)]
        {
            // regmain：HKCU\Software\{registry_uuid()}
            let line = out.lines().find(|l| l.starts_with("regmain:")).unwrap();
            assert!(
                line.contains(&license::fingerprint::registry_uuid()),
                "regmain embeds real registry_uuid: {line}"
            );
            assert!(
                line.contains("HKCU\\Software\\"),
                "regmain has HKCU\\Software\\ prefix: {line}"
            );
            assert!(!line.contains("{uuid}"), "no literal {{uuid}} placeholder: {line}");

            // regclsid：HKCU\Software\Classes\CLSID\{registry_uuid_clsid()}
            let line = out.lines().find(|l| l.starts_with("regclsid:")).unwrap();
            assert!(
                line.contains(&license::fingerprint::registry_uuid_clsid()),
                "regclsid embeds real registry_uuid_clsid: {line}"
            );
            assert!(
                line.contains("HKCU\\Software\\Classes\\CLSID\\"),
                "regclsid has HKCU\\Software\\Classes\\CLSID\\ prefix: {line}"
            );
        }
    }
}
