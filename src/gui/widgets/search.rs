//! 搜索窗口组件
//!
//! 提供列筛选与行筛选两类独立的筛选维度：从配置文件 `search.column` / `search.row`
//! 读取可选范围（支持单格、普通范围、离散、以及**步长语法** `(:+N)` 与 `~` 末尾占位），
//! 通过比较运算符与 AND/OR 多条件组合隐藏不匹配的行/列，支持合并单元格跨行/跨列感知。

use eframe::egui;
use std::collections::HashSet;
use crate::excel::reader::{CellData, ExcelData, SheetData};

// ═══════════════════════════════════════════════════════════════
// 性能优化参数
// ═══════════════════════════════════════════════════════════════

/// 并行行扫描阈值：超过此行数时启用多线程线性扫描
const PARALLEL_ROW_THRESHOLD: usize = 1000;
/// 每个线程最少处理的行数（避免线程粒度过细）
const MIN_ROWS_PER_THREAD: usize = 500;

/// 获取单元格的搜索用显示值
///
/// 与表格渲染 `cell_display_text` 保持一致：
/// - 日期格式单元格返回格式化日期字符串（如 "2028/7/14"）
/// - 普通单元格返回原始值
fn cell_search_value(cell: &CellData) -> String {
    if let Some(ref fmt) = cell.number_format {
        if ExcelData::is_date_format(fmt) {
            if let Ok(serial) = cell.value.parse::<f64>() {
                return ExcelData::format_date(serial, fmt);
            }
        }
    }
    cell.value.clone()
}

/// 一个可选范围（列筛选 / 行筛选共用）
///
/// - **普通段**（单格或普通范围扁平展开后）：`cells` 仅含一个锚点单元格；
/// - **步长语法段**（如 `(B:+2)14:~14`）：作为一整组，`cells` 含按步长展开的全部锚点单元格，
///   匹配时对这些锚点取 **OR（任一命中）**。
#[derive(Debug, Clone)]
pub struct RangeOption {
    /// 显示文本：首个锚点单元格的值（如 "序号"、"入库"）
    pub title: String,
    /// 首个锚点单元格的坐标字符串，如 "A1"、"B14"（普通项显示用）
    pub cell_ref: String,
    /// 展开后的全部锚点单元格 (col, row)，1-based，按展开顺序；普通项长度为 1
    pub cells: Vec<(u32, u32)>,
    /// 是否步长语法范围（决定 `display` 是否追加 `(expr)`）
    pub is_step: bool,
    /// 原始段表达式文本（步长项显示用，如 "(B:+2)14"）
    pub expr: String,
}

impl RangeOption {
    /// 下拉框显示文案
    /// - 步长项：`值(expr)`，如 `入库((B:+2)14)`（expr 自带括号，外层再加一对）
    /// - 普通项：`值 (cell_ref)`，如 `序号 (A1)`
    pub fn display(&self) -> String {
        if self.is_step {
            format!("{}({})", self.title, self.expr)
        } else {
            format!("{} ({})", self.title, self.cell_ref)
        }
    }

    /// 首个锚点列号（1-based），无锚点时返回 0
    pub fn first_col(&self) -> u32 {
        self.cells.first().map(|(c, _)| *c).unwrap_or(0)
    }

    /// 首个锚点行号（1-based），无锚点时返回 0
    pub fn first_row(&self) -> u32 {
        self.cells.first().map(|(_, r)| *r).unwrap_or(0)
    }
}

/// 行筛选条件（支持多条动态增删，与列筛选 `ColumnFilter` 同构）
///
/// 每个条件通过 `range_index` 引用 `row_options` 中的一个可选范围：
/// - 普通范围 → 单个锚点列；
/// - 步长范围 → 一组锚点列，匹配时对这些列取 **OR（任一命中）**。
#[derive(Debug, Clone)]
pub struct RowFilterCondition {
    /// 选中的范围选项索引（0-based，对应 `SearchWindowState::row_options`）
    pub range_index: usize,
    /// 用户输入的关键字
    pub keyword: String,
    /// 该条件与下一条条件的组合逻辑（And 取交集，Or 取并集）
    /// 最后一条的 logic 无实际效果（无下一条可组合）。
    pub logic: FilterLogic,
    /// 比较运算符（默认 包含）
    pub op: CompareOp,
}

impl RowFilterCondition {
    /// 该条件是否激活（用户输入了非空白关键字）
    pub fn is_active(&self) -> bool {
        !self.keyword.trim().is_empty()
    }
}

/// 列筛选条件（支持多条动态增删）
///
/// 每个条件包含一个目标列选择（通过 column_options 索引）、
/// 一个筛选值和一个独立的逻辑组合方式（And/Or），
/// 用于按列值过滤数据行。
#[derive(Debug, Clone)]
pub struct ColumnFilter {
    /// 选中的列选项索引（0-based，对应 column_options）
    pub column_index: usize,
    /// 筛选值（关键字）
    pub filter_value: String,
    /// 该条件与下一条条件的组合逻辑（And 取交集，Or 取并集）
    /// 最后一条条件的 logic 无实际效果（无下一条可组合）。
    pub logic: FilterLogic,
    /// 比较运算符（默认 包含）
    pub op: CompareOp,
}

impl ColumnFilter {
    /// 该筛选条件是否激活（用户输入了筛选值）
    pub fn is_active(&self) -> bool {
        !self.filter_value.is_empty()
    }
}

/// 多条件列筛选的逻辑组合
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilterLogic {
    /// 所有条件均需匹配
    And,
    /// 任一条件匹配即可
    Or,
}

/// 比较运算符（列筛选 / 行筛选用）
///
/// 决定单元格值与关键字之间的匹配关系：
/// - 字符串类 `Contains`/`NotContains`：大小写不敏感的子串匹配；
/// - 精确类 `Equal`/`NotEqual`：忽略大小写的相等比较；
/// - 数值类 `GreaterThan`/`LessThan`/`GreaterEqual`/`LessEqual`：优先按 f64 数值比较，
///   任一端无法解析为数值时**降级为字符串字典序比较**，避免类型转换失败导致误判。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CompareOp {
    /// 包含（子串匹配）
    Contains,
    /// 不包含（排除匹配）
    NotContains,
    /// 等于（精确比较）
    Equal,
    /// 不等于
    NotEqual,
    /// 大于
    GreaterThan,
    /// 小于
    LessThan,
    /// 大于等于
    GreaterEqual,
    /// 小于等于
    LessEqual,
}

impl Default for CompareOp {
    fn default() -> Self {
        CompareOp::Contains
    }
}

impl CompareOp {
    /// 下拉框显示文案
    pub fn label(&self) -> &'static str {
        match self {
            CompareOp::Contains => "包含",
            CompareOp::NotContains => "不包含",
            CompareOp::Equal => "=",
            CompareOp::NotEqual => "!=",
            CompareOp::GreaterThan => ">",
            CompareOp::LessThan => "<",
            CompareOp::GreaterEqual => ">=",
            CompareOp::LessEqual => "<=",
        }
    }

    /// 所有运算符（下拉框渲染顺序）
    pub const ALL: [CompareOp; 8] = [
        CompareOp::Contains,
        CompareOp::NotContains,
        CompareOp::Equal,
        CompareOp::NotEqual,
        CompareOp::GreaterThan,
        CompareOp::LessThan,
        CompareOp::GreaterEqual,
        CompareOp::LessEqual,
    ];
}

/// 搜索窗口状态
#[derive(Debug)]
pub struct SearchWindowState {
    // ========== 窗口控制 ==========
    /// 搜索窗口是否可见
    pub visible: bool,
    /// 是否折叠（折叠时仅显示标题栏，点击标题栏展开）。默认展开。
    pub collapsed: bool,

    // ========== 下拉框数据 ==========
    /// 列筛选可选范围列表（从 search.column 解析，每个段一个 `RangeOption`）
    pub column_options: Vec<RangeOption>,
    /// 当前选中的选项索引（0-based）
    pub selected_index: usize,
    /// 下拉框选项是否已加载（避免每帧重新解析配置）
    pub options_loaded: bool,

    // ========== 搜索输入 ==========
    /// 搜索关键字
    pub search_keyword: String,

    // ========== 搜索状态 ==========
    /// 是否已执行搜索（搜索结果生效中）
    pub is_searching: bool,
    /// 搜索匹配的列数
    pub matched_count: usize,
    /// 被搜索的总列数
    pub total_searched: usize,
    /// 是否使用二分查找（自动检测，仅供参考）
    pub use_binary_search: bool,
    /// 诊断信息：搜索目标行/列 + 前几个搜索值的采样
    pub debug_info: String,

    // ========== 行筛选（支持多列、动态增删） ==========
    /// 行筛选可选范围列表（从 search.row 解析，每个段一个 `RangeOption`）
    pub row_options: Vec<RangeOption>,
    /// 行筛选条件列表（动态增删，每条引用 `row_options` 中的一个范围）
    pub row_filters: Vec<RowFilterCondition>,
    /// 行筛选是否已执行
    pub is_row_searching: bool,
    /// 行筛选匹配的行数
    pub row_matched_count: usize,
    /// 行筛选搜索的总行数
    pub row_total_searched: usize,
    /// 行筛选诊断信息
    pub row_debug_info: String,

    // ========== 多条件列筛选（扩展行过滤） ==========
    /// 动态列筛选条件列表
    pub column_filters: Vec<ColumnFilter>,
    /// 列筛选逻辑（And / Or，保留兼容，实际使用 ColumnFilter.logic）
    #[allow(dead_code)]
    pub filter_logic: FilterLogic,
}

impl Default for SearchWindowState {
    fn default() -> Self {
        Self {
            visible: false,
            collapsed: false,
            column_options: Vec::new(),
            selected_index: 0,
            options_loaded: false,
            search_keyword: String::new(),
            is_searching: false,
            matched_count: 0,
            total_searched: 0,
            use_binary_search: false,
            debug_info: String::new(),
            row_options: Vec::new(),
            row_filters: Vec::new(),
            is_row_searching: false,
            row_matched_count: 0,
            row_total_searched: 0,
            row_debug_info: String::new(),
            column_filters: vec![ColumnFilter {
                column_index: 0,
                filter_value: String::new(),
                logic: FilterLogic::And,
                op: CompareOp::Contains,
            }],
            filter_logic: FilterLogic::And, // 保留兼容，新逻辑使用 ColumnFilter.logic
        }
    }
}

/// 获取配置文件路径 ~/.MyExcel/my-excel.yaml
fn config_path() -> std::path::PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    home.join(".MyExcel").join("my-excel.yaml")
}

/// 解析单个单元格引用 "A1" → (col: 1, row: 1)
fn parse_cell_ref(s: &str) -> Option<(u32, u32)> {
    let s = s.trim().to_uppercase();
    if s.is_empty() {
        return None;
    }

    let col_part: String = s.chars().take_while(|c| c.is_alphabetic()).collect();
    let row_part: String = s.chars().skip_while(|c| c.is_alphabetic()).collect();

    if col_part.is_empty() || row_part.is_empty() {
        return None;
    }

    // 列字母 → 数字 (A=1, B=2, ..., Z=26, AA=27, ...)
    let col = col_part
        .chars()
        .fold(0u32, |acc, c| acc * 26 + (c as u32 - 'A' as u32 + 1));

    let row = row_part.parse::<u32>().ok()?;

    if col == 0 || row == 0 {
        return None;
    }

    Some((col, row))
}

