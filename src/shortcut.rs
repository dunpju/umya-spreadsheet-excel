//! 桌面快捷方式：启动时确保 Windows 桌面存在指向本程序的 `my-excel.lnk`。
//!
//! # 设计原则
//! - **best-effort**：快捷方式是“锦上添花”，任何失败（取不到 exe 路径、桌面不可写、
//!   COM 调用失败）一律静默忽略，**绝不阻塞或中断启动**。
//! - **缺失才创建**：先做一次廉价的文件存在检测；已存在直接返回，正常启动几乎零开销
//!   （不初始化 COM）。只有真正缺失时才走 COM 生成。
//! - **平台隔离**：全部 Windows 专属逻辑用 `#[cfg(target_os = "windows")]` 门控，
//!   非 Windows 平台编译为空操作，保证跨平台可编译。
//!
//! # 实现方式
//! 通过原始 COM FFI（`IShellLinkW` + `IPersistFile`）生成标准 `.lnk`，**零新增依赖**——
//! 与 `main.rs` 中 `console_print` 的 `AttachConsole` FFI 风格一致，避免引入
//! `windows` / `winapi` crate 的版本与 feature 门控复杂度（项目对依赖与编译稳定性
//! 极为敏感，见 `Cargo.toml` 中对 `windows` crate 的 opt-level 规避说明）。
//!
//! 快捷方式图标无需显式 `SetIconLocation`：`build.rs` 已把 `icon.ico` 嵌入 exe 资源段，
//! Windows 资源管理器会自动以该资源作为 `.lnk` 的默认图标。

use std::path::PathBuf;

/// 快捷方式文件名（含扩展名，不含路径）。
const SHORTCUT_NAME: &str = "my-excel.lnk";

/// 启动入口：确保桌面存在 `my-excel.lnk`，缺失则创建。
///
/// 调用点：`main.rs` 在 CLI 分支、panic hook 之后、GUI 主窗口装配之前调用一次。
/// 全平台可调用——非 Windows 平台为空操作。
pub fn ensure_desktop_shortcut() {
    #[cfg(target_os = "windows")]
    {
        if let Err(()) = ensure_inner() {
            // best-effort：忽略错误。GUI 子系统下无控制台；快捷方式非核心功能。
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        // 非 Windows 平台无 .lnk 概念，直接跳过。
    }
}

#[cfg(target_os = "windows")]
fn ensure_inner() -> Result<(), ()> {
    // 1. 当前可执行文件实际路径（current_exe 已解析符号链接 / shim 后的真实位置）
    let exe = std::env::current_exe().map_err(|_| ())?;
    // 2. 桌面目录（优先 dirs，正确处理 OneDrive 重定向；回退 %USERPROFILE%\Desktop）
    let desktop = desktop_dir().ok_or(())?;
    // 3. 目标 .lnk 完整路径
    let lnk = desktop.join(SHORTCUT_NAME);
    // 4. 已存在则跳过（廉价检测，避免每次启动都初始化 COM）
    if lnk.exists() {
        return Ok(());
    }
    // 5. 工作目录 = exe 所在目录（保证程序启动后的相对路径基准正确）
    let work_dir = exe
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    // 6. COM 生成 .lnk
    if imp::create(&lnk, &exe, &work_dir) {
        Ok(())
    } else {
        Err(())
    }
}

/// 取当前用户桌面目录。
///
/// 优先 `dirs::desktop_dir()`：它走 Windows 已知文件夹（Known Folder）API，
/// 能正确处理 **OneDrive 桌面重定向**（Win10/11 极常见：桌面实际位于
/// `%USERPROFILE%\OneDrive\Desktop`）与本地化名称。回退到 `%USERPROFILE%\Desktop`，
/// 兼顾“未启用重定向 / dirs 不可用”的朴素情形。
#[cfg(target_os = "windows")]
fn desktop_dir() -> Option<PathBuf> {
    if let Some(d) = dirs::desktop_dir() {
        return Some(d);
    }
    std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join("Desktop"))
}

