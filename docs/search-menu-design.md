# 「搜索」菜单详细实现方案

> 项目：umya-spreadsheet-excel  
> 日期：2026-06-09  
> 状态：待确认

---

## 一、架构概览

```
┌──────────────────────────────────────────────────────────┐
│  menu_bar.rs                        搜索 菜单            │
│  ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐          │
│  │ 文件 │ │ 编辑 │ │ 搜索 │ │ 设置 │ │ 关于 │          │
│  └──────┘ └──────┘ └──┬───┘ └──────┘ └──────┘          │
│                       │ 点击                              │
│                       ▼                                  │
│              search_window (非模态浮窗)                   │
│              ┌──────────────────────┐                    │
│              │ 🔄  🔍    搜索    ✖ │  ← 标题栏           │
│              ├──────────────────────┤                    │
│              │ 列筛选: [▼ 序号(A1)]│  ← 下拉框           │
│              │ 关键字: [__________]│  ← 输入框           │
│              │ 匹配: X/Y 列         │  ← 统计信息         │
│              └──────────────────────┘                    │
├──────────────────────────────────────────────────────────┤
│  ExcelViewer                                             │
│  ├── search_window: SearchWindowState  (新增)            │
│  └── hidden_columns: HashSet<u32>      (新增)            │
├──────────────────────────────────────────────────────────┤
│  table.rs  渲染时跳过 hidden_columns 中的列               │
│  处理合并单元格：跨列合并 → 一并隐藏                       │
└──────────────────────────────────────────────────────────┘
```

### 数据流

```
~/.MyExcel/my-excel.yaml          Excel 工作表数据
  search.column: "A1-A13"    +    A1="序号", A2="名称" ...
         │                              │
         └──────────┬───────────────────┘
                    ▼
         load_column_options()  解析 → 下拉选项
                    │
         ┌──────────┼──────────┐
         ▼          ▼          ▼
    选择列筛选   输入关键字   点击搜索
         │          │          │
         └──────────┼──────────┘
                    ▼
           execute_search()
           ├── 收集目标行右侧单元格值
           ├── 模糊匹配 → 标记隐藏列
           ├── 处理合并单元格跨列
           └── 写入 hidden_columns
                    │
                    ▼
           table.rs 渲染时跳过隐藏列
```

---

## 二、数据结构定义

### 2.1 `SearchColumnOption` — 下拉选项

```rust
/// 单个下拉选项
#[derive(Debug, Clone)]
pub struct SearchColumnOption {
    /// 显示文本：单元格的值，如 "序号"、"名称"
    pub title: String,
    /// 单元格引用字符串，如 "A1"、"A2"
    pub cell_ref: String,
    /// 列号（1-based）
    pub col: u32,
    /// 行号（1-based）
    pub row: u32,
}
```

### 2.2 `SearchWindowState` — 搜索窗口状态（新增文件 `src/gui/widgets/search.rs`）

```rust
use std::collections::HashSet;

/// 搜索窗口状态
#[derive(Debug)]
pub struct SearchWindowState {
    // ========== 窗口控制 ==========
    /// 搜索窗口是否可见
    pub visible: bool,

    // ========== 下拉框数据 ==========
    /// 从配置 + 单元格数据解析出的下拉选项列表
    pub column_options: Vec<SearchColumnOption>,
    /// 当前选中的选项索引（0-based）
    pub selected_index: usize,
    /// 下拉框选项是否已加载（避免每帧重新解析配置）
    pub options_loaded: bool,

    // ========== 搜索输入 ==========
    /// 搜索关键字
    pub search_keyword: String,

    // ========== 搜索状态 ==========
    /// 是否已执行搜索（搜索结果生效中）
    pub is_searching: bool,
    /// 搜索匹配的列数
    pub matched_count: usize,
    /// 被搜索的总列数
    pub total_searched: usize,
    /// 是否使用二分查找（自动检测，仅供参考）
    pub use_binary_search: bool,
}
```

### 2.3 `ExcelViewer` 新增字段（修改 `src/gui/viewer.rs`）

在 `ExcelViewer` 结构体中新增：

```rust
pub struct ExcelViewer {
    // ... 现有字段保持不变 ...

    /// 搜索窗口状态
    pub search_window: SearchWindowState,
    /// 隐藏的列号集合（1-based），由搜索功能写入，table 渲染时读取
    pub hidden_columns: HashSet<u32>,
}
```

### 2.4 `Default` 实现

