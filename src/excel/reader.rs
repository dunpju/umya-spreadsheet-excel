// 引入 umya-spreadsheet 库用于读取 Excel 文件
use umya_spreadsheet::reader;
// 引入 HashMap 用于存储单元格和列宽数据
use std::collections::HashMap;

/// 单元格数据结构，存储单元格的值和公式
#[derive(Debug, Clone)]
pub struct CellData {
    /// 单元格的显示值
    pub value: String,
    /// 单元格的公式（如存在）
    #[allow(dead_code)]
    pub formula: String,
}

/// CellData 的默认实现，创建空值和空公式的单元格
impl Default for CellData {
    fn default() -> Self {
        Self {
            value: String::new(),
            formula: String::new(),
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
    /// * `row` - 要检查的行号
    /// * `col` - 要检查的列号
    /// 
    /// # 返回值
    /// 如果坐标在范围内返回 true，否则返回 false
    pub fn contains(&self, row: u32, col: u32) -> bool {
        row >= self.start_row && row <= self.end_row && col >= self.start_col && col <= self.end_col
    }

    /// 检查指定的行列坐标是否是范围的左上角（起始单元格）
    /// 
    /// # 参数
    /// * `row` - 要检查的行号
    /// * `col` - 要检查的列号
    /// 
    /// # 返回值
    /// 如果是起始单元格返回 true，否则返回 false
    pub fn is_top_left(&self, row: u32, col: u32) -> bool {
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
    pub fn get_merged_range(&self, row: u32, col: u32) -> Option<&CellRange> {
        self.merged_cells.iter().find(|r| r.contains(row, col))
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

        let mut sheets = Vec::new();

        // 遍历工作簿中的所有工作表
        for worksheet in book.get_sheet_collection().iter() {
            // 创建工作表数据对象
            let mut sheet = SheetData::new(worksheet.get_name().to_string());

            // 使用库提供的方法动态获取工作表的最大行和最大列（去除硬编码限制）
            let highest_row = worksheet.get_highest_row();
            let highest_col = worksheet.get_highest_column();

            // 遍历所有单元格，读取有数据的单元格
            for row_idx in 1..=highest_row {
                for col_idx in 1..=highest_col {
                    if let Some(cell) = worksheet.get_cell((row_idx, col_idx)) {
                        let value = cell.get_value().to_string();
                        // 只保存非空的单元格
                        if !value.trim().is_empty() {
                            let cell_data = CellData {
                                value,
                                formula: cell.get_formula().to_string(),
                            };
                            sheet.cells.insert((row_idx, col_idx), cell_data);
                        }
                    }
                }
            }

            // 设置工作表的最大行和最大列
            sheet.max_row = highest_row;
            sheet.max_col = highest_col;

            // 读取合并单元格信息
            for range in worksheet.get_merge_cells() {
                if let (Some(start_row), Some(start_col), Some(end_row), Some(end_col)) = (
                    range.get_coordinate_start_row(),
                    range.get_coordinate_start_col(),
                    range.get_coordinate_end_row(),
                    range.get_coordinate_end_col(),
                ) {
                    let start_row_num = *start_row.get_num();
                    let start_col_num = *start_col.get_num();
                    let end_row_num = *end_row.get_num();
                    let end_col_num = *end_col.get_num();
                    
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
            for col_dimension in worksheet.get_column_dimensions() {
                let width = col_dimension.get_width();
                // 只保存宽度大于0的列
                if *width > 0.0 {
                    sheet.column_widths.insert(col_index, *width);
                }
                col_index += 1;
            }

            // 将工作表添加到列表中
            sheets.push(sheet);
        }

        // 检查是否有工作表
        if sheets.is_empty() {
            return Err("Excel文件中没有找到工作表".to_string());
        }

        Ok(Self { sheets })
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
    let mut result = String::new();
    while col > 0 {
        col -= 1;
        // 计算当前位的字母并插入到结果前面
        result.insert(0, (b'A' + (col % 26) as u8) as char);
        col /= 26;
    }
    result
}
