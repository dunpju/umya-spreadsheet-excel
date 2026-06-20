# `src/main.rs` 实现文档

> 本文件是 `umya-spreadsheet-excel`（可执行名 `my-excel`）的程序入口。
> 它本身代码量很小（约 200 行），但承担了三件关键职责：**CLI 诊断子命令**、**崩溃捕获与日志**、**GUI 主窗口的装配与启动**。本文从设计意图出发，逐块解释其实现逻辑。

---

## 1. 文件定位与职责

`main.rs` 是 `[[bin]] name = "my-excel"` 的入口（见 `Cargo.toml`），一个基于 `eframe`/`egui` 的桌面 Excel 查看器。它把程序分为四个模块：

```rust
mod excel;    // Excel 读写（umya-spreadsheet 封装）
mod gui;      // eframe/egui 界面，主结构 ExcelViewer
mod util;     // 通用工具（无 chrono 的日期换算）
mod license;  // 离线授权（机器指纹 + 多点加密存储 + ed25519 验签）
```

`main.rs` 自身**不实现业务逻辑**，只做三件事：

1. **CLI 分支**：`--uuid` / `--license` 两个诊断子命令，命中即处理后 `exit`，绝不进入 GUI。
2. **崩溃兜底**：注册 panic hook，把崩溃信息 + 调用栈落盘并弹窗提示。
3. **GUI 装配**：构建 `Viewport`（尺寸 + 图标），交给 `eframe::run_native` 启动 `ExcelViewer`。

这种“入口极薄、逻辑下沉到模块”的结构，让 GUI 与 CLI、授权诊断、崩溃处理彼此正交，互不耦合。

---

## 2. Windows GUI 子系统属性（第一行）

```rust
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
```

### 2.1 作用
告诉链接器把可执行文件标记为 **Windows GUI 子系统**（而非默认的控制台子系统）。效果：用户在资源管理器里**双击运行时不再弹出黑色控制台窗口**。

### 2.2 关键副作用 —— 这是全文最需要理解的设计权衡
GUI 子系统意味着程序**默认没有标准控制台**，因此：

- `println!` / `eprintln!` **没有任何输出**（写到一个不存在的 stdout/stderr）；
- 但程序仍可能被**从终端**调用（例如客服远程指导用户跑 `my-excel --uuid`）。

这就产生了一对矛盾：“双击要静默” vs “命令行要能看到输出”。`main.rs` 用 **`console_print`** 解决它（见 §5）。

### 2.3 平台条件
`cfg_attr(target_os = "windows", ...)` 表示仅在 Windows 上启用；其它平台保持默认（有控制台），保证跨平台可编译。

---

## 3. `load_window_icon()` —— 运行时窗口图标

```rust
fn load_window_icon() -> Option<egui::IconData>
```

### 3.1 它做什么
用 `image` crate（`png` feature）把**编译期内嵌**的 `assets/icon-v3-256.png`（`include_bytes!`）解码为 RGBA，封装成 `egui::IconData`，供 `ViewportBuilder::with_icon` 使用。eframe 在 Windows 上据此调用 `WM_SETICON`，使**标题栏图标与任务栏图标同源**。

### 3.2 为什么解码失败返回 `None` 而非 panic
图标是“锦上添花”，不是核心功能。解码失败时返回 `None`，调用方回退到默认图标，**绝不阻塞启动**。这与 §6 里 `if let Some(icon) = ...` 的可选装配呼应。

### 3.3 与 `build.rs` 的互补关系（重点）
项目里图标有**两套来源**，分工明确，不可互相替代：

| 来源 | 嵌入方式 | 覆盖场景 |
|---|---|---|
| `build.rs`（winresource 把 `icon-v3-128.ico` 嵌入 `.exe` 资源） | 编译期、Windows 资源段 | 资源管理器文件图标、跳转列表、**运行前**任务栏图标 |
| `load_window_icon()`（内嵌 PNG 解码） | 运行时 | **运行中**窗口的标题栏 + 任务栏图标 |