```rust
impl Default for SearchWindowState {
    fn default() -> Self {
        Self {
            visible: false,
            column_options: Vec::new(),
            selected_index: 0,
            options_loaded: false,
            search_keyword: String::new(),
            is_searching: false,
            matched_count: 0,
            total_searched: 0,
            use_binary_search: false,
        }
    }
}
```

---

## 三、配置解析

### 3.1 配置文件格式（`~/.MyExcel/my-excel.yaml`）

```yaml
search:
  column: "A1-A13"    # 定义可供列筛选的单元格坐标范围
```

**支持两种格式：**

| 格式 | 示例 | 说明 |
|------|------|------|
| 范围格式 | `"A1-A13"` | A列第1行到第13行 |
| 离散格式 | `"A1,A3,B5"` | 逗号分隔的独立单元格 |

### 3.2 解析流程

```
配置文件 search.column: "A1-A13"
  │
  ▼ parse_search_range("A1-A13")
  │
  ▼ 产生 Vec<(col, row)> = [(1,1), (1,2), (1,3), ..., (1,13)]
  │
  ▼ 对每个 (col, row) 读取已加载 Excel 数据中对应 cell.value
  │
  ▼ 构建 Vec<SearchColumnOption>
    [
      { title: "序号", cell_ref: "A1", col: 1, row: 1 },
      { title: "名称", cell_ref: "A2", col: 1, row: 2 },
      { title: "规格", cell_ref: "A3", col: 1, row: 3 },
      ...
    ]
```

### 3.3 解析函数

```rust
/// 解析单元格引用字符串，如 "A1" → (col: 1, row: 1), "AB13" → (col: 28, row: 13)
fn parse_cell_ref(s: &str) -> Option<(u32, u32)>;

/// 解析范围或离散格式字符串，返回所有单元格坐标
/// "A1-A13" → [(1,1), (1,2), ..., (1,13)]
/// "A1,A3,B5" → [(1,1), (1,3), (2,5)]
fn parse_search_range(input: &str) -> Vec<(u32, u32)>;

/// 从配置文件和 Excel 数据加载下拉选项
fn load_column_options(
    excel_data: &ExcelData,
    current_sheet: usize,
) -> Vec<SearchColumnOption>;
```

### 3.4 加载时机

| 事件 | 行为 |
|------|------|
| 搜索窗口首次打开（有数据） | 加载选项，设置 `options_loaded = true` |
| 切换工作表后打开搜索窗口 | 重新加载选项 |
| 导入新文件后 | 重置 `options_loaded = false`，下次打开时重新加载 |

---

## 四、搜索算法

### 4.1 算法入口

```
输入: selected_option=(col, row), keyword, max_col
输出: hidden_columns: HashSet<u32>

步骤:
  1. 确定搜索范围
     target_row = row  （下拉选项所在行）
     search_cols = [col+1, col+2, ..., max_col]  （选中列右侧所有列）

  2. 收集单元格值
     for each c in search_cols:
       val[c] = sheet.get_cell(target_row, c).value.to_lowercase()

  3. 检测是否可二分
     is_sorted = val 序列是否单调非递减

  4. 执行匹配
     if is_sorted && 前缀/精确匹配:
       二分查找 → 确定匹配区间 [L, R]
       hidden_columns = 不在 [L, R] 内的列
     else:
       线性扫描 → 逐个模糊匹配
       不匹配的列 → hidden_columns.insert(c)

  5. 处理合并单元格
     expand_hidden_for_merged_cells(sheet, hidden_columns, target_row)

  6. 确保选定列自身不被隐藏
     hidden_columns.remove(col)
```

### 4.2 模糊匹配规则

```rust
/// 模糊匹配：大小写不敏感的子串匹配
fn fuzzy_match(cell_value: &str, keyword: &str) -> bool {
    cell_value.to_lowercase().contains(&keyword.to_lowercase())
}
```

示例：
| 单元格值 | 关键字 | 匹配？ |
|----------|--------|--------|
| `"序号"` | `"序"` | ✅ |
| `"HelloWorld"` | `"world"` | ✅ |
| `"ABC"` | `"abc"` | ✅ |
| `"123"` | `"2"` | ✅ |
| `"苹果"` | `"香蕉"` | ❌ |

### 4.3 二分查找（排序数据优化）

