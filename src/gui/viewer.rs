//! Excel 查看器主模块
//! 
//! 整合所有子模块，提供完整的 Excel 查看功能

use eframe::egui;
use crate::excel::reader::ExcelData;
use crate::gui::state::LoadState;
use crate::gui::fonts::setup_fonts;
use crate::gui::widgets::{
    draw_menu_bar,
    draw_import_dialog,
    draw_table_content,
    draw_empty_state,
    draw_name_box,
    NameBoxState,
};
use std::sync::mpsc::Receiver;

/// 右键菜单状态
#[derive(Debug)]
pub struct ContextMenuState {
    /// 是否可见
    pub visible: bool,
    /// 弹出位置（屏幕坐标）
    pub position: egui::Pos2,
    /// 右键点击的目标单元格 (col, row)
    pub target_cell: Option<(u32, u32)>,
    /// 插入行数
    pub insert_rows_count: u32,
    /// 插入列数
    pub insert_cols_count: u32,
}

/// 右键菜单操作类型
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ContextAction {
    InsertRowAbove,
    InsertRowBelow,
    InsertColumnLeft,
    InsertColumnRight,
}

impl Default for ContextMenuState {
    fn default() -> Self {
        Self {
            visible: false,
            position: egui::Pos2::ZERO,
            target_cell: None,
            insert_rows_count: 1,
            insert_cols_count: 1,
        }
    }
}

/// 设置面板状态
#[derive(Debug)]
pub struct SettingsPanelState {
    /// 是否显示设置面板
    pub visible: bool,
    /// 当前选中的设置页
    pub active_page: Option<SettingsPage>,
    // 合并配置参数
    /// 列范围起始
    pub merge_col_start: u32,
    /// 列范围结束
    pub merge_col_end: u32,
    /// 横向每 N 个单元格合并
    pub merge_col_group: u32,
    /// 行范围起始
    pub merge_row_start: u32,
    /// 行范围结束
    pub merge_row_end: u32,
    /// 纵向每 N 个单元格合并
    pub merge_row_group: u32,
    /// 保存成功提示计时（秒）
    pub save_success_timer: f32,
}

impl Default for SettingsPanelState {
    fn default() -> Self {
        Self {
            visible: false,
            active_page: None,
            merge_col_start: 0,
            merge_col_end: 0,
            merge_col_group: 0,
            merge_row_start: 0,
            merge_row_end: 0,
            merge_row_group: 0,
            save_success_timer: 0.0,
        }
    }
}

/// 设置页类型
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SettingsPage {
    ColumnConfig,
    RowConfig,
}

/// Excel 查看器主结构体，管理所有 UI 状态和数据
pub struct ExcelViewer {
    /// 当前加载的 Excel 数据（未加载时为 None）
    pub excel_data: Option<ExcelData>,
    /// 当前显示的工作表索引（从0开始）
    pub current_sheet: usize,
    /// 错误信息（有错误时为 Some）
    pub error_message: Option<String>,
    /// 当前选中的单元格坐标（列, 行）
    pub selected_cell: Option<(u32, u32)>,
    /// 当前正在编辑的单元格坐标（列, 行）
    pub editing_cell: Option<(u32, u32)>,
    /// 当前编辑的值
    pub edit_value: String,
    /// 是否刚进入编辑模式（用于忽略进入编辑时的Enter键）
    pub just_entered_edit_mode: bool,
    /// 当前鼠标悬停的单元格坐标
    pub hovered_cell: Option<(u32, u32)>,
    /// 是否显示导入文件对话框
    pub show_import_dialog: bool,
    /// 当前的加载状态
    pub load_state: LoadState,
    /// 异步加载的通道接收器
    pub rx: Option<Receiver<Result<ExcelData, String>>>,
    /// 名称框状态
    pub name_box_state: NameBoxState,
    /// 待保存的公式值（由公式栏触发）
    pub pending_formula_save: Option<String>,
    /// 数据有效性校验错误弹窗
    pub validation_error: Option<(String, String)>, // (title, message)
    /// 校验错误弹窗的固定位置（记录触发校验时的单元格位置，不随选中变化）
    pub validation_error_pos: Option<egui::Pos2>,
    /// 编辑前的原始单元格数据，用于校验失败恢复
    pub original_cell_data: Option<((u32, u32), String, String)>, // ((col, row), value, formula)
    /// 右键菜单状态
    pub context_menu: ContextMenuState,
    /// 设置面板状态
    pub settings_panel: SettingsPanelState,
    /// 当前加载的文件路径
    pub file_path: Option<String>,
}

