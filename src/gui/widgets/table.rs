//! 表格渲染组件
//! 
//! 负责渲染 Excel 表格内容，包括单元格、合并单元格和对齐方式

use eframe::egui;
use crate::excel::reader::{CellAlignment, CellData, ExcelData, col_to_letter};
use crate::gui::alignment::alignment_to_egui;
use std::borrow::Cow;
use std::collections::HashSet;

/// 获取单元格的显示文本，处理日期格式转换
/// 返回 Cow<str> 避免无谓的 String clone
fn cell_display_text<'a>(cell: &'a CellData) -> Cow<'a, str> {
    if let Some(ref fmt) = cell.number_format {
        if ExcelData::is_date_format(fmt) {
            if let Ok(serial) = cell.value.parse::<f64>() {
                return Cow::Owned(ExcelData::format_date(serial, fmt));
            }
        }
    }
    Cow::Borrowed(&cell.value)
}

/// 在单元格右上角绘制红色批注指示三角（Comment indicator）
fn draw_comment_indicator(painter: &egui::Painter, x: f32, y: f32, width: f32) {
    const SIZE: f32 = 7.0;
    let points = vec![
        egui::Pos2::new(x + width - SIZE, y),
        egui::Pos2::new(x + width, y),
        egui::Pos2::new(x + width, y + SIZE),
    ];
    painter.add(egui::Shape::convex_polygon(
        points,
        egui::Color32::from_rgb(217, 83, 25),
        egui::Stroke::NONE,
    ));
}

