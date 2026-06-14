//! 帮助弹窗组件
//!
//! 展示转换工具规则语法和条件格式规则说明。

use eframe::egui;

/// 帮助弹窗状态
#[derive(Debug, Clone)]
pub struct HelpPopupState {
    pub visible: bool,
}

impl Default for HelpPopupState {
    fn default() -> Self {
        Self { visible: false }
    }
}

/// 绘制帮助弹窗
pub fn draw_help_popup(ctx: &egui::Context, state: &mut HelpPopupState) {
    if !state.visible {
        return;
    }

    let mut keep_open = true;

    egui::Window::new("help_popup")
        .title_bar(false)
        .resizable(true)
        .collapsible(false)
        .open(&mut keep_open)
        .default_size(egui::vec2(660.0, 520.0))
        .show(ctx, |ui| {
            ui.set_min_width(600.0);

            // ══════ 自定义标题栏 ══════
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("帮助").size(13.0).strong());
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui.button("✖").clicked() {
                            state.visible = false;
                        }
                    },
                );
            });
            ui.separator();

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.heading("转换工具规则语法");
                    ui.add_space(4.0);

                    render_convert_rules(ui);
                    ui.add_space(16.0);
                    ui.separator();
                    ui.add_space(8.0);

                    ui.heading("条件格式规则说明");
                    ui.add_space(4.0);
                    render_cond_format_rules(ui);
                });
        });

    if !keep_open {
        state.visible = false;
    }
}

fn help_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(12.0));
}

fn help_code(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .size(12.0)
            .monospace()
            .color(egui::Color32::from_rgb(0, 100, 0)),
    );
}

fn help_title(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(12.5).strong());
}

fn render_convert_rules(ui: &mut egui::Ui) {
    // --- 基本语法 ---
    help_title(ui, "基本语法");
    help_code(ui, "源范围 -> 目标起始位置 : 方向标记 ~ ;");
    help_label(ui, "方向标记: | 纵向填充  - 横向填充");
    help_label(ui, "~ 表示按源单元格数量等量填充到目标");
    ui.add_space(6.0);

    // --- 规则 1 ---
    help_title(ui, "1. 整行 → 纵向单列");
    help_code(ui, "A2:M2 -> A1 : | ~ ;");
    help_label(ui, "将 A2~M2（一行 13 列）的数据，按列顺序纵向填入 A1 开始向下的列。");
    help_label(ui, "即: A2→A1, B2→A2, ..., M2→A13");
    ui.add_space(6.0);

    // --- 规则 2 ---
    help_title(ui, "2. 合并单元格对 → 纵向单列");
    help_code(ui, "(N1:O1):(BV1:BW1) -> A15 : | ~ ;");
    help_label(ui, "将原表从 N1:O1 合并单元格开始，到 BV1:BW1 为止的每一对合并单元格，纵向填入 A15 开始向下的列。");
    help_label(ui, "括号内每两个单元格为一个合并区域，冒号连接起止对。");
    ui.add_space(6.0);

    // --- 规则 3 ---
    help_title(ui, "3. 整列 → 横向单行（合并目标）");
    help_code(ui, "A3:A12 -> (B1:C1) : - ~ ;");
    help_label(ui, "将 A3~A12（一列 10 行）的数据，横向填入 B1（与 C1 合并）开始向右的行中。");
    help_label(ui, "即: A3→B1, A4→D1, A5→F1, ...（每次右移 2 列，为合并留空）");
    ui.add_space(6.0);

    // --- 规则 4 ---
    help_title(ui, "4. 整行 → 横向单行");
    help_code(ui, "N1:BW1 -> B14 : - ~ ;");
    help_label(ui, "将 N1~BW1 的数据，横向填入 B14 开始向右的行中。");
    help_label(ui, "即: N1→B14, O1→C14, P1→D14, ...");
    ui.add_space(6.0);

    // --- 规则 5 ---
    help_title(ui, "5. 带步长跳跃取值 → 纵向单列");
    help_code(ui, "(N3+2):BV3 -> B15 : | ~ ;");
    help_label(ui, "从 N3 开始每次跳 2 格（N3→P3→R3→...→BV3），将取到的值纵向填入 B15 开始向下的列。");
    help_label(ui, "括号内 + 号后面为步长数值。");
    ui.add_space(6.0);

    // --- 规则 6 ---
    help_title(ui, "6. 批量列范围 → 逐行横向展开");
    help_code(ui, "(A:M)3:(A:M)12 -> B(1:13):C(1:13) : - ~ ;");
    help_label(ui, "将 A~M 共 13 列、每列第 3~12 行的数据，逐列横向填入目标区域。");
    help_label(ui, "等价于 13 条逐列规则: A3:A12→(B1:C1):-~; B3:B12→(B2:C2):-~; ...");
    help_label(ui, "括号内为列范围，后跟行号范围。");
    ui.add_space(6.0);

    // --- 规则 7 ---
    help_title(ui, "7. 批量步进范围");
    help_code(ui, "(N(3:12)+2):BV(3:12) -> (B+2)15 : | ~ ;");
    help_label(ui, "从 N 列第 3 行开始，每次向右跳 2 列，直到 BV 列；同一行数据纵向填入目标列。");
    help_label(ui, "目标列也按步长递增: B→D→F→...→T。共 10 行（3~12）。");
    ui.add_space(6.0);

    // --- 自定义公式 ---
    help_title(ui, "8. 自定义公式（可选）");
    help_code(ui, "(A:M)3:(A:M)12->(B(1:13):C(1:13)),formula(B12=SUM(B15:B~),B13=SUM($C$15:$C$~)):-~;");
    help_label(ui, "在规则末尾追加 ,formula(cell=expr,cell=expr,...) 部分。");
    help_label(ui, "~ 为动态行尾占位符，自动替换为输出表最大行号。");
    help_label(ui, "自定义公式会随批量规则一起向右方向自动扩充列引用。");
    help_label(ui, "支持绝对引用 ($C$15) 和相对引用 (B15)。");
    ui.add_space(6.0);
}