/// 解析单元格引用，支持 `~` 末尾占位
///
/// 与 `alert_notify.rs` / `reader.rs` 的 `resolve_dynamic_range` 约定一致：
/// - `"A1"` → `(1, 1)`（无 `~` 时等价于 [`parse_cell_ref`]）
/// - `"~14"`（`~` 在列位）→ `(max_col, 14)`：该行最大列
/// - `"A~"`（`~` 在行位）→ `(1, max_row)`：该列最大行
/// - `"~"` → `(max_col, max_row)`：右下角
fn parse_cell_ref_bound(s: &str, max_col: u32, max_row: u32) -> Option<(u32, u32)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if !s.contains('~') {
        return parse_cell_ref(s);
    }
    if s == "~" {
        return Some((max_col.max(1), max_row.max(1)));
    }
    let upper = s.to_uppercase();
    if upper.starts_with('~') {
        // ~行号 → (max_col, row)
        let row: u32 = upper[1..].trim().parse().ok()?;
        if row == 0 {
            return None;
        }
        Some((max_col.max(1), row))
    } else if upper.ends_with('~') {
        // 列字母~ → (col, max_row)
        let col_part: String = upper.chars().take_while(|c| c.is_alphabetic()).collect();
        if col_part.is_empty() {
            return None;
        }
        let col = col_part
            .chars()
            .fold(0u32, |acc, c| acc * 26 + (c as u32 - 'A' as u32 + 1));
        if col == 0 {
            return None;
        }
        Some((col, max_row.max(1)))
    } else {
        // 形如 A~B 之类的异常片段，回退普通解析（通常解析失败返回 None）
        parse_cell_ref(s)
    }
}

/// 解析后的一个范围段（一个逗号分隔段）
#[derive(Debug, Clone)]
struct RangeSegment {
    /// 展开后的锚点单元格 (col, row)，≥1
    cells: Vec<(u32, u32)>,
    /// 原始段表达式文本（步长项显示用）
    expr: String,
    /// 是否步长语法范围
    is_step: bool,
}

/// 解析步长语法段
///
/// 由 `(:+N)` 的位置决定步进方向：
/// - **纵向（步进行）**：`字母(起始行:+N)[:终止单元格]`，如 `A(1:+2):A13`、`A(1:+2)`
/// - **横向（进列）**：`(起始列:+N)行[:终止单元格]`，如 `(B:+2)14:~14`、`(B:+2)14`
///
/// 终止单元格可省略（纵向默认至 `max_row`，横向默认至 `max_col`），可含 `~`（见 [`parse_cell_ref_bound`]）。
/// 解析失败或步长为 0 时返回 `None`。
fn parse_step_segment(seg: &str, max_col: u32, max_row: u32) -> Option<RangeSegment> {
    let open = seg.find('(')?;
    let close = seg.find(')').filter(|&c| c > open)?;
    let before = seg[..open].trim(); // `(` 前：纵向时为列字母，横向时为空
    let inner = seg[open + 1..close].trim(); // `(` `)` 之间：`X:+N`
    let tail = seg[close + 1..].trim_start(); // `)` 之后

    // 内层按 `:+` 拆出 (左, 步长)
    let plus = inner.find(":+")?;
    let left = inner[..plus].trim();
    let step: u32 = inner[plus + 2..].trim().parse().ok()?;
    if step == 0 {
        return None;
    }

    // 拆 tail 为 (前缀, 可选 end)：首个 `:` 之后为终止单元格
    let (prefix, end_str) = match tail.find(':') {
        Some(idx) => (tail[..idx].trim(), Some(tail[idx + 1..].trim())),
        None => (tail.trim(), None),
    };

    let left_is_alpha = !left.is_empty() && left.chars().all(|c| c.is_alphabetic());

    let mut cells = Vec::new();
    if !before.is_empty() {
        // 纵向：before=列字母，left=起始行（数字）
        let col = parse_cell_ref(&format!("{}1", before))?.0;
        let start_row: u32 = left.parse().ok()?;
        let end_row = match end_str {
            Some(s) => parse_cell_ref_bound(s, max_col, max_row)?.1,
            None => max_row,
        };
        let mut r = start_row;
        while r <= end_row {
            cells.push((col, r));
            r += step;
        }
    } else if left_is_alpha {
        // 横向：left=起始列字母，prefix=行号（数字）
        let row: u32 = prefix.parse().ok()?;
        let start_col = parse_cell_ref(&format!("{}1", left))?.0;
        let end_col = match end_str {
            Some(s) => parse_cell_ref_bound(s, max_col, max_row)?.0,
            None => max_col,
        };
        let mut c = start_col;
        while c <= end_col {
            cells.push((c, row));
            c += step;
        }
    } else {
        return None;
    }

    if cells.is_empty() {
        return None;
    }
    Some(RangeSegment {
        cells,
        expr: seg.trim().to_string(),
        is_step: true,
    })
}

/// 解析普通段（单格或 `-` 范围），支持 `~` 末尾占位，返回扁平单元格列表
fn parse_one_segment_resolved(seg: &str, max_col: u32, max_row: u32) -> Vec<(u32, u32)> {
    let seg = seg.trim();
    if seg.is_empty() {
        return Vec::new();
    }
    // 范围格式 "A1-A13" / "A1-C1"（终点可含 `~`）
    if let Some(idx) = seg.find('-') {
        let after_dash = &seg[idx + 1..];
        if after_dash
            .trim()
            .chars()
            .next()
            .map_or(false, |c| c.is_alphabetic() || c == '~')
        {
            let start_str = &seg[..idx];
            let end_str = after_dash;
            if let (Some(start), Some(end)) = (
                parse_cell_ref_bound(start_str, max_col, max_row),
                parse_cell_ref_bound(end_str, max_col, max_row),
            ) {
                let mut result = Vec::new();
                if start.0 == end.0 {
                    // 同列：行范围 A1-A13
                    let (lo, hi) = if start.1 <= end.1 {
                        (start.1, end.1)
                    } else {
                        (end.1, start.1)
                    };
                    for row in lo..=hi {
                        result.push((start.0, row));
                    }
                } else if start.1 == end.1 {
                    // 同行：列范围 A1-C1
                    let (lo, hi) = if start.0 <= end.0 {
                        (start.0, end.0)
                    } else {
                        (end.0, start.0)
                    };
                    for col in lo..=hi {
                        result.push((col, start.1));
                    }
                }
                return result;
            }
        }
    }
    // 单个单元格
    parse_cell_ref_bound(seg, max_col, max_row).into_iter().collect()
}

/// 解析单个逗号段：步长段 → 1 个分组段；普通段 → 若干扁平单格段（每格一段）
fn parse_one_range_segment(seg: &str, max_col: u32, max_row: u32) -> Vec<RangeSegment> {
    let seg = seg.trim();
    if seg.is_empty() {
        return Vec::new();
    }
    // 步长语法段：含 `(` 且能解析为步长 → 一整组
    if seg.contains('(') {
        if let Some(rs) = parse_step_segment(seg, max_col, max_row) {
            return vec![rs];
        }
        // 形似步长但解析失败：按普通段兜底（通常返回空）
    }
    // 普通段：扁平展开为单格
    parse_one_segment_resolved(seg, max_col, max_row)
        .into_iter()
        .map(|(col, row)| {
            let col_letter = crate::excel::reader::col_to_letter(col);
            RangeSegment {
                cells: vec![(col, row)],
                expr: format!("{}{}", col_letter, row),
                is_step: false,
            }
        })
        .collect()
}

/// 解析搜索范围字符串为若干 [`RangeSegment`]
///
/// 按 `,` 分段：
/// - **步长语法段**（含 `(`...`:+N`...`)`）作为**一整组**产出 1 个段（覆盖范围内所有单元格）；
/// - **普通段**（单格 / 普通范围）**扁平展开**为若干单格段（每个单元格一个段，向后兼容）。
///
/// 普通格式：`A1`、`A1-A13`、`A1,A3`、`A1-A13,A15`（含 `~` 末尾占位）。
/// 步长格式：`A(1:+2):A13`、`(B:+2)14:~14`、`A(1:+2)`、`(B:+2)14`。
fn parse_search_segments(input: &str, max_col: u32, max_row: u32) -> Vec<RangeSegment> {
    let input = input.trim();
    if input.is_empty() {
        return Vec::new();
    }
    input
        .split(',')
        .flat_map(|seg| parse_one_range_segment(seg, max_col, max_row))
        .collect()
}

/// 读取 `my-excel.yaml` 中 `search` 节点下指定键（`column` / `row`）的字符串值
fn read_search_config(key: &str) -> String {
    let path = config_path();
    if !path.exists() {
        return String::new();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|c| serde_yaml::from_str::<serde_yaml::Value>(&c).ok())
        .and_then(|doc| {
            doc.get("search")
                .and_then(|s| s.get(key))
                .and_then(|v| v.as_str().map(|s| s.to_string()))
        })
        .unwrap_or_default()
}

/// 将配置范围字符串解析为 [`RangeOption`] 列表
///
/// `title` 取首个锚点单元格的值；步长段的 `cells` 含按步长展开的全部锚点。
fn build_range_options(sheet: &SheetData, range_str: &str) -> Vec<RangeOption> {
    if range_str.trim().is_empty() {
        return Vec::new();
    }
    parse_search_segments(range_str, sheet.max_col, sheet.max_row)
        .into_iter()
        .filter_map(|seg| {
            let first = seg.cells.first().copied()?;
            let title = sheet
                .get_cell(first.1, first.0)
                .map(|c| cell_search_value(c))
                .unwrap_or_default();
            let col_letter = crate::excel::reader::col_to_letter(first.0);
            let cell_ref = format!("{}{}", col_letter, first.1);
            Some(RangeOption {
                title,
                cell_ref,
                cells: seg.cells,
                is_step: seg.is_step,
                expr: seg.expr,
            })
        })
        .collect()
}

/// 从配置文件和 Excel 数据加载列筛选下拉选项（`search.column`）
pub fn load_column_options(
    excel_data: &ExcelData,
    current_sheet: usize,
) -> Vec<RangeOption> {
    let sheet = match excel_data.get_sheet(current_sheet) {
        Some(s) => s,
        None => return Vec::new(),
    };
    build_range_options(sheet, &read_search_config("column"))
}

/// 从配置文件和 Excel 数据加载行筛选可选范围（`search.row`）
///
/// 每个 [`RangeOption`] 对应 `search.row` 中的一个段：普通段为单个锚点列，
/// 步长段为一组锚点列（匹配时取 OR）。
pub fn load_row_options(
    excel_data: &ExcelData,
    current_sheet: usize,
) -> Vec<RangeOption> {
    let sheet = match excel_data.get_sheet(current_sheet) {
        Some(s) => s,
        None => return Vec::new(),
    };
    build_range_options(sheet, &read_search_config("row"))
}