> ICO 必须是 `.ico` 格式（Windows 资源编译器不支持 SVG），由 `npm run gen-icon` 从 SVG 生成多分辨率版本；PNG 则供运行时解码。两者覆盖的“图标出现时机”不同，所以需要**并存**。

---

## 4. `handle_panic()` —— 崩溃捕获与日志

```rust
fn handle_panic(info: &std::panic::PanicHookInfo)
```

注册为全局 panic hook（`std::panic::set_hook`）。这是 GUI 程序的**必备兜底**：因为 §2 的副作用，GUI 程序崩溃时既没有控制台可看，普通用户也无法读 backtrace，必须把信息持久化并友好提示。

### 4.1 三步处理
1. **解析 panic 信息**：从 `payload` 尝试 `downcast_ref::<&str>` / `String` 得到消息；从 `info.location()` 得到 `文件:行:列`；`Backtrace::capture()` 取调用栈。
2. **写入日志文件**：路径为“exe 同目录 / `crash-YYYYMMDD.log`”，以 `append` 方式追加（同一天多次崩溃不会互相覆盖）。日期由 `days_to_ymd` 从 Unix 天数换算得到（见 §7）。
3. **弹窗提示**：用 `rfd::MessageDialog`（Error 级别）告诉用户日志路径与崩溃消息。

### 4.2 时间戳来自 `chrono_free_timestamp()`（见 §7），刻意不依赖 `chrono`。

### 4.3 路径定位策略
```rust
std::env::current_exe() → .parent() → 回退 "."
```
优先把日志写在 exe 同目录（用户最容易找到），取不到 exe 路径时回退当前目录。文件写入用 `OpenOptions::create+append`，忽略错误（崩溃处理本身不能再崩溃）。

### 4.4 为什么 hook 里能弹 `rfd` 对话框
panic 发生在主线程，此时事件循环虽已不可用，但 `rfd` 的同步 `MessageDialog` 走的是原生 Win32 对话框 API（不依赖 egui 事件循环），所以可以在 hook 末尾安全弹窗。

---

## 5. `console_print()` —— GUI 子系统下的控制台输出

```rust
#[cfg(windows)]
fn console_print(msg: &str) { /* AttachConsole + CONOUT$ */ }

#[cfg(not(windows))]
fn console_print(msg: &str) { /* 直接写 stdout */ }
```

### 5.1 解决的问题
§2 的副作用：GUI 子系统程序没有自己的控制台，`println!` 无效。但 `--uuid`/`--license` 这两个 CLI 子命令又**必须**把结果打印出来。

### 5.2 Windows 实现（三步）
1. FFI 声明并调用 `AttachConsole(ATTACH_PARENT_PROCESS = u32::MAX)`：**附加到父进程（即调用它的终端）的控制台**。
2. 附加成功后，用 `OpenOptions::write(true).open("CONOUT$")` 打开该控制台的输出设备，写入字节。
3. 附加失败（典型场景：**双击运行**，没有父终端）则**静默忽略**——正符合“双击要静默”的诉求。

### 5.3 为什么刻意只用 ASCII
注释明确：**输出均为 ASCII，无控制台编码问题**。中文在 Windows 控制台默认 GBK 代码页下极易乱码，故 CLI 输出（机器码、UUID、授权串）全部设计为 ASCII，规避编码坑。

### 5.4 非 Windows 分支
直接 `stdout().write_all`，因为非 Windows 下没有 §2 的子系统副作用。

---

## 6. `main()` —— 入口主流程

```rust
fn main() -> eframe::Result<()>
```

执行顺序严格分四段，**CLI 分支优先于一切**：

### 6.1 CLI 分支一：`--uuid`
```rust
if std::env::args().any(|a| a == "--uuid") {
    console_print(&format!("{}\n", license::fingerprint::registry_uuid()));
    std::process::exit(0);
}
```
打印本机的**注册表路径 UUID** 后 `exit(0)`。用途：技术支持让用户跑此命令拿到一个稳定标识，定位授权存储的注册表位置（`HKCU\Software\{uuid}`）。**不进入 GUI**。

