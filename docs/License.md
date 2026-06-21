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
| **本地存储加密** | **AES-256-GCM**（AEAD） | 文件 + 注册表均加密存储；密钥由机器指纹派生，换机器无法解密；GCM 认证标签保证任何篡改都会导致解密失败。 |
| **试用状态防篡改** | **HMAC-SHA256** | 对称、快；密钥由”机器指纹 + 内置胡椒”派生，换机器 / 改文件都校验失败。 |
| **机器指纹** | **SHA-256** 聚合多个稳定标识 | 绑定到具体机器。 |

**密钥拓扑**：

- 开发者离线生成一对 Ed25519 密钥，**私钥永不分发**（最好放离线机 / 加密 U 盘）。
- **32 字节公钥**以二进制格式写入 `keygen/public_key.bin`，由 `crypto.rs` 通过 `include_bytes!` 在编译时嵌入。
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

[试用期内（剩余天数 > 0，可选）]
  用户可经"关于 → 激活"菜单主动唤起付款/激活弹窗提前激活
  ├─ 与到期拦截共用同一个 LicensePopupState，仅多一个右上角"[X]"关闭按钮（可关闭）
  └─ 关闭后继续试用，倒计时 / 高水位逻辑照常运行（不延长试用）

[试用到期]
  UI 渲染遮罩 → 付款二维码弹窗（is_blocking 自动弹出，无"关闭"按钮，强制激活）
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

### 6.1 存储位置（多点分散 + 差异化加密 + 交叉校验，防删文件 / 防批量定位绕过）

试用状态 + 授权码分散写入 **5 个存储点**（文件 + 注册表），每点用**分位置密钥**加密 → 各点密文互不相同，单点删除无法绕过，按内容批量定位也失效。实现见 [`src/license/store.rs`](../src/license/store.rs) 与 [`src/license/crypto.rs`](../src/license/crypto.rs)。

| tag | 位置 | 值名 | 说明 |
|---|---|---|---|
| `home` | `~/.MyExcel/license.dat` | 文件整体 | 既有主存储（保兼容） |
| `config` | `{config_dir}/{dir_uuid(config)}/state.dat`（如 `%APPDATA%\{dir_uuid(config)}\state.dat`） | 文件整体 | 新增分散点 |
| `local` | `{data_local_dir}/{dir_uuid(local)}/cache.bin`（如 `%LOCALAPPDATA%\{dir_uuid(local)}\cache.bin`） | 文件整体 | 新增分散点 |
| `regmain` | 注册表 `HKCU\Software\{uuid}` | `Data` | 既有（路径由硬件派生 UUID 混淆） |
| `regclsid` | 注册表 `HKCU\Software\Classes\CLSID\{大写 dir_uuid(regclsid)}`（如 `…\CLSID\{71445FAC-…}`，UUID 由 `dir_uuid("regclsid")` 动态派生后转大写并加花括号，**大写 + 花括号**，Windows CLSID 惯例） | `Data` | 新增分支（仅 Windows） |
| `regmain` | 注册表 `HKCU\Software\{uuid}` | `LicenseBlob` | AES-256-GCM 加密的导出格式（供 `--license` 显示，**保持无 tag**） |

> **注册表 UUID 格式差异**：`regmain` 子键用小写、无花括号的 UUID（`Software\{uuid}`，此处 `{uuid}` 仅为占位）；`regclsid` 单独用**大写 + 花括号**的 CLSID 风格 UUID（`Software\Classes\CLSID\{大写UUID}`，如 `…\{71445FAC-…}`），符合 Windows CLSID 命名惯例、更像合法 COM 条目。两者分别由 `fingerprint::registry_uuid()`（regmain）与 `fingerprint::registry_uuid_clsid()`（regclsid）派生——其中 regclsid 的 UUID 生成策略与 `config` / `local` 一致：先以本点前缀调用 `dir_uuid("regclsid")` 动态派生每点不同的 UUID（**非**直接复用 `registry_uuid()`），再转大写并加花括号成 CLSID 风格。UUID 格式只决定**注册表路径**，不影响加密密钥（密钥由 tag 派生，故 `--license` 解密不受影响）。

**目录名混淆（config / local）**：这两点的目录名不再用固定的 `MyExcel`，而是按存储位置前缀派生一个**每机确定、每点不同**的 UUID 目录名 `{dir_uuid(prefix)}`。生成规则：以机器派生 UUID `registry_uuid()`（见 §9.4）为基，将位置前缀字符串与该 UUID **拼接后计算**得到最终目录名——

