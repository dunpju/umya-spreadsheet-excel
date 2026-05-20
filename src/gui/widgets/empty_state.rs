//! 空状态组件
//! 
//! 负责显示未加载文件时的提示信息

use eframe::egui;

/// 绘制空状态提示
/// 
/// 当未加载文件时显示提示信息
/// 
/// # 参数
/// * `ui` - egui UI 上下文
pub fn draw_empty_state(ui: &mut egui::Ui) {
    ui.centered_and_justified(|ui| {
        ui.label("请通过 \"文件 > 导入\" 打开Excel文件");
    });
}
