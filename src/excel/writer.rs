//! Excel 文件写入模块
//!
//! 将内存中的 ExcelData 写回到 Excel 文件，完整保留原始文件的所有属性。

use umya_spreadsheet::{reader, writer};
use super::reader::{CellData, ExcelData};

/// 将 ExcelData 保存到 Excel 文件
///
/// 重新读取原始文件以获取完整的 Workbook 对象（保留所有原始属性），
/// 然后将 ExcelData 中的单元格变更应用回 Workbook，最后写入新文件。
///
/// 保留的原始属性包括：单元格合并、公式、样式、数据有效性、
/// 字体大小、字体颜色、单元格背景颜色、列宽与行高、冻结区域、单元格边框。
///
/// # 参数
/// * `original_path` - 原始导入的文件路径
/// * `excel_data` - 内存中的 Excel 数据
/// * `output_path` - 输出文件路径
pub fn save_to_file(original_path: &str, excel_data: &ExcelData, output_path: &str) -> Result<(), String> {
    // 重新读取原始文件，保留所有格式属性
    let mut book = reader::xlsx::read(original_path)
        .map_err(|e| format!("重新读取原文件失败: {}", e))?;

    // 将 ExcelData 中的单元格数据写回 Workbook
    apply_cell_changes(&mut book, excel_data);

    // 写入新文件
    writer::xlsx::write(&book, output_path)
        .map_err(|e| format!("写入文件失败: {}", e))?;

    Ok(())
}

/// 将 ExcelData 中的单元格变更应用到 Workbook
///
/// 遍历每个工作表的每个单元格，更新值和公式。
/// 对于空值+空公式的单元格，仅清除内容而保留样式（边框、背景色等）。
fn apply_cell_changes(book: &mut umya_spreadsheet::Workbook, excel_data: &ExcelData) {
    for (sheet_idx, sheet_data) in excel_data.sheets.iter().enumerate() {
        if let Some(worksheet) = book.sheet_collection_mut().get_mut(sheet_idx) {
            for ((row, col), cell_data) in &sheet_data.cells {
                apply_cell(worksheet, *col, *row, cell_data);
            }
        }
    }
}

/// 将单个单元格的数据应用到 Worksheet
///
/// 使用 `cell_mut()` 获取或创建单元格（通过 `or_insert_with` 保留已有样式），
/// 然后更新值和公式。不使用 `remove_cell()` 以避免丢失边框、背景色等样式属性。
fn apply_cell(
    worksheet: &mut umya_spreadsheet::Worksheet,
    col: u32,
    row: u32,
    cell_data: &CellData,
) {
    // 获取或创建单元格（保留原有样式：边框、背景色、字体等）
    let cell = worksheet.cell_mut((col, row));

    if cell_data.value.is_empty() && cell_data.formula.is_empty() {
        // 值和公式均为空：仅清除值和公式，不移除单元格（保留样式）
        cell.set_value("");
        cell.set_formula("");
    } else {
        // 设置值
        cell.set_value(cell_data.value.as_str());
        // 设置公式
        if cell_data.formula.is_empty() {
            cell.set_formula("");
        } else {
            cell.set_formula(cell_data.formula.as_str());
        }
    }
}
