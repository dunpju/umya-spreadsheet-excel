// 引入 umya-spreadsheet 库用于读取 Excel 文件
use umya_spreadsheet::reader;
// 引入 HashMap 用于存储单元格和列宽数据
use std::collections::{HashMap, HashSet};
use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HorizontalAlignment {
    General,
    Left,
    Center,
    Right,
    Fill,
    Justify,
    CenterContinuous,
    Distributed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalAlignment {
    Top,
    Center,
    Bottom,
    Justify,
    Distributed,
}

#[derive(Debug, Clone)]
pub struct CellAlignment {
    pub horizontal: HorizontalAlignment,
    pub vertical: VerticalAlignment,
    #[allow(dead_code)]
    pub indent: i32,
    #[allow(dead_code)]
    pub text_wrap: bool,
}

impl Default for CellAlignment {
    fn default() -> Self {
        Self {
            horizontal: HorizontalAlignment::General,
            vertical: VerticalAlignment::Bottom,
            indent: 0,
            text_wrap: false,
        }
    }
}

/// 单个边框样式
#[derive(Debug, Clone, PartialEq)]
pub struct CellBorder {
    /// 边框样式字符串（如 "thin", "medium", "none" 等）
    pub style: String,
    /// 边框颜色（RGB）
    pub color: Option<(u8, u8, u8)>,
}

impl Default for CellBorder {
    fn default() -> Self {
        Self {
            style: String::new(),
            color: None,
        }
    }
}

/// 单元格四边边框
#[derive(Debug, Clone, PartialEq)]
pub struct CellBorders {
    pub left: CellBorder,
    pub right: CellBorder,
    pub top: CellBorder,
    pub bottom: CellBorder,
}

impl Default for CellBorders {
    fn default() -> Self {
        Self {
            left: CellBorder::default(),
            right: CellBorder::default(),
            top: CellBorder::default(),
            bottom: CellBorder::default(),
        }
    }
}

/// 单元格批注（Comment）
///
/// 由 umya-spreadsheet 解析的经典批注（legacy `<comment>`）。
/// 作者已由 OOXML 的 `authorId` 解析为作者名字符串；
/// `text` 为完整文本（富文本各 run 已拼接）。
#[derive(Debug, Clone)]
pub struct CellComment {
    /// 批注作者（读取时已由 authorId 解析）
    pub author: String,
    /// 批注全文（plain text + rich text 各 run 已拼接）
    pub text: String,
}

/// 单元格数据结构，存储单元格的值和公式
#[derive(Debug, Clone)]
pub struct CellData {
    /// 单元格的显示值
    pub value: String,
    /// 单元格的原始数值（如日期序列号等，用于公式计算）
    pub raw_number: Option<f64>,
    /// 单元格的公式（如存在）
    pub formula: String,
    /// 单元格对齐方式
    pub alignment: CellAlignment,
    /// 背景颜色（RGB）
    pub background_color: Option<(u8, u8, u8)>,
    /// 原始背景颜色（条件格式应用前，用于恢复）
    pub original_bg: Option<(u8, u8, u8)>,
    /// 字体大小（磅）
    pub font_size: Option<f64>,
    /// 字体颜色（RGB）
    pub font_color: Option<(u8, u8, u8)>,
    /// 数字格式代码（如日期格式 "yyyy/m/d"）
    pub number_format: Option<String>,
    /// 字体是否加粗
    pub bold: bool,
    /// 单元格边框：上/下/左/右边框样式和颜色
    pub borders: CellBorders,
    /// 单元格批注（Comment），无批注时为 None
    pub comment: Option<CellComment>,
}

/// CellData 的默认实现，创建空值和空公式的单元格
impl Default for CellData {
    fn default() -> Self {
        Self {
            value: String::new(),
            raw_number: None,
            formula: String::new(),
            alignment: CellAlignment::default(),
            background_color: None,
            original_bg: None,
            font_size: None,
            font_color: None,
            number_format: None,
            bold: false,
            borders: CellBorders::default(),
            comment: None,
        }
    }
}

/// 单元格范围结构，表示合并单元格的区域
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellRange {
    /// 起始行号（从1开始）
    pub start_row: u32,
    /// 起始列号（从1开始）
    pub start_col: u32,
    /// 结束行号（从1开始）
    pub end_row: u32,
    /// 结束列号（从1开始）
    pub end_col: u32,
}

/// 数据有效性类型
#[derive(Debug, Clone, PartialEq)]
pub enum DataValidationType {
    None,
    Whole,
    Decimal,
    List,
    Date,
    Time,
    TextLength,
    Custom,
}

/// 数据有效性运算符
#[derive(Debug, Clone, PartialEq)]
pub enum DataValidationOperator {
    Between,
    NotBetween,
    Equal,
    NotEqual,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
}

/// 数据有效性信息
#[derive(Debug, Clone)]
pub struct DataValidationInfo {
    /// 输入提示标题
    pub prompt_title: String,
    /// 输入提示内容
    pub prompt: String,
    /// 错误提示标题
    pub error_title: String,
    /// 错误提示内容
    pub error_message: String,
    /// 是否显示错误提示
    pub show_error_message: bool,
    /// 有效性类型
    pub dv_type: DataValidationType,
    /// 运算符
    pub dv_operator: DataValidationOperator,
    /// 公式1（如最小值、起始值）
    pub formula1: String,
    /// 公式2（如最大值、结束值）
    pub formula2: String,
    /// 适用的单元格范围（提取时记录，转换工具迁移用）
    #[allow(dead_code)]
    pub ranges: Vec<CellRange>,
}

/// 列插入复制选项
#[derive(Debug, Clone, Copy)]
pub struct ColumnCopyOptions {
    /// 是否复制合并单元格信息
    pub copy_merge: bool,
    /// 是否复制公式（列引用自动偏移）
    pub copy_formula: bool,
    /// 是否复制样式（字体大小、字体颜色、背景色、对齐、数字格式）
    pub copy_style: bool,
    /// 是否复制单元格值
    pub copy_value: bool,
}

impl Default for ColumnCopyOptions {
    fn default() -> Self {
        Self {
            copy_merge: false,
            copy_formula: false,
            copy_style: true,
            copy_value: false,
        }
    }
}

impl ColumnCopyOptions {
    pub fn new(copy_merge: bool, copy_formula: bool, copy_style: bool, copy_value: bool) -> Self {
        Self { copy_merge, copy_formula, copy_style, copy_value }
    }
}

impl CellRange {
    /// 创建新的单元格范围
    /// 
    /// # 参数
    /// * `start_row` - 起始行号
    /// * `start_col` - 起始列号
    /// * `end_row` - 结束行号
    /// * `end_col` - 结束列号
    pub fn new(start_row: u32, start_col: u32, end_row: u32, end_col: u32) -> Self {
        Self {
            start_row,
            start_col,
            end_col,
            end_row,
        }
    }

    /// 检查指定的行列坐标是否在当前范围内
    /// 
    /// # 参数
    /// * `col` - 要检查的列号
    /// * `row` - 要检查的行号
    /// 
    /// # 返回值
    /// 如果坐标在范围内返回 true，否则返回 false
    pub fn contains(&self, col: u32, row: u32) -> bool {
        row >= self.start_row && row <= self.end_row && col >= self.start_col && col <= self.end_col
    }

    /// 检查指定的行列坐标是否是范围的左上角（起始单元格）
    /// 
    /// # 参数
    /// * `col` - 要检查的列号
    /// * `row` - 要检查的行号
    /// 
    /// # 返回值
    /// 如果是起始单元格返回 true，否则返回 false
    pub fn is_top_left(&self, col: u32, row: u32) -> bool {
        row == self.start_row && col == self.start_col
    }
}

/// 工作表数据结构，包含工作表的所有信息
#[derive(Debug)]
pub struct SheetData {
    /// 工作表名称
    pub name: String,
    /// 单元格数据，键为 (行号, 列号)，值为 CellData
    pub cells: HashMap<(u32, u32), CellData>,
    /// 合并单元格列表
    pub merged_cells: Vec<CellRange>,
    /// 工作表最大行号
    pub max_row: u32,
    /// 工作表最大列号
    pub max_col: u32,
    /// 列宽数据，键为列号，值为宽度
    pub column_widths: HashMap<u32, f64>,
    /// 行高数据，键为行号，值为高度
    pub row_heights: HashMap<u32, f64>,
    /// 冻结行数（Excel 冻结窗格中的水平分割值，0 表示无冻结）
    pub frozen_rows: u32,
    /// 冻结列数（Excel 冻结窗格中的垂直分割值，0 表示无冻结）
    pub frozen_cols: u32,
    /// 数据有效性规则列表
    pub data_validations: Vec<DataValidationInfo>,
    /// 合并单元格索引：映射每个被合并覆盖的单元格到 merged_cells 中的索引
    /// 用于 O(1) 查找替代原来的 O(n) 线性扫描
    pub merge_index: HashMap<(u32, u32), usize>,
    /// 条件格式规则（来自原表的 conditional formatting → dxf）
    pub conditional_rules: Vec<CondFormatRule>,
    /// 条件格式脏标志：单元格值变化（公式求值）后置位，viewer 仅在置位时重算条件格式
    pub cf_dirty: bool,
    /// 增量 CF 重算：仅需重算的单元格（粘贴/编辑小范围变更时填入，避免全表 10万+ 格遍历）。
    /// 为 None 或空集时回退到 cf_dirty 全量重算。
    pub cf_dirty_cells: Option<std::collections::HashSet<(u32, u32)>>,
    /// 条件格式列级索引缓存（列号 → 相关规则索引列表），避免每次 CF 重算时重建
    pub cf_col_index: Option<std::collections::HashMap<u32, Vec<usize>>>,
    /// 条件格式索引脏标志（规则变更时置 true，下次访问时重建索引）
    pub cf_col_index_dirty: bool,
    /// 公式单元格位置索引（用于 `build_formula_graph` 快速定位公式，替代遍历全部 cells 的 O(cells) 迭代）。
    /// 由 `rebuild_formula_positions()` 构建；在单元格公式被写入/清除时通过
    /// `mark_formula`/`unmark_formula` 维护；行/列插入删除等难以精确追踪的场景通过
    /// `formula_positions_dirty` 标记，下次使用时按需全量重建。
    pub formula_positions: HashSet<(u32, u32)>,
    /// 公式位置索引脏标志：为 true 时下次 `build_formula_graph` 先重建索引再迭代（回退到 O(cells)）。
    /// 由行/列插入删除、合并变更等设置；精确的写入/清除路径通过
    /// `mark_formula`/`unmark_formula` 维护且不清脏（脏标志仅作安全兜底）。
    pub formula_positions_dirty: bool,
    /// 缓存的公式依赖图（AST + 正向/反向依赖边），由 `build_formula_graph` 构建后写入。
    /// 缓存命中时直接克隆返回，避免 O(F × parse_formula) 的全量 AST 解析（250K 公式格 ~300ms → ~5ms）。
    /// 公式变更时通过 `invalidate_formula_graph()` 置脏重建。
    /// 类型擦除为 `Box<dyn Any>` 以避免 reader.rs ↔ formula.rs 循环类型依赖
    /// （`CachedFormulaGraph` 定义在 formula.rs 中，含 `FormulaNode`）。
    pub cached_graph: Option<Box<dyn std::any::Any + Send + Sync>>,
    /// 公式依赖图缓存脏标志：为 true 时下次 `build_formula_graph` 重建完整图。
    /// 由 `invalidate_formula_graph()` 设置（在编辑/填充/粘贴/撤销/行列插入删除等修改公式的路径中调用）。
    pub cached_graph_dirty: bool,
}

impl Clone for SheetData {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            cells: self.cells.clone(),
            merged_cells: self.merged_cells.clone(),
            max_row: self.max_row,
            max_col: self.max_col,
            column_widths: self.column_widths.clone(),
            row_heights: self.row_heights.clone(),
            frozen_rows: self.frozen_rows,
            frozen_cols: self.frozen_cols,
            data_validations: self.data_validations.clone(),
            merge_index: self.merge_index.clone(),
            conditional_rules: self.conditional_rules.clone(),
            cf_dirty: self.cf_dirty,
            cf_dirty_cells: self.cf_dirty_cells.clone(),
            cf_col_index: self.cf_col_index.clone(),
            cf_col_index_dirty: self.cf_col_index_dirty,
            formula_positions: self.formula_positions.clone(),
            formula_positions_dirty: self.formula_positions_dirty,
            cached_graph: None, // 缓存懒重建，不需要克隆
            cached_graph_dirty: self.cached_graph_dirty,
        }
    }
}

/// 用户自定义的条件格式规则（YAML 持久化）
#[derive(Debug, Clone, PartialEq)]
pub struct UserCondFormatRule {
    pub operator: String,   // ">", "<", "=", ">=", "<=", "!="
    pub value: String,      // 阈值 "60"
    pub color: String,      // 填充色 "#FFC7CE"
    pub range: String,      // "=G3:G154"
}

