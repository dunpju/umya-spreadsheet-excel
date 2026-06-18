# 离线 License 授权机制设计方案

> 为 `umya-spreadsheet-excel` 设计的完全离线授权方案：30 天免费试用 → 到期弹付款二维码 → 离线激活码解锁，全程不联网。

---

## 〇、关键设计判断（基于现有代码）

阅读 `src/main.rs`、`src/gui/viewer.rs`、`Cargo.toml` 后，有三处既有约定决定了本方案的形态：

1. **刻意零外部日期依赖**：`main.rs` 自写了 `days_to_ymd` / `is_leap`，`viewer.rs::generate_save_path` 又用 Howard Hinnant 算法算日期 —— 说明作者不愿引入 `chrono`。故本方案**全程复用 `SystemTime` + epoch 天数**，不引入 `chrono`。
2. **存储走 `serde_yaml` + `dirs::home_dir()`**：现有配置存在 `~/.MyExcel/my-excel.yaml`，且全程**手工拼 `serde_yaml::Value`**（未用 `#[derive(Serialize)]`）。授权负载也采用**手工文本编码 + Base64**，不引入 `serde` derive，保持风格一致。
3. **GUI 是 egui**：弹窗用 `egui::Window`（参见 `help_popup.rs`），付款/激活弹窗照此模式。

> ⚠️ 实话：**纯客户端离线授权无法做到绝对防破解**。任何足够有耐心的逆向都能 patch 掉 `verify()`。本方案的目标是把门槛抬到“需要专业逆向、且无法靠改系统时间 / 删配置文件绕过”，并提供**授权码不可伪造**的强保证（这是非对称签名真正能兜底的部分）。

---

## 一、整体架构与模块划分

新增顶层模块 `src/license/`，并把现有日期工具抽到 `src/util/` 复用：

```
src/
├── util/
│   └── date.rs        ← 把 main.rs 的 days_to_ymd / is_leap 移到这里（复用点）
├── license/
│   ├── mod.rs         ← 对外门面：LicenseManager + LicenseStatus
│   ├── crypto.rs      ← Ed25519 验签 / HMAC-SHA256 / SHA-256 / Base64 / hex
│   ├── fingerprint.rs ← 机器指纹（绑定机器，防一份授权多机用）
│   ├── payload.rs     ← LicensePayload：手工编码/解析 + 签名输入
│   ├── store.rs       ← 试用状态 + 授权码持久化（文件 + 注册表冗余 + HMAC）
│   └── time.rs        ← today_epoch_day()（复用 epoch 天数）
├── gui/widgets/
│   └── license_popup.rs  ← 付款二维码 + 激活码输入弹窗
└── main.rs            ← 增加 `mod util; mod license;`，启动时 gate；支持 `--uuid` 查看本机注册表路径
```

依赖方向（单向、无环）：

```
license_popup ──► viewer ──► LicenseManager(mod.rs)
                                  ├──► store ──► crypto
                                  ├──► payload ──► crypto
                                  ├──► fingerprint ──► crypto
                                  └──► time ──► util::date
```

---

## 二、密码学方案选型

| 用途 | 算法 | 为什么 |
|---|---|---|
| **授权码防伪造**（核心强保证） | **Ed25519** 非对称签名 | 私钥只在开发者手里，公钥编译进 exe；程序能验真伪但**无法伪造**。签名 64B、公钥 32B、验证极快。 |
| **试用状态防篡改** | **HMAC-SHA256** | 对称、快；密钥由“机器指纹 + 内置胡椒”派生，换机器 / 改文件都校验失败。 |
| **机器指纹** | **SHA-256** 聚合多个稳定标识 | 绑定到具体机器。 |

**密钥拓扑**：

- 开发者离线生成一对 Ed25519 密钥，**私钥永不分发**（最好放离线机 / 加密 U 盘）。
- **32 字节公钥**作为常量编进 `crypto.rs`。
- 胡椒（`HMAC_PEPPER`）混淆编进二进制（非真正机密，仅抬高门槛）。

**授权码格式**（JWT 风格，可读、可调试）：

```
<base64(负载明文)>.<base64(64字节Ed25519签名)>
```

---

## 三、数据结构定义