```rust
/// 在已排序的列值数组中，二分查找第一个匹配的索引
/// 适用于前缀匹配（startswith）场景
fn binary_search_first_match(
    col_values: &[(u32, String)],  // (col, value)，按 value 排序
    keyword: &str,
) -> Option<usize> {
    let mut lo = 0;
    let mut hi = col_values.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        if col_values[mid].1.as_str() < keyword {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo < col_values.len() && col_values[lo].1.contains(keyword) {
        Some(lo)
    } else {
        None
    }
}

/// 二分查找最后一个匹配的索引
fn binary_search_last_match(
    col_values: &[(u32, String)],
    keyword: &str,
    start: usize,
) -> usize {
    let mut lo = start;
    let mut hi = col_values.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        let val = &col_values[mid].1;
        // 比较前缀：取 keyword 长度的前缀
        let prefix = if val.len() >= keyword.len() {
            &val[..keyword.len()]
        } else {
            val
        };
        if prefix <= keyword {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo.saturating_sub(1)
}
```

> ⚠️ **二分查找的局限性说明**：二分查找要求数据**已排序**，且在**模糊子串匹配**（`contains`）场景下收益有限，因为 `contains` 不满足单调性。实际策略：
> - 数据已排序 + 前缀匹配 → 二分 O(log n) + 区间确认
> - 数据已排序 + 模糊匹配 → 二分缩小候选范围 + 双向线性扩展
> - 数据未排序 → 全量线性扫描 O(n)
>
> n 为列数，通常 ≤ 几百列，线性扫描完全可接受。

### 4.4 合并单元格跨列处理

```
场景示例:
  B1 和 C1 合并了 (merged_range: start_col=2, end_col=3, start_row=1, end_row=1)

搜索: selected_cell = A1, keyword = "1"
  B1 值 = "2" → 不匹配 → hidden_columns.insert(2)
  → 检测 col=2, row=1 在合并范围 (B1:C1, cols=[2,3]) 内
  → 扩展: hidden_columns.insert(3)
  → 结果: B 列和 C 列同时隐藏 ✓
```

```rust
/// 如果隐藏列属于跨列合并，将整个合并范围的列都加入隐藏集合
fn expand_hidden_for_merged_cells(
    sheet: &SheetData,
    hidden_columns: &mut HashSet<u32>,
    target_row: u32,
) {
    let mut to_add = Vec::new();
    for mr in &sheet.merged_cells {
        // 只处理跨列合并
        if mr.start_col == mr.end_col {
            continue;
        }
        // 只处理包含目标行的合并
        if target_row < mr.start_row || target_row > mr.end_row {
            continue;
        }
        // 检查是否有隐藏列在此合并范围内
        for &col in hidden_columns.iter() {
            if col >= mr.start_col && col <= mr.end_col {
                for c in mr.start_col..=mr.end_col {
                    to_add.push(c);
                }
            }
        }
    }
    for c in to_add {
        hidden_columns.insert(c);
    }
}
```

---

## 五、列隐藏机制

### 5.1 数据流

```
SearchWindowState
  → execute_search() 计算 hidden_columns
  → 写入 ExcelViewer.hidden_columns: HashSet<u32>
  → 传递给 draw_table_content(ui, hidden_columns)
  → 渲染时过滤
```

### 5.2 `draw_table_content` 函数签名变更

```rust
// src/gui/widgets/table.rs

pub fn draw_table_content(
    ui: &mut egui::Ui,
    excel_data: &mut ExcelData,
    current_sheet: usize,
    selected_cell: &mut Option<(u32, u32)>,
    selected_range: &mut Option<(u32, u32, u32, u32)>,
    editing_cell: &mut Option<(u32, u32)>,
    edit_value: &mut String,
    just_entered_edit_mode: &mut bool,
    validation_error: &mut Option<(String, String)>,
    original_cell_data: &mut Option<((u32, u32), String, String)>,
    context_menu: &mut crate::gui::viewer::ContextMenuState,
    dirty: &mut bool,
    drag_anchor: &mut Option<(u32, u32)>,
    hidden_columns: &HashSet<u32>,  // ← 新增参数
) -> (Option<egui::Rect>, Option<egui::Rect>) { ... }
```

### 5.3 渲染过滤实现

```rust
// 在列头绘制循环中:
for col in 1..=max_col {
    if hidden_columns.contains(&col) {
        continue;  // 跳过隐藏列
    }
    // ... 原有列头绘制逻辑 ...
}

// 在数据行绘制循环中:
for col in 1..=max_col {
    if hidden_columns.contains(&col) {
        continue;
    }
    // ... 原有单元格绘制逻辑 ...
}
```

### 5.4 `col_cumulative_width` 调整