impl Default for UserCondFormatRule {
    fn default() -> Self {
        Self {
            operator: "<=".to_string(),
            value: "60".to_string(),
            color: "#FFC7CE".to_string(),
            range: "=G3:G154".to_string(),
        }
    }
}

/// 条件格式规则（简化后存储，供加载后求值应用）
#[derive(Debug, Clone)]
pub struct CondFormatRule {
    /// 适用的单元格范围（提取时记录，转换工具迁移用）
    #[allow(dead_code)]
    pub ranges: Vec<CellRange>,
    /// 条件类型（"CellIs" / "ContainsText" / "Expression" 等）
    #[allow(dead_code)]
    pub rule_type: String,
    /// 运算符（"greaterThan" / "lessThan" / "between" 等）
    pub operator: String,
    /// 公式文本（阈值等）
    pub formula_text: String,
    /// containsText 的搜索文本
    pub text: String,
    /// 应用样式时覆盖的背景色
    pub bg_color: Option<(u8, u8, u8)>,
    /// 应用样式时覆盖的字体色
    pub font_color: Option<(u8, u8, u8)>,
    /// 应用样式时覆盖的字体加粗
    pub bold: bool,
}

impl SheetData {
    /// 创建新的工作表数据
    /// 
    /// # 参数
    /// * `name` - 工作表名称
    pub fn new(name: String) -> Self {
        Self {
            name,
            cells: HashMap::new(),
            merged_cells: Vec::new(),
            max_row: 0,
            max_col: 0,
            column_widths: HashMap::new(),
            row_heights: HashMap::new(),
            frozen_rows: 0,
            frozen_cols: 0,
            data_validations: Vec::new(),
            merge_index: HashMap::new(),
            conditional_rules: Vec::new(),
            cf_dirty: false,
            cf_dirty_cells: None,
            cf_col_index: None,
            cf_col_index_dirty: true,
            formula_positions: HashSet::new(),
            formula_positions_dirty: true, // 初始脏：首次使用时全量构建
            cached_graph: None,
            cached_graph_dirty: true,     // 初始脏：首次使用时全量构建
        }
    }

    /// 重建合并单元格索引（在 merged_cells 变更后调用）
    pub fn rebuild_merge_index(&mut self) {
        self.merge_index.clear();
        for (idx, mr) in self.merged_cells.iter().enumerate() {
            for row in mr.start_row..=mr.end_row {
                for col in mr.start_col..=mr.end_col {
                    self.merge_index.insert((col, row), idx);
                }
            }
        }
    }

    /// 重建公式位置索引（遍历全部 cells，收集 formula 非空的单元格坐标）。
    ///
    /// 在加载完成后（已有全部 cells 数据）调用一次即可；后续通过 `mark_formula`/`unmark_formula`
    /// 精确维护。若脏标志为 true（行/列插入删除后），下次 `build_formula_graph` 自动
    /// 调用本函数重建。
    pub fn rebuild_formula_positions(&mut self) {
        self.formula_positions.clear();
        for (&key, cell) in self.cells.iter() {
            if !cell.formula.is_empty() {
                self.formula_positions.insert(key);
            }
        }
        self.formula_positions_dirty = false;
    }

    /// 标记指定位置为公式单元格（写入 `formula_positions` 索引）。
    ///
    /// 在单元格公式被设置/写入时调用（编辑、填充、粘贴、撤销恢复等路径）。
    /// 不触碰 `formula_positions_dirty`（脏标志仅由行列插入删除等结构性操作设置）。
    #[inline]
    pub fn mark_formula(&mut self, row: u32, col: u32) {
        self.formula_positions.insert((row, col));
    }

    /// 取消标记指定位置的公式单元格（从 `formula_positions` 索引中移除）。
    ///
    /// 在单元格公式被清除或覆盖为纯值时调用（编辑、清空、填充非公式值等路径）。
    /// 不触碰 `formula_positions_dirty`。
    #[inline]
    pub fn unmark_formula(&mut self, row: u32, col: u32) {
        self.formula_positions.remove(&(row, col));
    }

    /// 获取指定单元格的数据
    /// 
    /// # 参数
    /// * `row` - 行号
    /// * `col` - 列号
    /// 
    /// # 返回值
    /// 如果单元格存在返回 Some(&CellData)，否则返回 None
    pub fn get_cell(&self, row: u32, col: u32) -> Option<&CellData> {
        self.cells.get(&(row, col))
    }

    /// 获取指定单元格所在的合并范围
    /// 
    /// # 参数
    /// * `row` - 行号
    /// * `col` - 列号
    /// 
    /// # 返回值
    /// 如果单元格在合并范围内返回 Some(&CellRange)，否则返回 None
    pub fn get_merged_range(&self, col: u32, row: u32) -> Option<&CellRange> {
        self.merge_index
            .get(&(col, row))
            .and_then(|&idx| self.merged_cells.get(idx))
    }

    /// 获取指定单元格的数据有效性输入提示信息
    ///
    /// # 参数
    /// * `col` - 列号
    /// * `row` - 行号
    ///
    /// # 返回值
    /// 如果单元格有数据有效性且配置了输入提示，返回 Some(&DataValidationInfo)
    pub fn get_input_message(&self, col: u32, row: u32) -> Option<&DataValidationInfo> {
        self.data_validations.iter().find(|dv| {
            dv.ranges.iter().any(|r| r.contains(col, row))
        })
    }

    /// 校验输入值是否符合数据有效性规则
    ///
    /// # 参数
    /// * `col` - 列号
    /// * `row` - 行号
    /// * `input` - 用户输入的值
    ///
    /// # 返回值
    /// 如果校验失败返回 Some((error_title, error_message))，校验通过返回 None
    pub fn validate_cell(&self, col: u32, row: u32, input: &str) -> Option<(String, String)> {
        let dv = self.data_validations.iter().find(|dv| {
            dv.ranges.iter().any(|r| r.contains(col, row))
        })?;

        if !dv.show_error_message { return None; }
        if dv.dv_type == DataValidationType::None { return None; }

        let valid = match dv.dv_type {
            DataValidationType::Date => {
                let v = input.parse::<f64>().ok().or_else(|| ExcelData::parse_date_string(input));
                validate_number(&v, &dv.dv_operator, &dv.formula1, &dv.formula2, true)
            }
            DataValidationType::Whole => {
                let v = input.parse::<f64>();
                if let Ok(n) = v {
                    if n != n.trunc() { false } // 不是整数
                    else { validate_number(&Some(n), &dv.dv_operator, &dv.formula1, &dv.formula2, false) }
                } else { false }
            }
            DataValidationType::Decimal => {
                validate_number(&input.parse::<f64>().ok(), &dv.dv_operator, &dv.formula1, &dv.formula2, false)
            }
            DataValidationType::List => {
                let items: Vec<&str> = dv.formula1.split(',').map(|s| s.trim()).collect();
                items.iter().any(|item| item.eq_ignore_ascii_case(input.trim()))
            }
            DataValidationType::TextLength => {
                let len = input.chars().count() as f64;
                validate_number(&Some(len), &dv.dv_operator, &dv.formula1, &dv.formula2, false)
            }
            _ => true, // Time, Custom, None 暂不严格校验
        };

        if !valid {
            let title = if dv.error_title.is_empty() { "输入错误".to_string() } else { dv.error_title.clone() };
            let msg = if dv.error_message.is_empty() { "输入的值不符合数据有效性规则".to_string() } else { dv.error_message.clone() };
            Some((title, msg))
        } else {
            None
        }
    }

    /// 获取默认插入数量：普通单元格返回 1，合并单元格返回对应维度的跨度
    /// 查找包含指定列的跨列合并范围（不考虑行）
    pub fn get_column_merge(&self, col: u32) -> Option<&CellRange> {
        self.merged_cells.iter().find(|mr| {
            col >= mr.start_col && col <= mr.end_col && mr.start_col != mr.end_col
        })
    }

    pub fn default_insert_count(&self, col: u32, row: u32, is_row: bool) -> u32 {
        // 先检查单元格本身是否在合并范围内
        if let Some(mr) = self.get_merged_range(col, row) {
            if is_row {
                return mr.end_row - mr.start_row + 1;
            } else {
                return mr.end_col - mr.start_col + 1;
            }
        }
        // 单元格本身不在合并范围，检查该列/行是否属于其他跨列/行合并
        for mr in &self.merged_cells {
            if is_row {
                if row >= mr.start_row && row <= mr.end_row && mr.start_row != mr.end_row {
                    return mr.end_row - mr.start_row + 1;
                }
            } else {
                if col >= mr.start_col && col <= mr.end_col && mr.start_col != mr.end_col {
                    return mr.end_col - mr.start_col + 1;
                }
            }
        }
        1
    }

