// 引入 umya-spreadsheet 库用于读取 Excel 文件
use umya_spreadsheet::{reader, EnumTrait};
// 引入 HashMap 用于存储单元格和列宽数据
use std::collections::HashMap;

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
            font_size: None,
            font_color: None,
            number_format: None,
            bold: false,
            borders: CellBorders::default(),
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
    /// 适用的单元格范围
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
#[derive(Debug, Clone)]
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
                    background_color: template.background_color,
                    font_size: template.font_size,
                    font_color: template.font_color,
                    number_format: template.number_format.clone(),
                    bold: template.bold,
                    borders: template.borders.clone(),
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
                    background_color: template.background_color,
                    font_size: template.font_size,
                    font_color: template.font_color,
                    number_format: template.number_format.clone(),
                    bold: template.bold,
                    borders: template.borders.clone(),
                };
                self.cells.insert((old_max_row + 1, col), styled);
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
                        background_color: if options.copy_style {
                            source_cell.background_color
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
                                font_size: template.font_size,
                                font_color: template.font_color,
                                number_format: template.number_format.clone(),
                                bold: template.bold,
                                borders: template.borders.clone(),
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
        // 使用 umya-spreadsheet 库读取 Excel 文件
        let book = reader::xlsx::read(path)
            .map_err(|e| format!("读取失败: {}", e))?;

        // 获取主题对象，用于解析主题颜色
        let theme = book.theme();

        let mut sheets = Vec::new();

        // 遍历工作簿中的所有工作表
        for worksheet in book.sheet_collection().iter() {
            // 创建工作表数据对象
            let mut sheet = SheetData::new(worksheet.name().to_string());

            // 使用库提供的方法动态获取工作表的最大行和最大列（去除硬编码限制）
            let highest_row = worksheet.highest_row();
            let highest_col = worksheet.highest_column();

            // 获取所有实际存在的单元格（仅非空单元格有 XML 记录）
            let cells = worksheet.cells();
            // 预分配 HashMap 容量，避免加载过程中的多次 rehash
            sheet.cells.reserve(cells.len());

            // 多线程阈值：低于此数量直接顺序处理，减少线程调度开销
            const PARALLEL_THRESHOLD: usize = 5000;
            if cells.len() >= PARALLEL_THRESHOLD {
                // ===== 多线程并行解析单元格 =====
                let num_threads = std::thread::available_parallelism()
                    .map(|n| n.get()).unwrap_or(1);
                let chunk_size = (cells.len() + num_threads - 1) / num_threads;

                std::thread::scope(|s| {
                    let mut handles = Vec::with_capacity(num_threads);
                    for chunk in cells.chunks(chunk_size) {
                        // 每个线程需要的数据：单元格引用 + worksheet 引用 + theme 引用
                        let handle = s.spawn(move || {
                            // 样式缓存键类型
                            type SK = (
                                Option<(u64, String, bool)>,
                                Option<String>,
                                Option<(String, Option<String>, String, Option<String>, String, Option<String>, String, Option<String>)>,
                                Option<(String, String)>,
                                Option<String>,
                            );
                            let mut local_cells: std::collections::HashMap<(u32, u32), CellData> =
                                std::collections::HashMap::with_capacity(chunk.len());
                            let mut local_cache: std::collections::HashMap<SK, (CellAlignment, Option<(u8, u8, u8)>, Option<f64>, Option<(u8, u8, u8)>, Option<String>, bool, CellBorders)> =
                                std::collections::HashMap::new();

                            for cell in chunk {
                                let col_idx = cell.coordinate().col_num();
                                let row_idx = cell.coordinate().row_num();
                                let value = cell.value().to_string();
                                let raw_number = cell.value_number();
                                let style = worksheet.style((col_idx, row_idx));

                                let cache_key: SK = (
                                    style.font().map(|f| (f.size().to_bits(), f.color().argb_with_theme(theme).into_owned(), f.font_bold().val())),
                                    style.background_color().map(|c| c.argb_with_theme(theme).into_owned()),
                                    style.borders().map(|b| (
                                        b.left().border_style().to_string(), b.left().color().map(|c| c.argb_with_theme(theme).into_owned()),
                                        b.right().border_style().to_string(), b.right().color().map(|c| c.argb_with_theme(theme).into_owned()),
                                        b.top().border_style().to_string(), b.top().color().map(|c| c.argb_with_theme(theme).into_owned()),
                                        b.bottom().border_style().to_string(), b.bottom().color().map(|c| c.argb_with_theme(theme).into_owned()),
                                    )),
                                    style.alignment().map(|a| (a.horizontal().value_string().to_string(), a.vertical().value_string().to_string())),
                                    style.number_format().map(|n| n.format_code().to_string()),
                                );
                                let (alignment, background_color, font_size, font_color, number_format, bold, borders) =
                                    if let Some(cached) = local_cache.get(&cache_key) {
                                        cached.clone()
                                    } else {
                                        let parsed = Self::parse_style(style, theme);
                                        local_cache.insert(cache_key, parsed.clone());
                                        parsed
                                    };

                                local_cells.insert((row_idx, col_idx), CellData {
                                    value,
                                    raw_number,
                                    formula: cell.formula().to_string(),
                                    alignment,
                                    background_color,
                                    font_size,
                                    font_color,
                                    number_format,
                                    bold,
                                    borders,
                                });
                            }
                            local_cells
                        });
                        handles.push(handle);
                    }
                    // 合并各线程结果
                    for handle in handles {
                        if let Ok(local) = handle.join() {
                            sheet.cells.extend(local);
                        }
                    }
                });
            } else {
                // ===== 顺序解析（小文件） =====
                type StyleKey = (
                    Option<(u64, String, bool)>,
                    Option<String>,
                    Option<(String, Option<String>, String, Option<String>, String, Option<String>, String, Option<String>)>,
                    Option<(String, String)>,
                    Option<String>,
                );
                let mut style_cache: std::collections::HashMap<StyleKey, (CellAlignment, Option<(u8, u8, u8)>, Option<f64>, Option<(u8, u8, u8)>, Option<String>, bool, CellBorders)> = std::collections::HashMap::new();

                for cell in cells {
                    let col_idx = cell.coordinate().col_num();
                    let row_idx = cell.coordinate().row_num();
                    let value = cell.value().to_string();
                    let raw_number = cell.value_number();
                    let style = worksheet.style((col_idx, row_idx));

                    let cache_key: StyleKey = (
                        style.font().map(|f| (f.size().to_bits(), f.color().argb_with_theme(theme).into_owned(), f.font_bold().val())),
                        style.background_color().map(|c| c.argb_with_theme(theme).into_owned()),
                        style.borders().map(|b| (
                            b.left().border_style().to_string(), b.left().color().map(|c| c.argb_with_theme(theme).into_owned()),
                            b.right().border_style().to_string(), b.right().color().map(|c| c.argb_with_theme(theme).into_owned()),
                            b.top().border_style().to_string(), b.top().color().map(|c| c.argb_with_theme(theme).into_owned()),
                            b.bottom().border_style().to_string(), b.bottom().color().map(|c| c.argb_with_theme(theme).into_owned()),
                        )),
                        style.alignment().map(|a| (a.horizontal().value_string().to_string(), a.vertical().value_string().to_string())),
                        style.number_format().map(|n| n.format_code().to_string()),
                    );
                    let (alignment, background_color, font_size, font_color, number_format, bold, borders) =
                        if let Some(cached) = style_cache.get(&cache_key) {
                            cached.clone()
                        } else {
                            let parsed = Self::parse_style(style, theme);
                            style_cache.insert(cache_key, parsed.clone());
                            parsed
                        };

                    sheet.cells.insert((row_idx, col_idx), CellData {
                        value,
                        raw_number,
                        formula: cell.formula().to_string(),
                        alignment,
                        background_color,
                        font_size,
                        font_color,
                        number_format,
                        bold,
                        borders,
                    });
                }
            }

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

            // 将工作表添加到列表中
            sheets.push(sheet);
        }

        // 检查是否有工作表
        if sheets.is_empty() {
            return Err("Excel文件中没有找到工作表".to_string());
        }

        // 文件加载后立即求值所有公式
        for sheet in &mut sheets {
            sheet.rebuild_merge_index();
            crate::excel::formula::evaluate_sheet(sheet);
        }

        Ok(Self { sheets })
    }

    /// 解析 Excel 单元格样式
    ///
    /// 从 umya-spreadsheet 的 Style 对象中提取对齐方式、背景颜色、字体大小、字体颜色、
    /// 字体加粗、边框和数字格式
    ///
    /// # 参数
    /// * `style` - Excel 单元格样式对象
    /// * `theme` - 工作簿主题对象，用于解析主题颜色
    ///
    /// # 返回值
    /// 元组 (对齐方式, 背景颜色(RGB), 字体大小, 字体颜色(RGB), 数字格式, 是否加粗, 边框)
    fn parse_style(style: &umya_spreadsheet::Style, theme: &umya_spreadsheet::drawing::Theme) -> (CellAlignment, Option<(u8, u8, u8)>, Option<f64>, Option<(u8, u8, u8)>, Option<String>, bool, CellBorders) {
        let mut alignment = CellAlignment::default();
        let mut background_color: Option<(u8, u8, u8)> = None;
        let mut font_size: Option<f64> = None;
        let mut font_color: Option<(u8, u8, u8)> = None;
        let mut number_format: Option<String> = None;
        let mut bold = false;
        let mut borders = CellBorders::default();

        // 解析对齐方式
        if let Some(align) = style.alignment() {
            // 解析水平对齐方式
            let horizontal = align.horizontal();
            let h_str = horizontal.value_string();
            alignment.horizontal = match &*h_str {
                "left" => HorizontalAlignment::Left,
                "center" => HorizontalAlignment::Center,
                "right" => HorizontalAlignment::Right,
                "fill" => HorizontalAlignment::Fill,
                "justify" => HorizontalAlignment::Justify,
                "centerContinuous" => HorizontalAlignment::CenterContinuous,
                "distributed" => HorizontalAlignment::Distributed,
                _ => HorizontalAlignment::General,
            };

            // 解析垂直对齐方式
            let vertical = align.vertical();
            let v_str = vertical.value_string();
            alignment.vertical = match &*v_str {
                "top" => VerticalAlignment::Top,
                "center" => VerticalAlignment::Center,
                "justify" => VerticalAlignment::Justify,
                "distributed" => VerticalAlignment::Distributed,
                _ => VerticalAlignment::Bottom,
            };
        }

        // 解析背景颜色（使用 argb_with_theme 自动解析主题颜色和 tint）
        if let Some(bg_color) = style.background_color() {
            let resolved = bg_color.argb_with_theme(theme);
            if !resolved.is_empty() && resolved != "00000000" {
                if let Ok(rgb) = Self::parse_hex_color(&resolved) {
                    background_color = Some(rgb);
                }
            }
        }

        // 解析字体信息（大小、颜色、加粗）
        if let Some(font) = style.font() {
            font_size = Some(font.size());

            // 解析字体颜色
            let color = font.color();
            let resolved = color.argb_with_theme(theme);
            if !resolved.is_empty() && resolved != "00000000" {
                if let Ok(rgb) = Self::parse_hex_color(&resolved) {
                    font_color = Some(rgb);
                }
            }

            // 解析字体加粗
            bold = font.font_bold().val();
        }

        // 解析边框
        if let Some(style_borders) = style.borders() {
            let parse_border = |border: &umya_spreadsheet::structs::Border| -> CellBorder {
                let mut style_str = String::new();
                let mut color = None;
                // 检查边框样式：非 "none" 或空字符串表示有边框
                let bs = border.border_style();
                if !bs.is_empty() && bs != "none" {
                    style_str = bs.to_string();
                }
                // 解析边框颜色
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
    fn serial_to_date(serial: f64) -> (u32, u32, u32) {
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
