//! 预警消息通知组件
//!
//! 在菜单栏右侧显示黄色闪烁警示图标，点击后弹出预警消息弹窗，
//! 列出所有已触发的预警规则，支持点击过滤和重置功能。

use eframe::egui;
use std::collections::HashSet;
use crate::excel::reader::SheetData;
use crate::gui::widgets::alert_popup::AlertRule;

/// 相等比较：优先数值比较，回退字符串比较（与 ExcelData::compare_equal 逻辑一致）
fn compare_equal(cell_value: &str, threshold: &str) -> bool {
    if let (Some(cv), Some(tv)) = (
        cell_value.trim().parse::<f64>().ok(),
        threshold.trim().parse::<f64>().ok(),
    ) {
        (cv - tv).abs() < f64::EPSILON
    } else {
        cell_value.trim().to_lowercase() == threshold.trim().to_lowercase()
    }
}

// ═══════════════════════════════════════════════════════════════
// 常量
// ═══════════════════════════════════════════════════════════════

/// 闪烁频率：每秒 2 次，即每 500ms 切换一次
const BLINK_INTERVAL_SECS: f32 = 0.5;

// ═══════════════════════════════════════════════════════════════
// 数据结构
// ═══════════════════════════════════════════════════════════════

/// 已触发的预警规则条目（带解析后的范围信息）
#[derive(Debug, Clone)]
pub struct TriggeredRule {
    /// 规则消息
    pub message: String,
    /// 规则原始范围字符串
    pub range: String,
    /// 规则运算符
    pub operator: String,
    /// 规则阈值
    pub value: String,
    /// 解析后的范围: (start_col, start_row, end_col, end_row)
    pub resolved_range: Option<(u32, u32, u32, u32)>,
    /// 范围方向: true = 横向（同行多列），false = 纵向（同列多行）
    pub is_horizontal: bool,
}

/// 预警通知弹窗状态
#[derive(Debug)]
pub struct AlertNotifyState {
    /// 弹窗是否可见
    pub visible: bool,
    /// 当前已触发的规则列表
    pub triggered_rules: Vec<TriggeredRule>,
    /// 闪烁计时器（累计秒数）
    blink_timer: f32,
    /// 是否有任意规则被触发（用于控制图标显隐）
    pub has_triggered: bool,
    /// 当前是否处于过滤状态（有来自预警通知的过滤）
    pub is_filtering: bool,
}