/// 绘制表格内容
/// 
/// 使用虚拟渲染技术，只绘制可见区域的单元格，提高性能
/// 
/// # 参数
/// * `ui` - egui UI 上下文
/// * `excel_data` - Excel 数据引用（可变引用，用于编辑）
/// * `current_sheet` - 当前工作表索引
/// * `selected_cell` - 当前选中单元格（可变引用，用于更新选中状态）
/// * `editing_cell` - 当前正在编辑的单元格（可变引用）
/// * `edit_value` - 当前编辑的值（可变引用）
/// * `just_entered_edit_mode` - 是否刚进入编辑模式（用于忽略进入编辑时的Enter键）
/// 
/// # 返回值
/// 返回需要滚动到的目标矩形（用于键盘导航时自动滚动），如果没有则返回 None
pub fn draw_table_content(
    ui: &mut egui::Ui,
    excel_data: &mut ExcelData,
    current_sheet: usize,
    selected_cell: &mut Option<(u32, u32)>,
    selected_range: &mut Option<(u32, u32, u32, u32)>,
    editing_cell: &mut Option<(u32, u32)>,
    edit_value: &mut String,
    just_entered_edit_mode: &mut bool,
    validation_error: &mut Option<(String, String)>,
    original_cell_data: &mut Option<((u32, u32), String, String)>,
    context_menu: &mut crate::gui::viewer::ContextMenuState,
    dirty: &mut bool,
    drag_anchor: &mut Option<(u32, u32)>,
    hidden_columns: &HashSet<u32>,
    hidden_rows: &HashSet<u32>,
) -> (Option<egui::Rect>, Option<egui::Rect>) {
    // 先获取必要的数据用于键盘处理
    let (max_col, max_row, frozen_rows, frozen_cols) = if let Some(sheet) = excel_data.get_sheet(current_sheet) {
        (sheet.max_col, sheet.max_row, sheet.frozen_rows, sheet.frozen_cols)
    } else {
        return (None, None);
    };
    
    // 如果已经在编辑模式中，重置"刚进入编辑模式"标志位
    // 这样下一帧就可以正常处理Enter键了
    if editing_cell.is_some() && *just_entered_edit_mode {
        *just_entered_edit_mode = false;
    }
    
    // 定义默认列宽和行高（与渲染部分保持一致）
    let default_col_width = 80.0;   // 默认列宽（像素）
    let default_row_height = 25.0;  // 默认行高（像素）
    let header_width = 60.0;        // 行号列宽度
    let border_width = 1.0;         // 边框宽度
    
    // 获取sheet用于获取列宽和行高
    let sheet = excel_data.get_sheet(current_sheet);
    
    // 获取列宽的辅助函数
    let get_col_width = |col: u32| -> f32 {
        if let Some(s) = sheet {
            if let Some(&width) = s.column_widths.get(&col) {
                return width as f32 * 8.0;
            }
        }
        default_col_width
    };

    // 获取行高的辅助函数
    let get_row_height = |row: u32| -> f32 {
        if let Some(s) = sheet {
            if let Some(&height) = s.row_heights.get(&row) {
                return (height as f32 * 1.333).max(default_row_height);
            }
        }
        default_row_height
    };

    // 一次性构建累积宽度/高度数组，后续全部通过数组索引取值（避免每帧重复 HashMap 查询）
    let mut col_cumulative_width = vec![0.0];
    let mut cur_w = 0.0;
    cur_w += header_width + border_width;
    col_cumulative_width.push(cur_w);
    for col in 1..=max_col {
        // 隐藏列宽度贡献为 0，确保后续 col_cumulative_width[col] 索引正确
        // 且 partition_point 点击检测不受隐藏列影响
        if !hidden_columns.contains(&col) {
            cur_w += get_col_width(col) + border_width;
        }
        col_cumulative_width.push(cur_w);
    }

    let mut row_cumulative_height = vec![0.0];
    let mut cur_h = 0.0;
    for row in 0..=max_row {
        // 隐藏行高度贡献为 0
        if !hidden_rows.contains(&row) {
            cur_h += get_row_height(row) + border_width;
        }
        row_cumulative_height.push(cur_h);
    }

    // 从累积数组推导总尺寸和冻结区域尺寸（替代循环累加）
    let total_width = col_cumulative_width.last().copied().unwrap_or(0.0) + 11.0;
    let total_height = row_cumulative_height.last().copied().unwrap_or(0.0) + border_width + 11.0;
    let frozen_left_width = col_cumulative_width.get((frozen_cols + 1) as usize).copied().unwrap_or(0.0);
    let frozen_top_height = row_cumulative_height.get((frozen_rows + 1) as usize).copied().unwrap_or(0.0);
    
    // 使用 allocate_space 分配表格空间
    let (_id, rect) = ui.allocate_space(egui::vec2(total_width, total_height));
    let table_top_left = rect.min;
    
    // 计算单元格像素矩形的辅助函数（相对于表格左上角）
    // 仅用于键盘导航，不在渲染热路径上
    let get_cell_rect = |col: u32, row: u32| -> (f32, f32, f32, f32) {
        let mut x = header_width; // 行号列
        for c in 1..col {
            x += get_col_width(c) + border_width;
        }

        let mut y = 0.0; // 表头行
        for r in 0..row {
            y += get_row_height(r) + border_width;
        }

        let width = get_col_width(col);
        let height = get_row_height(row);

        (x, y, width, height)
    };
    
    // frozen_left_width / frozen_top_height 已从累积数组推导（见上方），此处直接使用

    // 检查单元格是否在可视区域内（使用全局坐标）
    // 有效可见区域 = clip_rect 减去冻结窗格区域（左侧冻结列 + 顶部冻结行）
    // 检查逻辑：单元格左/上边缘必须在冻结区域之后，右/下边缘在视口之内
    let is_cell_in_viewport = |col: u32, row: u32| -> bool {
        let (x, y, width, height) = get_cell_rect(col, row);
        let cell_rect = egui::Rect::from_min_size(
            egui::Pos2::new(x + table_top_left.x, y + table_top_left.y),
            egui::vec2(width, height)
        );
        let clip_rect = ui.clip_rect();

        // 有效数据区域：左边界 = 冻结左侧区域右边缘，上边界 = 冻结顶部区域下边缘
        let effective_min_x = clip_rect.min.x + frozen_left_width;
        let effective_min_y = clip_rect.min.y + frozen_top_height;

        // 单元格四条边都必须在有效区域内
        cell_rect.min.x >= effective_min_x
            && cell_rect.max.x <= clip_rect.max.x
            && cell_rect.min.y >= effective_min_y
            && cell_rect.max.y <= clip_rect.max.y
    };
    
    // 获取单元格的全局坐标（用于滚动）
    // 如果目标单元格属于合并范围，返回合并区域的完整矩形
    let get_cell_global_rect = |col: u32, row: u32| -> egui::Rect {
        // 检查目标单元格是否属于合并单元格，如果是则使用合并区域的完整矩形
        if let Some(s) = sheet {
            if let Some(merged_range) = s.get_merged_range(col, row) {
                // 从合并区域的左上角开始计算位置
                let (x, y, _, _) = get_cell_rect(merged_range.start_col, merged_range.start_row);

                // 累加合并区域所有列的宽度
                let mut total_width = 0.0f32;
                for c in merged_range.start_col..=merged_range.end_col {
                    total_width += get_col_width(c) + border_width;
                }
                total_width -= border_width;

                // 累加合并区域所有行的高度
                let mut total_height = 0.0f32;
                for r in merged_range.start_row..=merged_range.end_row {
                    total_height += get_row_height(r) + border_width;
                }
                total_height -= border_width;

                return egui::Rect::from_min_size(
                    egui::Pos2::new(x + table_top_left.x, y + table_top_left.y),
                    egui::vec2(total_width, total_height)
                );
            }
        }
        // 非合并单元格，使用原有逻辑
        let (x, y, width, height) = get_cell_rect(col, row);
        egui::Rect::from_min_size(
            egui::Pos2::new(x + table_top_left.x, y + table_top_left.y),
            egui::vec2(width, height)
        )
    };

    // 校验错误弹窗显示时，锁定表格交互（禁止键盘导航、点击选中等操作）
    let validation_error_active = validation_error.is_some();

    // 键盘事件处理
    let input = ui.input(|i| i.clone());
    let mut save_current_edit = false;
    let mut clear_current_edit = false;
    let mut enter_edit_mode = false;
    let mut new_selected_cell: Option<(u32, u32)> = None;
    let editing_cell_for_save = editing_cell.clone();

    // 用于存储滚动目标矩形
    let mut scroll_to_rect: Option<egui::Rect> = None;
    // 用于存储选中单元格屏幕矩形，供数据有效性弹窗定位
    let mut selected_cell_rect: Option<egui::Rect> = None;
    
    // Tab键处理（编辑模式下）
    // 保存并退出编辑
    if (input.key_pressed(egui::Key::Tab) || (input.modifiers.shift && input.key_pressed(egui::Key::Tab))) && editing_cell.is_some() {
        save_current_edit = true;
        clear_current_edit = true;
        // 消费Tab键事件，防止传递到菜单栏
        ui.input_mut(|i| i.consume_key(input.modifiers, egui::Key::Tab));
    }
    
    // Tab键处理（非编辑模式下）- 在表格有焦点时进行单元格切换
    if (input.key_pressed(egui::Key::Tab) || (input.modifiers.shift && input.key_pressed(egui::Key::Tab))) && editing_cell.is_none() {
        if let Some((col, row)) = *selected_cell {
            let mut new_col = col;
            let mut new_row = row;
            let _direction = if input.modifiers.shift { "Shift+Tab (左)" } else { "Tab (右)" };
            
            // 获取sheet用于检查合并单元格
            let sheet = excel_data.get_sheet(current_sheet);
            
            if input.modifiers.shift {
                // Shift+Tab: 向左移动
                if col > 1 {
                    // 检查当前单元格是否是合并单元格的一部分
                    let current_col = if let Some(s) = sheet {
                        if let Some(merged_range) = s.get_merged_range(col, row) {
                            merged_range.start_col
                        } else {
                            col
                        }
                    } else {
                        col
                    };
                    
                    if current_col > 1 {
                        new_col = current_col - 1;
                        // 检查新位置是否是合并单元格，如果是，使用合并区域的起始列
                        if let Some(s) = sheet {
                            if let Some(merged_range) = s.get_merged_range(new_col, row) {
                                new_col = merged_range.start_col;
                            }
                        }
                    } else if row > 1 {
                        // 已经在最左边，跳到上一行最后一列
                        new_col = max_col;
                        new_row = row - 1;
                        // 检查新位置是否是合并单元格，如果是，使用合并区域的起始列
                        if let Some(s) = sheet {
                            if let Some(merged_range) = s.get_merged_range(new_col, new_row) {
                                new_col = merged_range.start_col;
                            }
                        }
                    }
                }
            } else {
                // Tab: 向右移动
                // 检查当前单元格是否是合并单元格的一部分
                let current_col = if let Some(s) = sheet {
                    if let Some(merged_range) = s.get_merged_range(col, row) {
                        merged_range.end_col
                    } else {
                        col
                    }
                } else {
                    col
                };
                
                if current_col < max_col {
                    new_col = current_col + 1;
                    // 检查新位置是否是合并单元格，如果是，使用合并区域的起始列
                    if let Some(s) = sheet {
                        if let Some(merged_range) = s.get_merged_range(new_col, row) {
                            new_col = merged_range.start_col;
                        }
                    }
                } else if row < max_row {
                    // 已经在最右边，跳到下一行第一列
                    new_col = 1;
                    new_row = row + 1;
                    // 检查新位置是否是合并单元格，如果是，使用合并区域的起始列
                    if let Some(s) = sheet {
                        if let Some(merged_range) = s.get_merged_range(new_col, new_row) {
                            new_col = merged_range.start_col;
                        }
                    }
                }
            }
            
            if new_col != col || new_row != row {
                new_selected_cell = Some((new_col, new_row));

                // 触边滚动机制：只有当新单元格不在可视区域内时才触发滚动
                if !is_cell_in_viewport(new_col, new_row) {
                    // 使用全局坐标进行滚动
                    let target_rect = get_cell_global_rect(new_col, new_row);
                    let clip_rect = ui.clip_rect();

                    // Excel行为：滚动最小距离使新单元格可见（滚动1行/列）
                    // 补偿冻结窗格：对比有效区域边界（而非 clip_rect）
                    let effective_min_x = clip_rect.min.x + frozen_left_width;
                    let effective_min_y = clip_rect.min.y + frozen_top_height;
                    let mut scroll_rect = target_rect;
                    if target_rect.min.x < effective_min_x {
                        scroll_rect.min.x = target_rect.min.x - frozen_left_width;
                    }
                    if target_rect.min.y < effective_min_y {
                        scroll_rect.min.y = target_rect.min.y - frozen_top_height;
                    }
                    ui.scroll_to_rect(scroll_rect, None);
                    ui.ctx().request_repaint();
                    scroll_to_rect = Some(target_rect);
                }
            } else {
            }
        } else {
        }
        // 消费Tab键事件，防止传递到菜单栏
        ui.input_mut(|i| i.consume_key(input.modifiers, egui::Key::Tab));
    }
    
    // 方向键处理（非编辑模式下）- 在表格有焦点时进行单元格切换
    if editing_cell.is_none() && selected_cell.is_some() {
        if let Some((col, row)) = *selected_cell {
            let mut new_col = col;
            let mut new_row = row;
            let mut direction = String::new();
            
            // 获取sheet用于检查合并单元格
            let sheet = excel_data.get_sheet(current_sheet);
            
            if input.key_pressed(egui::Key::ArrowUp) {
                direction = "上".to_string();
                // 向上移动
                if row > 1 {
                    new_row = row - 1;
                    // 检查新位置是否是合并单元格，如果是，使用合并区域的起始行
                    if let Some(s) = sheet {
                        if let Some(merged_range) = s.get_merged_range(col, new_row) {
                            new_row = merged_range.start_row;
                        }
                    }
                }
            } else if input.key_pressed(egui::Key::ArrowDown) {
                direction = "下".to_string();
                // 向下移动
                if row < max_row {
                    new_row = row + 1;
                    // 检查新位置是否是合并单元格，如果是，使用合并区域的起始行
                    if let Some(s) = sheet {
                        if let Some(merged_range) = s.get_merged_range(col, new_row) {
                            new_row = merged_range.start_row;
                        }
                    }
                }
            } else if input.key_pressed(egui::Key::ArrowLeft) {
                direction = "左".to_string();
                // 向左移动
                if col > 1 {
                    // 检查当前单元格是否是合并单元格的一部分
                    let current_col = if let Some(s) = sheet {
                        if let Some(merged_range) = s.get_merged_range(col, row) {
                            merged_range.start_col
                        } else {
                            col
                        }
                    } else {
                        col
                    };
                    
                    if current_col > 1 {
                        new_col = current_col - 1;
                        // 检查新位置是否是合并单元格，如果是，使用合并区域的起始列
                        if let Some(s) = sheet {
                            if let Some(merged_range) = s.get_merged_range(new_col, row) {
                                new_col = merged_range.start_col;
                            }
                        }
                    }
                }
            } else if input.key_pressed(egui::Key::ArrowRight) {
                direction = "右".to_string();
                // 向右移动
                // 检查当前单元格是否是合并单元格的一部分
                let current_col = if let Some(s) = sheet {
                    if let Some(merged_range) = s.get_merged_range(col, row) {
                        merged_range.end_col
                    } else {
                        col
                    }
                } else {
                    col
                };
                
                if current_col < max_col {
                    new_col = current_col + 1;
                    // 检查新位置是否是合并单元格，如果是，使用合并区域的起始列
                    if let Some(s) = sheet {
                        if let Some(merged_range) = s.get_merged_range(new_col, row) {
                            new_col = merged_range.start_col;
                        }
                    }
                }
            }
            
            if !direction.is_empty() {
                if new_col != col || new_row != row {
                    new_selected_cell = Some((new_col, new_row));

                    // 触边滚动机制：只有当新单元格不在可视区域内时才触发滚动
                    if !is_cell_in_viewport(new_col, new_row) {
                        // 使用全局坐标进行滚动
                        let target_rect = get_cell_global_rect(new_col, new_row);
                        let clip_rect = ui.clip_rect();

                        // Excel行为：滚动最小距离使新单元格可见（即滚动1行/列）
                        // 补偿冻结窗格：对比有效区域边界（而非 clip_rect）
                        let effective_min_x = clip_rect.min.x + frozen_left_width;
                        let effective_min_y = clip_rect.min.y + frozen_top_height;
                        let mut scroll_rect = target_rect;
                        if target_rect.min.x < effective_min_x {
                            scroll_rect.min.x = target_rect.min.x - frozen_left_width;
                        }
                        if target_rect.min.y < effective_min_y {
                            scroll_rect.min.y = target_rect.min.y - frozen_top_height;
                        }
                        ui.scroll_to_rect(scroll_rect, None);

                        // 请求立即重绘，确保滚动生效
                        ui.ctx().request_repaint();

                        scroll_to_rect = Some(target_rect);
                    }

                    // 消费方向键事件
                    ui.input_mut(|i| {
                        if input.key_pressed(egui::Key::ArrowUp) {
                            i.consume_key(input.modifiers, egui::Key::ArrowUp);
                        } else if input.key_pressed(egui::Key::ArrowDown) {
                            i.consume_key(input.modifiers, egui::Key::ArrowDown);
                        } else if input.key_pressed(egui::Key::ArrowLeft) {
                            i.consume_key(input.modifiers, egui::Key::ArrowLeft);
                        } else if input.key_pressed(egui::Key::ArrowRight) {
                            i.consume_key(input.modifiers, egui::Key::ArrowRight);
                        }
                    });
                }
            }
        }
    }
    
    // Enter键处理
    // 只有在非编辑模式下按Enter才进入编辑模式
    // 编辑模式下的Enter键处理交给输入框自己处理（见下方输入框逻辑）
    if input.key_pressed(egui::Key::Enter) {
        if editing_cell.is_none() && selected_cell.is_some() {
            enter_edit_mode = true;
            *just_entered_edit_mode = true;
            // 消费Enter键事件，防止被名称框检测到
            ui.input_mut(|i| i.consume_key(input.modifiers, egui::Key::Enter));
        }
    }
    
    // 执行状态更新（在借用之前完成）
    if save_current_edit {
        if let Some((edit_col, edit_row)) = editing_cell_for_save {
            let is_formula = edit_value.starts_with('=');
            // 非公式值做数据有效性校验
            if !is_formula {
                if let Some(sheet) = excel_data.sheets.get(current_sheet) {
                    if let Some((title, msg)) = sheet.validate_cell(edit_col, edit_row, edit_value) {
                        *validation_error = Some((title, msg));
                        save_current_edit = false;
                        clear_current_edit = false;
                    }
                }
            }
            // 只有校验通过才执行保存
            if save_current_edit {
                let is_formula = edit_value.starts_with('=');
                if let Some(sheet) = excel_data.sheets.get_mut(current_sheet) {
                    let cell = sheet.cells.entry((edit_row, edit_col))
                        .or_insert_with(CellData::default);
                    if is_formula {
                        cell.formula = edit_value.clone();
                    } else {
                        // 检查是否为日期格式单元格，转换日期字符串为序列号
                        let save_value = if let Some(ref fmt) = cell.number_format {
                            if ExcelData::is_date_format(fmt) {
                                ExcelData::parse_date_string(edit_value)
                                    .map(|serial| serial.to_string())
                                    .unwrap_or_else(|| edit_value.clone())
                            } else {
                                edit_value.clone()
                            }
                        } else {
                            edit_value.clone()
                        };
                        cell.value = save_value;
                        cell.formula.clear();
                    }
                }
                if is_formula {
                    // 公式变更需要全量求值
                    crate::excel::formula::evaluate_sheet(&mut excel_data.sheets[current_sheet]);
                } else {
                    // 值变更只需增量求值受影响的公式
                    crate::excel::formula::evaluate_dependents(&mut excel_data.sheets[current_sheet], edit_row, edit_col);
                }
                *dirty = true;
            }
        }
    }
    if clear_current_edit {
        *editing_cell = None;
        edit_value.clear();
    }
    if let Some(cell) = new_selected_cell {
        *selected_cell = Some(cell);
    }
    
    // 处理Enter进入编辑模式（需要重新获取sheet）
    if enter_edit_mode {
        if let Some((col, row)) = *selected_cell {  // 使用更新后的selected_cell
            if let Some(sheet) = excel_data.get_sheet(current_sheet) {
                let (edit_col, edit_row) = if let Some(merged_range) = sheet.get_merged_range(col, row) {
                    (merged_range.start_col, merged_range.start_row)
                } else {
                    (col, row)
                };
                
                *editing_cell = Some((edit_col, edit_row));
                *edit_value = sheet.get_cell(edit_row, edit_col)
                    .map(|cell| {
                        if !cell.formula.is_empty() {
                            // 确保公式以 = 开头，使保存逻辑正确识别为公式
                            let f = &cell.formula;
                            if f.starts_with('=') {
                                f.clone()
                            } else {
                                format!("={}", f)
                            }
                        } else {
                            cell_display_text(cell).into_owned()
                        }
                    })
                    .unwrap_or_default();
                // 保存原始单元格数据，用于校验失败时恢复
                let orig = sheet.get_cell(edit_row, edit_col)
                    .map(|c| (c.value.clone(), c.formula.clone()))
                    .unwrap_or_default();
                *original_cell_data = Some(((edit_col, edit_row), orig.0, orig.1));
            }
        }
    }

    // 校验弹窗锁定时消费所有按键，防止穿透
    if validation_error_active {
        ui.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::Tab);
            i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab);
            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown);
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft);
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight);
        });
    }

    // 现在开始渲染（获取不可变借用）
    if let Some(sheet) = excel_data.get_sheet(current_sheet) {
        // 表格渲染常量定义
        let default_row_height = 25.0; // 默认行高
        let default_col_width = 80.0;  // 默认列宽
        let header_width = 60.0;       // 行号列宽度
        let border_width = 1.0;        // 边框宽度
        
        // 获取列宽的辅助函数
        let get_col_width = |col: u32| -> f32 {
            if let Some(&width) = sheet.column_widths.get(&col) {
                width as f32 * 8.0
            } else {
                default_col_width
            }
        };
        
        // 获取行高的辅助函数（从 Excel 读取，或使用默认值）
        let get_row_height = |row: u32| -> f32 {
            if let Some(&height) = sheet.row_heights.get(&row) {
                // Excel 行高单位是磅，转换为像素（1磅 ≈ 1.333像素）
                // 使用 max 确保行高不小于默认行高
                (height as f32 * 1.333).max(default_row_height)
            } else {
                default_row_height
            }
        };

        // 复用外层已构建的累积数组和尺寸（避免每帧重复 HashMap 查询）
        // col_cumulative_width / row_cumulative_height / frozen_left_width /
        // frozen_top_height / total_width / total_height 均在外层计算

        // 外层已分配空间，直接使用保存的 table_top_left 和 total_width / total_height 构建 rect
        let rect = egui::Rect::from_min_size(table_top_left, egui::vec2(total_width, total_height));
        let top_left = table_top_left;

        // 获取painter用于绘制
        let painter = ui.painter_at(rect);

        // 创建交互区域来处理点击事件（使用同一个rect）
        let response = ui.interact(rect, egui::Id::new("table_interaction"), egui::Sense::click_and_drag());

        // 如果表格被点击，请求焦点
        if response.clicked() {
            response.request_focus();
        }

        // 如果选中了单元格但表格没有焦点，重新请求焦点
        // 仅在Tab键或方向键操作时请求焦点，不每帧强制抢焦点
        // 否则会阻止名称框/公式输入框获取焦点
        if !editing_cell.is_some() && selected_cell.is_some() && input.key_pressed(egui::Key::Tab) {
            response.request_focus();
        }


        let tl_x = top_left.x;
        let tl_y = top_left.y;

        // 绘制灰色背景
        painter.rect_filled(
            egui::Rect::from_min_size(egui::Pos2::new(tl_x, tl_y), egui::vec2(total_width, total_height)),
            0.0,
            egui::Color32::GRAY,
        );

        // 获取当前可见区域，用于虚拟渲染
        let viewport_rect = ui.clip_rect();
        let margin = 100.0; // 适当的边距即可，不需要太大

        // 根据实际列宽计算可见列范围
        let target_start_x = viewport_rect.min.x - tl_x - margin;
        let target_end_x = viewport_rect.max.x - tl_x + margin;

        // 二分查找可见列范围（累积数组严格单调递增）
        let visible_cols_start = col_cumulative_width
            .partition_point(|&w| w <= target_start_x)
            .saturating_sub(1)
            .max(0) as u32;
        let visible_cols_end = col_cumulative_width
            .partition_point(|&w| w <= target_end_x)
            .min(sheet.max_col as usize + 1) as u32;

        // 确保第 0 列（行号列）始终可见
        // (visible_cols_start 已 >= 0，无需额外处理)

        // 二分查找可见行范围（累积数组严格单调递增）
        let target_start_y = viewport_rect.min.y - tl_y - margin;
        let target_end_y = viewport_rect.max.y - tl_y + margin;

        let visible_rows_start = row_cumulative_height
            .partition_point(|&h| h <= target_start_y)
            .saturating_sub(1)
            .max(0) as u32;
        let visible_rows_end = row_cumulative_height
            .partition_point(|&h| h <= target_end_y)
            .min(sheet.max_row as usize) as u32;

        // 确保第0行（列标题行）始终可见
        // (visible_rows_start 已 >= 0，无需额外处理)

        // 冻结区域边界：主网格渲染跳过这些行列，由冻结覆盖层单独绘制
        let fr = sheet.frozen_rows;
        let fc = sheet.frozen_cols;

        // 处理点击事件（校验错误弹窗显示时禁止点击）
        if response.clicked() && !validation_error_active {
            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                // 冻结区域在视口上位置固定，不随滚动变化
                // 因此需要根据点击位置是否在冻结区域内，选择不同的坐标参考系
                let in_frozen_left = pos.x < viewport_rect.min.x + frozen_left_width;
                let in_frozen_top = pos.y < viewport_rect.min.y + frozen_top_height;

                // 冻结区域使用视口相对坐标（不随滚动变化），非冻结区域使用表格内容坐标
                let click_x = if in_frozen_left {
                    pos.x - viewport_rect.min.x
                } else {
                    pos.x - tl_x
                };
                let click_y = if in_frozen_top {
                    pos.y - viewport_rect.min.y
                } else {
                    pos.y - tl_y
                };

                // 查找被点击的列（二分查找，累积数组严格单调递增）
                let col_idx = col_cumulative_width.partition_point(|&w| w <= click_x);
                let clicked_col = if col_idx > 1 { Some(col_idx as u32 - 1) } else { None };

                // 查找被点击的行（二分查找，累积数组严格单调递增）
                let row_idx = row_cumulative_height.partition_point(|&h| h <= click_y);
                let clicked_row = if row_idx > 0 { Some(row_idx as u32 - 1) } else { None };

                // 更新选中单元格（保持 col, row 顺序）
                if let (Some(col), Some(row)) = (clicked_col, clicked_row) {
                    if col > 0 && row > 0 {
                        *selected_cell = Some((col, row));
                        
                        // 处理双击事件，进入编辑模式
                        if response.double_clicked() {
                            // 检查是否是合并单元格，如果是则获取左上角单元格
                            let (edit_col, edit_row) = if let Some(merged_range) = sheet.get_merged_range(col, row) {
                                // 合并单元格：使用左上角单元格
                                (merged_range.start_col, merged_range.start_row)
                            } else {
                                // 普通单元格：使用当前单元格
                                (col, row)
                            };
                            
                            *editing_cell = Some((edit_col, edit_row));
                            // 有公式的单元格显示公式，无公式的显示值
                            *edit_value = sheet.get_cell(edit_row, edit_col)
                                .map(|cell| {
                                    if !cell.formula.is_empty() {
                                        let f = &cell.formula;
                                        if f.starts_with('=') {
                                            f.clone()
                                        } else {
                                            format!("={}", f)
                                        }
                                    } else {
                                        cell_display_text(cell).into_owned()
                                    }
                                })
                                .unwrap_or_default();
                            // 保存原始单元格数据，用于校验失败时恢复
                            let orig = sheet.get_cell(edit_row, edit_col)
                                .map(|c| (c.value.clone(), c.formula.clone()))
                                .unwrap_or_default();
                            *original_cell_data = Some(((edit_col, edit_row), orig.0, orig.1));
                        }
                    }
                }
            }
        }

        // 右键点击：打开上下文菜单（校验错误弹窗显示时禁止）
        if response.secondary_clicked() && !validation_error_active {
            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                let in_frozen_left = pos.x < viewport_rect.min.x + frozen_left_width;
                let in_frozen_top = pos.y < viewport_rect.min.y + frozen_top_height;
                let click_x = if in_frozen_left { pos.x - viewport_rect.min.x } else { pos.x - tl_x };
                let click_y = if in_frozen_top { pos.y - viewport_rect.min.y } else { pos.y - tl_y };

                let col_idx = col_cumulative_width.partition_point(|&w| w <= click_x);
                let clicked_col = if col_idx > 1 { Some(col_idx as u32 - 1) } else { None };
                let row_idx = row_cumulative_height.partition_point(|&h| h <= click_y);
                let clicked_row = if row_idx > 0 { Some(row_idx as u32 - 1) } else { None };

                if let (Some(col), Some(row)) = (clicked_col, clicked_row) {
                    if col > 0 && row > 0 {
                        *selected_cell = Some((col, row));
                        let (default_rows, default_cols) = (
                            sheet.default_insert_count(col, row, true),
                            sheet.default_insert_count(col, row, false),
                        );
                        context_menu.visible = true;
                        context_menu.position = pos;
                        context_menu.target_cell = Some((col, row));
                        context_menu.insert_rows_count = default_rows;
                        context_menu.insert_cols_count = default_cols;
                    }
                }
            }
        }

        // ========== 拖拽选择：按住左键拖拽扩大选中范围 ==========
        // 使用与点击处理相同的坐标转换逻辑（冻结区域感知）
        // 兼容合并单元格：锚点和当前格都会展开到所在合并区域的完整边界
        if !validation_error_active && editing_cell.is_none() {
            let screen_to_cell = |pos: egui::Pos2| -> Option<(u32, u32)> {
                let in_frozen_left = pos.x < viewport_rect.min.x + frozen_left_width;
                let in_frozen_top = pos.y < viewport_rect.min.y + frozen_top_height;
                let rel_x = if in_frozen_left { pos.x - viewport_rect.min.x } else { pos.x - tl_x };
                let rel_y = if in_frozen_top { pos.y - viewport_rect.min.y } else { pos.y - tl_y };

                // 二分查找（累积数组严格单调递增）
                let col_idx = col_cumulative_width.partition_point(|&w| w <= rel_x);
                let col = if col_idx > 1 { Some(col_idx as u32 - 1) } else { None };
                let row_idx = row_cumulative_height.partition_point(|&h| h <= rel_y);
                let row = if row_idx > 0 { Some(row_idx as u32 - 1) } else { None };
                match (col, row) {
                    (Some(c), Some(r)) if c > 0 && r > 0 => Some((c, r)),
                    _ => None,
                }
            };

            // 将单元格扩展到所在合并区域的完整边界
            let expand_to_merge = |col: u32, row: u32| -> (u32, u32, u32, u32) {
                if let Some(mr) = sheet.get_merged_range(col, row) {
                    return (mr.start_col, mr.start_row, mr.end_col, mr.end_row);
                }
                (col, row, col, row)
            };

            if response.drag_started() {
                if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                    if let Some((col, row)) = screen_to_cell(pos) {
                        let (asc, asr, aec, aer) = expand_to_merge(col, row);
                        // 锚点记录合并区域的左上角
                        *drag_anchor = Some((asc, asr));
                        *selected_cell = Some((col, row));
                        *selected_range = Some((asc, asr, aec, aer));
                    }
                }
            }

            if response.dragged() {
                if let Some((anchor_col, anchor_row)) = *drag_anchor {
                    if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                        if let Some((cur_col, cur_row)) = screen_to_cell(pos) {
                            // 展开锚点和当前格到各自合并区域的完整边界，取并集
                            let (asc, asr, aec, aer) = expand_to_merge(anchor_col, anchor_row);
                            let (csc, csr, cec, cer) = expand_to_merge(cur_col, cur_row);
                            let sr_col = asc.min(csc);
                            let er_col = aec.max(cec);
                            let sr_row = asr.min(csr);
                            let er_row = aer.max(cer);
                            *selected_range = Some((sr_col, sr_row, er_col, er_row));
                        }
                    }
                }
            }

            if response.drag_stopped() {
                *drag_anchor = None;
            }
        }

        // ========== 第一遍：绘制所有单元格背景 ==========
        for row in visible_rows_start..=visible_rows_end {
            // 跳过冻结区域内的行（由冻结覆盖层单独绘制，避免重影）
            if row <= fr {
                continue;
            }
            // 跳过隐藏行
            if row > 0 && hidden_rows.contains(&row) {
                continue;
            }

            // 使用累积行高计算 y 坐标
            let y = tl_y + border_width + row_cumulative_height[row as usize];

            // 绘制可见列（使用累积宽度数组定位，隐藏列自动跳过）
            for col in visible_cols_start..=visible_cols_end {
                // 跳过冻结区域内的列（由冻结覆盖层单独绘制，避免重影）
                if col <= fc {
                    continue;
                }
                // 跳过隐藏列
                if col > 0 && hidden_columns.contains(&col) {
                    continue;
                }
                let cell_width = if col == 0 {
                    header_width
                } else {
                    get_col_width(col)
                };
                let cell_height = get_row_height(row);
                // 使用累积宽度数组计算 x（隐藏列贡献 0 宽度，自动正确）
                let x = tl_x + border_width + col_cumulative_width[col as usize];

                // 确定单元格背景色
                let bg_color = if row == 0 && col == 0 {
                    egui::Color32::LIGHT_GRAY // 左上角空白
                } else if row == 0 {
                    egui::Color32::LIGHT_GRAY // 列标题行
                } else if col == 0 {
                    egui::Color32::LIGHT_GRAY // 行标题列
                } else {
                    // 从单元格获取背景颜色，否则使用默认白色
                    if let Some(cell) = sheet.get_cell(row, col) {
                        if let Some((r, g, b)) = cell.background_color {
                            egui::Color32::from_rgb(r, g, b)
                        } else {
                            egui::Color32::WHITE
                        }
                    } else {
                        egui::Color32::WHITE // 数据单元格
                    }
                };

                // 检查是否是合并单元格的一部分
                let mut is_merged_top_left = false;
                let mut is_merged_part = false;

                if row > 0 && col > 0 {
                    if let Some(merged_range) = sheet.get_merged_range(col, row) {
                        if merged_range.is_top_left(col, row) {
                            is_merged_top_left = true;
                        } else {
                            is_merged_part = true;
                        }
                    }
                }

                // 如果是合并单元格的非左上角部分，跳过绘制背景（由左上角单元格绘制）
                if is_merged_part {
                    continue;
                }

                // 如果是合并单元格的左上角，绘制合并背景
                if is_merged_top_left {
                    if let Some(merged_range) = sheet.get_merged_range(col, row) {
                        // 使用累积宽度差值计算合并宽度（自动处理隐藏列）
                        let merged_col_width = col_cumulative_width[merged_range.end_col as usize + 1]
                            - col_cumulative_width[merged_range.start_col as usize] - border_width;

                        let mut merged_row_height = 0.0;
                        for r in merged_range.start_row..=merged_range.end_row {
                            merged_row_height += get_row_height(r) + border_width;
                        }
                        merged_row_height -= border_width;

                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                egui::Pos2::new(x, y),
                                egui::vec2(merged_col_width, merged_row_height),
                            ),
                            0.0,
                            bg_color,
                        );
                    }
                } else {
                    // 绘制普通单元格背景
                    painter.rect_filled(
                        egui::Rect::from_min_size(egui::Pos2::new(x, y), egui::vec2(cell_width, cell_height)),
                        0.0,
                        bg_color,
                    );
                }
            }
        }

        // ========== 第二遍：绘制所有单元格内容 ==========
        for row in visible_rows_start..=visible_rows_end {
            // 跳过冻结区域内的行（由冻结覆盖层单独绘制，避免重影）
            if row <= fr {
                continue;
            }
            // 跳过隐藏行
            if row > 0 && hidden_rows.contains(&row) {
                continue;
            }

            // 使用累积行高计算 y 坐标
            let y = tl_y + border_width + row_cumulative_height[row as usize];

            // 绘制可见列（使用累积宽度数组定位，隐藏列自动跳过）
            for col in visible_cols_start..=visible_cols_end {
                // 跳过冻结区域内的列（由冻结覆盖层单独绘制，避免重影）
                if col <= fc {
                    continue;
                }
                // 跳过隐藏列
                if col > 0 && hidden_columns.contains(&col) {
                    continue;
                }
                let cell_width = if col == 0 {
                    header_width
                } else {
                    get_col_width(col)
                };
                let cell_height = get_row_height(row);
                // 使用累积宽度数组计算 x（隐藏列贡献 0 宽度，自动正确）
                let x = tl_x + border_width + col_cumulative_width[col as usize];

                // 绘制列标题（A, B, C...）
                if row == 0 && col > 0 {
                    painter.text(
                        egui::Pos2::new(x + cell_width / 2.0, y + cell_height / 2.0),
                        egui::Align2::CENTER_CENTER,
                        col_to_letter(col),
                        egui::FontId::default(),
                        egui::Color32::BLACK,
                    );
                }
                // 绘制行标题（1, 2, 3...）
                else if col == 0 && row > 0 {
                    painter.text(
                        egui::Pos2::new(x + cell_width / 2.0, y + cell_height / 2.0),
                        egui::Align2::CENTER_CENTER,
                        row.to_string(),
                        egui::FontId::default(),
                        egui::Color32::BLACK,
                    );
                }
                // 绘制数据单元格内容
                else if row > 0 && col > 0 {
                    let mut cell_content = String::new();
                    let mut is_merged_top_left = false;
                    let mut is_merged_part = false;
                    let mut alignment = CellAlignment::default();
                    let mut font_size: Option<f32> = None;
                    let mut font_color: egui::Color32 = egui::Color32::BLACK;

                    // 检查是否是合并单元格的一部分
                    if let Some(merged_range) = sheet.get_merged_range(col, row) {
                        if merged_range.is_top_left(col, row) {
                            is_merged_top_left = true;
                            if let Some(cell) = sheet.get_cell(row, col) {
                                cell_content = cell_display_text(cell).into_owned();
                                alignment = cell.alignment.clone();
                                font_size = cell.font_size.map(|s| s as f32);
                                font_color = cell.font_color.map(|(r, g, b)| egui::Color32::from_rgb(r, g, b)).unwrap_or(egui::Color32::BLACK);
                            }
                        } else {
                            is_merged_part = true;
                        }
                    } else {
                        // 普通单元格
                        if let Some(cell) = sheet.get_cell(row, col) {
                            cell_content = cell_display_text(cell).into_owned();
                            alignment = cell.alignment.clone();
                            font_size = cell.font_size.map(|s| s as f32);
                            font_color = cell.font_color.map(|(r, g, b)| egui::Color32::from_rgb(r, g, b)).unwrap_or(egui::Color32::BLACK);
                        }
                    }

                    // 如果是合并单元格的非左上角部分，跳过绘制
                    if is_merged_part {
                        continue;
                    }

                    // 绘制合并单元格内容
                    if is_merged_top_left {
                        if let Some(merged_range) = sheet.get_merged_range(col, row) {
                            // 使用累积宽度差值计算合并宽度（自动处理隐藏列）
                            let merged_col_width = col_cumulative_width[merged_range.end_col as usize + 1]
                                - col_cumulative_width[merged_range.start_col as usize] - border_width;

                            let mut merged_row_height = 0.0;
                            for r in merged_range.start_row..=merged_range.end_row {
                                merged_row_height += get_row_height(r) + border_width;
                            }
                            merged_row_height -= border_width;

                            // 根据对齐方式绘制内容
                            let egui_align = alignment_to_egui(&alignment);
                            let text_pos = match egui_align {
                                egui::Align2::LEFT_TOP       => egui::Pos2::new(x + 4.0, y + 4.0),
                                egui::Align2::LEFT_CENTER    => egui::Pos2::new(x + 4.0, y + merged_row_height / 2.0),
                                egui::Align2::LEFT_BOTTOM    => egui::Pos2::new(x + 4.0, y + merged_row_height - 4.0),
                                egui::Align2::CENTER_TOP     => egui::Pos2::new(x + merged_col_width / 2.0, y + 4.0),
                                egui::Align2::CENTER_CENTER  => egui::Pos2::new(x + merged_col_width / 2.0, y + merged_row_height / 2.0),
                                egui::Align2::CENTER_BOTTOM  => egui::Pos2::new(x + merged_col_width / 2.0, y + merged_row_height - 4.0),
                                egui::Align2::RIGHT_TOP      => egui::Pos2::new(x + merged_col_width - 4.0, y + 4.0),
                                egui::Align2::RIGHT_CENTER   => egui::Pos2::new(x + merged_col_width - 4.0, y + merged_row_height / 2.0),
                                egui::Align2::RIGHT_BOTTOM   => egui::Pos2::new(x + merged_col_width - 4.0, y + merged_row_height - 4.0),
                            };

                            let font_id = font_size.map(|size| egui::FontId::proportional(size)).unwrap_or(egui::FontId::default());
                            painter.text(
                                text_pos,
                                egui_align,
                                &cell_content,
                                font_id,
                                font_color,
                            );
                        }
                    }
                    // 绘制普通单元格内容
                    else {
                        if !cell_content.is_empty() {
                            let egui_align = alignment_to_egui(&alignment);
                            let text_pos = match egui_align {
                                egui::Align2::LEFT_TOP       => egui::Pos2::new(x + 4.0, y + 4.0),
                                egui::Align2::LEFT_CENTER    => egui::Pos2::new(x + 4.0, y + cell_height / 2.0),
                                egui::Align2::LEFT_BOTTOM    => egui::Pos2::new(x + 4.0, y + cell_height - 4.0),
                                egui::Align2::CENTER_TOP     => egui::Pos2::new(x + cell_width / 2.0, y + 4.0),
                                egui::Align2::CENTER_CENTER  => egui::Pos2::new(x + cell_width / 2.0, y + cell_height / 2.0),
                                egui::Align2::CENTER_BOTTOM  => egui::Pos2::new(x + cell_width / 2.0, y + cell_height - 4.0),
                                egui::Align2::RIGHT_TOP      => egui::Pos2::new(x + cell_width - 4.0, y + 4.0),
                                egui::Align2::RIGHT_CENTER   => egui::Pos2::new(x + cell_width - 4.0, y + cell_height / 2.0),
                                egui::Align2::RIGHT_BOTTOM   => egui::Pos2::new(x + cell_width - 4.0, y + cell_height - 4.0),
                            };

                            let font_id = font_size.map(|size| egui::FontId::proportional(size)).unwrap_or(egui::FontId::default());
                            painter.text(
                                text_pos,
                                egui_align,
                                &cell_content,
                                font_id,
                                font_color,
                            );
                        }
                    }
                }
            }
        }

        // ========== 批注指示器：有批注的单元格右上角画红三角（主网格非冻结区） ==========
        for row in visible_rows_start..=visible_rows_end {
            if row <= fr { continue; }
            if row > 0 && hidden_rows.contains(&row) { continue; }
            for col in visible_cols_start..=visible_cols_end {
                if col <= fc { continue; }
                if col > 0 && hidden_columns.contains(&col) { continue; }
                // 合并非左上角单元格跳过（指示器只在合并左上角画）
                if let Some(mr) = sheet.get_merged_range(col, row) {
                    if !mr.is_top_left(col, row) { continue; }
                }
                // 批注挂在合并左上角；reader 解析时已为有批注的格创建 CellData
                let has_comment = sheet.get_cell(row, col).map_or(false, |c| c.comment.is_some());
                if !has_comment { continue; }
                let x = tl_x + border_width + col_cumulative_width[col as usize];
                let y = tl_y + border_width + row_cumulative_height[row as usize];
                let w = match sheet.get_merged_range(col, row) {
                    Some(mr) => col_cumulative_width[mr.end_col as usize + 1]
                        - col_cumulative_width[mr.start_col as usize]
                        - border_width,
                    None => get_col_width(col),
                };
                draw_comment_indicator(&painter, x, y, w);
            }
        }

        // （编辑输入框已移至冻结覆盖层之后，防止覆盖层遮挡输入框）
        
        // ========== 冻结窗格：固定列标题、行标题和冻结数据区域 ==========
        let viewport_rect = ui.clip_rect();

        // 先用背景色填充冻结覆盖区域，遮住主网格在滚动时透出的内容（消除重影）
        // 顶部冻结区域（行标题行 + 冻结数据行，全宽）
        if frozen_top_height > 0.0 {
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::Pos2::new(viewport_rect.min.x, viewport_rect.min.y),
                    egui::Pos2::new(viewport_rect.max.x, viewport_rect.min.y + frozen_top_height),
                ),
                0.0,
                egui::Color32::WHITE,
            );
        }
        // 左侧冻结区域（行号列 + 冻结数据列，全高）
        if frozen_left_width > 0.0 {
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::Pos2::new(viewport_rect.min.x, viewport_rect.min.y),
                    egui::Pos2::new(viewport_rect.min.x + frozen_left_width, viewport_rect.max.y),
                ),
                0.0,
                egui::Color32::WHITE,
            );
        }

        // 辅助函数：在指定位置绘制数据单元格（背景+内容）
        let draw_frozen_cell = |painter: &egui::Painter, col: u32, row: u32, x: f32, y: f32| {
            let cell_width = get_col_width(col);
            let cell_height = get_row_height(row);

            // 检查合并单元格（只调用一次 get_merged_range）
            let merged_range = sheet.get_merged_range(col, row);
            let is_merged_top_left = merged_range.map_or(false, |mr| mr.is_top_left(col, row));
            let is_merged_part = merged_range.is_some() && !is_merged_top_left;

            if is_merged_part {
                return;
            }

            // 获取单元格数据（只查一次，避免后续重复 get_cell）
            let cell_data = sheet.get_cell(row, col);

            // 获取背景色
            let bg_color = cell_data.and_then(|c| c.background_color)
                .map(|(r, g, b)| egui::Color32::from_rgb(r, g, b))
                .unwrap_or(egui::Color32::WHITE);

            // 绘制背景
            if is_merged_top_left {
                if let Some(merged_range) = merged_range {
                    // 使用累积数组差值替代循环累加
                    let mw = col_cumulative_width[merged_range.end_col as usize + 1]
                        - col_cumulative_width[merged_range.start_col as usize] - border_width;
                    let mh = row_cumulative_height[merged_range.end_row as usize + 1]
                        - row_cumulative_height[merged_range.start_row as usize] - border_width;

                    painter.rect_filled(
                        egui::Rect::from_min_size(egui::Pos2::new(x, y), egui::vec2(mw, mh)),
                        0.0,
                        bg_color,
                    );

                    // 绘制合并单元格边框
                    painter.rect_stroke(
                        egui::Rect::from_min_size(egui::Pos2::new(x, y), egui::vec2(mw, mh)),
                        0.0,
                        egui::Stroke::new(border_width, egui::Color32::GRAY),
                        egui::StrokeKind::Outside,
                    );

                    // 绘制内容
                    if let Some(cell) = cell_data {
                        let display = cell_display_text(cell);
                        if !display.is_empty() {
                            let egui_align = alignment_to_egui(&cell.alignment);
                            let font_id = cell.font_size.map(|s| egui::FontId::proportional(s as f32)).unwrap_or(egui::FontId::default());
                            let font_color = cell.font_color.map(|(r, g, b)| egui::Color32::from_rgb(r, g, b)).unwrap_or(egui::Color32::BLACK);
                            let text_pos = match egui_align {
                                egui::Align2::LEFT_TOP       => egui::Pos2::new(x + 4.0, y + 4.0),
                                egui::Align2::LEFT_CENTER    => egui::Pos2::new(x + 4.0, y + mh / 2.0),
                                egui::Align2::LEFT_BOTTOM    => egui::Pos2::new(x + 4.0, y + mh - 4.0),
                                egui::Align2::CENTER_TOP     => egui::Pos2::new(x + mw / 2.0, y + 4.0),
                                egui::Align2::CENTER_CENTER  => egui::Pos2::new(x + mw / 2.0, y + mh / 2.0),
                                egui::Align2::CENTER_BOTTOM  => egui::Pos2::new(x + mw / 2.0, y + mh - 4.0),
                                egui::Align2::RIGHT_TOP      => egui::Pos2::new(x + mw - 4.0, y + 4.0),
                                egui::Align2::RIGHT_CENTER   => egui::Pos2::new(x + mw - 4.0, y + mh / 2.0),
                                egui::Align2::RIGHT_BOTTOM   => egui::Pos2::new(x + mw - 4.0, y + mh - 4.0),
                            };
                            painter.text(text_pos, egui_align, &display, font_id, font_color);
                        }
                    }
                }
            } else {
                painter.rect_filled(
                    egui::Rect::from_min_size(egui::Pos2::new(x, y), egui::vec2(cell_width, cell_height)),
                    0.0,
                    bg_color,
                );

                // 绘制单元格边框
                painter.rect_stroke(
                    egui::Rect::from_min_size(egui::Pos2::new(x, y), egui::vec2(cell_width, cell_height)),
                    0.0,
                    egui::Stroke::new(border_width, egui::Color32::GRAY),
                    egui::StrokeKind::Outside,
                );

                // 绘制内容（复用 cell_data 避免重复 get_cell 查询）
                if let Some(cell) = cell_data {
                    let display = cell_display_text(cell);
                    if !display.is_empty() {
                        let egui_align = alignment_to_egui(&cell.alignment);
                        let font_id = cell.font_size.map(|s| egui::FontId::proportional(s as f32)).unwrap_or(egui::FontId::default());
                        let font_color = cell.font_color.map(|(r, g, b)| egui::Color32::from_rgb(r, g, b)).unwrap_or(egui::Color32::BLACK);
                        let text_pos = match egui_align {
                            egui::Align2::LEFT_TOP       => egui::Pos2::new(x + 4.0, y + 4.0),
                            egui::Align2::LEFT_CENTER    => egui::Pos2::new(x + 4.0, y + cell_height / 2.0),
                            egui::Align2::LEFT_BOTTOM    => egui::Pos2::new(x + 4.0, y + cell_height - 4.0),
                            egui::Align2::CENTER_TOP     => egui::Pos2::new(x + cell_width / 2.0, y + 4.0),
                            egui::Align2::CENTER_CENTER  => egui::Pos2::new(x + cell_width / 2.0, y + cell_height / 2.0),
                            egui::Align2::CENTER_BOTTOM  => egui::Pos2::new(x + cell_width / 2.0, y + cell_height - 4.0),
                            egui::Align2::RIGHT_TOP      => egui::Pos2::new(x + cell_width - 4.0, y + 4.0),
                            egui::Align2::RIGHT_CENTER   => egui::Pos2::new(x + cell_width - 4.0, y + cell_height / 2.0),
                            egui::Align2::RIGHT_BOTTOM   => egui::Pos2::new(x + cell_width - 4.0, y + cell_height - 4.0),
                        };
                        painter.text(text_pos, egui_align, &display, font_id, font_color);
                    }
                }
            }

            // 批注指示器：有批注的冻结单元格右上角画红三角
            if let Some(cell) = cell_data {
                if cell.comment.is_some() {
                    let w = if is_merged_top_left {
                        if let Some(mr) = merged_range {
                            col_cumulative_width[mr.end_col as usize + 1]
                                - col_cumulative_width[mr.start_col as usize]
                                - border_width
                        } else {
                            cell_width
                        }
                    } else {
                        cell_width
                    };
                    draw_comment_indicator(painter, x, y, w);
                }
            }
        };

        // ===== 绘制顺序说明 =====
        // 关键：冻结顶部数据行的合并单元格（如 N1:O1）可能向左溢出到冻结左侧区域
        // 因此必须先画顶部区域，再白色重填左侧区域覆盖溢出，最后画左侧区域内容

        // === 第1步：绘制顶部冻结区域（列标题行 + 冻结数据行）===

        // 绘制冻结列范围内的列标题（row 0，cols 1..=fc）
        for col in 1..=fc {
            if hidden_columns.contains(&col) { continue; }
            let fixed_x = viewport_rect.min.x + col_cumulative_width[col as usize];
            let col_width = get_col_width(col);
            let col_height = get_row_height(0);
            painter.rect_filled(
                egui::Rect::from_min_size(egui::Pos2::new(fixed_x, viewport_rect.min.y), egui::vec2(col_width, col_height)),
                0.0,
                egui::Color32::LIGHT_GRAY,
            );
            painter.text(
                egui::Pos2::new(fixed_x + col_width / 2.0, viewport_rect.min.y + col_height / 2.0),
                egui::Align2::CENTER_CENTER,
                col_to_letter(col),
                egui::FontId::default(),
                egui::Color32::BLACK,
            );
        }

        // 绘制非冻结列的列标题（row 0，cols > fc）- scroll-dependent x
        for col in (fc + 1).max(visible_cols_start)..=visible_cols_end.min(sheet.max_col) {
            if hidden_columns.contains(&col) { continue; }
            let col_x = tl_x + border_width + col_cumulative_width[col as usize];
            let col_width = get_col_width(col);
            let col_height = get_row_height(0);

            let col_rect = egui::Rect::from_min_size(
                egui::Pos2::new(col_x, viewport_rect.min.y),
                egui::vec2(col_width, col_height),
            );

            if col_rect.max.x > viewport_rect.min.x + frozen_left_width && col_rect.min.x < viewport_rect.max.x {
                painter.rect_filled(col_rect, 0.0, egui::Color32::LIGHT_GRAY);
                painter.rect_stroke(col_rect, 0.0, egui::Stroke::new(border_width, egui::Color32::GRAY), egui::StrokeKind::Outside);
                painter.text(
                    egui::Pos2::new(col_rect.center().x, col_rect.center().y),
                    egui::Align2::CENTER_CENTER,
                    col_to_letter(col),
                    egui::FontId::default(),
                    egui::Color32::BLACK,
                );
            }
        }

        // 绘制冻结顶部数据行（rows 1..=fr，所有数据列）
        // 注意：合并单元格可能向左溢出到冻结左侧区域
        for row in 1..=fr {
            if hidden_rows.contains(&row) { continue; }
            let fixed_y = viewport_rect.min.y + row_cumulative_height[row as usize];
            // 冻结列部分（cols 1..=fc）
            for col in 1..=fc {
                if hidden_columns.contains(&col) { continue; }
                let fixed_x = viewport_rect.min.x + col_cumulative_width[col as usize];
                draw_frozen_cell(&painter, col, row, fixed_x, fixed_y);
            }
            // 非冻结列部分（cols > fc）- scroll-dependent x，限可见范围
            for col in (fc + 1).max(visible_cols_start)..=visible_cols_end.min(sheet.max_col) {
                if hidden_columns.contains(&col) { continue; }
                let col_x = tl_x + border_width + col_cumulative_width[col as usize];
                if col_x >= viewport_rect.max.x { break; }
                draw_frozen_cell(&painter, col, row, col_x, fixed_y);
            }
        }

        // === 第2步：白色重填左侧冻结区域，覆盖顶部数据行合并单元格的溢出 ===
        if frozen_left_width > 0.0 {
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::Pos2::new(viewport_rect.min.x, viewport_rect.min.y),
                    egui::Pos2::new(viewport_rect.min.x + frozen_left_width, viewport_rect.max.y),
                ),
                0.0,
                egui::Color32::WHITE,
            );
        }

        // === 第3步：绘制左侧冻结区域内容 ===

        // 左上角固定区域背景
        let frozen_corner_rect = egui::Rect::from_min_max(
            egui::Pos2::new(viewport_rect.min.x, viewport_rect.min.y),
            egui::Pos2::new(viewport_rect.min.x + frozen_left_width, viewport_rect.min.y + frozen_top_height),
        );
        painter.rect_filled(frozen_corner_rect, 0.0, egui::Color32::LIGHT_GRAY);

        // 左上角冻结列范围内的列标题（row 0，cols 1..=fc）
        for col in 1..=fc {
            if hidden_columns.contains(&col) { continue; }
            let fixed_x = viewport_rect.min.x + col_cumulative_width[col as usize];
            let col_width = get_col_width(col);
            let col_height = get_row_height(0);
            painter.rect_filled(
                egui::Rect::from_min_size(egui::Pos2::new(fixed_x, viewport_rect.min.y), egui::vec2(col_width, col_height)),
                0.0,
                egui::Color32::LIGHT_GRAY,
            );
            painter.text(
                egui::Pos2::new(fixed_x + col_width / 2.0, viewport_rect.min.y + col_height / 2.0),
                egui::Align2::CENTER_CENTER,
                col_to_letter(col),
                egui::FontId::default(),
                egui::Color32::BLACK,
            );
        }

        // 左上角冻结行范围内的行号（rows 1..=fr）
        for row in 1..=fr {
            if hidden_rows.contains(&row) { continue; }
            let fixed_y = viewport_rect.min.y + row_cumulative_height[row as usize];
            let row_height = get_row_height(row);
            painter.rect_filled(
                egui::Rect::from_min_size(egui::Pos2::new(viewport_rect.min.x, fixed_y), egui::vec2(header_width, row_height)),
                0.0,
                egui::Color32::LIGHT_GRAY,
            );
            painter.text(
                egui::Pos2::new(viewport_rect.min.x + header_width / 2.0, fixed_y + row_height / 2.0),
                egui::Align2::CENTER_CENTER,
                row.to_string(),
                egui::FontId::default(),
                egui::Color32::BLACK,
            );
        }

        // 左上角冻结数据单元格（冻结行 ∩ 冻结列）
        for row in 1..=fr {
            if hidden_rows.contains(&row) { continue; }
            let fixed_y = viewport_rect.min.y + row_cumulative_height[row as usize];
            for col in 1..=fc {
                if hidden_columns.contains(&col) { continue; }
                let fixed_x = viewport_rect.min.x + col_cumulative_width[col as usize];
                draw_frozen_cell(&painter, col, row, fixed_x, fixed_y);
            }
        }

        // 非冻结行的行号（col 0，rows > fr）
        for row in (fr + 1).max(visible_rows_start)..=visible_rows_end.min(sheet.max_row) {
            if hidden_rows.contains(&row) { continue; }
            let row_y = tl_y + border_width + row_cumulative_height[row as usize];
            let row_width = header_width;
            let row_height = get_row_height(row);

            let row_rect = egui::Rect::from_min_size(
                egui::Pos2::new(viewport_rect.min.x, row_y),
                egui::vec2(row_width, row_height),
            );

            if row_rect.max.y > viewport_rect.min.y + frozen_top_height && row_rect.min.y < viewport_rect.max.y {
                painter.rect_filled(row_rect, 0.0, egui::Color32::LIGHT_GRAY);
                painter.rect_stroke(row_rect, 0.0, egui::Stroke::new(border_width, egui::Color32::GRAY), egui::StrokeKind::Outside);
                painter.text(
                    egui::Pos2::new(row_rect.center().x, row_rect.center().y),
                    egui::Align2::CENTER_CENTER,
                    row.to_string(),
                    egui::FontId::default(),
                    egui::Color32::BLACK,
                );
            }
        }

        // 冻结左侧数据列（rows > fr，cols 1..=fc）
        // 注意：这些单元格按滚动 y 绘制，可能向上溢出到冻结顶部区域（如 A15 遮盖 A14）
        for row in (fr + 1).max(visible_rows_start)..=visible_rows_end.min(sheet.max_row) {
            if hidden_rows.contains(&row) { continue; }
            let row_y = tl_y + border_width + row_cumulative_height[row as usize];
            if row_y + get_row_height(row) <= viewport_rect.min.y + frozen_top_height { continue; }
            if row_y >= viewport_rect.max.y { break; }
            for col in 1..=fc {
                if hidden_columns.contains(&col) { continue; }
                let fixed_x = viewport_rect.min.x + col_cumulative_width[col as usize];
                draw_frozen_cell(&painter, col, row, fixed_x, row_y);
            }
        }

        // === 第4步：白色重填左上角区域，覆盖左侧数据列向上溢出的部分 ===
        // 然后重绘左上角区域内容（角落数据量小，重绘代价低）
        if frozen_left_width > 0.0 && frozen_top_height > 0.0 {
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::Pos2::new(viewport_rect.min.x, viewport_rect.min.y),
                    egui::Pos2::new(viewport_rect.min.x + frozen_left_width, viewport_rect.min.y + frozen_top_height),
                ),
                0.0,
                egui::Color32::WHITE,
            );
            // 重绘左上角背景
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::Pos2::new(viewport_rect.min.x, viewport_rect.min.y),
                    egui::Pos2::new(viewport_rect.min.x + frozen_left_width, viewport_rect.min.y + frozen_top_height),
                ),
                0.0,
                egui::Color32::LIGHT_GRAY,
            );
            // 重绘冻结列标题
            for col in 1..=fc {
                if hidden_columns.contains(&col) { continue; }
                let fixed_x = viewport_rect.min.x + col_cumulative_width[col as usize];
                let col_width = get_col_width(col);
                let col_height = get_row_height(0);
                painter.rect_filled(
                    egui::Rect::from_min_size(egui::Pos2::new(fixed_x, viewport_rect.min.y), egui::vec2(col_width, col_height)),
                    0.0,
                    egui::Color32::LIGHT_GRAY,
                );
                painter.text(
                    egui::Pos2::new(fixed_x + col_width / 2.0, viewport_rect.min.y + col_height / 2.0),
                    egui::Align2::CENTER_CENTER,
                    col_to_letter(col),
                    egui::FontId::default(),
                    egui::Color32::BLACK,
                );
            }
            // 重绘冻结行号
            for row in 1..=fr {
                if hidden_rows.contains(&row) { continue; }
                let fixed_y = viewport_rect.min.y + row_cumulative_height[row as usize];
                let row_height = get_row_height(row);
                painter.rect_filled(
                    egui::Rect::from_min_size(egui::Pos2::new(viewport_rect.min.x, fixed_y), egui::vec2(header_width, row_height)),
                    0.0,
                    egui::Color32::LIGHT_GRAY,
                );
                painter.text(
                    egui::Pos2::new(viewport_rect.min.x + header_width / 2.0, fixed_y + row_height / 2.0),
                    egui::Align2::CENTER_CENTER,
                    row.to_string(),
                    egui::FontId::default(),
                    egui::Color32::BLACK,
                );
            }
            // 重绘角落冻结数据单元格
            for row in 1..=fr {
                if hidden_rows.contains(&row) { continue; }
                let fixed_y = viewport_rect.min.y + row_cumulative_height[row as usize];
                for col in 1..=fc {
                    if hidden_columns.contains(&col) { continue; }
                    let fixed_x = viewport_rect.min.x + col_cumulative_width[col as usize];
                    draw_frozen_cell(&painter, col, row, fixed_x, fixed_y);
                }
            }
        }

        // 绘制冻结窗格分隔线
        if frozen_top_height > 0.0 {
            let line_y = viewport_rect.min.y + frozen_top_height;
            painter.line_segment(
                [egui::Pos2::new(viewport_rect.min.x, line_y), egui::Pos2::new(viewport_rect.max.x, line_y)],
                egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 100, 100)),
            );
        }
        if frozen_left_width > 0.0 {
            let line_x = viewport_rect.min.x + frozen_left_width;
            painter.line_segment(
                [egui::Pos2::new(line_x, viewport_rect.min.y), egui::Pos2::new(line_x, viewport_rect.max.y)],
                egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 100, 100)),
            );
        }

        // ========== 选中单元格高亮边框（最后绘制，确保在所有单元格之上） ==========
        if let Some((sel_col, sel_row)) = *selected_cell {
            // 确定选中单元格的实际矩形（处理合并单元格）
            let (start_col, start_row, end_col, end_row) = if let Some(merged_range) = sheet.get_merged_range(sel_col, sel_row) {
                (merged_range.start_col, merged_range.start_row, merged_range.end_col, merged_range.end_row)
            } else {
                (sel_col, sel_row, sel_col, sel_row)
            };

            // 计算选中单元格位置：冻结区域用固定视口坐标，非冻结区域用表格坐标
            // 使用累积数组索引替代循环累加（索引已通过受信任来源保证在下界内；上界通过 min 安全裁剪）
            let sel_x = if start_col <= fc {
                let idx = (start_col as usize).min(col_cumulative_width.len() - 1);
                viewport_rect.min.x + col_cumulative_width[idx]
            } else {
                let idx = (start_col as usize).min(col_cumulative_width.len() - 1);
                tl_x + border_width + col_cumulative_width[idx]
            };
            let sel_y = if start_row <= fr {
                let idx = (start_row as usize).min(row_cumulative_height.len() - 1);
                viewport_rect.min.y + row_cumulative_height[idx]
            } else {
                let idx = (start_row as usize).min(row_cumulative_height.len() - 1);
                tl_y + border_width + row_cumulative_height[idx]
            };

            // 计算选中区域宽高：使用累积数组差值替代循环累加
            let end_col_idx = ((end_col as usize).saturating_add(1)).min(col_cumulative_width.len() - 1);
            let sel_w = col_cumulative_width[end_col_idx]
                - col_cumulative_width[(start_col as usize).min(col_cumulative_width.len() - 1)] - border_width;
            let end_row_idx = ((end_row as usize).saturating_add(1)).min(row_cumulative_height.len() - 1);
            let sel_h = row_cumulative_height[end_row_idx]
                - row_cumulative_height[(start_row as usize).min(row_cumulative_height.len() - 1)] - border_width;

            // 绘制2px蓝色选中边框
            painter.rect_stroke(
                egui::Rect::from_min_size(egui::Pos2::new(sel_x, sel_y), egui::vec2(sel_w, sel_h)),
                0.0,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(0, 112, 192)),
                egui::StrokeKind::Outside,
            );

            // 保存选中单元格屏幕矩形，供数据有效性弹窗定位
            selected_cell_rect = Some(egui::Rect::from_min_size(
                egui::Pos2::new(sel_x, sel_y),
                egui::vec2(sel_w, sel_h),
            ));
        }

        // ========== 绘制选中范围（蓝色半透明背景） ==========
        if let Some((sr_col, sr_row, er_col, er_row)) = selected_range {
            // 确保范围有效且在可见区域内（冻结区域始终可见）
            let r_start_col = (*sr_col).max(0u32);
            let r_end_col = (*er_col).min(visible_cols_end.max(fc));
            let r_start_row = (*sr_row).max(0u32);
            let r_end_row = (*er_row).min(visible_rows_end.max(fr));
            if r_start_col <= r_end_col && r_start_row <= r_end_row {
                // 计算起始位置：冻结区域用固定视口坐标，非冻结区域用表格坐标
                let rx = if r_start_col <= fc {
                    let idx = (r_start_col as usize).min(col_cumulative_width.len() - 1);
                    viewport_rect.min.x + col_cumulative_width[idx]
                } else {
                    let idx = (r_start_col as usize).min(col_cumulative_width.len() - 1);
                    tl_x + border_width + col_cumulative_width[idx]
                };
                let ry = if r_start_row <= fr {
                    let idx = (r_start_row as usize).min(row_cumulative_height.len() - 1);
                    viewport_rect.min.y + row_cumulative_height[idx]
                } else {
                    let idx = (r_start_row as usize).min(row_cumulative_height.len() - 1);
                    tl_y + border_width + row_cumulative_height[idx]
                };
                let end_col_idx = ((r_end_col as usize).saturating_add(1)).min(col_cumulative_width.len() - 1);
                let rw = col_cumulative_width[end_col_idx]
                    - col_cumulative_width[(r_start_col as usize).min(col_cumulative_width.len() - 1)] - border_width;
                let end_row_idx = ((r_end_row as usize).saturating_add(1)).min(row_cumulative_height.len() - 1);
                let rh = row_cumulative_height[end_row_idx]
                    - row_cumulative_height[(r_start_row as usize).min(row_cumulative_height.len() - 1)] - border_width;
                // 绘制半透明蓝色背景
                painter.rect_filled(
                    egui::Rect::from_min_size(egui::Pos2::new(rx, ry), egui::vec2(rw, rh)),
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(0, 112, 192, 40),
                );
                // 绘制范围边框
                painter.rect_stroke(
                    egui::Rect::from_min_size(egui::Pos2::new(rx, ry), egui::vec2(rw, rh)),
                    0.0,
                    egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(0, 112, 192, 120)),
                    egui::StrokeKind::Outside,
                );
            }
        }

        // ========== 批注悬停气泡：指针悬停在有批注的单元格上时显示作者+正文 ==========
        if !validation_error_active && editing_cell.is_none() && response.hovered() && !response.dragged() {
            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                // 屏幕坐标 → 单元格（冻结区感知，复用与点击相同的坐标转换逻辑）
                let in_frozen_left = pos.x < viewport_rect.min.x + frozen_left_width;
                let in_frozen_top = pos.y < viewport_rect.min.y + frozen_top_height;
                let rel_x = if in_frozen_left { pos.x - viewport_rect.min.x } else { pos.x - tl_x };
                let rel_y = if in_frozen_top { pos.y - viewport_rect.min.y } else { pos.y - tl_y };
                let col_idx = col_cumulative_width.partition_point(|&w| w <= rel_x);
                let row_idx = row_cumulative_height.partition_point(|&h| h <= rel_y);
                if col_idx > 1 && row_idx > 0 {
                    let c = col_idx as u32 - 1;
                    let r = row_idx as u32 - 1;
                    if c > 0 && r > 0 {
                        // 合并单元格：取左上角（批注挂在左上角）
                        let (tr, tc) = match sheet.get_merged_range(c, r) {
                            Some(mr) => (mr.start_row, mr.start_col),
                            None => (r, c),
                        };
                        if let Some(cell) = sheet.get_cell(tr, tc) {
                            if let Some(comment) = &cell.comment {
                                // 正文（黑色，自动换行）；空批注占位
                                let body_text = if comment.text.is_empty() {
                                    "（空批注）".to_string()
                                } else {
                                    comment.text.clone()
                                };
                                let body_galley = painter.layout_job(
                                    egui::text::LayoutJob::simple(
                                        body_text,
                                        egui::FontId::proportional(13.0),
                                        egui::Color32::BLACK,
                                        300.0,
                                    ),
                                );
                                // 作者头：仅当正文未以「作者:」开头时才单独显示一行。
                                // Excel 会把作者名作为正文首行嵌入（如 "s:\n..."），重复显示会产生多余的作者行。
                                let author_prefix = format!("{}:", comment.author);
                                let author_galley = if !comment.author.is_empty()
                                    && !comment.text.starts_with(&author_prefix)
                                {
                                    Some(painter.layout_job(
                                        egui::text::LayoutJob::simple(
                                            comment.author.clone(),
                                            egui::FontId::proportional(11.0),
                                            egui::Color32::from_rgb(120, 120, 120),
                                            300.0,
                                        ),
                                    ))
                                } else {
                                    None
                                };
                                let pad = 8.0;
                                let gap = 3.0;
                                let author_h = author_galley.as_ref().map_or(0.0, |g| g.size().y);
                                let inner_w = author_galley
                                    .as_ref()
                                    .map_or(0.0, |g| g.size().x)
                                    .max(body_galley.size().x);
                                let inner_h = author_h
                                    + if author_galley.is_some() {
                                        gap + body_galley.size().y
                                    } else {
                                        body_galley.size().y
                                    };
                                let box_w = inner_w + pad * 2.0;
                                let box_h = inner_h + pad * 2.0;
                                // 定位：指针右下方；越界则向左/上翻转，并夹紧到视口
                                let clip = ui.clip_rect();
                                let mut bx = pos.x + 14.0;
                                let mut by = pos.y + 14.0;
                                if bx + box_w > clip.max.x { bx = pos.x - 14.0 - box_w; }
                                if by + box_h > clip.max.y { by = pos.y - 14.0 - box_h; }
                                let bx = bx.max(clip.min.x);
                                let by = by.max(clip.min.y);
                                let rect = egui::Rect::from_min_size(egui::Pos2::new(bx, by), egui::vec2(box_w, box_h));
                                // 淡黄背景（Excel 批注配色）+ 边框
                                painter.rect_filled(rect, 3.0, egui::Color32::from_rgb(255, 255, 224));
                                painter.rect_stroke(rect, 3.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(190, 190, 120)), egui::StrokeKind::Outside);
                                let mut text_y = by + pad;
                                if let Some(ag) = author_galley {
                                    painter.galley(egui::Pos2::new(bx + pad, text_y), ag, egui::Color32::from_rgb(120, 120, 120));
                                    text_y += author_h + gap;
                                }
                                painter.galley(egui::Pos2::new(bx + pad, text_y), body_galley, egui::Color32::BLACK);
                            }
                        }
                    }
                }
            }
        }

        // ========== 编辑模式：显示输入框（在冻结覆盖层之后绘制，防止覆盖层遮挡） ==========
        // 复制编辑单元格坐标，避免在闭包中借用冲突
        let editing_coords = editing_cell.map(|(c, r)| (c, r));
        if let Some((edit_col, edit_row)) = editing_coords {
            // 检查是否在可见范围内（冻结区域单元格始终可见）
            let col_visible = edit_col <= fc
                || (edit_col >= visible_cols_start && edit_col <= visible_cols_end);
            let row_visible = edit_row <= fr
                || (edit_row >= visible_rows_start && edit_row <= visible_rows_end);
            if col_visible && row_visible {

                // 计算编辑单元格的位置
                // 冻结区域使用固定视口坐标，非冻结区域使用表格内容坐标
                // 使用累积数组索引替代循环累加
                let x = if edit_col <= fc {
                    viewport_rect.min.x + col_cumulative_width[edit_col as usize]
                } else {
                    tl_x + border_width + col_cumulative_width[edit_col as usize]
                };
                let y = if edit_row <= fr {
                    viewport_rect.min.y + row_cumulative_height[edit_row as usize]
                } else {
                    tl_y + border_width + row_cumulative_height[edit_row as usize]
                };

                // 检查是否是合并单元格，如果是则计算合并区域的尺寸
                // 使用累积数组差值替代循环累加
                let (cell_width, cell_height) = if let Some(merged_range) = sheet.get_merged_range(edit_col, edit_row) {
                    let mw = col_cumulative_width[merged_range.end_col as usize + 1]
                        - col_cumulative_width[merged_range.start_col as usize] - border_width;
                    let mh = row_cumulative_height[merged_range.end_row as usize + 1]
                        - row_cumulative_height[merged_range.start_row as usize] - border_width;
                    (mw, mh)
                } else {
                    (get_col_width(edit_col), get_row_height(edit_row))
                };

                // 限制输入框尺寸，避免超出单元格
                let input_width = (cell_width - 4.0).max(10.0);
                let input_height = (cell_height - 6.0).max(16.0);

                // 保存编辑状态用于闭包
                let mut save_cell = false;
                let mut clear_edit = false;

                // 创建输入框响应区域
                let edit_rect = egui::Rect::from_min_size(
                    egui::Pos2::new(x + 2.0, y + 2.0),
                    egui::vec2(input_width, input_height)
                );
                let builder = egui::UiBuilder::new().max_rect(edit_rect);
                ui.scope_builder(builder, |ui| {
                        let text_edit = egui::TextEdit::singleline(edit_value)
                            .font(egui::FontId::default())
                            .desired_width(input_width)
                            .min_size(egui::vec2(input_width, input_height));

                        let response = ui.add(text_edit);

                        // 自动聚焦输入框
                        if !response.has_focus() {
                            response.request_focus();
                        }

                        // Ctrl+A 全选
                        if response.has_focus() && ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::A)) {
                            let text_len = edit_value.chars().count();
                            if let Some(mut ts) = egui::TextEdit::load_state(ui.ctx(), response.id) {
                                ts.cursor = egui::text::CCursorRange::two(
                                    egui::text::CCursor::default(),
                                    egui::text::CCursor::new(text_len),
                                ).into();
                                egui::TextEdit::store_state(ui.ctx(), response.id, ts);
                            }
                            ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::A));
                        }

                        // 处理键盘事件
                        if response.has_focus() {
                            // 如果刚进入编辑模式，忽略Enter键（避免同一帧中进入又退出）
                            if !*just_entered_edit_mode && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                save_cell = true;
                                clear_edit = true;
                            }
                            // 处理 ESC 键，取消编辑
                            else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                clear_edit = true;
                            }
                            // 点击其他地方时退出编辑
                            else if ui.input(|i| i.pointer.any_click() && !response.rect.contains(i.pointer.hover_pos().unwrap_or_default())) {
                                save_cell = true;
                                clear_edit = true;
                            }
                        }
                    }
                );

                // 在闭包外部处理状态更新
                if save_cell {
                    // 保存编辑值并触发公式重算
                    let is_formula = edit_value.starts_with('=');
                    // 非公式值做数据有效性校验
                    if !is_formula {
                        if let Some(sheet) = excel_data.sheets.get(current_sheet) {
                            if let Some((title, msg)) = sheet.validate_cell(edit_col, edit_row, edit_value) {
                                *validation_error = Some((title, msg));
                                save_cell = false;
                                clear_edit = false;
                            }
                        }
                    }
                }
                if save_cell {
                    let is_formula = edit_value.starts_with('=');
                    if let Some(sheet) = excel_data.sheets.get_mut(current_sheet) {
                        let cell = sheet.cells.entry((edit_row, edit_col))
                            .or_insert_with(CellData::default);
                        if is_formula {
                            cell.formula = edit_value.clone();
                        } else {
                            // 检查是否为日期格式单元格，如果是则将日期字符串转回序列号
                            let save_value = if let Some(ref fmt) = cell.number_format {
                                if ExcelData::is_date_format(fmt) {
                                    ExcelData::parse_date_string(edit_value)
                                        .map(|serial| serial.to_string())
                                        .unwrap_or_else(|| edit_value.clone())
                                } else {
                                    edit_value.clone()
                                }
                            } else {
                                edit_value.clone()
                            };
                            cell.value = save_value;
                            cell.formula.clear();
                        }
                    }
                    if is_formula {
                        crate::excel::formula::evaluate_sheet(&mut excel_data.sheets[current_sheet]);
                    } else {
                        crate::excel::formula::evaluate_dependents(&mut excel_data.sheets[current_sheet], edit_row, edit_col);
                    }
                    *dirty = true;
                }
                if clear_edit {
                    *editing_cell = None;
                    edit_value.clear();
                    *just_entered_edit_mode = false;
                }
            }
        }
    }
    
    // 编辑模式处理：实时更新单元格值并触发公式重算
    // 仅在编辑值实际发生变化时触发（避免每帧重复重算）
    if editing_cell.is_some() && !edit_value.is_empty() {
        if let Some((edit_col, edit_row)) = *editing_cell {
            // 记录编辑前的值，用于判断是否需要重算
            let prev_display = excel_data.sheets.get(current_sheet)
                .and_then(|s| s.cells.get(&(edit_row, edit_col)))
                .map(|c| {
                    if edit_value.starts_with('=') {
                        // 公式比较：归一化 = 前缀，与编辑模式进入时的逻辑一致
                        let f = &c.formula;
                        if f.starts_with('=') { f.clone() } else { format!("={}", f) }
                    } else {
                        cell_display_text(c).into_owned()
                    }
                })
                .unwrap_or_default();

            if edit_value != &prev_display {
                let is_formula = edit_value.starts_with('=');
                if let Some(sheet) = excel_data.sheets.get_mut(current_sheet) {
                    let cell = sheet.cells.entry((edit_row, edit_col))
                        .or_insert_with(CellData::default);
                    if is_formula {
                        cell.formula = edit_value.clone();
                    } else {
                        // 检查是否为日期格式单元格，转换日期字符串为序列号
                        let save_value = if let Some(ref fmt) = cell.number_format {
                            if ExcelData::is_date_format(fmt) {
                                ExcelData::parse_date_string(edit_value)
                                    .map(|serial| serial.to_string())
                                    .unwrap_or_else(|| edit_value.clone())
                            } else {
                                edit_value.clone()
                            }
                        } else {
                            edit_value.clone()
                        };
                        cell.value = save_value;
                    }
                }
                // 编辑中实时重算依赖公式
                if is_formula {
                    crate::excel::formula::evaluate_sheet(&mut excel_data.sheets[current_sheet]);
                } else {
                    crate::excel::formula::evaluate_dependents(&mut excel_data.sheets[current_sheet], edit_row, edit_col);
                }
                *dirty = true;
            }
        }
    }
    
    // 返回（滚动目标矩形, 选中单元格屏幕矩形）
    (scroll_to_rect, selected_cell_rect)
}
