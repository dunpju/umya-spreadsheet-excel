//! 用系统默认程序打开文件（零额外依赖）。
//!
//! 提供 [`open_in_default_app`]：按平台调用系统默认程序打开指定路径——
//! Windows 走 `ShellExecuteW`（UTF-16 原生支持任意 Unicode / 空格 / 特殊字符路径，
//! 不经 shell 解析），macOS 走 `open`，Linux 走 `xdg-open`。全程仅"发起启动"，
//! 不 `wait` 目标程序退出，**非阻塞**，可在 UI 线程直接调用。

use std::path::Path;

/// 用系统默认程序打开文件 / 路径。
///
/// 非阻塞：仅发起启动即返回，不等待目标程序退出，不会卡住调用线程。
///
/// # 参数
/// * `path` - 要打开的文件路径
///
/// # 返回
/// 成功返回 `Ok(())`；失败返回底层 IO 错误——Windows 下 `ShellExecuteW`
/// 返回值 `≤ 32` 视为失败（如文件不存在 / 无关联程序）。
pub fn open_in_default_app(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        return open_windows(path.as_os_str());
    }

    #[cfg(target_os = "macos")]
    {
        return spawn("open", path);
    }

    // 其余类 Unix（Linux / FreeBSD 等）
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return spawn("xdg-open", path);
    }

    // 兜底：未覆盖平台（项目实际仅构建于 Windows，此处理论不可达）
    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    {
        let _ = path;
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "当前平台不支持用系统默认程序打开文件",
        ))
    }
}

/// Unix 系（macOS / Linux）：spawn 对应启动器，不 `wait`（非阻塞）。
#[cfg(any(target_os = "macos", all(unix, not(target_os = "macos"))))]
fn spawn(prog: &str, path: &Path) -> std::io::Result<()> {
    std::process::Command::new(prog).arg(path).spawn().map(|_| ())
}

/// Windows：`ShellExecuteW`（verb = `"open"`）。
///
/// 路径经 [`OsStrExt::encode_wide`] 转 UTF-16，原生保留任意 Unicode、反斜杠、
/// 空格与特殊字符（不经过 `cmd` 解析，规避代码页 / 元字符问题）。FFI 写法与
/// `shortcut.rs` 的 `#[link(name = "ole32")] extern "system"` 一致，零新增依赖。
#[cfg(target_os = "windows")]
fn open_windows(path: &std::ffi::OsStr) -> std::io::Result<()> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(
            hwnd: *mut c_void,
            op: *const u16,
            file: *const u16,
            params: *const u16,
            dir: *const u16,
            show: i32,
        ) -> *mut c_void;
    }

    /// `&str` → NUL 结尾的 UTF-16
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    // OsStr → UTF-16（NUL 结尾）：保留任意 Unicode / 反斜杠 / 空格
    let mut file: Vec<u16> = path.encode_wide().collect();
    file.push(0);
    let op = wide("open"); // verb

    const SW_SHOWNORMAL: i32 = 1;
    // Win32 约定：返回的 HINSTANCE ≤ 32 表示错误
    let hinstance = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if (hinstance as isize) <= 32 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("ShellExecuteW 失败 (错误码 {})", hinstance as isize),
        ));
    }
    Ok(())
}
