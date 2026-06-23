# 搜索组件业务实现分析（`src/gui/widgets/search.rs`）

> 本文档基于 `search.rs`（约 2076 行）源码梳理，系统阐述 GUI 搜索窗口的模块定位、数据结构、
> 搜索/筛选流程、隐藏集合填充机制、关键调用链，状态与 UI 联动逻辑，以及视觉布局。

---

## 1. 模块概览

`search.rs` 是 Excel 查看器 GUI 层的**搜索/筛选窗口组件**，隶属于 `gui::widgets` 模块。它负责：

- **读取配置驱动搜索范围**：从用户主目录下 `~/.MyExcel/my-excel.yaml` 的 `search.column` 与 `search.row` 两个键，解析出若干单元格引用（支持单格、范围、混合格式），将它们的值作为"可选列"与"行筛选配置项"。
- **两类独立的筛选维度**：
  - **列筛选（Column Filter）**：以某个表头单元格为基准，在其所在行的**右侧所有列**中模糊匹配关键字，**隐藏不匹配的列**。
  - **行筛选（Row Filter）**：以某个表头单元格为基准，在其所在列的**下方所有行**中匹配关键字，**隐藏不匹配的行**。
- **多条件 AND/OR 组合**：两类筛选都支持动态增删多条条件，遵循 **MySQL 风格的运算符优先级——AND 优先级高于 OR**。
- **性能优化**：自适应地在「二分查找 / 多线程并行扫描 / 串行线性扫描」三种路径间选择，并对已排序、跨行/跨列合并单元格做了专门处理。
- **非模态窗口**：`draw_search_window` 绘制一个可折叠、可关闭、独立于主窗口操作的浮层。

依赖关系上，它只向上暴露：
- 结构体：`SearchColumnOption`、`RowFilterState`、`ColumnFilter`、`FilterLogic`、`SearchWindowState`；
- 公开函数：`load_column_options`、`load_row_filter_configs`、`execute_multi_column_search`、`execute_row_search`、`draw_search_window`，以及配置解析工具 `parse_search_range`。

> 注：`execute_search`、`search_sorted`、`execute_column_filter` 等被标注 `#[allow(dead_code)]`，是**早期单条件实现/历史保留**，当前 UI 统一走 `execute_multi_column_search` + `execute_row_search`。

---

## 2. 核心数据结构

### 2.1 配置侧数据来源（来自 `crate::excel::reader`）

| 类型 | 关键字段 | 用途 |
|------|----------|------|
| `CellData` | `value: String`、`raw_number`、`formula`、`number_format: Option<String>`、`bold`、`background_color` … | 单元格显示值与格式。`number_format` 决定是否按日期格式化。 |
| `SheetData` | `cells: HashMap<(row,col), CellData>`、`merged_cells: Vec<CellRange>`、`max_row: u32`、`max_col: u32` | 工作表数据。`get_cell(row, col)` 会处理合并单元格的左上角回退查找。 |
| `ExcelData` | `get_sheet(idx)`、`is_date_format(fmt)`、`format_date(serial, fmt)` | 多 sheet 容器与日期格式化能力。 |
| `CellRange`（合并单元格） | `start_row/end_row/start_col/end_col` | 合并区域坐标，用于跨列/跨行可见性对齐。 |

### 2.2 配置项结构

**`SearchColumnOption`** —— 列下拉框的一个选项（`search.column` 解析结果）：
- `title: String`：显示文本（单元格的值，如 "序号"）。
- `cell_ref: String`：坐标字符串，如 `"A1"`。
- `col: u32` / `row: u32`：1-based 列号、行号。

**`RowFilterState`** —— 行筛选项（`search.row` 解析结果，每个单元格一项）：
- `title` / `cell_ref` / `col` / `row`：同上；**搜索从此行 `+1` 开始向下**。
- `keyword: String`：用户输入的关键字。
- `logic: FilterLogic`：**与下一条筛选项**的组合逻辑（最后一条无意义）。
- `op: CompareOp`：比较运算符（默认 `包含`），决定值与关键字的匹配方式。
- `is_active()`：关键字 trim 后非空即激活。

**`ColumnFilter`** —— 动态列筛选条件：
- `column_index: usize`：选中项在 `column_options` 中的下标。
- `filter_value: String`：筛选关键字。
- `logic: FilterLogic`：与下一条条件的组合逻辑。
- `op: CompareOp`：比较运算符（默认 `包含`）。
- `is_active()`：`filter_value` 非空即激活。

**`FilterLogic`** 枚举：
```rust
pub enum FilterLogic { And, Or }
```
语义统一约定：**字段 `logic` 表示"本条与下一条"的连接符**，最后一条的 `logic` 不参与运算。

