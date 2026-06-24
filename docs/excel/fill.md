# `excel/fill.rs` 文档

## 1. 文件概述

`src/excel/fill.rs` 实现**单元格填充柄（Fill Handle）的填充逻辑**——纯数据层，不涉及 UI。给定源选区与拖拽目标格，按源数据类型推断序列并写入目标单元格，返回被覆盖目标格的原始数据（供撤销）。

由 `draw_table_content`（[`gui/widgets/table`](../gui/widgets/table.md) §2.12）在填充柄**拖拽释放**时调用（`apply_fill`）；双击填充柄时则先调 `compute_autofill_target` 算出目标格，再复用 `apply_fill` 写入。

### 职责

- **序列推断与填充**：公式（相对引用平移）、日期（按天递增）、数字（算术/等比）、中文日期文本（`08月24号` 等按天递增）、纯文本（复制）。
- **格式复制**：目标格先克隆对应源格（保留字体/底色/边框/对齐/批注/数字格式），再覆写 `value`/`formula`/`raw_number`——与 Excel 填充一致。
- **撤销快照**：返回目标格写入前的原始数据，供上层构造 `UndoAction::RangeClear`。
- **合并单元格感知**：源序列与目标序列都按合并区域折叠——合并体内的非左上角格被跳过（`is_merged_part`），使一个合并单元格在序列中只占一个元素、目标也只在合并左上角写入。避免合并体空格被当作 0 污染步长推断（曾导致 AJ1:AK1 合并值 18 水平填充得到 `-18` 而非 19，已修复；见 §2「合并单元格感知」）。
- **双击填充柄自动填充（目标推断）**：[`compute_autofill_target`] 根据源选区**朝向**与**相邻连续数据**的边界算出自动填充目标格——横向线向右填到该行相邻数据末列、纵向线/单格向下填到该列相邻数据末行（与 Excel 双击填充柄一致）。边界扫描合并感知（合并区域折叠为左上角值、跨过整个合并跨度）、隐藏行/列透明；含安全上限 [`AUTO_FILL_MAX_CELLS`] 防止单帧海量写入阻塞 UI。
- **分批跨帧填充（预计算）**：[`compute_fill_values`] 只读预计算全部填充值（不写入 sheet），供调用方分批写入实现跨帧填充，避免大范围填充阻塞 UI。

### 依赖

| 类别 | 依赖 | 用途 |
|------|------|------|
| 内部模块 | `crate::excel::formula::shift_formula_relative` | 公式相对引用平移（复制语义，绝对 `$` 不变） |
| 内部模块 | `crate::excel::formula::invalidate_formula_graph` | 含公式填充时使公式依赖图 L2 缓存失效 |
| 内部模块 | `crate::excel::reader::{CellData, ExcelData, SheetData}` | 数据模型、日期解析/格式化、`is_date_format`、`get_cell`/`get_merged_range` |
| 标准库 | `std` | 基本类型 |
| 标准库 | `std::collections::HashSet` | 隐藏行/列集合（双击自动填充边界扫描时透明跳过） |

---

## 2. 公开 API

### `apply_fill`

```rust
pub fn apply_fill(
    sheet: &mut SheetData,
    src: (u32, u32, u32, u32),   // (start_col, start_row, end_col, end_row)，自动归一化
    target: (u32, u32),          // (col, row) 拖拽结束格
) -> (Vec<(u32, u32, Option<CellData>)>, bool)
//   ^被覆盖目标格原始数据 (row,col,旧值)     ^是否含公式填充（提示重算粒度：公式→evaluate_sheet 全量；仅值→evaluate_dependents_many 批量增量）
```

含公式填充时调用 `crate::excel::formula::invalidate_formula_graph(sheet)` 使公式依赖图 L2 缓存失效（下次 `build_formula_graph` 重建）。

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

### `compute_fill_values`（只读预计算填充值）

```rust
pub fn compute_fill_values(
    sheet: &SheetData,
    src: (u32, u32, u32, u32),   // (start_col, start_row, end_col, end_row)，自动归一化
    target: (u32, u32),          // (col, row) 拖拽结束格
) -> Option<FillValues>
//   ^None 表示 target 落在源内（无填充操作）
```

只读预计算填充值，不写入 sheet。逻辑与 `apply_fill` 一致（车道推断/Kind 检测/步长计算/合并感知），区别是把 `sheet.cells.insert` 替换为收集到 `Vec`。返回 `None` 表示 target 落在源内。

供分批跨帧填充使用：调用方先用 `compute_fill_values` 获取全部待写入值，再按批次写入 sheet（每帧写 `FILL_BATCH_SIZE` 格），避免大范围填充阻塞 UI（详见 §5「分批跨帧填充」）。

```rust
/// 预计算的所有填充目标格值（只读，无副作用）。
#[derive(Clone)]
pub struct FillValues {
    /// 待写入的目标格列表 `(row, col, new_cell_data)`。
    pub cells: Vec<(u32, u32, CellData)>,
    /// 目标中是否含公式填充（决定重算策略：公式→`evaluate_sheet`，仅值→`evaluate_dependents_many`）。
    pub has_formula: bool,
}
```