/// 合并单元格列可见性对齐
///
/// 对于跨列合并：左上角单元格的值代表整个合并区域。
/// - 左上角匹配（不在隐藏集）→ 整个合并范围的所有列都设为可见
/// - 左上角不匹配（在隐藏集中）→ 整个合并范围的所有列都隐藏
#[allow(dead_code)]
fn expand_hidden_for_merged_cells(
    sheet: &SheetData,
    hidden_columns: &mut HashSet<u32>,
    target_row: u32,
) {
    for mr in &sheet.merged_cells {
        // 只处理跨列合并（start_col != end_col）
        if mr.start_col == mr.end_col {
            continue;
        }
        // 只处理包含目标行的合并
        if target_row < mr.start_row || target_row > mr.end_row {
            continue;
        }
        // 以左上角是否匹配为准：左上角不在隐藏集中 = 匹配
        let top_left_visible = !hidden_columns.contains(&mr.start_col);
        if top_left_visible {
            // 左上角匹配 → 整个合并范围全部可见
            for c in mr.start_col..=mr.end_col {
                hidden_columns.remove(&c);
            }
        } else {
            // 左上角不匹配 → 整个合并范围全部隐藏
            for c in mr.start_col..=mr.end_col {
                hidden_columns.insert(c);
            }
        }
    }
}

