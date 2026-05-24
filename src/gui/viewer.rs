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
    draw_sheet_selector,
    draw_empty_state,
    draw_name_box,
    NameBoxState,
};
use std::sync::mpsc::Receiver;

/// Excel 查看器主结构体，管理所有 UI 状态和数据
pub struct ExcelViewer {
    /// 当前加载的 Excel 数据（未加载时为 None）
    pub excel_data: Option<ExcelData>,
    /// 当前显示的工作表索引（从0开始）
    pub current_sheet: usize,
    /// 错误信息（有错误时为 Some）
    pub error_message: Option<String>,
    /// 当前选中的单元格坐标（行, 列）
    pub selected_cell: Option<(u32, u32)>,
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
}

impl ExcelViewer {
    /// 创建新的 Excel 查看器实例，初始化所有状态
    pub fn new() -> Self {
        Self {
            excel_data: None,
            current_sheet: 0,
            error_message: None,
            selected_cell: None,
            hovered_cell: None,
            show_import_dialog: false,
            load_state: LoadState::Idle,
            rx: None,
            name_box_state: NameBoxState::default(),
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
            draw_menu_bar(ui, &mut self.show_import_dialog);
        });
        
        // 绘制导入对话框
        if let Some(path) = draw_import_dialog(&mut self.show_import_dialog) {
            self.start_async_load(path, ctx.clone());
        }
        
        // 检查异步加载结果
        self.check_load_result();
        
        // 主内容区域
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(excel_data) = &self.excel_data {
                // 获取当前工作表信息
                if let Some(sheet) = excel_data.get_sheet(self.current_sheet) {
                    // 绘制名称框工具栏（在表格上方）
                    ui.set_min_height(28.0);
                    ui.style_mut().spacing.item_spacing = egui::vec2(4.0, 4.0);
                    
                    // 获取选中单元格的显示内容（优先显示公式，否则显示值）
                    // 如果点击的是合并单元格，则获取左上角单元格的内容
                    let display_text = self.selected_cell.and_then(|(col, row)| {
                        // 检查是否是合并单元格，如果是则获取左上角单元格
                        let (target_col, target_row) = if let Some(merged_range) = sheet.get_merged_range(col, row) {
                            (merged_range.start_col, merged_range.start_row)
                        } else {
                            (col, row)
                        };
                        
                        sheet.get_cell(target_row, target_col).map(|cell| {
                            if !cell.formula.is_empty() {
                                cell.formula.as_str()
                            } else {
                                cell.value.as_str()
                            }
                        })
                    });
                    
                    // 计算名称框显示的单元格位置（合并单元格显示左上角位置）
                    let display_cell = self.selected_cell.map(|(col, row)| {
                        if let Some(merged_range) = sheet.get_merged_range(col, row) {
                            (merged_range.start_col, merged_range.start_row)
                        } else {
                            (col, row)
                        }
                    });
                    
                    if let Some((col, row)) = draw_name_box(
                        ui,
                        &mut self.name_box_state,
                        display_cell,
                        display_text,
                        sheet.max_col,
                        sheet.max_row,
                    ) {
                        self.selected_cell = Some((col, row));
                    }
                    
                    ui.separator();
                }
                
                // 已加载文件，显示表格
                let total_height = ui.available_height();
                let table_height = total_height - 35.0; // 留出名称框和工作表选择器空间
                
                // 使用滚动区域包裹表格
                egui::ScrollArea::both()
                    .max_height(table_height)
                    .show(ui, |ui| {
                        draw_table_content(ui, excel_data, self.current_sheet, &mut self.selected_cell);
                    });
                
                // 分隔线
                ui.separator();
                // 设置按钮间距
                ui.style_mut().spacing.button_padding = egui::vec2(8.0, 4.0);
                // 绘制工作表选择器
                draw_sheet_selector(ui, excel_data, &mut self.current_sheet, &mut self.selected_cell);
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