### 分批跨帧填充常量

```rust
/// 分批跨帧填充每帧写入上限（格数）。
/// 2000 格 × ~1.5μs/格 ≈ 3ms/帧，远低于 16ms 帧预算，UI 保持流畅。
pub const FILL_BATCH_SIZE: usize = 2000;

/// 低于此目标格数的填充走同步路径（单帧完成）；超过则启用分批跨帧模式。
pub const FILL_SYNC_THRESHOLD: usize = 2000;
```

### `compute_autofill_target`（双击填充柄自动填充目标推断）

```rust
pub const AUTO_FILL_MAX_CELLS: u32 = 50_000;

pub fn compute_autofill_target(
    sheet: &SheetData,
    src: (u32, u32, u32, u32),        // (start_col, start_row, end_col, end_row)，自动归一化
    hidden_cols: &HashSet<u32>,
    hidden_rows: &HashSet<u32>,
) -> Option<(u32, u32)>               // Some((target_col, target_row)) 直接传给 apply_fill；None=无相邻数据、不填充
```

双击填充柄时，由 `draw_table_content` 调用，**只算"填到哪一格"**——写入仍走 [`apply_fill`](#apply_fill)，故序列推断/合并感知/步长推断全部复用，无重复逻辑。

#### 方向推断（按源选区朝向）

| 源选区形状 | 首选方向 | 说明 |
|-----------|---------|------|
| 横向线（`end_col>start_col` 且单行） | **仅向右** | 如 `AH1:AK1`（含 `AH1:AI1`/`AJ1:AK1` 合并） |
| 纵向线（`end_row>start_row` 且单列） | **仅向下** | 如 `A38:A39` |
| 单格 / 方块 | 向下（默认），允许回退向右 | 与 Excel 双击填充柄默认一致 |

方向明确的选区（横向线/纵向线）**不回退另一方向**，避免横向选区误触纵向填充（与 Excel 一致）。单格/方块首选向下，无相邻数据时回退向右；两者都无 → 返回 `None`（不填充，避免误操作）。

#### 边界判定（仿 Excel「双击填充柄填充到相邻连续数据末尾」）

- **向下**：从「源末行下一行 `end_row+1`」起，在候选列（**源列优先** `start_col..=end_col`，再**向左扫描 ≤10 列** `start_col-1` 至 `max(1, start_col-10)`，再**向右扫描 ≤10 列** `end_col+1` 至 `end_col+10`）中，取首个有数据的列作锚点；从该处起 [`scan_down`] 向下扫**连续非空格**，末行即目标行。紧邻位（`end_row+1`）即空 → 该方向无边界。
- **向右**：从「源末列右一列 `end_col+1`」起，在候选行（**源行优先** `start_row..=end_row`，再**向上扫描 ≤10 行** `start_row-1` 至 `max(1, start_row-10)`，再**向下扫描 ≤10 行** `end_row+1` 至 `end_row+10`）中，取首个有数据的行作锚点；从该处起 [`scan_right`] 向右扫连续非空格，末列即目标列。扩大搜索范围避免因紧邻行/列为空而错过稍远行的相邻数据。

> 用例对齐：横向 `AH1:AI1=17`/`AJ1:AK1=18`，相邻第 2 列数据延伸到 `AN` 列 → 向右填到 `AN1`（19,20,21）；纵向 `A38="08月17号"`/`A39="08月18号"`，那么 A 列数据延伸到第 44 行 → 向下填到 `A44`（`08月19号`…`08月23号`）。

#### 合并感知 / 隐藏行列透明

- `cell_occupied(col, row)`：合并感知的非空判定——若位置属于某合并区域，以其**左上角**值/公式为准（合并数据只存于左上角）；"有效"= 有单元格且 `value`/`formula` 非空。
- `scan_down`/`scan_right`：遇到合并区域时，若左上角有数据则**整个跨度**（行/列方向 `start..=end`）视为连续占据并跳到 `end+1`，否则视为空隙终止；遇到**隐藏行/列**则透明跳过（不断连续性、不计入占据）。
- 扫描受 `sheet.max_row`/`max_col` 约束，防无限扫描。

#### 安全上限

总填充格数 = **车道**（垂直=源列数 / 水平=源行数）× **沿轴长度**。若超过 [`AUTO_FILL_MAX_CELLS`]（默认 50 000），把沿轴长度夹紧到 `MAX / 车道`，避免异常超大表（如相邻列数十万行连续数据）单帧海量写入阻塞 UI。现实数据量下边界由相邻数据天然限定，远小于此值。

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

`#[cfg(test)] mod tests` 覆盖：单数字递增、等差序列（`1,2,3→4,5,6`）、等差步长 2（`2,4→6,8`）、等比（`2,4,8→16,32`）、水平填充、反向（向上）、文本复制、公式相对平移、`$` 绝对不变、返回 `old_cells` 用于撤销、中文日期文本递增（`08月24号→08月25号`、月末跨月、含/不含年份）、纯文本不被误判为日期、**合并单元格水平填充不被空合并体污染（报修用例：合并值 18 → 19 而非 -18）**、**目标为合并格时只写左上角**；以及双击自动填充目标推断：**横向合并源填到相邻边界**（`A1:D1` 合并 17/18 → `E1:G1`=19/20/21）、**纵向中文日期文本填到边界**（`A1:A2` → `A6`）、**相邻数据含合并**（横向/纵向）、**无相邻数据→None**、**上限夹紧**（6 万行连续数据夹到 5 万）、**隐藏行透明**、**横向线不回退纵向**（右侧无数据→None）、**纵向线不回退横向**（下方无数据→None）、**横向扩大锚点搜索**（row1/2 空、row3 有数据→正确横向填充）、**纵向扩大锚点搜索**（col1/2 空、col3 有数据→正确纵向填充）；以及 `compute_fill_values`：**数字垂直预计算**、**预计算结果与 apply_fill 一致**、**水平预计算**、**日期文本预计算**、**合并场景预计算不被污染**、**公式预计算**、**target 在源内返回 None**。

运行：`cargo test excel::fill`（或 `cargo test --bin my-excel fill::`）。

---

## 5. 性能

双击自动填充的写入复用 `apply_fill`，本身已是高效路径，现实数据量下**不会阻塞 UI**：

| 关注点 | 现状（已优化） | 说明 |
|--------|--------------|------|
| 填充范围 | 由相邻连续数据天然限定 | 边界即相邻数据末尾，几乎不会是全表；`AUTO_FILL_MAX_CELLS` 兜底防异常超大表 |
| 写入开销 | `apply_fill` 为 `O(K)` HashMap 插入 | K=目标格数；每格仅一次 `cells.insert` + 一次 `get_cell` 克隆源格 |
| 重算开销 | `evaluate_dependents_many` **批量增量** | 一次建依赖图重算受影响公式，替代逐格 `evaluate_dependents`（后者大表上 K × O(2M) 卡顿）；含公式才走全量 `evaluate_sheet` |
| 渲染开销 | 虚拟渲染（仅视口内可见格） | 即便填了数万格，每帧也只绘制视口内 ~数十格，填充量不影响帧绘制 |

### 分批跨帧填充（已实现）

当目标格数超过 `FILL_SYNC_THRESHOLD`（2000）时，启用分批跨帧填充模式，避免单帧海量写入阻塞 UI。小填充（≤2000 格）走同步路径。

**实现机制**：

- **同步路径**（`table.rs`）：目标格数 ≤ `FILL_SYNC_THRESHOLD` 时，直接在 `draw_table_content` 中调用 `apply_fill` 同步写入，单帧完成。
- **分批跨帧路径**（`viewer.rs`）：目标格数 > `FILL_SYNC_THRESHOLD` 时：
  1. 调用 `compute_fill_values` 只读预计算全部填充值（`FillValues`），得到 `cells: Vec<(u32, u32, CellData)>` 和 `has_formula`。
  2. 在 `viewer.rs` 中创建 `PendingFill` 状态，保存 `FillValues`、当前写入偏移量、累积的 `old_cells`（撤销快照）。
  3. 每帧写入 `FILL_BATCH_SIZE`（2000）格到 `sheet.cells`，写完后调用 `ctx.request_repaint()` 驱动下一帧继续。
  4. 所有格写入完毕后：统一触发公式重算（`evaluate_sheet` 或 `evaluate_dependents_many`，取决于 `has_formula`）、选区更新、入撤销栈。

**性能预算**：2000 格 × ~1.5μs/格 ≈ 3ms/帧，远低于 16ms 帧预算，UI 保持流畅。

### 进阶优化方案（按需启用，未来选项）

1. **多线程批量写入（rayon）**：序列推断为纯函数，可把不同车道（列/行）并行写入各自的 `HashMap` 局部副本，最后合并。注意 `SheetData` 非线程友好（含 `RefCell` 式懒重建？实际为 owned 字段，可 `par_iter_mut`），且 egui 主循环外需 `Arc<Mutex>` 回传结果。适用于纯值填充（无公式即时重算依赖）。
2. **虚拟滚动惰性填充**：只填充当前可视区 + 缓冲区，滚动到未填充区域时按需补填（惰性求值）。实现复杂（需脏标记 + 滚动钩子），且导出/重算时需保证全量已填，一般不必要。

> 取舍：分批跨帧填充已实现且改动最小、收益最大、与现有撤销/选区模型兼容最好；`AUTO_FILL_MAX_CELLS` 上限使最坏批次也可控。当前实现已用上限兜底 + 分批跨帧填充，**多数场景无需启用上述进阶方案**。
