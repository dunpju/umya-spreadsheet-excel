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

/// 单元格数据结构，存储单元格的值和公式
#[derive(Debug, Clone)]
pub struct CellData {
    /// 单元格的显示值
    pub value: String,
    /// 单元格的公式（如存在）
    #[allow(dead_code)]
    pub formula: String,
    /// 单元格对齐方式
    pub alignment: CellAlignment,
    /// 背景颜色（RGB）
    pub background_color: Option<(u8, u8, u8)>,
    /// 字体大小（磅）
    pub font_size: Option<f64>,
    /// 字体颜色（RGB）
    pub font_color: Option<(u8, u8, u8)>,
}

/// CellData 的默认实现，创建空值和空公式的单元格
impl Default for CellData {
    fn default() -> Self {
        Self {
            value: String::new(),
            formula: String::new(),
            alignment: CellAlignment::default(),
            background_color: None,
            font_size: None,
            font_color: None,
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
        self.merged_cells.iter().find(|r| r.contains(col, row))
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
                        let style = worksheet.get_style((row_idx, col_idx));
                        let (alignment, background_color, font_size, font_color) = Self::parse_style(style);
                        
                        let cell_data = CellData {
                            value,
                            formula: cell.get_formula().to_string(),
                            alignment,
                            background_color,
                            font_size,
                            font_color,
                        };
                        sheet.cells.insert((row_idx, col_idx), cell_data);
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

            // 读取行高信息
            for row_dimension in worksheet.get_row_dimensions() {
                let row_num = *row_dimension.get_row_num();
                let height = row_dimension.get_height();
                // 只保存高度大于0的行
                if *height > 0.0 {
                    sheet.row_heights.insert(row_num, *height);
                }
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

    /// 解析 Excel 单元格样式
    /// 
    /// 从 umya-spreadsheet 的 Style 对象中提取对齐方式、背景颜色、字体大小和字体颜色
    /// 
    /// # 参数
    /// * `style` - Excel 单元格样式对象
    /// 
    /// # 返回值
    /// 元组 (对齐方式, 背景颜色(RGB), 字体大小, 字体颜色(RGB))
    fn parse_style(style: &umya_spreadsheet::Style) -> (CellAlignment, Option<(u8, u8, u8)>, Option<f64>, Option<(u8, u8, u8)>) {
        let mut alignment = CellAlignment::default();
        let mut background_color: Option<(u8, u8, u8)> = None;
        let mut font_size: Option<f64> = None;
        let mut font_color: Option<(u8, u8, u8)> = None;
        
        // 解析对齐方式
        if let Some(align) = style.get_alignment() {
            // 解析水平对齐方式
            let horizontal = align.get_horizontal();
            let h_str = horizontal.get_value_string();
            alignment.horizontal = match h_str.as_str() {
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
            let vertical = align.get_vertical();
            let v_str = vertical.get_value_string();
            alignment.vertical = match v_str.as_str() {
                "top" => VerticalAlignment::Top,
                "center" => VerticalAlignment::Center,
                "justify" => VerticalAlignment::Justify,
                "distributed" => VerticalAlignment::Distributed,
                _ => VerticalAlignment::Bottom,
            };
        }
        
        // 解析背景颜色
        if let Some(bg_color) = style.get_background_color() {
            let bg_argb = bg_color.get_argb();
            // 跳过空值和透明色（00000000）
            if !bg_argb.is_empty() && bg_argb != "00000000" {
                if let Ok(rgb) = Self::parse_hex_color(bg_argb) {
                    background_color = Some(rgb);
                }
            }
        }
        
        // 解析字体信息（大小和颜色）
        if let Some(font) = style.get_font() {
            font_size = Some(*font.get_size());
            
            // 解析字体颜色
            let color = font.get_color();
            let argb = color.get_argb();
            // 跳过空值和透明色（00000000）
            if !argb.is_empty() && argb != "00000000" {
                if let Ok(rgb) = Self::parse_hex_color(argb) {
                    font_color = Some(rgb);
                }
            }
        }
        
        (alignment, background_color, font_size, font_color)
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
        // 移除可能的 # 前缀
        let hex_str = hex_str.trim_start_matches('#');
        
        match hex_str.len() {
            // 6位 RGB 格式：RRGGBB
            6 => {
                let r = u8::from_str_radix(&hex_str[0..2], 16).map_err(|_| ())?;
                let g = u8::from_str_radix(&hex_str[2..4], 16).map_err(|_| ())?;
                let b = u8::from_str_radix(&hex_str[4..6], 16).map_err(|_| ())?;
                Ok((r, g, b))
            }
            // 8位 ARGB 格式：AARRGGBB，忽略 alpha 通道
            8 => {
                let r = u8::from_str_radix(&hex_str[2..4], 16).map_err(|_| ())?;
                let g = u8::from_str_radix(&hex_str[4..6], 16).map_err(|_| ())?;
                let b = u8::from_str_radix(&hex_str[6..8], 16).map_err(|_| ())?;
                Ok((r, g, b))
            }
            // 不支持的格式
            _ => Err(()),
        }
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