impl Default for AlertNotifyState {
    fn default() -> Self {
        Self {
            visible: false,
            triggered_rules: Vec::new(),
            blink_timer: 0.0,
            has_triggered: false,
            is_filtering: false,
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// 范围解析
// ═══════════════════════════════════════════════════════════════

/// 解析预警规则的应用范围字符串
///
/// 支持格式：
/// - `=B8:~8` → 横向扩展（B8 到第8行最右列）
/// - `=B8:D8` → 横向固定范围
/// - `=B8:B~` → 纵向扩展（B8 到B列最底行）
/// - `=B8:B12` → 纵向固定范围
/// - `=B8:~` → 全方向扩展
///
/// 对于固定范围（无 ~），expand_col/expand_row 偏移量会加到结束边界上，
/// 实现「插入列/行」后范围自动扩展。
///
/// 返回 (start_col, start_row, end_col, end_row, is_horizontal)
fn parse_alert_range(
    range_str: &str,
    sheet: &SheetData,
    expand_col: u32,
    expand_row: u32,
) -> Option<(u32, u32, u32, u32, bool)> {
    // 去掉前导 = 和 $
    let raw_range = range_str.trim_start_matches('=').replace('$', "");
    let has_tilde = raw_range.contains('~');

    // 解析动态范围引用（~）
    let range_str = resolve_dynamic_range(&raw_range, sheet);

    let parts: Vec<&str> = range_str.split(':').collect();
    let (start_col, start_row, mut end_col, mut end_row) = if parts.len() == 2 {
        let start = parse_cell_ref_str(parts[0])?;
        let end = parse_cell_ref_str(parts[1])?;
        (start.0, start.1, end.0, end.1)
    } else if parts.len() == 1 {
        let (c, r) = parse_cell_ref_str(parts[0])?;
        (c, r, c, r)
    } else {
        return None;
    };

    // 固定范围：应用扩展偏移量（~ 动态范围不需要，已自动跟随 max_col/max_row）
    if !has_tilde {
        end_col += expand_col;
        end_row += expand_row;
    }

    // 判断方向：同行为横向，同列为纵向
    let is_horizontal = start_row == end_row;

    Some((start_col, start_row, end_col, end_row, is_horizontal))
}

/// 解析动态范围引用：~行号 → 该行最大列，列字母~ → 该列最大行
fn resolve_dynamic_range(range_str: &str, sheet: &SheetData) -> String {
    if !range_str.contains('~') {
        return range_str.to_string();
    }
    let parts: Vec<&str> = range_str.split(':').collect();
    let resolve_part = |s: &str| -> String {
        let s = s.trim();
        if s == "~" {
            format!("{}{}", crate::excel::reader::col_to_letter(sheet.max_col.max(1)), sheet.max_row.max(1))
        } else if s.starts_with('~') {
            let row: u32 = s[1..].parse().unwrap_or(1);
            format!("{}{}", crate::excel::reader::col_to_letter(sheet.max_col.max(1)), row)
        } else if s.ends_with('~') {
            let col_letters: String = s.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
            format!("{}{}", col_letters, sheet.max_row.max(1))
        } else {
            s.to_string()
        }
    };
    if parts.len() == 2 {
        format!("{}:{}", resolve_part(parts[0]), resolve_part(parts[1]))
    } else {
        resolve_part(range_str)
    }
}

/// 解析单元格引用字符串 "B8" → (col, row) 1-based
fn parse_cell_ref_str(s: &str) -> Option<(u32, u32)> {
    let s = s.trim().to_uppercase();
    if s.is_empty() {
        return None;
    }
    let col_part: String = s.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let row_part: String = s.chars().skip_while(|c| c.is_ascii_alphabetic()).collect();
    if col_part.is_empty() || row_part.is_empty() {
        return None;
    }
    let col = col_part
        .chars()
        .fold(0u32, |acc, c| acc * 26 + (c as u32 - 'A' as u32 + 1));
    let row = row_part.parse::<u32>().ok()?;
    if col == 0 || row == 0 {
        return None;
    }
    Some((col, row))
}

// ═══════════════════════════════════════════════════════════════
// 规则触发检测
// ═══════════════════════════════════════════════════════════════

/// 比较单元格值与规则阈值
fn compare_values(cell_value: &str, operator: &str, threshold: &str) -> bool {
    match operator {
        "=" => compare_equal(cell_value, threshold),
        "!=" => !compare_equal(cell_value, threshold),
        ">" | "<" | ">=" | "<=" => {
            // 尝试数值比较
            if let (Some(cv), Some(tv)) = (
                cell_value.trim().parse::<f64>().ok(),
                threshold.trim().parse::<f64>().ok(),
            ) {
                match operator {
                    ">" => cv > tv,
                    "<" => cv < tv,
                    ">=" => cv >= tv,
                    "<=" => cv <= tv,
                    _ => false,
                }
            } else {
                // 回退字符串比较
                let cv = cell_value.trim().to_lowercase();
                let tv = threshold.trim().to_lowercase();
                match operator {
                    ">" => cv > tv,
                    "<" => cv < tv,
                    ">=" => cv >= tv,
                    "<=" => cv <= tv,
                    _ => false,
                }
            }
        }
        _ => false,
    }
}

/// 检查所有预警规则是否被触发
pub fn check_alert_rules(
    rules: &[AlertRule],
    sheet: &SheetData,
) -> Vec<TriggeredRule> {
    let mut triggered = Vec::new();

    for rule in rules {
        let range_str = rule.range.trim();
        if range_str.is_empty() {
            continue;
        }

        let parsed = match parse_alert_range(range_str, sheet, rule.range_expand_col, rule.range_expand_row) {
            Some(p) => p,
            None => continue,
        };

        let (start_col, start_row, end_col, end_row, is_horizontal) = parsed;

        // 遍历范围内的每个单元格，检查是否触发规则
        let mut rule_triggered = false;

        if is_horizontal {
            // 横向：同行多列
            for col in start_col..=end_col {
                if let Some(cell) = sheet.get_cell(start_row, col) {
                    if compare_values(&cell.value, &rule.operator, &rule.value) {
                        rule_triggered = true;
                        break;
                    }
                }
            }
        } else {
            // 纵向：同列多行
            for row in start_row..=end_row {
                if let Some(cell) = sheet.get_cell(row, start_col) {
                    if compare_values(&cell.value, &rule.operator, &rule.value) {
                        rule_triggered = true;
                        break;
                    }
                }
            }
        }

        if rule_triggered {
            triggered.push(TriggeredRule {
                message: rule.message.clone(),
                range: rule.range.clone(),
                operator: rule.operator.clone(),
                value: rule.value.clone(),
                resolved_range: Some((start_col, start_row, end_col, end_row)),
                is_horizontal,
            });
        }
    }

    triggered
}

/// 当插入/添加列时，更新所有受影响规则的 range_expand_col
///
/// 如果插入位置在某个规则固定范围的 [start_col, end_col] 内或紧邻 end_col 之后，
/// 则将该规则的 range_expand_col 增加 n。
pub fn update_alert_range_expansions_for_col(
    rules: &mut [AlertRule],
    insert_col: u32,
    n: u32,
    _sheet: &SheetData,
) {
    for rule in rules.iter_mut() {
        let range_str = rule.range.trim();
        if range_str.is_empty() {
            continue;
        }
        // 含 ~ 的动态范围不需要偏移量，已自动跟随 max_col
        if range_str.contains('~') {
            continue;
        }
        let raw = range_str.trim_start_matches('=').replace('$', "");
        let parts: Vec<&str> = raw.split(':').collect();
        let (start_col, _start_row, end_col, _end_row) = if parts.len() == 2 {
            match (parse_cell_ref_str(parts[0]), parse_cell_ref_str(parts[1])) {
                (Some(s), Some(e)) => (s.0, s.1, e.0, e.1),
                _ => continue,
            }
        } else if parts.len() == 1 {
            match parse_cell_ref_str(parts[0]) {
                Some((c, r)) => (c, r, c, r),
                None => continue,
            }
        } else {
            continue;
        };
        // 插入位置在范围内部或紧邻末尾 → 范围需要扩展
        if insert_col >= start_col && insert_col <= end_col + rule.range_expand_col + 1 {
            rule.range_expand_col += n;
        }
    }
}

/// 当插入/添加行时，更新所有受影响规则的 range_expand_row
pub fn update_alert_range_expansions_for_row(
    rules: &mut [AlertRule],
    insert_row: u32,
    n: u32,
    _sheet: &SheetData,
) {
    for rule in rules.iter_mut() {
        let range_str = rule.range.trim();
        if range_str.is_empty() {
            continue;
        }
        if range_str.contains('~') {
            continue;
        }
        let raw = range_str.trim_start_matches('=').replace('$', "");
        let parts: Vec<&str> = raw.split(':').collect();
        let (_start_col, start_row, _end_col, end_row) = if parts.len() == 2 {
            match (parse_cell_ref_str(parts[0]), parse_cell_ref_str(parts[1])) {
                (Some(s), Some(e)) => (s.0, s.1, e.0, e.1),
                _ => continue,
            }
        } else if parts.len() == 1 {
            match parse_cell_ref_str(parts[0]) {
                Some((c, r)) => (c, r, c, r),
                None => continue,
            }
        } else {
            continue;
        };
        if insert_row >= start_row && insert_row <= end_row + rule.range_expand_row + 1 {
            rule.range_expand_row += n;
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// 点击过滤逻辑
// ═══════════════════════════════════════════════════════════════

/// 合并单元格列可见性对齐（横向过滤）
///
/// 对于跨列合并：左上角单元格的值代表整个合并区域。
/// - 左上角匹配 → 整个合并范围的所有列都设为可见
/// - 左上角不匹配 → 整个合并范围的所有列都隐藏
fn expand_hidden_for_merged_cols(
    sheet: &SheetData,
    hidden_columns: &mut HashSet<u32>,
    target_row: u32,
) {
    for mr in &sheet.merged_cells {
        if mr.start_col == mr.end_col {
            continue;
        }
        if target_row < mr.start_row || target_row > mr.end_row {
            continue;
        }
        let top_left_visible = !hidden_columns.contains(&mr.start_col);
        if top_left_visible {
            for c in mr.start_col..=mr.end_col {
                hidden_columns.remove(&c);
            }
        } else {
            for c in mr.start_col..=mr.end_col {
                hidden_columns.insert(c);
            }
        }
    }
}

/// 合并单元格行可见性对齐（纵向过滤）
///
/// 对于跨行合并：左上角单元格的值代表整个合并区域。
fn expand_hidden_for_merged_rows(
    sheet: &SheetData,
    hidden_rows: &mut HashSet<u32>,
    target_col: u32,
) {
    for mr in &sheet.merged_cells {
        if mr.start_row == mr.end_row {
            continue;
        }
        if target_col < mr.start_col || target_col > mr.end_col {
            continue;
        }
        let top_left_visible = !hidden_rows.contains(&mr.start_row);
        if top_left_visible {
            for r in mr.start_row..=mr.end_row {
                hidden_rows.remove(&r);
            }
        } else {
            for r in mr.start_row..=mr.end_row {
                hidden_rows.insert(r);
            }
        }
    }
}

/// 根据触发的规则过滤表格
///
/// 横向规则：只显示匹配规则值的列，隐藏不匹配的列
/// 纵向规则：只显示匹配规则值的行，隐藏不匹配的行
pub fn filter_by_triggered_rule(
    rule: &TriggeredRule,
    sheet: &SheetData,
    hidden_columns: &mut HashSet<u32>,
    hidden_rows: &mut HashSet<u32>,
) {
    hidden_columns.clear();
    hidden_rows.clear();

    let (start_col, start_row, end_col, end_row) = match rule.resolved_range {
        Some(r) => r,
        None => return,
    };

    if rule.is_horizontal {
        // 横向过滤：从 start_col 到 end_col，同行逐单元格比对
        for col in start_col..=end_col {
            if let Some(cell) = sheet.get_cell(start_row, col) {
                if !compare_values(&cell.value, &rule.operator, &rule.value) {
                    hidden_columns.insert(col);
                }
            } else {
                // 空单元格视为不匹配
                hidden_columns.insert(col);
            }
        }

        // 处理合并单元格
        expand_hidden_for_merged_cols(sheet, hidden_columns, start_row);
    } else {
        // 纵向过滤：从 start_row 到 end_row，同列逐单元格比对
        for row in start_row..=end_row {
            if let Some(cell) = sheet.get_cell(row, start_col) {
                if !compare_values(&cell.value, &rule.operator, &rule.value) {
                    hidden_rows.insert(row);
                }
            } else {
                // 空单元格视为不匹配
                hidden_rows.insert(row);
            }
        }

        // 处理合并单元格
        expand_hidden_for_merged_rows(sheet, hidden_rows, start_col);
    }
}

// ═══════════════════════════════════════════════════════════════
// 绘制函数
// ═══════════════════════════════════════════════════════════════

/// 绘制预警通知图标（黄色小灯泡，闪烁）
///
/// 在菜单栏最右侧显示。有任何规则被触发时显示，否则隐藏。
/// 显示时以 500ms 频率闪烁（交替可见/不可见）。
pub fn draw_alert_icon(ui: &mut egui::Ui, state: &mut AlertNotifyState, dt: f32) {
    if !state.has_triggered {
        return;
    }

    // 更新闪烁计时器
    state.blink_timer += dt;
    if state.blink_timer >= BLINK_INTERVAL_SECS {
        state.blink_timer -= BLINK_INTERVAL_SECS;
    }

    // 每 500ms 切换一次可见状态：前半段可见，后半段不可见
    let visible = state.blink_timer < (BLINK_INTERVAL_SECS / 2.0);

    if !visible {
        // 仍然占用空间以保持布局稳定
        ui.add_space(20.0);
        return;
    }

    // 黄色闪烁警示图标（小灯泡）
    let response = ui.button(
        egui::RichText::new("💡")
            .size(16.0)
            .color(egui::Color32::from_rgb(255, 200, 0)),
    );

    if response.clicked() {
        state.visible = !state.visible;
    }

    // 鼠标悬停提示
    response.on_hover_text(format!(
        "{} 条预警规则已触发，点击查看详情",
        state.triggered_rules.len()
    ));
}

/// 绘制预警消息弹窗
///
/// 列出所有已触发的预警规则（红色文字），点击某条规则可过滤表格，
/// 重置按钮恢复原始显示。
pub fn draw_alert_notify_popup(
    ctx: &egui::Context,
    state: &mut AlertNotifyState,
    hidden_columns: &mut HashSet<u32>,
    hidden_rows: &mut HashSet<u32>,
    sheet: Option<&SheetData>,
) {
    if !state.visible {
        return;
    }

    let mut keep_open = true;

    egui::Window::new("alert_notify_popup")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .open(&mut keep_open)
        .default_pos(ctx.content_rect().right_center() - egui::vec2(320.0, 0.0))
        .default_size(egui::vec2(300.0, 250.0))
        .show(ctx, |ui| {
            ui.set_min_width(280.0);

            // ══════ 自定义标题栏 ══════
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("⚠ 预警消息")
                        .size(13.0)
                        .strong()
                        .color(egui::Color32::from_rgb(200, 150, 0)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // 重置按钮
                    if ui.button("🔄 重置").clicked() {
                        hidden_columns.clear();
                        hidden_rows.clear();
                        state.is_filtering = false;
                    }
                    if ui.button("✖").clicked() {
                        state.visible = false;
                    }
                });
            });

            ui.separator();

            // ══════ 规则列表 ══════
            let available_height = ui.available_height() - 10.0;

            egui::ScrollArea::vertical()
                .max_height(available_height.max(100.0))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if state.triggered_rules.is_empty() {
                        ui.label(
                            egui::RichText::new("暂无触发的预警规则")
                                .size(12.0)
                                .color(egui::Color32::GRAY),
                        );
                    } else {
                        for (idx, rule) in state.triggered_rules.iter().enumerate() {
                            // 规则消息（红色文字），可点击过滤
                            let msg_text = if rule.message.is_empty() {
                                format!(
                                    "规则{}: {} {} {} (范围: {})",
                                    idx + 1, rule.operator, rule.value, "", rule.range
                                )
                            } else {
                                rule.message.clone()
                            };

                            let label = egui::RichText::new(msg_text)
                                .size(12.0)
                                .color(egui::Color32::RED);

                            let response = ui.selectable_label(false, label);

                            if response.clicked() {
                                // 点击过滤
                                if let Some(sheet) = sheet {
                                    filter_by_triggered_rule(
                                        rule,
                                        sheet,
                                        hidden_columns,
                                        hidden_rows,
                                    );
                                    state.is_filtering = true;
                                }
                            }

                            // 悬停提示
                            response.on_hover_text(format!(
                                "点击过滤 | {} {} {} | 范围: {}",
                                rule.operator, rule.value, if rule.is_horizontal { "横向" } else { "纵向" }, rule.range
                            ));
                        }
                    }
                });

            // 底部提示
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new("💡 点击预警消息过滤表格，点击「重置」恢复")
                    .size(10.0)
                    .color(egui::Color32::from_rgb(140, 140, 140)),
            );
        });

    if !keep_open {
        state.visible = false;
    }
}