```text
dir_uuid(prefix) = uuid_v5_style( sha256( prefix_string + registry_uuid() ) )
# 取 SHA-256 的前 16 字节，按 UUID v5 风格格式化（与 registry_uuid() 同套派生方式）
# config 点用前缀 "config" → dir_uuid("config")；local 点用前缀 "local" → dir_uuid("local")
# 例：得到形如 71445fac-d6ef-5436-9da7-5a323762d7f5 的目录名，且两个前缀产出互不相同
```

故 config / local / 注册表 `{uuid}` 三者目录名互不相同、且与机器绑定——路径中无可识别的固定产品名，进一步抬高"按路径批量定位并删除"的门槛。

> 所有内部存储点均为 AES-256-GCM 加密。`dirs` 返回 `None`（极少数环境）的点直接跳过；非 Windows 仅有 3 个文件点。`regclsid` 分支为 user-writable、best-effort：偶有 AV/EDR 会清理 `CLSID` 下的值，清理后该点缺失（按"非篡改"处理），不影响其余 4 点兜底。

**差异化加密**：每点密钥 = `SHA256(LOCATION_LABEL || PEPPER || 机器指纹 || tag)`（`tag` 即位置标识，`LOCATION_LABEL` 与导出/HMAC 的标签区隔以防密钥复用），且每次写入随机 nonce。因此：

- 各点密文**互不相同** → 无法凭一份样本按内容 `find` / 注册表搜索批量定位全部；
- 无法把 A 点密文搬到 B 点解密（密钥不同）→ 抗重定位；
- 路径 / 文件名 / 注册表分支均不同 → 按路径批量定位也失效。

**明文格式**（加密前，两行）：

```text
f=<first_run_day>|l=<last_run_day>|r=<rollback_count>|mac=<hex>|loc=<tag>|mani=<manifest>
<授权码或空>
```

- `mac` = HMAC over `f|l|r`（核心试用字段，与 `TrialState::body` 一致）；
- `loc`（本点 tag）与 `mani`（清单哈希 = `sha256(排序后全部 tag)`，每个二进制版本固定）追加在后，**不**进 HMAC，但被 AES-GCM 整体认证 → 防篡改。

**加载与交叉校验**（`store::load`）：

1. 每点**三级解密兜底**：分位置解密 → 无 tag 旧版解密（兼容升级前数据）→ 原始明文（pre-encryption 时代）；
2. 缺失点**不算**篡改（best-effort 写入，单点失败 / 被 AV 清理不影响其余）；
3. 有数据但解不开 / HMAC 失败 / 当前版本 blob 的 `loc` 与读取点 tag 不符 → **篡改**；
4. 合并：`min(first_run_day)` / `max(last_run_day)` / `max(rollback_count)`，重新签名；
5. **交叉校验**（纯函数 `cross_validate`，仅"当前版本"blob 参与）：≥2 点的 `first_run_day` 或非空 license 不一致 → **篡改**。`last_run_day` / `rollback_count` 的不一致**不算**篡改（volatile，靠 `max` 合并即可，避免中断保存误锁合法用户）。

```rust
// 对试用状态：取所有幸存点中 last_run_day 的最大值（高水位）
// → 删任意子集都不降高水位：幸存点仍记得"已经用了 35 天"，试用不重置、不进 Tampered
// 对授权码：各点都验签；任一有效且机器码匹配即放行
// 交叉校验：各点 first_run_day / license 必须一致，否则判定篡改
```

> **为什么删点无法重置试用**：每次 `save()` 向**所有**点写入**相同的** `f|l|r` 值（但**不同**密文）。删除任意**子集**都不能降低高水位——幸存点仍记着最先进的 `last_run_day`，`max` 合并生效，试用**不**重置、也**不**进入 Tampered。只有删除**全部**点才回到"首次运行"（离线场景不可判定的固有局限），而差异化密文 + 分散路径 / 文件名 / 注册表分支让"找到并删除全部"成本极高。

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
| 删除 `license.dat` 重置 | **5 个分散存储点**（3 文件 + 2 注册表分支），各点**差异化密文**；删任意子集不降高水位（幸存点 `max(last_run_day)` 生效），删全部才重置（见 6.1） |
| 按内容 / 路径批量定位删除全部存储 | 每点**分位置密钥**（密文互不相同）+ 分散路径 / 文件名 / 注册表分支 → 无统一模式可 grep / find |
| 搬迁 / 复制某点密文到他处绕过 | 分位置密钥（tag 绑定密钥）+ blob 内嵌 `loc` 校验 → 换位置解密失败、加载标篡改 |
| 手改试用状态文件 | AES-256-GCM 加密存储 + HMAC 双重校验，改密文或改明文均失败 |
| 直接读取授权数据 | AES-256-GCM 加密，无机器密钥无法解密 |
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
aes-gcm = "0.10"                                               # 本地存储加密（AEAD）
getrandom = "0.2"                                              # AES-GCM 随机 nonce 生成
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

