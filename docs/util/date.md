# `util/date.rs` 文档

## 1. 模块职责

`src/util/date.rs` 提供**基于 Unix epoch 的纯算术日期/时间换算工具**，全模块不依赖 `chrono` 或任何第三方日期库。它是项目内日期换算的**单一事实来源**，被 `license::time`（到期日显示）、`util::backup`（备份命名时间戳）以及 `viewer.rs`（保存路径日期后缀）复用，避免各处重复实现历法换算。

## 2. 主要类型与函数

本模块无自定义类型，导出三个 `pub fn`：

| 函数 | 签名 | 用途 |
|------|------|------|
| [`days_to_ymd`](#days_to_ymd) | `(days: u64) -> (u64, u64, u64)` | Unix 天数（自 1970-01-01）→ `(年, 月, 日)` |
| [`is_leap`](#is_leap) | `(year: u64) -> bool` | 闰年判定 |
| [`now_timestamp14`](#now_timestamp14) | `() -> String` | 当前 14 位时间戳 `yyyymmddhhmmss` |

### `days_to_ymd`

```rust
pub fn days_to_ymd(mut days: u64) -> (u64, u64, u64)
```

逐年代扣 `365`/`366` 定位年份，再逐月扣月天数定位月份，剩余天数 +1 即日。返回公历 `(年, 月, 日)`，月/日均为 **1-based**。`now_timestamp14` 与 `license::time::day_to_ymd_string` 均调用它。

### `is_leap`

```rust
pub fn is_leap(year: u64) -> bool
```

标准格里高利历闰年规则：能被 4 整除且（不被 100 整除或被 400 整除）。

### `now_timestamp14`

```rust
pub fn now_timestamp14() -> String
```

返回当前时间的 14 位连续数字串 `yyyymmddhhmmss`（年 4 位 + 月 2 + 日 2 + 时 2 + 分 2 + 秒 2）。实现复用 `days_to_ymd` 计算日期部分，再由日内剩余秒数（`secs % 86400`）推导时分秒：

```rust
let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
let (y, m, d) = days_to_ymd(secs / 86400);
let day_secs = secs % 86400;
let h = day_secs / 3600; let min = (day_secs % 3600) / 60; let s = day_secs % 60;
format!("{:04}{:02}{:02}{:02}{:02}{:02}", y, m, d, h, min, s)
```

> **时区口径**：与 `license` / `viewer.rs` 一致，均基于 **UTC** epoch 换算（项目刻意不引入本地时区换算以保持零额外依赖）。调用方（如备份命名）对此不敏感。

## 3. 核心逻辑与数据流

```
SystemTime::now()
   ▼ duration_since(UNIX_EPOCH)
epoch 总秒数 secs
   ├─ secs / 86400 ──► days_to_ymd ──► (年, 月, 日)
   └─ secs % 86400 ──► 时 / 分 / 秒（整除与取余）
   ▼ format!
"yyyymmddhhmmss"（14 位字符串）
```

## 4. 依赖关系

- **对外依赖**：仅 `std::time`（`SystemTime` / `UNIX_EPOCH`）。
- **被依赖**：
  - `license::time::day_to_ymd_string` —— 调 `days_to_ymd` 渲染到期日 UI。
  - [`util::backup`](./backup.md) —— 调 `now_timestamp14` 拼备份文件名。
  - `viewer.rs`（`generate_save_path`）—— 自行内联了等价的日期换算（历史遗留，可后续收敛到本模块）。
