//! 配置模块
//!
//! 从 `viewer.rs` 抽离的配置相关类型与 UI 组件，包括：
//! - **插入配置面板**：列配置 / 行配置两个页签，编辑合并参数与复制选项；
//! - **搜索配置对话框**：列筛选 / 行筛选两个页签，编辑搜索范围输入；
//! - **配置持久化**：读取 / 写入用户主目录下的 `~/.MyExcel/my-excel.yaml`
//!   （`insert.column` 与 `search.{column,row}` 节点，保留文件其它配置块）。
//!
//! # 与其它模块的关系
//! - `viewer.rs` 持有 `SettingsPanelState` 作为 `ExcelViewer.settings_panel` 字段，
//!   并每帧调用 [`draw_settings_panel`] / [`draw_search_config_dialog`] 渲染。
//! - `menu_bar.rs` 通过菜单项触发面板可见性与页签切换（`visible` / `active_page` /
//!   `show_search_dialog`）。
//! - `search.rs` 消费 `search.column` / `search.row` 配置作为筛选选项来源
//!   （本模块负责编辑落盘，二者通过同一份 yaml 文件解耦）。

use eframe::egui;

/// 设置面板状态
///
/// 同时承载「插入配置」与「搜索配置」两个弹窗的全部状态。
/// `Default` 会从 `~/.MyExcel/my-excel.yaml` 加载已保存的配置。
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
    /// 是否显示搜索对话框
    pub show_search_dialog: bool,
    /// 搜索：列筛选输入内容（如 "A1-A13" 或 "A1,A3"）
    pub search_column_input: String,
    /// 搜索保存成功提示计时（秒）
    pub search_save_success_timer: f32,
    /// 搜索对话框当前页签
    pub search_active_page: SearchPage,
    /// 搜索：行筛选输入内容
    pub search_row_input: String,
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
            show_search_dialog: false,
            search_column_input: String::new(),
            search_save_success_timer: 0.0,
            search_active_page: SearchPage::ColumnFilter,
            search_row_input: String::new(),
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
                // 读取 search.column 节点
                if let Some(val) = doc.get("search").and_then(|s| s.get("column")) {
                    if let Some(s) = val.as_str() {
                        self.search_column_input = s.to_string();
                    }
                }
                // 读取 search.row 节点
                if let Some(val) = doc.get("search").and_then(|s| s.get("row")) {
                    if let Some(s) = val.as_str() {
                        self.search_row_input = s.to_string();
                    }
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

    /// 保存到 search.column 节点（保留文件中已有的其他配置）
    pub fn save_search_column(&self) -> bool {
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

        // 获取或创建 search 节点，写入 column 和 row
        let doc_mapping = doc.as_mapping_mut().unwrap();
        let column_value = serde_yaml::Value::String(self.search_column_input.clone());
        let row_value = serde_yaml::Value::String(self.search_row_input.clone());
        let update_search_map = |search_map: &mut serde_yaml::Mapping| {
            search_map.insert("column".into(), column_value);
            search_map.insert("row".into(), row_value);
        };
        if let Some(search_val) = doc_mapping.get_mut(&serde_yaml::Value::String("search".into())) {
            if let Some(search_map) = search_val.as_mapping_mut() {
                update_search_map(search_map);
            } else {
                let mut search = serde_yaml::Mapping::new();
                update_search_map(&mut search);
                *search_val = serde_yaml::Value::Mapping(search);
            }
        } else {
            let mut search = serde_yaml::Mapping::new();
            update_search_map(&mut search);
            doc_mapping.insert("search".into(), serde_yaml::Value::Mapping(search));
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

/// 搜索对话框页签类型
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SearchPage {
    ColumnFilter,
    RowFilter,
}

// ===========================================================================
// UI 渲染（从 viewer.rs::ui() 抽离，仅依赖 &egui::Context + &mut SettingsPanelState）
// ===========================================================================

/// 绘制「插入配置」面板。
///
/// 仅当 `sp.visible` 为真时渲染；关闭按钮 / 外部关闭（`keep_open`）会置
/// `sp.visible = false`。包含列配置 / 行配置两个页签，列配置编辑合并参数与
/// 复制选项，保存调用 [`SettingsPanelState::save_to_file`]。
pub fn draw_settings_panel(ctx: &egui::Context, sp: &mut SettingsPanelState) {
    if !sp.visible {
        return;
    }
    let active_page = sp.active_page;
    let title = "插入配置";
    let mut keep_open = true;
    egui::Window::new("settings_panel")
        .title_bar(false)
        .open(&mut keep_open)
        .resizable(false)
        .collapsible(false)
        .default_pos(ctx.content_rect().center() - egui::vec2(190.0, 80.0))
        .show(ctx, |ui| {
            ui.set_min_width(420.0);
            // 自定义小字体标题栏
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(title).size(12.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("X").clicked() {
                        sp.visible = false;
                    }
                    if ui.button("保存").clicked() {
                        if sp.save_to_file() {
                            sp.save_success_timer = 2.0;
                        }
                    }
                    if sp.save_success_timer > 0.0 {
                        ui.label(egui::RichText::new("保存成功").size(11.0).color(egui::Color32::GREEN));
                        sp.save_success_timer -= ui.input(|i| i.stable_dt);
                    }
                });
            });
            ui.separator();
            // 选项卡切换
            ui.horizontal(|ui| {
                if ui.selectable_label(active_page == Some(SettingsPage::ColumnConfig), "列配置").clicked() {
                    sp.active_page = Some(SettingsPage::ColumnConfig);
                }
                if ui.selectable_label(active_page == Some(SettingsPage::RowConfig), "行配置").clicked() {
                    sp.active_page = Some(SettingsPage::RowConfig);
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
                                ui.add(egui::DragValue::new(&mut sp.merge_col_start)
                                    .range(0..=10000).speed(0.1));
                                ui.label("列 至");
                                ui.add(egui::DragValue::new(&mut sp.merge_col_end)
                                    .range(0..=10000).speed(0.1));
                                ui.label("列");
                                ui.separator();
                                ui.label("横向每");
                                ui.add(egui::DragValue::new(&mut sp.merge_col_group)
                                    .range(0..=1000).speed(0.1));
                                ui.label("个单元格进行合并");
                            });
                            ui.add_space(6.0);
                            // 行范围 + 合并数量在同一行
                            ui.horizontal(|ui| {
                                ui.label("行范围:");
                                ui.add(egui::DragValue::new(&mut sp.merge_row_start)
                                    .range(0..=10000).speed(0.1));
                                ui.label("行 至");
                                ui.add(egui::DragValue::new(&mut sp.merge_row_end)
                                    .range(0..=10000).speed(0.1));
                                ui.label("行");
                                ui.separator();
                                ui.label("纵向每");
                                ui.add(egui::DragValue::new(&mut sp.merge_row_group)
                                    .range(0..=1000).speed(0.1));
                                ui.label("个单元格进行合并");
                            });
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                ui.label("复制: 公式");
                                ui.checkbox(&mut sp.copy_formula, "");
                                ui.separator();
                                ui.label("样式");
                                ui.checkbox(&mut sp.copy_style, "");
                                ui.separator();
                                ui.label("值");
                                ui.checkbox(&mut sp.copy_value, "");
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
        sp.visible = false;
    }
}

/// 绘制「搜索配置」对话框。
///
/// 仅当 `sp.show_search_dialog` 为真时渲染；关闭会置 `sp.show_search_dialog = false`。
/// 包含列筛选 / 行筛选两个页签，编辑搜索范围输入，保存调用
/// [`SettingsPanelState::save_search_column`]。
pub fn draw_search_config_dialog(ctx: &egui::Context, sp: &mut SettingsPanelState) {
    if !sp.show_search_dialog {
        return;
    }
    let mut keep_open = true;
    let active_page = sp.search_active_page;
    egui::Window::new("search_dialog")
        .title_bar(false)
        .open(&mut keep_open)
        .resizable(false)
        .collapsible(false)
        .default_pos(ctx.content_rect().center() - egui::vec2(190.0, 80.0))
        .show(ctx, |ui| {
            ui.set_min_width(420.0);
            // 自定义小字体标题栏
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("搜索配置").size(12.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("X").clicked() {
                        sp.show_search_dialog = false;
                    }
                    if ui.button("保存").clicked() {
                        if sp.save_search_column() {
                            sp.search_save_success_timer = 2.0;
                        }
                    }
                    if sp.search_save_success_timer > 0.0 {
                        ui.label(egui::RichText::new("保存成功").size(11.0).color(egui::Color32::GREEN));
                        sp.search_save_success_timer -= ui.input(|i| i.stable_dt);
                    }
                });
            });
            ui.separator();
            // 选项卡切换
            ui.horizontal(|ui| {
                if ui.selectable_label(active_page == SearchPage::ColumnFilter, "列筛选").clicked() {
                    sp.search_active_page = SearchPage::ColumnFilter;
                }
                if ui.selectable_label(active_page == SearchPage::RowFilter, "行筛选").clicked() {
                    sp.search_active_page = SearchPage::RowFilter;
                }
            });
            ui.separator();

            match active_page {
                SearchPage::ColumnFilter => {
                    ui.vertical(|ui| {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.label("单元格范围:");
                                ui.add(
                                    egui::TextEdit::singleline(&mut sp.search_column_input)
                                        .desired_width(f32::INFINITY)
                                        .hint_text("例如: A1-A13 或 A1,A3"),
                                );
                            });
                            ui.add_space(4.0);
                            ui.colored_label(
                                egui::Color32::GRAY,
                                egui::RichText::new("支持范围格式（A1-A13）和离散格式（A1,A3）").size(11.0),
                            );
                        });
                    });
                }
                SearchPage::RowFilter => {
                    ui.vertical(|ui| {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.label("行搜索标题范围:");
                                ui.add(
                                    egui::TextEdit::singleline(&mut sp.search_row_input)
                                        .desired_width(f32::INFINITY)
                                        .hint_text("例如: A14,B14 或 D14-F14"),
                                );
                            });
                            ui.add_space(4.0);
                            ui.colored_label(
                                egui::Color32::GRAY,
                                egui::RichText::new("支持单元格引用（A14）、范围格式（D14-F14）和离散格式（A14,B14）").size(11.0),
                            );
                        });
                    });
                }
            }
        });
    if !keep_open {
        sp.show_search_dialog = false;
    }
}