    /// 在指定位置插入 N 行
    ///
    /// # 参数
    /// * `anchor_row` - 锚点行号
    /// * `n` - 插入行数
    /// * `after` - true 表示在锚点下方插入，false 表示在锚点上方插入
    pub fn insert_rows(&mut self, anchor_row: u32, n: u32, after: bool) {
        let insert_at = if after { anchor_row + 1 } else { anchor_row };

        // 1. 移动单元格
        let mut new_cells = HashMap::new();
        for ((row, col), cell_data) in self.cells.drain() {
            let new_row = if row >= insert_at { row + n } else { row };
            new_cells.insert((new_row, col), cell_data);
        }
        self.cells = new_cells;

        // 2. 处理合并单元格：整体下移 / 跨越扩展 / 不动
        for mr in &mut self.merged_cells {
            if mr.start_row >= insert_at {
                // 整个范围在插入点下方 → 整体下移
                mr.start_row += n;
                mr.end_row += n;
            } else if mr.end_row >= insert_at {
                // 跨越插入线 → 扩展（不拆分）
                mr.end_row += n;
            }
            // else: 完全在上方，不变
        }

        // 3. 移动行高
        let mut new_heights = HashMap::new();
        for (row, height) in self.row_heights.drain() {
            let new_row = if row >= insert_at { row + n } else { row };
            new_heights.insert(new_row, height);
        }
        self.row_heights = new_heights;

        // 4. 更新 max_row
        self.max_row += n;

        // 5. 处理数据有效性范围
        for dv in &mut self.data_validations {
            for r in &mut dv.ranges {
                if r.start_row >= insert_at {
                    r.start_row += n;
                    r.end_row += n;
                } else if r.end_row >= insert_at {
                    r.end_row += n;
                }
            }
        }

        // 5.1 处理条件格式范围：跨插入点自动扩展 + 源行条件格式延伸至新行
        for rule in &mut self.conditional_rules {
            for r in &mut rule.ranges {
                if r.start_row >= insert_at {
                    r.start_row += n;
                    r.end_row += n;
                } else if r.end_row >= insert_at {
                    r.end_row += n;
                }
                if after {
                    let src_start = insert_at.saturating_sub(n);
                    let src_end = insert_at.saturating_sub(1);
                    if r.start_row <= src_end && r.end_row >= src_start {
                        let new_end = insert_at + n - 1;
                        if new_end > r.end_row {
                            r.end_row = new_end;
                        }
                    }
                }
            }
        }

        // 6. 新行样式继承：从相邻行（原 insert_at 行，现已移到 insert_at + n）复制样式
        // 收集需要继承样式的列
        let style_cols: Vec<u32> = self.cells.iter()
            .filter(|((row, _), _)| *row >= insert_at && *row <= insert_at + n + self.max_row)
            .map(|((_, col), _)| *col)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let mut new_style_cells: Vec<((u32, u32), CellData)> = Vec::new();
        for col in style_cols {
            // 从 insert_at + n 处（即原来的 insert_at 行）获取样式模板
            if let Some(template) = self.cells.get(&(insert_at + n, col)) {
                let styled = CellData {
                    value: String::new(),
                    raw_number: None,
                    formula: String::new(),
                    alignment: template.alignment.clone(),
                    background_color: template.original_bg,
                    original_bg: template.original_bg,
                    font_size: template.font_size,
                    font_color: template.font_color,
                    number_format: template.number_format.clone(),
                    bold: template.bold,
                    borders: template.borders.clone(),
                    comment: None,
                };
                for offset in 0..n {
                    let new_row = insert_at + offset;
                    if !self.cells.contains_key(&(new_row, col)) {
                        new_style_cells.push(((new_row, col), styled.clone()));
                    }
                }
            }
        }
        for (key, cell) in new_style_cells {
            self.cells.insert(key, cell);
        }

        // 6.5 复制合并结构：将源行的合并范围偏移到新行位置
        // 源行 = insert_at + n 处（即原来的 insert_at 行，已被下移）
        // 只复制行范围完全落在源行内的合并
        let source_merges: Vec<CellRange> = self.merged_cells.iter()
            .filter(|mr| {
                mr.start_row >= insert_at + n && mr.end_row < insert_at + n + n
            })
            .copied()
            .collect();
        let row_shift = insert_at as i32 - (insert_at + n) as i32;
        for mr in &source_merges {
            let new_start_row = (mr.start_row as i32 + row_shift).max(1) as u32;
            let new_end_row = (mr.end_row as i32 + row_shift).max(1) as u32;
            let new_merge = CellRange::new(
                new_start_row,
                mr.start_col,
                new_end_row,
                mr.end_col,
            );
            // 去重：不添加已存在的合并范围
            if !self.merged_cells.iter().any(|existing| {
                existing.start_row == new_merge.start_row
                && existing.start_col == new_merge.start_col
                && existing.end_row == new_merge.end_row
                && existing.end_col == new_merge.end_col
            }) {
                self.merged_cells.push(new_merge);
            }
        }

        // 6.6 复制数据有效性规则：将范围完全落在源行内的数据有效性复制并偏移到新行
        let source_dvs: Vec<DataValidationInfo> = self.data_validations.iter()
            .filter(|dv| {
                dv.ranges.iter().any(|r| {
                    r.start_row >= insert_at + n && r.end_row < insert_at + n + n
                })
            })
            .cloned()
            .collect();
        for dv in &source_dvs {
            let mut new_dv = dv.clone();
            for r in &mut new_dv.ranges {
                let new_start = (r.start_row as i32 + row_shift).max(1) as u32;
                let new_end = (r.end_row as i32 + row_shift).max(1) as u32;
                r.start_row = new_start;
                r.end_row = new_end;
            }
            if !new_dv.formula1.is_empty() {
                new_dv.formula1 = crate::excel::formula::adjust_formula_rows(
                    &new_dv.formula1, insert_at, -(n as i32),
                );
            }
            if !new_dv.formula2.is_empty() {
                new_dv.formula2 = crate::excel::formula::adjust_formula_rows(
                    &new_dv.formula2, insert_at, -(n as i32),
                );
            }
            self.data_validations.push(new_dv);
        }

        // 7. 修正已有公式引用：所有现有单元格和数据有效性中的公式，
        //    行号 >= insert_at 的相对行引用下移 n 行
        //    单元格公式数量较多时使用多线程并行处理以提升性能
        {
            let formulas: Vec<((u32, u32), String)> = self.cells.iter()
                .filter(|(_, cell)| !cell.formula.is_empty())
                .map(|(&key, cell)| (key, cell.formula.clone()))
                .collect();

            if !formulas.is_empty() {
                let cpu_count = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(1);

                if formulas.len() >= 100 && cpu_count > 1 {
                    // 多线程并行处理：将公式分块，每块一个线程
                    let num_threads = cpu_count.min(formulas.len());
                    let chunk_size = (formulas.len() + num_threads - 1) / num_threads;
                    let threshold = insert_at;
                    let shift = n as i32;

                    let handles: Vec<std::thread::JoinHandle<Vec<((u32, u32), String)>>> = formulas
                        .chunks(chunk_size)
                        .map(|chunk| {
                            let chunk = chunk.to_vec();
                            std::thread::spawn(move || {
                                chunk.into_iter()
                                    .map(|(key, formula)| {
                                        let adjusted = crate::excel::formula::adjust_formula_rows(
                                            &formula, threshold, shift,
                                        );
                                        (key, adjusted)
                                    })
                                    .collect()
                            })
                        })
                        .collect();

                    for handle in handles {
                        if let Ok(results) = handle.join() {
                            for (key, adjusted) in results {
                                if let Some(cell) = self.cells.get_mut(&key) {
                                    cell.formula = adjusted;
                                }
                            }
                        }
                    }
                } else {
                    // 公式数量较少或单核，直接在当前线程处理
                    for (key, formula) in formulas {
                        let adjusted = crate::excel::formula::adjust_formula_rows(
                            &formula, insert_at, n as i32,
                        );
                        if let Some(cell) = self.cells.get_mut(&key) {
                            cell.formula = adjusted;
                        }
                    }
                }
            }
        }
        for dv in &mut self.data_validations {
            if !dv.formula1.is_empty() {
                dv.formula1 = crate::excel::formula::adjust_formula_rows(
                    &dv.formula1, insert_at, n as i32,
                );
            }
            if !dv.formula2.is_empty() {
                dv.formula2 = crate::excel::formula::adjust_formula_rows(
                    &dv.formula2, insert_at, n as i32,
                );
            }
        }

        // 8. 重建合并单元格索引
        self.rebuild_merge_index();

        // 行插入移动了所有单元格的行坐标 → formula_positions 索引失效，下次 build_formula_graph 全量重建
        self.formula_positions_dirty = true;
        crate::excel::formula::invalidate_formula_graph(self);
    }

    /// 在表格末尾追加一行。
    ///
    /// 与 `insert_rows(max_row, 1, true)` 不同，此方法使用 `old_max_row` 作为
    /// 公式调整阈值，使得 `=SUM(B15:B199)` 等引用最后一行的公式能正确扩展为
    /// `=SUM(B15:B200)`，将新增行纳入聚合范围。
    ///
    /// 使用多线程并行处理公式更新以提升性能。
    pub fn append_row(&mut self) {
        let old_max_row = self.max_row;

        // 1. 增加 max_row
        self.max_row += 1;

        // 2. 新行样式继承：从原最后一行复制样式到新行
        let style_cols: Vec<u32> = self.cells.iter()
            .filter(|((row, _), _)| *row == old_max_row)
            .map(|((_, col), _)| *col)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        for col in style_cols {
            if let Some(template) = self.cells.get(&(old_max_row, col)) {
                let styled = CellData {
                    value: String::new(),
                    raw_number: None,
                    formula: String::new(),
                    alignment: template.alignment.clone(),
                    background_color: template.original_bg,
                    original_bg: template.original_bg,
                    font_size: template.font_size,
                    font_color: template.font_color,
                    number_format: template.number_format.clone(),
                    bold: template.bold,
                    borders: template.borders.clone(),
                    comment: None,
                };
                self.cells.insert((old_max_row + 1, col), styled);
            }
        }

        // 2.5 扩展条件格式范围：原最后一行被规则覆盖时，新行也纳入
        for rule in &mut self.conditional_rules {
            for r in &mut rule.ranges {
                if r.end_row == old_max_row {
                    r.end_row = self.max_row;
                }
            }
        }

        // 3. 扩展公式引用范围：将行号 >= old_max_row 的相对行引用下移1行
        //    使得 =SUM(B15:B199) → =SUM(B15:B200)
        //    使用多线程并行处理提升性能
        {
            let formulas: Vec<((u32, u32), String)> = self.cells.iter()
                .filter(|(_, cell)| !cell.formula.is_empty())
                .map(|(&key, cell)| (key, cell.formula.clone()))
                .collect();

            if !formulas.is_empty() {
                let cpu_count = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(1);

                if formulas.len() >= 100 && cpu_count > 1 {
                    let num_threads = cpu_count.min(formulas.len());
                    let chunk_size = (formulas.len() + num_threads - 1) / num_threads;
                    let threshold = old_max_row;
                    let shift = 1i32;

                    let handles: Vec<std::thread::JoinHandle<Vec<((u32, u32), String)>>> = formulas
                        .chunks(chunk_size)
                        .map(|chunk| {
                            let chunk = chunk.to_vec();
                            std::thread::spawn(move || {
                                chunk.into_iter()
                                    .map(|(key, formula)| {
                                        let adjusted = crate::excel::formula::adjust_formula_rows(
                                            &formula, threshold, shift,
                                        );
                                        (key, adjusted)
                                    })
                                    .collect()
                            })
                        })
                        .collect();

                    for handle in handles {
                        if let Ok(results) = handle.join() {
                            for (key, adjusted) in results {
                                if let Some(cell) = self.cells.get_mut(&key) {
                                    cell.formula = adjusted;
                                }
                            }
                        }
                    }
                } else {
                    for (key, formula) in formulas {
                        let adjusted = crate::excel::formula::adjust_formula_rows(
                            &formula, old_max_row, 1,
                        );
                        if let Some(cell) = self.cells.get_mut(&key) {
                            cell.formula = adjusted;
                        }
                    }
                }
            }
        }

        // 4. 调整数据有效性公式（通常数量较少，顺序处理）
        for dv in &mut self.data_validations {
            if !dv.formula1.is_empty() {
                dv.formula1 = crate::excel::formula::adjust_formula_rows(
                    &dv.formula1, old_max_row, 1,
                );
            }
            if !dv.formula2.is_empty() {
                dv.formula2 = crate::excel::formula::adjust_formula_rows(
                    &dv.formula2, old_max_row, 1,
                );
            }
        }
    }

