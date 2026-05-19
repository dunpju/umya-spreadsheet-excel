use eframe::egui;
use crate::excel::reader::{ExcelData, SheetData, col_to_letter};

pub struct ExcelViewer {
    excel_data: Option<ExcelData>,
    current_sheet: usize,
    error_message: Option<String>,
    selected_cell: Option<(u32, u32)>,
    hovered_cell: Option<(u32, u32)>,
    show_import_dialog: bool,
}

impl ExcelViewer {
    pub fn new() -> Self {
        Self {
            excel_data: None,
            current_sheet: 0,
            error_message: None,
            selected_cell: None,
            hovered_cell: None,
            show_import_dialog: false,
        }
    }

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

    fn load_chinese_font() -> Option<(String, Vec<u8>)> {
        let font_paths = vec![
            r"C:\Windows\Fonts\msyh.ttc",
            r"C:\Windows\Fonts\msyh.ttf",
            r"C:\Windows\Fonts\simhei.ttf",
            r"C:\Windows\Fonts\simkai.ttf",
        ];

        for font_path in font_paths {
            if let Ok(font_data) = std::fs::read(font_path) {
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

    fn setup_fonts(ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::default();
        
        if let Some((ref font_name, font_data)) = Self::load_chinese_font() {
            fonts.font_data.insert(
                font_name.clone(),
                egui::FontData::from_owned(font_data),
            );
            
            fonts.families.get_mut(&egui::FontFamily::Proportional).unwrap().insert(0, font_name.clone());
            fonts.families.get_mut(&egui::FontFamily::Monospace).unwrap().insert(0, font_name.clone());
            
            ctx.set_fonts(fonts);
        }
    }

    fn draw_menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("文件", |ui| {
                    if ui.button("导入").clicked() {
                        ui.close_menu();
                        self.show_import_dialog = true;
                    }
                    ui.add_enabled(false, egui::Button::new("模板"));
                });
                
                ui.menu_button("编辑", |ui| {
                    ui.label("编辑功能");
                });
                
                ui.menu_button("关于", |ui| {
                    ui.label("Excel Viewer v0.1.0");
                    ui.label("使用 umya-spreadsheet 和 egui 构建");
                });
            });
        });
    }

    fn draw_import_dialog(&mut self, _ctx: &egui::Context) {
        if self.show_import_dialog {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Excel Files", &["xlsx", "xls"])
                .pick_file()
            {
                self.load_file(path.to_string_lossy().as_ref());
            }
            self.show_import_dialog = false;
        }
    }

    fn draw_error(&mut self, ui: &mut egui::Ui) {
        if let Some(error) = &self.error_message {
            ui.colored_label(egui::Color32::RED, error);
        }
    }

    fn draw_sheet_selector(&mut self, ui: &mut egui::Ui) {
        if let Some(data) = &self.excel_data {
            ui.horizontal(|ui| {
                for (i, sheet) in data.sheets.iter().enumerate() {
                    if ui.selectable_label(self.current_sheet == i, &sheet.name).clicked() {
                        self.current_sheet = i;
                        self.selected_cell = None;
                        self.hovered_cell = None;
                    }
                }
            });
        }
    }

    fn draw_table(&mut self, ui: &mut egui::Ui) {
        if let Some(data) = &self.excel_data {
            if let Some(sheet) = data.get_sheet(self.current_sheet) {
                let selected_cell = self.selected_cell;
                
                egui::ScrollArea::both().show(ui, |ui| {
                    let row_height = 25.0;
                    let col_width = 80.0;
                    let header_width = 60.0;
                    let border_width = 1.0;
                    
                    let total_width = header_width + col_width * sheet.max_col as f32 + border_width * (sheet.max_col + 1) as f32;
                    let total_height = row_height * (sheet.max_row + 1) as f32 + border_width * (sheet.max_row + 2) as f32;
                    
                    let (response, painter) = ui.allocate_painter(egui::vec2(total_width, total_height), egui::Sense::hover());
                    let rect = response.rect;
                    let top_left = rect.min;
                    
                    let tl_x = top_left.x;
                    let tl_y = top_left.y;
                    
                    painter.rect_filled(
                        egui::Rect::from_min_size(egui::Pos2::new(tl_x, tl_y), egui::vec2(total_width, total_height)),
                        0.0,
                        egui::Color32::GRAY,
                    );
                    
                    let mut y = tl_y + border_width;
                    for row in 0..=sheet.max_row {
                        let mut x = tl_x + border_width;
                        for col in 0..=sheet.max_col {
                            let cell_width = if col == 0 { header_width } else { col_width };
                            let cell_height = row_height;
                            
                            let bg_color = if row == 0 && col == 0 {
                                egui::Color32::LIGHT_GRAY
                            } else if row == 0 {
                                egui::Color32::LIGHT_GRAY
                            } else if col == 0 {
                                egui::Color32::LIGHT_GRAY
                            } else {
                                egui::Color32::WHITE
                            };
                            
                            painter.rect_filled(
                                egui::Rect::from_min_size(egui::Pos2::new(x, y), egui::vec2(cell_width, cell_height)),
                                0.0,
                                bg_color,
                            );
                            
                            if row == 0 && col > 0 {
                                painter.text(
                                    egui::Pos2::new(x + cell_width / 2.0, y + cell_height / 2.0),
                                    egui::Align2::CENTER_CENTER,
                                    col_to_letter(col),
                                    egui::FontId::default(),
                                    egui::Color32::BLACK,
                                );
                            } else if col == 0 && row > 0 {
                                painter.text(
                                    egui::Pos2::new(x + cell_width / 2.0, y + cell_height / 2.0),
                                    egui::Align2::CENTER_CENTER,
                                    row.to_string(),
                                    egui::FontId::default(),
                                    egui::Color32::BLACK,
                                );
                            } else if row > 0 && col > 0 {
                                let mut cell_content = String::new();
                                let mut is_merged_top_left = false;
                                
                                if let Some(merged_range) = sheet.get_merged_range(col, row) {
                                    if merged_range.is_top_left(col, row) {
                                        is_merged_top_left = true;
                                        if let Some(cell) = sheet.get_cell(col, row) {
                                            cell_content = cell.value.clone();
                                        }
                                    }
                                } else {
                                    if let Some(cell) = sheet.get_cell(col, row) {
                                        cell_content = cell.value.clone();
                                    }
                                }
                                
                                if is_merged_top_left {
                                    if let Some(merged_range) = sheet.get_merged_range(col, row) {
                                        let merged_col_width = (merged_range.end_col - merged_range.start_col + 1) as f32 * col_width + 
                                            (merged_range.end_col - merged_range.start_col) as f32 * border_width;
                                        let merged_row_height = (merged_range.end_row - merged_range.start_row + 1) as f32 * row_height + 
                                            (merged_range.end_row - merged_range.start_row) as f32 * border_width;
                                        
                                        let is_selected = selected_cell.is_some() && 
                                            merged_range.contains(selected_cell.unwrap().0, selected_cell.unwrap().1);
                                        
                                        if is_selected {
                                            painter.rect_filled(
                                                egui::Rect::from_min_size(
                                                    egui::Pos2::new(x, y),
                                                    egui::vec2(merged_col_width, merged_row_height),
                                                ),
                                                0.0,
                                                egui::Color32::from_rgb(173, 216, 230),
                                            );
                                        }
                                        
                                        painter.text(
                                            egui::Pos2::new(x + merged_col_width / 2.0, y + merged_row_height / 2.0),
                                            egui::Align2::CENTER_CENTER,
                                            &cell_content,
                                            egui::FontId::default(),
                                            egui::Color32::BLACK,
                                        );
                                    }
                                } else {
                                    let is_selected = selected_cell == Some((col, row));
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
                            
                            x += cell_width + border_width;
                        }
                        y += row_height + border_width;
                    }
                });
            }
        }
    }

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

    fn draw_empty_state(&mut self, ui: &mut egui::Ui) {
        ui.centered_and_justified(|ui| {
            ui.label("请通过 '文件 > 导入' 打开Excel文件");
        });
    }
}

impl eframe::App for ExcelViewer {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        Self::setup_fonts(ctx);
        
        self.draw_menu_bar(ctx);
        self.draw_import_dialog(ctx);
        
        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_error(ui);
            
            if self.excel_data.is_some() {
                self.draw_sheet_selector(ui);
                self.draw_table(ui);
                self.draw_selected_info(ui);
            } else {
                self.draw_empty_state(ui);
            }
        });
    }
}