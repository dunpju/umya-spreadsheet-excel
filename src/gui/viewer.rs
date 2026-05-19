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
                let hovered_cell = self.hovered_cell;
                
                egui::ScrollArea::both().show(ui, move |ui| {
                    ui.vertical(|ui| {
                        Self::draw_col_headers(ui, sheet);
                        Self::draw_rows(ui, sheet, selected_cell, hovered_cell);
                    });
                });
            }
        }
    }

    fn draw_col_headers(ui: &mut egui::Ui, sheet: &SheetData) {
        ui.horizontal(|ui| {
            ui.add_sized([60.0, 25.0], egui::Label::new(""));
            for col in 1..=sheet.max_col {
                ui.add_sized([80.0, 25.0], egui::Label::new(col_to_letter(col)));
            }
        });
    }

    fn draw_rows(ui: &mut egui::Ui, sheet: &SheetData, selected_cell: Option<(u32, u32)>, hovered_cell: Option<(u32, u32)>) {
        let mut row = 1;
        while row <= sheet.max_row {
            ui.horizontal(|ui| {
                ui.add_sized([60.0, 25.0], egui::Label::new(row.to_string()));
                
                let mut col = 1;
                while col <= sheet.max_col {
                    if sheet.is_merged_cell_covered(col, row) {
                        col += 1;
                        continue;
                    }
                    
                    let merge_range = sheet.get_merged_range(col, row);
                    
                    let cell_width = if let Some(range) = merge_range {
                        range.width() as f32 * 80.0
                    } else {
                        80.0
                    };
                    
                    let cell_height = if let Some(range) = merge_range {
                        range.height() as f32 * 25.0
                    } else {
                        25.0
                    };
                    
                    let value = if let Some(cell) = sheet.get_cell(col, row) {
                        &cell.value
                    } else {
                        ""
                    };
                    
                    let is_selected = selected_cell.map(|(r, c)| {
                        if let Some(range) = sheet.get_merged_range(r, c) {
                            range.contains(col, row)
                        } else {
                            r == col && c == row
                        }
                    }).unwrap_or(false);
                    
                    let is_hovered = hovered_cell.map(|(r, c)| {
                        if let Some(range) = sheet.get_merged_range(r, c) {
                            range.contains(col, row)
                        } else {
                            r == col && c == row
                        }
                    }).unwrap_or(false);
                    
                    let bg_color = if is_selected {
                        egui::Color32::from_rgb(173, 216, 230)
                    } else if is_hovered {
                        egui::Color32::from_rgb(240, 240, 240)
                    } else {
                        egui::Color32::WHITE
                    };
                    
                    let response = ui.add_sized([cell_width, cell_height], egui::Label::new(value));
                    
                    ui.painter().rect_filled(
                        response.rect,
                        0.0,
                        bg_color,
                    );
                    
                    ui.painter().rect_stroke(
                        response.rect,
                        0.0,
                        egui::Stroke::new(1.0, egui::Color32::GRAY),
                    );
                    
                    col += merge_range.map(|r| r.width()).unwrap_or(1);
                }
            });
            
            row += 1;
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
