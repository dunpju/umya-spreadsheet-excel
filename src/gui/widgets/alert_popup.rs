//! 预警消息弹窗组件
//!
//! 用于管理自定义预警消息规则，支持增删改和 YAML 持久化。

use eframe::egui;

/// 预警消息规则
#[derive(Debug, Clone, Default)]
pub struct AlertRule {
    /// 比较运算符
    pub operator: String,
    /// 阈值
    pub value: String,
    /// 预警消息内容
    pub message: String,
    /// 应用范围
    pub range: String,
}

/// 预警消息弹窗状态
#[derive(Debug, Clone)]
pub struct AlertPopupState {
    pub visible: bool,
    pub rules: Vec<AlertRule>,
}

impl Default for AlertPopupState {
    fn default() -> Self {
        Self {
            visible: false,
            rules: Vec::new(),
        }
    }
}

impl AlertPopupState {
    /// 从 YAML Value 加载规则列表
    pub fn load_from_yaml(doc: &serde_yaml::Value) -> Self {
        let rules = doc
            .get("alertMessages")
            .and_then(|am| am.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|item| {
                        let operator = item.get("operator")?.as_str()?.to_string();
                        let value = item.get("value")?.as_str()?.to_string();
                        let message = item.get("message")?.as_str()?.to_string();
                        let range = item.get("range")?.as_str()?.to_string();
                        Some(AlertRule { operator, value, message, range })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Self {
            visible: false,
            rules,
        }
    }

    /// 将规则列表写入 YAML Value
    pub fn save_to_yaml(&self, doc: &mut serde_yaml::Value) {
        let seq: Vec<serde_yaml::Value> = self
            .rules
            .iter()
            .map(|r| {
                let mut m = serde_yaml::Mapping::new();
                m.insert("operator".into(), r.operator.clone().into());
                m.insert("value".into(), r.value.clone().into());
                m.insert("message".into(), r.message.clone().into());
                m.insert("range".into(), r.range.clone().into());
                serde_yaml::Value::Mapping(m)
            })
            .collect();
        let am_val = serde_yaml::Value::Sequence(seq);
        if let Some(mapping) = doc.as_mapping_mut() {
            mapping.insert("alertMessages".into(), am_val);
        }
    }
}

// ============================================================================
// 常量
// ============================================================================

const OPERATORS: &[&str] = &[">", "<", "=", ">=", "<=", "!="];

// ============================================================================
// 绘制函数
// ============================================================================

/// 绘制预警消息弹窗
pub fn draw_alert_popup(ctx: &egui::Context, state: &mut AlertPopupState) {
    if !state.visible {
        return;
    }

    let mut keep_open = true;
    let mut rules_changed = false;

    egui::Window::new("alert_popup")
        .title_bar(false)
        .resizable(true)
        .collapsible(false)
        .open(&mut keep_open)
        .default_size(egui::vec2(560.0, 400.0))
        .show(ctx, |ui| {
            ui.set_min_width(520.0);

            // ══════ 自定义标题栏 ══════
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("预警规则配置")
                        .size(13.0)
                        .strong(),
                );
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui.button("✖").clicked() {
                            state.visible = false;
                        }
                        if ui.button("保存").clicked() {
                            state.save_to_file();
                        }
                    },
                );
            });

            ui.separator();

            // ══════ 表头 ══════
            ui.horizontal(|ui| {
                ui.set_height(20.0);
                ui.label(egui::RichText::new("规则").size(12.0).strong());
                ui.separator();
                ui.label(egui::RichText::new("值").size(12.0).strong());
                ui.separator();
                ui.label(egui::RichText::new("预警消息").size(12.0).strong());
                ui.separator();
                ui.label(egui::RichText::new("应用于").size(12.0).strong());
                ui.label(egui::RichText::new("操作").size(12.0).strong());
            });

            ui.separator();

            // ══════ 规则列表 ══════
            let mut to_delete: Option<usize> = None;
            let available_height = ui.available_height() - 40.0;

            egui::ScrollArea::vertical()
                .max_height(available_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (idx, rule) in state.rules.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            // 运算符下拉框
                            let mut selected_op = OPERATORS
                                .iter()
                                .position(|&o| o == rule.operator)
                                .unwrap_or(0);
                            egui::ComboBox::from_id_salt(format!("alert_op_{}", idx))
                                .width(50.0)
                                .selected_text(rule.operator.clone())
                                .show_ui(ui, |ui| {
                                    for (i, op) in OPERATORS.iter().enumerate() {
                                        if ui.selectable_label(i == selected_op, *op).clicked() {
                                            selected_op = i;
                                        }
                                    }
                                });
                            if selected_op < OPERATORS.len()
                                && OPERATORS[selected_op] != rule.operator
                            {
                                rule.operator = OPERATORS[selected_op].to_string();
                                rules_changed = true;
                            }

                            // 阈值输入
                            let val_resp = ui.add(
                                egui::TextEdit::singleline(&mut rule.value)
                                    .desired_width(50.0)
                                    .hint_text("60"),
                            );
                            if val_resp.changed() {
                                rules_changed = true;
                            }

                            // 预警消息输入
                            let msg_resp = ui.add(
                                egui::TextEdit::singleline(&mut rule.message)
                                    .desired_width(120.0)
                                    .hint_text("提示信息"),
                            );
                            if msg_resp.changed() {
                                rules_changed = true;
                            }

                            // 范围输入
                            let range_resp = ui.add(
                                egui::TextEdit::singleline(&mut rule.range)
                                    .desired_width(120.0)
                                    .hint_text("=G3:G154"),
                            );
                            if range_resp.changed() {
                                rules_changed = true;
                            }

                            // 删除按钮
                            if ui.button("删除").clicked() {
                                to_delete = Some(idx);
                            }
                        });
                    }
                });

            // 执行删除
            if let Some(idx) = to_delete {
                state.rules.remove(idx);
                rules_changed = true;
            }

            // ══════ 底部操作 ══════
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("+ 新增规则").clicked() {
                    let new_rule = state
                        .rules
                        .last()
                        .map(|last| AlertRule {
                            operator: last.operator.clone(),
                            value: last.value.clone(),
                            message: last.message.clone(),
                            range: String::new(),
                        })
                        .unwrap_or_default();
                    state.rules.push(new_rule);
                    rules_changed = true;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if rules_changed {
                        ui.label(
                            egui::RichText::new("已修改，请点击保存")
                                .size(11.0)
                                .color(egui::Color32::from_rgb(200, 150, 0)),
                        );
                    }
                });
            });
        });

    if !keep_open {
        state.visible = false;
    }
}

impl AlertPopupState {
    /// YAML 配置文件路径
    fn config_path() -> std::path::PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        home.join(".MyExcel").join("my-excel.yaml")
    }

    /// 保存规则到 YAML 文件
    pub fn save_to_file(&mut self) {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let mut doc: serde_yaml::Value = if path.exists() {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_yaml::from_str(&s).ok())
                .unwrap_or(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
        } else {
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
        };

        self.save_to_yaml(&mut doc);

        if let Ok(yaml_str) = serde_yaml::to_string(&doc) {
            let _ = std::fs::write(&path, yaml_str);
        }
    }

    /// 从 YAML 文件加载规则
    pub fn load_from_file() -> Self {
        let path = Self::config_path();
        if !path.exists() {
            return Self::default();
        }
        let doc: serde_yaml::Value = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_yaml::from_str(&s).ok())
            .unwrap_or(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
        Self::load_from_yaml(&doc)
    }
}
