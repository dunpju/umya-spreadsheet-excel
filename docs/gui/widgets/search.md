# 搜索组件业务实现分析（`src/gui/widgets/search.rs`）

> 本文档基于 `search.rs`（约 2076 行）源码梳理，系统阐述 GUI 搜索窗口的模块定位、数据结构、
> 搜索/筛选流程、隐藏集合填充机制、关键调用链，状态与 UI 联动逻辑，以及视觉布局。

---

## 1. 模块概览

`search.rs` 是 Excel 查看器 GUI 层的**搜索/筛选窗口组件**，隶属于 `gui::widgets` 模块。它负责：

- **读取配置驱动搜索范围**：从用户主目录下 `~/.MyExcel/my-excel.yaml` 的 `search.column` 与 `search.row` 两个键，解析出若干可选的段（支持单格、普通范围、离散、**步长语法** `(:+N)` 以及 `~` 末尾占位），将它们的值作为"列可选范围"与"行可选范围"。
- **两类独立的筛选维度**：
  - **列筛选（Column Filter）**：以某组锚点单元格为基准，在其所在行的**右侧所有列**中按运算符匹配关键字，**隐藏不匹配的列**。步长段（纵向步进）覆盖多个锚点行，对各行取 OR。
  - **行筛选（Row Filter）**：以某组锚点单元格为基准，在其所在列的**下方所有行**中按运算符匹配关键字，**隐藏不匹配的行**。步长段（横向步进）覆盖多个锚点列，对各列取 OR。
- **多条件 AND/OR 组合**：两类筛选都支持动态增删多条条件（行筛选与列筛选同构），遵循 **MySQL 风格的运算符优先级——AND 优先级高于 OR**。
- **性能优化**：自适应地在「二分查找 / 多线程并行扫描 / 串行线性扫描」三种路径间选择，并对已排序、跨行/跨列合并单元格做了专门处理。
- **非模态窗口**：`draw_search_window` 绘制一个可折叠、可关闭、独立于主窗口操作的浮层。

依赖关系上，它只向上暴露：
- 结构体：`RangeOption`、`ColumnFilter`、`RowFilterCondition`、`FilterLogic`、`CompareOp`、`SearchWindowState`；
- 公开函数：`load_column_options`、`load_row_options`、`execute_multi_column_search`、`execute_row_search`、`draw_search_window`。

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

**`RangeOption`** —— 列筛选 / 行筛选共用的一个可选范围（`search.column` / `search.row` 每个逗号段对应一个选项）：
- `title: String`：显示文本（首个锚点单元格的值，如 "序号"、"入库"）。
- `cell_ref: String`：首个锚点坐标字符串，如 `"A1"`、`"B14"`。
- `cells: Vec<(u32, u32)>`：展开后的**全部**锚点单元格 (col, row)，1-based。普通范围长度为 1；步长语法范围含按步长展开的全部锚点。
- `is_step: bool`：是否步长语法范围（决定 `display()` 是否追加 `(expr)`）。
- `expr: String`：原始段表达式文本（步长项显示用，如 `"(B:+2)14"`）。
- `display() -> String`：`is_step` 时返回 `` `值(expr)` ``（如 `入库((B:+2)14)`）；否则返回 `` `值 (cell_ref)` ``（如 `入库 (A14)`）。
- `first_col() -> u32` / `first_row() -> u32`：首个锚点的列号 / 行号（便捷访问器）。

**`RowFilterCondition`** —— 动态行筛选条件（与 `ColumnFilter` 同构）：
- `range_index: usize`：选中项在 `row_options` 中的下标。
- `keyword: String`：用户输入的关键字。
- `logic: FilterLogic`：**与下一条条件**的组合逻辑（最后一条无意义）。
- `op: CompareOp`：比较运算符（默认 `包含`）。
- `is_active()`：关键字 trim 后非空即激活。

**`ColumnFilter`** —— 动态列筛选条件（现有，不变）：
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
- 下拉框宽度固定 `60.0`，插入位置：列筛选行 / 行筛选行的「范围 ComboBox」与「关键字输入框」之间。

### 2.3 状态聚合 `SearchWindowState`

`SearchWindowState` 是搜索窗口的完整可变状态，分四组：