### 3.1 `license/payload.rs` —— 授权负载

```rust
//! 授权负载：被 Ed25519 签名的内容。
//! 采用手工文本编码（固定字段顺序 + 分隔符），保证签名/验签字节完全一致，
//! 不依赖 serde-derive，与项目现有风格一致。

pub const PRODUCT_ID: &str = "umya-excel";
pub const TRIAL_DAYS: u64 = 30;
pub const EXPIRES_NEVER: u64 = 0; // 0 = 永久授权

#[derive(Clone, Debug)]
pub struct LicensePayload {
    pub version: u32,        // 负载版本，便于将来升级
    pub product: String,     // 产品标识，防止别的产品授权码串用
    pub machine: String,     // 绑定的机器码（与本地指纹比对）
    pub issued_day: u64,     // 签发日（epoch 天）
    pub expires_day: u64,    // 到期日（epoch 天），0=永久
    pub edition: String,     // 版本/功能位，如 "pro"
    pub customer: String,    // 客户名（可选，便于核对）
}

impl LicensePayload {
    /// 规范化明文：签名与编码都基于这串字节。
    /// 关键：行序、分隔符固定，绝不能在生成后改变格式（否则旧授权码全部失效）。
    pub fn to_text(&self) -> String {
        format!(
            "v={}\np={}\nm={}\ni={}\ne={}\ned={}\nc={}\n",
            self.version, self.product, self.machine,
            self.issued_day, self.expires_day, self.edition, self.customer,
        )
    }

    /// 解析明文（验签通过后调用）
    pub fn parse(text: &str) -> Option<Self> {
        let mut p = LicensePayload {
            version: 0, product: String::new(), machine: String::new(),
            issued_day: 0, expires_day: 0, edition: String::new(), customer: String::new(),
        };
        for line in text.lines() {
            let (k, v) = line.split_once('=')?;
            match k {
                "v"  => p.version = v.parse().ok()?,
                "p"  => p.product = v.to_string(),
                "m"  => p.machine = v.to_string(),
                "i"  => p.issued_day = v.parse().ok()?,
                "e"  => p.expires_day = v.parse().ok()?,
                "ed" => p.edition = v.to_string(),
                "c"  => p.customer = v.to_string(),
                _ => {}
            }
        }
        if p.product.is_empty() || p.machine.is_empty() { return None; }
        Some(p)
    }

    /// 是否已到期（基于当前 epoch 天）
    pub fn is_expired(&self, today: u64) -> bool {
        self.expires_day != EXPIRES_NEVER && today >= self.expires_day
    }
}
```

### 3.2 `license/store.rs` —— 试用状态

```rust
//! 本地试用状态：带 HMAC 校验，多位置冗余存储。
#[derive(Clone, Debug)]
pub struct TrialState {
    pub first_run_day: u64,  // 首次启动日（试用起点）
    pub last_run_day: u64,   // 高水位：已观测到的最大 day（防回拨核心）
    pub rollback_count: u32, // 累计检测到的时钟回拨次数
    pub mac: String,         // HMAC(机器指纹, body)
}

impl TrialState {
    /// 参与 HMAC 的明文（不含 mac 自身）
    fn body(&self) -> String {
        format!("f={}|l={}|r={}", self.first_run_day, self.last_run_day, self.rollback_count)
    }

    pub fn sign(&mut self, machine_fp: &[u8]) {
        self.mac = crate::license::crypto::hmac_hex(machine_fp, self.body().as_bytes());
    }

    pub fn verify(&self, machine_fp: &[u8]) -> bool {
        crate::license::crypto::hmac_verify(machine_fp, self.body().as_bytes(), &self.mac)
    }
}
```

### 3.3 `license/mod.rs` —— 授权状态机

