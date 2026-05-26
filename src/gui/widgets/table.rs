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
pub fn draw_table_content(
    ui: &mut egui::Ui,
    excel_data: &mut ExcelData,
    current_sheet: usize,
    selected_cell: &mut Option<(u32, u32)>,
    editing_cell: &mut Option<(u32, u32)>,
    edit_value: &mut String,
    just_entered_edit_mode: &mut bool,
) {
    // 先获取必要的数据用于键盘处理
    let (max_col, max_row) = if let Some(sheet) = excel_data.get_sheet(current_sheet) {
        (sheet.max_col, sheet.max_row)
    } else {
        return;
    };
    
    // 如果已经在编辑模式中，重置"刚进入编辑模式"标志位
    // 这样下一帧就可以正常处理Enter键了
    if editing_cell.is_some() && *just_entered_edit_mode {
        *just_entered_edit_mode = false;
    }
    
    // 键盘事件处理（在借用之前）
    let input = ui.input(|i| i.clone());
    let mut save_current_edit = false;
    let mut clear_current_edit = false;
    let mut enter_edit_mode = false;
    let mut new_selected_cell: Option<(u32, u32)> = None;
    let editing_cell_for_save = editing_cell.clone();
    
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
            
            // 获取sheet用于检查合并单元格
            let sheet = excel_data.get_sheet(current_sheet);
            
            if input.modifiers.shift {
                // Shift+Tab: 向左移动
                if col > 1 {
                    // 检查当前单元格是否是合并单元格的一部分
                    let current_col = if let Some(s) = sheet {
                        if let Some(merged_range) = s.get_merged_range(col, row) {
                            // 如果是合并单元格，从合并区域的起始列开始向左移动
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
                        // 如果是合并单元格，从合并区域的结束列开始向右移动
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
            }
        }
        // 消费Tab键事件，防止传递到菜单栏
        ui.input_mut(|i| i.consume_key(input.modifiers, egui::Key::Tab));
    }
    
    // 方向键处理（非编辑模式下）- 在表格有焦点时进行单元格切换
    if editing_cell.is_none() && selected_cell.is_some() {
        if let Some((col, row)) = *selected_cell {
            let mut new_col = col;
            let mut new_row = row;
            
            // 获取sheet用于检查合并单元格
            let sheet = excel_data.get_sheet(current_sheet);
            
            if input.key_pressed(egui::Key::ArrowUp) {
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
            
            if new_col != col || new_row != row {
                new_selected_cell = Some((new_col, new_row));
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
                height as f32 * 1.333
            } else {
                default_row_height
            }
        };
        
        // 计算表格总宽度（添加滚动条宽度边距，避免右侧单元格被覆盖）
        let mut total_width = header_width;
        for col in 1..=sheet.max_col {
            total_width += get_col_width(col) + border_width;
        }
        total_width += border_width + 11.0; // +11 像素用于垂直滚动条
        
        // 计算表格总高度（添加滚动条高度边距，避免底部单元格被覆盖）
        let mut total_height = border_width; // 顶部边框
        for row in 0..=sheet.max_row {
            total_height += get_row_height(row) + border_width;
        }
        total_height += 11.0; // +11 像素用于水平滚动条
        
        // 使用 allocate_space 分配表格空间（不强制精确尺寸）
        let (_id, rect) = ui.allocate_space(egui::vec2(total_width, total_height));
        let top_left = rect.min;
        
        // 获取painter用于绘制
        let painter = ui.painter_at(rect);
        
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
            let mut x = tl_x + border_width;
            // 跳过不可见的左侧列
            for c in 0..visible_cols_start {
                x += if c == 0 { header_width } else { get_col_width(c) } + border_width;
            }

            // 使用累积行高计算 y 坐标
            let y = tl_y + border_width + row_cumulative_height[row as usize];

            // 绘制可见列
            for col in visible_cols_start..=visible_cols_end {
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
            let mut x = tl_x + border_width;
            // 跳过不可见的左侧列
            for c in 0..visible_cols_start {
                x += if c == 0 { header_width } else { get_col_width(c) } + border_width;
            }

            // 使用累积行高计算 y 坐标
            let y = tl_y + border_width + row_cumulative_height[row as usize];

            // 绘制可见列
            for col in visible_cols_start..=visible_cols_end {
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
                
                // 限制输入框宽度，避免超出单元格
                let input_width = (cell_width - 8.0).max(10.0);
                
                // 保存编辑状态用于闭包
                let mut save_cell = false;
                let mut clear_edit = false;
                
                // 创建输入框响应区域
                let rect = egui::Rect::from_min_size(
                    egui::Pos2::new(x + 4.0, y + 2.0),
                    egui::vec2(input_width, cell_height - 4.0)
                );
                let builder = egui::UiBuilder::new().max_rect(rect);
                ui.allocate_new_ui(builder, |ui| {
                        let text_edit = egui::TextEdit::singleline(edit_value)
                            .font(egui::FontId::default())
                            .desired_width(input_width);
                        
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
                if save_cell {
                    if let Some(sheet) = excel_data.sheets.get_mut(current_sheet) {
                        let cell = sheet.cells.entry((edit_row, edit_col))
                            .or_insert_with(CellData::default);
                        cell.value = edit_value.clone();
                    }
                }
                if clear_edit {
                    *editing_cell = None;
                    edit_value.clear();
                    *just_entered_edit_mode = false;
                }
            }
        }
    }
}