    /// 在指定位置插入 M 列
    ///
    /// # 参数
    /// * `anchor_col` - 锚点列号
    /// * `m` - 插入列数
    /// * `after` - true 表示在锚点右侧插入，false 表示在锚点左侧插入
    /// * `options` - 复制选项（合并、公式、样式、值）
    pub fn insert_columns(&mut self, anchor_col: u32, m: u32, after: bool, options: ColumnCopyOptions) {
        let insert_at = if after { anchor_col + 1 } else { anchor_col };

        // ========== Phase A: 结构性移动 ==========

        // 1. 移动单元格
        let mut new_cells = HashMap::new();
        for ((row, col), cell_data) in self.cells.drain() {
            let new_col = if col >= insert_at { col + m } else { col };
            new_cells.insert((row, new_col), cell_data);
        }
        self.cells = new_cells;

        // 2. 处理合并单元格
        for mr in &mut self.merged_cells {
            if mr.start_col >= insert_at {
                mr.start_col += m;
                mr.end_col += m;
            } else if mr.end_col >= insert_at {
                mr.end_col += m;
            }
        }

        // 3. 移动列宽
        let mut new_widths = HashMap::new();
        for (col, width) in self.column_widths.drain() {
            let new_col = if col >= insert_at { col + m } else { col };
            new_widths.insert(new_col, width);
        }
        self.column_widths = new_widths;

        // 3.5 新列宽度：从源列逐列复制列宽
        let source_start_for_width = if after { insert_at.saturating_sub(m) } else { insert_at + m };
        for offset in 0..m {
            let src_col = source_start_for_width + offset;
            if let Some(&src_width) = self.column_widths.get(&src_col) {
                self.column_widths.insert(insert_at + offset, src_width);
            }
        }

        // 4. 更新 max_col
        self.max_col += m;

        // 5. 处理数据有效性范围
        for dv in &mut self.data_validations {
            for r in &mut dv.ranges {
                if r.start_col >= insert_at {
                    r.start_col += m;
                    r.end_col += m;
                } else if r.end_col >= insert_at {
                    r.end_col += m;
                }
            }
        }

        // 5.1 处理条件格式范围：跨插入点自动扩展 + 源列条件格式延伸至新列
        for rule in &mut self.conditional_rules {
            for r in &mut rule.ranges {
                if r.start_col >= insert_at {
                    // 整个范围在插入点右侧 → 整体右移
                    r.start_col += m;
                    r.end_col += m;
                } else if r.end_col >= insert_at {
                    // 范围跨越插入点 → 扩展
                    r.end_col += m;
                }
                // 复制操作：只有范围右侧边缘恰好接触源列时才水平扩展（防止纵向范围被横向串扰）
                if after {
                    let src_start = insert_at.saturating_sub(m);
                    let src_end = insert_at.saturating_sub(1);
                    // 范围的列区间 [r.start_col, r.end_col] 必须与源区间 [src_start, src_end] 有交集
                    if r.start_col <= src_end && r.end_col >= src_start {
                        let new_end = insert_at + m - 1;
                        if new_end > r.end_col {
                            r.end_col = new_end;
                        }
                    }
                }
            }
        }

        // 5.5 修正已有公式引用：所有现有单元格和数据有效性中的公式，
        //     相对列引用 >= insert_at 的右移 m 列
        //     单元格公式数量较多时使用多线程并行处理以提升性能
        {
            let formulas: Vec<((u32, u32), String)> = self.cells.iter()
                .filter(|(_, cell)| !cell.formula.is_empty())
                .map(|(&key, cell)| (key, cell.formula.clone()))
                .collect();

            if !formulas.is_empty() {
                let cpu_count = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(1);

                if formulas.len() >= 100 && cpu_count > 1 {
                    let num_threads = cpu_count.min(formulas.len());
                    let chunk_size = (formulas.len() + num_threads - 1) / num_threads;
                    let threshold = insert_at;
                    let shift = m as i32;

                    let handles: Vec<std::thread::JoinHandle<Vec<((u32, u32), String)>>> = formulas
                        .chunks(chunk_size)
                        .map(|chunk| {
                            let chunk = chunk.to_vec();
                            std::thread::spawn(move || {
                                chunk.into_iter()
                                    .map(|(key, formula)| {
                                        let adjusted = crate::excel::formula::adjust_formula_columns(
                                            &formula, threshold, shift,
                                        );
                                        (key, adjusted)
                                    })
                                    .collect()
                            })
                        })
                        .collect();

                    for handle in handles {
                        if let Ok(results) = handle.join() {
                            for (key, adjusted) in results {
                                if let Some(cell) = self.cells.get_mut(&key) {
                                    cell.formula = adjusted;
                                }
                            }
                        }
                    }
                } else {
                    for (key, formula) in formulas {
                        let adjusted = crate::excel::formula::adjust_formula_columns(
                            &formula, insert_at, m as i32,
                        );
                        if let Some(cell) = self.cells.get_mut(&key) {
                            cell.formula = adjusted;
                        }
                    }
                }
            }
        }
        for dv in &mut self.data_validations {
            if !dv.formula1.is_empty() {
                dv.formula1 = crate::excel::formula::adjust_formula_columns(
                    &dv.formula1, insert_at, m as i32,
                );
            }
            if !dv.formula2.is_empty() {
                dv.formula2 = crate::excel::formula::adjust_formula_columns(
                    &dv.formula2, insert_at, m as i32,
                );
            }
        }

        // ========== Phase B: 复制内容到新列 ==========

        // 确定源列范围的起始列（从哪些列复制内容）
        // - 左侧插入: 源列被右移到 insert_at+m 开始，逐列对应
        // - 右侧插入: 源列在插入点左侧，从 insert_at-m 开始，逐列对应
        let source_start_col = if after {
            insert_at.saturating_sub(m)
        } else {
            insert_at + m
        };

        // 公式偏移参数：
        // - 左侧插入: 源列公式已做 Phase A.5 调整（>=insert_at 右移 m），需用
        //   threshold=insert_at, shift=-m 还原到原始位置
        // - 右侧插入: 源列公式未受影响，用标准复制 threshold=1, shift=+m
        let (formula_threshold, formula_shift): (u32, i32) = if after {
            (1, m as i32)
        } else {
            (insert_at, -(m as i32))
        };

        // 6. 按选项逐列复制单元格数据
        // 收集所有源列涉及的行
        let mut row_set = std::collections::HashSet::new();
        for offset in 0..m {
            let src_col = source_start_col + offset;
            for (row, col) in self.cells.keys() {
                if *col == src_col {
                    row_set.insert(*row);
                }
            }
        }
        let source_rows: Vec<u32> = row_set.into_iter().collect();

        let mut new_cells_to_insert: Vec<((u32, u32), CellData)> = Vec::new();

        for row in source_rows {
            for offset in 0..m {
                let src_col = source_start_col + offset;
                let new_col = insert_at + offset;

                if let Some(source_cell) = self.cells.get(&(row, src_col)).cloned() {
                    let new_cell = CellData {
                        value: if options.copy_value {
                            source_cell.value.clone()
                        } else {
                            String::new()
                        },
                        raw_number: if options.copy_value {
                            source_cell.raw_number
                        } else {
                            None
                        },
                        formula: if options.copy_formula && !source_cell.formula.is_empty() {
                            crate::excel::formula::adjust_formula_columns(
                                &source_cell.formula, formula_threshold, formula_shift,
                            )
                        } else {
                            String::new()
                        },
                        alignment: if options.copy_style {
                            source_cell.alignment.clone()
                        } else {
                            CellAlignment::default()
                        },
                        // 复制原始背景色（非条件格式计算后的颜色），新列由 per-frame 求值独立决定
                        background_color: if options.copy_style {
                            source_cell.original_bg
                        } else {
                            None
                        },
                        original_bg: if options.copy_style {
                            source_cell.original_bg
                        } else {
                            None
                        },
                        font_size: if options.copy_style {
                            source_cell.font_size
                        } else {
                            None
                        },
                        font_color: if options.copy_style {
                            source_cell.font_color
                        } else {
                            None
                        },
                        number_format: if options.copy_style {
                            source_cell.number_format.clone()
                        } else {
                            None
                        },
                        bold: if options.copy_style {
                            source_cell.bold
                        } else {
                            false
                        },
                        borders: if options.copy_style {
                            source_cell.borders.clone()
                        } else {
                            CellBorders::default()
                        },
                        comment: None,
                    };
                    new_cells_to_insert.push(((row, new_col), new_cell));
                }
            }
        }

        // 如果没有 copy_value 也没有 copy_formula，但 copy_style 为 true，
        // 则对源列没有数据但相邻列有数据的行，仍然继承样式
        if options.copy_style && !options.copy_value && !options.copy_formula {
            let style_rows: Vec<u32> = self.cells.iter()
                .filter(|((_, col), _)| *col >= insert_at && *col <= insert_at + m + 10)
                .map(|((row, _), _)| *row)
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            for row in style_rows {
                let has_source = (0..m).any(|o| self.cells.contains_key(&(row, source_start_col + o)));
                if has_source {
                    continue; // 已在上面处理
                }
                for offset in 0..m {
                    let src_col = source_start_col + offset;
                    let new_col = insert_at + offset;
                    if !self.cells.contains_key(&(row, new_col)) {
                        if let Some(template) = self.cells.get(&(row, src_col)) {
                            let styled = CellData {
                                value: String::new(),
                                raw_number: None,
                                formula: String::new(),
                                alignment: template.alignment.clone(),
                                background_color: template.background_color,
                                original_bg: template.original_bg,
                                font_size: template.font_size,
                                font_color: template.font_color,
                                number_format: template.number_format.clone(),
                                bold: template.bold,
                                borders: template.borders.clone(),
                                comment: None,
                            };
                            new_cells_to_insert.push(((row, new_col), styled));
                        }
                    }
                }
            }
        }

        for (key, cell) in new_cells_to_insert {
            self.cells.insert(key, cell);
        }

        // 7. 复制合并结构
        // 只复制完全落在源列范围内的合并范围（避免部分重叠的合并被错误复制）
        // Phase A 已经移动了合并范围的列号，需要还原到移动前的坐标来判断
        if options.copy_merge {
            // Phase A 移动后的合并中，找出源列对应的原始合并
            // 源列在移动后的位置是 source_start_col..source_start_col+m
            // （右侧插入时源列未移动；左侧插入时源列已被右移，正好在 source_start_col 位置）
            let source_merges: Vec<CellRange> = self.merged_cells.iter()
                .filter(|mr| {
                    // 合并范围的列完全落在源列范围内
                    mr.start_col >= source_start_col && mr.end_col < source_start_col + m
                })
                .copied()
                .collect();

            // 将源合并范围整体偏移到新列位置
            let col_shift = insert_at as i32 - source_start_col as i32;
            for mr in &source_merges {
                let new_start_col = (mr.start_col as i32 + col_shift).max(1) as u32;
                let new_end_col = (mr.end_col as i32 + col_shift).max(1) as u32;
                let new_merge = CellRange::new(
                    mr.start_row,
                    new_start_col,
                    mr.end_row,
                    new_end_col,
                );
                // 去重：不添加已存在的合并范围
                if !self.merged_cells.iter().any(|existing| {
                    existing.start_row == new_merge.start_row
                    && existing.start_col == new_merge.start_col
                    && existing.end_row == new_merge.end_row
                    && existing.end_col == new_merge.end_col
                }) {
                    self.merged_cells.push(new_merge);
                }
            }
        }

        // 8. 复制数据有效性规则
        // 只复制范围完全落在源列内的规则
        if options.copy_merge || options.copy_formula || options.copy_style || options.copy_value {
            let source_dvs: Vec<DataValidationInfo> = self.data_validations.iter()
                .filter(|dv| {
                    dv.ranges.iter().any(|r| {
                        // 数据有效性范围完全落在源列范围内
                        r.start_col >= source_start_col && r.end_col < source_start_col + m
                    })
                })
                .cloned()
                .collect();

            let col_shift = insert_at as i32 - source_start_col as i32;
            for dv in &source_dvs {
                let mut new_dv = dv.clone();
                for r in &mut new_dv.ranges {
                    let new_start = (r.start_col as i32 + col_shift).max(1) as u32;
                    let new_end = (r.end_col as i32 + col_shift).max(1) as u32;
                    r.start_col = new_start;
                    r.end_col = new_end;
                }
                if !new_dv.formula1.is_empty() {
                    new_dv.formula1 = crate::excel::formula::adjust_formula_columns(
                        &new_dv.formula1, formula_threshold, formula_shift,
                    );
                }
                if !new_dv.formula2.is_empty() {
                    new_dv.formula2 = crate::excel::formula::adjust_formula_columns(
                        &new_dv.formula2, formula_threshold, formula_shift,
                    );
                }
                self.data_validations.push(new_dv);
            }
        }

        // 9. 重建合并单元格索引
        self.rebuild_merge_index();

        // 列插入移动了所有单元格的列坐标 → formula_positions 索引失效，下次 build_formula_graph 全量重建
        self.formula_positions_dirty = true;
        crate::excel::formula::invalidate_formula_graph(self);
    }
}

/// 数值比较校验辅助函数
/// `value` 为已解析的数值，`date_mode` 为 true 时 formula 也作为日期解析
fn validate_number(
    value: &Option<f64>,
    op: &DataValidationOperator,
    formula1: &str,
    formula2: &str,
    date_mode: bool,
) -> bool {
    let v = match value {
        Some(n) => *n,
        None => return false,
    };

    let parse = |s: &str| -> Option<f64> {
        if date_mode {
            s.parse::<f64>().ok().or_else(|| ExcelData::parse_date_string(s))
        } else {
            s.parse::<f64>().ok()
        }
    };

    let f1 = match parse(formula1) {
        Some(n) => n,
        None => return true, // 无法解析公式值，放行
    };

    match op {
        DataValidationOperator::Between => {
            if let Some(f2) = parse(formula2) { v >= f1 && v <= f2 } else { true }
        }
        DataValidationOperator::NotBetween => {
            if let Some(f2) = parse(formula2) { !(v >= f1 && v <= f2) } else { true }
        }
        DataValidationOperator::Equal => (v - f1).abs() < f64::EPSILON,
        DataValidationOperator::NotEqual => (v - f1).abs() >= f64::EPSILON,
        DataValidationOperator::GreaterThan => v > f1,
        DataValidationOperator::GreaterThanOrEqual => v >= f1,
        DataValidationOperator::LessThan => v < f1,
        DataValidationOperator::LessThanOrEqual => v <= f1,
    }
}

/// Excel 数据结构，包含整个工作簿的所有工作表
#[derive(Debug, Clone)]
pub struct ExcelData {
    /// 工作表列表
    pub sheets: Vec<SheetData>,
}

