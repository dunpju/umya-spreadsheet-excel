# `src/main.rs` 实现文档

> 本文件是 `umya-spreadsheet-excel`（可执行名 `my-excel`）的程序入口。
> 它本身代码量很小（约 300 行），但承担了三件关键职责：**CLI 诊断子命令**、**崩溃捕获与日志**、**GUI 主窗口的装配与启动**。本文从设计意图出发，逐块解释其实现逻辑。

---

## 1. 文件定位与职责

`main.rs` 是 `[[bin]] name = "my-excel"` 的入口（见 `Cargo.toml`），一个基于 `eframe`/`egui` 的桌面 Excel 查看器。它把程序分为五个模块：

```rust
mod excel;    // Excel 读写（umya-spreadsheet 封装）
mod gui;      // eframe/egui 界面，主结构 ExcelViewer
mod util;     // 通用工具（无 chrono 的日期换算）
mod license;  // 离线授权（机器指纹 + 多点加密存储 + ed25519 验签）
mod shortcut; // 桌面快捷方式（Windows：启动时确保桌面存在 my-excel.lnk）
```

`main.rs` 自身**不实现业务逻辑**，只做四件事：

1. **CLI 分支**：`--uuid` / `--stores` / `--license` 三个诊断子命令（受 `diagnostic` feature 门控，**默认关闭**，见 §6），命中即处理后 `exit`，绝不进入 GUI。
2. **崩溃兜底**：注册 panic hook，把崩溃信息 + 调用栈落盘并弹窗提示。
3. **桌面快捷方式**：启动时确保 Windows 桌面存在指向本程序的 `my-excel.lnk`（缺失才创建）。
4. **GUI 装配**：构建 `Viewport`（尺寸 + 图标），交给 `eframe::run_native` 启动 `ExcelViewer`。

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
- 但程序仍可能被**从终端**调用（例如客服用内部诊断构建跑 `my-excel --uuid`，见 §6）。

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

> **受 `diagnostic` feature 门控**：本函数仅诊断构建（`--features diagnostic`）编译，与 §6 的三个 CLI 子命令同进退——公开发布版既无这些命令，也无 `console_print`。

```rust
#[cfg(all(windows, feature = "diagnostic"))]
fn console_print(msg: &str) { /* AttachConsole + CONOUT$ */ }

#[cfg(all(not(windows), feature = "diagnostic"))]
fn console_print(msg: &str) { /* 直接写 stdout */ }
```

### 5.1 解决的问题
§2 的副作用：GUI 子系统程序没有自己的控制台，`println!` 无效。但 `--uuid` / `--stores` / `--license` 这三个 CLI 子命令又**必须**把结果打印出来。

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

> **⚠️ 诊断 CLI 受 `diagnostic` feature 门控**：§6.1–6.3 的 `--uuid` / `--stores` / `--license`（连同 `console_print`、`format_store_paths`）由 `#[cfg(feature = "diagnostic")]` 包裹，**默认不编译**（`Cargo.toml` 的 `diagnostic` feature 默认关闭）。这些命令会暴露本机存储路径 / UUID / 授权状态，公开发布版（`build.bat` → `cargo build --release`）不含它们，逆向者 `strings` 也看不到 `--stores` 等字面量。仅开发者 / 技术支持的**内部诊断构建**启用：
>
> ```bash
> cargo build --release --features diagnostic      # 带诊断命令（内部 / 支持用）
> cargo test --features diagnostic                 # 跑 format_store_paths 等诊断单测
> ```
>
> 门控只去掉"诊断入口"，加密 / 签名 / 机器绑机等核心能力不受影响。

### 6.1 CLI 分支一：`--uuid`
```rust
if std::env::args().any(|a| a == "--uuid") {
    console_print(&format!("{}\n", license::fingerprint::registry_uuid()));
    std::process::exit(0);
}
```
打印本机的**注册表路径 UUID** 后 `exit(0)`。用途：技术支持让用户跑此命令拿到一个稳定标识，定位授权存储的注册表位置（`HKCU\Software\{uuid}`）。**不进入 GUI**。

> 该 UUID 由 `fingerprint::registry_uuid()` 生成：仅取**稳定硬件标识**（主板序列号/型号、CPU）经 SHA-256 → UUID v5 风格。重装系统不变，仅更换主板/CPU 才变（见 `license/fingerprint.rs`）。

