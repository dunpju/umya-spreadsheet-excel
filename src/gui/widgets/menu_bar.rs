//! 菜单栏组件
//!
//! 负责绘制应用程序的顶部菜单栏

use eframe::egui;
use crate::gui::viewer::{SettingsPanelState, SettingsPage};

/// 绘制菜单栏
///
/// 包含文件、编辑、设置、关于等菜单项
///
/// # 参数
/// * `ui` - egui UI 上下文
/// * `show_import_dialog` - 用于控制是否显示导入对话框的可变引用
/// * `settings_panel` - 设置面板状态
pub fn draw_menu_bar(ui: &mut egui::Ui, show_import_dialog: &mut bool, settings_panel: &mut SettingsPanelState) {
    egui::menu::bar(ui, |ui| {
        // 文件菜单
        ui.menu_button("文件", |ui| {
            if ui.button("导入").clicked() {
                ui.close_menu();
                *show_import_dialog = true;
            }
            ui.add_enabled(false, egui::Button::new("模板"));
        });

        // 编辑菜单（暂未实现）
        ui.menu_button("编辑", |ui| {
            ui.label("编辑功能");
        });

        // 设置菜单
        ui.menu_button("设置", |ui| {
            ui.menu_button("插入", |ui| {
                if ui.button("列").clicked() {
                    ui.close_menu();
                    settings_panel.visible = true;
                    settings_panel.active_page = Some(SettingsPage::ColumnConfig);
                }
                if ui.button("行").clicked() {
                    ui.close_menu();
                    settings_panel.visible = true;
                    settings_panel.active_page = Some(SettingsPage::RowConfig);
                }
            });
        });

        // 关于菜单
        ui.menu_button("关于", |ui| {
            ui.label("Excel Viewer v0.1.0");
            ui.label("使用 umya-spreadsheet 和 egui 构建");
        });
    });
}