impl ExcelData {
    /// 从文件加载 Excel 数据
    /// 
    /// # 参数
    /// * `path` - Excel 文件路径
    /// 
    /// # 返回值
    /// 成功返回 ExcelData，失败返回错误信息
    pub fn load_from_file(path: &str) -> Result<Self, String> {
        let t0 = std::time::Instant::now();
        // 使用 umya-spreadsheet 库读取 Excel 文件
        let book = reader::xlsx::read(path)
            .map_err(|e| format!("读取失败: {}", e))?;
        let t1 = std::time::Instant::now();
        log::info!(
            "📂 加载 {}: umya XML 解析 {:.2}s，{} sheets, 文件大小 {}KB",
            path, t1.duration_since(t0).as_secs_f64(),
            book.sheet_collection().len(),
            std::fs::metadata(path).map(|m| m.len() / 1024).unwrap_or(0),
        );

        // 获取主题对象，用于解析主题颜色
        let theme = book.theme();

        let mut sheets = Vec::new();

        // 遍历工作簿中的所有工作表
        for (si, worksheet) in book.sheet_collection().iter().enumerate() {
            let sheet_t0 = std::time::Instant::now();
            // 创建工作表数据对象
            let mut sheet = SheetData::new(worksheet.name().to_string());

            // 使用库提供的方法动态获取工作表的最大行和最大列（去除硬编码限制）
            let highest_row = worksheet.highest_row();
            let highest_col = worksheet.highest_column();

            // 获取所有实际存在的单元格（仅非空单元格有 XML 记录）
            let cells = worksheet.cells();
            // 预分配 HashMap 容量，避免加载过程中的多次 rehash
            sheet.cells.reserve(cells.len());
            let cell_count = cells.len();

            // 多线程阈值：低于此数量直接顺序处理，减少线程调度开销
            const PARALLEL_THRESHOLD: usize = 5000;
            if cell_count >= PARALLEL_THRESHOLD {
                // ===== 多线程并行解析单元格 =====
                let num_threads = std::thread::available_parallelism()
                    .map(|n| n.get()).unwrap_or(1);
                let chunk_size = (cell_count + num_threads - 1) / num_threads;

                std::thread::scope(|s| {
                    let mut handles = Vec::with_capacity(num_threads);
                    for chunk in cells.chunks(chunk_size) {
                        let handle = s.spawn(move || {
                            // 预解析 ARGB 缓存 Key（避免 cache_key 与 parse_style 双重 argb_with_theme）
                            // 直接用已解析的 ARGB 字符串作为缓存 Key 的组成部分
                            type SK = (
                                Option<(u64, /*font_color_argb*/ u32, bool)>,       // font: (size_bits, color_hash, bold)
                                Option<u32>,                                        // bg argb hash
                                Option<[u32; 8]>,                                   // border: style+color hash per side: [left_style_hash, left_color_hash, right..., bottom...]
                                Option<(/*h_align*/ u8, /*v_align*/ u8)>,            // alignment compact
                                Option<u32>,                                        // num_fmt hash
                            );
                            let mut local_cells: std::collections::HashMap<(u32, u32), CellData> =
                                std::collections::HashMap::with_capacity(chunk.len());
                            let mut local_cache: std::collections::HashMap<SK, (CellAlignment, Option<(u8, u8, u8)>, Option<f64>, Option<(u8, u8, u8)>, Option<String>, bool, CellBorders)> =
                                std::collections::HashMap::new();
                            let mut local_formula_pos: std::collections::HashSet<(u32, u32)> =
                                std::collections::HashSet::new();

                            for cell in chunk {
                                let col_idx = cell.coordinate().col_num();
                                let row_idx = cell.coordinate().row_num();
                                let value = cell.value().to_string();
                                let raw_number = cell.value_number();
                                let style = cell.style();  // 直接用 cell 自带 style，无需 worksheet.style() 再次 HashMap 查找

                                // 预解析所有 ARGB 颜色，同时用于 cache key 和 parse_style
                                let font_argb = style.font()
                                    .map(|f| Self::resolve_color(&f.color(), theme));
                                let bg_argb = style.background_color()
                                    .map(|c| Self::resolve_color(c, theme));

                                // ParsedStyle: (font_argb_str, bg_argb_str, ...) for passing to parse
                                let cache_key: SK = (
                                    style.font().map(|f| (
                                        f.size().to_bits(),
                                        hash_str(font_argb.as_deref().unwrap_or("")),
                                        f.font_bold().val()
                                    )),
                                    bg_argb.as_ref().map(|s| hash_str(s)),
                                    style.borders().map(|b| {
                                        let ls = b.left().border_style();
                                        let rs = b.right().border_style();
                                        let ts = b.top().border_style();
                                        let bs = b.bottom().border_style();
                                        let lc = b.left().color().as_ref().map(|c| hash_str(&c.argb_with_theme(theme)));
                                        let rc = b.right().color().as_ref().map(|c| hash_str(&c.argb_with_theme(theme)));
                                        let tc = b.top().color().as_ref().map(|c| hash_str(&c.argb_with_theme(theme)));
                                        let bc = b.bottom().color().as_ref().map(|c| hash_str(&c.argb_with_theme(theme)));
                                        [
                                            hash_str(ls), lc.unwrap_or(0),
                                            hash_str(rs), rc.unwrap_or(0),
                                            hash_str(ts), tc.unwrap_or(0),
                                            hash_str(bs), bc.unwrap_or(0),
                                        ]
                                    }),
                                    style.alignment().map(|a| {
                                        (align_h_to_u8(a.horizontal()), align_v_to_u8(a.vertical()))
                                    }),
                                    style.number_format().map(|n| hash_str(n.format_code())),
                                );
                                let (alignment, background_color, font_size, font_color, number_format, bold, borders) =
                                    if let Some(cached) = local_cache.get(&cache_key) {
                                        cached.clone()
                                    } else {
                                        let parsed = Self::parse_style_from_argb(
                                            style, theme,
                                            font_argb.as_deref(),
                                            bg_argb.as_deref(),
                                        );
                                        local_cache.insert(cache_key, parsed.clone());
                                        parsed
                                    };

                                let formula_str = cell.formula().to_string();
                                let has_formula = !formula_str.is_empty();
                                if has_formula {
                                    local_formula_pos.insert((row_idx, col_idx));
                                }
                                local_cells.insert((row_idx, col_idx), CellData {
                                    value,
                                    raw_number,
                                    formula: formula_str,
                                    alignment,
                                    background_color,
                                    original_bg: background_color,
                                    font_size,
                                    font_color,
                                    number_format,
                                    bold,
                                    borders,
                                    comment: None,
                                });
                            }
                            (local_cells, local_formula_pos)
                        });
                        handles.push(handle);
                    }
                    // 合并各线程结果
                    for handle in handles {
                        if let Ok((local_cells, local_fp)) = handle.join() {
                            sheet.formula_positions.extend(local_fp);
                            sheet.cells.extend(local_cells);
                        }
                    }
                    sheet.formula_positions_dirty = false;
                });
            } else {
                // ===== 顺序解析（小文件） =====
                type StyleKey = (
                    Option<(u64, u32, bool)>,
                    Option<u32>,
                    Option<[u32; 8]>,
                    Option<(u8, u8)>,
                    Option<u32>,
                );
                let mut style_cache: std::collections::HashMap<StyleKey, (CellAlignment, Option<(u8, u8, u8)>, Option<f64>, Option<(u8, u8, u8)>, Option<String>, bool, CellBorders)> = std::collections::HashMap::new();

                for cell in cells {
                    let if_formula = !cell.formula().is_empty();
                    let col_idx = cell.coordinate().col_num();
                    let row_idx = cell.coordinate().row_num();
                    let value = cell.value().to_string();
                    let raw_number = cell.value_number();
                    let style = cell.style();

                    let font_argb = style.font()
                        .map(|f| Self::resolve_color(&f.color(), theme));
                    let bg_argb = style.background_color()
                        .map(|c| Self::resolve_color(c, theme));

                    let cache_key: StyleKey = (
                        style.font().map(|f| (
                            f.size().to_bits(),
                            hash_str(font_argb.as_deref().unwrap_or("")),
                            f.font_bold().val()
                        )),
                        bg_argb.as_ref().map(|s| hash_str(s)),
                        style.borders().map(|b| {
                            let ls = b.left().border_style();
                            let rs = b.right().border_style();
                            let ts = b.top().border_style();
                            let bs = b.bottom().border_style();
                            [
                                hash_str(ls), b.left().color().as_ref().map(|c| hash_str(&c.argb_with_theme(theme))).unwrap_or(0),
                                hash_str(rs), b.right().color().as_ref().map(|c| hash_str(&c.argb_with_theme(theme))).unwrap_or(0),
                                hash_str(ts), b.top().color().as_ref().map(|c| hash_str(&c.argb_with_theme(theme))).unwrap_or(0),
                                hash_str(bs), b.bottom().color().as_ref().map(|c| hash_str(&c.argb_with_theme(theme))).unwrap_or(0),
                            ]
                        }),
                        style.alignment().map(|a| {
                            (align_h_to_u8(a.horizontal()), align_v_to_u8(a.vertical()))
                        }),
                        style.number_format().map(|n| hash_str(n.format_code())),
                    );
                    let (alignment, background_color, font_size, font_color, number_format, bold, borders) =
                        if let Some(cached) = style_cache.get(&cache_key) {
                            cached.clone()
                        } else {
                            let parsed = Self::parse_style_from_argb(
                                style, theme,
                                font_argb.as_deref(),
                                bg_argb.as_deref(),
                            );
                            style_cache.insert(cache_key, parsed.clone());
                            parsed
                        };

                    let formula_str = cell.formula().to_string();
                    if if_formula {
                        sheet.formula_positions.insert((row_idx, col_idx));
                    }
                    sheet.cells.insert((row_idx, col_idx), CellData {
                        value,
                        raw_number,
                        formula: formula_str,
                        alignment,
                        background_color,
                        original_bg: background_color,
                        font_size,
                        font_color,
                        number_format,
                        bold,
                        borders,
                        comment: None,
                    });
                }
                sheet.formula_positions_dirty = false;
            }

            let sheet_t1 = std::time::Instant::now();
            log::info!(
                "  └ Sheet[{}] \"{}\": {} cells, cell_parse={:.2}s",
                si, sheet.name, cell_count, sheet_t1.duration_since(sheet_t0).as_secs_f64(),
            );

            // 设置工作表的最大行和最大列
            sheet.max_row = highest_row;
            sheet.max_col = highest_col;

            // 读取合并单元格信息
            for range in worksheet.merge_cells() {
                if let (Some(start_row), Some(start_col), Some(end_row), Some(end_col)) = (
                    range.coordinate_start_row(),
                    range.coordinate_start_col(),
                    range.coordinate_end_row(),
                    range.coordinate_end_col(),
                ) {
                    let start_row_num = start_row.num();
                    let start_col_num = start_col.num();
                    let end_row_num = end_row.num();
                    let end_col_num = end_col.num();
                    
                    // 确保合并范围在有效数据区域内
                    if end_col_num <= highest_col && end_row_num <= highest_row {
                        let cell_range = CellRange::new(
                            start_row_num,
                            start_col_num,
                            end_row_num,
                            end_col_num,
                        );
                        sheet.merged_cells.push(cell_range);
                    }
                }
            }

            // 读取列宽信息
            let mut col_index = 1;
            for col_dimension in worksheet.column_dimensions() {
                let width = col_dimension.width();
                // 只保存宽度大于0的列
                if width > 0.0 {
                    sheet.column_widths.insert(col_index, width);
                }
                col_index += 1;
            }

            // 读取行高信息
            for row_dimension in worksheet.row_dimensions() {
                let row_num = row_dimension.row_num();
                let height = row_dimension.height();
                // 只保存高度大于0的行
                if height > 0.0 {
                    sheet.row_heights.insert(row_num, height);
                }
            }

            // 解析冻结窗格信息
            if let Some(sheet_view) = worksheet.sheets_views().sheet_view_list().first() {
                if let Some(pane) = sheet_view.pane() {
                    use umya_spreadsheet::PaneStateValues;
                    let state = pane.state();
                    if matches!(state, PaneStateValues::Frozen | PaneStateValues::FrozenSplit) {
                        // umya-spreadsheet 命名与 OOXML 对照：
                        // horizontal_split → XML xSplit → 冻结列数（水平位置）
                        // vertical_split   → XML ySplit → 冻结行数（垂直位置）
                        sheet.frozen_cols = pane.horizontal_split() as u32;
                        sheet.frozen_rows = pane.vertical_split() as u32;
                    }
                }
            }

            // 读取数据有效性规则
            if let Some(dvs) = worksheet.data_validations() {
                for dv in dvs.data_validation_list() {
                    let show_input = dv.show_input_message();
                    let show_error = dv.show_error_message();
                    let title = dv.prompt_title().to_string();
                    let prompt = dv.prompt().to_string();
                    let err_title = dv.error_title().to_string();
                    let err_msg = dv.error_message().to_string();

                    if !show_input && !show_error { continue; }

                    let dv_type = match dv.get_type() {
                        umya_spreadsheet::DataValidationValues::Whole => DataValidationType::Whole,
                        umya_spreadsheet::DataValidationValues::Decimal => DataValidationType::Decimal,
                        umya_spreadsheet::DataValidationValues::List => DataValidationType::List,
                        umya_spreadsheet::DataValidationValues::Date => DataValidationType::Date,
                        umya_spreadsheet::DataValidationValues::Time => DataValidationType::Time,
                        umya_spreadsheet::DataValidationValues::TextLength => DataValidationType::TextLength,
                        umya_spreadsheet::DataValidationValues::Custom => DataValidationType::Custom,
                        _ => DataValidationType::None,
                    };
                    let dv_operator = match dv.operator() {
                        umya_spreadsheet::DataValidationOperatorValues::Between => DataValidationOperator::Between,
                        umya_spreadsheet::DataValidationOperatorValues::NotBetween => DataValidationOperator::NotBetween,
                        umya_spreadsheet::DataValidationOperatorValues::Equal => DataValidationOperator::Equal,
                        umya_spreadsheet::DataValidationOperatorValues::NotEqual => DataValidationOperator::NotEqual,
                        umya_spreadsheet::DataValidationOperatorValues::GreaterThan => DataValidationOperator::GreaterThan,
                        umya_spreadsheet::DataValidationOperatorValues::GreaterThanOrEqual => DataValidationOperator::GreaterThanOrEqual,
                        umya_spreadsheet::DataValidationOperatorValues::LessThan => DataValidationOperator::LessThan,
                        umya_spreadsheet::DataValidationOperatorValues::LessThanOrEqual => DataValidationOperator::LessThanOrEqual,
                    };

                    let mut ranges = Vec::new();
                    for range in dv.sequence_of_references().range_collection() {
                        let sc = range.coordinate_start_col().map(|c| c.num());
                        let sr = range.coordinate_start_row().map(|r| r.num());
                        let ec = range.coordinate_end_col().map(|c| c.num());
                        let er = range.coordinate_end_row().map(|r| r.num());
                        if let (Some(sc), Some(sr), Some(ec), Some(er)) = (sc, sr, ec, er) {
                            ranges.push(CellRange::new(sr, sc, er, ec));
                        }
                    }
                    if !ranges.is_empty() {
                        sheet.data_validations.push(DataValidationInfo {
                            prompt_title: title,
                            prompt,
                            error_title: err_title,
                            error_message: err_msg,
                            show_error_message: show_error,
                            dv_type,
                            dv_operator,
                            formula1: dv.formula1().to_string(),
                            formula2: dv.formula2().to_string(),
                            ranges,
                        });
                    }
                }
            }

            // 读取条件格式规则（CellIs 类型）
            for cf in worksheet.conditional_formatting_collection() {
                let mut ranges = Vec::new();
                for range in cf.sequence_of_references().range_collection() {
                    let sc = range.coordinate_start_col().map(|c| c.num());
                    let sr = range.coordinate_start_row().map(|r| r.num());
                    let ec = range.coordinate_end_col().map(|c| c.num());
                    let er = range.coordinate_end_row().map(|r| r.num());
                    if let (Some(sc), Some(sr), Some(ec), Some(er)) = (sc, sr, ec, er) {
                        ranges.push(CellRange::new(sr, sc, er, ec));
                    }
                }
                for rule in cf.conditional_collection() {
                    if let Some(style) = rule.style() {
                        let rule_type = rule.get_type().clone();
                        let op = rule.operator().clone();
                        let formula_text = rule.formula()
                            .map(|f| f.address_str())
                            .unwrap_or_default();
                        let mut bg = None;
                        let mut fc = None;
                        let mut b = false;
                        // dxf 填充色可能在 fgColor 或 bgColor 中，两者都检查
                        let bg_color = style.background_color()
                            .or_else(|| style.fill()
                                .and_then(|f| f.pattern_fill()?.background_color()));
                        if let Some(bg_color) = bg_color {
                            let resolved = bg_color.argb_with_theme(theme);
                            if let Ok(rgb) = Self::parse_hex_color(&resolved) {
                                bg = Some(rgb);
                            }
                        }
                        if let Some(font) = style.font() {
                            let resolved = font.color().argb_with_theme(theme);
                            if let Ok(rgb) = Self::parse_hex_color(&resolved) {
                                fc = Some(rgb);
                            }
                            b = font.bold();
                        }
                        sheet.conditional_rules.push(CondFormatRule {
                            ranges: ranges.clone(),
                            rule_type: format!("{:?}", rule_type),
                            operator: format!("{:?}", op),
                            formula_text,
                            text: rule.text().to_string(),
                            bg_color: bg,
                            font_color: fc,
                            bold: b,
                        });
                    }
                }
            }

            // 读取单元格批注（Comment）：作者 + 富文本/纯文本
            // 用 entry().or_insert_with() 兼容「仅有批注、无 <c> 记录」的空单元格
            for comment in worksheet.comments() {
                let col = comment.coordinate().col_num();
                let row = comment.coordinate().row_num();
                let author = comment.author().to_string();
                let text = extract_comment_text(comment.text());
                let cell = sheet.cells.entry((row, col)).or_insert_with(CellData::default);
                cell.comment = Some(CellComment { author, text });
            }

            // 将工作表添加到列表中
            sheets.push(sheet);
        }

        // 检查是否有工作表
        if sheets.is_empty() {
            return Err("Excel文件中没有找到工作表".to_string());
        }

        let t_formula = std::time::Instant::now();
        // 文件加载后并行求值所有公式（每 sheet 独立，互不依赖）
        // 条件格式延后到首帧渲染时惰性求值（viewer 已支持 cf_dirty 机制）
        if sheets.len() > 1 {
            std::thread::scope(|s| {
                for sheet in &mut sheets {
                    s.spawn(move || {
                        sheet.rebuild_merge_index();
                        crate::excel::formula::evaluate_sheet(sheet);
                    });
                }
            });
        } else {
            for sheet in &mut sheets {
                sheet.rebuild_merge_index();
                crate::excel::formula::evaluate_sheet(sheet);
            }
        }
        for (si, sheet) in sheets.iter().enumerate() {
            let formula_count = sheet.formula_positions.len();
            if formula_count > 0 {
                log::info!(
                    "  ⚡ Sheet[{}] {}: {} formulas evaluated (CF deferred)",
                    si, sheet.name, formula_count,
                );
            }
        }
        let t_total = std::time::Instant::now();
        log::info!(
            "✅ 加载完成: 总 {:.2}s (umya={:.1}s, formula={:.1}s), {} sheets",
            t_total.duration_since(t0).as_secs_f64(),
            t1.duration_since(t0).as_secs_f64(),
            t_total.duration_since(t_formula).as_secs_f64(),
            sheets.len(),
        );

        Ok(Self { sheets })
    }