```rust
/// 授权状态：启动时由 LicenseManager 计算，UI 据此决定是否放行
#[derive(Debug, Clone, PartialEq)]
pub enum LicenseStatus {
    /// 试用中，剩余天数
    Trial { days_left: i64 },
    /// 试用期已结束，必须激活
    TrialExpired,
    /// 已激活；None=永久，Some=剩余天数
    Licensed { days_left: Option<i64> },
    /// 授权已到期（限时授权），需续期
    LicensedExpired,
    /// 检测到篡改（签名失败 / 时钟回拨 / 存储冲突），锁定
    Tampered,
}

impl LicenseStatus {
    pub fn is_blocking(&self) -> bool {
        matches!(self, Self::TrialExpired | Self::LicensedExpired | Self::Tampered)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ActivateError {
    Format,          // 授权码格式不对
    InvalidSig,      // 签名校验失败（伪造/损坏）
    MachineMismatch, // 机器码不匹配（授权码不是给本机的）
    ProductMismatch,
    AlreadyExpired,  // 签发即过期
}
```

---

## 四、关键函数签名（`LicenseManager`）

```rust
pub struct LicenseManager {
    machine_code: String,       // 用户可见机器码（发给开发者）
    machine_fp: Vec<u8>,        // 机器指纹（HMAC 密钥派生用）
    license: Option<LicensePayload>,
    trial: TrialState,
}

impl LicenseManager {
    /// 启动时调用：读取本地状态、校验签名、检测时钟回拨、得到状态
    pub fn load() -> Self;

    /// 计算当前授权状态（传入 today_epoch_day，便于测试）
    pub fn status(&self, today: u64) -> LicenseStatus;

    /// 用户输入授权码 → 验签 → 比对机器码 → 落盘激活
    pub fn activate(&mut self, license_str: &str, today: u64)
        -> Result<LicensePayload, ActivateError>;

    /// 每次成功运行后调用：推进高水位并持久化（文件 + 注册表）
    pub fn checkpoint(&mut self, today: u64) -> bool;

    pub fn machine_code(&self) -> &str;
    pub fn license(&self) -> Option<&LicensePayload>;
}
```

---

## 五、授权流程

**首次启动 → 试用 → 到期 → 付款 → 激活 → 永久 / 限期使用**

```
[首次运行]
  load() 发现无 license.dat
  ├─ first_run_day = today, last_run_day = today
  ├─ sign + 写盘(文件 + 注册表)
  └─ status = Trial { days_left = 30 }

[每次运行]
  checkpoint(today):
  ├─ 校验 HMAC（失败 → Tampered）
  ├─ if today < last_run_day - 容差  → 时钟被回拨 → rollback_count++, 视为到期/锁定
  ├─ last_run_day = max(last_run_day, today)   ← 高水位只增不减
  └─ 持久化
  status(): 已激活且未到期 → 放行；否则按 Trial/Expired 决定是否拦截

[试用到期]
  UI 渲染遮罩 → 付款二维码弹窗
  ├─ 展示机器码（用户复制）
  ├─ 提示：扫码付款 → 把机器码发给开发者 → 获取授权码
  └─ 授权码输入框 + "激活"

[激活]
  activate(code):
  ├─ split('.') → (b64负载, b64签名)
  ├─ ed25519_verify(负载明文, 签名)  ← 失败 = InvalidSig / 伪造
  ├─ parse → 比对 product、machine == 本地机器码
  ├─ 检查 issued_day、expires_day
  └─ 通过 → 写 license.dat → status = Licensed
```

---

## 六、数据存储方案

### 6.1 存储位置（冗余 + 交叉校验，防删文件绕过）

| 位置 | 内容 | 作用 |
|---|---|---|
| `~/.MyExcel/license.dat`（主） | 试用状态 + 授权码 | 主存储 |
| 注册表 `HKCU\Software\{uuid}`（Win 备份） | 同上的副本 | 路径由硬件派生 UUID 混淆，破解者无法直接搜索到；与文件交叉比对 |

**加载时的冲突仲裁（取“更严格”的那份）**：

```rust
// 对试用状态：取所有有效副本中 last_run_day 的最大值（高水位）
// → 即使用户删了 license.dat，注册表里仍记得“已经用了 35 天”
// 对授权码：两份都验签；任一有效且机器码匹配即放行
```

> 这是抗“删配置文件重置试用”的关键：**高水位只增不减，且写两处**。删一处，另一处仍记得试用已过期。

### 6.2 防时钟回拨