/// ⚠️ 内嵌的开发者公钥（32 字节）。由 keygen `gen-keys` 生成，私钥离线保管。
/// 从 keygen/public_key.bin 二进制文件在编译时嵌入。
const DEVELOPER_PUBLIC_KEY: [u8; 32] = *include_bytes!("../../keygen/public_key.bin");

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

/// 导出用加密密钥上下文标签（与 HMAC_PEPPER 不同，避免密钥复用）
const EXPORT_LABEL: &[u8] = b"umya-excel-license-export-v1";

/// 从机器指纹派生 AES-256 加密密钥（32 字节）
fn derive_export_key(machine_fingerprint: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(EXPORT_LABEL);
    hasher.update(HMAC_PEPPER);
    hasher.update(machine_fingerprint);
    let out = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out);
    key
}

/// AES-256-GCM 加密。返回 `base64(nonce[12] || ciphertext || tag[16])`。
pub fn aes256gcm_encrypt(plaintext: &[u8], machine_fingerprint: &[u8]) -> Option<String> {
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

    let key = derive_export_key(machine_fingerprint);
    let cipher = Aes256Gcm::new_from_slice(&key).ok()?;
    let mut nonce_bytes = [0u8; 12];
    if getrandom::getrandom(&mut nonce_bytes).is_err() { return None; }
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, plaintext).ok()?;

    let mut out = Vec::with_capacity(12 + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Some(base64::engine::general_purpose::STANDARD.encode(&out))
}