/// 在已排序的列值中搜索匹配区间（二分查找 + 双向线性确认）
#[allow(dead_code)]
fn search_sorted(
    col_values: &[(u32, String)],
    keyword: &str,
    hidden_columns: &mut HashSet<u32>,
) {
    let n = col_values.len();
    if n == 0 {
        return;
    }

    // 二分定位第一个 ≥ keyword 的元素
    let mut lo = 0usize;
    let mut hi = n;
    while lo < hi {
        let mid = (lo + hi) / 2;
        if col_values[mid].1.as_str() < keyword {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }

    let mut matched_indices = HashSet::new();

    // 向右扩展
    let mut i = lo;
    while i < n {
        let val = &col_values[i].1;
        if val.contains(keyword) {
            matched_indices.insert(i);
        } else {
            // 如果当前值已经超出 keyword 的字典序范围，右侧不再可能有匹配
            if val.as_str() > keyword {
                let prefix_len = keyword.len().min(val.len());
                if !val[..prefix_len].starts_with(&keyword[..prefix_len]) {
                    break;
                }
            }
        }
        i += 1;
    }

    // 向左扩展
    if lo > 0 {
        let mut i = lo.saturating_sub(1);
        loop {
            let val = &col_values[i].1;
            if val.contains(keyword) {
                matched_indices.insert(i);
            } else {
                let prefix_len = keyword.len().min(val.len());
                if val.as_str() < keyword
                    && !keyword[..prefix_len].starts_with(&val[..prefix_len])
                {
                    break;
                }
            }
            if i == 0 {
                break;
            }
            i -= 1;
        }
    }

    // 未匹配的列 → 隐藏
    for (idx, (col, _)) in col_values.iter().enumerate() {
        if !matched_indices.contains(&idx) {
            hidden_columns.insert(*col);
        }
    }
}

/// 在已排序列值中二分查找匹配列（返回匹配的列号集合）
///
/// 与 `search_sorted` 算法同构，区别在于返回匹配列而非隐藏列，
/// 用于多条件组合搜索时独立计算每条条件的匹配结果。
fn find_sorted_column_matches(
    col_values: &[(u32, String)],
    keyword: &str,
) -> HashSet<u32> {
    let n = col_values.len();
    if n == 0 {
        return HashSet::new();
    }

    // 二分定位第一个 ≥ keyword 的元素
    let mut lo = 0usize;
    let mut hi = n;
    while lo < hi {
        let mid = (lo + hi) / 2;
        if col_values[mid].1.as_str() < keyword {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }

    let mut matched_indices = HashSet::new();

    // 向右扩展
    let mut i = lo;
    while i < n {
        let val = &col_values[i].1;
        if val.contains(keyword) {
            matched_indices.insert(i);
        } else if val.as_str() > keyword {
            let prefix_len = keyword.len().min(val.len());
            if !val[..prefix_len].starts_with(&keyword[..prefix_len]) {
                break;
            }
        }
        i += 1;
    }

    // 向左扩展
    if lo > 0 {
        let mut i = lo.saturating_sub(1);
        loop {
            let val = &col_values[i].1;
            if val.contains(keyword) {
                matched_indices.insert(i);
            } else {
                let prefix_len = keyword.len().min(val.len());
                if val.as_str() < keyword
                    && !keyword[..prefix_len].starts_with(&val[..prefix_len])
                {
                    break;
                }
            }
            if i == 0 {
                break;
            }
            i -= 1;
        }
    }

    matched_indices.into_iter().map(|idx| col_values[idx].0).collect()
}

/// 计算单条列筛选条件匹配的列集合
///
/// 以 `option` 的锚点为基准，在锚点列**右侧的所有列**中匹配关键字：
/// - **单锚点**（普通段）：保留「收集该行右侧列值 + (已排序 && Contains) 二分」优化；
/// - **多锚点**（纵向步长段）：线性扫描，**任一锚点行**命中即该列可见（OR）。
///
/// 返回：(匹配列集合, 搜索的列总数, 是否使用二分查找)
fn compute_column_matches(
    sheet: &SheetData,
    option: &RangeOption,
    keyword: &str,
    op: CompareOp,
) -> (HashSet<u32>, usize, bool) {
    let keyword = keyword.to_lowercase();
    let max_col = sheet.max_col;
    let start_col = option.first_col();
    if start_col == 0 {
        return (HashSet::new(), 0, false);
    }

    // 锚点行：去重升序（列筛选锚点同列，行集合即锚点行）
    let mut anchor_rows: Vec<u32> = option.cells.iter().map(|(_, r)| *r).collect();
    anchor_rows.sort_unstable();
    anchor_rows.dedup();

    if anchor_rows.len() == 1 {
        // ── 单锚点：收集该行右侧列值，(已排序 && Contains) 时二分 ──
        let target_row = anchor_rows[0];
        let mut col_values: Vec<(u32, String)> = Vec::new();
        for col in (start_col + 1)..=max_col {
            let value = sheet
                .get_cell(target_row, col)
                .map(|c| cell_search_value(c).to_lowercase())
                .unwrap_or_default();
            col_values.push((col, value));
        }
        let total = col_values.len();
        if total == 0 {
            return (HashSet::new(), 0, false);
        }
        let is_sorted = col_values.windows(2).all(|w| w[0].1 <= w[1].1);
        let used_binary = is_sorted && op == CompareOp::Contains;
        let mut visible: HashSet<u32> = if used_binary {
            find_sorted_column_matches(&col_values, &keyword)
        } else {
            col_values
                .iter()
                .filter(|(_, v)| compare_value(v, &keyword, op))
                .map(|(col, _)| *col)
                .collect()
        };
        align_visible_for_merged(sheet, &mut visible, &anchor_rows);
        return (visible, total, used_binary);
    }

    // ── 多锚点（步长段）：线性扫描，任一锚点行命中即该列可见（OR） ──
    let mut visible: HashSet<u32> = HashSet::new();
    let mut total = 0usize;
    for col in (start_col + 1)..=max_col {
        total += 1;
        let matched = anchor_rows.iter().any(|&r| {
            let value = sheet
                .get_cell(r, col)
                .map(|c| cell_search_value(c).to_lowercase())
                .unwrap_or_default();
            compare_value(&value, &keyword, op)
        });
        if matched {
            visible.insert(col);
        }
    }
    align_visible_for_merged(sheet, &mut visible, &anchor_rows);
    (visible, total, false)
}

/// 合并单元格跨列可见性对齐（作用于可见列集合）
///
/// 对跨列合并且**与任一锚点行重叠**的区域：以左上角是否可见为准，整段对齐。
/// 与 `expand_hidden_for_merged_cells` 对称，区别在作用于可见集合而非隐藏集合。
fn align_visible_for_merged(
    sheet: &SheetData,
    visible: &mut HashSet<u32>,
    anchor_rows: &[u32],
) {
    for mr in &sheet.merged_cells {
        if mr.start_col == mr.end_col {
            continue;
        }
        let overlaps = anchor_rows.iter().any(|&r| r >= mr.start_row && r <= mr.end_row);
        if !overlaps {
            continue;
        }
        if visible.contains(&mr.start_col) {
            for c in mr.start_col..=mr.end_col {
                visible.insert(c);
            }
        } else {
            for c in mr.start_col..=mr.end_col {
                visible.remove(&c);
            }
        }
    }
}

/// 执行多条件列搜索（支持 AND/OR 组合，AND 优先级高于 OR）
///
/// 遍历所有激活的列筛选条件，对每条条件独立计算匹配列集合，
/// 然后遵循 MySQL 一致的运算符优先级规则进行组合：
/// - **AND** 优先级高于 **OR**
/// - 将条件按 OR 边界分组，每组内用 AND 取交集，组间取并集
///
/// 例如 `条件1 OR 条件2 AND 条件3` 被解析为 `条件1 OR (条件2 AND 条件3)`。
///
/// 第一条条件的列选项和关键字同步写入 `selected_index` / `search_keyword`
/// 以保持向后兼容。
pub fn execute_multi_column_search(
    state: &mut SearchWindowState,
    sheet: &SheetData,
    hidden_columns: &mut HashSet<u32>,
) {
    let active: Vec<&ColumnFilter> = state
        .column_filters
        .iter()
        .filter(|f| f.is_active())
        .collect();

    if active.is_empty() {
        return;
    }

    // 同步第一条条件到 selected_index / search_keyword（向后兼容）
    state.selected_index = active[0].column_index;
    state.search_keyword = active[0].filter_value.clone();

    // ═══ AND 优先级高于 OR：按 OR 分组，组内 AND 取交集，组间取并集 ═══
    // 例如: C1 OR C2 AND C3 AND C4 OR C5
    //   → 分组: [C1], [C2 AND C3 AND C4], [C5]
    //   → 结果: C1 ∪ (C2∩C3∩C4) ∪ C5
    let mut or_groups: Vec<HashSet<u32>> = Vec::new();
    let mut current_and_group: Option<HashSet<u32>> = None;
    let mut max_total = 0usize;
    let mut any_binary = false;

    for (i, f) in active.iter().enumerate() {
        if f.column_index >= state.column_options.len() {
            continue;
        }
        let opt = &state.column_options[f.column_index];
        let (filter_visible, ft, fis) =
            compute_column_matches(sheet, opt, &f.filter_value, f.op);
        max_total = max_total.max(ft);
        any_binary = any_binary || fis;

        // 使用前一条条件的 logic 决定当前条件与前一条的组合方式
        // 条件 N 的 logic 表示"该条件与下一条条件的组合逻辑"
        // 因此条件 i 的前驱运算符 = active[i-1].logic
        let prev_logic = if i == 0 {
            FilterLogic::And // 第一条无前驱，作为 AND 组起始
        } else {
            active[i - 1].logic
        };

        if prev_logic == FilterLogic::And {
            // 第一条条件或 AND 连接：合并到当前 AND 组
            if let Some(ref mut group) = current_and_group {
                // AND：取交集（缩小范围）
                *group = group.intersection(&filter_visible).copied().collect();
            } else {
                current_and_group = Some(filter_visible);
            }
        } else {
            // OR：结束当前 AND 组，开始新的 AND 组
            if let Some(group) = current_and_group.take() {
                or_groups.push(group);
            }
            current_and_group = Some(filter_visible);
        }
    }

    // 将最后一个 AND 组加入 OR 组列表
    if let Some(group) = current_and_group.take() {
        or_groups.push(group);
    }

    // 合并所有 OR 组（并集）
    let mut visible: HashSet<u32> = HashSet::new();
    for group in or_groups {
        visible.extend(group);
    }

    // 应用结果到 hidden_columns
    hidden_columns.clear();
    for col in 1..=sheet.max_col {
        if !visible.contains(&col) {
            hidden_columns.insert(col);
        }
    }

    // 确保所有激活条件的目标列自身不被隐藏
    for f in &active {
        if f.column_index < state.column_options.len() {
            hidden_columns.remove(&state.column_options[f.column_index].first_col());
        }
    }

    // 更新状态
    state.total_searched = max_total;
    state.matched_count = visible.len();
    state.use_binary_search = any_binary;
    state.is_searching = true;

    // 诊断信息
    let first_ref = &state.column_options[active[0].column_index];
    let col_letter = crate::excel::reader::col_to_letter(first_ref.first_col());
    state.debug_info = format!(
        "选中{}{} | 共{}列 | 匹配{}列 隐藏{}列",
        col_letter,
        first_ref.first_row(),
        max_total,
        visible.len(),
        hidden_columns.len()
    );
}

/// 执行搜索操作
///
/// 在选中列所在行的右侧所有列中进行模糊匹配，
/// 不匹配的列将被加入 hidden_columns 集合。
#[allow(dead_code)]
pub fn execute_search(
    state: &mut SearchWindowState,
    sheet: &SheetData,
    hidden_columns: &mut HashSet<u32>,
) {
    hidden_columns.clear();

    let opt = match state.column_options.get(state.selected_index) {
        Some(o) => o,
        None => return,
    };

    let keyword = state.search_keyword.to_lowercase();
    let target_col = opt.first_col();
    let target_row = opt.first_row();
    let max_col = sheet.max_col;

    // 收集搜索范围内所有列的头值（选中列右侧的所有列）
    // 使用 cell_search_value 获取显示值（日期格式化等），与表格渲染一致
    let mut col_values: Vec<(u32, String)> = Vec::new();
    for col in (target_col + 1)..=max_col {
        let value = sheet
            .get_cell(target_row, col)
            .map(|c| cell_search_value(c).to_lowercase())
            .unwrap_or_default();
        col_values.push((col, value));
    }

    state.total_searched = col_values.len();

    if col_values.is_empty() {
        state.matched_count = 0;
        state.is_searching = true;
        let col_letter = crate::excel::reader::col_to_letter(target_col);
        state.debug_info = format!(
            "选中{}{} → 行{}右侧无列可搜索（max_col={}）",
            col_letter, target_row, target_row, max_col
        );
        return;
    }

    // 记录展开前的隐藏列数，用于检测合并单元格影响
    let hidden_before_merge: usize;

    // 检测是否已排序（单调非递减）
    let is_sorted = col_values.windows(2).all(|w| w[0].1 <= w[1].1);
    state.use_binary_search = is_sorted;

    if is_sorted {
        search_sorted(&col_values, &keyword, hidden_columns);
    } else {
        for (col, value) in &col_values {
            if !value.contains(&keyword) {
                hidden_columns.insert(*col);
            }
        }
    }

    hidden_before_merge = hidden_columns.len();

    // 处理合并单元格跨列
    expand_hidden_for_merged_cells(sheet, hidden_columns, target_row);

    // 确保选中列自身不被隐藏
    hidden_columns.remove(&target_col);

    state.matched_count = state.total_searched.saturating_sub(hidden_columns.len());
    state.is_searching = true;

    // 构建详细诊断信息
    let col_letter = crate::excel::reader::col_to_letter(target_col);
    let col_letter_next = crate::excel::reader::col_to_letter(target_col + 1);
    let max_col_letter = crate::excel::reader::col_to_letter(max_col);
    let merged_effect = hidden_columns.len().saturating_sub(hidden_before_merge);
    let mut diag = format!(
        "选中{}{} | 行{} {}→{} 共{}列 | 匹配{}列 隐藏{}列",
        col_letter, target_row, target_row,
        col_letter_next, max_col_letter,
        state.total_searched, state.matched_count, hidden_columns.len()
    );
    if merged_effect > 0 {
        diag.push_str(&format!(" (含合并扩展+{})", merged_effect));
    }
    // 采样：显示匹配列的前几个
    let matched_cols: Vec<String> = col_values
        .iter()
        .filter(|(col, _)| !hidden_columns.contains(col))
        .take(5)
        .map(|(col, v)| format!("{}='{}'", crate::excel::reader::col_to_letter(*col), v))
        .collect();
    if !matched_cols.is_empty() {
        diag.push_str(" | 匹配: ");
        diag.push_str(&matched_cols.join(", "));
    } else {
        diag.push_str(" | ⚠ 无匹配列!");
        // 采样几个被隐藏列的值帮助排查
        let hidden_samples: Vec<String> = col_values
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .take(3)
            .map(|(col, v)| format!("{}='{}'", crate::excel::reader::col_to_letter(*col), v))
            .collect();
        if !hidden_samples.is_empty() {
            diag.push_str(&format!(" 被隐藏列样本: {}", hidden_samples.join(", ")));
        }
    }
    state.debug_info = diag;
}

/// 绘制搜索窗口
///
/// 非模态窗口，支持独立于主窗口操作。
///
/// # 参数
/// * `ctx` - egui 上下文
// ═══════════════════════════════════════════════════════════════
// 行筛选
// ═══════════════════════════════════════════════════════════════

/// 解析行筛选关键字输入
///
/// 支持三种格式：
/// - 单值: `xxxx` → vec!["xxxx"]
/// - 多值: `'xxx1','xxx2'` → vec!["xxx1", "xxx2"]
/// - 范围: `'xxx3'-'xxx4'` → vec!["xxx3", "xxx4"] (标记为范围)
fn parse_row_keywords(input: &str) -> (Vec<String>, bool) {
    // (keywords, is_range)
    let input = input.trim();
    if input.is_empty() {
        return (Vec::new(), false);
    }

    // 检测范围格式：包含 '-' 且两端有引号值
    if let Some(idx) = input.find('\'') {
        let rest = &input[idx..];
        if let Some(dash_pos) = rest.find('-') {
            let before_dash = &rest[..dash_pos].trim();
            let after_dash = rest[dash_pos + 1..].trim();
            if before_dash.ends_with('\'') && after_dash.starts_with('\'') {
                let v1 = before_dash.trim_matches('\'').to_string();
                let v2 = after_dash.trim_matches('\'').to_string();
                if !v1.is_empty() && !v2.is_empty() {
                    return (vec![v1, v2], true);
                }
            }
        }
    }

    // 检测多值格式：逗号分隔的引号值
    if input.contains(',') {
        let values: Vec<String> = input
            .split(',')
            .map(|s| s.trim().trim_matches('\'').to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !values.is_empty() {
            return (values, false);
        }
    }

    // 单值
    (vec![input.to_string()], false)
}

/// 行筛选模糊匹配：大小写不敏感子串匹配
fn row_fuzzy_match(cell_value: &str, keyword: &str) -> bool {
    cell_value.to_lowercase().contains(&keyword.to_lowercase())
}

/// 尝试将两个字符串解析为 f64 并比较，返回排序结果
///
/// 任一端无法解析为数值（或出现 NaN）时返回 `None`，调用方可据此降级为字符串比较。
fn try_f64_cmp(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    let av: f64 = a.trim().parse().ok()?;
    let bv: f64 = b.trim().parse().ok()?;
    av.partial_cmp(&bv)
}

/// 比较运算符求值：单元格值 `value` 与关键字 `keyword` 在 `op` 下是否匹配
///
/// - 字符串类：`Contains`/`NotContains` 大小写不敏感子串；`Equal`/`NotEqual` 忽略大小写精确比较。
/// - 数值类：`>`/`<`/`>=`/`<=` 优先按 f64 数值比较；**任一端不可解析为数值时降级为字符串字典序比较**，
///   避免因类型转换失败而误判（例如单元格是文本而关键字是数字）。
fn compare_value(value: &str, keyword: &str, op: CompareOp) -> bool {
    let v = value.trim();
    let kw = keyword.trim();
    match op {
        CompareOp::Contains => row_fuzzy_match(v, kw),
        CompareOp::NotContains => !row_fuzzy_match(v, kw),
        CompareOp::Equal => v.eq_ignore_ascii_case(kw),
        CompareOp::NotEqual => !v.eq_ignore_ascii_case(kw),
        CompareOp::GreaterThan => match try_f64_cmp(v, kw) {
            Some(o) => o.is_gt(),
            None => v > kw,
        },
        CompareOp::LessThan => match try_f64_cmp(v, kw) {
            Some(o) => o.is_lt(),
            None => v < kw,
        },
        CompareOp::GreaterEqual => match try_f64_cmp(v, kw) {
            Some(o) => !o.is_lt(),
            None => v >= kw,
        },
        CompareOp::LessEqual => match try_f64_cmp(v, kw) {
            Some(o) => !o.is_gt(),
            None => v <= kw,
        },
    }
}

// ═══════════════════════════════════════════════════════════════
// 行筛选性能优化辅助函数
// ═══════════════════════════════════════════════════════════════

/// P1: 预收集某列的搜索用值
///
/// 一次性提取 [start_row+1, max_row] 范围内指定列的所有单元格值，
/// 完成日期格式化 + 小写转换，消除搜索循环内的重复 HashMap 查找。
/// 返回按行号升序排列的 (行号, 值) 列表。
fn collect_column_values(
    sheet: &SheetData,
    col: u32,
    start_row: u32,
    max_row: u32,
) -> Vec<(u32, String)> {
    (start_row + 1..=max_row)
        .map(|row| {
            let value = sheet
                .get_cell(row, col)
                .map(|c| cell_search_value(c).to_lowercase())
                .unwrap_or_default();
            (row, value)
        })
        .collect()
}

/// 检查单个值是否匹配筛选条件（统一范围匹配、运算符匹配与模糊匹配逻辑）
fn match_filter_value(value: &str, keywords: &[String], is_range: bool, op: CompareOp) -> bool {
    if is_range && keywords.len() == 2 {
        // 范围查询：闭区间 [lo, hi]，与运算符无关（范围本身即区间语义）
        let v = value.trim();
        v >= keywords[0].as_str() && v <= keywords[1].as_str()
    } else {
        // 单值/多值：按运算符逐个判定，命中任一即可
        keywords.iter().any(|kw| compare_value(value, kw, op))
    }
}

/// 评估单行在多条件 AND/OR 表达式下是否匹配（AND 优先级高于 OR）
///
/// 与列搜索（`execute_multi_column_search`）的分组规则一致：
/// 按 OR 边界将条件切分为若干 AND 组，组内全部为真（取交集），
/// 组间任一为真即可（取并集）。
///
/// - `matches[i]`：第 i 个激活筛选条件在该行是否匹配；
/// - `logic_seq[i]`：第 i 个筛选条件与下一条的组合逻辑（最后一条无意义），
///   因此条件 i 与条件 i-1 之间的运算符为 `logic_seq[i - 1]`。
fn row_matches_expr(matches: &[bool], logic_seq: &[FilterLogic]) -> bool {
    if matches.is_empty() {
        return true;
    }
    let mut group_ok = true; // 当前 AND 组是否全部为真
    let mut group_active = false; // 是否已开始第一个 AND 组
    for (i, &m) in matches.iter().enumerate() {
        // 条件 i 的前驱运算符 = logic_seq[i - 1]（首条件无前驱，按 AND 组起始）
        let prev_logic = if i == 0 {
            FilterLogic::And
        } else {
            // 数组越界保护：logic_seq 与 matches 等长
            logic_seq.get(i - 1).copied().unwrap_or(FilterLogic::And)
        };
        if i == 0 || prev_logic == FilterLogic::Or {
            // 开始新 AND 组：若上一组全部为真，OR 整体即匹配（短路）
            if group_active && group_ok {
                return true;
            }
            group_ok = m;
            group_active = true;
        } else {
            // AND：组内取交集
            group_ok = group_ok && m;
        }
    }
    // 最后一组的 AND 结果
    group_ok
}

/// P0: 在已排序行值中二分查找匹配行
///
/// 返回匹配行的行号集合。与列筛选 `search_sorted` 算法同构：
/// - 范围查询: 二分定位 lo，向右扫描至 > hi
/// - 模糊查询: 二分定位 keyword，双向扩展 + 前缀边界提前终止
fn find_rows_in_sorted(
    row_values: &[(u32, String)],
    keywords: &[String],
    is_range: bool,
) -> HashSet<u32> {
    let n = row_values.len();
    if n == 0 {
        return HashSet::new();
    }

    if is_range && keywords.len() == 2 {
        // ── 范围匹配：二分定位 lo，区间扫描 ──
        let lo = &keywords[0];
        let hi = &keywords[1];

        // 二分定位第一个 >= lo 的元素
        let mut left = 0usize;
        let mut right = n;
        while left < right {
            let mid = (left + right) / 2;
            if row_values[mid].1.as_str() < lo.as_str() {
                left = mid + 1;
            } else {
                right = mid;
            }
        }

        // 从 left 向右扫描直到值 > hi
        let mut matched = HashSet::new();
        for i in left..n {
            let val = row_values[i].1.as_str();
            if val > hi.as_str() {
                break;
            }
            matched.insert(row_values[i].0);
        }
        matched
    } else {
        // ── 模糊匹配：二分定位 + 双向扩展（与 search_sorted 同构） ──
        let keyword = &keywords[0];

        // 二分定位第一个 >= keyword 的元素
        let mut lo = 0usize;
        let mut hi = n;
        while lo < hi {
            let mid = (lo + hi) / 2;
            if row_values[mid].1.as_str() < keyword.as_str() {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        let mut matched = HashSet::new();

        // 向右扩展
        let mut i = lo;
        while i < n {
            let val = &row_values[i].1;
            if keywords.iter().any(|kw| row_fuzzy_match(val, kw)) {
                matched.insert(row_values[i].0);
            } else if val.as_str() > keyword.as_str() {
                let prefix_len = keyword.len().min(val.len());
                if !val[..prefix_len].starts_with(&keyword[..prefix_len]) {
                    break;
                }
            }
            i += 1;
        }

        // 向左扩展
        if lo > 0 {
            let mut i = lo.saturating_sub(1);
            loop {
                let val = &row_values[i].1;
                if keywords.iter().any(|kw| row_fuzzy_match(val, kw)) {
                    matched.insert(row_values[i].0);
                } else if val.as_str() < keyword.as_str() {
                    let prefix_len = keyword.len().min(val.len());
                    if !keyword[..prefix_len].starts_with(&val[..prefix_len]) {
                        break;
                    }
                }
                if i == 0 {
                    break;
                }
                i -= 1;
            }
        }

        matched
    }
}

/// 执行行筛选搜索（支持多列 AND 逻辑 + 性能优化）
///
/// 自适应选择最优搜索路径：
/// - **二分路径**: 第一列已排序 → O(log n + k×f)（k = 匹配行数）
/// - **并行路径**: 未排序且 >5000 行 → 多线程分块线性扫描
/// - **串行路径**: 未排序且 ≤5000 行 → 常规线性扫描
///
/// 所有路径均使用 P1 预收集消除重复 HashMap 查找。
///
/// 每个筛选器独立解析关键字格式：
/// - 单值: `xxxx` → 模糊匹配
/// - 多值: `'xxx1','xxx2'` → 任一匹配
/// - 范围: `'xxx3'-'xxx4'` → 值在区间内
pub fn execute_row_search(
    state: &mut SearchWindowState,
    sheet: &SheetData,
    hidden_rows: &mut HashSet<u32>,
) {
    hidden_rows.clear();

    if state.row_options.is_empty() {
        state.row_debug_info = "行筛选未配置".to_string();
        return;
    }

    // 收集所有激活的筛选条件（每条引用 row_options 中的一个范围）
    let active_filters: Vec<&RowFilterCondition> = state
        .row_filters
        .iter()
        .filter(|f| f.is_active())
        .collect();

    if active_filters.is_empty() {
        state.row_debug_info = "请输入行筛选关键字".to_string();
        return;
    }

    let max_row = sheet.max_row;
    // start_row 取首个激活条件所选范围的首个锚点行号（以首条件表头行为基准，与旧实现一致）
    let start_row = state
        .row_options
        .get(active_filters[0].range_index)
        .map(|o| o.first_row())
        .unwrap_or(0);

    if max_row <= start_row {
        state.row_total_searched = 0;
        state.row_matched_count = 0;
        state.is_row_searching = true;
        state.row_debug_info = format!("行筛选: 行{}→{} 无数据行可搜索", start_row + 1, max_row);
        return;
    }

    state.row_total_searched = (max_row - start_row) as usize;
    let row_count = state.row_total_searched;

    // ═══ 解析每条条件：锚点列集合（多列=步长段）+ 关键字 + 运算符 ═══
    struct ParsedFilter {
        cols: Vec<u32>,
        keywords: Vec<String>,
        is_range: bool,
        op: CompareOp,
    }

    let parsed: Vec<ParsedFilter> = active_filters
        .iter()
        .map(|f| {
            let (keywords, is_range) = parse_row_keywords(&f.keyword);
            // 锚点列：去重升序（步长段覆盖多列，匹配时取 OR）
            let mut cols: Vec<u32> = state
                .row_options
                .get(f.range_index)
                .map(|o| o.cells.iter().map(|(c, _)| *c).collect())
                .unwrap_or_default();
            cols.sort_unstable();
            cols.dedup();
            ParsedFilter {
                cols,
                keywords,
                is_range,
                op: f.op,
            }
        })
        .collect();

    // ═══ AND/OR 运算符序列（与列搜索一致） ═══
    // active[i].logic 表示条件 i 与下一条的组合逻辑；最后一条无下一条，
    // 其 logic 不参与运算。存在 OR 时整体不再是纯 AND。
    let logic_seq: Vec<FilterLogic> = active_filters.iter().map(|f| f.logic).collect();
    let has_or = logic_seq
        .iter()
        .take(logic_seq.len().saturating_sub(1))
        .any(|&l| l == FilterLogic::Or);

    // ═══ 预计算 cond_match[fi][idx]：条件 fi 在数据行 idx 是否匹配 ═══
    // 多列（步长段）在此一次性收敛为单 bool（任一列命中即 true，OR）。
    // 预计算后并行/串行热循环不再 get_cell，并统一多列处理。
    let cond_match: Vec<Vec<bool>> = parsed
        .iter()
        .map(|pf| {
            (0..row_count)
                .map(|idx| {
                    let row = start_row + 1 + idx as u32;
                    pf.cols.iter().any(|&c| {
                        let value = sheet
                            .get_cell(row, c)
                            .map(|c| cell_search_value(c).to_lowercase())
                            .unwrap_or_default();
                        match_filter_value(&value, &pf.keywords, pf.is_range, pf.op)
                    })
                })
                .collect()
        })
        .collect();

    // 用于诊断信息的搜索模式标签
    let search_mode: &str;

    // ═══ P1: 预收集首列的值（排序检测 + 二分查找）；仅首条件单列时有效 ═══
    let first_single = parsed[0].cols.len() == 1;
    let first_col_data = if first_single {
        collect_column_values(sheet, parsed[0].cols[0], start_row, max_row)
    } else {
        Vec::new()
    };
    // 检测首列是否已排序（单调非递减）；多列时不走二分
    let is_sorted = first_single && first_col_data.windows(2).all(|w| w[0].1 <= w[1].1);
    // 二分查找（find_rows_in_sorted）仅支持「范围」与「包含」两种语义
    let first_binary_compatible = parsed[0].is_range || parsed[0].op == CompareOp::Contains;

    if !has_or && first_single && is_sorted && first_binary_compatible {
        // ══════════ P0: 二分查找路径（纯 AND、首条件单列且已排序） ══════════
        search_mode = "二分";

        // 在首列中二分查找匹配行（候选集）
        let candidate_rows = find_rows_in_sorted(
            &first_col_data,
            &parsed[0].keywords,
            parsed[0].is_range,
        );

        // 候选行之外直接隐藏；候选行用 cond_match 验证其余条件（首条件已由二分保证）
        for (idx, (row, _)) in first_col_data.iter().enumerate() {
            if !candidate_rows.contains(row) {
                hidden_rows.insert(*row);
            } else {
                let all_matched = (1..parsed.len()).all(|fi| cond_match[fi][idx]);
                if !all_matched {
                    hidden_rows.insert(*row);
                }
            }
        }
    } else if row_count > PARALLEL_ROW_THRESHOLD {
        // ══════════ P2: 并行线性扫描路径 ══════════
        search_mode = "并行";

        let num_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let chunk_size = ((row_count + num_threads - 1) / num_threads)
            .max(MIN_ROWS_PER_THREAD);
        let num_chunks = (row_count + chunk_size - 1) / chunk_size;

        // 使用 scope 允许线程借用栈上数据，无需 Arc；cond_match 已预计算，热循环不再 get_cell
        let thread_results: Vec<HashSet<u32>> = std::thread::scope(|s| {
            let mut handles = Vec::with_capacity(num_chunks);
            let cond_match_ref = &cond_match;
            let logic_seq_ref = &logic_seq;
            let use_or = has_or;

            for chunk_idx in 0..num_chunks {
                let start_idx = chunk_idx * chunk_size;
                let end_idx = ((chunk_idx + 1) * chunk_size).min(row_count);

                handles.push(s.spawn(move || {
                    let mut local_hidden = HashSet::new();
                    for idx in start_idx..end_idx {
                        let row = start_row + 1 + idx as u32;
                        let hide = if use_or {
                            // 多条件 AND/OR 组合：按运算符优先级求整体
                            let matches: Vec<bool> =
                                cond_match_ref.iter().map(|cm| cm[idx]).collect();
                            !row_matches_expr(&matches, logic_seq_ref)
                        } else {
                            // 纯 AND：全部匹配才可见
                            !cond_match_ref.iter().all(|cm| cm[idx])
                        };
                        if hide {
                            local_hidden.insert(row);
                        }
                    }
                    local_hidden
                }));
            }

            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });

        for set in thread_results {
            hidden_rows.extend(set);
        }
    } else {
        // ══════════ 串行线性扫描路径 ══════════
        search_mode = "串行";

        for idx in 0..row_count {
            let row = start_row + 1 + idx as u32;
            let hide = if has_or {
                // 多条件 AND/OR 组合：按运算符优先级求整体
                let matches: Vec<bool> = cond_match.iter().map(|cm| cm[idx]).collect();
                !row_matches_expr(&matches, &logic_seq)
            } else {
                // 纯 AND：全部匹配才可见
                !cond_match.iter().all(|cm| cm[idx])
            };
            if hide {
                hidden_rows.insert(row);
            }
        }
    }

    // 处理跨行合并：对每个激活条件的每个锚点列进行合并单元格对齐
    for pf in &parsed {
        for &col in &pf.cols {
            expand_hidden_rows_for_merged_cells(sheet, hidden_rows, col);
        }
    }

    // 确保配置行（锚点行）自身不被隐藏
    for f in &active_filters {
        if let Some(opt) = state.row_options.get(f.range_index) {
            for (_, r) in &opt.cells {
                hidden_rows.remove(r);
            }
        }
    }

    state.row_matched_count = state.row_total_searched.saturating_sub(hidden_rows.len());
    state.is_row_searching = true;

    // 诊断信息（含搜索模式标签）
    let col_labels: Vec<String> = active_filters
        .iter()
        .map(|f| {
            let opt = state.row_options.get(f.range_index);
            let col_letter =
                crate::excel::reader::col_to_letter(opt.map(|o| o.first_col()).unwrap_or(0));
            let title = opt.map(|o| o.title.as_str()).unwrap_or("");
            format!("{}={}", title, col_letter)
        })
        .collect();
    state.row_debug_info = format!(
        "行筛选[{}]: [{}] 行{}→{} 共{}行 | 匹配{}行 隐藏{}行",
        search_mode,
        col_labels.join(", "),
        start_row + 1,
        max_row,
        state.row_total_searched,
        state.row_matched_count,
        hidden_rows.len()
    );
}

/// 合并单元格行可见性对齐（与 expand_hidden_for_merged_cells 对称）
fn expand_hidden_rows_for_merged_cells(
    sheet: &SheetData,
    hidden_rows: &mut HashSet<u32>,
    target_col: u32,
) {
    for mr in &sheet.merged_cells {
        // 只处理跨行合并
        if mr.start_row == mr.end_row {
            continue;
        }
        // 只处理包含目标列的合并
        if target_col < mr.start_col || target_col > mr.end_col {
            continue;
        }
        // 以左上角是否匹配为准
        let top_left_visible = !hidden_rows.contains(&mr.start_row);
        if top_left_visible {
            for r in mr.start_row..=mr.end_row {
                hidden_rows.remove(&r);
            }
        } else {
            for r in mr.start_row..=mr.end_row {
                hidden_rows.insert(r);
            }
        }
    }
}

/// 重置搜索状态并恢复表格显示（关闭弹窗 / 重置按钮共用）
///
/// 清空所有隐藏行列集合、搜索/筛选状态字段、用户输入与筛选条件，
/// 使表格恢复到未搜索前的完整显示状态。遵循 `alert_notify.rs` 的
/// `reset_filter` 模式：关闭与重置共享同一个还原函数。
fn reset_search(
    state: &mut SearchWindowState,
    hidden_columns: &mut HashSet<u32>,
    hidden_rows: &mut HashSet<u32>,
) {
    hidden_columns.clear();
    hidden_rows.clear();
    state.is_searching = false;
    state.matched_count = 0;
    state.total_searched = 0;
    state.search_keyword.clear();
    state.is_row_searching = false;
    state.row_matched_count = 0;
    state.row_total_searched = 0;
    state.debug_info.clear();
    state.row_filters.clear();
    if !state.row_options.is_empty() {
        state.row_filters.push(RowFilterCondition {
            range_index: 0,
            keyword: String::new(),
            logic: FilterLogic::And,
            op: CompareOp::Contains,
        });
    }
    state.row_debug_info.clear();
    state.column_filters.clear();
    state.column_filters.push(ColumnFilter {
        column_index: 0,
        filter_value: String::new(),
        logic: FilterLogic::And,
        op: CompareOp::Contains,
    });
}

/// 执行多条件列筛选（按列值过滤数据行，AND 优先级高于 OR）
///
/// 根据 `column_filters` 中的条件，对每行数据检查指定列的值是否匹配。
/// 遵循 MySQL 一致的运算符优先级规则：
/// - **AND** 优先级高于 **OR**
/// - 将条件按 OR 边界分组，每组内用 AND 取交集，组间取并集
///
/// 例如 `条件1 OR 条件2 AND 条件3` 被解析为 `条件1 OR (条件2 AND 条件3)`。
///
/// 不匹配的行加入 `hidden_rows`。注意：本函数不清空 `hidden_rows`，
/// 调用者需在调用前根据需要清空。
#[allow(dead_code)]
pub fn execute_column_filter(
    state: &SearchWindowState,
    sheet: &SheetData,
    hidden_rows: &mut HashSet<u32>,
) {
    // 收集激活的筛选条件
    let active: Vec<&ColumnFilter> = state
        .column_filters
        .iter()
        .filter(|f| f.is_active())
        .collect();

    if active.is_empty() {
        return;
    }

    let max_row = sheet.max_row;
    let config_rows: HashSet<u32> = state
        .column_options
        .iter()
        .map(|opt| opt.first_row())
        .collect();

    // ═══ AND 优先级高于 OR：按 OR 分组，组内 AND 取交集，组间取并集 ═══
    let mut or_groups: Vec<HashSet<u32>> = Vec::new();
    let mut current_and_group: Option<HashSet<u32>> = None;

    for (i, f) in active.iter().enumerate() {
        // 计算当前条件匹配的行集合
        let mut filter_matches: HashSet<u32> = HashSet::new();
        if f.column_index < state.column_options.len() {
            let col = state.column_options[f.column_index].first_col();
            let keyword = f.filter_value.to_lowercase();
            for row in 1..=max_row {
                if config_rows.contains(&row) {
                    continue;
                }
                let value = sheet
                    .get_cell(row, col)
                    .map(|c| cell_search_value(c).to_lowercase())
                    .unwrap_or_default();
                if value.contains(&keyword) {
                    filter_matches.insert(row);
                }
            }
        }

        // 使用前一条条件的 logic 决定当前条件与前一条的组合方式
        let prev_logic = if i == 0 {
            FilterLogic::And
        } else {
            active[i - 1].logic
        };

        if prev_logic == FilterLogic::And {
            // 第一条条件或 AND 连接：合并到当前 AND 组
            if let Some(ref mut group) = current_and_group {
                // AND：取交集（缩小范围）
                *group = group.intersection(&filter_matches).copied().collect();
            } else {
                current_and_group = Some(filter_matches);
            }
        } else {
            // OR：结束当前 AND 组，开始新的 AND 组
            if let Some(group) = current_and_group.take() {
                or_groups.push(group);
            }
            current_and_group = Some(filter_matches);
        }
    }

    // 将最后一个 AND 组加入 OR 组列表
    if let Some(group) = current_and_group.take() {
        or_groups.push(group);
    }

    // 合并所有 OR 组（并集）
    let mut matched_rows: HashSet<u32> = HashSet::new();
    for group in or_groups {
        matched_rows.extend(group);
    }

    // 未匹配的行加入 hidden_rows
    for row in 1..=max_row {
        if !config_rows.contains(&row) && !matched_rows.contains(&row) {
            hidden_rows.insert(row);
        }
    }
}

/// * `state` - 搜索窗口状态（可变引用）
/// * `excel_data` - Excel 数据（只读引用）
/// * `current_sheet` - 当前工作表索引
/// * `hidden_columns` - 隐藏列集合（可变引用，搜索执行时修改）
pub fn draw_search_window(
    ctx: &egui::Context,
    state: &mut SearchWindowState,
    excel_data: Option<&ExcelData>,
    current_sheet: usize,
    hidden_columns: &mut HashSet<u32>,
    hidden_rows: &mut HashSet<u32>,
) {
    if !state.visible {
        return;
    }

    let mut keep_open = true;

    // ── 展开/折叠动画 ──
    // 与 alert_notify 弹窗一致：用 egui 内置动画器驱动进度，它在动画期间会自动
    // request_repaint，避免「手动逐帧 + egui 休眠」造成的卡顿。
    const ANIM_TIME: f32 = 0.2;
    let target = if state.collapsed { 0.0 } else { 1.0 };
    let p = ctx
        .animate_value_with_time(egui::Id::new("search_window_expand"), target, ANIM_TIME)
        .clamp(0.0, 1.0);

    egui::Window::new("search_window")
        .title_bar(false) // 自定义标题栏
        .open(&mut keep_open)
        .resizable(false)
        .collapsible(false)
        .default_pos(ctx.content_rect().center() - egui::vec2(270.0, 70.0))
        .show(ctx, |ui| {
            ui.set_min_width(520.0);
            ui.set_max_width(520.0);

            // ══════ 自定义标题栏（可点击展开/折叠）══════
            ui.horizontal(|ui| {
                // 展开/折叠箭头：▶ 折叠 / ▼ 展开
                let arrow = if state.collapsed { "▶" } else { "▼" };
                let title_text =
                    egui::RichText::new(format!("{}  搜索", arrow)).size(13.0).strong();
                let title_resp = ui.add(egui::Button::new(title_text).frame(false));
                if title_resp.clicked() {
                    state.collapsed = !state.collapsed;
                }
                title_resp
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(if state.collapsed { "点击展开" } else { "点击折叠" });

                // 右侧按钮（从右到左排列）
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        // 关闭按钮：关闭弹窗并恢复表格到搜索前状态
                        if ui.button("✖").clicked() {
                            reset_search(state, hidden_columns, hidden_rows);
                            state.visible = false;
                        }

                        // 搜索按钮（统一执行列搜索 + 列过滤 + 行筛选）
                        let has_col_filter = excel_data.is_some()
                            && state.column_filters.iter().any(|f| f.is_active());
                        let has_row_input = excel_data.is_some()
                            && state.row_filters.iter().any(|f| f.is_active());
                        if ui
                            .add_enabled(has_col_filter || has_row_input, egui::Button::new("🔍 搜索"))
                            .clicked()
                        {
                            if let Some(data) = excel_data {
                                if let Some(sheet) = data.get_sheet(current_sheet) {
                                    // 第一步：列搜索（多条件 AND/OR 组合，隐藏列）
                                    if has_col_filter {
                                        execute_multi_column_search(state, sheet, hidden_columns);
                                    } else {
                                        hidden_columns.clear();
                                    }
                                    // 第二步：行筛选（独立于列搜索，仅在行筛选有输入时触发）
                                    if has_row_input {
                                        hidden_rows.clear();
                                        execute_row_search(state, sheet, hidden_rows);
                                    }
                                }
                            }
                        }

                        // 重置按钮：清空搜索状态并恢复表格显示
                        if ui.button("🔄 重置").clicked() {
                            reset_search(state, hidden_columns, hidden_rows);
                        }
                    },
                );
            });

            // ══════ 展开内容：折叠时隐藏内容区 ══════
            if p > 0.001 {
                ui.separator();

                // ══════ 内容区 ══════
                // 延迟加载（仅在窗口打开或切换 sheet 后首次渲染时加载）
                if !state.options_loaded {
                    if let Some(data) = excel_data {
                        state.column_options = load_column_options(data, current_sheet);
                        state.options_loaded = true;
                        if !state.column_options.is_empty()
                            && state.selected_index >= state.column_options.len()
                        {
                            state.selected_index = 0;
                        }
                        // 同步加载行筛选可选范围；默认展示第一条范围（动态增删）
                        state.row_options = load_row_options(data, current_sheet);
                        if state.row_filters.is_empty() && !state.row_options.is_empty() {
                            state.row_filters.push(RowFilterCondition {
                                range_index: 0,
                                keyword: String::new(),
                                logic: FilterLogic::And,
                                op: CompareOp::Contains,
                            });
                        }
                    }
                }

                // ══════ 列筛选行（支持多条件动态增删） ══════
                ui.horizontal(|ui| {
                    ui.label("列筛选:");
                    // 统计信息（搜索后显示）
                    if state.is_searching {
                        ui.add_space(16.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "匹配 {}/{} 列",
                                state.matched_count,
                                state.total_searched
                            ))
                            .size(11.0)
                            .color(egui::Color32::from_rgb(0, 130, 0)),
                        );
                        if state.use_binary_search {
                            ui.label(
                                egui::RichText::new("(二分)")
                                    .size(10.0)
                                    .color(egui::Color32::from_rgb(100, 100, 100)),
                            );
                        }
                    }
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if ui.button("添加筛选条件").clicked() {
                                state.column_filters.push(ColumnFilter {
                                    column_index: 0,
                                    filter_value: String::new(),
                                    logic: FilterLogic::And,
                                    op: CompareOp::Contains,
                                });
                            }
                        },
                    );
                });

                // 渲染每条列筛选条件
                {
                    let filter_count = state.column_filters.len();
                    let mut delete_idx = None;

                    for idx in 0..filter_count {
                        ui.horizontal(|ui| {
                            // 列选择下拉框（width=166）
                            let col_sel = state.column_filters[idx].column_index;
                            let selected_text = state.column_options
                                .get(col_sel)
                                .map(|o| o.display())
                                .unwrap_or_else(|| "请选择列...".to_string());
                            egui::ComboBox::from_id_salt(format!("col_filter_select_{}", idx))
                                .selected_text(&selected_text)
                                .width(166.0)
                                .show_ui(ui, |ui| {
                                    for (i, opt) in state.column_options.iter().enumerate() {
                                        let label = opt.display();
                                        if ui
                                            .selectable_label(col_sel == i, &label)
                                            .clicked()
                                        {
                                            state.column_filters[idx].column_index = i;
                                        }
                                    }
                                });

                            ui.add_space(0.0);

                            // 比较运算符下拉框（width=60）
                            let op = state.column_filters[idx].op;
                            egui::ComboBox::from_id_salt(format!("col_filter_op_{}", idx))
                                .selected_text(op.label())
                                .width(60.0)
                                .show_ui(ui, |ui| {
                                    for variant in CompareOp::ALL {
                                        if ui
                                            .selectable_label(op == variant, variant.label())
                                            .clicked()
                                        {
                                            state.column_filters[idx].op = variant;
                                        }
                                    }
                                });

                            ui.add_space(0.0);

                            // 筛选值输入框（desired_width=180）
                            let input = egui::TextEdit::singleline(&mut state.column_filters[idx].filter_value)
                                .desired_width(180.0)
                                .hint_text("输入搜索关键字...");
                            let response = ui.add(input);

                            // Enter 键触发统一搜索（列搜索 + 列过滤 + 行筛选）
                            if ui.input(|i| i.key_pressed(egui::Key::Enter)) && response.has_focus() {
                                if let Some(data) = excel_data {
                                    if let Some(sheet) = data.get_sheet(current_sheet) {
                                        let has_cf = state.column_filters.iter().any(|f| f.is_active());
                                        let has_rf = state.row_filters.iter().any(|f| f.is_active());
                                        // 列搜索（多条件 AND/OR 组合，隐藏列）
                                        if has_cf {
                                            execute_multi_column_search(state, sheet, hidden_columns);
                                        } else {
                                            hidden_columns.clear();
                                        }
                                        // 行筛选（独立于列搜索，仅在行筛选有输入时触发）
                                        if has_rf {
                                            hidden_rows.clear();
                                            execute_row_search(state, sheet, hidden_rows);
                                        }
                                        if has_cf || has_rf {
                                            response.surrender_focus();
                                        }
                                    }
                                }
                            }

                            // AND/OR 选择下拉框（每条条件独立设置，支持混合 AND/OR 表达式）
                            let filter_logic = state.column_filters[idx].logic;
                            egui::ComboBox::from_id_salt(format!("filter_logic_{}", idx))
                                .selected_text(match filter_logic {
                                    FilterLogic::And => "AND",
                                    FilterLogic::Or => "OR",
                                })
                                .width(50.0)
                                .show_ui(ui, |ui| {
                                    if ui.selectable_label(filter_logic == FilterLogic::And, "AND").clicked() {
                                        state.column_filters[idx].logic = FilterLogic::And;
                                    }
                                    if ui.selectable_label(filter_logic == FilterLogic::Or, "OR").clicked() {
                                        state.column_filters[idx].logic = FilterLogic::Or;
                                    }
                                });

                            // 删除按钮（至少保留一条）
                            let can_delete = filter_count > 1;
                            if ui.add_enabled(can_delete, egui::Button::new("X")).clicked() && can_delete {
                                delete_idx = Some(idx);
                            }
                        });

                        // 条件行间添加小间距
                        if idx + 1 < filter_count {
                            ui.add_space(2.0);
                        }
                    }

                    // 延迟执行删除（避免迭代中修改集合导致越界）
                    if let Some(idx) = delete_idx {
                        state.column_filters.remove(idx);
                    }
                }

                // ══════ 行筛选（列筛选下方，动态增删，与列筛选同构） ══════
                if !state.row_options.is_empty() {
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);

                    // ══════ 行筛选标题行（含「添加筛选条件」按钮） ══════
                    ui.horizontal(|ui| {
                        ui.label("行筛选:");
                        // 统计信息（搜索后显示）
                        if state.row_total_searched > 0 {
                            ui.add_space(16.0);
                            ui.label(
                                egui::RichText::new(format!(
                                    "匹配 {}/{} 行",
                                    state.row_matched_count,
                                    state.row_total_searched
                                ))
                                .size(11.0)
                                .color(egui::Color32::from_rgb(0, 130, 0)),
                            );
                        }
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if ui.button("添加筛选条件").clicked() {
                                    state.row_filters.push(RowFilterCondition {
                                        range_index: 0,
                                        keyword: String::new(),
                                        logic: FilterLogic::And,
                                        op: CompareOp::Contains,
                                    });
                                }
                            },
                        );
                    });

                    // 渲染每条行筛选条件
                    {
                        let filter_count = state.row_filters.len();
                        let mut delete_idx = None;

                        for idx in 0..filter_count {
                            ui.horizontal(|ui| {
                                // 行范围选择下拉框（width=166），选项来自 row_options
                                let range_sel = state.row_filters[idx].range_index;
                                let selected_text = state
                                    .row_options
                                    .get(range_sel)
                                    .map(|o| o.display())
                                    .unwrap_or_else(|| "请选择行...".to_string());
                                egui::ComboBox::from_id_salt(format!("row_filter_select_{}", idx))
                                    .selected_text(&selected_text)
                                    .width(166.0)
                                    .show_ui(ui, |ui| {
                                        for (i, opt) in state.row_options.iter().enumerate() {
                                            let label = opt.display();
                                            if ui
                                                .selectable_label(range_sel == i, &label)
                                                .clicked()
                                            {
                                                state.row_filters[idx].range_index = i;
                                            }
                                        }
                                    });

                                ui.add_space(0.0);

                                // 比较运算符下拉框（width=60）
                                let op = state.row_filters[idx].op;
                                egui::ComboBox::from_id_salt(format!("row_filter_op_{}", idx))
                                    .selected_text(op.label())
                                    .width(60.0)
                                    .show_ui(ui, |ui| {
                                        for variant in CompareOp::ALL {
                                            if ui
                                                .selectable_label(op == variant, variant.label())
                                                .clicked()
                                            {
                                                state.row_filters[idx].op = variant;
                                            }
                                        }
                                    });

                                ui.add_space(0.0);

                                // 关键字输入框（desired_width=180）
                                let input =
                                    egui::TextEdit::singleline(&mut state.row_filters[idx].keyword)
                                        .desired_width(180.0)
                                        .hint_text("xxxx 或 'xx1','xx2' 或 'xx3'-'xx4'");
                                let response = ui.add(input);

                                // 输入被清空时自动还原行筛选结果（恢复显示所有行）
                                if response.changed()
                                    && state.row_filters[idx].keyword.trim().is_empty()
                                    && state.is_row_searching
                                {
                                    hidden_rows.clear();
                                    state.is_row_searching = false;
                                    state.row_matched_count = 0;
                                    state.row_total_searched = 0;
                                    state.row_debug_info.clear();
                                }

                                // Enter 键触发统一搜索（列筛选 + 行筛选）
                                if ui.input(|i| i.key_pressed(egui::Key::Enter))
                                    && response.has_focus()
                                {
                                    if let Some(data) = excel_data {
                                        if let Some(sheet) = data.get_sheet(current_sheet) {
                                            let has_col =
                                                state.column_filters.iter().any(|f| f.is_active());
                                            let has_row =
                                                state.row_filters.iter().any(|f| f.is_active());
                                            // 列搜索（多条件 AND/OR 组合，隐藏列）
                                            if has_col {
                                                execute_multi_column_search(
                                                    state,
                                                    sheet,
                                                    hidden_columns,
                                                );
                                            } else {
                                                hidden_columns.clear();
                                            }
                                            if has_row {
                                                hidden_rows.clear();
                                                execute_row_search(state, sheet, hidden_rows);
                                            }
                                            if has_col || has_row {
                                                response.surrender_focus();
                                            }
                                        }
                                    }
                                }

                                // AND/OR 选择下拉框（每条条件独立设置，支持混合 AND/OR 表达式）
                                let filter_logic = state.row_filters[idx].logic;
                                egui::ComboBox::from_id_salt(format!("row_filter_logic_{}", idx))
                                    .selected_text(match filter_logic {
                                        FilterLogic::And => "AND",
                                        FilterLogic::Or => "OR",
                                    })
                                    .width(50.0)
                                    .show_ui(ui, |ui| {
                                        if ui
                                            .selectable_label(
                                                filter_logic == FilterLogic::And,
                                                "AND",
                                            )
                                            .clicked()
                                        {
                                            state.row_filters[idx].logic = FilterLogic::And;
                                        }
                                        if ui
                                            .selectable_label(filter_logic == FilterLogic::Or, "OR")
                                            .clicked()
                                        {
                                            state.row_filters[idx].logic = FilterLogic::Or;
                                        }
                                    });

                                // 删除按钮（至少保留一条）
                                let can_delete = filter_count > 1;
                                if ui.add_enabled(can_delete, egui::Button::new("X")).clicked()
                                    && can_delete
                                {
                                    delete_idx = Some(idx);
                                }
                            });

                            // 条件行间添加小间距
                            if idx + 1 < filter_count {
                                ui.add_space(2.0);
                            }
                        }

                        // 延迟执行删除（避免迭代中修改集合导致越界）
                        if let Some(idx) = delete_idx {
                            state.row_filters.remove(idx);
                        }
                    }

                    // 行筛选诊断
                    if state.is_row_searching && !state.row_debug_info.is_empty() {
                        ui.add_space(2.0);
                        ui.label(
                            egui::RichText::new(&state.row_debug_info)
                                .size(10.0)
                                .color(egui::Color32::from_rgb(0, 100, 0)),
                        );
                    }
                }

                // 诊断信息（搜索后显示，帮助排查搜索问题）
                if state.is_searching && !state.debug_info.is_empty() {
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new(&state.debug_info)
                            .size(10.0)
                            .color(egui::Color32::from_rgb(100, 100, 100)),
                    );
                }

                // 底部提示
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "💡 搜索选中列右侧所有列；已排序数据自动启用二分查找",
                    )
                    .size(10.0)
                    .color(egui::Color32::from_rgb(140, 140, 140)),
                );
            }
        });

    // 窗口关闭（egui 内置关闭机制，如按 Escape）：恢复表格到搜索前状态
    if !keep_open {
        reset_search(state, hidden_columns, hidden_rows);
        state.visible = false;
    }
}