```rust
// 构建累积宽度时跳过隐藏列
let mut col_cumulative_width = vec![0.0];
let mut cur_w = 0.0;
cur_w += header_width + border_width;
col_cumulative_width.push(cur_w);
for col in 1..=max_col {
    if hidden_columns.contains(&col) {
        col_cumulative_width.push(cur_w);  // 宽度不增长
        continue;
    }
    cur_w += get_col_width(col) + border_width;
    col_cumulative_width.push(cur_w);
}
```

---

## 六、UI 组件设计

### 6.1 菜单栏修改（`src/gui/widgets/menu_bar.rs`）

```rust
pub fn draw_menu_bar(
    ui: &mut egui::Ui,
    show_import_dialog: &mut bool,
    settings_panel: &mut SettingsPanelState,
    search_window: &mut SearchWindowState,  // ← 新增参数
    add_column: &mut bool,
    add_row: &mut bool,
    has_data: bool,
) {
    egui::MenuBar::new().ui(ui, |ui| {
        // ── 文件菜单（保持原有）──
        ui.menu_button("文件", |ui| {
            if ui.button("导入").clicked() {
                ui.close();
                *show_import_dialog = true;
            }
            ui.add_enabled(false, egui::Button::new("模板"));
        });

        // ── 编辑菜单（保持原有）──
        ui.menu_button("编辑", |ui| {
            if ui.add_enabled(has_data, egui::Button::new("添加列")).clicked() {
                ui.close();
                *add_column = true;
            }
            if ui.add_enabled(has_data, egui::Button::new("添加行")).clicked() {
                ui.close();
                *add_row = true;
            }
        });

        // ── ★ 搜索菜单（新增，插入在编辑和设置之间）──
        ui.menu_button("搜索", |ui| {
            if ui.add_enabled(has_data, egui::Button::new("搜索")).clicked() {
                ui.close();
                search_window.visible = true;
                search_window.options_loaded = false; // 触发重新加载下拉选项
            }
            // 快捷键提示（仅在无数据时灰显，不可点击）
            if !has_data {
                ui.add_enabled(false, egui::Button::new("Ctrl+F"));
            }
        });

        // ── 设置菜单（保持原有）──
        ui.menu_button("设置", |ui| { ... });

        // ── 关于菜单（保持原有）──
        ui.menu_button("关于", |ui| { ... });
    });
}
```

### 6.2 搜索窗口布局

```
┌─────────────────────────────────────────────┐
│  搜索                 匹配 3/10 列  🔄  🔍  ✖ │  ← 自定义标题栏
├─────────────────────────────────────────────┤
│  列筛选:  [▼ 序号(A1)                ]      │  ← ComboBox 下拉框
│  关键字:  [___________________________]      │  ← TextEdit 输入框
├─────────────────────────────────────────────┤
│  💡 提示: 在已排序数据中自动启用二分查找       │  ← 底部提示（小字灰色）
└─────────────────────────────────────────────┘
```

#### 布局规则

- 标题栏左侧：`"搜索"` 标题文字
- 标题栏中间：搜索统计信息（仅搜索后显示，如 `"匹配 3/10 列"`）
- 标题栏右侧（从右到左排列）：
  - `✖` 关闭按钮（最右）
  - `🔍 搜索` 搜索按钮（关闭按钮左侧）
  - `🔄 重置` 重置按钮（搜索按钮左侧）
- 内容区：
  - 第一行：列筛选标签 + ComboBox 下拉框
  - 第二行：关键字标签 + 输入框
  - 底部：辅助提示信息

### 6.3 窗口渲染入口函数