**`CompareOp`** 枚举 —— 比较运算符（列筛选 / 行筛选共用）：
```rust
pub enum CompareOp {
    Contains,    // 包含（子串匹配，默认）
    NotContains, // 不包含（排除匹配）
    Equal,       // =（忽略大小写精确相等）
    NotEqual,    // !=（不等）
    GreaterThan, // >（数值，失败降级为字典序）
    LessThan,    // <
    GreaterEqual,// >=
    LessEqual,   // <=
}
```
- 提供 `Default::default() → Contains`、`label()` 返回下拉文案（`包含/不包含/=/!=/>/</>=/<=`）、常量数组 `ALL`（下拉渲染顺序）。
- **字符串类**（`Contains`/`NotContains`）为大小写不敏感子串；**精确类**（`Equal`/`NotEqual`）忽略大小写比较；**数值类**（`>`/`<`/`>=`/`<=`）优先按 `f64` 比较，任一端不可解析为数值时**降级为字符串字典序比较**。
- 下拉框宽度固定 `60.0`，插入位置：列筛选行在「列下拉框」与「输入框」之间，行筛选项在「标题标签」与「输入框」之间。

### 2.3 状态聚合 `SearchWindowState`

`SearchWindowState` 是搜索窗口的完整可变状态，分四组：

| 分组 | 字段 | 作用 |
|------|------|------|
| 窗口控制 | `visible`、`collapsed` | 窗口可见性 / 折叠状态（默认展开） |
| 下拉框数据 | `column_options`、`selected_index`、`options_loaded` | 可选列、当前选中索引、是否已加载（避免每帧重解析） |
| 搜索输入/状态 | `search_keyword`、`is_searching`、`matched_count`、`total_searched`、`use_binary_search`、`debug_info` | 列搜索关键字、是否生效、匹配/总数、是否走了二分、诊断文本 |
| 行筛选 | `row_filters`、`is_row_searching`、`row_matched_count`、`row_total_searched`、`row_debug_info` | 行筛选配置与结果 |
| 多条件列筛选 | `column_filters`、`filter_logic`（`#[allow(dead_code)]` 兼容保留） | 动态列条件列表 |

`Default` 初始化：`visible=false`、`collapsed=false`，并预置一条空的 `ColumnFilter{column_index:0, filter_value:"", logic:And, op:Contains}`。

---

## 3. 搜索功能流程

### 3.1 配置与范围解析

**`parse_cell_ref(s) -> Option<(col,row)>`**
将 `"A1"` 转为 `(1,1)`：取前导字母段做 26 进制累加（`A=1…Z=26, AA=27`），取后续数字段解析行号；`col==0||row==0` 视为非法。

**`parse_one_segment(s) -> Vec<(col,row)>`**
解析**单段**：
- 若含 `-` 且**其后的首字符是字母** → 判定为范围：
  - 同列不同行（`A1-A13`）→ 展开为同列的行区间；
  - 同行不同列（`A1-C1`）→ 展开为同行的列区间；
  - 自动按 `lo..=hi` 归一化方向。
- 否则 → 按单格 `parse_cell_ref` 返回。

> "其后首字符是字母"这一判定，确保像 `A1-A13`（范围）与纯字母坐标的语义区分。

**`parse_search_range(input) -> Vec<(col,row)>`**
**总入口**：含逗号则逐段 `parse_one_segment` 后扁平化；否则整段解析。支持 `"A1-A13"`、`"A1,A3,B5"`、`"A1-A13,A15"` 等混合格式。

`load_column_options` / `load_row_filter_configs` 复用该入口：读 `search.column` / `search.row` → 解析单元格 → 用 `cell_search_value` 取显示值组装选项。配置文件不存在或键为空则返回空 `Vec`。

**`cell_search_value(cell)` —— 搜索用显示值**
与表格渲染保持一致：若 `number_format` 是日期格式且 `value` 可解析为 `f64` 序列号，返回 `format_date` 结果（如 `"2028/7/14"`）；否则返回 `value` 原值。这保证搜索体验与用户所见一致。

### 3.2 关键字解析（行筛选）`parse_row_keywords`

行筛选每条输入支持三种格式，返回 `(Vec<String>, bool is_range)`：
- **范围**：`'xxx3'-'xxx4'` → `(["xxx3","xxx4"], true)`。判定：找到首个 `'`，若其后存在 `-`，且 `-` 前段以 `'` 结尾、后段以 `'` 开头，两端 trim 去引号非空。
- **多值**：含 `,` 时按逗号切分、去引号、去空 → `([...], false)`。
- **单值**：其余 → `([input], false)`。

> 注意：**列筛选**（`ColumnFilter.filter_value`）只做单一模糊匹配，不经过此解析器；**行筛选**才支持多值/范围语义。