> 该 UUID 由 `fingerprint::registry_uuid()` 生成：仅取**稳定硬件标识**（主板序列号/型号、CPU）经 SHA-256 → UUID v5 风格。重装系统不变，仅更换主板/CPU 才变（见 `license/fingerprint.rs`）。

### 6.2 CLI 分支二：`--license <encrypted>`
```rust
if let Some(pos) = args.iter().position(|a| a == "--license") {
    if pos + 1 < args.len() {
        let encrypted = &args[pos + 1];
        let machine_fp = license::fingerprint::fingerprint_bytes();
        match license::store::decrypt_for_display(encrypted, &machine_fp) {
            Some(text) => { console_print(...); exit(0); }
            None       => { console_print("Error: ..."); exit(1); }
        }
    } else { console_print("Error: --license requires an argument\n"); exit(1); }
}
```

- **用途**：解密一段授权密文并输出**统一导出格式**（`f=<首跑日>|l=<高水位日>|r=<剩余天数>|mac=<指纹哈希>`），供技术支持阅读当前授权/试用状态。
- **多存储位置兼容**（设计要点）：入参密文可能来自两种来源——
  - `LicenseBlob` 导出（`save()` 写注册表，**无 tag** 导出密钥）；
  - 任一存储点的内部密文（`home`/`config`/`local`/`regmain`/`regclsid`，**分位置密钥**，互不相同）。

  `decrypt_for_display` 依次尝试“无 tag 导出密钥 → 各存储点分位置密钥”，**任一成功即解密**，并经 `normalize_for_display` 规范化为导出格式，使**输出与来源位置无关**。详见 `license/store.rs` 文档。
- **绑机**：用本机 `fingerprint_bytes()` 当 AES-GCM 密钥派生输入，换机器或被篡改的串都解不开，返回 `None` → 打印错误并 `exit(1)`。

> 这两个 CLI 分支共享一个设计原则：**诊断能力必须脱离 GUI**。GUI 子系统下没有控制台，但客服远程排查时恰恰需要纯命令行的、确定性的输出，因此把这两条路径放在 `main()` 最前面、早于 panic hook 注册、早于 GUI 创建。

### 6.3 崩溃兜底装配
```rust
std::panic::set_hook(Box::new(handle_panic));
std::env::set_var("RUST_BACKTRACE", "1");
```
- 仅在确认**不是 CLI 调用**后才注册 hook（CLI 路径已 `exit`，不会走到这里）。
- `RUST_BACKTRACE=1`：release 模式下默认不捕获 backtrace，显式开启才能让 §4 的 `Backtrace::capture()` 拿到真实调用栈。

### 6.4 GUI 装配与启动
```rust
let mut viewport = egui::ViewportBuilder::default().with_inner_size([1200.0, 800.0]);
if let Some(icon) = load_window_icon() {
    viewport = viewport.with_icon(icon);
}
let options = NativeOptions { viewport, ..Default::default() };
eframe::run_native("My Excel", options, Box::new(|_cc| Ok(Box::new(ExcelViewer::new()))))
```
- **初始窗口 1200×800**。
- **图标可选装配**：`load_window_icon()` 返回 `None` 时跳过，窗口仍能正常启动（呼应 §3.2）。
- **`ExcelViewer::new()` 的启动开销**：内部会 `LicenseManager::load()`（读多点存储、验签、时钟回拨检测、必要时自愈补写），并据此决定激活弹窗是否初始可见（`LicensePopupState { visible: blocking, ... }`）。即**授权校验发生在 GUI 构造期**，而非每帧。

---

## 7. `chrono_free_timestamp()` 与 `util::date::days_to_ymd` —— 刻意不用 chrono

```rust
fn chrono_free_timestamp() -> String {
    // secs → 天 + 当天时分秒 → days_to_ymd 换算年月日
}
```

