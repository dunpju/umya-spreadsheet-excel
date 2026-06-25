//! Excel 查看器主模块
//! 
//! 整合所有子模块，提供完整的 Excel 查看功能

use eframe::egui;
use crate::excel::reader::ExcelData;
use crate::gui::state::LoadState;
use crate::gui::fonts::setup_fonts;
use crate::license::{time as lic_time, LicenseManager, LicenseStatus};
use crate::gui::widgets::{
    draw_menu_bar,
    draw_import_dialog,
    draw_table_content,
    draw_empty_state,
    draw_name_box,
    draw_alert_popup,
    draw_cond_format_popup,
    draw_convert_popup,
    draw_help_popup,
    draw_alert_notify_popup,
    draw_license_popup,
    check_alert_rules,
    update_alert_range_expansions_for_col,
    update_alert_range_expansions_for_row,
    AlertPopupState,
    CondFormatPopupState,
    ConvertPopupState,
    HelpPopupState,
    NameBoxState,
    SearchWindowState,
    AlertNotifyState,
    LicensePopupState,
    draw_search_window,
    // 配置模块（gui/widgets/config.rs）
    SettingsPanelState,
    draw_settings_panel,
    draw_search_config_dialog,
};
use std::collections::HashSet;
use std::sync::mpsc::Receiver;

/// 撤销栈最大深度
const MAX_UNDO_DEPTH: usize = 20;

/// 撤销操作：支持全量快照、单单元格和范围清空三种粒度
enum UndoAction {
    /// 全量快照：用于插入行/列等结构性操作
    FullSnapshot {
        sheet_data: crate::excel::reader::SheetData,
        sheet_index: usize,
    },
    /// 单单元格变更：用于清空、编辑等单格操作
    CellChange {
        sheet_index: usize,
        row: u32,
        col: u32,
        old_cell: Option<crate::excel::reader::CellData>,
        old_selected: Option<(u32, u32)>,
    },
    /// 范围清空：保存范围内所有单元格原始数据
    RangeClear {
        sheet_index: usize,
        old_cells: Vec<(u32, u32, Option<crate::excel::reader::CellData>)>,
        old_selected: Option<(u32, u32)>,
        old_range: Option<(u32, u32, u32, u32)>,
    },
}

/// 填充柄（Fill Handle）拖拽提交信号：由 `draw_table_content` 在一次成功填充后写入出参 `committed_fill`，
/// 调用方据此把填充入撤销栈（构造为 `UndoAction::RangeClear`，复用其回放：恢复 old_cells + 选区）。
#[derive(Clone, Default)]
pub struct FillCommit {
    /// 被覆盖目标格的原始数据 `(row, col, 旧值)`
    pub old_cells: Vec<(u32, u32, Option<crate::excel::reader::CellData>)>,
    /// 填充前的活动单元格
    pub old_selected: Option<(u32, u32)>,
    /// 填充前的选中范围
    pub old_range: Option<(u32, u32, u32, u32)>,
}

/// 粘贴提交信号：由 `draw_table_content` 在一次成功粘贴后写入出参 `committed_paste`，
/// 调用方据此把粘贴覆盖的旧数据入撤销栈（构造为 `UndoAction::RangeClear`）。
#[derive(Clone, Default)]
pub struct PasteCommit {
    /// 被覆盖目标格的原始数据 `(row, col, 旧值)`
    pub old_cells: Vec<(u32, u32, Option<crate::excel::reader::CellData>)>,
    /// 粘贴前的活动单元格
    pub old_selected: Option<(u32, u32)>,
    /// 粘贴前的选中范围
    pub old_range: Option<(u32, u32, u32, u32)>,
}

/// 分批跨帧填充状态（仅在目标格数 > `FILL_SYNC_THRESHOLD` 时激活）。
///
/// 双击填充柄的目标范围可能很大（数千~数万格），单帧同步写入会导致 UI 卡顿。
/// `PendingFill` 将预计算的填充值分批写入：每帧写入 `FILL_BATCH_SIZE` 格，
/// 帧间 UI 正常响应，填充完成后统一触发公式重算 + 选区更新 + 撤销入栈。
pub struct PendingFill {
    /// 预计算的待写入值 `(row, col, new_cell_data)`
    pub values: Vec<(u32, u32, crate::excel::reader::CellData)>,
    /// 下一个待写入的索引
    pub next_idx: usize,
    /// 目标中是否含公式填充（决定重算策略：公式→`evaluate_sheet`，仅值→`evaluate_dependents_many`）
    pub has_formula: bool,
    /// 累积的旧单元格数据（撤销用）
    pub old_cells: Vec<(u32, u32, Option<crate::excel::reader::CellData>)>,
    /// 填充前的活动单元格（撤销用）
    pub prev_selected: Option<(u32, u32)>,
    /// 填充前的选中范围（撤销用）
    pub prev_range: Option<(u32, u32, u32, u32)>,
    /// 源选区 `(start_col, start_row, end_col, end_row)`（填充完成后更新选区用）
    pub src: (u32, u32, u32, u32),
    /// 目标格 `(col, row)`（填充完成后更新选区用）
    pub target: (u32, u32),
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
    /// 向下选中行数（0=选到边界）
    pub select_down_count: u32,
    /// 向上选中行数（0=选到边界）
    pub select_up_count: u32,
    /// 向左选中列数（0=选到边界）
    pub select_left_count: u32,
    /// 向右选中列数（0=选到边界）
    pub select_right_count: u32,
    /// 确认弹窗是否可见
    pub confirm_visible: bool,
    /// 确认弹窗是否已显示超过一帧（用于跳过首帧外部点击检测）
    pub confirm_established: bool,
    /// 确认弹窗对应的操作
    pub confirm_action: Option<ContextAction>,
    /// 清空操作是否针对选中范围（true=范围清空，false=单格清空）
    pub clear_is_range: bool,
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
    SelectDown,
    SelectUp,
    SelectLeft,
    SelectRight,
}