    // ────────────────────────────────────────────────
    // 性能辅助函数
    // ────────────────────────────────────────────────

    /// 简单的字符串哈希（用于缓存 Key），比 `to_string()` 更轻量，避免 String 分配。
    #[inline]
    fn resolve_color(color: &umya_spreadsheet::Color, theme: &umya_spreadsheet::drawing::Theme) -> Cow<'static, str> {
        color.argb_with_theme(theme)
    }
}

// ────────────────────────────────────────────────
// 独立辅助函数（非 impl 块）
// ────────────────────────────────────────────────

/// 快速字符串哈希（fnv-like，避免 String 分配）
#[inline]
fn hash_str(s: &str) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for b in s.bytes() {
        h = h.wrapping_mul(0x01000193).wrapping_add(b as u32);
    }
    h
}

/// 水平对齐 → u8（用枚举值直接匹配，避免字符串比较）
#[inline]
fn align_h_to_u8(h: &umya_spreadsheet::HorizontalAlignmentValues) -> u8 {
    match h {
        umya_spreadsheet::HorizontalAlignmentValues::Left => 1,
        umya_spreadsheet::HorizontalAlignmentValues::Center => 2,
        umya_spreadsheet::HorizontalAlignmentValues::Right => 3,
        umya_spreadsheet::HorizontalAlignmentValues::Fill => 4,
        umya_spreadsheet::HorizontalAlignmentValues::Justify => 5,
        umya_spreadsheet::HorizontalAlignmentValues::CenterContinuous => 6,
        umya_spreadsheet::HorizontalAlignmentValues::Distributed => 7,
        _ => 0,
    }
}

/// 垂直对齐 → u8
#[inline]
fn align_v_to_u8(v: &umya_spreadsheet::VerticalAlignmentValues) -> u8 {
    match v {
        umya_spreadsheet::VerticalAlignmentValues::Top => 1,
        umya_spreadsheet::VerticalAlignmentValues::Center => 2,
        umya_spreadsheet::VerticalAlignmentValues::Justify => 3,
        umya_spreadsheet::VerticalAlignmentValues::Distributed => 4,
        _ => 0,
    }
}

/// 快速水平对齐解析（直接用枚举值匹配，零分配）
#[inline]
fn quick_h_align(h: &umya_spreadsheet::HorizontalAlignmentValues) -> HorizontalAlignment {
    match h {
        umya_spreadsheet::HorizontalAlignmentValues::Left => HorizontalAlignment::Left,
        umya_spreadsheet::HorizontalAlignmentValues::Center => HorizontalAlignment::Center,
        umya_spreadsheet::HorizontalAlignmentValues::Right => HorizontalAlignment::Right,
        umya_spreadsheet::HorizontalAlignmentValues::Fill => HorizontalAlignment::Fill,
        umya_spreadsheet::HorizontalAlignmentValues::Justify => HorizontalAlignment::Justify,
        umya_spreadsheet::HorizontalAlignmentValues::CenterContinuous => HorizontalAlignment::CenterContinuous,
        umya_spreadsheet::HorizontalAlignmentValues::Distributed => HorizontalAlignment::Distributed,
        _ => HorizontalAlignment::General,
    }
}

/// 快速垂直对齐解析
#[inline]
fn quick_v_align(v: &umya_spreadsheet::VerticalAlignmentValues) -> VerticalAlignment {
    match v {
        umya_spreadsheet::VerticalAlignmentValues::Top => VerticalAlignment::Top,
        umya_spreadsheet::VerticalAlignmentValues::Center => VerticalAlignment::Center,
        umya_spreadsheet::VerticalAlignmentValues::Justify => VerticalAlignment::Justify,
        umya_spreadsheet::VerticalAlignmentValues::Distributed => VerticalAlignment::Distributed,
        _ => VerticalAlignment::Bottom,
    }
}

impl ExcelData {
    /// 解析 Excel 单元格样式（使用预解析的 ARGB，避免重复 argb_with_theme）
    fn parse_style_from_argb(
        style: &umya_spreadsheet::Style,
        theme: &umya_spreadsheet::drawing::Theme,
        resolved_font_color: Option<&str>,
        resolved_bg_color: Option<&str>,
    ) -> (CellAlignment, Option<(u8, u8, u8)>, Option<f64>, Option<(u8, u8, u8)>, Option<String>, bool, CellBorders) {
        let mut raw_style = Self::parse_style_raw(style, theme, resolved_font_color, resolved_bg_color);
        // 如果 pre-resolved bg 为 None，但 style 自带背景色，则回退到 argb_with_theme
        if raw_style.1.is_none() {
            if let Some(bg_color) = style.background_color() {
                if resolved_bg_color.is_none() {
                    let resolved = bg_color.argb_with_theme(theme);
                    if !resolved.is_empty() && resolved != "00000000" {
                        if let Ok(rgb) = Self::parse_hex_color(&resolved) {
                            raw_style.1 = Some(rgb);
                        }
                    }
                }
            }
        }
        raw_style
    }

    /// 解析 Excel 单元格样式（不调用 argb_with_theme 的颜色部分，改用预解析值）
    fn parse_style_raw(
        style: &umya_spreadsheet::Style,
        theme: &umya_spreadsheet::drawing::Theme,
        resolved_font_color: Option<&str>,
        resolved_bg_color: Option<&str>,
    ) -> (CellAlignment, Option<(u8, u8, u8)>, Option<f64>, Option<(u8, u8, u8)>, Option<String>, bool, CellBorders) {
        let mut alignment = CellAlignment::default();
        let mut background_color: Option<(u8, u8, u8)> = None;
        let mut font_size: Option<f64> = None;
        let mut font_color: Option<(u8, u8, u8)> = None;
        let mut number_format: Option<String> = None;
        let mut bold = false;
        let mut borders = CellBorders::default();

        // 解析对齐方式（用 u8 快速映射，避免字符串比较）
        if let Some(align) = style.alignment() {
            let (h, v) = (align.horizontal(), align.vertical());
            alignment.horizontal = quick_h_align(h);
            alignment.vertical = quick_v_align(v);
        }

        // 解析背景颜色（使用预解析值）
        if let Some(resolved) = resolved_bg_color {
            if !resolved.is_empty() && resolved != "00000000" {
                if let Ok(rgb) = Self::parse_hex_color(resolved) {
                    background_color = Some(rgb);
                }
            }
        }

        // 解析字体信息
        if let Some(font) = style.font() {
            font_size = Some(font.size());
            // 字体颜色（使用预解析值）
            if let Some(resolved) = resolved_font_color {
                if !resolved.is_empty() && resolved != "00000000" {
                    if let Ok(rgb) = Self::parse_hex_color(resolved) {
                        font_color = Some(rgb);
                    }
                }
            }
            bold = font.font_bold().val();
        }

        // 解析边框（使用 argb_with_theme 但已预解析颜色）
        if let Some(style_borders) = style.borders() {
            let parse_border = |border: &umya_spreadsheet::structs::Border| -> CellBorder {
                let mut style_str = String::new();
                let mut color = None;
                let bs = border.border_style();
                if !bs.is_empty() && bs != "none" {
                    style_str = bs.to_string();
                }
                if let Some(border_color) = border.color() {
                    let resolved = border_color.argb_with_theme(theme);
                    if !resolved.is_empty() && resolved != "00000000" {
                        if let Ok(rgb) = Self::parse_hex_color(&resolved) {
                            color = Some(rgb);
                        }
                    }
                }
                CellBorder { style: style_str, color }
            };
            borders.left = parse_border(style_borders.left());
            borders.right = parse_border(style_borders.right());
            borders.top = parse_border(style_borders.top());
            borders.bottom = parse_border(style_borders.bottom());
        }

        // 解析数字格式
        if let Some(num_fmt) = style.number_format() {
            let fmt = num_fmt.format_code();
            if !fmt.is_empty() && fmt != "General" {
                number_format = Some(fmt.to_string());
            }
        }

        (alignment, background_color, font_size, font_color, number_format, bold, borders)
    }

