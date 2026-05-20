//! 工作表选择器组件
//! 
//! 负责在底部显示工作表标签，支持切换工作表

use eframe::egui;
use crate::excel::reader::ExcelData;

/// 绘制工作表选择器
/// 
/// 在底部显示所有工作表名称，支持切换工作表
/// 
/// # 参数
/// * `ui` - egui UI 上下文
/// * `excel_data` - Excel 数据引用
/// * `current_sheet` - 当前工作表索引的可变引用
/// * `selected_cell` - 选中单元格的可变引用（切换时会清空）
pub fn draw_sheet_selector(
    ui: &mut egui::Ui,
    excel_data: &ExcelData,
    current_sheet: &mut usize,
    selected_cell: &mut Option<(u32, u32)>,
) {
    ui.horizontal(|ui| {
        // 遍历所有工作表，显示可选择的标签
        for (i, sheet) in excel_data.sheets.iter().enumerate() {
            if ui.selectable_label(*current_sheet == i, &sheet.name).clicked() {
                // 切换工作表时重置选中和悬停状态
                *current_sheet = i;
                *selected_cell = None;
            }
        }
    });
}
