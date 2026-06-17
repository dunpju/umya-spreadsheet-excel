//! 搜索窗口组件
//!
//! 提供列筛选搜索功能：从配置文件读取可选的列范围，通过模糊匹配
//! 隐藏不匹配的列，支持合并单元格跨列感知。

use eframe::egui;
use std::collections::HashSet;
use crate::excel::reader::{CellData, ExcelData, SheetData};

// ═══════════════════════════════════════════════════════════════
// 性能优化参数
// ═══════════════════════════════════════════════════════════════

/// 并行行扫描阈值：超过此行数时启用多线程线性扫描
const PARALLEL_ROW_THRESHOLD: usize = 5000;
/// 每个线程最少处理的行数（避免线程粒度过细）
const MIN_ROWS_PER_THREAD: usize = 2000;

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

/// 单个下拉选项：title = 单元格显示值, cell_ref = 单元格坐标（如 "A1"）
#[derive(Debug, Clone)]
pub struct SearchColumnOption {
    /// 显示文本：单元格的值，如 "序号"、"名称"
    pub title: String,
    /// 单元格引用字符串，如 "A1"、"A2"
    pub cell_ref: String,
    /// 列号（1-based）
    pub col: u32,
    /// 行号（1-based）
    pub row: u32,
}

/// 单个行筛选配置项
///
/// 每个配置项对应 search.row 中一个单元格引用，包含
/// 标题（单元格值）、坐标和用户输入的关键字。
#[derive(Debug, Clone)]
pub struct RowFilterState {
    /// 显示标题：单元格的值，如 "日期"、"入库"
    pub title: String,
    /// 单元格引用字符串，如 "A14"、"D14"
    pub cell_ref: String,
    /// 列号（1-based）
    pub col: u32,
    /// 行号（1-based），搜索从此行+1开始向下
    pub row: u32,
    /// 用户输入的关键字
    pub keyword: String,
}

