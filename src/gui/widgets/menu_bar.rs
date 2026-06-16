//! 菜单栏组件
//!
//! 负责绘制应用程序的顶部菜单栏

use eframe::egui;
use crate::gui::viewer::{SettingsPanelState, SettingsPage};
use crate::gui::widgets::search::SearchWindowState;
use crate::gui::widgets::convert_popup::ConvertPopupState;
use crate::gui::widgets::alert_popup::AlertPopupState;
use crate::gui::widgets::cond_format_popup::CondFormatPopupState;
use crate::gui::widgets::help_popup::HelpPopupState;
use crate::gui::widgets::alert_notify::AlertNotifyState;
use crate::gui::widgets::draw_alert_icon;

/// 绘制菜单栏
///
/// 包含文件、编辑、搜索、设置、关于等菜单项
///
/// # 参数
/// * `ui` - egui UI 上下文
/// * `show_import_dialog` - 用于控制是否显示导入对话框的可变引用
/// * `settings_panel` - 设置面板状态
/// * `search_window` - 搜索窗口状态
pub fn draw_menu_bar(
    ui: &mut egui::Ui,
    show_import_dialog: &mut bool,
    settings_panel: &mut SettingsPanelState,
    search_window: &mut SearchWindowState,
    add_column: &mut bool,
    add_row: &mut bool,
    has_data: bool,
    convert_popup: &mut ConvertPopupState,
    alert_popup: &mut AlertPopupState,
    _cond_format_popup: &mut CondFormatPopupState,
    help_popup: &mut HelpPopupState,
    alert_notify_state: &mut AlertNotifyState,
) {
    let dt = ui.input(|i| i.stable_dt);

    egui::MenuBar::new().ui(ui, |ui| {
        // 文件菜单
        ui.menu_button("文件", |ui| {
            if ui.button("导入").clicked() {
                ui.close();
                *show_import_dialog = true;
            }
        });

        // 编辑菜单
        ui.menu_button("编辑", |ui| {
            if ui.add_enabled(has_data, egui::Button::new("添加列")).clicked() {
                ui.close();
                *add_column = true;
            }
            if ui.add_enabled(has_data, egui::Button::new("添加行")).clicked() {
                ui.close();
                *add_row = true;
            }
        });

        // 搜索菜单（插入在编辑和设置之间）
        ui.menu_button("搜索", |ui| {
            if ui.add_enabled(has_data, egui::Button::new("搜索")).clicked() {
                ui.close();
                search_window.visible = true;
                search_window.options_loaded = false; // 触发重新加载下拉选项
            }
        });

        // 配置菜单
        ui.menu_button("配置", |ui| {
            ui.menu_button("插入配置", |ui| {
                if ui.button("列配置").clicked() {
                    ui.close();
                    settings_panel.visible = true;
                    settings_panel.active_page = Some(SettingsPage::ColumnConfig);
                }
                if ui.button("行配置").clicked() {
                    ui.close();
                    settings_panel.visible = true;
                    settings_panel.active_page = Some(SettingsPage::RowConfig);
                }
            });
            if ui.button("搜索配置").clicked() {
                ui.close();
                settings_panel.show_search_dialog = true;
            }
            if ui.button("预警消息").clicked() {
                ui.close();
                alert_popup.visible = true;
            }
            // 使用原Excel表格条件格式功能，所以隐藏菜单功能
            // if ui.button("条件格式").clicked() {
            //     ui.close();
            //     _cond_format_popup.visible = true;
            // }
        });

        // 转换菜单
        ui.menu_button("转换", |ui| {
            if ui.button("转换工具").clicked() {
                ui.close();
                convert_popup.visible = true;
            }
        });

        // 关于菜单
        ui.menu_button("关于", |ui| {
            ui.label("My Excel v0.1.0");
            ui.label("使用 umya-spreadsheet 和 egui 构建");
            ui.separator();
            if ui.button("帮助").clicked() {
                ui.close();
                help_popup.visible = true;
            }
        });
    });

    // 菜单栏最右侧：预警通知图标
    draw_alert_icon(ui, alert_notify_state, dt);
}
