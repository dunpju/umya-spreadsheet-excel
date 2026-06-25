# `util/open.rs` 文档

## 1. 模块职责

`src/util/open.rs` 提供**用系统默认程序打开文件/路径**的能力，对外仅暴露一个 `pub fn`：
[`open_in_default_app`](#open_in_default_app)。它是项目内**唯一**的"打开外部文件"机制——
此前代码库无任何 `open`/`webbrowser`/`ShellExecute`/`process::Command` 启动外部进程的先例。

模块为**纯工具模块**：不依赖 GUI、不持有状态、跨平台分支以 `#[cfg(target_os=...)]` 门控。
设计目标有二：① **零新增依赖**（与项目"最小依赖 + 警惕 `windows` crate 编译崩溃"的约定一致）；
② **Windows 路径处理最稳**（`ShellExecuteW` + UTF-16，原生支持任意 Unicode / 空格 / 特殊字符）。

## 2. 主要函数

### `open_in_default_app`

```rust
pub fn open_in_default_app(path: &std::path::Path) -> std::io::Result<()>
```

按平台用系统默认程序打开 `path`，**非阻塞**（仅发起启动即返回，不 `wait` 目标程序退出），
故可直接在 UI 线程调用，不会卡帧。

**参数**

| 参数 | 类型 | 说明 |
|------|------|------|
| `path` | `&Path` | 要打开的文件路径 |

**返回**

- `Ok(())`：已成功发起启动。
- `Err(io::Error)`：启动失败。Windows 下 `ShellExecuteW` 返回值 `≤ 32` 视为失败
  （文件不存在 / 无关联程序等）；macOS/Linux 下为 `Command::spawn` 的底层错误。

## 3. 核心逻辑与数据流（平台分支）

```
open_in_default_app(path)
 │
 ├─ #[cfg(windows)]      open_windows(path.as_os_str())
 │      ├─ OsStr → UTF-16（encode_wide），保留任意 Unicode/空格/反斜杠
 │      ├─ verb = "open"，show = SW_SHOWNORMAL(1)
 │      └─ ShellExecuteW(null, "open", path, null, null, 1)
 │           └─ HINSTANCE ≤ 32 ⟹ Err；否则 Ok
 │
 ├─ #[cfg(macos)]        spawn("open", path)              // Command::spawn，不 wait
 ├─ #[cfg(linux/其它unix)] spawn("xdg-open", path)         // 同上
 └─ #[cfg(其它)]         Err(Unsupported)                  // 兜底，项目实际仅构建于 Windows
```

各平台分支互斥（`macos` 与 `unix` 通过 `not(target_os = "macos")` 去重），任何目标恰好激活一条；
`open_windows` / `spawn` 两个辅助函数各自 `#[cfg]` 到仅使用它的平台，故无 dead_code。

## 4. Windows 实现要点（`open_windows`）

- **FFI 写法**：`#[link(name = "shell32")] extern "system" { fn ShellExecuteW(...) }`，与
  [`shortcut.rs`](../../shortcut.rs) 的 `#[link(name = "ole32")] extern "system"`（COM / `CoCreateInstance`）
  **逐字同款**，无需引入 `winapi`/`windows` crate。
- **Unicode**：路径经 `std::os::windows::ffi::OsStrExt::encode_wide` 转 UTF-16（NUL 结尾），
  原生支持中文等任意 Unicode——这是选 `ShellExecuteW` 而非 `cmd /C start` 的核心原因
  （`cmd` 受系统代码页约束，中文路径易乱码）。
- **不经 shell 解析**：路径作为单个 UTF-16 字符串传入，**空格、`& ( ) ^ %` 等特殊字符全免疫**
  （`cmd start` 需 `start "" "..."` 占位且仍怕元字符）。
- **返回值判定**：Win32 约定 `ShellExecuteW` 的 `HINSTANCE ≤ 32` 表示错误（如 `SE_ERR_NOASSOC = 31`
  表示无关联程序），据此返回 `Err`。
- **路径分隔符**：`rfd` 给出的路径在 Windows 上已是反斜杠原生形式，无需转换；即便混入正斜杠
  `ShellExecuteW` 也能容忍。

## 5. 依赖关系

- **对外依赖**：
  - Windows：`shell32`（系统库，`ShellExecuteW`）+ `std::os::windows::ffi`。
  - macOS/Linux：`std::process::Command` + 系统自带 `open` / `xdg-open`。
- **被依赖**：[`gui/viewer.rs`](../gui/viewer.md) 的底部状态栏——保存完成后显示的**绿色文件路径**
  可点击，点击时调用本函数用系统默认程序打开该文件。

## 6. 与状态栏点击交互的关系

```
viewer.rs status_bar（右下角绿色 save_path）
   ▼ Label::sense(Sense::click()) + Response::clicked()
   ▼ open_in_default_app(Path::new(&save_path))
   ▼ Windows: ShellExecuteW("open", path)  ──► 系统默认程序（如 Excel）打开文件
```

非阻塞：`ShellExecuteW` / `Command::spawn` 均不 `wait`，UI 线程调用不卡帧。
