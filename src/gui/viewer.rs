// 引入 egui 用于界面渲染
use eframe::egui;
// 引入 Excel 数据读取模块
use crate::excel::reader::{ExcelData, col_to_letter};
// 引入通道接收器用于异步加载
use std::sync::mpsc::Receiver;

/// 文件加载状态枚举
#[derive(Debug, Clone)]
pub enum LoadState {
    /// 空闲状态，未加载任何文件
    Idle,
    /// 正在加载中
    Loading,
    /// 加载成功，包含 Excel 数据
    #[allow(dead_code)]
    Success(ExcelData),
    /// 加载失败，包含错误信息
    #[allow(dead_code)]
    Failed(String),
}

/// Excel 查看器主结构体，管理所有 UI 状态和数据
pub struct ExcelViewer {
    /// 当前加载的 Excel 数据（未加载时为 None）
    excel_data: Option<ExcelData>,
    /// 当前显示的工作表索引（从0开始）
    current_sheet: usize,
    /// 错误信息（有错误时为 Some）
    error_message: Option<String>,
    /// 当前选中的单元格坐标（行, 列）
    selected_cell: Option<(u32, u32)>,
    /// 当前鼠标悬停的单元格坐标
    hovered_cell: Option<(u32, u32)>,
    /// 是否显示导入文件对话框
    show_import_dialog: bool,
    /// 当前的加载状态
    load_state: LoadState,
    /// 异步加载的通道接收器
    rx: Option<Receiver<Result<ExcelData, String>>>,
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
    fn check_load_result(&mut self) {
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

    /// 同步加载 Excel 文件（已弃用，保留用于兼容性）
    #[allow(dead_code)]
    pub fn load_file(&mut self, path: &str) {
        match ExcelData::load_from_file(path) {
            Ok(data) => {
                self.excel_data = Some(data);
                self.current_sheet = 0;
                self.error_message = None;
                self.selected_cell = None;
                self.hovered_cell = None;
            }
            Err(e) => {
                self.error_message = Some(e);
            }
        }
    }

    /// 加载中文字体
    /// 
    /// 尝试从 Windows 系统字体目录加载常用中文字体
    /// 
    /// # 返回值
    /// 成功返回 (字体名称, 字体数据)，失败返回 None
    fn load_chinese_font() -> Option<(String, Vec<u8>)> {
        // 常用中文字体路径列表
        let font_paths = vec![
            r"C:\Windows\Fonts\msyh.ttc",    // 微软雅黑
            r"C:\Windows\Fonts\msyh.ttf",    // 微软雅黑
            r"C:\Windows\Fonts\simhei.ttf",  // 黑体
            r"C:\Windows\Fonts\simkai.ttf",  // 楷体
        ];

        // 遍历尝试加载字体
        for font_path in font_paths {
            if let Ok(font_data) = std::fs::read(font_path) {
                // 根据路径判断字体名称
                if font_path.contains("msyh") {
                    return Some(("Microsoft YaHei".to_string(), font_data));
                } else if font_path.contains("simhei") {
                    return Some(("SimHei".to_string(), font_data));
                } else if font_path.contains("simkai") {
                    return Some(("SimKai".to_string(), font_data));
                }
            }
        }
        None
    }

    /// 设置中文字体
    /// 
    /// 将中文字体添加到 egui 的字体系统中，确保中文能正常显示
    /// 
    /// # 参数
    /// * `ctx` - egui 上下文
    fn setup_fonts(ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::default();
        
        // 尝试加载中文字体
        if let Some((ref font_name, font_data)) = Self::load_chinese_font() {
            // 添加字体数据
            fonts.font_data.insert(
                font_name.clone(),
                egui::FontData::from_owned(font_data),
            );
            
            // 将字体插入到字体族的首位，优先使用
            fonts.families.get_mut(&egui::FontFamily::Proportional).unwrap().insert(0, font_name.clone());
            fonts.families.get_mut(&egui::FontFamily::Monospace).unwrap().insert(0, font_name.clone());
            
            // 应用字体设置
            ctx.set_fonts(fonts);
        }
    }

    /// 绘制菜单栏
    /// 
    /// 包含文件、编辑、关于等菜单项
    /// 
    /// # 参数
    /// * `ctx` - egui 上下文
    fn draw_menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                // 文件菜单
                ui.menu_button("文件", |ui| {
                    // 导入文件按钮
                    if ui.button("导入").clicked() {
                        ui.close_menu();
                        self.show_import_dialog = true;
                    }
                    // 模板按钮（暂不可用）
                    ui.add_enabled(false, egui::Button::new("模板"));
                });
                
                // 编辑菜单（暂未实现）
                ui.menu_button("编辑", |ui| {
                    ui.label("编辑功能");
                });
                
                // 关于菜单
                ui.menu_button("关于", |ui| {
                    ui.label("Excel Viewer v0.1.0");
                    ui.label("使用 umya-spreadsheet 和 egui 构建");
                });
            });
        });
    }

    /// 绘制导入文件对话框
    /// 
    /// 使用 rfd 库显示原生文件选择对话框
    /// 
    /// # 参数
    /// * `ctx` - egui 上下文
    fn draw_import_dialog(&mut self, ctx: &egui::Context) {
        if self.show_import_dialog {
            // 显示文件选择对话框，只允许 xlsx 和 xls 文件
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Excel Files", &["xlsx", "xls"])
                .pick_file()
            {
                // 选择文件后启动异步加载
                self.start_async_load(path.to_string_lossy().to_string(), ctx.clone());
            }
            self.show_import_dialog = false;
        }
    }

    /// 绘制错误信息（已弃用，保留用于兼容性）
    #[allow(dead_code)]
    fn draw_error(&mut self, ui: &mut egui::Ui) {
        if let Some(error) = &self.error_message {
            ui.colored_label(egui::Color32::RED, error);
        }
    }

    /// 绘制工作表选择器
    /// 
    /// 在底部显示所有工作表名称，支持切换工作表
    /// 
    /// # 参数
    /// * `ui` - egui UI 上下文
    fn draw_sheet_selector(&mut self, ui: &mut egui::Ui) {
        if let Some(data) = &self.excel_data {
            ui.horizontal(|ui| {
                // 遍历所有工作表，显示可选择的标签
                for (i, sheet) in data.sheets.iter().enumerate() {
                    if ui.selectable_label(self.current_sheet == i, &sheet.name).clicked() {
                        // 切换工作表时重置选中和悬停状态
                        self.current_sheet = i;
                        self.selected_cell = None;
                        self.hovered_cell = None;
                    }
                }
            });
        }
    }

    /// 绘制表格内容
    /// 
    /// 使用虚拟渲染技术，只绘制可见区域的单元格，提高性能
    /// 
    /// # 参数
    /// * `ui` - egui UI 上下文
    fn draw_table_content(&mut self, ui: &mut egui::Ui) {
        if let Some(data) = &self.excel_data {
            if let Some(sheet) = data.get_sheet(self.current_sheet) {
                let selected_cell = self.selected_cell;
                
                // 表格渲染常量定义
                let row_height = 25.0;        // 每行高度
                let default_col_width = 80.0; // 默认列宽
                let header_width = 60.0;      // 行号列宽度
                let border_width = 1.0;       // 边框宽度
                
                // 获取列宽的辅助函数
                let get_col_width = |col: u32| -> f32 {
                    if let Some(&width) = sheet.column_widths.get(&col) {
                        // 使用 Excel 中的列宽，乘以系数转换为像素
                        width as f32 * 8.0
                    } else {
                        default_col_width
                    }
                };
                
                // 计算表格总宽度
                let mut total_width = header_width;
                for col in 1..=sheet.max_col {
                    total_width += get_col_width(col) + border_width;
                }
                total_width += border_width;
                // 计算表格总高度（包含表头）
                let total_height = row_height * (sheet.max_row + 1) as f32 + border_width * (sheet.max_row + 2) as f32;
                
                // 分配绘画区域
                let (response, painter) = ui.allocate_painter(egui::vec2(total_width, total_height), egui::Sense::hover());
                let rect = response.rect;
                let top_left = rect.min;
                
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
                let margin = 100.0; // 额外渲染一些边距，避免滚动时空白
                    
                    // 计算可见行范围
                    let visible_rows_start = ((viewport_rect.min.y - tl_y - margin) / (row_height + border_width)).floor() as u32;
                    let visible_rows_end = ((viewport_rect.max.y - tl_y + margin) / (row_height + border_width)).ceil() as u32;
                    let visible_rows_start = visible_rows_start.max(0).min(sheet.max_row + 1);
                    let visible_rows_end = visible_rows_end.max(0).min(sheet.max_row + 1);
                    
                    // 计算可见列范围
                    let visible_cols_start = ((viewport_rect.min.x - tl_x - margin) / (default_col_width + border_width)).floor() as u32;
                    let visible_cols_end = ((viewport_rect.max.x - tl_x + margin) / (default_col_width + border_width)).ceil() as u32;
                    let visible_cols_start = visible_cols_start.max(0).min(sheet.max_col + 1);
                    let visible_cols_end = visible_cols_end.max(0).min(sheet.max_col + 1);
                    
                    // 计算起始绘制位置
                    let start_y = tl_y + border_width + visible_rows_start as f32 * (row_height + border_width);
                    let mut y = start_y;
                    
                    // 遍历可见行进行绘制
                    for row in visible_rows_start..=visible_rows_end {
                        let mut x = tl_x + border_width;
                        // 跳过不可见的左侧列
                        for c in 0..visible_cols_start {
                            x += if c == 0 { header_width } else { get_col_width(c) } + border_width;
                        }
                        
                        // 绘制可见列
                        for col in visible_cols_start..=visible_cols_end {
                            let cell_width = if col == 0 { 
                                header_width 
                            } else { 
                                get_col_width(col) 
                            };
                            let cell_height = row_height;
                            
                            // 确定单元格背景色
                            let bg_color = if row == 0 && col == 0 {
                                egui::Color32::LIGHT_GRAY // 左上角空白
                            } else if row == 0 {
                                egui::Color32::LIGHT_GRAY // 列标题行
                            } else if col == 0 {
                                egui::Color32::LIGHT_GRAY // 行标题列
                            } else {
                                egui::Color32::WHITE // 数据单元格
                            };
                            
                            // 绘制单元格背景
                            painter.rect_filled(
                                egui::Rect::from_min_size(egui::Pos2::new(x, y), egui::vec2(cell_width, cell_height)),
                                0.0,
                                bg_color,
                            );
                            
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
                            // 绘制数据单元格
                            else if row > 0 && col > 0 {
                                let mut cell_content = String::new();
                                let mut is_merged_top_left = false;
                                
                                // 检查是否是合并单元格
                                if let Some(merged_range) = sheet.get_merged_range(col, row) {
                                    // 只在合并单元格的左上角绘制内容
                                    if merged_range.is_top_left(col, row) {
                                        is_merged_top_left = true;
                                        if let Some(cell) = sheet.get_cell(col, row) {
                                            cell_content = cell.value.clone();
                                        }
                                    }
                                } else {
                                    // 普通单元格
                                    if let Some(cell) = sheet.get_cell(col, row) {
                                        cell_content = cell.value.clone();
                                    }
                                }
                                
                                // 绘制合并单元格
                                if is_merged_top_left {
                                    if let Some(merged_range) = sheet.get_merged_range(col, row) {
                                        // 计算合并单元格的总宽度和高度
                                        let mut merged_col_width = 0.0;
                                        for c in merged_range.start_col..=merged_range.end_col {
                                            merged_col_width += get_col_width(c) + border_width;
                                        }
                                        merged_col_width -= border_width;
                                        let merged_row_height = (merged_range.end_row - merged_range.start_row + 1) as f32 * row_height + 
                                            (merged_range.end_row - merged_range.start_row) as f32 * border_width;
                                        
                                        // 检查是否选中（合并范围内任一单元格选中都高亮）
                                        let is_selected = selected_cell.is_some() && 
                                            merged_range.contains(selected_cell.unwrap().0, selected_cell.unwrap().1);
                                        
                                        // 绘制选中高亮背景
                                        if is_selected {
                                            painter.rect_filled(
                                                egui::Rect::from_min_size(
                                                    egui::Pos2::new(x, y),
                                                    egui::vec2(merged_col_width, merged_row_height),
                                                ),
                                                0.0,
                                                egui::Color32::from_rgb(173, 216, 230), // 浅蓝
                                            );
                                        }
                                        
                                        // 在合并单元格中心绘制内容
                                        painter.text(
                                            egui::Pos2::new(x + merged_col_width / 2.0, y + merged_row_height / 2.0),
                                            egui::Align2::CENTER_CENTER,
                                            &cell_content,
                                            egui::FontId::default(),
                                            egui::Color32::BLACK,
                                        );
                                    }
                                } 
                                // 绘制普通单元格
                                else {
                                    let is_selected = selected_cell == Some((col, row));
                                    // 绘制选中高亮
                                    if is_selected {
                                        painter.rect_filled(
                                            egui::Rect::from_min_size(
                                                egui::Pos2::new(x, y),
                                                egui::vec2(cell_width, cell_height),
                                            ),
                                            0.0,
                                            egui::Color32::from_rgb(173, 216, 230),
                                        );
                                    }
                                    
                                    // 绘制单元格内容（如果不为空）
                                    if !cell_content.is_empty() {
                                        painter.text(
                                            egui::Pos2::new(x + cell_width / 2.0, y + cell_height / 2.0),
                                            egui::Align2::CENTER_CENTER,
                                            &cell_content,
                                            egui::FontId::default(),
                                            egui::Color32::BLACK,
                                        );
                                    }
                                }
                            }
                            
                            // 移动到下一列
                            x += cell_width + border_width;
                        }
                        // 移动到下一行
                        y += row_height + border_width;
                    }
            }
        }
    }

    /// 绘制选中单元格信息（已弃用，保留用于兼容性）
    #[allow(dead_code)]
    fn draw_selected_info(&mut self, ui: &mut egui::Ui) {
        if let Some(data) = &self.excel_data {
            if let Some(sheet) = data.get_sheet(self.current_sheet) {
                if let Some((row, col)) = self.selected_cell {
                    ui.separator();
                    ui.label(format!("Selected: {}{}", col_to_letter(col), row));
                    
                    if let Some(cell) = sheet.get_cell(row, col) {
                        if !cell.formula.is_empty() {
                            ui.label(format!("Formula: {}", cell.formula));
                        }
                    }
                }
            }
        }
    }

    /// 绘制空状态提示
    /// 
    /// 当未加载文件时显示提示信息
    /// 
    /// # 参数
    /// * `ui` - egui UI 上下文
    fn draw_empty_state(&mut self, ui: &mut egui::Ui) {
        ui.centered_and_justified(|ui| {
            ui.label("请通过 '文件 > 导入' 打开Excel文件");
        });
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
        Self::setup_fonts(ctx);
        
        // 绘制菜单栏
        self.draw_menu_bar(ctx);
        // 绘制导入对话框
        self.draw_import_dialog(ctx);
        
        // 检查异步加载结果
        self.check_load_result();
        
        // 主内容区域
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.excel_data.is_some() {
                // 已加载文件，显示表格
                let total_height = ui.available_height();
                let table_height = total_height - 40.0; // 留出工作表选择器空间
                
                // 使用滚动区域包裹表格
                egui::ScrollArea::both()
                    .max_height(table_height)
                    .show(ui, |ui| {
                        self.draw_table_content(ui);
                    });
                
                // 分隔线
                ui.separator();
                // 设置按钮间距
                ui.style_mut().spacing.button_padding = egui::vec2(8.0, 4.0);
                // 绘制工作表选择器
                self.draw_sheet_selector(ui);
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
                        self.draw_empty_state(ui);
                    }
                    _ => {
                        // 空闲状态，显示空状态
                        self.draw_empty_state(ui);
                    }
                }
            }
        });
    }
}