### 3.3 列搜索流程（多条件）

入口 `execute_multi_column_search(state, sheet, hidden_columns)`：

1. **收集激活条件** `active = column_filters.filter(is_active)`，为空则直接返回。
2. **向后兼容**：把第一条条件的 `column_index` / `filter_value` 同步到 `state.selected_index` / `state.search_keyword`。
3. **逐条计算匹配列集合**（调用 `compute_column_matches`，传入 `f.op`，见 §5），同时累加 `max_total` 与 `any_binary`。`compute_column_matches` 内部仅当「数据已排序 **且** `op==Contains`」时启用二分查找；其他运算符走线性 `compare_value` 比较。
4. **AND 优先于 OR 的分组组合**：维护 `current_and_group: Option<HashSet<u32>>` 与 `or_groups: Vec<HashSet<u32>>`：
   - 条件 `i` 的**前驱运算符** = `active[i-1].logic`（首条无前驱，按 AND 组起始）；
   - 前驱为 **AND** → 与当前组取**交集**（`intersection`，缩小范围）；
   - 前驱为 **OR** → 关闭当前 AND 组推入 `or_groups`，以本条为新组起点。
   - 循环结束把最后一个 AND 组也推入。
5. **组间并集**：`visible = or_groups` 的 `extend` 合并。
6. **写回隐藏集合**：`hidden_columns.clear()`，对 `1..=max_col` 中不在 `visible` 的列 `insert`；再确保所有激活条件的目标列自身不被隐藏（`hidden_columns.remove(opt.col)`）。
7. **更新状态与诊断**：`total_searched/max_total`、`matched_count/visible.len()`、`use_binary_search/any_binary`、`is_searching=true`，并拼装 `debug_info`。

> 示例：`C1 OR C2 AND C3 AND C4 OR C5` → 分组 `[C1] [C2∩C3∩C4] [C5]` → 结果 `C1 ∪ (C2∩C3∩C4) ∪ C5`。

单条件旧实现 `execute_search`/`search_sorted`（`#[allow(dead_code)]`）逻辑同构，区别在于直接写入 `hidden_columns` 且合并了对齐走 `expand_hidden_for_merged_cells`。

### 3.4 行筛选流程（多条件 + 自适应路径）

入口 `execute_row_search(state, sheet, hidden_rows)`：

1. `hidden_rows.clear()`；空配置 / 无激活项则写诊断后返回。
2. 记 `start_row = active_filters[0].row`、`max_row`；`row_total_searched = max_row - start_row`；无数据行则返回。
3. **解析所有筛选器关键字**为 owned 的 `ParsedFilter{ col, keywords, is_range, op }`（线程安全）。
4. **运算符序列** `logic_seq`（即各 `active[i].logic`），并预先求 `has_or = logic_seq[..len-1]` 中是否存在 `Or`（最后一条 logic 不参与运算）。
5. **P1 预收集首列** `first_col_data`，检测是否单调非递减（`is_sorted`）。
6. **路径选择**：

   | 条件 | 路径 | `search_mode` |
   |------|------|---------------|
   | `!has_or && is_sorted && 首列(范围\|包含)` | **二分查找**（P0）：`find_rows_in_sorted` 在首列取候选行；多筛选器时对候选行验证其余列 | `"二分"` |
   | `row_count > PARALLEL_ROW_THRESHOLD (1000)` | **多线程并行线性扫描**（P2）：`std::thread::scope` 分块 | `"并行"` |
   | 否则 | **串行线性扫描** | `"串行"` |

   二分路径仅适用于**纯 AND、首列已排序、且首列运算符为「范围」或「包含」**（`find_rows_in_sorted` 仅支持这两种语义）——OR 语义或其他运算符（`=/!=/>/<` 等）下首列未必匹配却仍可能命中，故回退线性扫描。多筛选器时二分先缩小候选集，再对每个候选行用 `match_filter_value`（携带各自 `op`）验证其余列（按需 `get_cell`，已被候选集大幅缩小）。

7. **跨行合并对齐**：对每个 `ParsedFilter.col` 调用 `expand_hidden_rows_for_merged_cells`。
8. **保证配置行可见**：`hidden_rows.remove(&f.row)`。
9. **状态与诊断**：`row_matched_count = total - hidden`、`is_row_searching=true`、拼装含 `search_mode` 与各列标签的 `row_debug_info`。

---

## 4. 行隐藏 / 显示逻辑（`local_hidden` 填充机制）

`hidden_rows` 是搜索结果的全局隐藏行集合，写入最终结果；`local_hidden` 仅存在于**并行路径**的每个工作线程内部。

### 4.1 `local_hidden` 的填充（并行路径）

并行扫描的关键片段（`search.rs:1397-1422`）：

