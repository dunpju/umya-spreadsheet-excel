# `excel/fill.rs` 文档

## 1. 文件概述

`src/excel/fill.rs` 实现**单元格填充柄（Fill Handle）的填充逻辑**——纯数据层，不涉及 UI。给定源选区与拖拽目标格，按源数据类型推断序列并写入目标单元格，返回被覆盖目标格的原始数据（供撤销）。

由 `draw_table_content`（[`gui/widgets/table`](../gui/widgets/table.md) §2.13）在填充柄拖拽释放时调用。

### 职责

- **序列推断与填充**：公式（相对引用平移）、日期（按天递增）、数字（算术/等比）、中文日期文本（`08月24号` 等按天递增）、纯文本（复制）。
- **格式复制**：目标格先克隆对应源格（保留字体/底色/边框/对齐/批注/数字格式），再覆写 `value`/`formula`/`raw_number`——与 Excel 填充一致。
- **撤销快照**：返回目标格写入前的原始数据，供上层构造 `UndoAction::RangeClear`。
- **合并单元格感知**：源序列与目标序列都按合并区域折叠——合并体内的非左上角格被跳过（`is_merged_part`），使一个合并单元格在序列中只占一个元素、目标也只在合并左上角写入。避免合并体空格被当作 0 污染步长推断（曾导致 AJ1:AK1 合并值 18 水平填充得到 `-18` 而非 19，已修复；见 §2「合并单元格感知」）。

### 依赖

| 类别 | 依赖 | 用途 |
|------|------|------|
| 内部模块 | `crate::excel::formula::shift_formula_relative` | 公式相对引用平移（复制语义，绝对 `$` 不变） |
| 内部模块 | `crate::excel::reader::{CellData, ExcelData, SheetData}` | 数据模型、日期解析/格式化、`is_date_format` |
| 标准库 | `std` | 基本类型 |

---

## 2. 公开 API

### `apply_fill`

```rust
pub fn apply_fill(
    sheet: &mut SheetData,
    src: (u32, u32, u32, u32),   // (start_col, start_row, end_col, end_row)，自动归一化
    target: (u32, u32),          // (col, row) 拖拽结束格
) -> (Vec<(u32, u32, Option<CellData>)>, bool)
//   ^被覆盖目标格原始数据 (row,col,旧值)     ^是否含公式填充（提示重算粒度）
```

#### 轴向与方向

由 `target` 相对源选区位置判定（优先级：行 > 列）：

| target 位置 | 轴 | 方向 | 目标格集合 |
|-------------|----|----|-----------|
| `row > src.end_row` | 垂直 | 前（下） | 各列 `end_row+1 ..= target.row` |
| `row < src.start_row` | 垂直 | 后（上） | 各列 `target.row .. start_row-1` |
| `col > src.end_col` | 水平 | 前（右） | 各行 `end_col+1 ..= target.col` |
| `col < src.start_col` | 水平 | 后（左） | 各行 `target.col .. start_col-1` |
| 落在源内 | — | — | 无操作（返回空） |

多列/多行源选区按**车道**独立填充：垂直填充按列、水平填充按行，每条车道各自取源序列扩展。

#### 类型推断与填充规则

按车道首个非空源格判定类型：

| 类型 | 判定 | 填充规则 |
|------|------|---------|
| **公式** | `!formula.is_empty()` | 目标公式 = 源公式经 `shift_formula_relative(src, col_off, row_off)`；`value` 清空（由重算回填）；标记 `has_formula` |
| **日期** | `number_format` 经 `is_date_format` 为真 | 序列号 `= base ± (k+1)·d`，`d` 为源序列差（≥2 格）否则 1 天；`value = format_date(serial, fmt)`、`raw_number = Some(serial)` |
| **数字** | `value`/`raw_number` 可解析为 `f64` | **优先等差**：源序列恒定差时 `base ± (k+1)·d`；仅当非恒定差但恒定比值（且无 0、比值≠1）才等比 `base · r^(k+1)`；单元素默认步长 1。`format_num` 清理浮点噪声 |
| **日期文本** | `value` 匹配 `[YYYY年]?M月D(日\|号)`（如 `08月24号`、`8月24日`、`2024年8月24日`） | 按天递增 `base ± (k+1)·d`（`d` 由源序列推断，默认 1 天）；序列号经 `date_to_serial`/`serial_to_date` 计算，月末跨月/闰年由序列号自动处理；无年份时取当前年份（`current_year()`）；输出经 `format_date_text` 按**源格原格式**（年/前导零/`日`或`号`后缀）回填 |
| **文本** | 其它 | 复制源 pattern 格 `value`（`k % n` 取模重复） |