| 分组 | 字段 | 作用 |
|------|------|------|
| 窗口控制 | `visible`、`collapsed` | 窗口可见性 / 折叠状态（默认展开） |
| 下拉框数据 | `column_options: Vec<RangeOption>`、`row_options: Vec<RangeOption>`、`selected_index`、`options_loaded` | 列/行可选范围、当前选中索引、是否已加载（避免每帧重解析） |
| 搜索输入/状态 | `search_keyword`、`is_searching`、`matched_count`、`total_searched`、`use_binary_search`、`debug_info` | 列搜索关键字、是否生效、匹配/总数、是否走了二分、诊断文本 |
| 行筛选 | `row_filters: Vec<RowFilterCondition>`、`is_row_searching`、`row_matched_count`、`row_total_searched`、`row_debug_info` | 行筛选动态条件（与列筛选同构，可增删） |
| 多条件列筛选 | `column_filters: Vec<ColumnFilter>`、`filter_logic`（`#[allow(dead_code)]` 兼容保留） | 动态列条件列表 |

`Default` 初始化：`visible=false`、`collapsed=false`，并预置一条空的 `ColumnFilter{column_index:0, filter_value:"", logic:And, op:Contains}`。`row_filters` 在首次加载 `row_options` 非空时初始化一条 `RowFilterCondition{range_index:0, keyword:"", logic:And, op:Contains}`。

---

## 3. 搜索功能流程

### 3.1 配置与范围解析

**`parse_cell_ref(s) -> Option<(col,row)>`**
将 `"A1"` 转为 `(1,1)`：取前导字母段做 26 进制累加（`A=1…Z=26, AA=27`），取后续数字段解析行号；`col==0||row==0` 视为非法。

**`parse_cell_ref_bound(s, max_col, max_row) -> Option<(col,row)>`**
扩展 `parse_cell_ref`，支持 `~` 末尾占位（与 `alert_notify.rs` / `reader.rs` 的 `resolve_dynamic_range` 约定一致）：
- `"~14"`（`~` 在列位）→ `(max_col, 14)`
- `"A~"`（`~` 在行位）→ `(1, max_row)`
- `"~"` → `(max_col, max_row)`
- 无 `~` 时等价于 `parse_cell_ref`。

**`parse_search_segments(input, max_col, max_row) -> Vec<RangeSegment>`**（总入口）
按 `,` 分段，每段产出若干 `RangeSegment{cells, expr, is_step}`：
- **步长语法段**（含 `(`...`:+N`...`)`）：`parse_step_segment` 解析 → 产出 **1 个**分组段（`is_step=true`，`cells` 含按步长展开的全部锚点）。
- **普通段**（单格 / `-` 范围）：`parse_one_segment_resolved` 解析（含 `~`）→ 按 **每个单元格一个段** 扁平产出（`is_step=false`）。

**`parse_step_segment(seg, max_col, max_row) -> Option<RangeSegment>`**
由 `(:+N)` 位置决定步进方向：
- **纵（步进行）**：`字母(起始行:+步长)[:终止单元格]`，如 `A(1:+2):A13`、`A(1:+2)`（无 `:end` 默认至 `max_row`）。
- **横（进列）**：`(起始列:+步长)行[:终止单元格]`，如 `(B:+2)14:~14`、`(B:+2)14`（无 `:end` 默认至 `max_col`）。
- `step==0` 视为非法返回 `None`。

**`read_search_config(key) -> String`** / **`build_range_options(sheet, range_str) -> Vec<RangeOption>`**
共享辅助：从 yaml 读 `search.column`/`search.row` → `parse_search_segments` → 每个段组装 `RangeOption`（`title` 取首个锚点单元格值，`expr` 存原始段文本）。

**`cell_search_value(cell)` —— 搜索用显示值**
与表格渲染保持一致：若 `number_format` 是日期格式且 `value` 可解析为 `f64` 序列号，返回 `format_date` 结果（如 `"2028/7/14"`）；否则返回 `value` 原值。

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
3. **逐条计算匹配列集合**（调用 `compute_column_matches(sheet, opt, …)`，传 `&RangeOption`，见 §5）：
   - **单锚点**（普通段）：收集该行右侧列值，已排序且 `op==Contains` 时二分；
   - **多锚点**（纵向步长段）：线性扫描，任一锚点行命中即该列可见（OR）。
   同时累加 `max_total` 与 `any_binary`。
4. **AND 优先于 OR 的分组组合**：维护 `current_and_group` 与 `or_groups`（与旧实现一致）。
5. **组间并集**、**写回隐藏集合**、**更新状态与诊断**（同旧流程）。

> 示例：`C1 OR C2 AND C3 AND C4 OR C5` → 分组 `[C1] [C2∩C3∩C4] [C5]` → 结果 `C1 ∪ (C2∩C3∩C4) ∪ C5`。