/// AES-256-GCM 解密。接受 `base64(nonce || ciphertext || tag)` 格式。
pub fn aes256gcm_decrypt(encoded: &str, machine_fingerprint: &[u8]) -> Option<Vec<u8>> {
    use aes_gcm::aead::Aead;
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

    let data = b64_decode(encoded)?;
    if data.len() < 12 + 16 { return None; }

    let key = derive_export_key(machine_fingerprint);
    let cipher = Aes256Gcm::new_from_slice(&key).ok()?;
    let nonce = Nonce::from_slice(&data[..12]);
    cipher.decrypt(nonce, &data[12..]).ok()
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

### 9.5 `license/store.rs` —— 持久化（AES-256-GCM 加密 + 文件 + 注册表冗余）

```rust
//! 本地存储：试用状态 + 授权码，AES-256-GCM 加密存储 + HMAC 校验 + 多位置冗余。
//!
//! 文件 license.dat 存储加密后的 base64 单行密文（解密后为两行：试用状态 + 授权码）。
//! 注册表 HKCU\Software\{uuid} 存储加密副本（Data 值）及导出格式（LicenseBlob 值）。
//! 加载时先尝试 AES-256-GCM 解密，失败则按明文解析（向后兼容升级前数据）。

use std::path::PathBuf;

const TRIAL_FILENAME: &str = "license.dat";
#[cfg(windows)]
fn reg_path() -> String {
    format!("Software\\{}", crate::license::fingerprint::registry_uuid())
}

/// 尝试对存储内容做 AES-256-GCM 解密。
/// 成功返回解密后明文；失败（非加密格式 / 被篡改 / 错误机器）返回 None。
fn try_decrypt(content: &str, machine_fp: &[u8]) -> Option<String> {
    let bytes = super::crypto::aes256gcm_decrypt(content.trim(), machine_fp)?;
    std::str::from_utf8(&bytes).ok().map(String::from)
}

pub fn save(trial: &TrialState, license_raw: &Option<String>, machine_fp: &[u8]) {
    // 1) 构建明文：trial_line + lic_line（两行）
    // 2) AES-256-GCM 加密 → base64 密文
    // 3) 写入 ~/.MyExcel/license.dat（加密单行）
    // 4) [cfg(windows)] 写注册表 Data 值（加密副本）+ LicenseBlob 值（导出格式）
}

pub fn load(machine_fp: &[u8]) -> LoadResult {
    // 读文件 → try_decrypt → 解密成功则用明文，失败按明文格式解析（兼容旧版）
    // 读注册表 Data 值 → 同上
    // 合并：trial 取 max(last_run_day)；license 任一有效即可
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

### 9.9 提前激活入口与可关闭模态（关于菜单 + 右上角 [X] 关闭按钮）

除“试用到期 → 自动拦截弹窗”这条主路径外，试用期内（剩余天数 > 0）用户也可**主动**唤起激活弹窗提前激活；到期 / 篡改等拦截态下弹窗则**强制不可关闭**。两者共用同一个 `LicensePopupState`，仅由 `can_close` 标志区分行为。

**统一判断条件**（“试用中且剩余天数 > 0”）：

```rust
let can_close = matches!(lic_status, LicenseStatus::Trial { days_left } if *days_left > 0);
```

**① 关于菜单 → “激活”子菜单项**（`menu_bar.rs`）

在“关于”下拉菜单中，分隔符之后、“帮助”之前，仅当处于试用期内时渲染“激活”按钮，点击即打开激活模态（`draw_menu_bar` 新增 `license_popup: &mut LicensePopupState` 参数）：

```rust
ui.separator();
// 仅在试用期内（剩余天数 > 0）显示"激活"入口，允许用户提前激活
let in_trial = matches!(lic_status, LicenseStatus::Trial { days_left } if *days_left > 0);
if in_trial {
    if ui.button("激活").clicked() {
        ui.close();
        license_popup.visible = true;   // 唤起激活模态
    }
}
if ui.button("帮助").clicked() { /* ... */ }
```

> 试用期已结束 / 篡改等拦截态由 `viewer.rs` 自动 `license_popup.visible = true`（参见 9.8），故菜单无需再提供入口。

**② 激活模态 → 右上角“[X]”关闭按钮**（`license_popup.rs`）

`draw_license_popup` 新增 `can_close: bool` 参数，控制是否渲染**弹窗右上角**的“[X]”关闭按钮：

- `can_close == true`（试用期内，通常由用户经“关于 → 激活”主动唤起）：右上角渲染“[X]”按钮，点击于帧末置 `state.visible = false` 关闭弹窗；
- `can_close == false`（到期 / 篡改 / 授权到期等拦截态）：**完全不渲染**该按钮，模态保持无标题栏、不可拖动、不可关闭，强制完成激活。

“[X]”按钮独立置于弹窗右上角，**不**与右下角的“激活”按钮同行：“激活”按钮始终渲染（用于验签激活），“[X]”仅在可关闭时渲染。

实现要点：弹窗主体为 `vertical_centered`，若直接用 `ui.with_layout`（egui 中其默认占用父级**全部剩余区域**）会挤占居中内容，故改用 `ui.allocate_ui_with_layout` 申请一条**固定高度（`interact_size.y`）、整宽**的顶部条带，再以 `right_to_left(TOP)` 布局把“[X]”钉到条带右上角；居中内容占其下剩余高度。

```rust
// viewer.rs：每帧由授权状态派生 can_close 并传入
draw_license_popup(&ctx, &mut self.license_popup, &status_text, can_close, &mut activate_cb);

// license_popup.rs：set_height 之后、vertical_centered 之前，渲染右上角 [X]
ui.allocate_ui_with_layout(
    egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
    egui::Layout::right_to_left(egui::Align::TOP),
    |ui| {
        // 短路求值：can_close 为 false 时不调用 ui.button，按钮不渲染
        if can_close && ui.button("[X]").clicked() {
            close_clicked = true;       // 帧末置 state.visible = false
        }
    },
);
// 右下角始终渲染“激活”按钮（与 [X] 分处弹窗上下两行）
ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
    let activate_clicked = ui.button("激  活").clicked();
    if activate_clicked { /* 验签激活 */ }
});
// .show() 之后
if close_clicked { state.visible = false; }
```

**安全性说明**：该“[X]”按钮是 UI 层便利性，不影响授权强度——拦截态（`is_blocking()`）下 `can_close` 恒为 `false`，按钮根本不渲染，用户无法绕过强制激活；试用期内允许关闭仅意味着“暂不激活、继续试用”，试用倒计时与防回拨高水位逻辑（见 6.2）照常运行、不延长试用期。

---

### 9.10 弹窗视觉布局图（`license_popup.rs`）

> 以下布局依据 `src/gui/widgets/license_popup.rs` 中 `draw_license_popup` 的 egui 组件层级绘制，描述弹窗在屏幕居中渲染时的实际视觉结构。`can_close` 由调用方（`viewer.rs`）依据 `LicenseStatus` 每帧派生：试用期内（剩余天数 > 0）为 `true`，到期 / 篡改等拦截态为 `false`。

#### 9.10.1 组件层级树

```
egui::Window "license_gate"
│   属性: title_bar(false) · resizable(false) · collapsible(false) · movable(false)
│         order(Foreground) · anchor(CENTER_CENTER,[0,0])
│         set_min_width(400.0) · set_height(300.0)
│
├──【顶部条带】allocate_ui_with_layout
│       布局: right_to_left(TOP)  尺寸: 整宽 × interact_size.y
│   └── Button "[X]"            (仅 can_close=true 渲染 → close_clicked=true → 帧末 visible=false)
│
└──【主体】vertical_centered      (占顶部条带以下全部剩余高度，子项整体水平居中)
    ├── add_space(4.0)
    ├── Label status_text                  (16px · strong 加粗)
    ├── add_space(8.0)
    │
    ├── ══ 分支 A：激活成功态 (activated_timer > 0) ══
    │   ├── add_space(20.0)
    │   ├── Label "✅ 激活成功，感谢支持！"      (15px · 绿色 rgb(0,150,0))
    │   └── add_space(20.0)
    │   (1.5 秒倒计时结束后 hide_after_frame → visible=false)
    │
    └── ══ 分支 B：常规激活态 (else，默认) ══
        ├── Image 二维码纹理                  (200×200)   ── 无纹理时占位 set_min_height(200)
        ├── Label "扫码付款(9.9元/30天)后，联系开发者获取授权码"
        ├── add_space(8.0)
        ├── Label "本机机器码（请发送给开发者）："
        ├── horizontal                        (机器码 + "复制" 整组水平居中)
        │   ├── allocate_space(left_pad)      ── left_pad=(可用宽-整组宽)/2，实现整组居中
        │   ├── monospace 机器码              (14px · 灰底 rgb(235))
        │   └── Button "复制"                 → ctx.copy_text(code)
        ├── add_space(12.0)
        ├── Label "输入授权码："
        ├── TextEdit multiline                (desired_width 380 · desired_rows 3)
        ├── colored_label error               (红 rgb(200,0,0), 仅 state.error 时)
        ├── add_space(8.0)
        └── with_layout right_to_left(Center)
            └── Button "激  活"               (14px) → 校验非空 → on_activate(code)
                                                Ok  → activated_timer=1.5
                                                Err → state.error=Some(msg)