- `last_run_day` 只增不减（高水位）。
- 若 `today < last_run_day - 容差(1~2天)` → 判定回拨，`rollback_count++`，试用期**不再延长**，按到期处理。
- 容差用于容忍跨时区 / 夏令时的微小抖动。

---

## 七、防破解 / 防篡改措施

| 威胁 | 对策 |
|---|---|
| 伪造授权码 | Ed25519 非对称签名，无私钥无法伪造 ✅（强保证） |
| 改系统时间延长试用 | 高水位 `last_run_day` + 回拨检测 + 多位置冗余 |
| 删除 `license.dat` 重置 | 注册表冗余副本，取 `max(last_run_day)` |
| 手改试用状态文件 | HMAC(机器指纹 + 胡椒) 校验，改任一字节都失败 |
| 一份授权码多机用 | 机器码绑定，`activate` 校验 `machine == 本地` |
| patch 二进制跳过 verify | 校验点**分散到核心功能**（见下），非仅启动时一处 |
| 提取公钥 / 胡椒伪造 | 公钥本就公开无妨；胡椒被提取只能伪造试用状态、不能伪造授权码 |

**校验点分散（提高 patch 成本）**：不要只在 `main` 启动时检查一次。把 `LicenseManager::status()` 调用分散到关键路径，例如**保存 / 导出**前再验一次：

```rust
// viewer.rs 保存前
if self.license.status(time::today_epoch_day()).is_blocking() {
    self.license_popup.visible = true; // 拦截
    return;
}
self.start_async_save(ctx);
```

**编译期建议**：`cargo build --release` + `strip = true` + LTO；公钥 / 胡椒用 `lazy once` 取地址而非明文常量（轻微混淆）；预算充足时上商业壳（VMProtect / Themida）。

---

## 八、依赖库推荐

```toml
[dependencies]
# —— 现有 ——
umya-spreadsheet = "3.0"
eframe = "0.34.3"
egui = "0.34.3"
rfd = "0.17.2"
serde_yaml = "0.9"
dirs = "6"

# —— 新增：授权 ——
ed25519-dalek = { version = "2", default-features = false }   # 授权码验签
sha2 = "0.10"                                                  # 指纹/摘要
hmac = "0.12"                                                  # 试用状态校验
base64 = "0.22"                                               # 授权码/指纹编码
image = { version = "0.25", default-features = false, features = ["png"] }  # 解码二维码 PNG
winreg = "0.52"                                               # Win 注册表（机器指纹 + 冗余存储）

[profile.release]
strip = true

# 单独降低 `windows` crate（eframe/wgpu 间接依赖）的优化级别，
# 规避 opt-level=3 下 rustc/LLVM 触发的 STATUS_STACK_BUFFER_OVERRUN 编译崩溃。
# 注：lto=true + codegen-units=1 在本依赖栈（wgpu/naga）上会触发同样的崩溃，故不启用。
[profile.release.package.windows]
opt-level = 1
```

> `ed25519-dalek` 2.x 验签路径不需要 `rand`；只有 keygen 工具需要 `rand = "0.8"`。

---

## 九、核心代码实现示例

### 9.1 复用日期工具（`src/util/date.rs`）

把 `main.rs` 里的 `days_to_ymd` / `is_leap` 搬过来并 `pub`：

```rust
//! 日期工具（复用，原 main.rs 中实现）。不依赖 chrono。
pub fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // 原 main.rs 实现
}

pub fn is_leap(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}
```

### 9.2 `license/time.rs`

```rust
//! 当前 epoch 天数（复用 SystemTime，无 chrono）
pub fn today_epoch_day() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() / 86400
}

/// 天数差 → 友好日期串（如 2026-06-18），用于 UI 显示到期日
pub fn day_to_ymd_string(day: u64) -> String {
    let (y, m, d) = crate::util::date::days_to_ymd(day);
    format!("{:04}-{:02}-{:02}", y, m, d)
}
```

### 9.3 `license/crypto.rs`