- 项目**刻意不引入 `chrono`**（见 `Cargo.toml` 依赖列表与 `util/date.rs` 文档说明），时间换算全部基于 `SystemTime` + 手写历法。
- `days_to_ymd(days)`：从 1970-01-01 起的天数，逐年减去当年天数（含闰年判断 `is_leap`）得到年份，再逐月减去当月天数得到月日。
- 该函数在 `main.rs`（崩溃日志文件名/时间戳）与 `license` 模块（试用天数、到期日显示）**复用同一份**，避免重复实现与 chrono 依赖。
- 注释自述“不处理闰秒等边缘情况”——对本场景（日志命名、试用天数）精度足够。

---

## 8. 设计逻辑总览（为什么是这样组织的）

把 `main.rs` 拆成这几块，背后是一组连贯的设计决策：

1. **“双击静默 + 命令行可用”的统一解法**
   `windows_subsystem = "windows"` 解决双击静默；`console_print` 解决命令行输出；二者通过“附加父进程控制台”桥接。CLI 子命令全部用 `console_print` 而非 `println!`，输出全部 ASCII 以规避 Windows 控制台编码问题。

2. **诊断与 GUI 彻底分离**
   `--uuid` / `--license` 在 `main()` 最前面短路 `exit`，**不创建任何 GUI 资源、不注册 panic hook**。这意味着即使 GUI/eframe 出问题，授权诊断命令依旧可用——这是离线授权场景下客服远程排障的底线。

3. **崩溃可追溯**
   GUI 程序的 panic 用户看不到，故 hook 落盘 + 弹窗；开启 `RUST_BACKTRACE=1` 让 release 也能拿到调用栈；日志按天 append、写 exe 同目录，用户最容易找到。

4. **图标双覆盖**
   运行时图标（PNG 解码）与编译期资源图标（ICO）覆盖不同时机，互为补充；解码失败优雅降级，不阻塞启动。

5. **依赖最小化**
   时间换算手写、不用 chrono；图标解码复用已依赖的 `image` crate；CLI 不引第三方 argparse。整个入口文件零业务依赖，只有 `eframe` / `rfd` / `image` / 标准库。

---

## 9. 与其他模块的关系（速查）

| `main.rs` 调用 | 作用 | 所在文件 |
|---|---|---|
| `gui::viewer::ExcelViewer` | GUI 主结构，构造期完成授权加载与 UI 状态初始化 | `src/gui/viewer.rs` |
| `util::date::days_to_ymd` | Unix 天数 → (年, 月, 日)，崩溃日志与时间戳复用 | `src/util/date.rs` |
| `license::fingerprint::registry_uuid` | `--uuid` 输出的稳定硬件派生 UUID（注册表路径） | `src/license/fingerprint.rs` |
| `license::fingerprint::fingerprint_bytes` | `--license` 解密用的机器指纹（HMAC/AES 密钥派生） | `src/license/fingerprint.rs` |
| `license::store::decrypt_for_display` | `--license` 多位置兼容解密 + 统一导出格式化 | `src/license/store.rs` |
| `build.rs`（间接） | 编译期把 ICO 嵌入 exe 资源，与运行时图标互补 | `build.rs` |

---

## 10. 执行流程图

```
            ┌───────────── 启动 my-excel ─────────────┐
            │                                          │
            ▼                                          │
   参数含 --uuid？  ── 是 ──► console_print(registry_uuid) ──► exit(0)
            │ 否
            ▼
   参数含 --license？ ─ 是 ──► 解密(指纹) ┬─ 成功 ► console_print(导出格式) ► exit(0)
            │ 否                          └─ 失败 ► console_print(Error)   ► exit(1)
            ▼
   注册 panic hook（handle_panic） + 设 RUST_BACKTRACE=1
            ▼
   构建 Viewport（1200×800 + 可选图标）
            ▼
   eframe::run_native("My Excel", ExcelViewer::new())
            │
            └─► 进入 GUI 事件循环
                  （ExcelViewer::new 内：LicenseManager::load → 授权状态 → 决定激活弹窗）

   —— 任意 panic ——► handle_panic：写 crash-YYYYMMDD.log + rfd 弹窗
```

---

*文档基于 `src/main.rs`（截至当前 master）及其直接依赖模块的实现整理。涉及授权存储/指纹的细节，请参阅 `docs/License.md` 与 `src/license/` 源码注释。*