```

#### 9.10.2 窗口属性

| 属性 | 取值 | 说明 |
|---|---|---|
| 标题栏 | `title_bar(false)` | 无原生标题栏，纯自绘内容 |
| 锚定 | `anchor(CENTER_CENTER, [0, 0])` | 屏幕正中，偏移 0 |
| 层级 | `Order::Foreground` | 覆盖主界面，模态遮罩 |
| 缩放 | `resizable(false)` | 不可拖拽拉伸 |
| 折叠 | `collapsible(false)` | 不可折叠收起 |
| 拖动 | `movable(false)` | 不可拖动移位 |
| 最小宽 | `set_min_width(400.0)` | 400px |
| 高度 | `set_height(300.0)` | 标称 300px；内容超出时窗口自动增高 |

#### 9.10.3 常规激活态布局（分支 B，主视图）

拦截态（`can_close=false`）下右上角**无** `[X]` 按钮，弹窗强制不可关闭；试用期内（`can_close=true`）则在顶部条带右上角额外渲染 `[X]`。两种情形的主体内容完全一致：

```
        ┌─────────────────────────────────────────────┐  ← 最小宽 400px
        │  顶部条带 (整宽 × interact_size.y)         [X]│     仅 can_close=true 时
        ├─────────────────────────────────────────────┤     渲染右上角 [X]
        │              ↑ 4px                          │
        │        状态文本  (16px · 加粗)               │  status_text
        │              ↓ 8px                          │
        │     ┌───────────────────┐                   │
        │     │                   │                   │
        │     │   二维码 200 × 200 │   ← 水平居中       │
        │     │                   │                   │
        │     └───────────────────┘                   │
        │  扫码付款(9.9元/月)后，联系开发者获取授权码    │
        │              ↑ 8px                          │
        │  本机机器码（请发送给开发者）：                │
        │       ┌──────────────┐ 6px ┌─────┐          │
        │       │ XXXX-XXXX-…  │     │ 复制 │          │  ← 整组水平居中
        │       └──────────────┘     └─────┘          │    (monospace 14px · 灰底)
        │              ↑ 12px                         │
        │           输入授权码：                       │
        │    ┌──────────────────────────────┐         │
        │    │                              │         │
        │    │   TextEdit (宽 380 · 3 行)    │         │
        │    │                              │         │
        │    └──────────────────────────────┘         │
        │    [红色错误提示  ·  仅 state.error 时显示]   │
        │              ↑ 8px                          │
        │                          ┌─────────┐        │
        │                          │  激  活  │        │  ← 右对齐 right_to_left
        │                          └─────────┘        │
        └─────────────────────────────────────────────┘
                          ↑
          标称高 300px (set_height)，内容超出时窗口自动增高