```rust
//! 离线授权的密码学原语：Ed25519 验签 / HMAC-SHA256 / SHA-256 / Base64 / hex
//!
//! 设计要点：
//! - 授权码使用非对称签名（Ed25519）。私钥仅开发者持有，公钥编译进程序，
//!   程序可验证授权码真伪但无法伪造。
//! - 试用状态使用 HMAC-SHA256 做完整性校验，密钥由“机器指纹 + 内置胡椒”派生，
//!   换机器或改文件均会校验失败。

use ed25519_dalek::{Signature, Verifier, VerifyingKey, SIGNATURE_LENGTH};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// ⚠️ 替换为你的真实公钥（keygen 生成的 verifying_key().to_bytes()）
pub const DEVELOPER_PUBLIC_KEY: [u8; 32] = [0u8; 32];

/// 内置胡椒（混淆），用于派生 HMAC 密钥，抬高本地篡改门槛。
const HMAC_PEPPER: &[u8] = b"umya-excel-v1-s3cr3t-pepper-CHANGE-ME";

/// SHA-256 摘要（十六进制）
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex(&hasher.finalize())
}

/// 用机器指纹派生 HMAC 密钥
fn derive_hmac_key(machine_fingerprint: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(HMAC_PEPPER);
    hasher.update(machine_fingerprint);
    let out = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out);
    key
}

/// 计算消息的 HMAC（返回十六进制）
pub fn hmac_hex(machine_fingerprint: &[u8], msg: &[u8]) -> String {
    let key = derive_hmac_key(machine_fingerprint);
    let mut mac = HmacSha256::new_from_slice(&key).expect("hmac key");
    mac.update(msg);
    hex(&mac.finalize().into_bytes())
}

/// 校验 HMAC（常量时间比较）
pub fn hmac_verify(machine_fingerprint: &[u8], msg: &[u8], expected_hex: &str) -> bool {
    let actual = hmac_hex(machine_fingerprint, msg);
    if actual.len() != expected_hex.len() {
        return false;
    }
    // 常量时间比较，防时序攻击
    actual.as_bytes()
        .iter()
        .zip(expected_hex.as_bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// 用内嵌公钥验证 Ed25519 签名
pub fn ed25519_verify(msg: &[u8], sig_bytes: &[u8]) -> bool {
    if sig_bytes.len() != SIGNATURE_LENGTH {
        return false;
    }
    let Ok(vk) = VerifyingKey::from_bytes(&DEVELOPER_PUBLIC_KEY) else { return false };
    let Ok(sig) = Signature::from_slice(sig_bytes) else { return false };
    vk.verify(msg, &sig).is_ok()
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}
```

### 9.4 `license/fingerprint.rs`