```rust
let mut local_hidden = HashSet::new();
for idx in start_idx..end_idx {
    let row = all_col_data_ref[0][idx].0;          // 行号取自第 0 列预收集值
    let hide = if use_or {                          // use_or == has_or
        // 多条件 AND/OR：逐条件求值，再按运算符整体求值
        let matches: Vec<bool> = parsed_ref.iter().enumerate()
            .map(|(fi, pf)| match_filter_value(
                &all_col_data_ref[fi][idx].1, &pf.keywords, pf.is_range, pf.op))
            .collect();
        !row_matches_expr(&matches, logic_seq_ref) // 不满足表达式 → 隐藏
    } else {
        // 纯 AND：任一条件不匹配即隐藏
        !parsed_ref.iter().enumerate().all(|(fi, pf)| match_filter_value(
            &all_col_data_ref[fi][idx].1, &pf.keywords, pf.is_range, pf.op))
    };
    if hide { local_hidden.insert(row); }
}
local_hidden  // 作为线程返回值
```

- 每个 chunk 线程只负责自己的行区间 `[start_idx, end_idx)`，结果放进**线程私有** `local_hidden`，无锁无竞争。
- 线程 `join` 后，主线程 `hidden_rows.extend(set)` 合并各线程结果。

### 4.2 与 `use_or`、`logic_seq_ref` 的关联

并行闭包显式捕获了三个共享引用（`&all_col_data`、`&parsed`、`&logic_seq`）和一个拷贝 `use_or = has_or`：

- **`use_or`（=`has_or`）**：决定**分支策略**。无 OR 时走 `all(...)` 短路（更快）；有 OR 时必须逐条件求值成 `Vec<bool>` 再交给 `row_matches_expr` 解析运算符优先级——因为 OR 改变了"全部匹配才可见"的前提。
- **`logic_seq_ref`（=`&logic_seq`）**：仅在 `use_or` 分支被 `row_matches_expr` 消费，用于还原 AND/OR 的分组优先级（见 §5.3）。
- **`all_col_data_ref`**：P1 预收集的所有列值，按 `[筛选器下标 fi][行下标 idx]` 取值，避免循环内重复 `HashMap` 查找与重复日期格式化。
- **`parsed_ref`**：各筛选器的 `col/keywords/is_range`。

### 4.3 合并单元格对齐 `expand_hidden_rows_for_merged_cells`

对每个激活筛选列 `target_col`：遍历 `sheet.merged_cells`，仅处理**跨行**（`start_row != end_row`）且**包含 `target_col`**的合并。以**左上角是否在隐藏集中**为准——左上角可见 → 整列范围全部从隐藏集移除；否则全部插入。这与列搜索的 `expand_hidden_for_merged_cells` 对称（后者处理跨列、按列对齐）。

### 4.4 行/列隐藏集合的语义总结

| 维度 | 集合 | 填充来源 | 清空时机 |
|------|------|----------|----------|
| 隐藏列 | `hidden_columns` | `execute_multi_column_search`：`1..=max_col` 中不在 `visible` 的列 + 合并对齐 | 搜索开始 `clear()`；重置按钮；无列筛选条件时清空 |
| 隐藏行 | `hidden_rows` | `execute_row_search`：三条路径之一 + 合并对齐 + 排除配置行 | 搜索开始 `clear()`；重置；行筛选输入被清空时自动还原 |

---

## 5. 关键函数调用链

### 5.1 `match_filter_value` —— 统一单值判定

```rust
fn match_filter_value(value, keywords, is_range, op) -> bool {
    if is_range && keywords.len() == 2 {
        v >= keywords[0] && v <= keywords[1]   // 闭区间范围匹配（字典序，与 op 无关）
    } else {
        keywords.iter().any(|kw| compare_value(value, kw, op)) // 按 op 逐个判定
    }
}
```

- **范围模式**（`'a'-'b'` 输入）：闭区间 `>= lo && <= hi`，按字符串字典序比较；与运算符 `op` 无关（范围本身即区间语义）。
- **单值/多值模式**：按运算符 `op` 调 `compare_value` 逐个判定，多值时 `any`（命中其一即可）。
- 所有列/行搜索路径（二分候选验证、串行、并行）都收敛到这一个函数，保证语义一致。

### 5.2 `compare_value` / `try_f64_cmp` —— 比较运算符核心