impl ExcelViewer {
    /// 创建新的 Excel 查看器实例，初始化所有状态
    pub fn new() -> Self {
        Self {
            excel_data: None,
            current_sheet: 0,
            error_message: None,
            selected_cell: None,
            editing_cell: None,
            edit_value: String::new(),
            just_entered_edit_mode: false,
            hovered_cell: None,
            show_import_dialog: false,
            load_state: LoadState::Idle,
            rx: None,
            name_box_state: NameBoxState::default(),
            pending_formula_save: None,
            validation_error: None,
            validation_error_pos: None,
            original_cell_data: None,
            context_menu: ContextMenuState::default(),
            settings_panel: SettingsPanelState::default(),
            file_path: None,
        }
    }

    /// 启动异步加载 Excel 文件
    /// 
    /// 在后台线程中读取文件，避免阻塞 UI
    /// 
    /// # 参数
    /// * `path` - Excel 文件路径
    /// * `ctx` - egui 上下文，用于加载完成后请求重绘
    pub fn start_async_load(&mut self, path: String, ctx: egui::Context) {
        // 创建消息通道用于线程间通信
        let (tx, rx) = std::sync::mpsc::channel();
        self.rx = Some(rx);
        self.load_state = LoadState::Loading;
        self.error_message = None;
        self.file_path = Some(path.clone());

        // 启动后台线程加载文件
        std::thread::spawn(move || {
            match ExcelData::load_from_file(&path) {
                Ok(data) => {
                    // 加载成功，发送数据
                    let _ = tx.send(Ok(data));
                }
                Err(e) => {
                    // 加载失败，发送错误信息
                    let _ = tx.send(Err(e));
                }
            }
            // 请求界面重绘
            ctx.request_repaint();
        });
    }

    /// 检查异步加载结果
    /// 
    /// 从通道中尝试接收加载结果，并更新状态
    pub fn check_load_result(&mut self) {
        if let Some(ref rx) = self.rx {
            // 尝试非阻塞地接收结果
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok(data) => {
                        // 加载成功，更新数据和状态
                        self.excel_data = Some(data);
                        self.current_sheet = 0;
                        self.selected_cell = None;
                        self.editing_cell = None;
                        self.edit_value.clear();
                        self.pending_formula_save = None;
                        self.hovered_cell = None;
                        self.error_message = None;
                        self.load_state = LoadState::Success(self.excel_data.clone().unwrap());
                    }
                    Err(e) => {
                        // 加载失败，保存错误信息
                        self.error_message = Some(e.clone());
                        self.load_state = LoadState::Failed(e);
                    }
                }
                // 清除接收器
                self.rx = None;
            }
        }
    }
}

/// 实现 eframe::App trait，这是 egui 应用程序的入口
impl eframe::App for ExcelViewer {
    /// 每帧更新 UI
    /// 
    /// # 参数
    /// * `ctx` - egui 上下文
    /// * `_frame` - eframe 框架
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 设置中文字体
        setup_fonts(ctx);
        