```rust
//! 机器指纹：绑定授权到具体机器，防止一份授权码多机共用。
//! 组合多个标识（按稳定性排序：硬件级 -> OS 级 -> 用户可改），
//! SHA-256 后取前若干字节做 hex 分组，得到用户可见的”机器码”（无额外依赖）。
//!
//! 硬件级标识（主板序列号/型号、CPU 型号）在重装系统、改计算机名后不变，
//! 确保已激活授权不会被误锁。仅更换主板/CPU 才会导致指纹变化，需重新激活。

#[cfg(windows)]
fn windows_machine_guid() -> Option<String> {
    use winreg::enums::*;
    use winreg::RegKey;
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(“SOFTWARE\\Microsoft\\Cryptography”).ok()
        .and_then(|k| k.get_value::<String, _>(“MachineGuid”).ok())
}

/// 主板序列号（最稳定的硬件标识，重装系统不变）
#[cfg(windows)]
fn windows_system_serial() -> Option<String> { /* HKLM\...\BIOS\SystemSerialNumber */ }

/// 主板/系统产品名（重装系统不变）
#[cfg(windows)]
fn windows_system_product() -> Option<String> { /* HKLM\...\BIOS\SystemProductName */ }

/// CPU 标识（更换 CPU 才会变）
#[cfg(windows)]
fn windows_cpu_id() -> Option<String> { /* HKLM\...\CentralProcessor\0\ProcessorNameString */ }

/// 采集机器标识原始串（按稳定性排序：硬件 -> OS 级 -> 用户可改）
fn collect_raw_identifiers() -> Vec<String> {
    let mut ids = Vec::new();
    #[cfg(windows)]
    {
        // 稳定：主板序列号（硬件唯一，重装系统不变）
        if let Some(s) = windows_system_serial() { ids.push(format!(“serial={s}”)); }
        // 稳定：主板型号
        if let Some(p) = windows_system_product() { ids.push(format!(“product={p}”)); }
        // 较稳定：CPU 型号（更换 CPU 才会变）
        if let Some(c) = windows_cpu_id() { ids.push(format!(“cpu={c}”)); }
        // 半稳定：MachineGuid（重装系统会变）
        if let Some(g) = windows_machine_guid() { ids.push(format!(“guid={g}”)); }
        // 不稳定：计算机名（用户可改）
        if let Ok(n) = std::env::var(“COMPUTERNAME”) { ids.push(format!(“host={n}”)); }
    }
    ids
}

/// 机器指纹（原始字节）—— 供 HMAC 与授权绑定使用
pub fn fingerprint_bytes() -> Vec<u8> {
    let raw = collect_raw_identifiers().join(“|”);
    crate::license::crypto::sha256_hex(raw.as_bytes()).into_bytes()
}

/// 用户可见的机器码：取指纹再哈希后 hex，按 XXXX-XXXX-XXXX-XXXX 分组
pub fn machine_code() -> String {
    let fp = fingerprint_bytes();
    let hex = crate::license::crypto::sha256_hex(&fp);
    let g = |i: usize| &hex[i..i + 4];
    format!(“{}-{}-{}-{}”, g(0), g(4), g(8), g(12))
}

/// 基于稳定硬件标识（主板/CPU）派生的 UUID，用于注册表路径混淆。
/// 格式如 71445fac-d6ef-5436-9da7-5a323762d7f5（UUID v5 风格）。
/// 确定性：同一台机器始终得到相同 UUID；更换主板/CPU 后改变。
pub fn registry_uuid() -> String {
    // 仅用稳定硬件标识（serial + product + cpu），不含 COMPUTERNAME/MachineGuid
    // SHA-256 取前 16 字节，设置 UUID v5 版本/变体位
}
```

> 💡 **指纹稳定性分级**：主板序列号/型号/CPU 三项硬件标识为**最稳定层**（重装系统、改计算机名均不变）；MachineGuid 为**半稳定层**（重装系统会变）；COMPUTERNAME 为**不稳定层**（用户可改）。硬件标识确保授权不会因 OS 级变更而误锁。

### 9.5 `license/store.rs` —— 持久化（文件 + 注册表冗余）

```rust
//! 本地存储：试用状态 + 授权码，带 HMAC 校验与多位置冗余（文件 + 注册表）。

use std::path::PathBuf;

const TRIAL_FILENAME: &str = "license.dat";
#[cfg(windows)]
fn reg_path() -> String {
    format!("Software\\{}", crate::license::fingerprint::registry_uuid())
}

fn primary_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".MyExcel").join(TRIAL_FILENAME)
}

/// 文件内容布局：
/// [trial]
/// f=...|l=...|r=...
/// mac=<hex>
/// [license]
/// <授权码或空>
pub fn save(trial: &TrialState, license_str: &Option<String>, machine_fp: &[u8]) -> bool {
    // 1) 写主文件 ~/.MyExcel/license.dat
    // 2) [cfg(windows)] 同步写注册表 HKCU\Software\{uuid}（硬件派生，防直接搜索）
    true
}

pub fn load(machine_fp: &[u8]) -> (Option<TrialState>, Option<String>) {
    // 读文件 + 注册表，各自验签；
    // trial 取所有有效副本中 max(last_run_day)；
    // license 任一有效（验签 + 机器码匹配）即可。
    (None, None)
}
```

### 9.6 `license/mod.rs` —— `activate` 与 `status` 核心逻辑