    /// 将十六进制颜色字符串转换为 RGB 元组
    /// 
    /// 支持两种格式：
    /// - 6位 RGB 格式：RRGGBB
    /// - 8位 ARGB 格式：AARRGGBB（忽略 alpha 通道）
    /// 
    /// # 参数
    /// * `hex_str` - 十六进制颜色字符串（可带或不带 # 前缀）
    /// 
    /// # 返回值
    /// 成功返回 RGB 元组 (r, g, b)，失败返回 Err(())
    fn parse_hex_color(hex_str: &str) -> Result<(u8, u8, u8), ()> {
        let hex_str = hex_str.trim_start_matches('#');

        // 将 hex 字符串解析为 u32 数值，再提取 RGB 分量（容错 calc_tint 溢出）
        let parse_rgb = |s: &str| -> (u8, u8, u8) {
            // 取有效 hex 字符，截断到 8 位（处理非标准长度）
            let clean: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
            // 取末尾部分（6 位 RRGGBB 或 8 位 AARRGGBB）
            let val_str = if clean.len() >= 8 {
                &clean[clean.len() - 8..]
            } else if clean.len() >= 6 {
                &clean[clean.len() - 6..]
            } else {
                &format!("{:0>6}", clean)
            };

            let val = u32::from_str_radix(val_str, 16).unwrap_or(0);
            if val_str.len() >= 8 {
                // AARRGGBB
                (
                    ((val >> 16) & 0xFF).min(255) as u8,
                    ((val >> 8) & 0xFF).min(255) as u8,
                    (val & 0xFF).min(255) as u8,
                )
            } else {
                // RRGGBB
                (
                    ((val >> 16) & 0xFF).min(255) as u8,
                    ((val >> 8) & 0xFF).min(255) as u8,
                    (val & 0xFF).min(255) as u8,
                )
            }
        };

        let (r, g, b) = parse_rgb(hex_str);
        Ok((r, g, b))
    }

    /// 获取或构建条件格式列级索引（缓存，规则变更时自动重建）
    fn get_cf_col_index(sheet: &mut SheetData) -> &std::collections::HashMap<u32, Vec<usize>> {
        if sheet.cf_col_index_dirty || sheet.cf_col_index.is_none() {
            let mut col_rules: std::collections::HashMap<u32, Vec<usize>> =
                std::collections::HashMap::new();
            for (ri, rule) in sheet.conditional_rules.iter().enumerate() {
                for range in &rule.ranges {
                    for col in range.start_col..=range.end_col {
                        col_rules.entry(col).or_default().push(ri);
                    }
                }
            }
            sheet.cf_col_index = Some(col_rules);
            sheet.cf_col_index_dirty = false;
        }
        sheet.cf_col_index.as_ref().unwrap()
    }

    /// 对已加载的 SheetData 应用条件格式规则。
    /// 目前支持 CellIs 类型，将匹配的 dxf 样式覆盖到对应单元格上。
    /// 公开入口：每帧重新求值文件自带的条件格式（仅覆盖规则声明的 sqref 范围）
    pub fn reapply_conditional_formatting(sheet: &mut SheetData) {
        if sheet.conditional_rules.is_empty() {
            return;
        }
        // 获取或构建列级索引
        let col_rules = Self::get_cf_col_index(sheet).clone();

        // 增量路径：仅重算少数被修改的单元格（粘贴/编辑小范围变更）
        if let Some(dirty_cells) = sheet.cf_dirty_cells.take() {
            if !dirty_cells.is_empty() {
                for (col, row) in dirty_cells {
                    // 恢复原始样式
                    if let Some(cell) = sheet.cells.get_mut(&(row, col)) {
                        cell.background_color = cell.original_bg;
                        cell.font_color = None;
                        cell.bold = false;
                    }
                    // 重新求值
                    let cell_value = sheet.cells.get(&(row, col))
                        .map(|c| c.value.clone())
                        .unwrap_or_default();
                    if let Some(rule_indices) = col_rules.get(&col) {
                        for &ri in rule_indices {
                            let rule = &sheet.conditional_rules[ri];
                            let in_range = rule.ranges.iter().any(|r| r.contains(col, row));
                            if in_range && Self::evaluate_rule(rule, &cell_value) {
                                if let Some(cell) = sheet.cells.get_mut(&(row, col)) {
                                    if let Some(bg) = rule.bg_color {
                                        cell.background_color = Some(bg);
                                    }
                                    if let Some(fc) = rule.font_color {
                                        cell.font_color = Some(fc);
                                    }
                                    if rule.bold {
                                        cell.bold = true;
                                    }
                                }
                            }
                        }
                    }
                }
                log::debug!("  🎨 CF incremental: {} cells", 1); // simplified count
                return;
            }
        }

        // 全量路径：遍历所有单元格（初始加载 / 公式变更 / 规则变更）
        let keys: Vec<(u32, u32)> = sheet.cells.keys().copied().collect();
        for (row, col) in keys {
            // 检查该格是否在任意 CF 规则范围内
            let in_any_range = col_rules.get(&col).map_or(false, |rules| {
                rules.iter().any(|&ri| {
                    sheet.conditional_rules[ri]
                        .ranges
                        .iter()
                        .any(|r| r.contains(col, row))
                })
            });
            if !in_any_range {
                continue;
            }
            // 恢复原始样式
            if let Some(cell) = sheet.cells.get_mut(&(row, col)) {
                cell.background_color = cell.original_bg;
                cell.font_color = None;
                cell.bold = false;
            }
            // 重新求值应用 CF 规则
            let cell_value = sheet.cells.get(&(row, col))
                .map(|c| c.value.clone())
                .unwrap_or_default();
            if let Some(rule_indices) = col_rules.get(&col) {
                for &ri in rule_indices {
                    let rule = &sheet.conditional_rules[ri];
                    let in_range = rule.ranges.iter().any(|r| r.contains(col, row));
                    if in_range && Self::evaluate_rule(rule, &cell_value) {
                        if let Some(cell) = sheet.cells.get_mut(&(row, col)) {
                            if let Some(bg) = rule.bg_color {
                                cell.background_color = Some(bg);
                            }
                            if let Some(fc) = rule.font_color {
                                cell.font_color = Some(fc);
                            }
                            if rule.bold {
                                cell.bold = true;
                            }
                        }
                    }
                }
            }
        }
    }

    /// 将单元格值转为数值（空值当作 0，匹配 WPS/Excel 行为）
    fn parse_cell_number(cell_value: &str) -> Option<f64> {
        if cell_value.trim().is_empty() {
            Some(0.0)
        } else {
            cell_value.parse::<f64>().ok()
        }
    }

    fn evaluate_rule(rule: &CondFormatRule, cell_value: &str) -> bool {
        if rule.rule_type.contains("ContainsText") {
            let search = if rule.text.is_empty() {
                Self::extract_contains_text_from_formula(&rule.formula_text)
            } else {
                rule.text.clone()
            };
            cell_value.contains(&search)
        } else if rule.operator.contains("GreaterThan") {
            let threshold: f64 = rule.formula_text.parse().unwrap_or(f64::MAX);
            Self::parse_cell_number(&cell_value).map_or(false, |v| v > threshold)
        } else if rule.operator.contains("GreaterThanOrEqual") {
            let threshold: f64 = rule.formula_text.parse().unwrap_or(f64::MAX);
            Self::parse_cell_number(&cell_value).map_or(false, |v| v >= threshold)
        } else if rule.operator.contains("LessThan") {
            let threshold: f64 = rule.formula_text.parse().unwrap_or(f64::MIN);
            Self::parse_cell_number(&cell_value).map_or(false, |v| v < threshold)
        } else if rule.operator.contains("LessThanOrEqual") {
            let threshold: f64 = rule.formula_text.parse().unwrap_or(f64::MIN);
            Self::parse_cell_number(&cell_value).map_or(false, |v| v <= threshold)
        } else if rule.operator.contains("Equal") {
            let threshold = &rule.formula_text;
            cell_value == *threshold
        } else if rule.operator.contains("NotEqual") {
            let threshold = &rule.formula_text;
            cell_value != *threshold
        } else if rule.operator.contains("Between") {
            let parts: Vec<&str> = rule.formula_text.split(',').collect();
            if parts.len() >= 2 {
                let lo: f64 = parts[0].trim().parse().unwrap_or(f64::MIN);
                let hi: f64 = parts[1].trim().parse().unwrap_or(f64::MAX);
                Self::parse_cell_number(&cell_value).map_or(false, |v| v >= lo && v <= hi)
            } else {
                false
            }
        } else {
            false
        }
    }

    /// 解析动态范围引用：~行号 → 该行最大列，列字母~ → 该列最大行
    fn resolve_dynamic_range(range_str: &str, sheet: &SheetData) -> String {
        if !range_str.contains('~') {
            return range_str.to_string();
        }
        let parts: Vec<&str> = range_str.split(':').collect();
        let resolve_part = |s: &str| -> String {
            let s = s.trim();
            if s == "~" {
                // 纯 ~ → 最右下角
                format!("{}{}", col_to_letter(sheet.max_col.max(1)), sheet.max_row.max(1))
            } else if s.starts_with('~') {
                // ~行号 → 该行最大列
                let row: u32 = s[1..].parse().unwrap_or(1);
                format!("{}{}", col_to_letter(sheet.max_col.max(1)), row)
            } else if s.ends_with('~') {
                // 列字母~ → 该列最大行
                let col_letters: String = s.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
                format!("{}{}", col_letters, sheet.max_row.max(1))
            } else {
                s.to_string()
            }
        };
        if parts.len() == 2 {
            format!("{}:{}", resolve_part(parts[0]), resolve_part(parts[1]))
        } else {
            resolve_part(range_str)
        }
    }

/// 从 containsText 公式中提取搜索文本。
/// 公式格式: NOT(ISERROR(SEARCH("文本",cell_ref)))
fn extract_contains_text_from_formula(formula: &str) -> String {
    // 找到 SEARCH(" 的位置，提取引号内的文本
    if let Some(start) = formula.find("SEARCH(\"") {
        let after = &formula[start + 8..]; // skip "SEARCH("
        if let Some(end) = after.find('\"') {
            return after[..end].to_string();
        }
        // 单引号变体 SEARCH('文本'
        if let Some(end) = after.find('\'') {
            return after[..end].to_string();
        }
    }
    // 简化版: 提取公式中第一个引号内的文本
    if let Some(start) = formula.find('"') {
        let after = &formula[start + 1..];
        if let Some(end) = after.find('"') {
            return after[..end].to_string();
        }
    }
    String::new()
}

    /// 相等比较：优先数值，回退字符串
    fn compare_equal(cell_value: &str, threshold: &str) -> bool {
        if let (Some(cv), Some(tv)) = (Self::parse_cell_number(&cell_value), threshold.parse::<f64>().ok()) {
            (cv - tv).abs() < f64::EPSILON
        } else {
            cell_value.trim().to_lowercase() == threshold.trim().to_lowercase()
        }
    }

    /// 应用用户自定义的条件格式规则（来自 YAML 配置），到已加载的 SheetData。
    pub fn apply_user_cond_format_rules(sheet: &mut SheetData, user_rules: &[UserCondFormatRule]) {
        for rule in user_rules {
            // 解析 HEX 颜色
            let bg = Self::parse_hex_color(rule.color.trim_start_matches('#')).ok();

            // 解析范围: =$G$3:$G$154 → G3:G154
            // 支持动态引用: ~7(行尾列), B~(列尾行)
            let range_str = rule.range.trim_start_matches('=').replace('$', "");
            let range_str = Self::resolve_dynamic_range(&range_str, sheet);

            let parts: Vec<&str> = range_str.split(':').collect();

            let (start_col, start_row, end_col, end_row) = if parts.len() == 2 {
                let start = Self::parse_cell_ref_str(parts[0]);
                let end = Self::parse_cell_ref_str(parts[1]);
                match (start, end) {
                    (Ok((sc, sr)), Ok((ec, er))) => (sc, sr, ec, er),
                    _ => continue,
                }
            } else if parts.len() == 1 {
                // 单个单元格
                match Self::parse_cell_ref_str(parts[0]) {
                    Ok((c, r)) => (c, r, c, r),
                    Err(_) => continue,
                }
            } else {
                continue;
            };

            // 求值并应用（稀疏遍历：只处理该范围内已存在单元格，避免按范围面积遍历空格）
            let threshold_val: f64 = rule.value.parse().unwrap_or(0.0);
            let keys: Vec<(u32, u32)> = sheet
                .cells
                .keys()
                .copied()
                .filter(|&(row, col)| {
                    row >= start_row && row <= end_row && col >= start_col && col <= end_col
                })
                .collect();
            for (row, col) in keys {
                let cell_value = match sheet.cells.get(&(row, col)) {
                    Some(c) => c.value.clone(),
                    None => continue,
                };
                let matches = match rule.operator.as_str() {
                    ">" => Self::parse_cell_number(&cell_value).map_or(false, |v| v > threshold_val),
                    "<" => Self::parse_cell_number(&cell_value).map_or(false, |v| v < threshold_val),
                    "=" => Self::compare_equal(&cell_value, &rule.value),
                    ">=" => Self::parse_cell_number(&cell_value).map_or(false, |v| v >= threshold_val),
                    "<=" => Self::parse_cell_number(&cell_value).map_or(false, |v| v <= threshold_val),
                    "!=" => !Self::compare_equal(&cell_value, &rule.value),
                    _ => false,
                };
                if matches {
                    if let Some(cell) = sheet.cells.get_mut(&(row, col)) {
                        if let Some(bg) = bg {
                            cell.background_color = Some(bg);
                        }
                    }
                }
            }
        }
    }