单条件旧实现 `execute_search`（`#[allow(dead_code)]`）仅改用 `RangeOption` 访问器，逻辑不变。

### 3.4 行筛选流程（多条件 + 自适应路径，含 cond_match 预计算）

入口 `execute_row_search(state, sheet, hidden_rows)`：

1. `hidden_rows.clear()`；空 `row_options` / 无激活项则写诊断后返回。
2. `start_row` 取首个激活条件所选范围的首个锚点行号；`row_count = max_row - start_row`。
3. **ParsedFilter** 改为 `{ cols: Vec<u32>, keywords, is_range, op }`——`cols` 为该条件锚点列的**去重升序**集合（步长段覆盖多列，匹配时取 OR）。
4. **cond_match 预计算**：`cond_match[fi][idx] = parsed[fi].cols.iter().any(|c| match_filter_value(cell(row,c), kw, is_range, op))`——多列 OR 在此一次性收敛为单 bool。预计算后并行/串行热循环 **不再 get_cell**，且统一多列处理。
5. 运算符序列 `logic_seq` / `has_or`（同上）。
6. **路径选择**：

   | 条件 | 路径 | `search_mode` |
   |------|------|---------------|
   | `!has_or && 首条件单列 && 已排序 && (范围\|包含)` | **二分**（P0）：首列 `find_rows_in_sorted` 取候选行，其余条件用 `cond_match` 校验 | `"二分"` |
   | `row_count > PARALLEL_ROW_THRESHOLD (1000)` | **并行**（P2）：`std::thread::scope` 分块，直接读 `cond_match` | `"并行"` |
   | 否则 | **串行线性**，直接读 `cond_match` | `"串行"` |

   二分路径追加门槛 `parsed[0].cols.len()==1`（首条件须单列），多列时必走线性/并行。
7. **跨行合并对齐**：对每个条件**每个锚点列**调用 `expand_hidden_rows_for_merged_cells`。
8. **配置行可见**：对每个激活条件的每个锚点 `hidden_rows.remove(&r)`。

---

## 4. 行隐藏 / 显示逻辑（`local_hidden` 填充机制）

`hidden_rows` 是搜索结果的全局隐藏行集合，写入最终结果；`local_hidden` 仅存在于**并行路径**的每个工作线程内部。

### 4.1 `cond_match` 预计算与并行/串行路径

回退扫描前统一预计算 `cond_match: Vec<Vec<bool>>`（条件 × 数据行），每条条件调用 `columns.iter().any(|c| match_filter_value(cell(row,c),kw,is_range,op))`，多列 OR 在此一次性收敛为单 bool。**并行/串行热循环直接读 `cond_match`，不再 get_cell**，且统一多列处理。

并行扫描的关键片段（合并前 `cond_match` 后简化）：

```rust
let mut local_hidden = HashSet::new();
for idx in start_idx..end_idx {
    let row = start_row + 1 + idx as u32;
    let hide = if use_or {
        let matches: Vec<bool> = cond_match_ref.iter().map(|cm| cm[idx]).collect();
        !row_matches_expr(&matches, logic_seq_ref)
    } else {
        !cond_match_ref.iter().all(|cm| cm[idx])
    };
    if hide { local_hidden.insert(row); }
}
local_hidden
```

### 4.2 线程间共享

并行闭包显式捕获 `cond_match_ref: &Vec<Vec<bool>>`（只读，无锁）与 `logic_seq_ref: &[FilterLogic]`，以及拷贝 `use_or = has_or`。每个 chunk 线程产出私有 `local_hidden`，`join` 后由主线程 `hidden_rows.extend` 合并。

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
- 从 `active_filters` 映射：`parse_row_keywords(&f.keyword)` → `(keywords, is_range)`，连同 `state.row_options[f.range_index].cells` 去重列集合打包为 `cols`。
- **owned 数据**（`Vec<u32>` / `Vec<String>`），可安全地跨线程借用。
- 消费点：`cond_match` 预计算（每个条件对其 `cols` 取 OR，每个数据行一个 bool）；二分路径（首条件须 `cols.len()==1` 时可用）。

### 5.5 调用链总览

