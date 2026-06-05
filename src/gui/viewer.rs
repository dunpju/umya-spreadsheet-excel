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

/// 撤销栈最大深度
const MAX_UNDO_DEPTH: usize = 20;

/// 撤销快照：保存操作前的整个工作表状态
struct UndoSnapshot {
    /// 操作前的工作表数据（深拷贝）
    sheet_data: crate::excel::reader::SheetData,
    /// 操作前的工作表索引
    sheet_index: usize,
    /// 操作前选中的单元格
    selected_cell: Option<(u32, u32)>,
}

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
    /// 确认弹窗是否可见
    pub confirm_visible: bool,
    /// 确认弹窗是否已显示超过一帧（用于跳过首帧外部点击检测）
    pub confirm_established: bool,
    /// 确认弹窗对应的操作
    pub confirm_action: Option<ContextAction>,
    /// 确认弹窗：复制合并
    pub copy_merge: bool,
    /// 确认弹窗：复制公式
    pub copy_formula: bool,
    /// 确认弹窗：复制样式
    pub copy_style: bool,
    /// 确认弹窗：复制值
    pub copy_value: bool,
}

/// 右键菜单操作类型
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ContextAction {
    InsertRowAbove,
    InsertRowBelow,
    InsertColumnLeft,
    InsertColumnRight,
    ClearCell,
}

impl Default for ContextMenuState {
    fn default() -> Self {
        Self {
            visible: false,
            position: egui::Pos2::ZERO,
            target_cell: None,
            insert_rows_count: 1,
            insert_cols_count: 1,
            confirm_visible: false,
            confirm_established: false,
            confirm_action: None,
            copy_merge: false,
            copy_formula: true,
            copy_style: true,
            copy_value: true,
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
    /// 复制公式
    pub copy_formula: bool,
    /// 复制样式
    pub copy_style: bool,
    /// 复制值
    pub copy_value: bool,
    /// 保存成功提示计时（秒）
    pub save_success_timer: f32,
}

impl Default for SettingsPanelState {
    fn default() -> Self {
        let mut state = Self {
            visible: false,
            active_page: None,
            merge_col_start: 0,
            merge_col_end: 0,
            merge_col_group: 0,
            merge_row_start: 0,
            merge_row_end: 0,
            merge_row_group: 0,
            copy_formula: true,
            copy_style: false,
            copy_value: false,
            save_success_timer: 0.0,
        };
        state.load_from_file();
        state
    }
}

impl SettingsPanelState {
    /// 获取配置文件路径 ~/.MyExcel/my-excel.yaml
    fn config_path() -> std::path::PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        home.join(".MyExcel").join("my-excel.yaml")
    }

    /// 从配置文件加载 insert 块
    fn load_from_file(&mut self) {
        let path = Self::config_path();
        if !path.exists() {
            return;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&content) {
                // 读取 insert.column 节点
                if let Some(column) = doc.get("insert").and_then(|i| i.get("column")) {
                    let get_u32 = |key: &str, default: u32| -> u32 {
                        column.get(key).and_then(|v| v.as_u64()).unwrap_or(default as u64) as u32
                    };
                    self.merge_col_start = get_u32("col_start", self.merge_col_start);
                    self.merge_col_end = get_u32("col_end", self.merge_col_end);
                    self.merge_col_group = get_u32("col_group", self.merge_col_group);
                    self.merge_row_start = get_u32("row_start", self.merge_row_start);
                    self.merge_row_end = get_u32("row_end", self.merge_row_end);
                    self.merge_row_group = get_u32("row_group", self.merge_row_group);
                    self.copy_formula = column.get("copy_formula")
                        .and_then(|v| v.as_bool()).unwrap_or(true);
                    self.copy_style = column.get("copy_style")
                        .and_then(|v| v.as_bool()).unwrap_or(false);
                    self.copy_value = column.get("copy_value")
                        .and_then(|v| v.as_bool()).unwrap_or(false);
                }
            }
        }
    }