### 6.2 CLI 分支二：`--stores`
```rust
if std::env::args().any(|a| a == "--stores") {
    console_print(&format_store_paths());
    std::process::exit(0);
}
```
打印本机**实际解析后**的全部存储点路径后 `exit(0)`，**不进入 GUI**。对应 `docs/gui/widgets/license.md` §6.1「存储位置」的全部 5 个存储点（home / config / local / regmain / regclsid），把文档占位符解析为当前系统的真实绝对路径：

| 行 | 占位符（doc 写法） | 实际解析 |
|---|---|---|
| `home:` | `~/.MyExcel/license.dat` | `dirs::home_dir()` → 真实主目录 |
| `config:` | `{config_dir}/{dir_uuid(config)}/state.dat` | `dirs::config_dir()` + `fingerprint::dir_uuid("config")` |
| `local:` | `{data_local_dir}/{dir_uuid(local)}/cache.bin` | `dirs::data_local_dir()` + `fingerprint::dir_uuid("local")` |
| `regmain:` | `HKCU\Software\{uuid}` | `fingerprint::registry_uuid()` → 真实 UUID（仅 Windows） |
| `regclsid:` | `HKCU\Software\Classes\CLSID\{大写UUID}` | `fingerprint::registry_uuid_clsid()` → 真实 CLSID UUID（仅 Windows） |

- **用途**：技术支持让用户跑此命令，一次性看到本机授权数据实际写到哪些文件 / 注册表键，便于排障时定位与清理。
- **与 `--uuid` 的区别**：`--uuid` 只给注册表路径 UUID（一个值）；`--stores` 给出全部 5 个存储点的**完整解析路径**（3 个文件 + 2 个注册表分支）。
- **路径派生原语与 `license::store::all_stores()` 完全一致**：`--stores` 输出的路径即程序实际读写的路径，不会漂移。
- **标签为 ASCII**（`home`/`config`/`local`/`regmain`/`regclsid`，沿用 doc 的 tag 名），呼应 §5.3 的控制台编码规避；`dirs` 返回 `None` 的极端环境打印兜底文案，非 Windows 的 `regmain` / `regclsid` 标注 `(N/A on non-Windows)`。

> 解析逻辑封装在 `format_store_paths()`（main.rs 内的纯函数，返回 `String`），便于单测验证占位符确已解析为真实路径。

### 6.3 CLI 分支三：`--license <encrypted>`
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

> 这三个 CLI 分支共享一个设计原则：**诊断能力必须脱离 GUI**。GUI 子系统下没有控制台，但客服远程排查时恰恰需要纯命令行的、确定性的输出，因此把这三条路径放在 `main()` 最前面、早于 panic hook 注册、早于 GUI 创建。

### 6.4 崩溃兜底装配
```rust
std::panic::set_hook(Box::new(handle_panic));
std::env::set_var("RUST_BACKTRACE", "1");
```
- 仅在确认**不是 CLI 调用**后才注册 hook（CLI 路径已 `exit`，不会走到这里）。
- `RUST_BACKTRACE=1`：release 模式下默认不捕获 backtrace，显式开启才能让 §4 的 `Backtrace::capture()` 拿到真实调用栈。

### 6.5 桌面快捷方式
```rust
shortcut::ensure_desktop_shortcut();
```
- **目的**：启动时确保 Windows 桌面存在指向当前 exe 的 `my-excel.lnk`，缺失才创建。
- **幂等且廉价**：先做一次 `lnk.exists()` 检测；已存在直接返回，**正常启动几乎零开销（不初始化 COM）**。只有真正缺失时才走 COM 生成。
- **桌面路径**：优先 `dirs::desktop_dir()`（走 Windows Known Folder API，正确处理 OneDrive 桌面重定向），回退 `%USERPROFILE%\Desktop`。
- **best-effort**：取不到 exe 路径 / 桌面不可写 / COM 失败一律静默忽略，绝不阻塞启动。
- **实现**：原始 COM FFI（`IShellLinkW` + `IPersistFile`），零新增依赖，与 `console_print` 的 FFI 风格一致；非 Windows 平台 `#[cfg]` 门控为空操作。详见 `src/shortcut.rs`。
- **图标**：不显式 `SetIconLocation`——`build.rs` 嵌入的 ICO 资源自动成为 `.lnk` 默认图标。