// ═══════════════════════════════════════════════════════════════
// 单元测试
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_cell_ref / parse_cell_ref_bound ──

    #[test]
    fn single_cell_ref() {
        assert_eq!(parse_cell_ref("A1"), Some((1, 1)));
        assert_eq!(parse_cell_ref("B14"), Some((2, 14)));
        assert_eq!(parse_cell_ref("AA100"), Some((27, 100)));
        assert_eq!(parse_cell_ref(""), None);
        assert_eq!(parse_cell_ref("A"), None);
        assert_eq!(parse_cell_ref("1"), None);
    }

    #[test]
    fn cell_ref_bound_no_tilde() {
        assert_eq!(parse_cell_ref_bound("A1", 10, 20), Some((1, 1)));
        assert_eq!(parse_cell_ref_bound("B14", 10, 20), Some((2, 14)));
    }

    #[test]
    fn cell_ref_bound_tilde_last_col() {
        assert_eq!(parse_cell_ref_bound("~14", 10, 20), Some((10, 14)));
        assert_eq!(parse_cell_ref_bound("~1", 5, 100), Some((5, 1)));
        assert_eq!(parse_cell_ref_bound("~0", 10, 20), None); // row 0 invalid
    }

    #[test]
    fn cell_ref_bound_tilde_last_row() {
        assert_eq!(parse_cell_ref_bound("A~", 10, 20), Some((1, 20)));
        assert_eq!(parse_cell_ref_bound("Z~", 10, 50), Some((26, 50)));
    }

    #[test]
    fn cell_ref_bound_tilde_both() {
        assert_eq!(parse_cell_ref_bound("~", 10, 20), Some((10, 20)));
        assert_eq!(parse_cell_ref_bound("~", 0, 0), Some((1, 1))); // .max(1)
    }

    // ── parse_search_segments ──

    fn seg_cells(segs: &[RangeSegment]) -> Vec<Vec<(u32, u32)>> {
        segs.iter().map(|s| s.cells.clone()).collect()
    }

    fn seg_exprs(segs: &[RangeSegment]) -> Vec<&str> {
        segs.iter().map(|s| s.expr.as_str()).collect()
    }

    fn seg_is_step(segs: &[RangeSegment]) -> Vec<bool> {
        segs.iter().map(|s| s.is_step).collect()
    }

    const MAX: (u32, u32) = (10, 20);

    #[test]
    fn flat_single_cell() {
        let segs = parse_search_segments("A14", MAX.0, MAX.1);
        assert_eq!(seg_cells(&segs), vec![vec![(1, 14)]]);
        assert!(!segs[0].is_step);
    }

    #[test]
    fn flat_range_same_col() {
        // A1-A3 → 3 flat segments, each 1 cell
        let segs = parse_search_segments("A1-A3", MAX.0, MAX.1);
        assert_eq!(seg_cells(&segs), vec![vec![(1, 1)], vec![(1, 2)], vec![(1, 3)]]);
        assert_eq!(seg_is_step(&segs), vec![false, false, false]);
    }

    #[test]
    fn flat_range_same_row() {
        // A1-C1 → 3 flat segments
        let segs = parse_search_segments("A1-C1", MAX.0, MAX.1);
        assert_eq!(seg_cells(&segs), vec![vec![(1, 1)], vec![(2, 1)], vec![(3, 1)]]);
    }

    #[test]
    fn flat_comma_separated() {
        let segs = parse_search_segments("A14,D14", MAX.0, MAX.1);
        assert_eq!(seg_cells(&segs), vec![vec![(1, 14)], vec![(4, 14)]]);
    }

    #[test]
    fn step_vertical_with_end() {
        // A(1:+2):A13 → 1 step segment, 7 cells (col A rows 1,3,5,7,9,11,13)
        let segs = parse_search_segments("A(1:+2):A13", MAX.0, MAX.1);
        assert_eq!(seg_cells(&segs), vec![vec![(1, 1), (1, 3), (1, 5), (1, 7), (1, 9), (1, 11), (1, 13)]]);
        assert!(segs[0].is_step);
    }

    #[test]
    fn step_vertical_no_end() {
        // A(1:+2) → steps to max_row (20)
        let segs = parse_search_segments("A(1:+2)", MAX.0, MAX.1);
        let cells = &segs[0].cells;
        assert!(segs[0].is_step);
        assert_eq!(cells[0], (1, 1));
        assert_eq!(*cells.last().unwrap(), (1, 19)); // 1,3,5,7,9,11,13,15,17,19
        assert_eq!(cells.len(), 10);
    }

    #[test]
    fn step_vertical_aa_col() {
        // AA(1:+2):AA7 → col 27, rows 1..7 step 2
        let segs = parse_search_segments("AA(1:+2):AA7", MAX.0, MAX.1);
        assert_eq!(seg_cells(&segs), vec![vec![(27, 1), (27, 3), (27, 5), (27, 7)]]);
        assert!(segs[0].is_step);
    }

    #[test]
    fn step_horizontal_with_end() {
        // (B:+2)14:~14 → row 14, cols B, D, F (last=G=7)
        let segs = parse_search_segments("(B:+2)14:~14", 7, MAX.1);
        assert_eq!(seg_cells(&segs), vec![vec![(2, 14), (4, 14), (6, 14)]]);
        assert!(segs[0].is_step);
    }

    #[test]
    fn step_horizontal_no_end() {
        // (B:+2)14 → steps to max_col (10)
        let segs = parse_search_segments("(B:+2)14", MAX.0, MAX.1);
        let cells = &segs[0].cells;
        assert!(segs[0].is_step);
        assert_eq!(cells[0], (2, 14));
        assert_eq!(*cells.last().unwrap(), (10, 14)); // B,D,F,H,J (2,4,6,8,10)
        assert_eq!(cells.len(), 5);
    }

    #[test]
    fn step_horizontal_tilde_explicit_end() {
        // (B:+2)14:F14 → row 14, cols B, D, F (end=F=6)
        let segs = parse_search_segments("(B:+2)14:F14", MAX.0, MAX.1);
        assert_eq!(seg_cells(&segs), vec![vec![(2, 14), (4, 14), (6, 14)]]);
        assert!(segs[0].is_step);
    }

    #[test]
    fn step_mixed_with_flat() {
        let segs = parse_search_segments("A14,(B:+2)14:~14", 7, MAX.1);
        assert_eq!(seg_cells(&segs), vec![vec![(1, 14)], vec![(2, 14), (4, 14), (6, 14)]]);
        assert_eq!(seg_is_step(&segs), vec![false, true]);
    }

    #[test]
    fn step_zero_rejected() {
        let segs = parse_search_segments("A(1:+0):A13", MAX.0, MAX.1);
        assert!(segs.is_empty());
    }

    #[test]
    fn malformed_step_falls_through() {
        // "(A:+2)14" — inner has letters but before="A" not empty → vertical branch
        // tries parse on left="A" as u32 → fails → falls through. "" is returned
        let segs = parse_search_segments("(A:+2)14", MAX.0, MAX.1);
        // before empty? "(A:+2)14": before="" inner="A:+2", left="A" (alpha), row=14, start_col=A=1, end=none→max_col=10. So it IS a valid horizontal step!
        // Actually: before="" (empty), inner="A:+2", left="A" is alpha, tail="14", prefix="14", end=None → horizontal step
        assert_eq!(segs.len(), 1, "should parse as horizontal step");
        assert!(segs[0].is_step);
    }

    #[test]
    fn step_invalid_nested_parens_skipped() {
        // Nested parens: just try to parse, if fails returns empty
        let segs = parse_search_segments("A((1:+2):A13", MAX.0, MAX.1);
        // Step parser expects clean inner; inner="(1:+2" → find ':' in inner → "..." doesn't make sense → None
        assert!(segs.is_empty() || segs.iter().all(|s| !s.is_step));
    }

    #[test]
    fn empty_input() {
        let segs = parse_search_segments("", MAX.0, MAX.1);
        assert!(segs.is_empty());
        let segs = parse_search_segments("   ", MAX.0, MAX.1);
        assert!(segs.is_empty());
    }

    #[test]
    fn flat_range_with_tilde_end() {
        // A1-A~ → flat range from row 1 to max_row (20)
        let segs = parse_search_segments("A1-A~", MAX.0, MAX.1);
        assert_eq!(segs.len(), 20);
        assert_eq!(segs[0].cells, vec![(1, 1)]);
        assert_eq!(segs[19].cells, vec![(1, 20)]);
    }

    // ── RangeOption::display ──

    #[test]
    fn range_option_display_normal() {
        let opt = RangeOption {
            title: "入库".into(),
            cell_ref: "A14".into(),
            cells: vec![(1, 14)],
            is_step: false,
            expr: "A14".into(),
        };
        assert_eq!(opt.display(), "入库 (A14)");
    }

    #[test]
    fn range_option_display_step() {
        let opt = RangeOption {
            title: "入库".into(),
            cell_ref: "B14".into(),
            cells: vec![(2, 14), (4, 14), (6, 14)],
            is_step: true,
            expr: "(B:+2)14:~14".into(),
        };
        assert_eq!(opt.display(), "入库((B:+2)14:~14)");
    }

    #[test]
    fn range_option_first_col_row() {
        let opt = RangeOption {
            title: "".into(),
            cell_ref: "".into(),
            cells: vec![(3, 5), (3, 7)],
            is_step: true,
            expr: "".into(),
        };
        assert_eq!(opt.first_col(), 3);
        assert_eq!(opt.first_row(), 5);
    }

    #[test]
    fn range_option_empty_cells() {
        let opt = RangeOption {
            title: "".into(),
            cell_ref: "".into(),
            cells: vec![],
            is_step: false,
            expr: "".into(),
        };
        assert_eq!(opt.first_col(), 0);
        assert_eq!(opt.first_row(), 0);
    }

    // ── parse_cell_ref_bound edges ──

    #[test]
    fn bound_weird_ab_form() {
        // A~B — falls through to parse_cell_ref, row_part "~B" fails parse → None
        assert_eq!(parse_cell_ref_bound("A~B", 10, 20), None);
    }
}