```rust
/// 绘制搜索窗口（放在 src/gui/widgets/search.rs）
///
/// # 参数
/// * `ctx` - egui 上下文
/// * `state` - 搜索窗口状态（可变引用）
/// * `excel_data` - Excel 数据（只读引用）
/// * `current_sheet` - 当前工作表索引
/// * `hidden_columns` - 隐藏列集合（可变引用，搜索执行时修改）
pub fn draw_search_window(
    ctx: &egui::Context,
    state: &mut SearchWindowState,
    excel_data: Option<&ExcelData>,
    current_sheet: usize,
    hidden_columns: &mut HashSet<u32>,
) {
    if !state.visible {
        return;
    }

    let mut keep_open = true;
    egui::Window::new("search_window")
        .title_bar(false)       // 自定义标题栏
        .open(&mut keep_open)
        .resizable(false)
        .collapsible(false)
        // 非模态：不阻塞主窗口交互
        .default_pos(ctx.content_rect().center() - egui::vec2(200.0, 60.0))
        .show(ctx, |ui| {
            ui.set_min_width(420.0);

            // ══════ 自定义标题栏 ══════
            ui.horizontal(|ui| {
                // 标题
                ui.label(egui::RichText::new("搜索").size(13.0).strong());

                // 统计信息（搜索后显示）
                if state.is_searching {
                    ui.add_space(12.0);
                    ui.label(
                        egui::RichText::new(
                            format!("匹配 {}/{} 列",
                                state.matched_count,
                                state.total_searched
                            )
                        )
                        .size(11.0)
                        .color(egui::Color32::from_rgb(0, 130, 0)),
                    );
                    if state.use_binary_search {
                        ui.label(
                            egui::RichText::new("(二分)")
                                .size(10.0)
                                .color(egui::Color32::from_rgb(100, 100, 100)),
                        );
                    }
                }

                // 右侧按钮（从右到左）
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // 关闭按钮
                    if ui.button("✖").clicked() {
                        state.visible = false;
                    }

                    // 搜索按钮
                    let can_search = excel_data.is_some()
                        && state.selected_index < state.column_options.len()
                        && !state.search_keyword.is_empty();
                    if ui.add_enabled(can_search, egui::Button::new("🔍 搜索")).clicked() {
                        if let Some(data) = excel_data {
                            if let Some(sheet) = data.get_sheet(current_sheet) {
                                execute_search(state, sheet, hidden_columns);
                            }
                        }
                    }

                    // 重置按钮
                    if ui.button("🔄 重置").clicked() {
                        hidden_columns.clear();
                        state.is_searching = false;
                        state.matched_count = 0;
                        state.total_searched = 0;
                        state.search_keyword.clear();
                    }
                });
            });
            ui.separator();

            // ══════ 内容区 ══════
            // 延迟加载下拉选项
            if !state.options_loaded {
                if let Some(data) = excel_data {
                    state.column_options = load_column_options(data, current_sheet);
                    state.options_loaded = true;
                    if !state.column_options.is_empty() && state.selected_index >= state.column_options.len() {
                        state.selected_index = 0;
                    }
                }
            }

            // 列筛选下拉框
            ui.horizontal(|ui| {
                ui.label("列筛选:");
                let selected_text = state.column_options
                    .get(state.selected_index)
                    .map(|o| format!("{} ({})", o.title, o.cell_ref))
                    .unwrap_or_else(|| "请选择列...".to_string());
                egui::ComboBox::from_id_salt("search_column_select")
                    .selected_text(&selected_text)
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        for (i, opt) in state.column_options.iter().enumerate() {
                            let label = format!("{} ({})", opt.title, opt.cell_ref);
                            if ui.selectable_label(i == state.selected_index, &label).clicked() {
                                state.selected_index = i;
                            }
                        }
                    });
            });

            ui.add_space(6.0);

            // 搜索关键字输入框
            ui.horizontal(|ui| {
                ui.label("关键字:");
                let input = egui::TextEdit::singleline(&mut state.search_keyword)
                    .desired_width(f32::INFINITY)
                    .hint_text("输入搜索关键字...");
                let response = ui.add(input);

                // Enter 键触发搜索
                if response.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                {
                    if let Some(data) = excel_data {
                        if let Some(sheet) = data.get_sheet(current_sheet) {
                            if state.selected_index < state.column_options.len()
                                && !state.search_keyword.is_empty()
                            {
                                execute_search(state, sheet, hidden_columns);
                            }
                        }
                    }
                }
            });

            // 底部提示
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("💡 搜索范围：选中列右侧所有列；已排序数据自动启用二分查找")
                    .size(10.0)
                    .color(egui::Color32::from_rgb(140, 140, 140)),
            );
        });

    // 窗口关闭
    if !keep_open {
        state.visible = false;
    }
}
```

---

## 七、核心函数实现

### 7.1 `execute_search` — 搜索执行

