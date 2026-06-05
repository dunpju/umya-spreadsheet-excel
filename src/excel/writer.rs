//! Excel 文件写入模块
//!
//! 将内存中的 ExcelData 写回到 Excel 文件，完整保留原始文件的所有属性。

use umya_spreadsheet::{reader, writer};
use super::reader::{
    CellAlignment, DataValidationOperator, DataValidationType, ExcelData,
    HorizontalAlignment, SheetData, VerticalAlignment,
};

/// 将 ExcelData 保存到 Excel 文件
///
/// 重新读取原始文件以获取完整的 Workbook 对象（保留所有原始属性），
/// 然后将 ExcelData 中的所有变更（结构+内容+样式）应用回 Workbook，最后写入新文件。
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

    // 将 ExcelData 中的所有变更应用回 Workbook
    for (sheet_idx, sheet_data) in excel_data.sheets.iter().enumerate() {
        if let Some(worksheet) = book.sheet_collection_mut().get_mut(sheet_idx) {
            apply_structural_changes(worksheet, sheet_data);
            apply_cell_changes(worksheet, sheet_data);
            apply_merge_changes(worksheet, sheet_data);
            apply_data_validations(worksheet, sheet_data);
            apply_column_widths(worksheet, sheet_data);
            apply_row_heights(worksheet, sheet_data);
        }
    }

    // 写入新文件
    writer::xlsx::write(&book, output_path)
        .map_err(|e| format!("写入文件失败: {}", e))?;

    Ok(())
}

/// 应用结构性变更（插入行/列）
///
/// 通过比较原始 Workbook 的行列数与 ExcelData 的行列数，
/// 推断出需要插入的行/列数量，在 Workbook 上执行相同的插入操作。
/// umya-spreadsheet 的 insert_new_row/column 会自动移动现有数据并调整公式引用。
fn apply_structural_changes(worksheet: &mut umya_spreadsheet::Worksheet, sheet_data: &SheetData) {
    let orig_max_row = worksheet.highest_row();
    let orig_max_col = worksheet.highest_column();

    // 插入新增的列（在原始最大列之后追加）
    if sheet_data.max_col > orig_max_col {
        let cols_to_insert = sheet_data.max_col - orig_max_col;
        worksheet.insert_new_column_by_index(orig_max_col, cols_to_insert);
    }

    // 插入新增的行（在原始最大行之后追加）
    if sheet_data.max_row > orig_max_row {
        let rows_to_insert = sheet_data.max_row - orig_max_row;
        worksheet.insert_new_row(orig_max_row, rows_to_insert);
    }
}

/// 将 ExcelData 中的单元格数据（值+公式+样式）写回 Worksheet
fn apply_cell_changes(worksheet: &mut umya_spreadsheet::Worksheet, sheet_data: &SheetData) {
    for ((row, col), cell_data) in &sheet_data.cells {
        let cell = worksheet.cell_mut((*col, *row));

        // 设置值和公式
        if cell_data.value.is_empty() && cell_data.formula.is_empty() {
            cell.set_value("");
            cell.set_formula("");
        } else {
            cell.set_value(cell_data.value.as_str());
            if cell_data.formula.is_empty() {
                cell.set_formula("");
            } else {
                cell.set_formula(cell_data.formula.as_str());
            }
        }

        // 设置样式（对齐、背景色、字体大小、字体颜色、数字格式）
        apply_cell_style(worksheet, *col, *row, cell_data);
    }
}