    /// 解析单元格引用字符串如 "G3" → (col, row)
    fn parse_cell_ref_str(s: &str) -> Result<(u32, u32), ()> {
        let s = s.trim().to_uppercase();
        let col_part: String = s.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        let row_part: String = s.chars().skip(col_part.len()).collect();
        if col_part.is_empty() || row_part.is_empty() {
            return Err(());
        }
        let col = crate::excel::formula::letter_to_col(&col_part).map_err(|_| ())?;
        let row: u32 = row_part.parse().map_err(|_| ())?;
        Ok((col, row))
    }

    /// 检测数字格式代码是否为日期格式
    ///
    /// Excel 日期格式包含 y(年)、d(日) 等标记，或包含 m(月) 但不包含 h(时)/s(秒)
    pub fn is_date_format(fmt: &str) -> bool {
        if fmt.is_empty() { return false; }
        // 只取分号前的第一个格式段
        let fmt_part = fmt.split(';').next().unwrap_or(fmt);
        // 去除方括号区域（如 [$-404]）和引号内容
        let mut cleaned = String::new();
        let mut in_bracket = false;
        let mut in_quote = false;
        for ch in fmt_part.chars() {
            match ch {
                '[' => in_bracket = true,
                ']' => { in_bracket = false; continue; }
                '"' => { in_quote = !in_quote; continue; }
                _ if in_bracket || in_quote => continue,
                _ => cleaned.push(ch),
            }
        }
        let lower = cleaned.to_lowercase();
        let has_year = lower.contains('y') || lower.contains('e');
        let has_day = lower.contains('d');
        let has_month = lower.contains('m') && !lower.contains('h') && !lower.contains('s');
        has_year || has_day || has_month
    }

    /// 将 Excel 日期序列号转换为格式化日期字符串
    ///
    /// # 参数
    /// * `serial` - Excel 日期序列号（如 46927）
    /// * `fmt` - 数字格式代码（如 "yyyy/m/d"），用于确定输出格式
    pub fn format_date(serial: f64, fmt: &str) -> String {
        let (year, month, day) = Self::serial_to_date(serial);
        // 只取分号前的第一个格式段（忽略 ;@ 等文本格式段）
        let fmt_part = fmt.split(';').next().unwrap_or(fmt);
        let lower = fmt_part.to_lowercase();
        // 去除方括号区域内容，但保留引号内的字面文本（仅去除引号标记本身）
        let cleaned;
        {
            let mut s = String::new();
            let mut in_bracket = false;
            let mut in_quote = false;
            for ch in lower.chars() {
                match ch {
                    '[' => in_bracket = true,
                    ']' => { in_bracket = false; continue; }
                    '"' => { in_quote = !in_quote; continue; }
                    _ if in_bracket => continue,
                    _ => s.push(ch), // in_quote 时保留字面文本（如 年、月、日）
                }
            }
            cleaned = s;
        }

        // 检测美式日期格式模式 (m/d/yyyy, mm/dd/yyyy 等)，重新排列为 yyyy/m/d
        // 中文 Excel 会按 locale 覆盖标准格式的显示顺序
        let cleaned = Self::normalize_date_format_order(&cleaned);

        // 逐字符解析格式生成输出
        let mut result = String::new();
        let chars: Vec<char> = cleaned.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                'y' => {
                    let count = Self::count_consecutive(&chars, i, 'y');
                    if count >= 4 { result.push_str(&format!("{}", year)); }
                    else if count >= 2 { result.push_str(&format!("{:02}", year % 100)); }
                    else { result.push_str(&format!("{}", year % 100)); }
                    i += count;
                }
                'm' => {
                    let count = Self::count_consecutive(&chars, i, 'm');
                    if count >= 3 {
                        // mmm 简写月份名
                        let names = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];
                        result.push_str(names.get(month as usize - 1).unwrap_or(&""));
                    } else if count >= 2 {
                        result.push_str(&format!("{:02}", month));
                    } else {
                        result.push_str(&format!("{}", month));
                    }
                    i += count;
                }
                'd' => {
                    let count = Self::count_consecutive(&chars, i, 'd');
                    if count >= 3 {
                        // 已知1970-01-01是周四，用 Unix days mod 7 计算星期几
                        let unix_days = (serial as i64) - 25569;
                        let dow = ((unix_days % 7 + 7) % 7) as usize; // 0=Thu
                        let names = ["Thu","Fri","Sat","Sun","Mon","Tue","Wed"];
                        result.push_str(names.get(dow).unwrap_or(&""));
                    } else if count >= 2 {
                        result.push_str(&format!("{:02}", day));
                    } else {
                        result.push_str(&format!("{}", day));
                    }
                    i += count;
                }
                _ => {
                    result.push(chars[i]);
                    i += 1;
                }
            }
        }
        if result.is_empty() {
            format!("{}/{}/{}", year, month, day)
        } else {
            result
        }
    }

    /// 将美式日期格式 (m/d/yyyy) 重新排列为中文格式 (yyyy/m/d)
    /// 检测格式中以 m/d 开头、以 yyyy 结尾的模式，将年份移到前面
    fn normalize_date_format_order(fmt: &str) -> String {
        let chars: Vec<char> = fmt.chars().collect();
        // 找到 y/m/d 各段的位置和长度
        let mut y_pos: Option<(usize, usize)> = None; // (start, count)
        let mut m_pos: Option<(usize, usize)> = None;
        let mut d_pos: Option<(usize, usize)> = None;
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == 'y' {
                let start = i;
                let count = Self::count_consecutive(&chars, i, 'y');
                y_pos = Some((start, count));
                i += count;
            } else if chars[i] == 'm' {
                let start = i;
                let count = Self::count_consecutive(&chars, i, 'm');
                m_pos = Some((start, count));
                i += count;
            } else if chars[i] == 'd' {
                let start = i;
                let count = Self::count_consecutive(&chars, i, 'd');
                d_pos = Some((start, count));
                i += count;
            } else {
                i += 1;
            }
        }

        // 如果格式同时包含 y、m、d，检查是否需要重排
        if let (Some((y_start, y_count)), Some((m_start, m_count)), Some((d_start, d_count))) = (y_pos, m_pos, d_pos) {
            // 如果 m 在 y 前面（美式格式如 m/d/yyyy），则重排为 yyyy/m/d
            if m_start < y_start {
                // 提取分隔符（m 和 d 之间的字符）
                let sep_idx = m_start + m_count;
                let sep = if sep_idx < chars.len() && chars[sep_idx] != 'd' {
                    chars[sep_idx]
                } else {
                    '/'
                };
                // 构建新格式: yyyy + sep + mm + sep + dd
                let y_part: String = chars[y_start..y_start + y_count].iter().collect();
                let m_part: String = chars[m_start..m_start + m_count].iter().collect();
                let d_part: String = chars[d_start..d_start + d_count].iter().collect();
                return format!("{}{}{}{}{}", y_part, sep, m_part, sep, d_part);
            }
        }
        fmt.to_string()
    }

    /// Excel 序列号转公历日期 (year, month, day)
    pub fn serial_to_date(serial: f64) -> (u32, u32, u32) {
        // Excel serial 1 = 1900-01-01, 但 Excel 错误地认为 1900-02-29 存在 (serial=60)
        // Unix epoch 1970-01-01 = Excel serial 25569
        let unix_days = (serial as i64) - 25569;
        // Howard Hinnant 的 civil_from_days 算法
        let z = unix_days + 719468;
        let era = if z >= 0 { z / 146097 } else { (z - 146096) / 146097 };
        let doe = z - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
        let y = if m <= 2 { y + 1 } else { y };
        (y as u32, m, d)
    }

    /// 将 (year, month, day) 转换为 Excel 日期序列号
    pub fn date_to_serial(year: u32, month: u32, day: u32) -> f64 {
        // Howard Hinnant 的 civil_from_days 逆算法
        let y = if month <= 2 { year as i64 - 1 } else { year as i64 };
        let m = if month <= 2 { month as i64 + 9 } else { month as i64 - 3 };
        let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
        let yoe = y - era * 400;
        let doy = (153 * m + 2) / 5 + day as i64 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let unix_days = era * 146097 + doe - 719468;
        (unix_days + 25569) as f64
    }

    /// 尝试将格式化日期字符串解析回序列号
    /// 支持格式: yyyy/m/d, yyyy/m/dd, yyyy/mm/d, yyyy/mm/dd,
    ///           yyyy年m月d日, m/d/yyyy, m/d/yy 等
    pub fn parse_date_string(s: &str) -> Option<f64> {
        let s = s.trim();
        // 尝试用常见分隔符分割
        let owned;
        let parts: Vec<&str> = if s.contains('年') {
            // yyyy年m月d日 格式
            owned = s.replace("年", "/").replace("月", "/").replace("日", "");
            owned.split('/').collect()
        } else {
            s.split(|c: char| c == '/' || c == '-').collect()
        };
        if parts.len() != 3 { return None; }
        let nums: Vec<Option<u32>> = parts.iter().map(|p| p.trim().parse::<u32>().ok()).collect();
        if nums.iter().any(|n| n.is_none()) { return None; }
        let nums: Vec<u32> = nums.into_iter().map(|n| n.unwrap()).collect();
        // 判断格式：如果第一个数 > 31，认为是年份
        let (year, month, day) = if nums[0] > 31 {
            (nums[0], nums[1], nums[2])
        } else if nums[2] > 31 {
            (nums[2], nums[0], nums[1])
        } else if nums[2] >= 100 {
            // m/d/yy 或 m/d/yyyy
            let y = if nums[2] < 100 { 2000 + nums[2] } else { nums[2] };
            (y, nums[0], nums[1])
        } else {
            // 无法确定，假设 yyyy/m/d
            (nums[0], nums[1], nums[2])
        };
        if month == 0 || month > 12 || day == 0 || day > 31 { return None; }
        Some(Self::date_to_serial(year, month, day))
    }

    /// 计算从位置 i 开始连续出现字符 c 的次数
    fn count_consecutive(chars: &[char], start: usize, c: char) -> usize {
        let mut count = 0;
        let mut i = start;
        while i < chars.len() && chars[i] == c {
            count += 1;
            i += 1;
        }
        count
    }

    /// 根据索引获取工作表
    ///
    /// # 参数
    /// * `index` - 工作表索引（从0开始）
    ///
    /// # 返回值
    /// 成功返回 Some(&SheetData)，索引越界返回 None
    pub fn get_sheet(&self, index: usize) -> Option<&SheetData> {
        self.sheets.get(index)
    }
}

/// 从批注文本对象中提取完整字符串。
///
/// 批注文本可能是纯文本（`<t>`）或富文本（多个 `<r>` run，作者名通常为首段）。
/// 两者都存在时拼接：纯文本 + 各富文本 run。
fn extract_comment_text(ct: &umya_spreadsheet::structs::CommentText) -> String {
    let mut s = String::new();
    if let Some(t) = ct.text() {
        s.push_str(t.value());
    }
    if let Some(rt) = ct.rich_text() {
        for te in rt.rich_text_elements() {
            s.push_str(te.text());
        }
    }
    s
}

/// 将列号转换为 Excel 列名（如 1->A, 26->Z, 27->AA）
/// 
/// # 参数
/// * `col` - 列号（从1开始）
/// 
/// # 返回值
/// 对应的 Excel 列名
pub fn col_to_letter(mut col: u32) -> String {
    let mut chars = Vec::new();
    while col > 0 {
        col -= 1;
        chars.push((b'A' + (col % 26) as u8) as char);
        col /= 26;
    }
    chars.reverse();
    chars.into_iter().collect()
}

#[cfg(test)]
mod cond_fmt_tests {
    use super::*;

    fn make_cell(value: &str) -> CellData {
        let mut c = CellData::default();
        c.value = value.to_string();
        c
    }

    #[test]
    fn test_two_rules_non_overlapping_ranges() {
        let mut sheet = SheetData::new("test".into());
        // B7 和 B8 分别在各自行
        sheet.cells.insert((7, 2), make_cell("-17"));
        sheet.cells.insert((8, 2), make_cell("充足"));

        let rules = vec![
            UserCondFormatRule {
                operator: "<=".into(),
                value: "60".into(),
                color: "#FFC7CE".into(),
                range: "=B7:AK7".into(),
            },
            UserCondFormatRule {
                operator: "=".into(),
                value: "充足".into(),
                color: "#FFC0CB".into(),
                range: "=B8:AK8".into(),
            },
        ];

        ExcelData::apply_user_cond_format_rules(&mut sheet, &rules);

        // 规则1: B7 ≤ 60 → -17 ≤ 60 → true → #FFC7CE (255,199,206)
        let b7 = sheet.get_cell(7, 2).unwrap();
        assert_eq!(b7.background_color, Some((255, 199, 206)));

        // 规则2: B8 = 充足 → true → #FFC0CB (255,192,203)
        let b8 = sheet.get_cell(8, 2).unwrap();
        assert_eq!(b8.background_color, Some((255, 192, 203)),
            "B8 should be #FFC0CB, got {:?}", b8.background_color);
    }
}