```

#### 9.10.4 激活成功态布局（分支 A，`activated_timer > 0`）

验签通过后 `activated_timer` 置为 1.5 秒，弹窗切到成功提示；倒计时归零后于帧末自动关闭（`visible=false`）。此态下**不渲染**二维码 / 机器码 / 输入框 / 激活按钮：

```
        ┌─────────────────────────────────────────────┐
        │  顶部条带                                  [X]│
        ├─────────────────────────────────────────────┤
        │              ↑ 4px                          │
        │        状态文本  (16px · 加粗)               │
        │              ↓ 8px + 20px                   │
        │                                              │
        │                                              │
        │      ✅ 激活成功，感谢支持！  (15px · 绿)     │
        │                                              │
        │              ↓ 20px                          │
        └─────────────────────────────────────────────┘
              1.5 秒后 hide_after_frame → 自动关闭
```

#### 9.10.5 交互区域与对齐说明

| 区域 | 位置 / 对齐 | 交互行为 |
|---|---|---|
| `[X]` 关闭按钮 | 顶部条带右上角（`right_to_left(TOP)`） | 仅 `can_close=true` 渲染；点击 → `visible=false` |
| 二维码 | 主体顶部，水平居中 | 仅展示（`ui.image`，无可点交互） |
| 机器码文本 | 与"复制"按钮成组，**整组**水平居中 | 仅展示；不可选中编辑 |
| "复制"按钮 | 紧邻机器码右侧，组内间距 6px | 点击 → `ctx.copy_text(code)` 写入剪贴板 |
| 授权码输入框 | 居中，宽 380px / 3 行 | 多行可编辑（`TextEdit::multiline`） |
| 错误提示 | 输入框正下方，红色 | 仅 `state.error` 为 `Some` 时出现 |
| "激  活"按钮 | 主体底部右对齐（`right_to_left(Center)`） | 点击 → 空则报错"请输入授权码"；非空调用 `on_activate` |
| 成功提示 | 替换整个输入区（分支 A） | 无交互，1.5 秒自动消失 |

#### 9.10.6 尺寸与间距一览

| 元素 | 尺寸 / 样式 | 来源 |
|---|---|---|
| 窗口最小宽 | 400px | `set_min_width(400.0)` |
| 窗口标称高 | 300px（最小值，可增高） | `set_height(300.0)` |
| 顶部条带高 | `interact_size.y`（≈ 按钮行高） | `ui.spacing().interact_size.y` |
| 状态文本 | 16px · 加粗 | `RichText::size(16.0).strong()` |
| 二维码 | 200×200px | `SizedTexture::new(.., [200.0, 200.0])` |
| 成功提示 | 15px · 绿色 rgb(0,150,0) | `RichText::size(15.0).color(..)` |
| 机器码 | monospace 14px · 灰底 rgb(235) | `monospace(..).background_color(..)` |
| 机器码 ↔ "复制" 间距 | 6px | `gap = 6.0` / `item_spacing.x = gap` |
| 整组居中左留白 | `(可用宽 − 整组宽) / 2` | `left_pad` 测量后计算 |
| 授权码输入框 | 宽 380px · 3 行 | `desired_width(380.0).desired_rows(3)` |
| 错误提示 | 红色 rgb(200,0,0) | `colored_label(..)` |
| "激  活"按钮 | 14px 文本 | `RichText::new("激  活").size(14.0)` |
| 段间垂直留白 | 4 / 8 / 8 / 12 / 8 / 20 px | `ui.add_space(..)` |

> **整组居中技巧**：egui 的 `ui.horizontal` 会声明满宽且左对齐，无内置"整组居中"。代码先用 `painter.layout_no_wrap` 测量机器码文本宽与"复制"按钮宽（`text宽 + button_padding*2`，且不小于 `interact_size.x`），得 `整组宽 = 码宽 + 6 + 按钮宽`，再在行首 `allocate_space(left_pad)` 预留 `(可用宽 − 整组宽)/2` 实现居中。

---

## 十、配套 keygen 工具（开发者离线生成授权码）

单独一个小 crate（含私钥，**绝不分发**）：

```rust
// keygen/src/main.rs —— 离线运行，读私钥 + 机器码 → 输出授权码
use ed25519_dalek::{Signer, SigningKey};
use base64::Engine;

