//! 搜索窗口组件
//!
//! 提供列筛选搜索功能：从配置文件读取可选的列范围，通过模糊匹配
//! 隐藏不匹配的列，支持合并单元格跨列感知。

use eframe::egui;
use std::collections::HashSet;
use crate::excel::reader::{CellData, ExcelData, SheetData};

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

    // ========== 行筛选 ==========
    /// 行筛选标题（从配置单元格读取的值，如 "日期"）
    pub row_title: String,
    /// 行筛选配置指向的列号（1-based）
    pub row_search_col: u32,
    /// 行筛选配置指向的行号（搜索从此行+1开始向下）
    pub row_search_start_row: u32,
    /// 行筛选关键字输入
    pub row_search_keyword: String,
    /// 行筛选是否已执行
    pub is_row_searching: bool,
    /// 行筛选匹配的行数
    pub row_matched_count: usize,
    /// 行筛选搜索的总行数
    pub row_total_searched: usize,
    /// 行筛选诊断信息
    pub row_debug_info: String,
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
            row_title: String::new(),
            row_search_col: 0,
            row_search_start_row: 0,
            row_search_keyword: String::new(),
            is_row_searching: false,
            row_matched_count: 0,
            row_total_searched: 0,
            row_debug_info: String::new(),
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

/// 加载行筛选配置：读取 my-excel.yaml 中 search.row，解析单元格值作为标题
pub fn load_row_filter_config(
    excel_data: &ExcelData,
    current_sheet: usize,
) -> (String, u32, u32) {
    // (title, col, start_row)
    let sheet = match excel_data.get_sheet(current_sheet) {
        Some(s) => s,
        None => return (String::new(), 0, 0),
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
        return (String::new(), 0, 0);
    }

    // 取第一个单元格引用（如 "A14,B14" → 取 A14）
    let first_seg = range_str.split(',').next().unwrap_or(&range_str).trim();
    let first_cell = if let Some(idx) = first_seg.find('-') {
        &first_seg[..idx] // "D14-F14" → "D14"
    } else {
        first_seg
    };

    if let Some((col, row)) = parse_cell_ref(first_cell) {
        let title = sheet
            .get_cell(row, col)
            .map(|c| cell_search_value(c))
            .unwrap_or_default();
        return (title, col, row);
    }

    (String::new(), 0, 0)
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

/// 执行行筛选搜索
///
/// 从配置单元格所在列、下一行开始向下遍历，
/// 对关键字进行模糊匹配，不匹配的行加入 hidden_rows。
pub fn execute_row_search(
    state: &mut SearchWindowState,
    sheet: &SheetData,
    hidden_rows: &mut HashSet<u32>,
) {
    hidden_rows.clear();

    if state.row_search_col == 0 || state.row_search_start_row == 0 {
        state.row_debug_info = "行筛选未配置".to_string();
        return;
    }

    let (keywords, is_range) = parse_row_keywords(&state.row_search_keyword);
    if keywords.is_empty() {
        state.row_debug_info = "请输入行筛选关键字".to_string();
        return;
    }

    let col = state.row_search_col;
    let start_row = state.row_search_start_row;
    let max_row = sheet.max_row;

    // 收集搜索范围内所有行的值
    let mut row_values: Vec<(u32, String)> = Vec::new();
    for row in (start_row + 1)..=max_row {
        let value = sheet
            .get_cell(row, col)
            .map(|c| cell_search_value(c).to_lowercase())
            .unwrap_or_default();
        row_values.push((row, value));
    }

    state.row_total_searched = row_values.len();

    if is_range && keywords.len() == 2 {
        // 范围匹配：值在 [kw1, kw2] 之间
        let lo = &keywords[0];
        let hi = &keywords[1];
        for (row, value) in &row_values {
            let v = value.trim();
            if v < lo.as_str() || v > hi.as_str() {
                hidden_rows.insert(*row);
            }
        }
    } else {
        // 单值或多值模糊匹配
        for (row, value) in &row_values {
            let matched = keywords.iter().any(|kw| row_fuzzy_match(value, kw));
            if !matched {
                hidden_rows.insert(*row);
            }
        }
    }

    // 处理跨行合并：以左上角为准
    expand_hidden_rows_for_merged_cells(sheet, hidden_rows, col);

    // 确保配置行自身不被隐藏
    hidden_rows.remove(&start_row);

    state.row_matched_count = state.row_total_searched.saturating_sub(hidden_rows.len());
    state.is_row_searching = true;

    // 诊断信息
    let col_letter = crate::excel::reader::col_to_letter(col);
    state.row_debug_info = format!(
        "行筛选: {}列 行{}→{} 共{}行 | 匹配{}行 隐藏{}行",
        col_letter, start_row + 1, max_row,
        state.row_total_searched, state.row_matched_count, hidden_rows.len()
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

                        // 搜索按钮（统一执行列筛选 + 行筛选）
                        let has_col_input = excel_data.is_some()
                            && state.selected_index < state.column_options.len()
                            && !state.search_keyword.is_empty();
                        let has_row_input = excel_data.is_some()
                            && !state.row_search_keyword.is_empty()
                            && state.row_search_col > 0;
                        if ui
                            .add_enabled(has_col_input || has_row_input, egui::Button::new("🔍 搜索"))
                            .clicked()
                        {
                            if let Some(data) = excel_data {
                                if let Some(sheet) = data.get_sheet(current_sheet) {
                                    // 第一步：列筛选（有输入则执行，否则清空旧结果）
                                    if has_col_input {
                                        execute_search(state, sheet, hidden_columns);
                                    } else {
                                        hidden_columns.clear();
                                    }
                                    // 第二步：行筛选（在列筛选结果上）
                                    if has_row_input {
                                        execute_row_search(state, sheet, hidden_rows);
                                    } else {
                                        hidden_rows.clear();
                                    }
                                }
                            }
                        }

                        // 重置按钮（统一清空列筛选 + 行筛选）
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
                            state.row_search_keyword.clear();
                            state.row_debug_info.clear();
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
                    // 同步加载行筛选配置
                    let (title, col, row) = load_row_filter_config(data, current_sheet);
                    state.row_title = title;
                    state.row_search_col = col;
                    state.row_search_start_row = row;
                }
            }

            // 列筛选下拉框 + 搜索关键字输入框（同行）
            ui.horizontal(|ui| {
                ui.label("列筛选:");
                let selected_text = state
                    .column_options
                    .get(state.selected_index)
                    .map(|o| format!("{} ({})", o.title, o.cell_ref))
                    .unwrap_or_else(|| "请选择列...".to_string());
                egui::ComboBox::from_id_salt("search_column_select")
                    .selected_text(&selected_text)
                    .width(166.0)
                    .show_ui(ui, |ui| {
                        for (i, opt) in state.column_options.iter().enumerate() {
                            let label = format!("{} ({})", opt.title, opt.cell_ref);
                            if ui
                                .selectable_label(i == state.selected_index, &label)
                                .clicked()
                            {
                                if i != state.selected_index {
                                    state.selected_index = i;
                                    // 切换列筛选 → 自动重置搜索结果，恢复表格
                                    hidden_columns.clear();
                                    hidden_rows.clear();
                                    state.search_keyword.clear();
                                    state.is_searching = false;
                                    state.matched_count = 0;
                                    state.total_searched = 0;
                                }
                            }
                        }
                    });

                ui.add_space(6.0);

                let input = egui::TextEdit::singleline(&mut state.search_keyword)
                    .desired_width(f32::INFINITY)
                    .hint_text("输入搜索关键字...");
                let response = ui.add(input);

                // Enter 键触发统一搜索（列筛选 + 行筛选）
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) && response.has_focus() {
                    if let Some(data) = excel_data {
                        if let Some(sheet) = data.get_sheet(current_sheet) {
                            let has_col = state.selected_index < state.column_options.len()
                                && !state.search_keyword.is_empty();
                            let has_row = !state.row_search_keyword.is_empty()
                                && state.row_search_col > 0;
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

            // ══════ 行筛选（列筛选下方） ══════
            if !state.row_title.is_empty() {
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(format!("{}:", state.row_title));
                    let input = egui::TextEdit::singleline(&mut state.row_search_keyword)
                        .desired_width(f32::INFINITY)
                        .hint_text("xxxx 或 'xx1','xx2' 或 'xx3'-'xx4'");
                    let response = ui.add(input);

                    // Enter 键触发统一搜索（列筛选 + 行筛选）
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) && response.has_focus() {
                        if let Some(data) = excel_data {
                            if let Some(sheet) = data.get_sheet(current_sheet) {
                                let has_col = state.selected_index < state.column_options.len()
                                    && !state.search_keyword.is_empty();
                                let has_row = !state.row_search_keyword.is_empty()
                                    && state.row_search_col > 0;
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