```rust
/// 执行搜索操作
fn execute_search(
    state: &mut SearchWindowState,
    sheet: &SheetData,
    hidden_columns: &mut HashSet<u32>,
) {
    hidden_columns.clear();

    let opt = match state.column_options.get(state.selected_index) {
        Some(o) => o,
        None => return,
    };

    let keyword = state.search_keyword.to_lowercase();
    let target_col = opt.col;
    let target_row = opt.row;

    // 收集搜索范围内所有列的头值
    let mut col_values: Vec<(u32, String)> = Vec::new();
    let max_col = sheet.max_col;
    for col in (target_col + 1)..=max_col {
        let value = sheet
            .get_cell(target_row, col)
            .map(|c| c.value.to_lowercase())
            .unwrap_or_default();
        col_values.push((col, value));
    }

    state.total_searched = col_values.len();

    if col_values.is_empty() {
        state.matched_count = 0;
        state.is_searching = true;
        return;
    }

    // 检测是否已排序
    let is_sorted = col_values.windows(2).all(|w| w[0].1 <= w[1].1);
    state.use_binary_search = is_sorted;

    if is_sorted {
        // 二分查找缩小区间 + 线性确认
        search_sorted(&col_values, &keyword, hidden_columns);
    } else {
        // 线性扫描
        for (col, value) in &col_values {
            if !value.contains(&keyword) {
                hidden_columns.insert(*col);
            }
        }
    }

    // 处理合并单元格跨列
    expand_hidden_for_merged_cells(sheet, hidden_columns, target_row);

    // 确保选中列自身不被隐藏
    hidden_columns.remove(&target_col);

    state.matched_count = state.total_searched.saturating_sub(hidden_columns.len());
    state.is_searching = true;
}
```

### 7.2 `search_sorted` — 二分优化搜索

```rust
/// 在已排序的列值中搜索匹配区间
fn search_sorted(
    col_values: &[(u32, String)],
    keyword: &str,
    hidden_columns: &mut HashSet<u32>,
) {
    // 二分定位第一个可能匹配的索引
    let mut lo = 0;
    let mut hi = col_values.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        if col_values[mid].1.as_str() < keyword {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }

    // 从二分位置双向扩展，找到所有模糊匹配的列
    let mut matched_indices = HashSet::new();

    // 向右扩展
    let mut i = lo;
    while i < col_values.len() {
        let val = &col_values[i].1;
        if val.contains(keyword) {
            matched_indices.insert(i);
        } else if val.as_str() > keyword
            && !val.starts_with(&keyword[..keyword.len().min(val.len())])
        {
            // 前缀已超过 keyword，右侧不再可能有匹配
            break;
        }
        i += 1;
    }

    // 向左扩展
    if lo > 0 {
        let mut i = lo.saturating_sub(1);
        loop {
            let val = &col_values[i].1;
            if val.contains(keyword) {
                matched_indices.insert(i);
            } else if val.as_str() < keyword
                && keyword.starts_with(&val[..val.len().min(keyword.len())])
            {
                // 仍在边界内
            } else {
                break;
            }
            if i == 0 {
                break;
            }
            i -= 1;
        }
    }

    // 未匹配的列 → 隐藏
    for (idx, (col, _)) in col_values.iter().enumerate() {
        if !matched_indices.contains(&idx) {
            hidden_columns.insert(*col);
        }
    }
}
```

### 7.3 `load_column_options` — 加载下拉选项

```rust
/// 从配置文件和 Excel 数据加载下拉选项
fn load_column_options(
    excel_data: &ExcelData,
    current_sheet: usize,
) -> Vec<SearchColumnOption> {
    let sheet = match excel_data.get_sheet(current_sheet) {
        Some(s) => s,
        None => return Vec::new(),
    };

    // 读取配置
    let config_path = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".MyExcel")
        .join("my-excel.yaml");

    let range_str = if config_path.exists() {
        std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|c| serde_yaml::from_str::<serde_yaml::Value>(&c).ok())
            .and_then(|doc| {
                doc.get("search")
                    .and_then(|s| s.get("column"))
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
            })
            .unwrap_or_default()
    } else {
        String::new()
    };

    if range_str.is_empty() {
        return Vec::new();
    }

    // 解析范围字符串
    let cells = parse_search_range(&range_str);

    // 读取每个单元格的值构建选项
    cells
        .into_iter()
        .map(|(col, row)| {
            let title = sheet
                .get_cell(row, col)
                .map(|c| c.value.clone())
                .unwrap_or_default();
            let col_letter = crate::excel::reader::col_to_letter(col);
            let cell_ref = format!("{}{}", col_letter, row);
            SearchColumnOption {
                title,
                cell_ref,
                col,
                row,
            }
        })
        .collect()
}
```

### 7.4 `parse_search_range` — 范围解析