```rust
fn compare_value(value, keyword, op: CompareOp) -> bool {
    match op {
        Contains    => row_fuzzy_match(v, kw),        // 子串
        NotContains => !row_fuzzy_match(v, kw),
        Equal       => v.eq_ignore_ascii_case(kw),    // 精确（忽略大小写）
        NotEqual    => !v.eq_ignore_ascii_case(kw),
        GreaterThan => match try_f64_cmp(v, kw) { Some(o)=>o.is_gt(),  None=> v >  kw },
        LessThan    => match try_f64_cmp(v, kw) { Some(o)=>o.is_lt(),  None=> v <  kw },
        GreaterEqual=> match try_f64_cmp(v, kw) { Some(o)=>!o.is_lt(), None=> v >= kw },
        LessEqual   => match try_f64_cmp(v, kw) { Some(o)=>!o.is_gt(), None=> v <= kw },
    }
}
```

- `try_f64_cmp(a, b)`：将两端 trim 后 `parse::<f64>`，任一失败或遇 `NaN` 返回 `None`。
- **降级策略**：数值类运算符在 `try_f64_cmp` 返回 `None`（非数值输入）时回退为**字符串字典序比较**，避免类型转换失败导致误判。
- 列搜索（`compute_column_matches`）的线性分支同样调用 `compare_value`，使两类筛选运算符语义完全统一。

**运算符语义总表**（`value` = 单元格值，`keyword` = 用户输入）：

| 运算符 | 下拉文案 | 匹配规则 | 非数值降级 |
|--------|----------|----------|------------|
| `Contains` | 包含 | 大小写不敏感子串 `contains` | — |
| `NotContains` | 不包含 | `!contains`（排除） | — |
| `Equal` | `=` | 忽略大小写精确相等 | — |
| `NotEqual` | `!=` | 忽略大小写不等 | — |
| `GreaterThan` | `>` | `f64` 数值 `>` | 字典序 `>` |
| `LessThan` | `<` | `f64` 数值 `<` | 字典序 `<` |
| `GreaterEqual` | `>=` | `f64` 数值 `>=` | 字典序 `>=` |
| `LessEqual` | `<=` | `f64` 数值 `<=` | 字典序 `<=` |

> 行筛选用 `'a','b'` 多值格式时：对每个值分别按 `op` 求值，`any` 命中即匹配；用 `'a'-'b'` 范围格式时：固定闭区间 `[lo,hi]`，与 `op` 无关。

> 注：原 `match_filter_value` 仅做子串包含；新增 `op` 后默认值 `Contains` 保持向后兼容行为不变。

### 5.3 `row_matches_expr` —— AND/OR 表达式求值

`row_matches_expr(matches: &[bool], logic_seq: &[FilterLogic]) -> bool`，实现 **AND 优先级高于 OR** 的分组短路求值：

- `matches[i]`：第 i 个激活条件在该行是否匹配；
- `logic_seq[i]`：第 i 个条件**与下一条**的连接符（最后一条无意义）；
- 因此**条件 i 的前驱运算符** = `logic_seq[i-1]`（首条按 AND 组起始）。

求值逻辑：
1. 维护 `group_ok`（当前 AND 组是否全真）、`group_active`（是否已开始第一组）。
2. 遇到首条件或前驱为 **OR** → 开启新 AND 组：若上一组已全真，立即 `return true`（**OR 短路**）；否则重置 `group_ok = m`。
3. 否则（前驱为 AND）→ `group_ok = group_ok && m`（组内交集）。
4. 末尾返回最后一组 `group_ok`。

> 与列搜索的「按 OR 边界切分 AND 组、组间并集」完全同构，只是一个用**集合运算**实现、一个用**布尔短路**实现。

### 5.4 `parsed_ref` / `parsed` 的构建与使用

`parsed: Vec<ParsedFilter>` 在 `execute_row_search` 内构建：
- 从 `active_filters` 映射：`parse_row_keywords(&f.keyword)` → `(keywords, is_range)`，连同 `f.col` 打包。
- **owned 数据**（`String`/`Vec`），可安全地被 `&parsed`（`parsed_ref`）跨线程借用，无需 `Arc`。
- 消费点：
  - 二分路径：`find_rows_in_sorted(&first_col_data, &parsed[0].keywords, parsed[0].is_range)`，再对候选行用 `parsed[1..].iter().all(|pf| match_filter_value(&value, &pf.keywords, pf.is_range, pf.op))` 验证（候选集已被二分缩小，按需 `get_cell`）；
  - 串行/并行路径：每行每条件取 `all_col_data[fi][idx].1`，配合 `parsed[fi].{keywords,is_range,op}` 调 `match_filter_value`。

### 5.5 调用链总览