impl Default for ContextMenuState {
    fn default() -> Self {
        Self {
            visible: false,
            position: egui::Pos2::ZERO,
            target_cell: None,
            insert_rows_count: 1,
            insert_cols_count: 1,
            select_down_count: 0,
            select_up_count: 0,
            select_left_count: 0,
            select_right_count: 0,
            confirm_visible: false,
            confirm_established: false,
            confirm_action: None,
            clear_is_range: false,
            copy_merge: false,
            copy_formula: true,
            copy_style: true,
            copy_value: true,
        }
    }
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
    /// 选中范围：Some((start_col, start_row, end_col, end_row))，None 表示仅单格选中
    pub selected_range: Option<(u32, u32, u32, u32)>,
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
    undo_stack: Vec<UndoAction>,
    /// 菜单栏触发的"添加列"操作标志
    pub add_column: bool,
    /// 标记当前确认弹窗由"编辑 → 添加列"触发（区别于右键菜单）
    add_column_pending: bool,
    /// 拖拽选择锚点（鼠标按下时的单元格），None 表示未在拖拽
    pub drag_anchor: Option<(u32, u32)>,
    /// 填充柄拖拽源锚点（按下填充柄时的选区右下角格），None 表示未在填充拖拽
    pub fill_drag_source: Option<(u32, u32)>,
    /// Shift+点击选择锚点（最后一次非 Shift 点击/键盘导航的单元格坐标），用于 Shift+点击范围选择
    pub shift_click_anchor: Option<(u32, u32)>,
    /// 插入完成后滚动到最右列，使新列出现在可视区域
    scroll_to_last_col: bool,
    /// 菜单栏触发的"添加行"操作标志
    pub add_row: bool,
    /// 插入完成后滚动到最后一行，使新行出现在可视区域
    scroll_to_last_row: bool,
    /// 是否有未保存的单元格变更
    pub dirty: bool,
    /// 是否正在保存中（用于显示 loading 动画）
    saving: bool,
    /// 最近一次保存的文件路径（用于状态栏显示）
    save_path: Option<String>,
    /// 异步保存的通道接收器
    save_rx: Option<Receiver<Result<String, String>>>,
    /// 保存请求标志（用于延迟到 excel_data 借用释放后执行）
    save_requested: bool,
    /// 当前在途保存的输出路径（供失败时拼装"文件被占用"提示文案）
    pending_save_path: Option<String>,
    /// 保存失败红色提示框文案（Some 时弹出居中红色提示，关闭后置 None）
    save_failed: Option<String>,
    /// 搜索窗口状态
    pub search_window: SearchWindowState,
    /// 转换弹窗状态
    pub convert_popup: ConvertPopupState,
    /// 预警消息弹窗状态
    pub alert_popup: AlertPopupState,
    /// 条件格式弹窗状态
    pub cond_format_popup: CondFormatPopupState,
    /// 帮助弹窗状态
    pub help_popup: HelpPopupState,
    /// 隐藏的列号集合（1-based），由搜索功能写入，table 渲染时读取
    pub hidden_columns: HashSet<u32>,
    /// 隐藏的行号集合（1-based），由行筛选功能写入，table 渲染时读取
    pub hidden_rows: HashSet<u32>,
    /// 分批跨帧填充状态（Some = 正在逐帧写入填充值）
    pending_fill: Option<PendingFill>,
    /// 预警通知状态（图标 + 弹窗）
    pub alert_notify_state: AlertNotifyState,
    /// 授权管理器（试用/激活状态）
    pub license: LicenseManager,
    /// 授权 / 付款弹窗状态
    pub license_popup: LicensePopupState,
}

impl ExcelViewer {
    /// 创建新的 Excel 查看器实例，初始化所有状态
    pub fn new() -> Self {
        let license = LicenseManager::load();
        let blocking = license.status(lic_time::today_epoch_day()).is_blocking();
        let machine_code = license.machine_code().to_string();
        Self {
            excel_data: None,
            current_sheet: 0,
            error_message: None,
            selected_cell: None,
            selected_range: None,
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
            dirty: false,
            saving: false,
            save_path: None,
            save_rx: None,
            save_requested: false,
            pending_save_path: None,
            save_failed: None,
            drag_anchor: None,
            fill_drag_source: None,
            shift_click_anchor: None,
            search_window: SearchWindowState::default(),
            convert_popup: ConvertPopupState::default(),
            alert_popup: AlertPopupState::load_from_file(),
            cond_format_popup: CondFormatPopupState::load_from_file(),
            help_popup: HelpPopupState::default(),
            hidden_columns: HashSet::new(),
            hidden_rows: HashSet::new(),
            pending_fill: None,
            alert_notify_state: AlertNotifyState::default(),
            license,
            license_popup: LicensePopupState {
                visible: blocking,
                machine_code,
                ..Default::default()
            },
        }
    }

    /// 保存当前工作表快照到撤销栈（不借用 self，避免与 excel_data 借用冲突）
    /// 推入全量快照撤销（用于插入行/列等结构性操作）
    fn push_undo_full(
        undo_stack: &mut Vec<UndoAction>,
        sheet: &crate::excel::reader::SheetData,
        sheet_index: usize,
    ) {
        if undo_stack.len() >= MAX_UNDO_DEPTH {
            undo_stack.remove(0);
        }
        undo_stack.push(UndoAction::FullSnapshot {
            sheet_data: sheet.clone(),
            sheet_index,
        });
    }

    /// 推入单单元格撤销（用于清空、编辑等单格操作）
    fn push_undo_cell(
        undo_stack: &mut Vec<UndoAction>,
        sheet_index: usize,
        row: u32,
        col: u32,
        sheet: &crate::excel::reader::SheetData,
        selected_cell: Option<(u32, u32)>,
    ) {
        if undo_stack.len() >= MAX_UNDO_DEPTH {
            undo_stack.remove(0);
        }
        let old_cell = sheet.cells.get(&(row, col)).cloned();
        undo_stack.push(UndoAction::CellChange {
            sheet_index,
            row,
            col,
            old_cell,
            old_selected: selected_cell,
        });
    }

    /// 保存范围内所有单元格的撤销快照
    fn push_undo_range(
        undo_stack: &mut Vec<UndoAction>,
        sheet_index: usize,
        start_col: u32,
        start_row: u32,
        end_col: u32,
        end_row: u32,
        sheet: &crate::excel::reader::SheetData,
        selected_cell: Option<(u32, u32)>,
        selected_range: Option<(u32, u32, u32, u32)>,
    ) {
        if undo_stack.len() >= MAX_UNDO_DEPTH {
            undo_stack.remove(0);
        }
        let mut old_cells = Vec::new();
        for r in start_row..=end_row {
            for c in start_col..=end_col {
                let old = sheet.cells.get(&(r, c)).cloned();
                old_cells.push((r, c, old));
            }
        }
        undo_stack.push(UndoAction::RangeClear {
            sheet_index,
            old_cells,
            old_selected: selected_cell,
            old_range: selected_range,
        });
    }

    /// 从撤销栈取出一个操作
    fn take_undo(&mut self) -> Option<UndoAction> {
        self.undo_stack.pop()
    }

    /// 启动异步加载 Excel 文件
    ///
    /// 导入入口：在后台线程中**先把用户所选文件备份**到 `~/.MyExcel/backup/`（命名
    /// `原文件名_yyyymmddhhmmss.ext`，目录不存在则递归创建），再读取解析文件。
    /// 备份与加载都放后台线程，避免阻塞 UI；备份为附加功能，失败仅记日志，不阻断加载。
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