```rust
pub fn activate(&mut self, license_str: &str, today: u64)
    -> Result<LicensePayload, ActivateError>
{
    // 1) 拆分 base64
    let (payload_b64, sig_b64) = license_str.split_once('.')
        .ok_or(ActivateError::Format)?;
    let payload_text = base64::engine::general_purpose::STANDARD
        .decode(payload_b64).map_err(|_| ActivateError::Format)?;
    let sig = base64::engine::general_purpose::STANDARD
        .decode(sig_b64).map_err(|_| ActivateError::Format)?;

    // 2) 验签（防伪造的核心）
    if !crypto::ed25519_verify(&payload_text, &sig) {
        return Err(ActivateError::InvalidSig);
    }

    // 3) 解析 + 语义校验
    let p = LicensePayload::parse(std::str::from_utf8(&payload_text).ok()?)
        .ok_or(ActivateError::Format)?;
    if p.product != payload::PRODUCT_ID    { return Err(ActivateError::ProductMismatch); }
    if p.machine != self.machine_code      { return Err(ActivateError::MachineMismatch); }
    if p.is_expired(today)                 { return Err(ActivateError::AlreadyExpired); }

    // 4) 落盘
    self.license = Some(p.clone());
    store::save(&self.trial, &Some(license_str.to_string()), &self.machine_fp);
    Ok(p)
}

pub fn status(&self, today: u64) -> LicenseStatus {
    if let Some(lic) = &self.license {
        if lic.is_expired(today) { return LicenseStatus::LicensedExpired; }
        let left = if lic.expires_day == payload::EXPIRES_NEVER { None }
                   else { Some(lic.expires_day as i64 - today as i64) };
        return LicenseStatus::Licensed { days_left: left };
    }
    // 试用
    let expire_day = self.trial.first_run_day + payload::TRIAL_DAYS;
    if today >= expire_day { LicenseStatus::TrialExpired }
    else { LicenseStatus::Trial { days_left: expire_day as i64 - today as i64 } }
}
```

### 9.7 付款二维码弹窗（`gui/widgets/license_popup.rs`）

二维码 PNG 通过 `include_bytes!` 嵌入，首次显示时解码为纹理：

```rust
use eframe::egui;

const QR_PNG: &[u8] = include_bytes!("../../assets/pay_qr.png");

pub struct LicensePopupState {
    pub visible: bool,
    pub license_input: String,
    pub error: Option<&'static str>,
    pub qr_texture: Option<egui::TextureHandle>,
    pub machine_code: String,        // 由 viewer 注入
    pub show_activated: bool,
}

impl LicensePopupState {
    fn ensure_qr(&mut self, ctx: &egui::Context) {
        if self.qr_texture.is_some() { return; }
        if let Ok(img) = image::load_from_memory(QR_PNG) {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [w as usize, h as usize], rgba.as_raw());
            self.qr_texture = Some(ctx.load_texture("pay_qr", image, Default::default()));
        }
    }
}

pub fn draw_license_popup(
    ctx: &egui::Context,
    st: &mut LicensePopupState,
    status: &crate::license::LicenseStatus,
    on_activate: &mut dyn FnMut(&str),
) {
    if !st.visible { return; }
    st.ensure_qr(ctx);

    // 模态遮罩 + 居中窗口，拦截主界面
    egui::Window::new("license_gate")
        .title_bar(false).resizable(false).collapsible(false)
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_min_width(380.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("试用期已结束").size(16.0).strong());
                ui.add_space(8.0);

                if let Some(t) = &st.qr_texture {
                    ui.image(egui::load::SizedTexture::new(t.id(), [200.0, 200.0]));
                }
                ui.label("扫码付款后，联系开发者获取授权码");
                ui.add_space(8.0);

                ui.label("本机机器码（请发送给开发者）：");
                ui.horizontal(|ui| {
                    let code = st.machine_code.clone();
                    ui.monospace(&code);
                    if ui.button("复制").clicked() { ui.output_mut(|o| o.copied_text = code); }
                });
                ui.add_space(12.0);

                ui.label("输入授权码：");
                ui.add(egui::TextEdit::multiline(&mut st.license_input)
                    .desired_width(360.0).desired_rows(3));
                if let Some(e) = st.error {
                    ui.colored_label(egui::Color32::RED, e);
                }
                ui.add_space(8.0);
                if ui.button(egui::RichText::new("激  活").size(14.0)).clicked() {
                    on_activate(&st.license_input.clone());
                }
            });
        });
}
```

### 9.8 集成进 `viewer.rs`

