# `util/mod.rs` 文档

## 1. 文件概述

`src/util/mod.rs` 是 **umya-spreadsheet-excel** 项目中 `util`（通用工具）子模块的**入口与组织文件**（module root）。

它的职责单一：声明并公开（`pub mod`）该目录下的子模块，使上层（`viewer.rs`、`license/*`、`main.rs` 等）能够通过 `crate::util::xxx` 路径访问通用工具能力。该文件本身不含业务逻辑、类型定义或函数实现，仅作为模块树的"目录节点"与统一门面（facade）存在。

它把跨模块复用的纯工具能力（日期换算、文件备份）抽离为关注点独立的子模块，体现**关注点分离**与**去重复实现**的设计目标——例如日期换算原先散落在 `main.rs`、`license` 模块与（已移除的）`viewer.rs::generate_save_path`，现统一收敛到 `util::date`。

| 子模块 | 职责定位 | 主要消费者 |
|--------|----------|------------|
| `date` | Unix epoch → 年月日/时间戳换算（无 chrono 依赖） | `license::time`、`util::backup`、`viewer.rs` |
| `backup` | 导入文件快照备份（复制到 `~/.MyExcel/backup/`） | `viewer.rs`（`start_async_load`） |

## 2. 代码逻辑分析

文件仅含两行有效声明：

```rust
//! 通用工具模块

pub mod backup;
pub mod date;
```

### 模块声明

- **`pub mod date;`** —— 公开日期工具模块。提供基于 Unix epoch 的纯算术换算：`days_to_ymd`（天数 → 年月日）、`is_leap`（闰年判定）、`now_timestamp14`（当前 14 位时间戳 `yyyymmddhhmmss`）。全程不依赖 `chrono`，与项目"最小依赖"原则一致。
- **`pub mod backup;`** —— 公开导入备份模块。提供 `backup_imported_file`：把用户经「文件 → 导入」选择的文件复制一份到 `~/.MyExcel/backup/`，命名 `原文件名_yyyymmddhhmmss.ext`，目录不存在则递归创建。

### 依赖关系说明

- `date` 是**底层无依赖**模块（仅用 `std::time`），被 `license::time`（`day_to_ymd_string`）、`backup`（`now_timestamp14`）及 `viewer.rs` 复用。
- `backup` 依赖 `date`（取时间戳）与外部 crate `dirs`（定位主目录）。

## 3. 关键类型与函数清单

`mod.rs` 本身**不定义任何类型或函数**，仅做模块声明。

> 模块导出的所有函数请参阅对应子模块文档：
> - [`date.md`](./date.md)
> - [`backup.md`](./backup.md)