        // 启动后台线程：先备份用户所选文件到 ~/.MyExcel/backup/（原文件名_yyyymmddhhmmss.ext），
        // 再读取解析。两者均放后台线程，避免阻塞 UI；备份失败仅记日志，不阻断加载流程。
        std::thread::spawn(move || {
            if let Err(e) = crate::util::backup::backup_imported_file(std::path::Path::new(&path)) {
                log::warn!("备份导入文件失败: {e}");
            }
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
                        self.selected_range = None;
                        self.editing_cell = None;
                        self.edit_value.clear();
                        self.pending_formula_save = None;
                        self.hovered_cell = None;
                        self.error_message = None;
                        self.undo_stack.clear();
                        self.dirty = false;
                        self.saving = false;
                        self.save_path = None;
                        self.save_rx = None;
                        self.hidden_columns.clear();
                        self.hidden_rows.clear();
                        self.search_window.options_loaded = false;
                        // 应用用户自定义条件格式
                        let user_rules = self.cond_format_popup.rules.clone();
                        if let Some(ref mut excel) = self.excel_data {
                            for sheet in &mut excel.sheets {
                                ExcelData::apply_user_cond_format_rules(sheet, &user_rules);
                            }
                        }
                        // 重置预警通知状态
                        self.alert_notify_state = AlertNotifyState::default();
                        // 导入完成后自动检测预警规则，若有触发则自动弹出预警消息弹窗
                        if !self.alert_popup.rules.is_empty() {
                            if let Some(ref excel_data) = self.excel_data {
                                if let Some(sheet) = excel_data.get_sheet(self.current_sheet) {
                                    let triggered = check_alert_rules(&self.alert_popup.rules, sheet);
                                    if !triggered.is_empty() {
                                        self.alert_notify_state.has_triggered = true;
                                        self.alert_notify_state.triggered_rules = triggered;
                                        self.alert_notify_state.visible = true;
                                    }
                                }
                            }
                        }
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

    /// 启动异步保存 Excel 文件
    ///
    /// 直接**覆盖写入**当前已打开文件的原始路径（`self.file_path`），不再生成带日期后缀的
    /// 新文件。`writer::save_to_file` 内部会先把原文件完整读入内存（`reader::xlsx::read`），
    /// 再应用变更并写回——故输出路径等于原始路径时也不会损坏数据。
    fn start_async_save(&mut self, ctx: egui::Context) {
        // 授权拦截：试用到期/未激活时禁止保存（校验点分散到核心功能）
        if self.license.status(lic_time::today_epoch_day()).is_blocking() {
            self.license_popup.visible = true;
            return;
        }
        let original_path = match &self.file_path {
            Some(p) => p.clone(),
            None => return,
        };
        let excel_data = match &self.excel_data {
            Some(d) => d.clone(),
            None => return,
        };
        // 输出路径 = 原文件路径：直接覆盖原文件（原文件先被完整读入内存后再回写）
        let output_path = original_path.clone();

        self.saving = true;
        // 记录在途保存的输出路径，供失败时拼装"文件被占用"提示文案
        self.pending_save_path = Some(output_path.clone());
        let (tx, rx) = std::sync::mpsc::channel();
        self.save_rx = Some(rx);

        std::thread::spawn(move || {
            let result = crate::excel::writer::save_to_file(&original_path, &excel_data, &output_path);
            match result {
                Ok(()) => {
                    let _ = tx.send(Ok(output_path));
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            }
            ctx.request_repaint();
        });
    }

    /// 检查异步保存结果
    fn check_save_result(&mut self) {
        if let Some(ref rx) = self.save_rx {
            if let Ok(result) = rx.try_recv() {
                // 取走在途保存的输出路径：成功则忽略，失败则用于拼装提示文案
                let pending_path = self.pending_save_path.take();
                match result {
                    Ok(path) => {
                        self.save_path = Some(path);
                        self.dirty = false;
                        self.save_failed = None; // 重试成功，清除失败提示框
                    }
                    Err(e) => {
                        self.error_message = Some(e);
                        // 弹出居中红色提示框（非状态栏文字）：文件可能被占用打开。
                        // 覆盖两种触发方式（点"保存"按钮 / Ctrl+S）——二者都汇入此函数。
                        if let Some(p) = pending_path {
                            self.save_failed = Some(format!(
                                "保存失败!请检查{}文件是否被占用打开",
                                p
                            ));
                        }
                    }
                }
                self.saving = false;
                self.save_rx = None;
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

        // 授权状态（每帧计算一次，供状态栏徽标与帧末拦截复用）
        let lic_today = lic_time::today_epoch_day();
        let lic_status = self.license.status(lic_today);
        if lic_status.is_blocking() {
            self.license_popup.visible = true;
            // 真模态：全屏遮罩屏蔽所有主界面交互，仅允许激活弹窗操作
            egui::CentralPanel::default().show_inside(ui, |ui| {
                ui.painter().rect_filled(
                    ui.max_rect(),
                    0.0,
                    egui::Color32::from_black_alpha(200),
                );
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() * 0.3);
                    ui.label(
                        egui::RichText::new("请激活后继续使用")
                            .size(18.0)
                            .color(egui::Color32::GRAY),
                    );
                });
            });
        } else {

        // 绘制菜单栏
        let has_data = self.excel_data.is_some();
        egui::Panel::top("menu_bar").show_inside(ui, |ui| {
            draw_menu_bar(ui, &mut self.show_import_dialog, &mut self.settings_panel, &mut self.search_window, &mut self.add_column, &mut self.add_row, has_data, &mut self.convert_popup, &mut self.alert_popup, &mut self.cond_format_popup, &mut self.help_popup, &mut self.alert_notify_state, &mut self.license_popup, &lic_status);
        });

        // 绘制导入对话框
        if let Some(path) = draw_import_dialog(&mut self.show_import_dialog) {
            self.start_async_load(path, ctx.clone());
        }

        // 绘制帮助弹窗
        draw_help_popup(&ctx, &mut self.help_popup);

        // 绘制预警消息弹窗
        draw_alert_popup(&ctx, &mut self.alert_popup);

        // 检查预警规则是否被触发（每帧检测，数据变化后自动更新）
        if let Some(ref excel_data) = self.excel_data {
            if let Some(sheet) = excel_data.get_sheet(self.current_sheet) {
                let triggered = check_alert_rules(&self.alert_popup.rules, sheet);
                self.alert_notify_state.has_triggered = !triggered.is_empty();
                self.alert_notify_state.triggered_rules = triggered;
            }
        }

        // 绘制预警通知弹窗
        draw_alert_notify_popup(
            &ctx,
            &mut self.alert_notify_state,
            &mut self.hidden_columns,
            &mut self.hidden_rows,
            self.excel_data.as_ref().and_then(|ed| ed.get_sheet(self.current_sheet)),
        );

        // 快照用户条件格式规则，用于检测本轮是否被编辑（弹窗内增删/修改）
        let user_rules_before = self.cond_format_popup.rules.clone();

        // 绘制条件格式弹窗
        draw_cond_format_popup(&ctx, &mut self.cond_format_popup);

        // 条件格式事件驱动：仅在「当前表标记为脏」或「用户规则变化」时重算，
        // 替代原先每帧对所有表重算。cf_dirty 由公式求值（单元格值变化）置位；
        // 加载时 load_from_file 内部调用 evaluate_sheet 已将各表置为脏，首帧会应用用户规则。
        if let Some(ref mut excel) = self.excel_data {
            if self.cond_format_popup.rules != user_rules_before {
                for sheet in &mut excel.sheets {
                    sheet.cf_dirty = true;
                }
            }
            if let Some(sheet) = excel.sheets.get_mut(self.current_sheet) {
                if sheet.cf_dirty {
                    ExcelData::reapply_conditional_formatting(sheet);
                    if !self.cond_format_popup.rules.is_empty() {
                        let user_rules = self.cond_format_popup.rules.clone();
                        ExcelData::apply_user_cond_format_rules(sheet, &user_rules);
                    }
                    sheet.cf_dirty = false;
                }
            }
        }

        // 绘制转换弹窗
        draw_convert_popup(
            &ctx,
            &mut self.convert_popup,
            self.excel_data.as_ref(),
            self.file_path.as_deref(),
            self.current_sheet,
        );

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
                    // 保存撤销快照（全量：追加行是结构性操作）
                    Self::push_undo_full(&mut self.undo_stack, sheet, self.current_sheet);
                    // 在末尾追加一行，公式引用范围自动扩展
                    sheet.append_row();
                    self.dirty = true;
                    // 更新预警规则固定范围的行扩展偏移量
                    let new_row = sheet.max_row;
                    update_alert_range_expansions_for_row(&mut self.alert_popup.rules, new_row, 1, sheet);
                }
                crate::excel::formula::evaluate_sheet(&mut excel_data.sheets[self.current_sheet]);
                self.scroll_to_last_row = true;
            }
        }

        // 绘制设置面板（配置模块，详见 gui/widgets/config.rs）
        draw_settings_panel(&ctx, &mut self.settings_panel);

        // 绘制搜索配置对话框（配置模块，详见 gui/widgets/config.rs）
        draw_search_config_dialog(&ctx, &mut self.settings_panel);

        // 绘制搜索窗口（非模态，独立于主窗口）
        {
            let excel_data_ref = self.excel_data.as_ref();
            draw_search_window(
                &ctx,
                &mut self.search_window,
                excel_data_ref,
                self.current_sheet,
                &mut self.hidden_columns,
                &mut self.hidden_rows,
            );
        }

        // 检查异步加载结果
        self.check_load_result();

        // 检查异步保存结果
        self.check_save_result();

        // 保存中持续请求重绘（驱动 loading 动画）
        if self.saving {
            ctx.request_repaint();
        }

        // Ctrl+S 保存快捷键
        if ui.input(|i| i.key_pressed(egui::Key::S) && i.modifiers.ctrl) {
            if self.dirty && !self.saving && self.excel_data.is_some() {
                self.start_async_save(ctx.clone());
            }
        }

        // 保存失败红色提示框（文件被占用等）：居中 Foreground 浮窗，区别于状态栏文字。
        // 两种触发方式（点"保存"按钮 / Ctrl+S）都汇入 start_async_save → check_save_result，
        // 失败时由 check_save_result 置位 self.save_failed，这里据此渲染。
        if let Some(msg) = self.save_failed.clone() {
            egui::Window::new("save_failed")
                .title_bar(false)
                .resizable(false)
                .collapsible(false)
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(&ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .fill(egui::Color32::from_rgb(253, 236, 236))
                        .stroke(egui::Stroke::new(2.0, egui::Color32::from_rgb(220, 53, 69)))
                        .show(ui, |ui| {
                            ui.set_min_width(360.0);
                            ui.horizontal_top(|ui| {
                                ui.label(
                                    egui::RichText::new("✖")
                                        .color(egui::Color32::from_rgb(220, 53, 69))
                                        .size(18.0),
                                );
                                ui.label(
                                    egui::RichText::new(&msg)
                                        .color(egui::Color32::from_rgb(190, 0, 0))
                                        .size(14.0)
                                        .strong(),
                                );
                            });
                            ui.add_space(10.0);
                            ui.vertical_centered(|ui| {
                                if ui.button("知道了").clicked() {
                                    self.save_failed = None;
                                }
                            });
                        });
                });
        }

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
                    // 右侧：保存路径 + loading 动画
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(6.0);
                        if self.saving {
                            // 保存中：显示 loading 动画 + 临时文本
                            ui.spinner();
                            ui.label(
                                egui::RichText::new("正在保存...")
                                    .font(egui::FontId::proportional(12.0))
                                    .color(egui::Color32::from_rgb(0, 150, 0)),
                            );
                        } else if let Some(save_path) = &self.save_path {
                            // 保存完成：显示绿色文件路径（可点击 → 用系统默认程序打开）
                            let path = save_path.clone(); // 闭包外取出，避开 self 借用
                            let resp = ui.add(
                                egui::Label::new(
                                    egui::RichText::new(save_path.as_str())
                                        .font(egui::FontId::proportional(12.0))
                                        .color(egui::Color32::from_rgb(0, 150, 0)),
                                )
                                .sense(egui::Sense::click()),
                            );
                            // on_hover_* 消费 Response 并返回 Self，需链式重绑定
                            let resp = resp
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text("点击用系统默认程序打开");
                            if resp.clicked() {
                                if let Err(e) =
                                    crate::util::open::open_in_default_app(std::path::Path::new(&path))
                                {
                                    log::warn!("打开文件失败: {e}");
                                }
                            }
                        }
                    });
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
                                self.selected_range = None;
                                // 切换工作表时重置搜索状态
                                self.hidden_columns.clear();
                                self.hidden_rows.clear();
                                self.search_window.options_loaded = false;
                                // 切换工作表时重置预警通知过滤状态
                                self.alert_notify_state.is_filtering = false;
                                self.alert_notify_state.visible = false;
                                self.alert_notify_state.triggered_rules.clear();
                                self.alert_notify_state.has_triggered = false;
                            }
                        }
                    });
                });
        }

        // 主内容区域
        egui::CentralPanel::default().show_inside(ui, |ui| {
            // Ctrl+Z 撤销：在借用 excel_data 之前取出 undo action
            // 编辑模式下不触发单元格级撤销：把 Ctrl+Z 留给输入框做文本内撤销，
            // 并避免编辑时弹出栈中无关的撤销动作（如上一次清空）干扰当前编辑
            let pending_undo = (ui.input(|i| i.key_pressed(egui::Key::Z) && i.modifiers.ctrl && !i.modifiers.shift)
                && self.editing_cell.is_none())
                .then(|| self.take_undo()).flatten();

            if let Some(excel_data) = &mut self.excel_data {
                // 应用撤销
                if let Some(action) = pending_undo {
                    match action {
                        UndoAction::FullSnapshot { sheet_data, sheet_index } => {
                            if excel_data.sheets.len() > sheet_index {
                                excel_data.sheets[sheet_index] = sheet_data;
                                self.selected_cell = None;
                                self.selected_range = None;
                                self.editing_cell = None;
                                self.edit_value.clear();
                                self.current_sheet = sheet_index;
                                crate::excel::formula::evaluate_sheet(&mut excel_data.sheets[sheet_index]);
                            }
                        }
                        UndoAction::CellChange { sheet_index, row, col, old_cell, old_selected } => {
                            if excel_data.sheets.len() > sheet_index {
                                // 恢复前记录当前格公式状态（用于精确维护 formula_positions）
                                let current_had_formula = excel_data.sheets[sheet_index]
                                    .get_cell(row, col)
                                    .map_or(false, |c| !c.formula.is_empty());
                                let old_had_formula = old_cell.as_ref().map_or(false, |c| !c.formula.is_empty());

                                if let Some(old) = old_cell {
                                    excel_data.sheets[sheet_index].cells.insert((row, col), old);
                                } else {
                                    excel_data.sheets[sheet_index].cells.remove(&(row, col));
                                }

                                // 精确维护 formula_positions 索引
                                if old_had_formula && !current_had_formula {
                                    excel_data.sheets[sheet_index].mark_formula(row, col);
                                } else if !old_had_formula && current_had_formula {
                                    excel_data.sheets[sheet_index].unmark_formula(row, col);
                                }

                                self.selected_cell = old_selected;
                                self.selected_range = None;
                                self.editing_cell = None;
                                self.edit_value.clear();
                                self.current_sheet = sheet_index;
                                crate::excel::formula::evaluate_dependents(&mut excel_data.sheets[sheet_index], row, col);
                            }
                        }
                        UndoAction::RangeClear { sheet_index, old_cells, old_selected, old_range } => {
                            if excel_data.sheets.len() > sheet_index {
                                // 恢复旧单元格（可能含公式文本）→ 依赖图缓存失效
                                crate::excel::formula::invalidate_formula_graph(&mut excel_data.sheets[sheet_index]);
                                for (r, c, old) in old_cells {
                                    if let Some(cell) = old {
                                        // 恢复含公式的旧格 → 重新标记 formula_positions
                                        if !cell.formula.is_empty() {
                                            excel_data.sheets[sheet_index].mark_formula(r, c);
                                        }
                                        excel_data.sheets[sheet_index].cells.insert((r, c), cell);
                                    } else {
                                        excel_data.sheets[sheet_index].cells.remove(&(r, c));
                                    }
                                }
                                self.selected_cell = old_selected;
                                self.selected_range = old_range;
                                self.editing_cell = None;
                                self.edit_value.clear();
                                self.current_sheet = sheet_index;
                                crate::excel::formula::evaluate_sheet(&mut excel_data.sheets[sheet_index]);
                            }
                        }
                    }
                    // 撤销操作也视为数据变更
                    self.dirty = true;
                }

                // Delete 键清空单元格（有内容时才弹窗确认）
                if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                    if self.selected_cell.is_some() && self.editing_cell.is_none() && !self.context_menu.confirm_visible {
                        // 优先处理选中范围
                        if let Some((sc, sr, ec, er)) = self.selected_range {
                            let has_content = excel_data.get_sheet(self.current_sheet).map(|sheet| {
                                (sr..=er).any(|r| (sc..=ec).any(|c| {
                                    sheet.get_cell(r, c)
                                        .map(|cell| !cell.value.is_empty() || !cell.formula.is_empty())
                                        .unwrap_or(false)
                                }))
                            }).unwrap_or(false);
                            if has_content {
                                self.context_menu.target_cell = self.selected_cell;
                                self.context_menu.confirm_action = Some(ContextAction::ClearCell);
                                self.context_menu.clear_is_range = true;
                                self.context_menu.confirm_visible = true;
                                self.context_menu.confirm_established = false;
                                self.context_menu.position = ui.ctx().memory(|mem| {
                                    mem.area_rect(egui::Id::new("table_scroll"))
                                        .map(|r| r.center())
                                        .unwrap_or(egui::Pos2::new(400.0, 300.0))
                                });
                            }
                        } else {
                            // 单格清空（原有逻辑）
                            let has_content = self.selected_cell.map(|(col, row)| {
                                excel_data.get_sheet(self.current_sheet)
                                    .and_then(|s| s.get_cell(row, col))
                                    .map(|c| !c.value.is_empty() || !c.formula.is_empty())
                                    .unwrap_or(false)
                            }).unwrap_or(false);
                            if has_content {
                                self.context_menu.target_cell = self.selected_cell;
                                self.context_menu.confirm_action = Some(ContextAction::ClearCell);
                                self.context_menu.clear_is_range = false;
                                self.context_menu.confirm_visible = true;
                                self.context_menu.confirm_established = false;
                                self.context_menu.position = ui.ctx().memory(|mem| {
                                    mem.area_rect(egui::Id::new("table_scroll"))
                                        .map(|r| r.center())
                                        .unwrap_or(egui::Pos2::new(400.0, 300.0))
                                });
                            }
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
                
                let (nav_result, save_clicked) = draw_name_box(
                    ui,
                    &mut self.name_box_state,
                    self.selected_cell,  // 直接使用选中的单元格，不转换为合并单元格的左上角
                    display_text.as_deref(),
                    max_col,
                    max_row,
                    &mut self.pending_formula_save,
                    self.dirty && !self.saving,
                );
                if let Some((col, row)) = nav_result {
                    self.selected_cell = Some((col, row));
                }
                if save_clicked {
                    self.save_requested = true;
                }

                ui.separator();

                // 记录调用前的选中单元格，用于检测变化后清除选中范围
                let prev_selected = self.selected_cell;
                // 本帧成功提交（保存）的编辑单元格 (row, col)：由 draw_table_content 写入，
                // 此处据此把编辑入撤销栈（无值＝本帧无提交 / 取消 / 校验失败）
                let mut committed_edit: Option<(u32, u32)> = None;
                // 填充柄提交信号：由 draw_table_content 在一次成功填充后写入
                let mut committed_fill: Option<FillCommit> = None;
                // 粘贴提交信号：由 draw_table_content 在一次成功粘贴后写入
                let mut committed_paste: Option<PasteCommit> = None;
                // 分批跨帧填充请求：由 draw_table_content 在大填充时写入
                let mut pending_fill_request: Option<PendingFill> = None;

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
                            &mut self.selected_range,
                            &mut self.editing_cell,
                            &mut self.edit_value,
                            &mut self.just_entered_edit_mode,
                            &mut self.validation_error,
                            &mut self.original_cell_data,
                            &mut committed_edit,
                            &mut self.context_menu,
                            &mut self.dirty,
                            &mut self.drag_anchor,
                            &mut self.fill_drag_source,
                            &mut committed_fill,
                            &self.hidden_columns,
                            &self.hidden_rows,
                            &mut self.shift_click_anchor,
                            &mut committed_paste,
                            &mut pending_fill_request,
                        );

                        // 检测 selected_cell 变化 → 清除选中范围（用户点击了新单元格）
                        // 拖拽选择期间不清除范围（drag_anchor 非 None）
                        if self.selected_cell != prev_selected && self.drag_anchor.is_none() {
                            self.selected_range = None;
                        }

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
                                        // 弹窗宽度为单元格宽度的 50%
                                        let popup_width = cell_rect.width() * 0.5;
                                        egui::Area::new(egui::Id::new("data_validation_popup"))
                                            .fixed_pos(pos)
                                            .order(egui::Order::Foreground)
                                            .show(ui.ctx(), |ui| {
                                                egui::Frame::popup(ui.style())
                                                    .fill(egui::Color32::from_rgb(255, 255, 225))
                                                    .show(ui, |ui| {
                                                        ui.set_min_width(popup_width);
                                                        ui.set_max_width(popup_width);
                                                        // 内容按弹窗宽度自动换行
                                                        if !dv.prompt_title.is_empty() {
                                                            ui.add(
                                                                egui::Label::new(
                                                                    egui::RichText::new(&dv.prompt_title).strong(),
                                                                )
                                                                .wrap(),
                                                            );
                                                        }
                                                        if !dv.prompt.is_empty() {
                                                            ui.add(egui::Label::new(&dv.prompt).wrap());
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
                                                                // 精确维护 formula_positions 索引
                                                                if orig_formula.is_empty() {
                                                                    sheet.unmark_formula(row, col);
                                                                } else {
                                                                    sheet.mark_formula(row, col);
                                                                }
                                                                self.dirty = true;
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

                    // 单元格编辑提交 → 入撤销栈（draw_table_content 经 committed_edit 通知本帧有一次成功保存）
                    // 复用编辑入口捕获的 original_cell_data：编辑只改 value/formula，
                    // 故"当前 cell 克隆 + 回填编辑前的 value/formula"即等价于编辑前快照（规避实时重算对 cell.value 的污染）
                    if let Some(((oc, or), orig_val, orig_fml)) = committed_edit.and_then(|_| self.original_cell_data.clone()) {
                        let row = or;
                        let col = oc;
                        if let Some(mut old) = excel_data.sheets.get(self.current_sheet)
                            .and_then(|s| s.cells.get(&(row, col)))
                            .cloned()
                        {
                            // 仅当 value/formula 确有变化才入栈，避免无操作占据撤销位
                            if old.value != orig_val || old.formula != orig_fml {
                                old.value = orig_val;
                                old.formula = orig_fml;
                                if self.undo_stack.len() >= MAX_UNDO_DEPTH {
                                    self.undo_stack.remove(0);
                                }
                                self.undo_stack.push(UndoAction::CellChange {
                                    sheet_index: self.current_sheet,
                                    row,
                                    col,
                                    old_cell: Some(old),
                                    old_selected: self.selected_cell,
                                });
                            }
                        }
                    }

                    // 填充柄提交 → 入撤销栈（复用 RangeClear：回放时恢复 old_cells + 选区 + 全表重算）
                    if let Some(fc) = committed_fill {
                        if self.undo_stack.len() >= MAX_UNDO_DEPTH {
                            self.undo_stack.remove(0);
                        }
                        self.undo_stack.push(UndoAction::RangeClear {
                            sheet_index: self.current_sheet,
                            old_cells: fc.old_cells,
                            old_selected: fc.old_selected,
                            old_range: fc.old_range,
                        });
                    }

                    // 粘贴提交 → 入撤销栈（复用 RangeClear：回放时恢复 old_cells + 选区 + 全表重算）
                    if let Some(pc) = committed_paste {
                        if self.undo_stack.len() >= MAX_UNDO_DEPTH {
                            self.undo_stack.remove(0);
                        }
                        self.undo_stack.push(UndoAction::RangeClear {
                            sheet_index: self.current_sheet,
                            old_cells: pc.old_cells,
                            old_selected: pc.old_selected,
                            old_range: pc.old_range,
                        });
                    }

                    // 分批跨帧填充请求：大填充由 draw_table_content 预计算后写入，
                    // 此处接管 PendingFill 状态，由下方「分批写入」段逐帧执行。
                    if let Some(pf) = pending_fill_request {
                        self.pending_fill = Some(pf);
                    }

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

                                            // 清空单元格/选中范围（无内容时灰色不可点击）
                                            let (clear_label, has_content, is_range) = if let Some((sc, sr, ec, er)) = self.selected_range {
                                                // 选中范围：检查范围内是否有内容
                                                let has = excel_data.get_sheet(self.current_sheet).map(|sheet| {
                                                    (sr..=er).any(|r| (sc..=ec).any(|c| {
                                                        sheet.get_cell(r, c)
                                                            .map(|cell| !cell.value.is_empty() || !cell.formula.is_empty())
                                                            .unwrap_or(false)
                                                    }))
                                                }).unwrap_or(false);
                                                ("清空选中范围", has, true)
                                            } else {
                                                // 单格清空
                                                let has = self.context_menu.target_cell.map(|(col, row)| {
                                                    excel_data.get_sheet(self.current_sheet)
                                                        .and_then(|s| s.get_cell(row, col))
                                                        .map(|c| !c.value.is_empty() || !c.formula.is_empty())
                                                        .unwrap_or(false)
                                                }).unwrap_or(false);
                                                ("清空单元格", has, false)
                                            };
                                            let clear_response = ui.add_enabled(has_content, egui::Button::new(clear_label));
                                            if clear_response.clicked() {
                                                self.context_menu.confirm_action = Some(ContextAction::ClearCell);
                                                self.context_menu.clear_is_range = is_range;
                                                self.context_menu.confirm_visible = true;
                                            }

                                            ui.separator();

                                            // 选中方向操作
                                            ui.horizontal(|ui| {
                                                if ui.button("向下选中").clicked() {
                                                    pending_action = Some(ContextAction::SelectDown);
                                                }
                                                ui.add(egui::DragValue::new(&mut self.context_menu.select_down_count)
                                                    .range(0..=10000)
                                                    .speed(0.1));
                                                ui.label("行");
                                            });
                                            ui.horizontal(|ui| {
                                                if ui.button("向上选中").clicked() {
                                                    pending_action = Some(ContextAction::SelectUp);
                                                }
                                                ui.add(egui::DragValue::new(&mut self.context_menu.select_up_count)
                                                    .range(0..=10000)
                                                    .speed(0.1));
                                                ui.label("行");
                                            });
                                            ui.horizontal(|ui| {
                                                if ui.button("向左选中").clicked() {
                                                    pending_action = Some(ContextAction::SelectLeft);
                                                }
                                                ui.add(egui::DragValue::new(&mut self.context_menu.select_left_count)
                                                    .range(0..=10000)
                                                    .speed(0.1));
                                                ui.label("列");
                                            });
                                            ui.horizontal(|ui| {
                                                if ui.button("向右选中").clicked() {
                                                    pending_action = Some(ContextAction::SelectRight);
                                                }
                                                ui.add(egui::DragValue::new(&mut self.context_menu.select_right_count)
                                                    .range(0..=10000)
                                                    .speed(0.1));
                                                ui.label("列");
                                            });
                                        });
                                    });
                            });

                        // 执行操作（在闭包外处理，避免借用冲突）
                        if let Some(action) = pending_action {
                            if let Some((col, row)) = self.context_menu.target_cell {
                                // 先关闭编辑状态
                                self.editing_cell = None;
                                self.edit_value.clear();
                                self.original_cell_data = None;
                                self.validation_error = None;
                                self.validation_error_pos = None;

                                match action {
                                    // 选中操作：直接设置 selected_range
                                    ContextAction::SelectDown | ContextAction::SelectUp
                                    | ContextAction::SelectLeft | ContextAction::SelectRight => {
                                        let n = match action {
                                            ContextAction::SelectDown => self.context_menu.select_down_count,
                                            ContextAction::SelectUp => self.context_menu.select_up_count,
                                            ContextAction::SelectLeft => self.context_menu.select_left_count,
                                            ContextAction::SelectRight => self.context_menu.select_right_count,
                                            _ => 0,
                                        };
                                        let max_row = excel_data.get_sheet(self.current_sheet).map(|s| s.max_row).unwrap_or(row);
                                        let max_col = excel_data.get_sheet(self.current_sheet).map(|s| s.max_col).unwrap_or(col);
                                        let (start_col, start_row, end_col, end_row) = match action {
                                            ContextAction::SelectDown => {
                                                let er = if n == 0 { max_row } else { (row + n).min(max_row) };
                                                (col, row, col, er)
                                            }
                                            ContextAction::SelectUp => {
                                                let sr = if n == 0 { 1 } else { row.saturating_sub(n).max(1) };
                                                (col, sr, col, row)
                                            }
                                            ContextAction::SelectRight => {
                                                let ec = if n == 0 { max_col } else { (col + n).min(max_col) };
                                                (col, row, ec, row)
                                            }
                                            ContextAction::SelectLeft => {
                                                let sc = if n == 0 { 1 } else { col.saturating_sub(n).max(1) };
                                                (sc, row, col, row)
                                            }
                                            _ => (col, row, col, row),
                                        };
                                        self.selected_range = Some((start_col, start_row, end_col, end_row));
                                    }
                                    // 插入/清空操作
                                    _ => {
                                        if let Some(sheet) = excel_data.sheets.get_mut(self.current_sheet) {
                                            let (anchor_col, anchor_row) = if let Some(mr) = sheet.get_merged_range(col, row) {
                                                match action {
                                                    ContextAction::InsertRowAbove => (col, mr.start_row),
                                                    ContextAction::InsertRowBelow => (col, mr.end_row),
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

                                            let n = self.context_menu.insert_rows_count;
                                            let m = self.context_menu.insert_cols_count;

                                            Self::push_undo_full(&mut self.undo_stack, sheet, self.current_sheet);

                                            let default_options = crate::excel::reader::ColumnCopyOptions::new(
                                                true,   // copy_merge: 复制合并单元格
                                                false,  // copy_formula
                                                true,   // copy_style: 复制样式
                                                false,  // copy_value
                                            );
                                            match action {
                                                ContextAction::InsertRowAbove => {
                                                    sheet.insert_rows(anchor_row, n, false);
                                                    self.dirty = true;
                                                    update_alert_range_expansions_for_row(&mut self.alert_popup.rules, anchor_row, n, sheet);
                                                }
                                                ContextAction::InsertRowBelow => {
                                                    sheet.insert_rows(anchor_row, n, true);
                                                    self.dirty = true;
                                                    // InsertRowBelow: 实际插入位置为 anchor_row + 1
                                                    update_alert_range_expansions_for_row(&mut self.alert_popup.rules, anchor_row + 1, n, sheet);
                                                }
                                                ContextAction::InsertColumnLeft => {
                                                    sheet.insert_columns(anchor_col, m, false, default_options);
                                                    self.dirty = true;
                                                    update_alert_range_expansions_for_col(&mut self.alert_popup.rules, anchor_col, m, sheet);
                                                }
                                                ContextAction::InsertColumnRight => {
                                                    sheet.insert_columns(anchor_col, m, true, default_options);
                                                    self.dirty = true;
                                                    // InsertColumnRight: 实际插入位置为 anchor_col + 1
                                                    update_alert_range_expansions_for_col(&mut self.alert_popup.rules, anchor_col + 1, m, sheet);
                                                }
                                                ContextAction::ClearCell => {
                                                    // 清空走确认弹窗路径，这里不应到达
                                                }
                                                _ => {}
                                            }
                                            crate::excel::formula::evaluate_sheet(&mut excel_data.sheets[self.current_sheet]);
                                        }
                                    }
                                }
                            }
                            self.context_menu.visible = false;
                            if !self.context_menu.confirm_visible {
                                self.context_menu.confirm_established = false;
                                self.context_menu.confirm_action = None;
                            }
                            self.context_menu.select_down_count = 0;
                            self.context_menu.select_up_count = 0;
                            self.context_menu.select_left_count = 0;
                            self.context_menu.select_right_count = 0;
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
                                        // 如果有确认弹窗正在等待，不在此处关闭它
                                        if !self.context_menu.confirm_visible {
                                            self.context_menu.confirm_established = false;
                                            self.context_menu.confirm_action = None;
                                        }
                                        self.context_menu.select_down_count = 0;
                            self.context_menu.select_up_count = 0;
                            self.context_menu.select_left_count = 0;
                            self.context_menu.select_right_count = 0;
                                    }
                                }
                            }
                        }
                        // Escape 关闭
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            self.context_menu.visible = false;
                            if !self.context_menu.confirm_visible {
                                self.context_menu.confirm_established = false;
                                self.context_menu.confirm_action = None;
                            }
                            self.context_menu.select_down_count = 0;
                            self.context_menu.select_up_count = 0;
                            self.context_menu.select_left_count = 0;
                            self.context_menu.select_right_count = 0;
                        }
                    }

                    // 绘制确认弹窗（插入列 / 清空单元格）
                    if self.context_menu.confirm_visible {
                        // Escape 关闭确认弹窗
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            self.context_menu.confirm_visible = false;
                            self.context_menu.confirm_established = false;
                            self.context_menu.confirm_action = None;
                        }

                        // 首帧标记为已建立，后续帧才检测外部点击
                        let is_established = self.context_menu.confirm_established;
                        self.context_menu.confirm_established = true;

                        let confirm_action = self.context_menu.confirm_action;
                        let mut confirm_execute = false;
                        let mut cancel_clicked = false;
                        let mut keep_open = true;

                        // 根据操作类型显示不同的确认弹窗
                        if confirm_action == Some(ContextAction::ClearCell) {
                            // 清空确认弹窗（区分范围/单格）—— 红色警示样式
                            // 清空属于破坏性操作，整个弹窗（标题/正文/边框/背景/确认按钮）采用红色系配色，
                            // 从视觉上强化警示效果，与插入列的普通确认弹窗区分开。
                            let confirm_text = if self.context_menu.clear_is_range {
                                "确定清空选中范围的内容？"
                            } else {
                                "确定清空该单元格的内容？"
                            };
                            // 红色系配色
                            let warn_red = egui::Color32::from_rgb(211, 47, 47);   // 主警示红：边框 + 标题 + 确认按钮填充
                            let light_red = egui::Color32::from_rgb(254, 236, 236); // 浅红背景
                            let dark_red = egui::Color32::from_rgb(150, 24, 24);    // 深红正文
                            let soft_red = egui::Color32::from_rgb(200, 80, 80);    // 柔红：取消按钮描边
                            egui::Window::new("clear_confirm")
                                .title_bar(false)
                                .open(&mut keep_open)
                                .resizable(false)
                                .collapsible(false)
                                .order(egui::Order::Foreground)
                                .fixed_pos(self.context_menu.position)
                                .min_height(80.0)
                                .max_height(80.0)
                                // 红色边框 + 浅红背景的警示框架（沿用 Window 默认阴影/圆角以保留层次感）
                                .frame(
                                    egui::Frame::window(ui.style())
                                        .fill(light_red)
                                        .stroke(egui::Stroke::new(2.0, warn_red))
                                        .inner_margin(egui::Margin::same(10)),
                                )
                                .show(ui.ctx(), |ui| {
                                    ui.set_width(240.0);
                                    // 警示标题行（红色加粗 + ⚠ 图标）
                                    ui.vertical_centered(|ui| {
                                        ui.label(
                                            egui::RichText::new("⚠ 警告")
                                                .color(warn_red)
                                                .strong()
                                                .size(15.0),
                                        );
                                    });
                                    ui.add_space(4.0);
                                    // 正文（深红）
                                    ui.vertical_centered(|ui| {
                                        ui.label(
                                            egui::RichText::new(confirm_text)
                                                .color(dark_red)
                                                .size(13.0),
                                        );
                                    });
                                    ui.add_space(8.0);
                                    // 按钮：确认（红色填充，破坏性操作）在右、取消（中性描边）在左
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        let confirm_btn = egui::Button::new(
                                            egui::RichText::new("确认")
                                                .color(egui::Color32::WHITE)
                                                .strong(),
                                        )
                                        .fill(warn_red)
                                        .stroke(egui::Stroke::new(1.0, warn_red));
                                        if ui.add(confirm_btn).clicked() {
                                            confirm_execute = true;
                                        }
                                        let cancel_btn = egui::Button::new("取消")
                                            .fill(egui::Color32::WHITE)
                                            .stroke(egui::Stroke::new(1.0, soft_red));
                                        if ui.add(cancel_btn).clicked() {
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
                                                if self.context_menu.clear_is_range {
                                                    // 范围清空：保存范围内所有单元格撤销快照
                                                    if let Some((sc, sr, ec, er)) = self.selected_range {
                                                        Self::push_undo_range(
                                                            &mut self.undo_stack,
                                                            self.current_sheet,
                                                            sc, sr, ec, er,
                                                            sheet,
                                                            self.selected_cell,
                                                            self.selected_range,
                                                        );
                                                        // 清空范围内所有单元格的值和公式
                                                        for r in sr..=er {
                                                            for c in sc..=ec {
                                                                if let Some(cell) = sheet.cells.get_mut(&(r, c)) {
                                                                    cell.value.clear();
                                                                    cell.formula.clear();
                                                                    sheet.unmark_formula(r, c);
                                                                }
                                                            }
                                                        }
                                                        // 范围清空后触发全表公式重算
                                                        crate::excel::formula::evaluate_sheet(&mut excel_data.sheets[self.current_sheet]);
                                                        self.dirty = true;
                                                    }
                                                } else {
                                                    // 单格清空（原有逻辑）
                                                    Self::push_undo_cell(&mut self.undo_stack, self.current_sheet, row, col, sheet, self.selected_cell);
                                                    if let Some(cell) = sheet.cells.get_mut(&(row, col)) {
                                                        cell.value.clear();
                                                        cell.formula.clear();
                                                        sheet.unmark_formula(row, col);
                                                    }
                                                    crate::excel::formula::evaluate_dependents(&mut excel_data.sheets[self.current_sheet], row, col);
                                                    self.dirty = true;
                                                }
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

                                                // 保存撤销快照（全量：插入列是结构性操作）
                                                Self::push_undo_full(&mut self.undo_stack, sheet, self.current_sheet);

                                                let copy_options = crate::excel::reader::ColumnCopyOptions::new(
                                                    self.context_menu.copy_merge,
                                                    self.context_menu.copy_formula,
                                                    self.context_menu.copy_style,
                                                    self.context_menu.copy_value,
                                                );
                                                match action {
                                                    ContextAction::InsertColumnLeft => {
                                                        sheet.insert_columns(anchor_col, m, false, copy_options);
                                                        self.dirty = true;
                                                        update_alert_range_expansions_for_col(&mut self.alert_popup.rules, anchor_col, m, sheet);
                                                    }
                                                    ContextAction::InsertColumnRight => {
                                                        sheet.insert_columns(anchor_col, m, true, copy_options);
                                                        self.dirty = true;
                                                        update_alert_range_expansions_for_col(&mut self.alert_popup.rules, anchor_col + 1, m, sheet);
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
                        // 保存单单元格撤销快照
                        if let Some((col, row)) = self.selected_cell {
                            if let Some(sheet) = excel_data.sheets.get(self.current_sheet) {
                                Self::push_undo_cell(&mut self.undo_stack, self.current_sheet, row, col, sheet, self.selected_cell);
                            }
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
                                    {
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
                                    }
                                    excel_data.sheets[self.current_sheet].unmark_formula(row, col);
                                    crate::excel::formula::evaluate_dependents(&mut excel_data.sheets[self.current_sheet], row, col);
                                    self.dirty = true;
                                }
                            }
                        } else {
                            // 公式直接保存
                            {
                                let cell = excel_data.sheets[self.current_sheet]
                                    .cells.entry((row, col))
                                    .or_insert_with(|| crate::excel::reader::CellData::default());
                                cell.formula = formula_value;
                            }
                            excel_data.sheets[self.current_sheet].mark_formula(row, col);
                            crate::excel::formula::evaluate_sheet(&mut excel_data.sheets[self.current_sheet]);
                            self.dirty = true;
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

        // ========== 分批跨帧填充：每帧写入 FILL_BATCH_SIZE 格 ==========
        // 由 draw_table_content 在大填充（> FILL_SYNC_THRESHOLD）时创建 PendingFill，
        // 此处逐帧写入预计算值，帧间 UI 保持流畅。
        if let Some(pending) = self.pending_fill.as_mut() {
            if let Some(excel_data) = self.excel_data.as_mut() {
                let end = (pending.next_idx + crate::excel::fill::FILL_BATCH_SIZE)
                    .min(pending.values.len());
                if let Some(sheet) = excel_data.sheets.get_mut(self.current_sheet) {
                    for &(row, col, ref new_cell) in &pending.values[pending.next_idx..end] {
                        let old = sheet.get_cell(row, col).cloned();
                        let old_had_formula = old.as_ref().map_or(false, |c| !c.formula.is_empty());
                        pending.old_cells.push((row, col, old));
                        sheet.cells.insert((row, col), new_cell.clone());
                        // 精确维护 formula_positions 索引
                        if !new_cell.formula.is_empty() {
                            sheet.mark_formula(row, col);
                        } else if old_had_formula {
                            sheet.unmark_formula(row, col);
                        }
                    }
                }
                pending.next_idx = end;
                ctx.request_repaint(); // 确保下一帧继续写入

                if pending.next_idx >= pending.values.len() {
                    // 填充完成 → 触发重算 + 选区更新 + 撤销入栈
                    let pf = self.pending_fill.take().unwrap();
                    if let Some(sheet) = excel_data.sheets.get_mut(self.current_sheet) {
                        if pf.has_formula {
                            crate::excel::formula::invalidate_formula_graph(sheet);
                            crate::excel::formula::evaluate_sheet(sheet);
                        } else {
                            crate::excel::formula::evaluate_dependents_many(
                                sheet,
                                pf.old_cells.iter().map(|(r, c, _)| (*r, *c)),
                            );
                        }
                    }
                    self.dirty = true;
                    // 填充后选区 = 源 ∪ 目标
                    let (sc0, sr0, sc1, sr1) = pf.src;
                    let (tcol, trow) = pf.target;
                    self.selected_cell = Some((sc0, sr0));
                    self.selected_range = Some((
                        sc0.min(tcol),
                        sr0.min(trow),
                        sc1.max(tcol),
                        sr1.max(trow),
                    ));
                    // 入撤销栈
                    if self.undo_stack.len() >= MAX_UNDO_DEPTH {
                        self.undo_stack.remove(0);
                    }
                    self.undo_stack.push(UndoAction::RangeClear {
                        sheet_index: self.current_sheet,
                        old_cells: pf.old_cells,
                        old_selected: pf.prev_selected,
                        old_range: pf.prev_range,
                    });
                }
            }
        }

        // 处理延迟的保存请求（在 excel_data 借用释放后执行）
        if self.save_requested {
            self.save_requested = false;
            self.start_async_save(ctx.clone());
        }
        } // end of !is_blocking

        // —— 授权状态检查（每帧）——
        let status_text = license_status_text(&lic_status);
        // 仅试用期内（剩余天数 > 0）允许用户主动关闭激活弹窗；
        // 到期 / 篡改等拦截态下模态不可关闭，强制激活。
        let can_close = matches!(lic_status, LicenseStatus::Trial { days_left } if days_left > 0);
        // 闭包只捕获 self.license，与 &mut self.license_popup 是不相交字段借用（edition 2021）
        let mut activate_cb = |code: &str| match self.license.activate(code, lic_today) {
            Ok(_) => Ok(()),
            Err(e) => Err(e.message()),
        };
        draw_license_popup(&ctx, &mut self.license_popup, &status_text, can_close, &mut activate_cb);
        // 正常（非拦截）运行时推进高水位，防时钟回拨
        if !lic_status.is_blocking() {
            self.license.checkpoint(lic_today);
        }
    }
}

/// 授权弹窗顶部标题文案（仅在 blocking 状态下调用）
fn license_status_text(status: &LicenseStatus) -> String {
    match status {
        LicenseStatus::TrialExpired => "试用期已结束".to_string(),
        LicenseStatus::LicensedExpired => "授权已到期".to_string(),
        LicenseStatus::Tampered => "检测到异常（时钟回拨或文件被改动）".to_string(),
        _ => "需要激活".to_string(),
    }
}