fn render_cond_format_rules(ui: &mut egui::Ui) {
    // --- 规则字段 ---
    help_title(ui, "规则字段");
    help_label(ui, "规则   : 条件运算符，可选值: > < = >= <= !=");
    help_label(ui, "值     : 比较阈值，数值或文本（如 60 或 充足）");
    help_label(ui, "填充色 : HEX 颜色值（如 FFC7CE 或 #FFC7CE）");
    help_label(ui, "应用于 : 应用范围的单元格引用");
    ui.add_space(6.0);

    // --- 应用于语法 ---
    help_title(ui, "「应用于」范围语法");
    ui.add_space(2.0);
    help_code(ui, "静态范围");
    help_label(ui, "=G3:G154     相对引用范围");
    help_label(ui, "=$G$3:$G$154 绝对引用范围（$ 符号会被自动忽略）");
    help_label(ui, "=G3          单个单元格");
    help_label(ui, "=B7:AK7      行范围（第7行 B~AK 列）");
    ui.add_space(4.0);
    help_code(ui, "动态范围（自动延伸到数据边界）");
    help_label(ui, "=B7:~7       从 B7 向右到第7行最右侧数据列（如 AK7）");
    help_label(ui, "=B7:B~       从 B7 向下到 B 列最底部数据行（如 B45）");
    help_label(ui, "=A1:~        从 A1 到全表右下角");
    ui.add_space(6.0);

    // --- 示例 ---
    help_title(ui, "完整示例");
    help_code(ui, "规则: <=  值: 60  填充色: FFC7CE  应用于: =B7:~7");
    help_label(ui, "→ 第7行 B~AK 列中值 ≤60 的单元格填充浅粉色");
    ui.add_space(2.0);
    help_code(ui, "规则: =  值: 充足  填充色: C6EFCE  应用于: =B8:~8");
    help_label(ui, "→ 第8行 B~AK 列中值等于「充足」的单元格填充浅绿色");
    ui.add_space(2.0);
    help_code(ui, "规则: <=  值: 60  填充色: FFC7CE  应用于: =G3:G~");
    help_label(ui, "→ G列第3行到底部中值 ≤60 的单元格填充浅粉色");
    ui.add_space(6.0);

    // --- 注意事项 ---
    help_title(ui, "注意事项");
    help_label(ui, "• 规则按列表顺序从上到下依次匹配，后面的规则会覆盖前面的。");
    help_label(ui, "• 数值比较 (<=60) 和文本比较 (=充足) 自动识别类型。");
    help_label(ui, "• 修改规则后自动生效，无需手动保存（保存用于持久化）。");
    help_label(ui, "• 颜色值 # 前缀可选（FFC7CE 和 #FFC7CE 等效）。");
}