```rust
/// 解析搜索范围字符串
/// "A1-A13" → [(1,1), (1,2), ..., (1,13)]
/// "A1,A3,B5" → [(1,1), (1,3), (2,5)]
fn parse_search_range(input: &str) -> Vec<(u32, u32)> {
    let input = input.trim();
    if input.is_empty() {
        return Vec::new();
    }

    // 处理逗号分隔的离散格式
    if input.contains(',') {
        return input
            .split(',')
            .filter_map(|s| parse_cell_ref(s.trim()))
            .collect();
    }

    // 处理范围格式 "A1-A13"
    if let Some(idx) = input.find('-') {
        // 检查是否是范围格式（排除负数）
        let after_dash = &input[idx + 1..];
        if after_dash.chars().next().map_or(false, |c| c.is_alphabetic()) {
            let start_str = &input[..idx];
            let end_str = after_dash;
            if let (Some(start), Some(end)) = (parse_cell_ref(start_str), parse_cell_ref(end_str)) {
                let mut result = Vec::new();
                if start.0 == end.0 {
                    // 同列：行范围
                    for row in start.1..=end.1 {
                        result.push((start.0, row));
                    }
                } else if start.1 == end.1 {
                    // 同行：列范围
                    for col in start.0..=end.0 {
                        result.push((col, start.1));
                    }
                }
                return result;
            }
        }
    }

    // 单个单元格
    parse_cell_ref(input).into_iter().collect()
}

/// 解析单个单元格引用 "A1" → (col: 1, row: 1)
fn parse_cell_ref(s: &str) -> Option<(u32, u32)> {
    let s = s.trim().to_uppercase();
    let col_part: String = s.chars().take_while(|c| c.is_alphabetic()).collect();
    let row_part: String = s.chars().skip_while(|c| c.is_alphabetic()).collect();

    if col_part.is_empty() || row_part.is_empty() {
        return None;
    }

    // 列字母 → 数字 (A=1, B=2, ..., Z=26, AA=27, ...)
    let col = col_part
        .chars()
        .fold(0u32, |acc, c| acc * 26 + (c as u32 - 'A' as u32 + 1));

    let row = row_part.parse::<u32>().ok()?;

    if col == 0 || row == 0 {
        return None;
    }

    Some((col, row))
}
```

---

## 八、文件修改清单

| # | 文件 | 操作 | 说明 |
|---|------|------|------|
| 1 | **`src/gui/widgets/search.rs`** | **新建** | 搜索窗口全部逻辑：状态定义、UI 渲染、搜索算法、配置解析 |
| 2 | `src/gui/widgets/mod.rs` | 修改 | 添加 `pub mod search;` 和 `pub use search::*;` |
| 3 | `src/gui/widgets/menu_bar.rs` | 修改 | 在编辑/设置菜单之间插入"搜索"菜单；函数签名新增 `search_window` 参数 |
| 4 | `src/gui/viewer.rs` | 修改 | `ExcelViewer` 新增 `search_window` / `hidden_columns` 字段；`ui()` 中调用 `draw_search_window`；`draw_menu_bar` 调用传入新参数；`draw_table_content` 调用传入 `hidden_columns` |
| 5 | `src/gui/widgets/table.rs` | 修改 | `draw_table_content` 新增 `hidden_columns` 参数；列/单元格渲染时跳过隐藏列；`col_cumulative_width` 计算时跳过隐藏列 |

### 修改量预估

| 文件 | 新增行 | 修改行 | 说明 |
|------|--------|--------|------|
| `search.rs` | ~300 | 0 | 全新文件 |
| `mod.rs` | +2 | 0 | 模块注册 |
| `menu_bar.rs` | +8 | +2 | 新菜单项 + 新参数 |
| `viewer.rs` | +20 | +5 | 新字段 + 窗口渲染 + 参数传递 |
| `table.rs` | +8 | +4 | 参数 + 循环过滤 |
| **合计** | **~338** | **~11** | |

---

## 九、事件与状态流转

### 9.1 完整生命周期

```
                         应用启动
                           │
                           ▼
              ExcelViewer::new()
              ├── search_window = SearchWindowState::default()
              └── hidden_columns = HashSet::new()
                           │
                    用户导入 Excel 文件
                           │
                           ▼
              has_data = true → 菜单"搜索"可用
                           │
              用户点击 "搜索 → 搜索"
                           │
                           ▼
              visible = true
              options_loaded = false  ← 触发加载
                           │
                           ▼
              draw_search_window() 首次渲染
              └── load_column_options()
                  读取 yaml 配置 + 单元格值
                  → 构建下拉选项列表
                  options_loaded = true
                           │
         ┌─────────────────┼─────────────────┐
         ▼                 ▼                  ▼
    选择列筛选        输入关键字           点击 [搜索]
    (ComboBox)     (TextEdit)          (或按 Enter)
         │                 │                  │
         └─────────────────┼──────────────────┘
                           ▼
                  execute_search()
                  ├── 收集目标行右侧各列值
                  ├── 检测排序 → 选择策略
                  ├── 模糊匹配 → hidden_columns
                  ├── expand_hidden_for_merged_cells()
                  └── is_searching = true
                           │
                           ▼
              table.rs 渲染时跳过 hidden_columns
              用户看到筛选后的表格
                           │
              ┌────────────┼────────────┐
              ▼            ▼            ▼
         [重置]按钮    [✖]关闭窗口   重新搜索
         hidden_columns  visible=false  更新 hidden_columns
         .clear()        隐藏列保持      覆盖旧结果
         is_searching    (不清除)
         =false
              │
              ▼
         表格恢复完整显示
```

