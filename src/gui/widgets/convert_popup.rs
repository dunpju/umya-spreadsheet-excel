//! 转换弹窗组件
//!
//! 负责显示"转换"弹出层，包含文本输入、进度条和开始转换按钮

use eframe::egui;

/// 转换弹窗状态
#[derive(Debug)]
pub struct ConvertPopupState {
    /// 是否显示弹窗
    pub visible: bool,
    /// 多行文本输入框内容
    pub text: String,
    /// 当前进度值（0-100）
    pub progress: f32,
}

impl Default for ConvertPopupState {
    fn default() -> Self {
        Self {
            visible: false,
            text: String::new(),
            progress: 0.0,
        }
    }
}

/// 绘制转换弹窗
///
/// 弹窗布局：
/// - 顶部行：左侧标题"转换"，右侧关闭按钮（X）
/// - 中间区域：多行文本输入框（TextArea）
/// - 底部行：左侧进度条（ProgressBar），右侧【开始转换】按钮
///
/// # 参数
/// * `ctx` - egui 上下文
/// * `state` - 转换弹窗状态的可变引用
pub fn draw_convert_popup(ctx: &egui::Context, state: &mut ConvertPopupState) {
    if !state.visible {
        return;
    }

    let mut visible = state.visible;
    egui::Modal::new(egui::Id::new("convert_popup")).show(ctx, |ui| {
        ui.set_min_width(600.0);

        // 顶部行：标题 + 关闭按钮
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("转换")
                    .size(18.0)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("✕").clicked() {
                    visible = false;
                }
            });
        });

        ui.separator();

        // 中间区域：多行文本输入框（限制宽高）
        egui::ScrollArea::vertical()
            .id_salt("convert_textarea")
            .max_height(220.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(590.0);
                egui::TextEdit::multiline(&mut state.text)
                    .hint_text("请输入要转换的内容...")
                    .desired_rows(9)
                    .desired_width(590.0)
                    .show(ui);
            });

        ui.separator();

        // 底部行：进度条 + 开始转换按钮
        ui.horizontal(|ui| {
            // 进度条
            ui.add(
                egui::ProgressBar::new(state.progress / 100.0)
                    .desired_width(240.0)
                    .text(format!("{:.0}%", state.progress)),
            );

            // 将按钮推到右侧
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let _ = ui.add_enabled(false, egui::Button::new("开始转换"));
            });
        });
    });

    state.visible = visible;
}