        // 绘制菜单栏
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            draw_menu_bar(ui, &mut self.show_import_dialog, &mut self.settings_panel);
        });
        
        // 绘制导入对话框
        if let Some(path) = draw_import_dialog(&mut self.show_import_dialog) {
            self.start_async_load(path, ctx.clone());
        }

        // 绘制设置面板
        if self.settings_panel.visible {
            let active_page = self.settings_panel.active_page;
            let title = "插入配置";
            let mut keep_open = true;
            egui::Window::new("settings_panel")
                .title_bar(false)
                .open(&mut keep_open)
                .resizable(false)
                .collapsible(false)
                .default_pos(ctx.screen_rect().center() - egui::vec2(190.0, 80.0))
                .show(ctx, |ui| {
                    ui.set_min_width(420.0);
                    // 自定义小字体标题栏
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(title).size(12.0).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("X").clicked() {
                                self.settings_panel.visible = false;
                            }
                            if ui.button("保存").clicked() {
                                // TODO: 执行合并操作
                                self.settings_panel.save_success_timer = 2.0;
                            }
                            if self.settings_panel.save_success_timer > 0.0 {
                                ui.label(egui::RichText::new("保存成功").size(11.0).color(egui::Color32::GREEN));
                                self.settings_panel.save_success_timer -= ui.input(|i| i.stable_dt);
                            }
                        });
                    });
                    ui.separator();
                    // 选项卡切换
                    ui.horizontal(|ui| {
                        if ui.selectable_label(active_page == Some(SettingsPage::ColumnConfig), "列配置").clicked() {
                            self.settings_panel.active_page = Some(SettingsPage::ColumnConfig);
                        }
                        if ui.selectable_label(active_page == Some(SettingsPage::RowConfig), "行配置").clicked() {
                            self.settings_panel.active_page = Some(SettingsPage::RowConfig);
                        }
                    });
                    ui.separator();

                    match active_page {
                        Some(SettingsPage::ColumnConfig) => {
                            ui.vertical(|ui| {
                                // 合并配置块
                                ui.group(|ui| {
                                    ui.label(egui::RichText::new("合并").size(12.0).strong());
                                    ui.add_space(6.0);
                                    // 列范围 + 合并数量在同一行
                                    ui.horizontal(|ui| {
                                        ui.label("列范围:");
                                        ui.add(egui::DragValue::new(&mut self.settings_panel.merge_col_start)
                                            .range(0..=10000).speed(0.1));
                                        ui.label("列 至");
                                        ui.add(egui::DragValue::new(&mut self.settings_panel.merge_col_end)
                                            .range(0..=10000).speed(0.1));
                                        ui.label("列");
                                        ui.separator();
                                        ui.label("横向每");
                                        ui.add(egui::DragValue::new(&mut self.settings_panel.merge_col_group)
                                            .range(0..=1000).speed(0.1));
                                        ui.label("个单元格进行合并");
                                    });
                                    ui.add_space(6.0);
                                    // 行范围 + 合并数量在同一行
                                    ui.horizontal(|ui| {
                                        ui.label("行范围:");
                                        ui.add(egui::DragValue::new(&mut self.settings_panel.merge_row_start)
                                            .range(0..=10000).speed(0.1));
                                        ui.label("行 至");
                                        ui.add(egui::DragValue::new(&mut self.settings_panel.merge_row_end)
                                            .range(0..=10000).speed(0.1));
                                        ui.label("行");
                                        ui.separator();
                                        ui.label("纵向每");
                                        ui.add(egui::DragValue::new(&mut self.settings_panel.merge_row_group)
                                            .range(0..=1000).speed(0.1));
                                        ui.label("个单元格进行合并");
                                    });
                                });
                            });
                        }
                        Some(SettingsPage::RowConfig) => {
                            ui.vertical(|ui| {
                                ui.label("列配置功能");
                                ui.add_space(4.0);
                                ui.colored_label(egui::Color32::GRAY, "（功能开发中...）");
                            });
                        }
                        None => {
                            ui.label("请选择配置页");
                        }
                    }
                });
            if !keep_open {
                self.settings_panel.visible = false;
            }
        }

        // 检查异步加载结果
        self.check_load_result();

        // 底部区域：工作表选择器 + 文件路径状态栏
        // 注意：TopBottomPanel 按代码顺序从下往上堆叠，先渲染的在最底部
        // 先渲染 status_bar（最底部），再渲染 sheet_bar（其上方），CentralPanel 在最上面

        // 文件路径状态栏（最底部）
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(20.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(6.0);
                    if let Some(path) = &self.file_path {
                        ui.label(
                            egui::RichText::new(path.as_str())
                                .font(egui::FontId::proportional(12.0))
                                .color(egui::Color32::from_rgb(100, 100, 100)),
                        );
                    }
                });
            });

        // 工作表选择器（状态栏上方）
        if self.excel_data.is_some() {
            egui::TopBottomPanel::bottom("sheet_bar")
                .exact_height(28.0)
                .show(ctx, |ui| {
                    ui.style_mut().spacing.button_padding = egui::vec2(8.0, 4.0);
                    ui.horizontal(|ui| {
                        for (i, sheet) in self.excel_data.as_ref().unwrap().sheets.iter().enumerate() {
                            if ui.selectable_label(self.current_sheet == i, &sheet.name).clicked() {
                                self.current_sheet = i;
                                self.selected_cell = None;
                            }
                        }
                    });
                });
        }

        // 主内容区域
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(excel_data) = &mut self.excel_data {
                // 预先获取工作表信息
                let max_col = excel_data.get_sheet(self.current_sheet).map(|s| s.max_col).unwrap_or(0);
                let max_row = excel_data.get_sheet(self.current_sheet).map(|s| s.max_row).unwrap_or(0);
                
                let display_text = self.selected_cell.and_then(|(col, row)| {
                    excel_data.get_sheet(self.current_sheet).and_then(|sheet| {
                        let (target_col, target_row) = if let Some(merged_range) = sheet.get_merged_range(col, row) {
                            (merged_range.start_col, merged_range.start_row)
                        } else {
                            (col, row)
                        };
                        sheet.get_cell(target_row, target_col).map(|cell| {
                            if !cell.formula.is_empty() {
                                let f = &cell.formula;
                                if f.starts_with('=') { f.clone() } else { format!("={}", f) }
                            } else if let Some(ref fmt) = cell.number_format {
                                if ExcelData::is_date_format(fmt) {
                                    if let Ok(serial) = cell.value.parse::<f64>() {
                                        ExcelData::format_date(serial, fmt)
                                    } else {
                                        cell.value.clone()
                                    }
                                } else {
                                    cell.value.clone()
                                }
                            } else {
                                cell.value.clone()
                            }
                        })
                    })
                });
                
                ui.set_min_height(28.0);
                ui.style_mut().spacing.item_spacing = egui::vec2(4.0, 4.0);
                
                if let Some((col, row)) = draw_name_box(
                    ui,
                    &mut self.name_box_state,
                    self.selected_cell,  // 直接使用选中的单元格，不转换为合并单元格的左上角
                    display_text.as_deref(),
                    max_col,
                    max_row,
                    &mut self.pending_formula_save,
                ) {
                    self.selected_cell = Some((col, row));
                }
                
                ui.separator();
                
                // 冻结窗格布局：列标题固定顶部，行标题固定左侧
                // 双向滚动区域（垂直+水平），替代嵌套 ScrollArea
                // 嵌套 ScrollArea 会导致 scroll_to_rect 无法同时作用于两个方向
                egui::ScrollArea::both()
                    .id_salt("table_scroll")
                    .show(ui, |ui| {
                        let (_, cell_rect) = draw_table_content(
                            ui,
                            excel_data,
                            self.current_sheet,
                            &mut self.selected_cell,
                            &mut self.editing_cell,
                            &mut self.edit_value,
                            &mut self.just_entered_edit_mode,
                            &mut self.validation_error,
                            &mut self.original_cell_data,
                            &mut self.context_menu,
                        );

                        // 绘制数据有效性输入提示弹窗
                        if let Some(cell_rect) = cell_rect {
                            if let Some(sheet) = excel_data.get_sheet(self.current_sheet) {
                                if let Some((col, row)) = self.selected_cell {
                                    if let Some(dv) = sheet.get_input_message(col, row) {
                                        let pos = cell_rect.left_bottom() + egui::vec2(0.0, 2.0);
                                        let popup_width = cell_rect.width().max(100.0);
                                        egui::Area::new(egui::Id::new("data_validation_popup"))
                                            .fixed_pos(pos)
                                            .order(egui::Order::Foreground)
                                            .show(ui.ctx(), |ui| {
                                                egui::Frame::popup(ui.style())
                                                    .fill(egui::Color32::from_rgb(255, 255, 225))
                                                    .show(ui, |ui| {
                                                        ui.set_min_width(popup_width);
                                                        ui.set_max_width(popup_width);
                                                        if !dv.prompt_title.is_empty() {
                                                            ui.strong(&dv.prompt_title);
                                                        }
                                                        if !dv.prompt.is_empty() {
                                                            ui.label(&dv.prompt);
                                                        }
                                                    });
                                            });
                                    }
                                }
                            }
                        }

                        // 首次记录校验错误弹窗位置（固定在触发校验的单元格下方）
                        if self.validation_error.is_some() && self.validation_error_pos.is_none() {
                            if let Some(cr) = cell_rect {
                                self.validation_error_pos = Some(cr.left_bottom() + egui::vec2(0.0, 2.0));
                            }
                        }

                        // 绘制数据有效性校验错误弹窗（使用固定位置，不随选中单元格变化）
                        if let Some((ref title, ref msg)) = self.validation_error {
                            if let Some(pos) = self.validation_error_pos {
                                let title = title.clone();
                                let msg = msg.clone();
                                let popup_width = 200.0;
                                egui::Area::new(egui::Id::new("data_validation_error_popup"))
                                    .fixed_pos(pos)
                                    .order(egui::Order::Foreground)
                                    .show(ui.ctx(), |ui| {
                                        egui::Frame::popup(ui.style())
                                            .fill(egui::Color32::from_rgb(255, 255, 225))
                                            .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(200, 160, 0)))
                                            .show(ui, |ui| {
                                                ui.set_min_width(popup_width);
                                                ui.set_max_width(popup_width.max(300.0));
                                                // 红色错误图标 + 标题
                                                ui.horizontal(|ui| {
                                                    ui.label(egui::RichText::new("✖").color(egui::Color32::RED).size(14.0));
                                                    ui.strong(egui::RichText::new(&title).size(12.0));
                                                });
                                                ui.label(egui::RichText::new(&msg).size(11.0));
                                                ui.add_space(4.0);
                                                ui.horizontal(|ui| {
                                                    if ui.button("重试").clicked() {
                                                        self.validation_error = None;
                                                        self.validation_error_pos = None;
                                                    }
                                                    if ui.button("取消").clicked() {
                                                        // 恢复原始单元格数据
                                                        if let Some(((col, row), ref orig_value, ref orig_formula)) = self.original_cell_data {
                                                            if let Some(sheet) = excel_data.sheets.get_mut(self.current_sheet) {
                                                                let cell = sheet.cells.entry((row, col))
                                                                    .or_insert_with(crate::excel::reader::CellData::default);
                                                                cell.value = orig_value.clone();
                                                                cell.formula = orig_formula.clone();
                                                                // 触发公式重算
                                                                if orig_formula.is_empty() {
                                                                    crate::excel::formula::evaluate_dependents(&mut excel_data.sheets[self.current_sheet], row, col);
                                                                } else {
                                                                    crate::excel::formula::evaluate_sheet(&mut excel_data.sheets[self.current_sheet]);
                                                                }
                                                            }
                                                        }
                                                        self.original_cell_data = None;
                                                        self.validation_error = None;
                                                        self.validation_error_pos = None;
                                                        self.editing_cell = None;
                                                        self.edit_value.clear();
                                                        self.pending_formula_save = None;
                                                    }
                                                });
                                            });
                                    });
                            }
                        }
                    });

                    // 绘制右键上下文菜单
                    if self.context_menu.visible {
                        let menu_pos = self.context_menu.position;

                        // 收集操作结果，避免闭包内多重借用
                        let mut pending_action: Option<ContextAction> = None;

                        egui::Area::new(egui::Id::new("context_menu"))
                            .fixed_pos(menu_pos)
                            .order(egui::Order::Foreground)
                            .show(ui.ctx(), |ui| {
                                egui::Frame::popup(ui.style())
                                    .fill(egui::Color32::WHITE)
                                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(180, 180, 180)))
                                    .show(ui, |ui| {
                                        ui.set_min_width(220.0);
                                        ui.vertical(|ui| {
                                            // 插入行
                                            ui.horizontal(|ui| {
                                                if ui.button("在上方插入行").clicked() {
                                                    pending_action = Some(ContextAction::InsertRowAbove);
                                                }
                                                ui.add(egui::DragValue::new(&mut self.context_menu.insert_rows_count)
                                                    .range(1..=1000)
                                                    .speed(0.1));
                                                ui.label("行");
                                            });
                                            ui.horizontal(|ui| {
                                                if ui.button("在下方插入行").clicked() {
                                                    pending_action = Some(ContextAction::InsertRowBelow);
                                                }
                                                ui.add(egui::DragValue::new(&mut self.context_menu.insert_rows_count)
                                                    .range(1..=1000)
                                                    .speed(0.1));
                                                ui.label("行");
                                            });

                                            ui.separator();

                                            // 插入列
                                            ui.horizontal(|ui| {
                                                if ui.button("在左侧插入列").clicked() {
                                                    pending_action = Some(ContextAction::InsertColumnLeft);
                                                }
                                                ui.add(egui::DragValue::new(&mut self.context_menu.insert_cols_count)
                                                    .range(1..=1000)
                                                    .speed(0.1));
                                                ui.label("列");
                                            });
                                            ui.horizontal(|ui| {
                                                if ui.button("在右侧插入列").clicked() {
                                                    pending_action = Some(ContextAction::InsertColumnRight);
                                                }
                                                ui.add(egui::DragValue::new(&mut self.context_menu.insert_cols_count)
                                                    .range(1..=1000)
                                                    .speed(0.1));
                                                ui.label("列");
                                            });
                                        });
                                    });
                            });

                        // 执行插入操作（在闭包外处理，避免借用冲突）
                        if let Some(action) = pending_action {
                            if let Some((col, row)) = self.context_menu.target_cell {
                                // 先关闭编辑状态
                                self.editing_cell = None;
                                self.edit_value.clear();
                                self.original_cell_data = None;
                                self.validation_error = None;
                                self.validation_error_pos = None;

                                if let Some(sheet) = excel_data.sheets.get_mut(self.current_sheet) {
                                    // 计算锚点：合并单元格取合并边界
                                    let (anchor_col, anchor_row) = if let Some(mr) = sheet.get_merged_range(col, row) {
                                        match action {
                                            ContextAction::InsertRowAbove => (col, mr.start_row),
                                            ContextAction::InsertRowBelow => (col, mr.end_row),
                                            ContextAction::InsertColumnLeft => (mr.start_col, row),
                                            ContextAction::InsertColumnRight => (mr.end_col, row),
                                        }
                                    } else {
                                        (col, row)
                                    };

                                    let n = self.context_menu.insert_rows_count;
                                    let m = self.context_menu.insert_cols_count;

                                    match action {
                                        ContextAction::InsertRowAbove => {
                                            sheet.insert_rows(anchor_row, n, false);
                                        }
                                        ContextAction::InsertRowBelow => {
                                            sheet.insert_rows(anchor_row, n, true);
                                        }
                                        ContextAction::InsertColumnLeft => {
                                            sheet.insert_columns(anchor_col, m, false);
                                        }
                                        ContextAction::InsertColumnRight => {
                                            sheet.insert_columns(anchor_col, m, true);
                                        }
                                    }
                                    crate::excel::formula::evaluate_sheet(&mut excel_data.sheets[self.current_sheet]);
                                }
                            }
                            self.context_menu.visible = false;
                        }

                        // 点击菜单外部关闭
                        let menu_id = egui::Id::new("context_menu");
                        let menu_area = ui.ctx().memory(|mem| {
                            mem.area_rect(menu_id)
                        });
                        if let Some(menu_rect) = menu_area {
                            if ui.input(|i| i.pointer.any_click()) {
                                if let Some(hover) = ui.input(|i| i.pointer.hover_pos()) {
                                    if !menu_rect.contains(hover) {
                                        self.context_menu.visible = false;
                                    }
                                }
                            }
                        }
                        // Escape 关闭
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            self.context_menu.visible = false;
                        }
                    }

                // 处理公式栏的待保存值
                if let Some(formula_value) = self.pending_formula_save.take() {
                    if let Some((col, row)) = self.selected_cell {
                        // 非公式值做数据有效性校验
                        if !formula_value.starts_with('=') {
                            if let Some(sheet) = excel_data.get_sheet(self.current_sheet) {
                                if let Some((_title, _msg)) = sheet.validate_cell(col, row, &formula_value) {
                                    self.validation_error = Some((_title, _msg));
                                    // 保存原始单元格数据，用于取消时恢复
                                    let orig = sheet.get_cell(row, col)
                                        .map(|c| (c.value.clone(), c.formula.clone()))
                                        .unwrap_or_default();
                                    self.original_cell_data = Some(((col, row), orig.0, orig.1));
                                } else {
                                    // 校验通过，执行保存
                                    let cell = excel_data.sheets[self.current_sheet]
                                        .cells.entry((row, col))
                                        .or_insert_with(|| crate::excel::reader::CellData::default());
                                    let save_value = if let Some(ref fmt) = cell.number_format {
                                        if ExcelData::is_date_format(fmt) {
                                            ExcelData::parse_date_string(&formula_value)
                                                .map(|serial| serial.to_string())
                                                .unwrap_or_else(|| formula_value.clone())
                                        } else {
                                            formula_value.clone()
                                        }
                                    } else {
                                        formula_value.clone()
                                    };
                                    cell.value = save_value;
                                    cell.formula.clear();
                                    crate::excel::formula::evaluate_dependents(&mut excel_data.sheets[self.current_sheet], row, col);
                                }
                            }
                        } else {
                            // 公式直接保存
                            let cell = excel_data.sheets[self.current_sheet]
                                .cells.entry((row, col))
                                .or_insert_with(|| crate::excel::reader::CellData::default());
                            cell.formula = formula_value;
                            crate::excel::formula::evaluate_sheet(&mut excel_data.sheets[self.current_sheet]);
                        }
                    }
                }
            } else {
                // 未加载文件，显示相应状态
                match &self.load_state {
                    LoadState::Loading => {
                        // 加载中，显示 spinner
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("正在解析 Excel 样式与公式，请稍候...");
                        });
                        ctx.request_repaint();
                    }
                    LoadState::Failed(_) => {
                        // 加载失败，显示空状态
                        draw_empty_state(ui);
                    }
                    _ => {
                        // 空闲状态，显示空状态
                        draw_empty_state(ui);
                    }
                }
            }
        });
    }
}