```
draw_search_window  (UI：搜索按钮 / Enter 键 / 重置)
 │
 ├─ load_column_options ──► parse_search_range ──► parse_one_segment ──► parse_cell_ref
 │                       └► cell_search_value
 ├─ load_row_filter_configs（同上解析链）
 │
 ├─【列筛选有输入】execute_multi_column_search ──► compute_column_matches(op)
 │                                                  ├► find_sorted_column_matches（已排序 && op==Contains）
 │                                                  └► 线性 compare_value（其他 op / 未排序）
 │                                                  写入 hidden_columns
 │
 ├─【行筛选有输入】execute_row_search
 │   ├─ parse_row_keywords → parsed (ParsedFilter, 含 op)
 │   ├─ logic_seq / has_or / first_binary_compatible
 │   ├─ collect_column_values (P1 预收集)
 │   ├─ 路径选择：
 │   │   ├─ find_rows_in_sorted（二分，首列须 范围|包含）
 │   │   ├─ std::thread::scope + local_hidden + row_matches_expr（并行）
 │   │   └─ 串行 + row_matches_expr
 │   ├─ expand_hidden_rows_for_merged_cells（合并对齐）
 │   └─ 写入 hidden_rows
 │
 └─ match_filter_value(op) ──► compare_value（统一单值判定核心，数值类经 try_f64_cmp）
```

### 5.6 二分查找的细节（`find_sorted_column_matches` / `find_rows_in_sorted` / `search_sorted`）

三者算法同构，核心：
1. 二分定位第一个 `>= keyword`（或范围下界）的元素；
2. **向右扩展**：命中则加入；不命中但已超出字典序范围（前缀不匹配）则提前 `break`；
3. **向左扩展**（仅模糊模式）：同理向左扫到前缀边界终止；
4. 行级二分在**范围模式**下更简单：定位 `>= lo`，向右扫至 `> hi`。

排序检测：`col_values.windows(2).all(|w| w[0].1 <= w[1].1)`（单调非递减）。

---

## 6. 状态管理与 UI 交互

`draw_search_window(ctx, state, excel_data, current_sheet, hidden_columns, hidden_rows)` 是每帧调用的绘制入口，状态与 UI 联动如下：

### 6.1 窗口控制与动画
- `state.visible` 为 false 直接 `return`；`keep_open` 与 egui `Window::open` 绑定。
- **关闭即重置**：点击 `✖` 关闭按钮或 egui 内置关闭（如 Escape）时，自动调用 `reset_search` 清空隐藏行列、搜索状态、用户输入与筛选条件，将表格恢复到搜索前的完整显示——用户无需手动点击重置。
- **折叠动画**：用 `ctx.animate_value_with_time("search_window_expand", target, 0.2)` 驱动 `p∈[0,1]`；`p > 0.001` 才渲染内容区。动画器自动 `request_repaint`，避免逐帧卡顿。点击自定义标题栏切换 `state.collapsed`。

### 6.2 延迟加载（一次性）
- 首次展开（`!options_loaded`）时调用 `load_column_options` + `load_row_filter_configs` 填充 `state.column_options` / `row_filters`，置 `options_loaded=true`，并对越界的 `selected_index` 归零。避免每帧重读配置文件。

### 6.3 搜索触发点（统一执行列搜索 + 行筛选）
三个入口都执行**同一套统一逻辑**：
1. **🔍 搜索按钮**（`has_col_filter || has_row_input` 时启用）；
2. **列筛选输入框 Enter 键**（焦点命中）；
3. **行筛选输入框 Enter 键**。

统一流程：
- 有列筛选输入 → `execute_multi_column_search`，否则 `hidden_columns.clear()`；
- 有行筛选输入 → `hidden_rows.clear()` 后 `execute_row_search`；
- 执行后 `response.surrender_focus()` 收起键盘焦点。

### 6.4 动态增删与编辑
- **列筛选**：`添加筛选条件` 按钮追加空 `ColumnFilter`；每行有列下拉、比较运算符下拉、关键字输入、AND/OR 下拉、删除按钮（`can_delete = count>1`）；删除用 `delete_idx` **延迟到循环外** `remove`，避免迭代中越界。
- **行筛选**：配置项数量由 `search.row` 决定，不可增删，但可编辑 `keyword`、`logic` 与 `op`。

### 6.5 自动还原（行筛选特有）
当某行筛选输入被改空（`response.changed() && keyword.trim().is_empty() && is_row_searching`）时，自动 `hidden_rows.clear()` 并重置行筛选状态——用户清空输入即恢复全表显示，无需点重置。

### 6.6 重置按钮与关闭按钮共享还原逻辑
三个入口共用同一个 `reset_search(state, hidden_columns, hidden_rows)` 私有函数：
- **🔄 重置按钮**：用户点击后调用 `reset_search`。
- **✖ 关闭按钮**：调用 `reset_search` 后置 `visible=false`，关闭弹窗的同时恢复表格。
- **egui 内置关闭**（`!keep_open`）：同上，覆盖 Escape 等关闭路径。