```
draw_search_window  (UI：搜索按钮 / Enter 键 / 重置)
 │
 ├─ load_column_options ──► build_range_options ──► parse_search_segments
 │                       │                           ├► parse_step_segment  (步长: `(:+N)`)
 │                       │                           └► parse_one_segment_resolved (普通 + `~`)
 │                       └► cell_search_value
 ├─ load_row_options（同上解析链）
 │
 ├─【列筛选有输入】execute_multi_column_search ──► compute_column_matches(option: &RangeOption, op)
 │                                                  ├► 单锚点: find_sorted_column_matches（已排序&&Contains）
 │                                                  └► 多锚点: 线性 OR 各锚点行
 │                                                  写入 hidden_columns
 │
 ├─【行筛选有输入】execute_row_search
 │   ├─ parse_row_keywords + row_options[*].cells 去重 → parsed.cols
 │   ├─ logic_seq / has_or / first_binary_compatible
 │   ├─ cond_match 预计算（每条件对其 cols 取 OR，每行一个 bool）
 │   ├─ 路径选择：
 │   │   ├─ find_rows_in_sorted（二分，首条件单列+排序+范围|包含）
 │   │   ├─ std::thread::scope + cond_match + row_matches_expr（并行）
 │   │   └─ 串行 + cond_match + row_matches_expr
 │   ├─ expand_hidden_rows_for_merged_cells（每列对齐）
 │   └─ 写入 hidden_rows
 │
 └─ match_filter_value(op) / compare_value / try_f64_cmp（统一比较核心）
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
- **列筛选**：`添加筛选条件` 按钮追加空 `ColumnFilter`；每行有范围 ComboBox（`RangeOption::display()`）、比较运算符下拉、关键字输入、AND/OR 下拉、删除按钮（`can_delete = count>1`）；删除用 `delete_idx` **延迟到循环外** `remove`。
- **行筛选**：与列筛选**完全同构**——头部 `添加筛选条件` 按钮追加 `RowFilterCondition{range_index:0,…}`；每行有行范围 ComboBox（从 `row_options` 选）、比较运算符、关键字、AND/OR、删除 `X`。

### 6.5 自动还原（行筛选特有）
当某行筛选输入被改空（`response.changed() && keyword.trim().is_empty() && is_row_searching`）时，自动 `hidden_rows.clear()` 并重置行筛选状态。

### 6.6 重置按钮与关闭按钮共享还原逻辑
三个入口共用同一个 `reset_search(state, hidden_columns, hidden_rows)` 私有函数：
- **🔄 重置按钮** / **✖ 关闭按钮** / **egui 内置关闭**：均调用 `reset_search`。

`reset_search` 统一清空：`hidden_columns`/`hidden_rows`、列/行搜索状态、`debug_info`、`row_filters` 恢复为单条 `RowFilterCondition{range_index:0,…}`（仅当 `row_options` 非空）、`column_filters` 恢复为单条空 `ColumnFilter`。模式与 `alert_notify.rs` 的 `reset_filter` 一致。

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
标题栏左侧为可点击的折叠箭头与标题，右侧三个按钮（egui `right_to_left` 布局）。
列筛选 / 行筛选均在「范围 ComboBox」与「关键字输入框」之间插入了一个 **比较运算符下拉框**（宽 60，选项见 `CompareOp`）。
行筛选与列筛选**同构**：范围 ComboBox（`RangeOption::display()` 步长项显示 `值((expr))` 否则 `值 (cell_ref)`）→ 比较运算符 → 关键字 → AND/OR → 删除 `X`；头部均有「添加筛选条件」按钮、均可动态增删（`delete_idx` 延迟删除）。

### 7.1 展开态整体布局

```
┌──────────────────────────────────────────────────────────────────────────────────────┐
│                                                                                      │
│  ▼  搜索                                        [🔄 重置]  [🔍 搜索]  [✖]            │ ← 标题栏
│ ─────────────────────────────────────────────────────────────────────────────────── │
│                                                                                      │
│ 列筛选:          匹配 3/10 列 (二分)                        [添加筛选条件] ▶        │ ← 列标题行
│                                                                                      │
│ ┌────────────────┐ ┌──────┐ ┌──────────────────────┐ ┌──────┐ ┌───┐                  │
│ │ 序号 (A1)    ▾ │ │包含 ▾│ │ 输入搜索关键字...    │ │ AND ▾│ │ X │                  │   列166/运算符60
│ └────────────────┘ └──────┘ └──────────────────────┘ └──────┘ └───┘                  │   输入180/逻辑50/删除
│ ┌────────────────┐ ┌──────┐ ┌──────────────────────┐ ┌──────┐ ┌───┐                  │
│ │ 名称 (B1)    ▾ │ │  >  ▾│ │                      │ │ OR  ▾│ │ X │                  │
│ └────────────────┘ └──────┘ └──────────────────────┘ └──────┘ └───┘                  │
│                                                                                      │
│ ─────────────────────────────────────────────────────────────────────────────────── │
│                                                                                      │
│ 行筛选:                              匹配 50/1000 行            [添加筛选条件] ▶    │ ← 行标题行
│                                                                                      │
│ ┌────────────────┐ ┌──────┐ ┌──────────────────────┐ ┌──────┐ ┌───┐                  │
│ │ 入库 (A14)   ▾ │ │包含 ▾│ │ xxxx 或 'xx1','xx2'  │ │ AND ▾│ │ X │                  │   行范围166/运算符60
│ └────────────────┘ └──────┘ └──────────────────────┘ └──────┘ └───┘                  │   输入250/逻辑50/删除
│ ┌────────────────┐ ┌──────┐ ┌──────────────────────┐ ┌──────┐ ┌───┐                  │
│ │ 入库((B:+2)14)▾│ │  >  ▾│ │                      │ │ OR  ▾│ │ X │                  │   步长项显示 expr
│ └────────────────┘ └──────┘ └──────────────────────┘ └──────┘ └───┘                  │
│                                                                                      │
│  行筛选[二分]: [入库=A] 行15→1000 共986行 | 匹配50行 隐藏936行                      │ ← row_debug_info
│  选中A1 | 共10列 | 匹配3列 隐藏7列                                                 │ ← debug_info
│ 💡 搜索选中列右侧所有列；已排序数据自动启用二分查找                                  │
└──────────────────────────────────────────────────────────────────────────────────────┘
```

### 7.2 折叠态（`state.collapsed = true`）

```
┌──────────────────────────────────────────────────────────────────────────┐
│  ▶  搜索                                  [🔄 重置]  [🔍 搜索]  [✖]      │
└──────────────────────────────────────────────────────────────────────────┘
```

### 7.3 区域与字段映射表

| 区域 | UI 元素 | 绑定 | 宽度 |
|------|---------|------|------|
| 列标题行 | "列筛选:" + stats | `is_searching` / `matched_count` | — |
| 列标题行 | `添加筛选条件` | push `ColumnFilter{column_index:0,…}` | right_to_left |
| 列条件行 | 列范围 ComboBox | `RangeOption::display()`（步长 `值(expr)` 否则 `值 (ref)`） | 166 |
| 列条件行 | 比较运算符 ComboBox | `column_filters[idx].op` | 60 |
| 列条件行 | 关键字 TextEdit | `column_filters[idx].filter_value` | 180 |
| 列条件行 | AND/OR ComboBox / 删除 `X` | `logic` / `remove(idx)` | 50 / — |
| 行标题行 | "行筛选:" + stats | `row_total_searched` | — |
| 行标题行 | `添加筛选条件` | push `RowFilterCondition{range_index:0,…}` | right_to_left |
| 行条件行 | 行范围 ComboBox | `row_options[range_index].display()` | 166 |
| 行条件行 | 比较运算符 ComboBox | `row_filters[idx].op` | 60 |
| 行条件行 | 关键字 TextEdit | `row_filters[idx].keyword`（清空自动还原） | 250 |
| 行条件行 | AND/OR ComboBox / 删除 `X` | `logic` / `remove(idx)` | 50 / — |
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
| cond_match 预计算 | `execute_row_search` | 多列 OR 一次性收敛为 bool，热循环零 get_cell |
| P1 预收集列值 | `collect_column_values` | 二分首列预收集，消除循环内 HashMap 查找 |
| P0 二分查找 | `find_rows_in_sorted` / `find_sorted_column_matches` | 已排序数据 O(log n + k) |
| P2 并行扫描 | `std::thread::scope` + `local_hidden` | >1000 行时分块并行，cond_match 只读无锁 |
| 自适应路径选择 | `execute_row_search` | 按排序性 / 行数 / 首条件单列 自动选最优路径 |
| 短路求值 | `row_matches_expr` 的 OR 短路 / `all(...)` | 命中即停，减少无效比较 |
| 阈值常量 | `PARALLEL_ROW_THRESHOLD=1000`、`MIN_ROWS_PER_THREAD=500` | 控制并行粒度，避免线程过细 |

---

*本文件基于 `src/gui/widgets/search.rs`（最新 master），已同步步长语法 `(:+N)`、`~` 末尾占位、`RangeOption` / `RowFilterCondition` 重构、`cond_match` 预计算、行筛选动态 ComboBox 等所有变更。如代码重构，请同步更新本文件。*