fn main() {
    let seed_bytes = std::fs::read("private_key.bin").expect("read private key failed");
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&seed_bytes);
    let signing = SigningKey::from_bytes(&seed);

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
use rand::RngCore;
let mut seed = [0u8; 32];
OsRng.fill_bytes(&mut seed);
let signing = SigningKey::from_bytes(&seed);
let pk = signing.verifying_key();
// seed → 写入 private_key.bin（32B），妥善离线保管，勿提交 git；
// pk.to_bytes() → 写入 public_key.bin（32B 二进制），供 crypto.rs 通过 include_bytes! 引用
```

**典型使用**：

```
keygen.exe ABCD-1234-EF89-5678 365 "客户公司名"
# 输出：eyJ2PTEK...<base64>.<base64签名>
```

把整串发给客户，客户粘贴进激活弹窗即可。

---

## 十一、现实局限与进阶建议

**本方案能挡住**：授权码伪造（Ed25519）、改时间（高水位）、删文件重置（5 点分散 + 差异化密文，删全部才重置）、按内容/路径批量定位删除（密文互不相同 + 分散路径）、改试用状态（HMAC + AES-256-GCM）、直接读取授权数据（AES-256-GCM 加密存储）、一码多机（机器码绑定）。

**本方案挡不住**：专业逆向直接 patch `ed25519_verify` 返回 `true`。进阶对策（按预算）：

1. 校验点分散到核心功能（保存 / 导出 / 打印），而非仅启动。
2. 二进制自校验 + `strip` + LTO，提高静态分析成本。
3. 预算允许时上 VMProtect / Themida 等商业壳。
4. 公钥 / 胡椒不要明文常量，运行时拼装。

---

## 十二、实施步骤建议

1. 抽离 `src/util/date.rs`，把 `main.rs` 的日期函数迁移过去并 `pub`。
2. 新建 `src/license/` 各子模块，先实现 `crypto` / `fingerprint` / `payload` / `time`（可单元测试）。
3. 写 keygen 工具生成密钥对（私钥写入 `private_key.bin`，公钥写入 `keygen/public_key.bin`），`crypto.rs` 通过 `include_bytes!` 自动引用公钥。
4. 实现 `store`（AES-256-GCM 加密 + 文件 + 注册表）与 `LicenseManager::load / status / checkpoint`。
5. 实现 `license_popup` 并接入 `viewer.rs`。
6. `Cargo.toml` 加入依赖（含 `aes-gcm`、`getrandom`），`--release` 编译验证。
7. 测试：试用倒计时、到期拦截、激活、回拨检测、删文件后注册表兜底。
8. 查看本机注册表路径：`umya-spreadsheet-excel.exe --uuid`。
9. 导出授权状态：`umya-spreadsheet-excel.exe --license "<LicenseBlob值>"`。

---

## 十三、`--license` 加密导出

### 13.1 用途

用于技术支持场景：用户把一段加密字符串发给开发者，开发者用本机 `--license` 解密查看授权状态。
加密字符串由程序在每次调用 `store::save()`（激活、每日 checkpoint、自愈补写）时**自动生成**并分散写入多处存储，无需用户手动操作。

> **兼容任意存储来源**：`--license` 不绑定具体存储位置——无论该串来自注册表 `LicenseBlob` 导出，还是任一存储点（`license.dat` / `state.dat` / `cache.bin` / 注册表 `Data`）的内部密文，都能解密。详见 13.4。

### 13.2 自动生成

程序在每次 `store::save()` 时向以下 6 处写入加密数据。**前 5 处是内部存储点**（各用**分位置密钥**，密文互不相同），第 6 处 `LicenseBlob` 是**导出专用**（无 tag 导出密钥）。`--license` 对这 6 处的密文**全部可解**（见 13.4）。

| tag | 位置 | 文件/值名 | 加密密钥 | `--license` 可解 |
|---|---|---|---|---|
| `home` | `~/.MyExcel/` | `license.dat` | 分位置（tag=`home`） | ✅ |
| `config` | `{config_dir}/{dir_uuid(config)}/` | `state.dat` | 分位置（tag=`config`） | ✅ |
| `local` | `{data_local_dir}/{dir_uuid(local)}/` | `cache.bin` | 分位置（tag=`local`） | ✅ |
| `regmain` | `HKCU\Software\{uuid}` | `Data` | 分位置（tag=`regmain`） | ✅ |
| `regclsid` | `HKCU\Software\Classes\CLSID\{大写UUID}`（大写 + 花括号） | `Data` | 分位置（tag=`regclsid`） | ✅ |
| `regmain` | `HKCU\Software\{uuid}` | `LicenseBlob` | 无 tag 导出密钥 | ✅ |

> 每个存储点密文各不相同（分位置密钥 + 随机 nonce），故**无法凭一份样本按内容批量定位**；但 `--license` 会依次尝试全部密钥，故来源不影响解密。

获取加密字符串的几种方式（任选其一，PowerShell；将 `{uuid}` / `{dir_uuid(...)}` 替换为本机实际值）：

```powershell
# 1) 查看本机注册表路径 UUID（用于下面的 {uuid}）
.\umya-spreadsheet-excel.exe --uuid