`reset_search` 统一清空：`hidden_columns`/`hidden_rows`、列搜索状态（`is_searching/matched_count/total_searched/search_keyword`/`debug_info`）、行筛选状态（`is_row_searching/matched_count/total_searched/`debug_info`）、所有 `row_filters[i].keyword` 与 `row_filters[i].op`（重置为 `包含`）、并把 `column_filters` 复位为一条空条件（`op=包含`，保留交互骨架）。模式与 `alert_notify.rs` 的 `reset_filter` 一致。

### 6.7 反馈信息
- **列筛选统计**：`is_searching` 时显示 `匹配 N/M 列`，走二分时附加 `(二分)` 标记。
- **行筛选统计**：`row_total_searched>0` 时显示 `匹配 N/M 行`。
- **诊断文本**：`debug_info`（灰色）、`row_debug_info`（深绿），含搜索模式、目标坐标、匹配/隐藏计数、采样值，便于排查。
- **底部提示**：固定文案说明"搜索选中列右侧所有列；已排序数据自动启用二分查找"。

### 6.8 状态 ↔ 隐藏集合 ↔ 渲染
搜索状态（`is_searching`/`is_row_searching`）与两个隐藏集合（`hidden_columns`/`hidden_rows`）是**同一事实的两面**：搜索函数同时更新二者，主表格渲染时根据隐藏集合决定哪些行/列不绘制。`draw_search_window` 把 `hidden_columns`/`hidden_rows` 作为可变引用透传进来，使一次搜索能直接生效到表格视图。

---

## 7. 视觉布局图（UI 布局）

`draw_search_window` 绘制一个 **固定宽度 520px、自定义标题栏、非模态** 的 egui 浮窗。
窗口从上到下分为四个区域：**标题栏 → 列筛选区 → 行筛选区 → 诊断/提示区**。
标题栏左侧为可点击的折叠箭头与标题，右侧三个按钮（egui `right_to_left` 布局，越先添加越靠右）。
列筛选条件行与行筛选项均在「输入框」前增加了一个 **比较运算符下拉框**（宽 60，选项见 `CompareOp`）。

### 7.1 展开态整体布局

```
┌──────────────────────────────────────────────────────────────────────────────────────┐
│                                                                                      │ ← egui::Window
│  ▼  搜索                                        [🔄 重置]  [🔍 搜索]  [✖]            │   520px 宽
│  └──┘点击折叠/展开                               └──────── 搜索触发 ──────────┘       │   title_bar=false
│ ─────────────────────────────────────────────────────────────────────────────────── │ ← separator
│                                                                                      │
│ 列筛选:          匹配 3/10 列 (二分)                        [添加筛选条件] ▶        │ ← 列标题行
│                                                                                      │
│ ┌────────────────┐ ┌──────┐ ┌──────────────────────┐ ┌──────┐ ┌───┐                  │ ← 条件行①
│ │ 序号 (A1)    ▾ │ │包含 ▾│ │ 输入搜索关键字...    │ │ AND ▾│ │ X │                  │   列166/运算符60
│ └────────────────┘ └──────┘ └──────────────────────┘ └──────┘ └───┘                  │   输入180/逻辑50
│ ┌────────────────┐ ┌──────┐ ┌──────────────────────┐ ┌──────┐ ┌───┐                  │ ← 条件行②
│ │ 名称 (B1)    ▾ │ │  >  ▾│ │                      │ │ OR  ▾│ │ X │                  │   删除X
│ └────────────────┘ └──────┘ └──────────────────────┘ └──────┘ └───┘                  │
│                                                                                      │
│ ─────────────────────────────────────────────────────────────────────────────────── │ ← separator
│                                                                                      │
│ 行筛选:                              匹配 50/1000 行                                  │ ← 行标题行
│                                                                                      │
│  日期 (A14): [包含 ▾] [ xxxx 或 'xx1','xx2' 或 'xx3'-'xx4'        ] [AND ▾]          │ ← 行筛选项①
│  入库 (D14): [  >  ▾] [                                            ] [OR ▾]          │   运算符60/输入250
│                                                                                      │
│  行筛选[二分]: [日期=A] 行15→1000 共986行 | 匹配50行 隐藏936行                      │ ← row_debug_info
│  选中A1 | 共10列 | 匹配3列 隐藏7列                                                 │ ← debug_info
│                                                                                      │
│ 💡 搜索选中列右侧所有列；已排序数据自动启用二分查找                                  │ ← 底部提示
└──────────────────────────────────────────────────────────────────────────────────────┘
```

### 7.2 折叠态（`state.collapsed = true`）

`p → 0.0` 动画过程中内容区渐隐，达到阈值（`p <= 0.001`）后**只渲染标题栏**：

```
┌──────────────────────────────────────────────────────────────────────────┐
│  ▶  搜索                                  [🔄 重置]  [🔍 搜索]  [✖]      │
└──────────────────────────────────────────────────────────────────────────┘
   ▲ 箭头变为 ▶，点击重新展开