### 6.6 GUI 装配与启动
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
   `--uuid` / `--stores` / `--license` 在 `main()` 最前面短路 `exit`，**不创建任何 GUI 资源、不注册 panic hook**。这意味着即使 GUI/eframe 出问题，授权诊断命令依旧可用——这是离线授权场景下客服远程排障的底线。

3. **崩溃可追溯**
   GUI 程序的 panic 用户看不到，故 hook 落盘 + 弹窗；开启 `RUST_BACKTRACE=1` 让 release 也能拿到调用栈；日志按天 append、写 exe 同目录，用户最容易找到。

4. **图标双覆盖**
   运行时图标（PNG 解码）与编译期资源图标（ICO）覆盖不同时机，互为补充；解码失败优雅降级，不阻塞启动。

5. **依赖最小化**
   时间换算手写、不用 chrono；图标解码复用已依赖的 `image` crate；CLI 不引第三方 argparse。整个入口文件零业务依赖，只有 `eframe` / `rfd` / `image` / 标准库。

6. **桌面集成幂等且无侵入**
   `shortcut::ensure_desktop_shortcut()` 先检测后创建：已存在即零成本跳过，缺失才初始化 COM 生成 `.lnk`；全链路 best-effort，失败静默，绝不阻塞启动或弹错。原始 COM FFI 实现零新增依赖，与 `console_print` 风格一致，非 Windows 编译为空操作。

---

## 9. 与其他模块的关系（速查）

| `main.rs` 调用 | 作用 | 所在文件 |
|---|---|---|
| `gui::viewer::ExcelViewer` | GUI 主结构，构造期完成授权加载与 UI 状态初始化 | `src/gui/viewer.rs` |
| `shortcut::ensure_desktop_shortcut` | 启动时确保桌面存在指向本程序的 `my-excel.lnk`（缺失才创建） | `src/shortcut.rs` |
| `util::date::days_to_ymd` | Unix 天数 → (年, 月, 日)，崩溃日志与时间戳复用 | `src/util/date.rs` |
| `license::fingerprint::registry_uuid` | `--uuid` 输出、`--stores` 的 regmain 行所用稳定硬件派生 UUID（注册表路径） | `src/license/fingerprint.rs` |
| `license::fingerprint::registry_uuid_clsid` | `--stores` 的 regclsid 行所用 CLSID 风格 UUID（大写 + 花括号） | `src/license/fingerprint.rs` |
| `license::fingerprint::dir_uuid` | `--stores` 的 config / local 目录名派生（`dir_uuid("config")` / `dir_uuid("local")`，与 `store::all_stores` 一致） | `src/license/fingerprint.rs` |
| `license::fingerprint::fingerprint_bytes` | `--license` 解密用的机器指纹（HMAC/AES 密钥派生） | `src/license/fingerprint.rs` |
| `license::store::decrypt_for_display` | `--license` 多位置兼容解密 + 统一导出格式化 | `src/license/store.rs` |
| `build.rs`（间接） | 编译期把 ICO 嵌入 exe 资源，与运行时图标互补 | `build.rs` |

---

## 10. 执行流程图

> 注：`--uuid` / `--stores` / `--license` 三分支**仅在 `diagnostic` 构建存在**（见 §6）；公开发布版无此三分支，直接进入「注册 panic hook」。

```
            ┌───────────── 启动 my-excel ─────────────┐
            │                                          │
            ▼                                          │
   参数含 --uuid？  ── 是 ──► console_print(registry_uuid) ──► exit(0)
            │ 否
            ▼
   参数含 --stores？ ─ 是 ──► console_print(全部 5 存储点实际路径) ──► exit(0)
            │ 否
            ▼
   参数含 --license？ ─ 是 ──► 解密(指纹) ┬─ 成功 ► console_print(导出格式) ► exit(0)
            │ 否                          └─ 失败 ► console_print(Error)   ► exit(1)
            ▼
   注册 panic hook（handle_panic） + 设 RUST_BACKTRACE=1
            ▼
   shortcut::ensure_desktop_shortcut()（桌面无 my-excel.lnk 才创建）
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

*文档基于 `src/main.rs`（截至当前 master）及其直接依赖模块的实现整理。涉及授权存储/指纹的细节，请参阅 `docs/gui/widgets/license.md` 与 `src/license/` 源码注释。*