### 9.2 状态一致性保证

| 场景 | 行为 |
|------|------|
| 搜索窗口关闭后再打开 | 隐藏列保持（不清除），可继续查看/重置 |
| 切换工作表 | 重置 `options_loaded = false`，`hidden_columns.clear()` |
| 导入新文件 | `hidden_columns.clear()`，`options_loaded = false` |
| 关闭应用再启动 | 所有状态重置（不做持久化） |
| 搜索时未匹配任何列 | 所有列隐藏，`matched_count = 0`，用户能看到空表格并重置 |

---

## 十、设计要点补充

### 10.1 非模态窗口特性

- 使用 `egui::Window` 默认行为（非模态）
- 用户可以同时操作主窗口和搜索窗口
- 搜索窗口可被拖动到任意位置
- 搜索窗口关闭后，隐藏列状态**保持**（用户可能需要在关闭窗口后仔细查看筛选结果）

### 10.2 性能分析

| 操作 | 复杂度 | 预估耗时（1000列） |
|------|--------|-------------------|
| 配置解析 | O(k)，k=配置单元格数 | <1ms |
| 加载下拉选项 | O(k) | <1ms |
| 线性扫描搜索 | O(n)，n=列数 | <1ms |
| 二分查找搜索 | O(log n + m)，m=匹配数 | <1ms |
| 合并单元格扩展 | O(n × r)，r=合并范围数 | <1ms |
| 表格渲染（跳过隐藏列） | O(n) | 无额外开销 |

> 结论：所有搜索操作均远小于 16ms 帧时间，对 60fps 无影响。

### 10.3 与现有代码的兼容性

| 关注点 | 处理 |
|--------|------|
| `hidden_columns` 默认为空 | 不影响未使用搜索时的渲染 |
| 搜索窗口未打开时不加载配置 | 延迟加载，避免不必要的 I/O |
| 重置仅清除隐藏集合 | 不修改 Excel 数据，不会触发 `dirty` |
| 隐藏列对数据无影响 | 列数据不变，公式计算不受影响 |

### 10.4 配置完整示例

```yaml
# ~/.MyExcel/my-excel.yaml
search:
  column: "A1-A13"   # A列第1~13行作为列筛选来源

# 下载该配置时效果：
# A1="序号"  → 下拉显示 "序号 (A1)"
# A2="名称"  → 下拉显示 "名称 (A2)"
# A3="规格"  → 下拉显示 "规格 (A3)"
# ...
```

### 10.5 边界情况处理

| 边界情况 | 处理方式 |
|----------|----------|
| 配置文件中无 `search.column` 节点 | 下拉框为空，显示"请选择列..." |
| 配置文件不存在 | 同上 |
| 下拉框选项为空时点击搜索 | 搜索按钮灰显（`add_enabled(false, ...)`） |
| 搜索关键字为空 | 搜索按钮灰显 |
| 选中列是最右侧列（无右侧列） | `total_searched = 0`，无列隐藏 |
| 所有列都匹配 | `hidden_columns` 为空，表格不变 |
| 所有列都不匹配 | 所有列隐藏，用户需点击重置恢复 |
| 合并单元格跨 N 列 | N 列一起隐藏 |

---

## 十一、扩展可能（后续迭代）

以下为本次方案范围外的后续迭代方向：

1. **快捷键 Ctrl+F**：全局快捷键打开搜索窗口
2. **高亮匹配单元格**：搜索结果中高亮显示匹配的单元格值
3. **正则表达式搜索**：支持正则匹配模式
4. **搜索历史**：记录最近的搜索关键字
5. **搜索结果导出**：将筛选后的表格导出为新 Excel 文件
6. **实时搜索**：输入关键字时实时更新筛选结果（debounce）
7. **行筛选**：类似列筛选，按行方向搜索（已预留 `search_row_input` 字段）

---

> 📌 **方案待确认**：请在审阅后确认是否按此方案执行开发。如有修改意见，将在确认前调整方案。
