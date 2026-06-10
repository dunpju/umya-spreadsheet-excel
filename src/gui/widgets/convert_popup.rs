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
/// - 顶部行：左侧标题"转换工具"，右侧关闭按钮（X）
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

    let ConvertPopupState { visible, text, progress } = state;
    let mut keep_open = true;

    egui::Window::new("转换工具")
        .id(egui::Id::new("convert_popup"))
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .open(&mut keep_open)
        .show(ctx, |ui| {
            ui.set_min_width(600.0);
            ui.set_max_width(600.0);

            // ══════ 自定义标题栏 ══════
            ui.horizontal(|ui| {
                // 标题
                ui.label(egui::RichText::new("转换工具").size(13.0).strong());

                // 右侧关闭按钮
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui.button("✖").clicked() {
                            *visible = false;
                        }
                    },
                );
            });

            ui.separator();

            // 中间区域：多行文本输入框（内容超出时滚动）
            egui::ScrollArea::vertical()
                .max_height(230.0)
                .max_width(ui.available_width())
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(text)
                            .hint_text("请输入要转换规则...")
                            .desired_width(f32::INFINITY)
                            .desired_rows(13),
                    );
                });

            ui.separator();

            // 底部行：进度条 + 开始转换按钮
            ui.horizontal(|ui| {
                // 进度条
                ui.add(
                    egui::ProgressBar::new(*progress / 100.0)
                        .desired_width(530.0)
                        .text(format!("{:.0}%", *progress)),
                );

                // 将按钮推到右侧
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let _ = ui.add_enabled(false, egui::Button::new("开始转换"));
                });
            });
        });

    if !keep_open {
        *visible = false;
    }
}
