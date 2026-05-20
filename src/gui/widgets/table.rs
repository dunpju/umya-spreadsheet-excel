//! 表格渲染组件
//! 
//! 负责渲染 Excel 表格内容，包括单元格、合并单元格和对齐方式

use eframe::egui;
use crate::excel::reader::{CellAlignment, ExcelData, col_to_letter};
use crate::gui::alignment::alignment_to_egui;

/// 绘制表格内容
/// 
/// 使用虚拟渲染技术，只绘制可见区域的单元格，提高性能
/// 
/// # 参数
/// * `ui` - egui UI 上下文
/// * `excel_data` - Excel 数据引用
/// * `current_sheet` - 当前工作表索引
/// * `selected_cell` - 当前选中单元格
pub fn draw_table_content(
    ui: &mut egui::Ui,
    excel_data: &ExcelData,
    current_sheet: usize,
    selected_cell: Option<(u32, u32)>,
) {
    if let Some(sheet) = excel_data.get_sheet(current_sheet) {
        // 表格渲染常量定义
        let row_height = 25.0;        // 每行高度
        let default_col_width = 80.0; // 默认列宽
        let header_width = 60.0;      // 行号列宽度
        let border_width = 1.0;       // 边框宽度
        
        // 获取列宽的辅助函数
        let get_col_width = |col: u32| -> f32 {
            if let Some(&width) = sheet.column_widths.get(&col) {
                // 使用 Excel 中的列宽，乘以系数转换为像素
                width as f32 * 8.0
            } else {
                default_col_width
            }
        };
        
        // 计算表格总宽度
        let mut total_width = header_width;
        for col in 1..=sheet.max_col {
            total_width += get_col_width(col) + border_width;
        }
        total_width += border_width;
        // 计算表格总高度（包含表头）
        let total_height = row_height * (sheet.max_row + 1) as f32 + border_width * (sheet.max_row + 2) as f32;
        
        // 分配绘画区域
        let (response, painter) = ui.allocate_painter(egui::vec2(total_width, total_height), egui::Sense::hover());
        let rect = response.rect;
        let top_left = rect.min;
        
        let tl_x = top_left.x;
        let tl_y = top_left.y;
        
        // 绘制灰色背景
        painter.rect_filled(
            egui::Rect::from_min_size(egui::Pos2::new(tl_x, tl_y), egui::vec2(total_width, total_height)),
            0.0,
            egui::Color32::GRAY,
        );
        
        // 获取当前可见区域，用于虚拟渲染
        let viewport_rect = ui.clip_rect();
        let margin = 100.0; // 适当的边距即可，不需要太大
            
            // 先计算所有列的累积宽度，用于准确计算可见列
            // 索引 0: 0.0 (起点
            // 索引 1: 行号列结束位置
            // 索引 2: A列结束位置
            // ...
            let mut col_cumulative_width = vec![0.0];
            let mut current_width = 0.0;
            
            // 第 0 列（行号列）
            current_width += header_width + border_width;
            col_cumulative_width.push(current_width);
            
            // 第 1 列及以后（数据列）
            for col in 1..=sheet.max_col {
                current_width += get_col_width(col) + border_width;
                col_cumulative_width.push(current_width);
            }
            
            // 根据实际列宽计算可见列范围
            let target_start_x = viewport_rect.min.x - tl_x - margin;
            let target_end_x = viewport_rect.max.x - tl_x + margin;
            
            // 查找可见列范围
            let mut visible_cols_start = 0;
            let mut visible_cols_end = sheet.max_col + 1;
            
            for (i, &width) in col_cumulative_width.iter().enumerate() {
                if width > target_start_x && visible_cols_start == 0 {
                    visible_cols_start = i.saturating_sub(1).max(0) as u32;
                }
                if width > target_end_x {
                    visible_cols_end = i.min((sheet.max_col + 1) as usize) as u32;
                    break;
                }
            }
            
            // 确保第 0 列（行号列）始终可见
            visible_cols_start = 0;
            
            // 计算可见行范围（行高固定，保持原逻辑）
            let visible_rows_start = ((viewport_rect.min.y - tl_y - margin) / (row_height + border_width)).floor() as u32;
            let visible_rows_end = ((viewport_rect.max.y - tl_y + margin) / (row_height + border_width)).ceil() as u32;
            let visible_rows_start = visible_rows_start.max(0).min(sheet.max_row + 1);
            let visible_rows_end = visible_rows_end.max(0).min(sheet.max_row + 1);
            
            // 计算起始绘制位置
            let start_y = tl_y + border_width + visible_rows_start as f32 * (row_height + border_width);
            let mut y = start_y;
            
            // 遍历可见行进行绘制
            for row in visible_rows_start..=visible_rows_end {
                let mut x = tl_x + border_width;
                // 跳过不可见的左侧列
                for c in 0..visible_cols_start {
                    x += if c == 0 { header_width } else { get_col_width(c) } + border_width;
                }
                
                // 绘制可见列
                for col in visible_cols_start..=visible_cols_end {
                    let cell_width = if col == 0 { 
                        header_width 
                    } else { 
                        get_col_width(col) 
                    };
                    let cell_height = row_height;
                    
                    // 确定单元格背景色
                    let bg_color = if row == 0 && col == 0 {
                        egui::Color32::LIGHT_GRAY // 左上角空白
                    } else if row == 0 {
                        egui::Color32::LIGHT_GRAY // 列标题行
                    } else if col == 0 {
                        egui::Color32::LIGHT_GRAY // 行标题列
                    } else {
                        egui::Color32::WHITE // 数据单元格
                    };
                    
                    // 绘制单元格背景
                    painter.rect_filled(
                        egui::Rect::from_min_size(egui::Pos2::new(x, y), egui::vec2(cell_width, cell_height)),
                        0.0,
                        bg_color,
                    );
                    
                    // 绘制列标题（A, B, C...）
                    if row == 0 && col > 0 {
                        painter.text(
                            egui::Pos2::new(x + cell_width / 2.0, y + cell_height / 2.0),
                            egui::Align2::CENTER_CENTER,
                            col_to_letter(col),
                            egui::FontId::default(),
                            egui::Color32::BLACK,
                        );
                    } 
                    // 绘制行标题（1, 2, 3...）
                    else if col == 0 && row > 0 {
                        painter.text(
                            egui::Pos2::new(x + cell_width / 2.0, y + cell_height / 2.0),
                            egui::Align2::CENTER_CENTER,
                            row.to_string(),
                            egui::FontId::default(),
                            egui::Color32::BLACK,
                        );
                    } 
                    // 绘制数据单元格
                    else if row > 0 && col > 0 {
                        let mut cell_content = String::new();
                        let mut is_merged_top_left = false;
                        let mut alignment = CellAlignment::default();
                        
                        // 检查是否是合并单元格
                        if let Some(merged_range) = sheet.get_merged_range(col, row) {
                            // 只在合并单元格的左上角绘制内容
                            if merged_range.is_top_left(col, row) {
                                is_merged_top_left = true;
                                if let Some(cell) = sheet.get_cell(col, row) {
                                    cell_content = cell.value.clone();
                                    alignment = cell.alignment.clone();
                                }
                            }
                        } else {
                            // 普通单元格
                            if let Some(cell) = sheet.get_cell(col, row) {
                                cell_content = cell.value.clone();
                                alignment = cell.alignment.clone();
                            }
                        }
                        
                        // 绘制合并单元格
                        if is_merged_top_left {
                            if let Some(merged_range) = sheet.get_merged_range(col, row) {
                                let mut merged_col_width = 0.0;
                                for c in merged_range.start_col..=merged_range.end_col {
                                    merged_col_width += get_col_width(c) + border_width;
                                }
                                merged_col_width -= border_width;
                                let merged_row_height = (merged_range.end_row - merged_range.start_row + 1) as f32 * row_height + 
                                    (merged_range.end_row - merged_range.start_row) as f32 * border_width;
                                
                                let is_selected = selected_cell.is_some() && 
                                    merged_range.contains(selected_cell.unwrap().0, selected_cell.unwrap().1);
                                
                                if is_selected {
                                    painter.rect_filled(
                                        egui::Rect::from_min_size(
                                            egui::Pos2::new(x, y),
                                            egui::vec2(merged_col_width, merged_row_height),
                                        ),
                                        0.0,
                                        egui::Color32::from_rgb(173, 216, 230),
                                    );
                                }
                                
                                // 根据对齐方式绘制内容
                                let egui_align = alignment_to_egui(&alignment);
                                let text_pos = match egui_align {
                                    egui::Align2::LEFT_TOP => egui::Pos2::new(x + 4.0, y + 4.0),
                                    egui::Align2::LEFT_CENTER => egui::Pos2::new(x + 4.0, y + merged_row_height / 2.0),
                                    egui::Align2::LEFT_BOTTOM => egui::Pos2::new(x + 4.0, y + merged_row_height - 4.0),
                                    egui::Align2::CENTER_TOP => egui::Pos2::new(x + merged_col_width / 2.0, y + 4.0),
                                    egui::Align2::CENTER_CENTER => egui::Pos2::new(x + merged_col_width / 2.0, y + merged_row_height / 2.0),
                                    egui::Align2::CENTER_BOTTOM => egui::Pos2::new(x + merged_col_width / 2.0, y + merged_row_height - 4.0),
                                    egui::Align2::RIGHT_TOP => egui::Pos2::new(x + merged_col_width - 4.0, y + 4.0),
                                    egui::Align2::RIGHT_CENTER => egui::Pos2::new(x + merged_col_width - 4.0, y + merged_row_height / 2.0),
                                    egui::Align2::RIGHT_BOTTOM => egui::Pos2::new(x + merged_col_width - 4.0, y + merged_row_height - 4.0),
                                };
                                
                                painter.text(
                                    text_pos,
                                    egui_align,
                                    &cell_content,
                                    egui::FontId::default(),
                                    egui::Color32::BLACK,
                                );
                            }
                        } 
                        // 绘制普通单元格
                        else {
                            let is_selected = selected_cell == Some((col, row));
                            if is_selected {
                                painter.rect_filled(
                                    egui::Rect::from_min_size(
                                        egui::Pos2::new(x, y),
                                        egui::vec2(cell_width, cell_height),
                                    ),
                                    0.0,
                                    egui::Color32::from_rgb(173, 216, 230),
                                );
                            }
                            
                            // 根据对齐方式绘制内容
                            if !cell_content.is_empty() {
                                let egui_align = alignment_to_egui(&alignment);
                                let text_pos = match egui_align {
                                    egui::Align2::LEFT_TOP => egui::Pos2::new(x + 4.0, y + 4.0),
                                    egui::Align2::LEFT_CENTER => egui::Pos2::new(x + 4.0, y + cell_height / 2.0),
                                    egui::Align2::LEFT_BOTTOM => egui::Pos2::new(x + 4.0, y + cell_height - 4.0),
                                    egui::Align2::CENTER_TOP => egui::Pos2::new(x + cell_width / 2.0, y + 4.0),
                                    egui::Align2::CENTER_CENTER => egui::Pos2::new(x + cell_width / 2.0, y + cell_height / 2.0),
                                    egui::Align2::CENTER_BOTTOM => egui::Pos2::new(x + cell_width / 2.0, y + cell_height - 4.0),
                                    egui::Align2::RIGHT_TOP => egui::Pos2::new(x + cell_width - 4.0, y + 4.0),
                                    egui::Align2::RIGHT_CENTER => egui::Pos2::new(x + cell_width - 4.0, y + cell_height / 2.0),
                                    egui::Align2::RIGHT_BOTTOM => egui::Pos2::new(x + cell_width - 4.0, y + cell_height - 4.0),
                                };
                                
                                painter.text(
                                    text_pos,
                                    egui_align,
                                    &cell_content,
                                    egui::FontId::default(),
                                    egui::Color32::BLACK,
                                );
                            }
                        }
                    }
                    
                    // 移动到下一列
                    x += cell_width + border_width;
                }
                // 移动到下一行
                y += row_height + border_width;
            }
    }
}
