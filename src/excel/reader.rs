use umya_spreadsheet::reader;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct CellData {
    pub value: String,
    pub formula: String,
}

impl Default for CellData {
    fn default() -> Self {
        Self {
            value: String::new(),
            formula: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellRange {
    pub start_row: u32,
    pub start_col: u32,
    pub end_row: u32,
    pub end_col: u32,
}

impl CellRange {
    pub fn new(start_row: u32, start_col: u32, end_row: u32, end_col: u32) -> Self {
        Self {
            start_row,
            start_col,
            end_col,
            end_row,
        }
    }

    pub fn contains(&self, row: u32, col: u32) -> bool {
        row >= self.start_row && row <= self.end_row && col >= self.start_col && col <= self.end_col
    }

    pub fn is_top_left(&self, row: u32, col: u32) -> bool {
        row == self.start_row && col == self.start_col
    }

}

#[derive(Debug, Clone)]
pub struct SheetData {
    pub name: String,
    pub cells: HashMap<(u32, u32), CellData>,
    pub merged_cells: Vec<CellRange>,
    pub max_row: u32,
    pub max_col: u32,
}

impl SheetData {
    pub fn new(name: String) -> Self {
        Self {
            name,
            cells: HashMap::new(),
            merged_cells: Vec::new(),
            max_row: 0,
            max_col: 0,
        }
    }

    pub fn get_cell(&self, row: u32, col: u32) -> Option<&CellData> {
        self.cells.get(&(row, col))
    }

    pub fn get_merged_range(&self, row: u32, col: u32) -> Option<&CellRange> {
        self.merged_cells.iter().find(|r| r.contains(row, col))
    }

}

#[derive(Debug, Clone)]
pub struct ExcelData {
    pub sheets: Vec<SheetData>,
}

impl ExcelData {
    pub fn load_from_file(path: &str) -> Result<Self, String> {
        let book = reader::xlsx::read(path)
            .map_err(|e| format!("读取失败: {}", e))?;

        let mut sheets = Vec::new();

        for worksheet in book.get_sheet_collection().iter() {
            let mut sheet = SheetData::new(worksheet.get_name().to_string());
            sheet.max_row = worksheet.get_highest_row();
            sheet.max_col = worksheet.get_highest_column();

            for row in 1..=sheet.max_row {
                for col in 1..=sheet.max_col {
                    if let Some(cell) = worksheet.get_cell((row, col)) {
                        let cell_data = CellData {
                            value: cell.get_value().to_string(),
                            formula: cell.get_formula().to_string(),
                        };
                        sheet.cells.insert((row, col), cell_data);
                    }
                }
            }

            for range in worksheet.get_merge_cells() {
                if let (Some(start_row), Some(start_col), Some(end_row), Some(end_col)) = (
                    range.get_coordinate_start_row(),
                    range.get_coordinate_start_col(),
                    range.get_coordinate_end_row(),
                    range.get_coordinate_end_col(),
                ) {
                    let cell_range = CellRange::new(
                        *start_row.get_num(),
                        *start_col.get_num(),
                        *end_row.get_num(),
                        *end_col.get_num(),
                    );
                    sheet.merged_cells.push(cell_range);
                }
            }

            sheets.push(sheet);
        }

        if sheets.is_empty() {
            return Err("Excel文件中没有找到工作表".to_string());
        }

        Ok(Self { sheets })
    }

    pub fn get_sheet(&self, index: usize) -> Option<&SheetData> {
        self.sheets.get(index)
    }
}

pub fn col_to_letter(mut col: u32) -> String {
    let mut result = String::new();
    while col > 0 {
        col -= 1;
        result.insert(0, (b'A' + (col % 26) as u8) as char);
        col /= 26;
    }
    result
}