    /// 保存到 insert.column 节点（保留文件中已有的其他配置）
    pub fn save_to_file(&self) -> bool {
        let path = Self::config_path();

        // 确保目录存在
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        // 读取已有内容（保留其他配置块）
        let mut doc = if path.exists() {
            std::fs::read_to_string(&path).ok()
                .and_then(|c| serde_yaml::from_str::<serde_yaml::Value>(&c).ok())
                .unwrap_or_else(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
        } else {
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
        };

        // 构建 column 块
        let mut column = serde_yaml::Mapping::new();
        column.insert("col_start".into(), serde_yaml::Value::Number(self.merge_col_start.into()));
        column.insert("col_end".into(), serde_yaml::Value::Number(self.merge_col_end.into()));
        column.insert("col_group".into(), serde_yaml::Value::Number(self.merge_col_group.into()));
        column.insert("row_start".into(), serde_yaml::Value::Number(self.merge_row_start.into()));
        column.insert("row_end".into(), serde_yaml::Value::Number(self.merge_row_end.into()));
        column.insert("row_group".into(), serde_yaml::Value::Number(self.merge_row_group.into()));
        column.insert("copy_formula".into(), serde_yaml::Value::Bool(self.copy_formula));
        column.insert("copy_style".into(), serde_yaml::Value::Bool(self.copy_style));
        column.insert("copy_value".into(), serde_yaml::Value::Bool(self.copy_value));

        // 获取或创建 insert 节点，写入 column
        let doc_mapping = doc.as_mapping_mut().unwrap();
        if let Some(insert_val) = doc_mapping.get_mut(&serde_yaml::Value::String("insert".into())) {
            if let Some(insert_map) = insert_val.as_mapping_mut() {
                insert_map.insert("column".into(), serde_yaml::Value::Mapping(column));
            } else {
                let mut insert = serde_yaml::Mapping::new();
                insert.insert("column".into(), serde_yaml::Value::Mapping(column));
                *insert_val = serde_yaml::Value::Mapping(insert);
            }
        } else {
            let mut insert = serde_yaml::Mapping::new();
            insert.insert("column".into(), serde_yaml::Value::Mapping(column));
            doc_mapping.insert("insert".into(), serde_yaml::Value::Mapping(insert));
        }

        // 写入文件
        match serde_yaml::to_string(&doc) {
            Ok(yaml_str) => std::fs::write(&path, yaml_str).is_ok(),
            Err(_) => false,
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
    /// 撤销栈：存储可撤销操作前的快照
    undo_stack: Vec<UndoSnapshot>,
    /// 菜单栏触发的"添加列"操作标志
    pub add_column: bool,
    /// 标记当前确认弹窗由"编辑 → 添加列"触发（区别于右键菜单）
    add_column_pending: bool,
    /// 插入完成后滚动到最右列，使新列出现在可视区域
    scroll_to_last_col: bool,
    /// 菜单栏触发的"添加行"操作标志
    pub add_row: bool,
    /// 插入完成后滚动到最后一行，使新行出现在可视区域
    scroll_to_last_row: bool,
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
            undo_stack: Vec::new(),
            add_column: false,
            add_column_pending: false,
            scroll_to_last_col: false,
            add_row: false,
            scroll_to_last_row: false,
        }
    }

    /// 保存当前工作表快照到撤销栈（不借用 self，避免与 excel_data 借用冲突）
    fn push_undo(
        undo_stack: &mut Vec<UndoSnapshot>,
        sheet: &crate::excel::reader::SheetData,
        sheet_index: usize,
        selected_cell: Option<(u32, u32)>,
    ) {
        if undo_stack.len() >= MAX_UNDO_DEPTH {
            undo_stack.remove(0);
        }
        undo_stack.push(UndoSnapshot {
            sheet_data: sheet.clone(),
            sheet_index,
            selected_cell,
        });
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
                        self.undo_stack.clear();
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
    /// 每帧绘制 UI
    ///
    /// # 参数
    /// * `ui` - egui UI 上下文
    /// * `_frame` - eframe 框架
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // 设置中文字体
        setup_fonts(&ctx);

        // 绘制菜单栏
        let has_data = self.excel_data.is_some();
        egui::Panel::top("menu_bar").show_inside(ui, |ui| {
            draw_menu_bar(ui, &mut self.show_import_dialog, &mut self.settings_panel, &mut self.add_column, &mut self.add_row, has_data);
        });

        // 绘制导入对话框
        if let Some(path) = draw_import_dialog(&mut self.show_import_dialog) {
            self.start_async_load(path, ctx.clone());
        }

        // 处理"编辑 → 添加列"：复用 insert_confirm 确认弹窗流程
        // 在最后一列 (max_col, 1) 上触发"在右侧插入列"操作
        if self.add_column {
            self.add_column = false;
            if let Some(excel_data) = &self.excel_data {
                if let Some(sheet) = excel_data.get_sheet(self.current_sheet) {
                    let max_col = sheet.max_col;
                    self.context_menu.target_cell = Some((max_col, 1));
                    self.context_menu.insert_cols_count = 1;
                    self.context_menu.confirm_action = Some(ContextAction::InsertColumnRight);
                    self.context_menu.confirm_visible = true;
                    self.context_menu.confirm_established = false;
                    // 标记来自"添加列"菜单，确认后自动滚动到最右列
                    self.add_column_pending = true;
                    // 弹窗定位在屏幕中央
                    self.context_menu.position = ctx.content_rect().center();
                }
            }
        }

        // 处理"编辑 → 添加行"：在表格末尾追加一行，自动扩展公式引用范围，完成后滚动到底部
        if self.add_row {
            self.add_row = false;
            if let Some(excel_data) = &mut self.excel_data {
                if let Some(sheet) = excel_data.sheets.get_mut(self.current_sheet) {
                    // 保存撤销快照
                    Self::push_undo(&mut self.undo_stack, sheet, self.current_sheet, self.selected_cell);
                    // 在末尾追加一行，公式引用范围自动扩展
                    sheet.append_row();
                }
                crate::excel::formula::evaluate_sheet(&mut excel_data.sheets[self.current_sheet]);
                self.scroll_to_last_row = true;
            }
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
                .default_pos(ctx.content_rect().center() - egui::vec2(190.0, 80.0))
                .show(&ctx, |ui| {
                    ui.set_min_width(420.0);
                    // 自定义小字体标题栏
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(title).size(12.0).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("X").clicked() {
                                self.settings_panel.visible = false;
                            }
                            if ui.button("保存").clicked() {
                                if self.settings_panel.save_to_file() {
                                    self.settings_panel.save_success_timer = 2.0;
                                }
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
                                    ui.add_space(6.0);
                                    ui.horizontal(|ui| {
                                        ui.label("复制: 公式");
                                        ui.checkbox(&mut self.settings_panel.copy_formula, "");
                                        ui.separator();
                                        ui.label("样式");
                                        ui.checkbox(&mut self.settings_panel.copy_style, "");
                                        ui.separator();
                                        ui.label("值");
                                        ui.checkbox(&mut self.settings_panel.copy_value, "");
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
        egui::Panel::bottom("status_bar")
            .exact_size(20.0)
            .show_inside(ui, |ui| {
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
            egui::Panel::bottom("sheet_bar")
                .exact_size(28.0)
                .show_inside(ui, |ui| {
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
        egui::CentralPanel::default().show_inside(ui, |ui| {
            if let Some(excel_data) = &mut self.excel_data {
                // Ctrl+Z 撤销
                if ui.input(|i| i.key_pressed(egui::Key::Z) && i.modifiers.ctrl && !i.modifiers.shift) {
                    if let Some(snapshot) = self.undo_stack.pop() {
                        if excel_data.sheets.len() > snapshot.sheet_index {
                            excel_data.sheets[snapshot.sheet_index] = snapshot.sheet_data;
                            self.selected_cell = snapshot.selected_cell;
                            self.editing_cell = None;
                            self.edit_value.clear();
                            self.current_sheet = snapshot.sheet_index;
                            crate::excel::formula::evaluate_sheet(
                                &mut excel_data.sheets[snapshot.sheet_index],
                            );
                        }
                    }
                }

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

                        // 添加列后滚动到最右列，使新列出现在可视区域内
                        if self.scroll_to_last_col {
                            self.scroll_to_last_col = false;
                            let content_rect = ui.min_rect();
                            let right_edge = egui::Rect::from_min_max(
                                egui::Pos2::new(content_rect.max.x - 2.0, content_rect.min.y),
                                egui::Pos2::new(content_rect.max.x, content_rect.max.y),
                            );
                            ui.scroll_to_rect(right_edge, None);
                        }

                        // 添加行后滚动到最后一行，使新行出现在可视区域内
                        if self.scroll_to_last_row {
                            self.scroll_to_last_row = false;
                            let content_rect = ui.min_rect();
                            let bottom_edge = egui::Rect::from_min_max(
                                egui::Pos2::new(content_rect.min.x, content_rect.max.y - 2.0),
                                egui::Pos2::new(content_rect.max.x, content_rect.max.y),
                            );
                            ui.scroll_to_rect(bottom_edge, None);
                        }

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
                                                    self.context_menu.confirm_action = Some(ContextAction::InsertColumnLeft);
                                                    self.context_menu.confirm_visible = true;
                                                }
                                                ui.add(egui::DragValue::new(&mut self.context_menu.insert_cols_count)
                                                    .range(1..=1000)
                                                    .speed(0.1));
                                                ui.label("列");
                                            });
                                            ui.horizontal(|ui| {
                                                if ui.button("在右侧插入列").clicked() {
                                                    self.context_menu.confirm_action = Some(ContextAction::InsertColumnRight);
                                                    self.context_menu.confirm_visible = true;
                                                }
                                                ui.add(egui::DragValue::new(&mut self.context_menu.insert_cols_count)
                                                    .range(1..=1000)
                                                    .speed(0.1));
                                                ui.label("列");
                                            });

                                            ui.separator();

                                            // 清空单元格
                                            if ui.button("清空单元格").clicked() {
                                                self.context_menu.confirm_action = Some(ContextAction::ClearCell);
                                                self.context_menu.confirm_visible = true;
                                            }
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
                                    // 计算锚点：
                                    // 1. 单元格本身在合并范围内 → 使用合并边界
                                    // 2. 单元格不在合并范围，但该列属于跨列合并 → 使用列合并的边界
                                    // 3. 无合并 → 使用单元格自身坐标
                                    let (anchor_col, anchor_row) = if let Some(mr) = sheet.get_merged_range(col, row) {
                                        match action {
                                            ContextAction::InsertRowAbove => (col, mr.start_row),
                                            ContextAction::InsertRowBelow => (col, mr.end_row),
                                            ContextAction::InsertColumnLeft => (mr.start_col, row),
                                            ContextAction::InsertColumnRight => (mr.end_col, row),
                                            ContextAction::ClearCell => (col, row),
                                        }
                                    } else if let Some(cm) = sheet.get_column_merge(col) {
                                        // 该列属于跨列合并，左侧插入用合并起始列，右侧插入用合并结束列
                                        match action {
                                            ContextAction::InsertColumnLeft => (cm.start_col, row),
                                            ContextAction::InsertColumnRight => (cm.end_col, row),
                                            _ => (col, row),
                                        }
                                    } else {
                                        (col, row)
                                    };

                                    let n = self.context_menu.insert_rows_count;
                                    let m = self.context_menu.insert_cols_count;

                                    // 保存撤销快照
                                    Self::push_undo(&mut self.undo_stack, sheet, self.current_sheet, self.selected_cell);

                                    let default_options = crate::excel::reader::ColumnCopyOptions::default();
                                    match action {
                                        ContextAction::InsertRowAbove => {
                                            sheet.insert_rows(anchor_row, n, false);
                                        }
                                        ContextAction::InsertRowBelow => {
                                            sheet.insert_rows(anchor_row, n, true);
                                        }
                                        ContextAction::InsertColumnLeft => {
                                            sheet.insert_columns(anchor_col, m, false, default_options);
                                        }
                                        ContextAction::InsertColumnRight => {
                                            sheet.insert_columns(anchor_col, m, true, default_options);
                                        }
                                        ContextAction::ClearCell => {
                                            // 清空走确认弹窗路径，这里不应到达
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

                    // 绘制插入列确认弹窗
                    if self.context_menu.confirm_visible {
                        // 首帧标记为已建立，后续帧才检测外部点击
                        let is_established = self.context_menu.confirm_established;
                        self.context_menu.confirm_established = true;

                        let confirm_action = self.context_menu.confirm_action;
                        let mut confirm_execute = false;
                        let mut cancel_clicked = false;
                        let mut keep_open = true;

                        // 根据操作类型显示不同的确认弹窗
                        if confirm_action == Some(ContextAction::ClearCell) {
                            // 清空单元格确认弹窗
                            egui::Window::new("clear_confirm")
                                .title_bar(false)
                                .open(&mut keep_open)
                                .resizable(false)
                                .collapsible(false)
                                .order(egui::Order::Foreground)
                                .fixed_pos(self.context_menu.position)
                                .show(ui.ctx(), |ui| {
                                    ui.set_min_width(260.0);
                                    ui.vertical_centered(|ui| {
                                        ui.label(egui::RichText::new("确定清空该单元格的内容？").size(13.0));
                                    });
                                    ui.add_space(8.0);
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui.button("确认").clicked() {
                                            confirm_execute = true;
                                        }
                                        if ui.button("取消").clicked() {
                                            cancel_clicked = true;
                                        }
                                    });
                                });
                        } else {
                            // 插入列确认弹窗（保留原有逻辑）
                            if let Some((col, _row)) = self.context_menu.target_cell {
                                if let Some(sheet) = excel_data.get_sheet(self.current_sheet) {
                                    self.context_menu.copy_merge = sheet.get_column_merge(col).is_some();
                                }
                            }

                            egui::Window::new("insert_confirm")
                                .title_bar(false)
                                .open(&mut keep_open)
                                .resizable(false)
                                .collapsible(false)
                                .order(egui::Order::Foreground)
                                .fixed_pos(self.context_menu.position)
                                .show(ui.ctx(), |ui| {
                                    ui.set_min_width(360.0);
                                    ui.set_height(50.0);
                                    // 复制选项
                                    ui.horizontal(|ui| {
                                        ui.label("复制合并:");
                                        ui.checkbox(&mut self.context_menu.copy_merge, "");
                                        ui.separator();
                                        ui.label("公式:");
                                        ui.checkbox(&mut self.context_menu.copy_formula, "");
                                        ui.separator();
                                        ui.label("样式:");
                                        ui.checkbox(&mut self.context_menu.copy_style, "");
                                        ui.separator();
                                        ui.label("值:");
                                        ui.checkbox(&mut self.context_menu.copy_value, "");
                                    });
                                    ui.separator();
                                    // 右下角按钮：取消（左） + 确认（右）
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui.button("确认").clicked() {
                                            confirm_execute = true;
                                        }
                                        if ui.button("取消").clicked() {
                                            cancel_clicked = true;
                                        }
                                    });
                                });
                        }

                        if !keep_open || cancel_clicked {
                            self.context_menu.confirm_visible = false;
                            self.context_menu.confirm_established = false;
                            self.context_menu.confirm_action = None;
                            self.add_column_pending = false;
                        }

                        // 点击弹窗外部关闭（仅当弹窗已建立后检测，避免首帧误关）
                        if is_established {
                            let confirm_id = egui::Id::new("insert_confirm");
                            let confirm_area = ui.ctx().memory(|mem| {
                                mem.area_rect(confirm_id)
                            });
                            if let Some(confirm_rect) = confirm_area {
                                if ui.input(|i| i.pointer.any_click()) {
                                    if let Some(hover) = ui.input(|i| i.pointer.hover_pos()) {
                                        if !confirm_rect.contains(hover) {
                                            self.context_menu.confirm_visible = false;
                                            self.context_menu.confirm_established = false;
                                            self.context_menu.confirm_action = None;
                                            self.add_column_pending = false;
                                        }
                                    }
                                }
                            }
                        }

                        if confirm_execute {
                            if let Some(action) = confirm_action {
                                if let Some((col, row)) = self.context_menu.target_cell {
                                    self.editing_cell = None;
                                    self.edit_value.clear();
                                    self.original_cell_data = None;
                                    self.validation_error = None;
                                    self.validation_error_pos = None;

                                    if let Some(sheet) = excel_data.sheets.get_mut(self.current_sheet) {
                                        match action {
                                            ContextAction::ClearCell => {
                                                // 清空单元格的值和公式（保留样式）
                                                if let Some(cell) = sheet.cells.get_mut(&(row, col)) {
                                                    cell.value.clear();
                                                    cell.formula.clear();
                                                }
                                                // 清空后触发依赖公式重算
                                                crate::excel::formula::evaluate_dependents(&mut excel_data.sheets[self.current_sheet], row, col);
                                            }
                                            _ => {
                                                // 插入列逻辑
                                                let (anchor_col, _anchor_row) = if let Some(mr) = sheet.get_merged_range(col, row) {
                                                    match action {
                                                        ContextAction::InsertColumnLeft => (mr.start_col, row),
                                                        ContextAction::InsertColumnRight => (mr.end_col, row),
                                                        _ => (col, row),
                                                    }
                                                } else if let Some(cm) = sheet.get_column_merge(col) {
                                                    match action {
                                                        ContextAction::InsertColumnLeft => (cm.start_col, row),
                                                        ContextAction::InsertColumnRight => (cm.end_col, row),
                                                        _ => (col, row),
                                                    }
                                                } else {
                                                    (col, row)
                                                };

                                                let mut m = self.context_menu.insert_cols_count;
                                                // 如果列属于跨列合并，自动将 m 设为合并宽度
                                                if let Some(cm) = sheet.get_column_merge(col) {
                                                    let merge_width = cm.end_col - cm.start_col + 1;
                                                    if m < merge_width {
                                                        m = merge_width;
                                                    }
                                                }

                                                // 保存撤销快照
                                                Self::push_undo(&mut self.undo_stack, sheet, self.current_sheet, self.selected_cell);

                                                let copy_options = crate::excel::reader::ColumnCopyOptions::new(
                                                    self.context_menu.copy_merge,
                                                    self.context_menu.copy_formula,
                                                    self.context_menu.copy_style,
                                                    self.context_menu.copy_value,
                                                );
                                                match action {
                                                    ContextAction::InsertColumnLeft => {
                                                        sheet.insert_columns(anchor_col, m, false, copy_options);
                                                    }
                                                    ContextAction::InsertColumnRight => {
                                                        sheet.insert_columns(anchor_col, m, true, copy_options);
                                                    }
                                                    _ => {}
                                                }
                                                crate::excel::formula::evaluate_sheet(&mut excel_data.sheets[self.current_sheet]);
                                            }
                                        }
                                    }
                                }
                            }
                            self.context_menu.confirm_visible = false;
                            self.context_menu.confirm_established = false;
                            self.context_menu.confirm_action = None;
                            self.context_menu.visible = false;
                            // 由"编辑 → 添加列"触发时，标记需要滚动到最右列
                            if self.add_column_pending {
                                self.add_column_pending = false;
                                self.scroll_to_last_col = true;
                            }
                        }
                    }

                // 处理公式栏的待保存值
                if let Some(formula_value) = self.pending_formula_save.take() {
                    if let Some((col, row)) = self.selected_cell {
                        // 保存撤销快照
                        if let Some(sheet) = excel_data.sheets.get(self.current_sheet) {
                            Self::push_undo(&mut self.undo_stack, sheet, self.current_sheet, self.selected_cell);
                        }
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