// ---------------------------------------------------------------------------
// Windows COM 原始 FFI：IShellLinkW + IPersistFile
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    type HRESULT = i32;
    type WCHAR = u16;

    const S_OK: HRESULT = 0;
    /// 0x80010106：线程已用不同并发模型初始化 COM —— 本函数无法继续。
    const RPC_E_CHANGED_MODE: HRESULT = -2_147_417_850i32;
    const COINIT_APARTMENTTHREADED: u32 = 0x2;
    const CLSCTX_INPROC_SERVER: u32 = 1;
    const TRUE: i32 = 1;

    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    /// CLSID_ShellLink = {00021401-0000-0000-C000-000000000046}
    const CLSID_SHELL_LINK: Guid = Guid {
        data1: 0x00021401,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };
    /// IID_IShellLinkW = {000214F9-0000-0000-C000-000000000046}
    const IID_ISHELL_LINKW: Guid = Guid {
        data1: 0x000214F9,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };
    /// IID_IPersistFile = {0000010B-0000-0000-C000-000000000046}
    const IID_IPERSIST_FILE: Guid = Guid {
        data1: 0x0000_010b,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(reserved: *const c_void, co_init: u32) -> HRESULT;
        fn CoUninitialize();
        fn CoCreateInstance(
            rclsid: *const Guid,
            punk_outer: *const c_void,
            clsctx: u32,
            riid: *const Guid,
            ppv: *mut *mut c_void,
        ) -> HRESULT;
    }

    #[repr(C)]
    struct IShellLinkW {
        vtbl: *const IShellLinkWVtbl,
    }

    /// `IShellLinkW` vtable：IUnknown(3) + IShellLinkW(18) 共 21 槽。
    /// 仅精确声明用到的方法（`query_interface` / `set_working_directory` /
    /// `set_path` / `release`），其余占位为不可调用的函数指针；
    /// `#[allow(dead_code)]` 抑制占位字段告警。
    #[repr(C)]
    #[allow(dead_code)]
    struct IShellLinkWVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> HRESULT,
        _add_ref: unsafe extern "system" fn(),
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        _get_path: unsafe extern "system" fn(),
        _get_id_list: unsafe extern "system" fn(),
        _set_id_list: unsafe extern "system" fn(),
        _get_description: unsafe extern "system" fn(),
        _set_description: unsafe extern "system" fn(),
        _get_working_directory: unsafe extern "system" fn(),
        set_working_directory: unsafe extern "system" fn(*mut c_void, *const WCHAR) -> HRESULT,
        _get_arguments: unsafe extern "system" fn(),
        _set_arguments: unsafe extern "system" fn(),
        _get_hotkey: unsafe extern "system" fn(),
        _set_hotkey: unsafe extern "system" fn(),
        _get_show_cmd: unsafe extern "system" fn(),
        _set_show_cmd: unsafe extern "system" fn(),
        _get_icon_location: unsafe extern "system" fn(),
        _set_icon_location: unsafe extern "system" fn(),
        _get_relative_path: unsafe extern "system" fn(),
        _resolve: unsafe extern "system" fn(),
        set_path: unsafe extern "system" fn(*mut c_void, *const WCHAR) -> HRESULT,
    }

    #[repr(C)]
    struct IPersistFile {
        vtbl: *const IPersistFileVtbl,
    }

    /// `IPersistFile` vtable：IUnknown(3) + IPersist::GetClassID(1) + IPersistFile(5) 共 9 槽。
    /// 仅用到 `release`（槽 2）与 `save`（槽 6）。
    #[repr(C)]
    #[allow(dead_code)]
    struct IPersistFileVtbl {
        _query_interface: unsafe extern "system" fn(),
        _add_ref: unsafe extern "system" fn(),
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        _get_class_id: unsafe extern "system" fn(),
        _is_dirty: unsafe extern "system" fn(),
        _load: unsafe extern "system" fn(),
        save: unsafe extern "system" fn(*mut c_void, *const WCHAR, i32) -> HRESULT,
        _save_completed: unsafe extern "system" fn(),
        _get_cur_file: unsafe extern "system" fn(),
    }

    /// 路径 → 以 0 结尾的 UTF-16 序列（COM 宽字符串）。
    fn to_wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    }

    /// 用 COM 生成 `.lnk`。成功返回 `true`，任一步失败返回 `false`（绝不 panic）。
    pub(super) fn create(lnk: &Path, exe: &Path, work_dir: &Path) -> bool {
        unsafe {
            // 主线程、GUI 启动前调用：通常首次初始化，返回 S_OK。
            // S_FALSE（线程已初始化过 COM）也继续；RPC_E_CHANGED_MODE 表示线程模型冲突 → 放弃。
            let hr = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED);
            if hr == RPC_E_CHANGED_MODE {
                return false;
            }
            let ok = create_inner(lnk, exe, work_dir);
            CoUninitialize();
            ok
        }
    }

    unsafe fn create_inner(lnk: &Path, exe: &Path, work_dir: &Path) -> bool {
        // 1. 创建 ShellLink 实例，取 IShellLinkW
        let mut psl_raw: *mut c_void = std::ptr::null_mut();
        if CoCreateInstance(
            &CLSID_SHELL_LINK,
            std::ptr::null(),
            CLSCTX_INPROC_SERVER,
            &IID_ISHELL_LINKW,
            &mut psl_raw,
        ) != S_OK
        {
            return false;
        }
        let psl = psl_raw as *mut IShellLinkW;
        let sl = (*psl).vtbl;

        // 2. 设置目标路径（exe 实际路径）——失败则释放并放弃
        let exe_w = to_wide(exe);
        if ((*sl).set_path)(psl as *mut c_void, exe_w.as_ptr()) != S_OK {
            ((*sl).release)(psl as *mut c_void);
            return false;
        }

        // 3. 设置工作目录（失败不致命，忽略返回值）
        let work_w = to_wide(work_dir);
        ((*sl).set_working_directory)(psl as *mut c_void, work_w.as_ptr());

        // 4. QueryInterface 取 IPersistFile（用于落盘）
        let mut ppf_raw: *mut c_void = std::ptr::null_mut();
        if ((*sl).query_interface)(psl as *mut c_void, &IID_IPERSIST_FILE, &mut ppf_raw) != S_OK {
            ((*sl).release)(psl as *mut c_void);
            return false;
        }
        let ppf = ppf_raw as *mut IPersistFile;
        let pf = (*ppf).vtbl;

        // 5. 保存 .lnk（TRUE = 保存后写回首选项列表）
        let lnk_w = to_wide(lnk);
        let saved = ((*pf).save)(ppf as *mut c_void, lnk_w.as_ptr(), TRUE) == S_OK;

        // 6. 释放（QueryInterface 各自 AddRef，故 IShellLinkW 与 IPersistFile 分别 Release）
        ((*pf).release)(ppf as *mut c_void);
        ((*sl).release)(psl as *mut c_void);
        saved
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    /// 验证 COM 路径确实生成合法 .lnk（写入临时目录，不触碰桌面，可重复运行）。
    #[test]
    fn create_lnk_via_com_to_temp() {
        let dir = std::env::temp_dir();
        let lnk = dir.join("umya-shortcut-smoke.lnk");
        let _ = std::fs::remove_file(&lnk);

        let exe = std::env::current_exe().expect("current_exe");
        let work = exe
            .parent()
            .map(std::path::PathBuf::from)
            .unwrap_or_default();

        assert!(imp::create(&lnk, &exe, &work), "COM IShellLinkW::Save should succeed");
        assert!(lnk.exists(), ".lnk should exist on disk");

        // ShellLinkHeader 首字段 HeaderSize = 0x0000004C (76)，小端字节序为 4C 00 00 00
        let bytes = std::fs::read(&lnk).expect("read lnk");
        assert!(bytes.len() >= 4, ".lnk too small");
        assert_eq!(&bytes[0..4], &[0x4C, 0x00, 0x00, 0x00], "HeaderSize magic = 0x4C");

        let _ = std::fs::remove_file(&lnk);
    }
}
