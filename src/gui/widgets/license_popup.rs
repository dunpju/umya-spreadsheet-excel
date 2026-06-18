//! 授权 / 付款弹窗组件
//!
//! 试用期到期或授权失效时弹出：展示付款二维码、本机机器码、授权码输入框。
//! 二维码 PNG 静态嵌入程序资源（`assets/pay_qr.png`），首次显示时解码为纹理。

use eframe::egui;

/// 二维码图片资源（请替换为真实付款码）
const QR_PNG: &[u8] = include_bytes!("../../../assets/pay_qr.png");

/// 授权弹窗状态
pub struct LicensePopupState {
    /// 是否显示（由 viewer 根据 LicenseStatus::is_blocking 决定）
    pub visible: bool,
    /// 用户输入的授权码
    pub license_input: String,
    /// 激活错误提示
    pub error: Option<&'static str>,
    /// 二维码纹理（惰性创建）
    pub qr_texture: Option<egui::TextureHandle>,
    /// 本机机器码（由 viewer 注入）
    pub machine_code: String,
    /// 激活成功提示（短暂显示后关闭）
    pub activated_timer: f32,
}

impl Default for LicensePopupState {
    fn default() -> Self {
        Self {
            visible: false,
            license_input: String::new(),
            error: None,
            qr_texture: None,
            machine_code: String::new(),
            activated_timer: 0.0,
        }
    }
}

impl LicensePopupState {
    /// 惰性解码二维码 PNG 并上传为纹理
    fn ensure_qr(&mut self, ctx: &egui::Context) {
        if self.qr_texture.is_some() {
            return;
        }
        if let Ok(img) = image::load_from_memory(QR_PNG) {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [w as usize, h as usize],
                rgba.as_raw(),
            );
            self.qr_texture = Some(ctx.load_texture("license_pay_qr", image, Default::default()));
        }
    }

    /// 重置输入与错误
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.license_input.clear();
        self.error = None;
    }
}

/// 绘制授权 / 付款弹窗。
///
/// `on_activate` 在用户点击“激活”时被调用，传入授权码字符串：
/// - `Ok(())` 表示激活成功（弹窗显示成功提示后关闭）
/// - `Err(msg)` 表示失败，`msg` 作为错误提示展示
pub fn draw_license_popup(
    ctx: &egui::Context,
    state: &mut LicensePopupState,
    status_text: &str,
    on_activate: &mut dyn FnMut(&str) -> Result<(), &'static str>,
) {
    if !state.visible {
        return;
    }
    state.ensure_qr(ctx);

    // 激活成功倒计时
    let mut hide_after_frame = false;
    if state.activated_timer > 0.0 {
        state.activated_timer -= ctx.input(|i| i.stable_dt);
        if state.activated_timer <= 0.0 {
            hide_after_frame = true;
        }
    }

    // 模态：居中、前台、无标题栏、不可关闭（强制处理授权）
    egui::Window::new("license_gate")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .movable(false)
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_min_width(400.0);
            ui.vertical_centered(|ui| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(status_text).size(16.0).strong());
                ui.add_space(8.0);

                if state.activated_timer > 0.0 {
                    // 激活成功态
                    ui.add_space(20.0);
                    ui.label(
                        egui::RichText::new("✅ 激活成功，感谢支持！")
                            .size(15.0)
                            .color(egui::Color32::from_rgb(0, 150, 0)),
                    );
                    ui.add_space(20.0);
                } else {
                    // 二维码
                    if let Some(tex) = &state.qr_texture {
                        ui.image(egui::load::SizedTexture::new(tex.id(), [200.0, 200.0]));
                    } else {
                        ui.set_min_height(200.0);
                    }
                    ui.label("扫码付款后，联系开发者获取授权码");
                    ui.add_space(8.0);

                    // 机器码
                    ui.label("本机机器码（请发送给开发者）：");
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let code = state.machine_code.clone();
                        ui.monospace(
                            egui::RichText::new(&code)
                                .size(14.0)
                                .background_color(egui::Color32::from_gray(235)),
                        );
                        if ui.button("复制").clicked() {
                            ctx.copy_text(code);
                        }
                    });
                    ui.add_space(12.0);

                    // 授权码输入
                    ui.label("输入授权码：");
                    ui.add(
                        egui::TextEdit::multiline(&mut state.license_input)
                            .desired_width(380.0)
                            .desired_rows(3),
                    );
                    if let Some(err) = state.error {
                        ui.colored_label(egui::Color32::from_rgb(200, 0, 0), err);
                    }
                    ui.add_space(8.0);

                    ui.horizontal_centered(|ui| {
                        let activate_clicked = ui
                            .button(egui::RichText::new("激  活").size(14.0))
                            .clicked();
                        if activate_clicked {
                            let code = state.license_input.trim().to_string();
                            if code.is_empty() {
                                state.error = Some("请输入授权码");
                            } else {
                                match on_activate(&code) {
                                    Ok(()) => {
                                        state.error = None;
                                        state.activated_timer = 1.5; // 显示成功提示后关闭
                                    }
                                    Err(msg) => state.error = Some(msg),
                                }
                            }
                        }
                    });
                }
            });
        });

    if hide_after_frame {
        state.visible = false;
        state.activated_timer = 0.0;
    }
}