impl RowFilterState {
    /// 该筛选项是否激活（用户输入了关键字）
    pub fn is_active(&self) -> bool {
        !self.keyword.is_empty()
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
    /// 该条件与前面条件的组合逻辑（And 取交集，Or 取并集）
    pub logic: FilterLogic,
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

/// 搜索窗口状态
#[derive(Debug)]
pub struct SearchWindowState {
    // ========== 窗口控制 ==========
    /// 搜索窗口是否可见
    pub visible: bool,

    // ========== 下拉框数据 ==========
    /// 从配置 + 单元格数据解析出的下拉选项列表
    pub column_options: Vec<SearchColumnOption>,
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

    // ========== 行筛选（支持多列） ==========
    /// 行筛选配置列表（从 search.row 解析的每个单元格对应一项）
    pub row_filters: Vec<RowFilterState>,
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
            column_options: Vec::new(),
            selected_index: 0,
            options_loaded: false,
            search_keyword: String::new(),
            is_searching: false,
            matched_count: 0,
            total_searched: 0,
            use_binary_search: false,
            debug_info: String::new(),
            row_filters: Vec::new(),
            is_row_searching: false,
            row_matched_count: 0,
            row_total_searched: 0,
            row_debug_info: String::new(),
            column_filters: vec![ColumnFilter {
                column_index: 0,
                filter_value: String::new(),
                logic: FilterLogic::And,
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

/// 解析单个段：可以是单格 "A1" 或范围 "A1-A13"
fn parse_one_segment(s: &str) -> Vec<(u32, u32)> {
    let s = s.trim();
    if s.is_empty() {
        return Vec::new();
    }

    // 尝试范围格式 "A1-A13"
    if let Some(idx) = s.find('-') {
        let after_dash = &s[idx + 1..];
        if after_dash.chars().next().map_or(false, |c| c.is_alphabetic()) {
            let start_str = &s[..idx];
            let end_str = after_dash;
            if let (Some(start), Some(end)) = (parse_cell_ref(start_str), parse_cell_ref(end_str)) {
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
    parse_cell_ref(s).into_iter().collect()
}

/// 解析搜索范围字符串
///
/// 支持混合格式：
/// - 范围格式: "A1-A13" → [(1,1), (1,2), ..., (1,13)]
/// - 离散格式: "A1,A3,B5" → [(1,1), (1,3), (2,5)]
/// - 混合格式: "A1-A13,A15" → [(1,1), ..., (1,13), (1,15)]
pub fn parse_search_range(input: &str) -> Vec<(u32, u32)> {
    let input = input.trim();
    if input.is_empty() {
        return Vec::new();
    }

    // 逗号分隔：每段独立解析
    if input.contains(',') {
        return input
            .split(',')
            .flat_map(|s| parse_one_segment(s))
            .collect();
    }

    // 无逗号：整段解析
    parse_one_segment(input)
}

/// 从配置文件和 Excel 数据加载下拉选项
pub fn load_column_options(
    excel_data: &ExcelData,
    current_sheet: usize,
) -> Vec<SearchColumnOption> {
    let sheet = match excel_data.get_sheet(current_sheet) {
        Some(s) => s,
        None => return Vec::new(),
    };

    // 读取配置
    let path = config_path();
    let range_str = if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|c| serde_yaml::from_str::<serde_yaml::Value>(&c).ok())
            .and_then(|doc| {
                doc.get("search")
                    .and_then(|s| s.get("column"))
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    if range_str.is_empty() {
        return Vec::new();
    }

    // 解析范围字符串
    let cells = parse_search_range(&range_str);

    // 读取每个单元格的值构建选项
    cells
        .into_iter()
        .map(|(col, row)| {
            let title = sheet
                .get_cell(row, col)
                .map(|c| cell_search_value(c))
                .unwrap_or_default();
            let col_letter = crate::excel::reader::col_to_letter(col);
            let cell_ref = format!("{}{}", col_letter, row);
            SearchColumnOption {
                title,
                cell_ref,
                col,
                row,
            }
        })
        .collect()
}

/// 合并单元格列可见性对齐
///
/// 对于跨列合并：左上角单元格的值代表整个合并区域。
/// - 左上角匹配（不在隐藏集）→ 整个合并范围的所有列都设为可见
/// - 左上角不匹配（在隐藏集中）→ 整个合并范围的所有列都隐藏
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

/// 执行搜索操作
///
/// 在选中列所在行的右侧所有列中进行模糊匹配，
/// 不匹配的列将被加入 hidden_columns 集合。
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
    let target_col = opt.col;
    let target_row = opt.row;
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

/// 加载行筛选配置列表
///
/// 读取 my-excel.yaml 中 search.row，使用 parse_search_range 解析所有单元格引用，
/// 为每个单元格构建一个 RowFilterState。
///
/// 支持格式：
/// - 离散: "A14,D14" → 两个独立的筛选项
/// - 范围: "A14-D14" → A14, B14, C14, D14 四个筛选项
/// - 混合: "A14,D14-F14" → A14 + D14, E14, F14
pub fn load_row_filter_configs(
    excel_data: &ExcelData,
    current_sheet: usize,
) -> Vec<RowFilterState> {
    let sheet = match excel_data.get_sheet(current_sheet) {
        Some(s) => s,
        None => return Vec::new(),
    };

    let path = config_path();
    let range_str = if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|c| serde_yaml::from_str::<serde_yaml::Value>(&c).ok())
            .and_then(|doc| {
                doc.get("search")
                    .and_then(|s| s.get("row"))
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    if range_str.is_empty() {
        return Vec::new();
    }

    // 复用 parse_search_range 解析所有单元格引用
    let cells = parse_search_range(&range_str);

    cells
        .into_iter()
        .map(|(col, row)| {
            let title = sheet
                .get_cell(row, col)
                .map(|c| cell_search_value(c))
                .unwrap_or_default();
            let col_letter = crate::excel::reader::col_to_letter(col);
            let cell_ref = format!("{}{}", col_letter, row);
            RowFilterState {
                title,
                cell_ref,
                col,
                row,
                keyword: String::new(),
            }
        })
        .collect()
}

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

/// 检查单个值是否匹配筛选条件（统一范围匹配和模糊匹配逻辑）
fn match_filter_value(value: &str, keywords: &[String], is_range: bool) -> bool {
    if is_range && keywords.len() == 2 {
        let v = value.trim();
        v >= keywords[0].as_str() && v <= keywords[1].as_str()
    } else {
        keywords
            .iter()
            .any(|kw| row_fuzzy_match(value, kw))
    }
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

    if state.row_filters.is_empty() {
        state.row_debug_info = "行筛选未配置".to_string();
        return;
    }

    // 收集所有激活的筛选器
    let active_filters: Vec<&RowFilterState> = state
        .row_filters
        .iter()
        .filter(|f| f.is_active())
        .collect();

    if active_filters.is_empty() {
        state.row_debug_info = "请输入行筛选关键字".to_string();
        return;
    }

    let max_row = sheet.max_row;
    let start_row = active_filters[0].row;

    if max_row <= start_row {
        state.row_total_searched = 0;
        state.row_matched_count = 0;
        state.is_row_searching = true;
        state.row_debug_info = format!("行筛选: 行{}→{} 无数据行可搜索", start_row + 1, max_row);
        return;
    }

    state.row_total_searched = (max_row - start_row) as usize;
    let row_count = state.row_total_searched;

    // ═══ 解析所有筛选器关键字（owned 数据，线程安全） ═══
    struct ParsedFilter {
        col: u32,
        keywords: Vec<String>,
        is_range: bool,
    }

    let parsed: Vec<ParsedFilter> = active_filters
        .iter()
        .map(|f| {
            let (keywords, is_range) = parse_row_keywords(&f.keyword);
            ParsedFilter {
                col: f.col,
                keywords,
                is_range,
            }
        })
        .collect();

    // 用于诊断信息的搜索模式标签
    let search_mode: &str;

    // ═══ P1: 预收集第一列的值（用于排序检测 + 二分查找） ═══
    let first_col_data = collect_column_values(sheet, parsed[0].col, start_row, max_row);

    // 检测第一列是否已排序（单调非递减）
    let is_sorted = first_col_data
        .windows(2)
        .all(|w| w[0].1 <= w[1].1);

    if is_sorted {
        // ══════════ P0: 二分查找路径 ══════════
        search_mode = "二分";

        // 在第一列中二分查找匹配行
        let candidate_rows = find_rows_in_sorted(
            &first_col_data,
            &parsed[0].keywords,
            parsed[0].is_range,
        );

        if parsed.len() == 1 {
            // 仅一个筛选器：候选行之外的全部隐藏
            for (row, _) in &first_col_data {
                if !candidate_rows.contains(row) {
                    hidden_rows.insert(*row);
                }
            }
        } else {
            // 多个筛选器：候选行需验证其余列，非候选行直接隐藏
            for (row, _) in &first_col_data {
                if !candidate_rows.contains(row) {
                    hidden_rows.insert(*row);
                } else {
                    // 验证其余筛选器（按需 get_cell，候选集已被二分缩小）
                    let all_matched = parsed[1..].iter().all(|pf| {
                        let value = sheet
                            .get_cell(*row, pf.col)
                            .map(|c| cell_search_value(c).to_lowercase())
                            .unwrap_or_default();
                        match_filter_value(&value, &pf.keywords, pf.is_range)
                    });
                    if !all_matched {
                        hidden_rows.insert(*row);
                    }
                }
            }
        }
    } else if row_count > PARALLEL_ROW_THRESHOLD {
        // ══════════ P2: 并行线性扫描路径 ══════════
        search_mode = "并行";

        // P1: 预收集所有筛选器的列值
        let all_col_data: Vec<Vec<(u32, String)>> = parsed
            .iter()
            .map(|pf| collect_column_values(sheet, pf.col, start_row, max_row))
            .collect();

        let num_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let chunk_size = ((row_count + num_threads - 1) / num_threads)
            .max(MIN_ROWS_PER_THREAD);
        let num_chunks = (row_count + chunk_size - 1) / chunk_size;

        // 使用 scope 允许线程借用栈上数据，无需 Arc
        let thread_results: Vec<HashSet<u32>> = std::thread::scope(|s| {
            let mut handles = Vec::with_capacity(num_chunks);

            for chunk_idx in 0..num_chunks {
                let start_idx = chunk_idx * chunk_size;
                let end_idx = ((chunk_idx + 1) * chunk_size).min(row_count);

                // 显式捕获引用（move 闭包中引用是 Copy，可安全传递到线程）
                let all_col_data_ref = &all_col_data;
                let parsed_ref = &parsed;

                handles.push(s.spawn(move || {
                    let mut local_hidden = HashSet::new();
                    for idx in start_idx..end_idx {
                        let row = all_col_data_ref[0][idx].0;
                        let all_matched = parsed_ref.iter().enumerate().all(|(fi, pf)| {
                            let value = &all_col_data_ref[fi][idx].1;
                            match_filter_value(value, &pf.keywords, pf.is_range)
                        });
                        if !all_matched {
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

        // P1: 预收集所有筛选器的列值
        let all_col_data: Vec<Vec<(u32, String)>> = parsed
            .iter()
            .map(|pf| collect_column_values(sheet, pf.col, start_row, max_row))
            .collect();

        for idx in 0..row_count {
            let row = all_col_data[0][idx].0;
            let all_matched = parsed.iter().enumerate().all(|(fi, pf)| {
                let value = &all_col_data[fi][idx].1;
                match_filter_value(value, &pf.keywords, pf.is_range)
            });
            if !all_matched {
                hidden_rows.insert(row);
            }
        }
    }

    // 处理跨行合并：对每个激活筛选器的列进行合并单元格对齐
    for pf in &parsed {
        expand_hidden_rows_for_merged_cells(sheet, hidden_rows, pf.col);
    }

    // 确保配置行自身不被隐藏
    for f in &active_filters {
        hidden_rows.remove(&f.row);
    }

    state.row_matched_count = state.row_total_searched.saturating_sub(hidden_rows.len());
    state.is_row_searching = true;

    // 诊断信息（含搜索模式标签）
    let col_labels: Vec<String> = active_filters
        .iter()
        .map(|f| {
            let col_letter = crate::excel::reader::col_to_letter(f.col);
            format!("{}={}", f.title, col_letter)
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

/// 执行多条件列筛选（按列值过滤数据行）
///
/// 根据 `column_filters` 中的条件，对每行数据检查指定列的值是否匹配。
/// 使用 `filter_logic` 决定多条件间的组合方式：
/// - **And**: 所有激活条件均匹配时，该行才可见
/// - **Or**: 任一激活条件匹配时，该行即可见
///
/// 不匹配的行加入 `hidden_rows`。注意：本函数不清空 `hidden_rows`，
/// 调用者需在调用前根据需要清空。
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
        .map(|opt| opt.row)
        .collect();

    // 逐条顺序组合：第一条收集匹配行，后续 AND 取交集、OR 取并集
    let mut matched_rows: HashSet<u32> = HashSet::new();

    for (i, f) in active.iter().enumerate() {
        // 计算当前条件匹配的行集合
        let mut filter_matches: HashSet<u32> = HashSet::new();
        if f.column_index < state.column_options.len() {
            let col = state.column_options[f.column_index].col;
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

        // 与已累积结果组合
        if i == 0 {
            // 第一条：直接作为初始集合
            matched_rows = filter_matches;
        } else if f.logic == FilterLogic::And {
            // AND：取交集（缩小范围）
            matched_rows = matched_rows
                .intersection(&filter_matches)
                .copied()
                .collect();
        } else {
            // OR：取并集（扩大范围）
            matched_rows.extend(filter_matches);
        }
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
    egui::Window::new("search_window")
        .title_bar(false) // 自定义标题栏
        .open(&mut keep_open)
        .resizable(false)
        .collapsible(false)
        .default_pos(ctx.content_rect().center() - egui::vec2(210.0, 70.0))
        .show(ctx, |ui| {
            ui.set_min_width(440.0);
            ui.set_max_width(440.0);

            // ══════ 自定义标题栏 ══════
            ui.horizontal(|ui| {
                // 标题
                ui.label(egui::RichText::new("搜索").size(13.0).strong());

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

                // 右侧按钮（从右到左排列）
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        // 关闭按钮
                        if ui.button("✖").clicked() {
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
                                    // 第一步：列搜索（由第一个激活的列筛选条件驱动，隐藏列）
                                    if let Some(first) = state.column_filters.iter().find(|f| f.is_active()) {
                                        state.selected_index = first.column_index;
                                        state.search_keyword = first.filter_value.clone();
                                        execute_search(state, sheet, hidden_columns);
                                    } else {
                                        hidden_columns.clear();
                                    }
                                    // 第二步：行过滤（清空后依次叠加行筛选 + 列过滤）
                                    hidden_rows.clear();
                                    if has_row_input {
                                        execute_row_search(state, sheet, hidden_rows);
                                    }
                                    if has_col_filter {
                                        execute_column_filter(state, sheet, hidden_rows);
                                    }
                                }
                            }
                        }

                        // 重置按钮（统一清空列搜索 + 列过滤 + 行筛选）
                        if ui.button("🔄 重置").clicked() {
                            hidden_columns.clear();
                            hidden_rows.clear();
                            state.is_searching = false;
                            state.matched_count = 0;
                            state.total_searched = 0;
                            state.search_keyword.clear();
                            state.is_row_searching = false;
                            state.row_matched_count = 0;
                            state.row_total_searched = 0;
                            for f in &mut state.row_filters {
                                f.keyword.clear();
                            }
                            state.row_debug_info.clear();
                            // 清空列筛选条件（保留一条空条件）
                            state.column_filters.clear();
                            state.column_filters.push(ColumnFilter {
                                column_index: 0,
                                filter_value: String::new(),
                                logic: FilterLogic::And,
                            });
                        }
                    },
                );
            });
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
                    // 同步加载行筛选配置（支持多单元格引用）
                    state.row_filters = load_row_filter_configs(data, current_sheet);
                }
            }

            // ══════ 列筛选行（支持多条件动态增删） ══════
            ui.horizontal(|ui| {
                ui.label("列筛选:");
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui.button("添加筛选条件").clicked() {
                            state.column_filters.push(ColumnFilter {
                                column_index: 0,
                                filter_value: String::new(),
                                logic: FilterLogic::And,
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
                            .map(|o| format!("{} ({})", o.title, o.cell_ref))
                            .unwrap_or_else(|| "请选择列...".to_string());
                        egui::ComboBox::from_id_salt(format!("col_filter_select_{}", idx))
                            .selected_text(&selected_text)
                            .width(166.0)
                            .show_ui(ui, |ui| {
                                for (i, opt) in state.column_options.iter().enumerate() {
                                    let label = format!("{} ({})", opt.title, opt.cell_ref);
                                    if ui
                                        .selectable_label(col_sel == i, &label)
                                        .clicked()
                                    {
                                        state.column_filters[idx].column_index = i;
                                    }
                                }
                            });

                        ui.add_space(2.0);

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
                                    // 列搜索（由第一个激活条件驱动）
                                    if let Some(first) = state.column_filters.iter().find(|f| f.is_active()) {
                                        state.selected_index = first.column_index;
                                        state.search_keyword = first.filter_value.clone();
                                        execute_search(state, sheet, hidden_columns);
                                    } else {
                                        hidden_columns.clear();
                                    }
                                    // 行过滤
                                    hidden_rows.clear();
                                    if has_rf {
                                        execute_row_search(state, sheet, hidden_rows);
                                    }
                                    if has_cf {
                                        execute_column_filter(state, sheet, hidden_rows);
                                    }
                                    if has_cf || has_rf {
                                        response.surrender_focus();
                                    }
                                }
                            }
                        }

                        // AND/OR 选择下拉框（每条条件独立）
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

            // ══════ 行筛选（列筛选下方，支持多列） ══════
            if !state.row_filters.is_empty() {
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                // 为每个行筛选项渲染一个输入行
                let filter_count = state.row_filters.len();
                for idx in 0..filter_count {
                    let title = state.row_filters[idx].title.clone();
                    ui.horizontal(|ui| {
                        ui.label(format!("{} ({}):", title, state.row_filters[idx].cell_ref));
                        let input = egui::TextEdit::singleline(&mut state.row_filters[idx].keyword)
                            .desired_width(f32::INFINITY)
                            .hint_text("xxxx 或 'xx1','xx2' 或 'xx3'-'xx4'");
                        let response = ui.add(input);

                        // Enter 键触发统一搜索（列筛选 + 行筛选）
                        if ui.input(|i| i.key_pressed(egui::Key::Enter)) && response.has_focus() {
                            if let Some(data) = excel_data {
                                if let Some(sheet) = data.get_sheet(current_sheet) {
                                    let has_col = state.selected_index < state.column_options.len()
                                        && !state.search_keyword.is_empty();
                                    let has_row = state.row_filters.iter().any(|f| f.is_active());
                                    if has_col {
                                        execute_search(state, sheet, hidden_columns);
                                    } else {
                                        hidden_columns.clear();
                                    }
                                    if has_row {
                                        execute_row_search(state, sheet, hidden_rows);
                                    } else {
                                        hidden_rows.clear();
                                    }
                                    if has_col || has_row {
                                        response.surrender_focus();
                                    }
                                }
                            }
                        }
                    });

                    // 行间添加小间距
                    if idx + 1 < filter_count {
                        ui.add_space(2.0);
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
        });

    // 窗口关闭
    if !keep_open {
        state.visible = false;
    }
}