> **日期文本识别**：纯文本日期（无 `number_format`、非数字，如 `08月24号`）不会被 `is_date_format`/`parse_date_string` 识别，故用独立的 `parse_date_text` 模式匹配（要求末尾为 `日`/`号`，两位年份 `<100` 视为歧义不予识别），匹配后按日期序列递增——与 Excel 拖拽此类文本的行为一致。

- `base`：前向（下/右）取源序列末元素；后向（上/左）取首元素。
- `d`/`r` 由 `detect_step` / `detect_number_pattern` 从源序列推断（容差 `1e-9`）。
- 公式偏移 `col_off/row_off` = 目标格相对其对应源 pattern 格的坐标差。

#### 合并单元格感知（merge-aware）

车道内的源序列与目标序列在参与推断/写入前，都用 `is_merged_part(sheet, col, row)` 过滤掉**合并区域的非左上角格**（合并值只存于左上角，其余部分无独立数据）：

```rust
fn is_merged_part(sheet: &SheetData, col: u32, row: u32) -> bool {
    sheet.get_merged_range(col, row).map_or(false, |mr| !mr.is_top_left(col, row))
}
// src_pos / target_pos 构建时：filter(|&(c, r)| !is_merged_part(sheet, c, r))
```

- **源侧**：一个合并单元格在源序列中只占**一个**元素（其左上角）。否则合并体内的空格会被 `cell_number(...).unwrap_or(0.0)` 当作 0，污染步长推断。
- **目标侧**：只在合并区域**左上角**写入，不向合并体内塞隐藏值。

> **修复的 Bug（报修用例）**：`AJ1:AK1` 合并值为 18，水平向右填充一格到 `AL1`（`AL1:AM1` 合并）。
> 修复前：源按物理列遍历得 `[AJ1=18, AK1=∅→0]`，`detect_number_pattern` 算出步长 `d0 = 0-18 = -18`、`base = vals[1] = 0` → `0 + 1·(-18) = -18`（错误地变成 -18）。
> 修复后：源折叠为 `[AJ1=18]`（`n=1`），单元素默认步长 1、`base = 18` → `18 + 1·1 = 19`（与 Excel 一致）。非合并场景因 `get_merged_range` 返回 `None`，过滤为空操作，行为不变。

#### 边界

- 目标越界（> `max_row`/`max_col`）由车道范围自然裁剪。
- 空源（`value` 与 `formula` 均空）→ 目标格克隆为空（清空）。
- `format_num`：`(v·1e10).round()/1e10` 后 `Display`，消除 `0.1+0.2` 类累加噪声。

---

## 3. 关联：`formula::shift_formula_relative`

`src/excel/formula.rs` 中新增的**复制语义**公式引用平移函数（区别于既有 `adjust_formula_columns/rows` 的"插入语义"——绝对引用随结构移动）：

```rust
pub fn shift_formula_relative(formula: &str, col_shift: i32, row_shift: i32) -> String
```

- 仅平移**相对**引用（无 `$` 前缀）；绝对（`$A` / `$1`）保持不变。
- 跳过字符串字面量；处理前导 `=`/`@`。
- 列字母经 `letter_to_col` ↔ `col_to_letter` 转换。

示例：`=$A$1+B1` 下移 1 行 → `=$A$1+B2`；`=A1+B1` 右移 1 列 → `=B1+C1`。

---

## 4. 测试

`#[cfg(test)] mod tests` 覆盖：单数字递增、等差序列（`1,2,3→4,5,6`）、等差步长 2（`2,4→6,8`）、等比（`2,4,8→16,32`）、水平填充、反向（向上）、文本复制、公式相对平移、`$` 绝对不变、返回 `old_cells` 用于撤销、中文日期文本递增（`08月24号→08月25号`、月末跨月、含/不含年份）、纯文本不被误判为日期、**合并单元格水平填充不被空合并体污染（报修用例：合并值 18 → 19 而非 -18）**、**目标为合并格时只写左上角**。

运行：`cargo test --bin my-excel fill::`。
