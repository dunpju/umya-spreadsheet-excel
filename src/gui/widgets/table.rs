//! 表格渲染组件
//! 
//! 负责渲染 Excel 表格内容，包括单元格、合并单元格和对齐方式

use eframe::egui;
use crate::excel::reader::{CellAlignment, CellData, ExcelData, col_to_letter};
use crate::gui::alignment::alignment_to_egui;

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
    editing_cell: &mut Option<(u32, u32)>,
    edit_value: &mut String,
    just_entered_edit_mode: &mut bool,
) -> Option<egui::Rect> {
    // 先获取必要的数据用于键盘处理
    let (max_col, max_row, frozen_rows, frozen_cols) = if let Some(sheet) = excel_data.get_sheet(current_sheet) {
        (sheet.max_col, sheet.max_row, sheet.frozen_rows, sheet.frozen_cols)
    } else {
        return None;
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
                // Excel 行高单位是磅，转换为像素（1磅 ≈ 1.333像素）
                // 使用 max 确保行高不小于默认行高
                return (height as f32 * 1.333).max(default_row_height);
            }
        }
        default_row_height
    };
    
    // 先获取表格的尺寸和位置（这一步很关键，先做）
    // 计算表格总宽度（添加滚动条宽度边距，避免右侧单元格被覆盖）
    let mut total_width = header_width;
    for col in 1..=max_col {
        total_width += get_col_width(col) + border_width;
    }
    total_width += border_width + 11.0; // +11 像素用于垂直滚动条
    
    // 计算表格总高度（添加滚动条高度边距，避免底部单元格被覆盖）
    let mut total_height = border_width; // 顶部边框
    for row in 0..=max_row {
        total_height += get_row_height(row) + border_width;
    }
    total_height += 11.0; // +11 像素用于水平滚动条
    
    // 使用 allocate_space 分配表格空间
    let (_id, rect) = ui.allocate_space(egui::vec2(total_width, total_height));
    let table_top_left = rect.min;
    
    // 计算单元格像素矩形的辅助函数（相对于表格左上角）
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
    
    // 计算冻结区域的总尺寸（行标题列 + 冻结数据列，列标题行 + 冻结数据行）
    // 用于 is_cell_in_viewport 和滚动补偿计算
    let frozen_left_width: f32 = {
        let mut w = header_width + border_width; // col 0 (行号列)
        for c in 1..=frozen_cols {
            w += get_col_width(c) + border_width;
        }
        w
    };
    let frozen_top_height: f32 = {
        let mut h = 0.0f32;
        for r in 0..=frozen_rows {
            h += get_row_height(r) + border_width;
        }
        h
    };

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
    
    // 键盘事件处理
    let input = ui.input(|i| i.clone());
    let mut save_current_edit = false;
    let mut clear_current_edit = false;
    let mut enter_edit_mode = false;
    let mut new_selected_cell: Option<(u32, u32)> = None;
    let editing_cell_for_save = editing_cell.clone();
    
    // 用于存储滚动目标矩形
    let mut scroll_to_rect: Option<egui::Rect> = None;
    
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
            let direction = if input.modifiers.shift { "Shift+Tab (左)" } else { "Tab (右)" };
            
            println!("[Tab键处理] 当前单元格: ({},{}), 方向: {}", col, row, direction);
            
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
                println!("[Tab键处理] 新单元格: ({},{}), 是否在视口内: {}", new_col, new_row, is_cell_in_viewport(new_col, new_row));
                
                // 触边滚动机制：只有当新单元格不在可视区域内时才触发滚动
                if !is_cell_in_viewport(new_col, new_row) {
                    println!("[Tab键处理] 触发滚动到单元格 ({},{})", new_col, new_row);
                    
                    // 使用全局坐标进行滚动
                    let target_rect = get_cell_global_rect(new_col, new_row);
                    let clip_rect = ui.clip_rect();
                    println!("[Tab键处理] 滚动目标矩形(全局): {:?}", target_rect);

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
                println!("[Tab键处理] 单元格位置未变化");
            }
        } else {
            println!("[Tab键处理] 未选中任何单元格");
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
                println!("[方向键处理] 当前单元格: ({},{}), 方向: {}", col, row, direction);
                
                if new_col != col || new_row != row {
                    new_selected_cell = Some((new_col, new_row));
                    println!("[方向键处理] 新单元格: ({},{}), 是否在视口内: {}", new_col, new_row, is_cell_in_viewport(new_col, new_row));
                    
                    // 触边滚动机制：只有当新单元格不在可视区域内时才触发滚动
                    if !is_cell_in_viewport(new_col, new_row) {
                        println!("[方向键处理] 触发滚动到单元格 ({},{})", new_col, new_row);
                        
                        // 使用全局坐标进行滚动
                        let target_rect = get_cell_global_rect(new_col, new_row);
                        let clip_rect = ui.clip_rect();
                        println!("[方向键处理] 滚动目标矩形(全局): {:?}", target_rect);
                        println!("[方向键处理] 视口: {:?}", clip_rect);

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
                } else {
                    println!("[方向键处理] 单元格位置未变化（已达边界）");
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
            if let Some(sheet) = excel_data.sheets.get_mut(current_sheet) {
                let cell = sheet.cells.entry((edit_row, edit_col))
                    .or_insert_with(CellData::default);
                cell.value = edit_value.clone();
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
                    .map(|cell| cell.value.clone())
                    .unwrap_or_default();
            }
        }
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

        // 计算冻结区域总尺寸（用于冻结窗格覆盖层渲染）
        let frozen_left_width: f32 = {
            let mut w = header_width + border_width; // col 0 (行号列)
            for c in 1..=sheet.frozen_cols {
                w += get_col_width(c) + border_width;
            }
            w
        };
        let frozen_top_height: f32 = {
            let mut h = 0.0f32;
            for r in 0..=sheet.frozen_rows {
                h += get_row_height(r) + border_width;
            }
            h
        };

        // 重新计算 total_width 和 total_height（因为之前的变量可能不在作用域）
        let mut total_width = header_width;
        for col in 1..=sheet.max_col {
            total_width += get_col_width(col) + border_width;
        }
        total_width += border_width + 11.0; // +11 像素用于垂直滚动条
        
        let mut total_height = border_width; // 顶部边框
        for row in 0..=sheet.max_row {
            total_height += get_row_height(row) + border_width;
        }
        total_height += 11.0; // +11 像素用于水平滚动条
        
        // 我们已经在前面分配了空间，直接使用保存的 rect
        let rect = egui::Rect::from_min_size(table_top_left, egui::vec2(total_width, total_height));
        let top_left = table_top_left;
        
        // 获取painter用于绘制
        let painter = ui.painter_at(rect);
        
        // 保存标题区域尺寸（用于冻结窗格）- 现在使用累积数组方式，不再需要这些变量
        
        // 创建交互区域来处理点击事件（使用同一个rect）
        let response = ui.interact(rect, egui::Id::new("table_interaction"), egui::Sense::click_and_drag());
        
        // 如果表格被点击，请求焦点
        if response.clicked() {
            response.request_focus();
        }
        
        // 如果选中了单元格但表格没有焦点，重新请求焦点
        // 这可以防止Tab键切换焦点到其他UI元素
        // 但在编辑模式下不强制请求焦点，让输入框能够正常获取焦点
        if !editing_cell.is_some() && selected_cell.is_some() && input.key_pressed(egui::Key::Tab) {
            response.request_focus();
        } else if !editing_cell.is_some() && selected_cell.is_some() && !response.has_focus() {
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
            
        // 先计算所有列的累积宽度，用于准确计算可见列
        // 索引 0: 0.0 (起点)
        // 索引 1: 行号列结束位置
        // 索引 2: A列结束位置
        // ...
        let mut col_cumulative_width = vec![0.0];
        let mut current_width = 0.0;

        // 第 0 列（行号列）
        current_width += header_width + border_width;
        col_cumulative_width.push(current_width);

        // 第 1 列及以后（数据列）
        for col in 1..=sheet.max_col {
            current_width += get_col_width(col) + border_width;
            col_cumulative_width.push(current_width);
        }

        // 根据实际列宽计算可见列范围
        let target_start_x = viewport_rect.min.x - tl_x - margin;
        let target_end_x = viewport_rect.max.x - tl_x + margin;

        // 查找可见列范围
        let mut visible_cols_start = 0;
        let mut visible_cols_end = sheet.max_col + 1;

        for (i, &width) in col_cumulative_width.iter().enumerate() {
            if width > target_start_x && visible_cols_start == 0 {
                visible_cols_start = i.saturating_sub(1).max(0) as u32;
            }
            if width > target_end_x {
                visible_cols_end = i.min((sheet.max_col + 1) as usize) as u32;
                break;
            }
        }

        // 确保第 0 列（行号列）始终可见
        visible_cols_start = 0;

        // 计算累积行高用于确定可见行范围
        let mut row_cumulative_height = vec![0.0];
        let mut current_height = 0.0;
        for row in 0..=sheet.max_row {
            current_height += get_row_height(row) + border_width;
            row_cumulative_height.push(current_height);
        }

        // 简化：确保第0行（列标题行）始终可见
        let visible_rows_start = 0;
        let visible_rows_end = sheet.max_row;

        // 冻结区域边界：主网格渲染跳过这些行列，由冻结覆盖层单独绘制
        let fr = sheet.frozen_rows;
        let fc = sheet.frozen_cols;

        // 处理点击事件
        if response.clicked() {
            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                let click_x = pos.x - tl_x;
                let click_y = pos.y - tl_y;

                // 查找被点击的列（使用 < 确保边界位置归属于后一列）
                let mut clicked_col: Option<u32> = None;
                for (i, &width) in col_cumulative_width.iter().enumerate() {
                    if click_x < width && i > 1 {
                        clicked_col = Some(i as u32 - 1);
                        break;
                    }
                }

                // 查找被点击的行（使用 < 确保边界位置归属于后一行）
                let mut clicked_row: Option<u32> = None;
                for (i, &height) in row_cumulative_height.iter().enumerate() {
                    if click_y < height && i > 0 {
                        clicked_row = Some(i as u32 - 1);
                        break;
                    }
                }

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
                            // 获取单元格当前值作为编辑初始值
                            *edit_value = sheet.get_cell(edit_row, edit_col)
                                .map(|cell| cell.value.clone())
                                .unwrap_or_default();
                        }
                    }
                }
            }
        }

        // ========== 第一遍：绘制所有单元格背景 ==========
        for row in visible_rows_start..=visible_rows_end {
            // 跳过冻结区域内的行（由冻结覆盖层单独绘制，避免重影）
            if row <= fr {
                continue;
            }
            let mut x = tl_x + border_width;
            // 跳过不可见的左侧列
            for c in 0..visible_cols_start {
                x += if c == 0 { header_width } else { get_col_width(c) } + border_width;
            }

            // 使用累积行高计算 y 坐标
            let y = tl_y + border_width + row_cumulative_height[row as usize];

            // 绘制可见列
            for col in visible_cols_start..=visible_cols_end {
                // 跳过冻结区域内的列（由冻结覆盖层单独绘制，避免重影）
                if col <= fc {
                    x += if col == 0 { header_width } else { get_col_width(col) } + border_width;
                    continue;
                }
                let cell_width = if col == 0 {
                    header_width
                } else {
                    get_col_width(col)
                };
                let cell_height = get_row_height(row);

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
                    x += cell_width + border_width;
                    continue;
                }

                // 如果是合并单元格的左上角，绘制合并背景
                if is_merged_top_left {
                    if let Some(merged_range) = sheet.get_merged_range(col, row) {
                        let mut merged_col_width = 0.0;
                        for c in merged_range.start_col..=merged_range.end_col {
                            merged_col_width += get_col_width(c) + border_width;
                        }
                        merged_col_width -= border_width;

                        let mut merged_row_height = 0.0;
                        for r in merged_range.start_row..=merged_range.end_row {
                            merged_row_height += get_row_height(r) + border_width;
                        }
                        merged_row_height -= border_width;

                        let is_selected = selected_cell.is_some() &&
                            merged_range.contains(selected_cell.unwrap().0, selected_cell.unwrap().1);

                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                egui::Pos2::new(x, y),
                                egui::vec2(merged_col_width, merged_row_height),
                            ),
                            0.0,
                            if is_selected {
                                egui::Color32::from_rgb(173, 216, 230)
                            } else {
                                bg_color
                            },
                        );
                    }
                } else {
                    // 绘制普通单元格背景
                    let is_selected = *selected_cell == Some((col, row));
                    painter.rect_filled(
                        egui::Rect::from_min_size(egui::Pos2::new(x, y), egui::vec2(cell_width, cell_height)),
                        0.0,
                        if is_selected {
                            egui::Color32::from_rgb(173, 216, 230)
                        } else {
                            bg_color
                        },
                    );
                }

                // 移动到下一列
                x += cell_width + border_width;
            }
        }

        // ========== 第二遍：绘制所有单元格内容 ==========
        for row in visible_rows_start..=visible_rows_end {
            // 跳过冻结区域内的行（由冻结覆盖层单独绘制，避免重影）
            if row <= fr {
                continue;
            }
            let mut x = tl_x + border_width;
            // 跳过不可见的左侧列
            for c in 0..visible_cols_start {
                x += if c == 0 { header_width } else { get_col_width(c) } + border_width;
            }

            // 使用累积行高计算 y 坐标
            let y = tl_y + border_width + row_cumulative_height[row as usize];

            // 绘制可见列
            for col in visible_cols_start..=visible_cols_end {
                // 跳过冻结区域内的列（由冻结覆盖层单独绘制，避免重影）
                if col <= fc {
                    x += if col == 0 { header_width } else { get_col_width(col) } + border_width;
                    continue;
                }
                let cell_width = if col == 0 {
                    header_width
                } else {
                    get_col_width(col)
                };
                let cell_height = get_row_height(row);

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
                                cell_content = cell.value.clone();
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
                            cell_content = cell.value.clone();
                            alignment = cell.alignment.clone();
                            font_size = cell.font_size.map(|s| s as f32);
                            font_color = cell.font_color.map(|(r, g, b)| egui::Color32::from_rgb(r, g, b)).unwrap_or(egui::Color32::BLACK);
                        }
                    }

                    // 如果是合并单元格的非左上角部分，跳过绘制
                    if is_merged_part {
                        x += cell_width + border_width;
                        continue;
                    }

                    // 绘制合并单元格内容
                    if is_merged_top_left {
                        if let Some(merged_range) = sheet.get_merged_range(col, row) {
                            let mut merged_col_width = 0.0;
                            for c in merged_range.start_col..=merged_range.end_col {
                                merged_col_width += get_col_width(c) + border_width;
                            }
                            merged_col_width -= border_width;

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

                // 移动到下一列
                x += cell_width + border_width;
            }
        }

        // ========== 编辑模式：显示输入框 ==========
        // 复制编辑单元格坐标，避免在闭包中借用冲突
        let editing_coords = editing_cell.map(|(c, r)| (c, r));
        if let Some((edit_col, edit_row)) = editing_coords {
            // 检查是否在可见范围内
            if edit_col >= visible_cols_start && edit_col <= visible_cols_end &&
               edit_row >= visible_rows_start && edit_row <= visible_rows_end {
                
                // 计算编辑单元格的位置
                let mut x = tl_x + border_width;
                for c in 0..edit_col {
                    x += if c == 0 { header_width } else { get_col_width(c) } + border_width;
                }
                let y = tl_y + border_width + row_cumulative_height[edit_row as usize];
                
                // 检查是否是合并单元格，如果是则计算合并区域的尺寸
                let (cell_width, cell_height) = if let Some(merged_range) = sheet.get_merged_range(edit_col, edit_row) {
                    // 合并单元格：计算整个合并区域的宽度和高度
                    let mut merged_width = 0.0;
                    for c in merged_range.start_col..=merged_range.end_col {
                        merged_width += get_col_width(c) + border_width;
                    }
                    merged_width -= border_width;
                    
                    let mut merged_height = 0.0;
                    for r in merged_range.start_row..=merged_range.end_row {
                        merged_height += get_row_height(r) + border_width;
                    }
                    merged_height -= border_width;
                    
                    (merged_width, merged_height)
                } else {
                    // 普通单元格：使用单个单元格的尺寸
                    (get_col_width(edit_col), get_row_height(edit_row))
                };
                
                // 限制输入框尺寸，避免超出单元格
                // 使用更小的内边距，避免输入框超出单元格边界
                let input_width = (cell_width - 4.0).max(10.0);
                let input_height = (cell_height - 6.0).max(16.0);
                
                // 保存编辑状态用于闭包
                let mut save_cell = false;
                let mut clear_edit = false;
                
                // 创建输入框响应区域
                let rect = egui::Rect::from_min_size(
                    egui::Pos2::new(x + 2.0, y + 2.0),
                    egui::vec2(input_width, input_height)
                );
                let builder = egui::UiBuilder::new().max_rect(rect);
                ui.allocate_new_ui(builder, |ui| {
                        let text_edit = egui::TextEdit::singleline(edit_value)
                            .font(egui::FontId::default())
                            .desired_width(input_width)
                            .min_size(egui::vec2(input_width, input_height));
                        
                        let response = ui.add(text_edit);
                        
                        // 自动聚焦输入框
                        if !response.has_focus() {
                            response.request_focus();
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
                if clear_edit {
                    *editing_cell = None;
                    edit_value.clear();
                    *just_entered_edit_mode = false;
                }
            }
        }
        
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

            // 检查合并单元格
            let mut is_merged_top_left = false;
            let mut is_merged_part = false;
            if let Some(merged_range) = sheet.get_merged_range(col, row) {
                if merged_range.is_top_left(col, row) {
                    is_merged_top_left = true;
                } else {
                    is_merged_part = true;
                }
            }

            if is_merged_part {
                return;
            }

            // 获取背景色
            let bg_color = if let Some(cell) = sheet.get_cell(row, col) {
                if let Some((r, g, b)) = cell.background_color {
                    egui::Color32::from_rgb(r, g, b)
                } else {
                    egui::Color32::WHITE
                }
            } else {
                egui::Color32::WHITE
            };

            // 绘制背景
            if is_merged_top_left {
                if let Some(merged_range) = sheet.get_merged_range(col, row) {
                    let mut mw = 0.0;
                    for c in merged_range.start_col..=merged_range.end_col {
                        mw += get_col_width(c) + border_width;
                    }
                    mw -= border_width;
                    let mut mh = 0.0;
                    for r in merged_range.start_row..=merged_range.end_row {
                        mh += get_row_height(r) + border_width;
                    }
                    mh -= border_width;

                    let is_selected = selected_cell.is_some() &&
                        merged_range.contains(selected_cell.unwrap().0, selected_cell.unwrap().1);

                    painter.rect_filled(
                        egui::Rect::from_min_size(egui::Pos2::new(x, y), egui::vec2(mw, mh)),
                        0.0,
                        if is_selected { egui::Color32::from_rgb(173, 216, 230) } else { bg_color },
                    );

                    // 绘制合并单元格边框
                    painter.rect_stroke(
                        egui::Rect::from_min_size(egui::Pos2::new(x, y), egui::vec2(mw, mh)),
                        0.0,
                        egui::Stroke::new(border_width, egui::Color32::GRAY),
                    );

                    // 绘制内容
                    if let Some(cell) = sheet.get_cell(row, col) {
                        if !cell.value.is_empty() {
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
                            painter.text(text_pos, egui_align, &cell.value, font_id, font_color);
                        }
                    }
                }
            } else {
                let is_selected = *selected_cell == Some((col, row));
                painter.rect_filled(
                    egui::Rect::from_min_size(egui::Pos2::new(x, y), egui::vec2(cell_width, cell_height)),
                    0.0,
                    if is_selected { egui::Color32::from_rgb(173, 216, 230) } else { bg_color },
                );

                // 绘制单元格边框
                painter.rect_stroke(
                    egui::Rect::from_min_size(egui::Pos2::new(x, y), egui::vec2(cell_width, cell_height)),
                    0.0,
                    egui::Stroke::new(border_width, egui::Color32::GRAY),
                );

                // 绘制内容
                if let Some(cell) = sheet.get_cell(row, col) {
                    if !cell.value.is_empty() {
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
                        painter.text(text_pos, egui_align, &cell.value, font_id, font_color);
                    }
                }
            }
        };

        // ===== 绘制顺序说明 =====
        // 关键：冻结顶部数据行的合并单元格（如 N1:O1）可能向左溢出到冻结左侧区域
        // 因此必须先画顶部区域，再白色重填左侧区域覆盖溢出，最后画左侧区域内容

        // === 第1步：绘制顶部冻结区域（列标题行 + 冻结数据行）===

        // 绘制冻结列范围内的列标题（row 0，cols 1..=fc）
        for col in 1..=fc {
            let mut fixed_x = viewport_rect.min.x + header_width + border_width;
            for c in 1..col {
                fixed_x += get_col_width(c) + border_width;
            }
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
        for col in (fc + 1).max(1)..=sheet.max_col {
            let col_x = tl_x + border_width + col_cumulative_width[col as usize];
            let col_width = get_col_width(col);
            let col_height = get_row_height(0);

            let col_rect = egui::Rect::from_min_size(
                egui::Pos2::new(col_x, viewport_rect.min.y),
                egui::vec2(col_width, col_height),
            );

            if col_rect.max.x > viewport_rect.min.x + frozen_left_width && col_rect.min.x < viewport_rect.max.x {
                painter.rect_filled(col_rect, 0.0, egui::Color32::LIGHT_GRAY);
                painter.rect_stroke(col_rect, 0.0, egui::Stroke::new(border_width, egui::Color32::GRAY));
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
            let mut fixed_y = viewport_rect.min.y;
            for r in 0..row {
                fixed_y += get_row_height(r) + border_width;
            }
            // 冻结列部分（cols 1..=fc）
            for col in 1..=fc {
                let mut fixed_x = viewport_rect.min.x + header_width + border_width;
                for c in 1..col {
                    fixed_x += get_col_width(c) + border_width;
                }
                draw_frozen_cell(&painter, col, row, fixed_x, fixed_y);
            }
            // 非冻结列部分（cols > fc）- scroll-dependent x
            for col in (fc + 1)..=sheet.max_col {
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
            let mut fixed_x = viewport_rect.min.x + header_width + border_width;
            for c in 1..col {
                fixed_x += get_col_width(c) + border_width;
            }
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
            let mut fixed_y = viewport_rect.min.y;
            for r in 0..row {
                fixed_y += get_row_height(r) + border_width;
            }
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
            let mut fixed_y = viewport_rect.min.y;
            for r in 0..row {
                fixed_y += get_row_height(r) + border_width;
            }
            for col in 1..=fc {
                let mut fixed_x = viewport_rect.min.x + header_width + border_width;
                for c in 1..col {
                    fixed_x += get_col_width(c) + border_width;
                }
                draw_frozen_cell(&painter, col, row, fixed_x, fixed_y);
            }
        }

        // 非冻结行的行号（col 0，rows > fr）
        for row in (fr + 1).max(1)..=sheet.max_row {
            let row_y = tl_y + border_width + row_cumulative_height[row as usize];
            let row_width = header_width;
            let row_height = get_row_height(row);

            let row_rect = egui::Rect::from_min_size(
                egui::Pos2::new(viewport_rect.min.x, row_y),
                egui::vec2(row_width, row_height),
            );

            if row_rect.max.y > viewport_rect.min.y + frozen_top_height && row_rect.min.y < viewport_rect.max.y {
                painter.rect_filled(row_rect, 0.0, egui::Color32::LIGHT_GRAY);
                painter.rect_stroke(row_rect, 0.0, egui::Stroke::new(border_width, egui::Color32::GRAY));
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
        for row in (fr + 1)..=sheet.max_row {
            let row_y = tl_y + border_width + row_cumulative_height[row as usize];
            if row_y + get_row_height(row) <= viewport_rect.min.y + frozen_top_height { continue; }
            if row_y >= viewport_rect.max.y { break; }
            for col in 1..=fc {
                let mut fixed_x = viewport_rect.min.x + header_width + border_width;
                for c in 1..col {
                    fixed_x += get_col_width(c) + border_width;
                }
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
                let mut fixed_x = viewport_rect.min.x + header_width + border_width;
                for c in 1..col {
                    fixed_x += get_col_width(c) + border_width;
                }
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
                let mut fixed_y = viewport_rect.min.y;
                for r in 0..row {
                    fixed_y += get_row_height(r) + border_width;
                }
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
                let mut fixed_y = viewport_rect.min.y;
                for r in 0..row {
                    fixed_y += get_row_height(r) + border_width;
                }
                for col in 1..=fc {
                    let mut fixed_x = viewport_rect.min.x + header_width + border_width;
                    for c in 1..col {
                        fixed_x += get_col_width(c) + border_width;
                    }
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
    }
    
    // 编辑模式处理使用独立的可变借用（在不可变借用作用域外）
    // 如果正在编辑且编辑值已更改，保存到单元格
    if editing_cell.is_some() && !edit_value.is_empty() {
        if let Some((edit_col, edit_row)) = *editing_cell {
            if let Some(sheet) = excel_data.sheets.get_mut(current_sheet) {
                let cell = sheet.cells.entry((edit_row, edit_col))
                    .or_insert_with(CellData::default);
                cell.value = edit_value.clone();
            }
        }
    }
    
    // 返回滚动目标矩形
    scroll_to_rect
}