/// 将单个单元格的样式应用到 Worksheet
fn apply_cell_style(
    worksheet: &mut umya_spreadsheet::Worksheet,
    col: u32,
    row: u32,
    cell_data: &super::reader::CellData,
) {
    let style = worksheet.style_mut((col, row));

    // 对齐方式
    apply_alignment(style, &cell_data.alignment);

    // 背景颜色
    if let Some((r, g, b)) = cell_data.background_color {
        style.set_background_color(format!("{:02X}{:02X}{:02X}", r, g, b));
    }

    // 字体大小和颜色
    let font = style.font_mut();
    if let Some(size) = cell_data.font_size {
        font.set_size(size);
    }
    if let Some((r, g, b)) = cell_data.font_color {
        font.color_mut().set_argb_str(format!("FF{:02X}{:02X}{:02X}", r, g, b));
    }

    // 数字格式
    if let Some(ref fmt) = cell_data.number_format {
        style.number_format_mut().set_format_code(fmt);
    }
}

/// 应用对齐方式到 Style
fn apply_alignment(style: &mut umya_spreadsheet::Style, alignment: &CellAlignment) {
    let align = style.alignment_mut();
    align.set_horizontal(match alignment.horizontal {
        HorizontalAlignment::General => umya_spreadsheet::HorizontalAlignmentValues::General,
        HorizontalAlignment::Left => umya_spreadsheet::HorizontalAlignmentValues::Left,
        HorizontalAlignment::Center => umya_spreadsheet::HorizontalAlignmentValues::Center,
        HorizontalAlignment::Right => umya_spreadsheet::HorizontalAlignmentValues::Right,
        HorizontalAlignment::Fill => umya_spreadsheet::HorizontalAlignmentValues::Fill,
        HorizontalAlignment::Justify => umya_spreadsheet::HorizontalAlignmentValues::Justify,
        HorizontalAlignment::CenterContinuous => umya_spreadsheet::HorizontalAlignmentValues::CenterContinuous,
        HorizontalAlignment::Distributed => umya_spreadsheet::HorizontalAlignmentValues::Distributed,
    });
    align.set_vertical(match alignment.vertical {
        VerticalAlignment::Top => umya_spreadsheet::VerticalAlignmentValues::Top,
        VerticalAlignment::Center => umya_spreadsheet::VerticalAlignmentValues::Center,
        VerticalAlignment::Bottom => umya_spreadsheet::VerticalAlignmentValues::Bottom,
        VerticalAlignment::Justify => umya_spreadsheet::VerticalAlignmentValues::Justify,
        VerticalAlignment::Distributed => umya_spreadsheet::VerticalAlignmentValues::Distributed,
    });
}

/// 将 ExcelData 中的合并单元格信息写回 Worksheet
///
/// 先清除原始 Workbook 中的所有合并，再按 ExcelData 中的数据重新添加。
fn apply_merge_changes(worksheet: &mut umya_spreadsheet::Worksheet, sheet_data: &SheetData) {
    // 清除原始 Workbook 中的所有合并
    worksheet.merge_cells_mut().clear();

    // 按 ExcelData 重新添加合并
    for mr in &sheet_data.merged_cells {
        let start = format!("{}{}", super::reader::col_to_letter(mr.start_col), mr.start_row);
        let end = format!("{}{}", super::reader::col_to_letter(mr.end_col), mr.end_row);
        let range_str = format!("{}:{}", start, end);
        worksheet.add_merge_cells(&range_str);
    }
}