# 2a) 从注册表 LicenseBlob 读取（导出专用）
Get-ItemProperty "HKCU:\Software\{uuid}" | Select-Object -ExpandProperty LicenseBlob

# 2b) 从注册表 Data 读取（regmain 存储点密文）
Get-ItemProperty "HKCU:\Software\{uuid}" | Select-Object -ExpandProperty Data

# 2c) 从文件存储点读取（如 local 点的 cache.bin）
Get-Content "$env:LOCALAPPDATA\{dir_uuid(local)}\cache.bin" -Raw
```

### 13.3 解密查看

```cmd
.\umya-spreadsheet-excel.exe --license "加密字符串"
```

无论入参来自哪个存储位置，输出都**统一规范化为导出格式**：存储点内部密文（带 `mac`/`loc`/`mani` 的内部格式）会被解析为 trial + license 后重新格式化，故输出与来源无关。

输出格式：

```
f=20622|l=20622|r=0|mac=abc123def456...
```

字段说明：

| 字段 | 类型 | 含义 |
|---|---|---|
| `f` | epoch 天数 | `first_run_day`：首次启动日（试用/激活起点） |
| `l` | epoch 天数 | `last_run_day`：高水位（已观测到的最大天数，防时钟回拨） |
| `r` | 天数 | 剩余天数：`0` = 永久授权；正整数 = 距离到期的剩余天数 |
| `mac` | hex 字符串 | 本机机器指纹的 SHA-256 摘要（用于验证绑机身份） |

### 13.4 多存储位置兼容性

`--license` 的解密逻辑（`store::decrypt_for_display`）依次尝试以下密钥，**短路于第一个成功**：

1. **无 tag 导出密钥**（`LicenseBlob` 原生格式）—— `derive_export_key(机器指纹)`；
2. **各存储点的分位置密钥**（`home` / `config` / `local` / `regmain` / `regclsid`）—— `derive_location_key(机器指纹, tag)`。

由于 AES-256-GCM 的认证标签，错误密钥必然解密失败、只有匹配的密钥能成功——故任一存储位置的密文都能被正确解密，且不会误判。解密成功后，若明文是内部存储格式（带 `mac`/`loc`/`mani`），会被 `normalize_for_display` 重新格式化为导出格式（13.3），使输出与来源无关。

### 13.5 加密方案

| 项目 | 说明 |
|---|---|
| **算法** | AES-256-GCM（AEAD，同时提供加密和完整性校验） |
| **内部存储点密钥** | `SHA-256(LOCATION_LABEL + 胡椒 + 机器指纹 + tag)` → 32B；每个 tag 不同 |
| **导出密钥（LicenseBlob）** | `SHA-256(EXPORT_LABEL + 胡椒 + 机器指纹)` → 32B；无 tag |
| **Wire 格式** | `base64( nonce[12字节] \|\| 密文 \|\| GCM_tag[16字节] )` |
| **Nonce** | 每次生成使用 OS 随机源，同一明文产出不同密文 |
| **防篡改** | 16 字节 GCM 认证标签保证任何字节篡改都会导致解密失败 |
| **绑机** | 密钥由机器指纹派生，换机器无法解密 |
| **密钥隔离** | 导出 / 分位置 / HMAC 三套密钥用不同上下文标签，避免复用 |

### 13.6 错误信息

| 输出 | 原因 |
|---|---|
| `f=...\|l=...\|r=...\|mac=...` | 解密成功，授权状态信息 |
| `Error: invalid or tampered license string, or wrong machine` | 所有密钥都解不开：串被篡改 / 损坏、格式错误、或非本机生成的加密串 |
| `Error: --license requires an argument` | 缺少加密字符串参数 |