```rust
pub struct ExcelViewer {
    // ...现有字段...
    pub license: LicenseManager,
    pub license_popup: LicensePopupState,
}

impl ExcelViewer {
    pub fn new() -> Self {
        let license = LicenseManager::load();     // 启动加载 + 回拨检测
        Self {
            // ...
            license_popup: LicensePopupState {
                visible: license.status(crate::license::time::today_epoch_day()).is_blocking(),
                machine_code: license.machine_code().to_string(),
                ..Default::default()
            },
            license,
        }
    }
}

// 在 ui() 每帧末尾：
let status = self.license.status(crate::license::time::today_epoch_day());
draw_license_popup(&ctx, &mut self.license_popup, &status, &mut |code| {
    match self.license.activate(code, crate::license::time::today_epoch_day()) {
        Ok(_) => { self.license_popup.visible = false; self.license_popup.show_activated = true; }
        Err(e) => self.license_popup.error = Some(error_msg(e)),
    }
});
// 运行正常时推进高水位
if !status.is_blocking() {
    self.license.checkpoint(crate::license::time::today_epoch_day());
}
```

---

## 十、配套 keygen 工具（开发者离线生成授权码）

单独一个小 crate（含私钥，**绝不分发**）：

```rust
// keygen/src/main.rs —— 离线运行，读私钥 + 机器码 → 输出授权码
use ed25519_dalek::{Signer, SigningKey};
use base64::Engine;

fn main() {
    let priv_bytes: [u8; 32] = [/* 从加密文件读出 */];
    let signing = SigningKey::from_bytes(&priv_bytes);

    let machine  = std::env::args().nth(1).expect("machine code");
    let days     = std::env::args().nth(2).and_then(|d| d.parse::<u64>().ok()).unwrap_or(0); // 0=永久
    let today    = /* epoch day */;
    let expires  = if days == 0 { 0 } else { today + days };
    let customer = std::env::args().nth(3).unwrap_or_default();

    let text = format!(
        "v=1\np=umya-excel\nm={machine}\ni={today}\ne={expires}\ned=pro\nc={customer}\n"
    );
    let sig = signing.sign(text.as_bytes());        // 64 字节
    let b64 = base64::engine::general_purpose::STANDARD;
    println!("{}.{}", b64.encode(text), b64.encode(sig));
}
```

**一次性生成密钥对**：

```rust
use rand::rngs::OsRng;
let mut rng = OsRng;
let sk = SigningKey::generate(&mut rng);
let pk = sk.verifying_key();
// sk.to_bytes() → 私钥文件(32B)，妥善离线保管；
// pk.to_bytes() → 填入 crypto.rs::DEVELOPER_PUBLIC_KEY
```

**典型使用**：

```
keygen.exe ABCD-1234-EF89-5678 365 "客户公司名"
# 输出：eyJ2PTEK...<base64>.<base64签名>
```

把整串发给客户，客户粘贴进激活弹窗即可。

---

## 十一、现实局限与进阶建议

**本方案能挡住**：授权码伪造（Ed25519）、改时间（高水位）、删文件（注册表冗余）、改试用状态（HMAC）、一码多机（机器码绑定）。

**本方案挡不住**：专业逆向直接 patch `ed25519_verify` 返回 `true`。进阶对策（按预算）：

1. 校验点分散到核心功能（保存 / 导出 / 打印），而非仅启动。
2. 二进制自校验 + `strip` + LTO，提高静态分析成本。
3. 预算允许时上 VMProtect / Themida 等商业壳。
4. 公钥 / 胡椒不要明文常量，运行时拼装。

---

## 十二、实施步骤建议

1. 抽离 `src/util/date.rs`，把 `main.rs` 的日期函数迁移过去并 `pub`。
2. 新建 `src/license/` 各子模块，先实现 `crypto` / `fingerprint` / `payload` / `time`（可单元测试）。
3. 写 keygen 工具生成密钥对，把公钥填入 `crypto.rs`。
4. 实现 `store`（文件 + 注册表）与 `LicenseManager::load / status / checkpoint`。
5. 实现 `license_popup` 并接入 `viewer.rs`。
6. `Cargo.toml` 加入依赖，`--release` 编译验证。
7. 测试：试用倒计时、到期拦截、激活、回拨检测、删文件后注册表兜底。
8. 查看本机注册表路径：`umya-spreadsheet-excel.exe --uuid`。
