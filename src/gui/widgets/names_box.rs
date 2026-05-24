//! 名称框组件
//! 
//! 实现 Excel 名称框功能，包括：
//! - 当前单元格位置显示
//! - 快速定位跳转功能
//! - 定义名称管理功能

use eframe::egui;
use crate::excel::reader::col_to_letter;

/// 解析单元格坐标字符串（如 "A1", "H8", "AA100"）
/// 
/// # 参数
/// * `input` - 输入的坐标字符串
/// 
/// # 返回值
/// 成功返回 (列号, 行号)，失败返回 None
fn parse_cell_reference(input: &str) -> Option<(u32, u32)> {
    let input = input.trim().to_uppercase();
    
    if input.is_empty() {
        return None;
    }
    
    let mut col_chars = String::new();
    let mut row_chars = String::new();
    
    for c in input.chars() {
        if c.is_alphabetic() {
            col_chars.push(c);
        } else if c.is_numeric() {
            row_chars.push(c);
        } else {
            return None;
        }
    }
    
    if col_chars.is_empty() || row_chars.is_empty() {
        return None;
    }
    
    // 将字母列转换为数字（A=1, B=2, ..., Z=26, AA=27...）
    let mut col: u32 = 0;
    for c in col_chars.chars() {
        if c < 'A' || c > 'Z' {
            return None;
        }
        col = col * 26 + (c as u32 - 'A' as u32 + 1);
    }
    
    // 解析行号
    let row: u32 = match row_chars.parse() {
        Ok(r) => r,
        Err(_) => return None,
    };
    
    if col == 0 || row == 0 {
        return None;
    }
    
    Some((col, row))
}

/// 名称框组件状态
#[derive(Clone)]
pub struct NameBoxState {
    /// 输入框内容（单元格位置）
    pub input_text: String,
    /// 当前单元格的公式
    pub formula_text: String,
    /// 是否显示下拉菜单
    pub show_dropdown: bool,
    /// 输入框是否有焦点
    pub has_focus: bool,
    /// 输入框唯一 ID
    pub input_id: egui::Id,
    /// 公式输入框是否有焦点
    pub formula_has_focus: bool,
}

impl Default for NameBoxState {
    fn default() -> Self {
        Self {
            input_text: String::new(),
            formula_text: String::new(),
            show_dropdown: false,
            has_focus: false,
            input_id: egui::Id::new("name_box_input"),
            formula_has_focus: false,
        }
    }
}

/// 绘制名称框
/// 
/// # 参数
/// * `ui` - egui UI 上下文
/// * `state` - 名称框状态
/// * `selected_cell` - 当前选中的单元格
/// * `formula` - 当前单元格的公式（可选）
/// * `max_col` - 表格最大列数
/// * `max_row` - 表格最大行数
/// * `pending_save` - 待保存的公式值（输出参数）
/// 
/// # 返回值
/// 如果用户输入了有效的单元格坐标并按下回车，返回 Some((列号, 行号))，否则返回 None
pub fn draw_name_box(
    ui: &mut egui::Ui,
    state: &mut NameBoxState,
    selected_cell: Option<(u32, u32)>,
    formula: Option<&str>,
    max_col: u32,
    max_row: u32,
    pending_save: &mut Option<String>,
) -> Option<(u32, u32)> {
    let mut result: Option<(u32, u32)> = None;
    
    // 创建水平布局
    ui.horizontal(|ui| {
        // 设置名称框样式
        let text_style = egui::TextStyle::Body;
        let font_id = ui.style().text_styles.get(&text_style).cloned().unwrap_or_default();
        
        // 名称框输入区域
        let input_response = ui.add(
            egui::TextEdit::singleline(&mut state.input_text)
                .id(state.input_id)
                .font(font_id.clone())
                .desired_width(80.0)
                .hint_text("名称框")
        );
        
        // 检测焦点状态
        state.has_focus = input_response.has_focus();
        
        // 处理键盘事件
        if input_response.has_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Some((col, row)) = parse_cell_reference(&state.input_text) {
                    // 验证行号和列号是否有效
                    if col <= max_col && row <= max_row {
                        result = Some((col, row));
                    }
                }
            }
        }
        
        // 下拉箭头按钮
        let dropdown_response = ui.add(
            egui::Button::new("▼")
                .small()
                .min_size(egui::vec2(20.0, 0.0))
        );
        
        if dropdown_response.clicked() {
            state.show_dropdown = !state.show_dropdown;
        }
        
        // 显示下拉菜单
        if state.show_dropdown {
            egui::Area::new(egui::Id::new("name_box_dropdown"))
                .fixed_pos(dropdown_response.rect.left_top())
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(150.0);
                        ui.label("定义名称...");
                        ui.separator();
                        ui.label("管理名称...");
                    });
                });
        }
        
        // 分隔线
        ui.add(egui::Separator::default().vertical());
        
        // 公式栏区域
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            // 插入函数按钮
            ui.add(egui::Button::new("fx").small());
            ui.add(egui::Separator::default().vertical());
            
            // 公式输入框
            let formula_response = ui.add(
                egui::TextEdit::singleline(&mut state.formula_text)
                    .font(font_id)
                    .hint_text("输入公式...")
                    .desired_width(200.0)
            );
            
            // 检测公式输入框焦点状态
            state.formula_has_focus = formula_response.has_focus();
            
            // 处理公式栏的回车键，设置待保存值
            if formula_response.has_focus() {
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    if !state.formula_text.is_empty() {
                        *pending_save = Some(state.formula_text.clone());
                    }
                }
            }
        });
    });
    
    // 如果选中单元格发生变化且输入框没有焦点，更新显示
    if !state.has_focus {
        if let Some((col, row)) = selected_cell {
            let cell_ref = format!("{}{}", col_to_letter(col), row);
            if state.input_text != cell_ref {
                state.input_text = cell_ref;
            }
        } else {
            if !state.input_text.is_empty() {
                state.input_text.clear();
            }
        }
    }
    
    // 如果选中单元格发生变化且公式输入框没有焦点，更新公式显示
    if !state.formula_has_focus {
        match formula {
            Some(f) if !f.is_empty() => {
                if state.formula_text != f {
                    state.formula_text = f.to_string();
                }
            }
            _ => {
                if !state.formula_text.is_empty() {
                    state.formula_text.clear();
                }
            }
        }
    }
    
    result
}