/// 将 ExcelData 中的数据有效性规则写回 Worksheet
///
/// 先清除原始 Workbook 中的所有数据有效性，再按 ExcelData 中的数据重新添加。
fn apply_data_validations(worksheet: &mut umya_spreadsheet::Worksheet, sheet_data: &SheetData) {
    // 清除原始数据有效性
    worksheet.remove_data_validations();

    if sheet_data.data_validations.is_empty() {
        return;
    }

    // 创建新的 DataValidations 对象并添加所有规则
    let mut dvs = umya_spreadsheet::structs::DataValidations::default();
    for dv in &sheet_data.data_validations {
        let mut new_dv = umya_spreadsheet::structs::DataValidation::default();

        // 设置类型
        new_dv.set_type(match dv.dv_type {
            DataValidationType::None => umya_spreadsheet::DataValidationValues::None,
            DataValidationType::Whole => umya_spreadsheet::DataValidationValues::Whole,
            DataValidationType::Decimal => umya_spreadsheet::DataValidationValues::Decimal,
            DataValidationType::List => umya_spreadsheet::DataValidationValues::List,
            DataValidationType::Date => umya_spreadsheet::DataValidationValues::Date,
            DataValidationType::Time => umya_spreadsheet::DataValidationValues::Time,
            DataValidationType::TextLength => umya_spreadsheet::DataValidationValues::TextLength,
            DataValidationType::Custom => umya_spreadsheet::DataValidationValues::Custom,
        });

        // 设置运算符
        new_dv.set_operator(match dv.dv_operator {
            DataValidationOperator::Between => umya_spreadsheet::DataValidationOperatorValues::Between,
            DataValidationOperator::NotBetween => umya_spreadsheet::DataValidationOperatorValues::NotBetween,
            DataValidationOperator::Equal => umya_spreadsheet::DataValidationOperatorValues::Equal,
            DataValidationOperator::NotEqual => umya_spreadsheet::DataValidationOperatorValues::NotEqual,
            DataValidationOperator::GreaterThan => umya_spreadsheet::DataValidationOperatorValues::GreaterThan,
            DataValidationOperator::GreaterThanOrEqual => umya_spreadsheet::DataValidationOperatorValues::GreaterThanOrEqual,
            DataValidationOperator::LessThan => umya_spreadsheet::DataValidationOperatorValues::LessThan,
            DataValidationOperator::LessThanOrEqual => umya_spreadsheet::DataValidationOperatorValues::LessThanOrEqual,
        });

        // 设置公式
        if !dv.formula1.is_empty() {
            new_dv.set_formula1(&dv.formula1);
        }
        if !dv.formula2.is_empty() {
            new_dv.set_formula2(&dv.formula2);
        }

        // 设置提示和错误信息
        new_dv.set_show_input_message(dv.show_error_message || !dv.prompt_title.is_empty() || !dv.prompt.is_empty());
        new_dv.set_show_error_message(dv.show_error_message);
        if !dv.prompt_title.is_empty() {
            new_dv.set_prompt_title(&dv.prompt_title);
        }
        if !dv.prompt.is_empty() {
            new_dv.set_prompt(&dv.prompt);
        }
        if !dv.error_title.is_empty() {
            new_dv.set_error_title(&dv.error_title);
        }
        if !dv.error_message.is_empty() {
            new_dv.set_error_message(&dv.error_message);
        }

        // 设置应用范围（单元格引用）
        let mut seq_refs = umya_spreadsheet::structs::SequenceOfReferences::default();
        for range in &dv.ranges {
            let start = format!("{}{}", super::reader::col_to_letter(range.start_col), range.start_row);
            let end = format!("{}{}", super::reader::col_to_letter(range.end_col), range.end_row);
            let mut r = umya_spreadsheet::structs::Range::default();
            r.set_range(if start == end { start.clone() } else { format!("{}:{}", start, end) });
            seq_refs.add_range_collection(r);
        }
        new_dv.set_sequence_of_references(seq_refs);

        dvs.add_data_validation_list(new_dv);
    }

    worksheet.set_data_validations(dvs);
}

/// 将 ExcelData 中的列宽信息写回 Worksheet
fn apply_column_widths(worksheet: &mut umya_spreadsheet::Worksheet, sheet_data: &SheetData) {
    for (&col, &width) in &sheet_data.column_widths {
        let col_dim = worksheet.column_dimension_by_number_mut(col);
        col_dim.set_width(width);
    }
}

/// 将 ExcelData 中的行高信息写回 Worksheet
fn apply_row_heights(worksheet: &mut umya_spreadsheet::Worksheet, sheet_data: &SheetData) {
    for (&row, &height) in &sheet_data.row_heights {
        let row_dim = worksheet.row_dimension_mut(row);
        row_dim.set_height(height);
    }
}