```

### 7.3 区域与字段映射表

| 区域 | UI 元素 | 绑定的状态字段 / 动作 | 宽度/约束 |
|------|---------|----------------------|-----------|
| 标题栏 | 折叠箭头 `▶/▼` + "搜索" | 点击切换 `state.collapsed` | — |
| 标题栏 | `🔄 重置` | 调用 `reset_search` 清空全部（与关闭按钮共用） | — |
| 标题栏 | `🔍 搜索` | 有激活输入时启用；执行列搜索 + 行筛选 | enabled = `has_col \|\| has_row` |
| 标题栏 | `✖` | `reset_search(...)` 后置 `visible = false`，关闭时自动恢复表格 | — |
| 列标题行 | "列筛选:" + `匹配 N/M 列 (二分)` | `is_searching` / `matched_count` / `total_searched` / `use_binary_search` | — |
| 列标题行 | `添加筛选条件` | `column_filters.push(空 ColumnFilter)` | right_to_left |
| 条件行 | 列下拉 ComboBox | `column_filters[idx].column_index` → `column_options` | 166 |
| 条件行 | 比较运算符 ComboBox | `column_filters[idx].op`（默认 `包含`，见 `CompareOp::ALL`） | 60 |
| 条件行 | 关键字 TextEdit | `column_filters[idx].filter_value`（Enter 触发搜索） | 180 |
| 条件行 | AND/OR ComboBox | `column_filters[idx].logic` | 50 |
| 条件行 | 删除 `X` | `column_filters.remove(idx)`（`count>1` 才可删） | — |
| 行标题行 | "行筛选:" + `匹配 N/M 行` | `row_total_searched` / `row_matched_count` | — |
| 行筛选项 | `标题 (cell_ref):` | 标签（取自单元格值） | — |
| 行筛选项 | 比较运算符 ComboBox | `row_filters[idx].op`（默认 `包含`） | 60 |
| 行筛选项 | 关键字 TextEdit | `row_filters[idx].keyword`（清空自动还原；Enter 触发搜索） | 250 |
| 行筛选项 | AND/OR ComboBox | `row_filters[idx].logic` | 50 |
| 诊断区 | 绿色/灰色文本 | `row_debug_info` / `debug_info` | size 10 |
| 底部 | 💡 提示 | 固定文案 | size 10 |

### 7.4 交互流程时序

```
用户输入 → [Enter] 或 [🔍 搜索]
   │
   ├─ has_col_filter? ─是─► execute_multi_column_search ─► hidden_columns 更新
   │                  └─否─► hidden_columns.clear()
   │
   └─ has_row_input?  ─是─► hidden_rows.clear()
   │                       └─► execute_row_search ─► hidden_rows 更新
   │
   └─► 主表格按 hidden_columns / hidden_rows 重新渲染（隐藏对应行列）
   └─► 状态统计/诊断文本刷新显示
```

> 布局说明：egui 中 `right_to_left` 布局会把"先 `add` 的控件"放到最右侧，因此标题栏右侧从左到右实际显示顺序是 `[🔄 重置] [🔍 搜索] [✖]`（`✖` 最后渲染、最靠右）。条件行/筛选项则按 `column_filters` / `row_filters` 的索引顺序自上而下排列，行间有 2px 间距。

---

## 附：性能优化要点汇总

| 手段 | 位置 | 作用 |
|------|------|------|
| 配置/选项一次性加载 | `options_loaded` | 避免每帧重读 yaml、重解析范围 |
| P1 预收集列值 | `collect_column_values` | 消除搜索循环内重复 `HashMap` 查找与日期格式化 |
| P0 二分查找 | `find_rows_in_sorted` / `find_sorted_column_matches` | 已排序数据 O(log n + k) |
| P2 并行扫描 | `std::thread::scope` + `local_hidden` | >1000 行时分块并行，无锁合并 |
| 自适应路径选择 | `execute_row_search` | 按排序性 / 行数自动选最优路径 |
| 短路求值 | `row_matches_expr` 的 OR 短路 / `all(...)` | 命中即停，减少无效比较 |
| 阈值常量 | `PARALLEL_ROW_THRESHOLD=1000`、`MIN_ROWS_PER_THREAD=500` | 控制并行粒度，避免线程过细 |

---

*文档生成依据：`src/gui/widgets/search.rs`（master 分支 HEAD `26d55bc`；search.rs 最近变更 `fe77811`「添加比较运算符支持」，本文件已据此同步 `CompareOp` 相关说明）。如代码重构，请同步更新本文件中的行号与函数签名。*
