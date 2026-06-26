# 配置模块分析（`src/gui/widgets/config.rs`）

> 本文档描述从 `viewer.rs` 抽离的配置模块：类型定义、YAML 持久化、两个配置弹窗的 UI 渲染。
> 配套阅读：主模块见 [`viewer.md`](../viewer.md)，搜索功能见 [`search.md`](search.md)。

---

## 1. 模块定位与职责

`config.rs` 隶属于 `gui::widgets`，集中管理「插入配置」与「搜索配置」两个弹窗的**状态、UI 与持久化**。
它是从 `viewer.rs` 抽离的独立模块，职责单一：让 `viewer.rs` 不再承载配置相关的类型与渲染细节。

### 1.1 职责

- **状态容器**：`SettingsPanelState` 同时承载插入配置（合并参数、复制选项）与搜索配置（筛选范围输入）。
- **UI 渲染**：`draw_settings_panel` / `draw_search_config_dialog` 绘制两个弹窗（自定义标题栏、页签、表单）。
- **配置持久化**：读写用户主目录下的 `~/.MyExcel/my-excel.yaml`（`insert.column` 与 `search.{column,row}`）。
- **枚举**：`SettingsPage`（列配置 / 行配置）、`SearchPage`（列筛选 / 行筛选）作为页签状态。

### 1.2 依赖与被依赖

```
 ┌──────────────────── gui/widgets/config.rs ────────────────────┐
 │                                                                │
 │  外部依赖                                                      │
 │   ├─ eframe::egui       Window / DragValue / TextEdit / 布局   │
 │   ├─ dirs               home_dir（定位 yaml 路径）             │
 │   └─ serde_yaml         读写 yaml（保留其它配置块）            │
 │                                                                │
 │  被依赖（调用方）                                              │
 │   ├─ viewer.rs          ExcelViewer.settings_panel 字段        │
 │   │                     + ui() 中调用 draw_settings_panel /    │
 │   │                       draw_search_config_dialog            │
 │   └─ menu_bar.rs        菜单项触发 visible / active_page /     │
 │                         show_search_dialog（经 config 导入类型）│
 │                                                                │
 │  协作（非直接调用，经 yaml 文件解耦）                          │
 │   └─ search.rs          消费 search.column / search.row 作为    │
 │                         筛选选项来源（本模块负责编辑落盘）      │
 └────────────────────────────────────────────────────────────────┘
```

> 抽离动机：`viewer.rs` 体量庞大（约 1900 行），配置弹窗逻辑自成一体且只依赖 `&egui::Context` +
> `&mut SettingsPanelState`，抽出后 `viewer.rs::ui()` 仅以两行调用即可，降低耦合、便于单独维护。

---

## 2. 核心类型

### 2.1 `SettingsPanelState`（状态容器）

承载两个弹窗的全部状态，字段按用途分组：

| 分组 | 字段 | 作用 |
|------|------|------|
| 窗口控制 | `visible: bool`、`active_page: Option<SettingsPage>` | 插入配置弹窗可见性 + 当前页签 |
| 列合并参数 | `merge_col_start`、`merge_col_end`、`merge_col_group`（`u32`） | 列范围起止 + 横向每 N 格合并 |
| 行合并参数 | `merge_row_start`、`merge_row_end`、`merge_row_group`（`u32`） | 行范围起止 + 纵向每 N 格合并 |
| 复制选项 | `copy_formula: bool`、`copy_style: bool`、`copy_value: bool` | 插入列时复制哪些（公式/样式/值） |
| 保存提示 | `save_success_timer: f32` | 插入配置保存成功提示倒计时（秒） |
| 搜索配置窗口 | `show_search_dialog: bool`、`search_active_page: SearchPage` | 搜索配置弹窗可见性 + 页签 |
| 搜索输入 | `search_column_input: String`、`search_row_input: String` | 列筛选 / 行筛选范围输入文本 |
| 搜索保存提示 | `search_save_success_timer: f32` | 搜索配置保存成功提示倒计时（秒） |

`Default` 实现：先以代码默认值构造，再调用私有 `load_from_file()` 从 yaml 覆盖已保存的字段。
因此 `SettingsPanelState::default()` 即"启动时自动加载上次配置"。

### 2.2 `SettingsPage`（插入配置页签）

```rust
pub enum SettingsPage { ColumnConfig, RowConfig }
```

- `ColumnConfig`：编辑列/行合并参数与复制选项（已实现）；
- `RowConfig`：行配置（当前为"功能开发中"占位）。

### 2.3 `SearchPage`（搜索配置页签）

```rust
pub enum SearchPage { ColumnFilter, RowFilter }
```

- `ColumnFilter`：编辑列筛选的单元格范围（`search.column`，如 `A1-A13` 或 `A1,A3`）；
- `RowFilter`：编辑行筛选的标题范围（`search.row`，如 `A14,B14` 或 `D14-F14`）。

两个枚举均 `#[derive(Debug, Clone, Copy, PartialEq)]`，可直接用于 `selectable_label` 的选中判定。

---

## 3. 配置持久化（YAML）

配置文件固定为 `~/.MyExcel/my-excel.yaml`（`dirs::home_dir` 回退 `.`）。读写都**保留文件中已有的其它配置块**，只覆写目标节点。

### 3.1 文件结构

```yaml
insert:
  column:
    col_start: 1
    col_end: 10
    col_group: 2
    row_start: 1
    row_end: 20
    row_group: 2
    copy_formula: true
    copy_style: false
    copy_value: false
search:
  column: "A1-A13"
  row: "A14,B14"
# 也支持步长语法与 ~末尾占位：
# search.column: "A(1:+2):A13"
# search.row: "(B:+2)14:~14"
```

### 3.2 方法

| 方法 | 可见性 | 作用 |
|------|--------|------|
| `config_path()` | 私有 | 返回 `~/.MyExcel/my-excel.yaml` 路径 |
| `load_from_file()` | 私有 | 读取 `insert.column`（合并参数 + 复制开关）与 `search.column`/`search.row` 覆盖字段；文件不存在则跳过。由 `Default` 调用 |
| `save_to_file()` | `pub` | 保留其它块，覆写 `insert.column`；返回是否写盘成功 |
| `save_search_column()` | `pub` | 保留其它块，覆写 `search.column` 与 `search.row`；返回是否写盘成功 |

> 读写均以 `serde_yaml::Value`（Mapping）操作：先读已有文件为 `Value`，再 `get_mut` 目标节点写入，
> 找不到则创建，最后 `serde_yaml::to_string` 落盘。这保证多个模块共享同一份 yaml 时互不覆盖。

---

## 4. UI 渲染

两个渲染函数签名统一为 `(ctx: &egui::Context, sp: &mut SettingsPanelState)`，仅依赖上下文与状态本身，
不触碰 `ExcelViewer` 的其它字段，故可干净地从 `viewer.rs::ui()` 抽出。

### 4.1 `draw_settings_panel(ctx, sp)` —— 插入配置弹窗

- **可见性驱动**：`sp.visible` 为真才渲染；关闭（`X` 按钮 / `keep_open=false`）置 `sp.visible = false`。
- **窗口属性**：无标题栏（`title_bar(false)`）、不可缩放/折叠、最小宽 420、默认居中偏移
  `content_rect().center() - (190, 80)`。
- **自定义标题栏**：左侧"插入配置"（12pt strong）；右侧 `right_to_left` 排列 `X` / `保存`；
  `save_success_timer > 0` 时显示绿色"保存成功"，每帧按 `ui.input(stable_dt)` 递减。
- **页签**：`selectable_label` 切换 `SettingsPage::{ColumnConfig, RowConfig}`。
- **列配置内容**（`ui.group`）：
  - 列范围（`DragValue` 起/止）+ 横向合并数量，同行；
  - 行范围（`DragValue` 起/止）+ 纵向合并数量，同行；
  - 复制选项（公式/样式/值 `checkbox`），同行。
- **行配置内容**：占位"功能开发中..."。
- **保存**：点击"保存"调用 `sp.save_to_file()`，成功则 `save_success_timer = 2.0`。

### 4.2 `draw_search_config_dialog(ctx, sp)` —— 搜索配置弹窗

- **可见性驱动**：`sp.show_search_dialog` 为真才渲染；关闭置 `sp.show_search_dialog = false`。
- **窗口属性**：同上（420 宽、无标题栏、居中偏移）。
- **自定义标题栏**：左侧"搜索配置"；右侧 `X` / `保存` + `search_save_success_timer` 驱动的"保存成功"。
- **页签**：切换 `SearchPage::{ColumnFilter, RowFilter}`。
- **列筛选**：`TextEdit`（`desired_width(INFINITY)`）编辑 `search_column_input`，提示 `A1-A13` / `A1,A3` / `A(1:+2):A13`；
  下方灰色说明"支持范围格式(A1-A13)、离散格式(A1,A3)与步长语法(A(1:+2):A13)，`~`=末尾"。
- **行筛选**：`TextEdit` 编辑 `search_row_input`，提示 `A14,B14` / `D14-F14` / `(B:+2)14:~14`；
  灰色说明"支持单元格引用(A14)、范围(D14-F14)、离散(A14,B14)与步长语法((B:+2)14:~14)，`~`=末尾"。
- **保存**：调用 `sp.save_search_column()`，成功则 `search_save_success_timer = 2.0`。

> 借用模式：进入 `Window::show` 闭包前先 `let active_page = sp.active_page;`（`Copy` 读出），避免闭包内
> 既读又写 `active_page` 的借用冲突；闭包内其余访问均为对单一 `sp: &mut` 的字段借用，互不冲突。

---

## 5. 视觉布局图

### 5.1 插入配置弹窗（`draw_settings_panel`）

```
┌──────────────────────────────────────────────────────────────────┐
│ 插入配置                                     保存成功   [保存] [X] │ ← 自定义标题栏（420 宽）
├──────────────────────────────────────────────────────────────────┤
│  列配置    行配置                                                │ ← 页签 selectable_label
├──────────────────────────────────────────────────────────────────┤
│ ┌──────────────────────────────────────────────────────────────┐ │
│ │ 列范围: [ 1] 列 至 [10] 列 │ 横向每 [2] 个单元格进行合并       │ │ ← ui.group
│ │ 行范围: [ 1] 行 至 [20] 行 │ 纵向每 [2] 个单元格进行合并       │ │
│ │ 复制: 公式 [✓] │ 样式 [ ] │ 值 [ ]                            │ │
│ └──────────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────────┘
```

### 5.2 搜索配置弹窗（`draw_search_config_dialog`）

```
┌──────────────────────────────────────────────────────────────────┐
│ 搜索配置                                     保存成功   [保存] [X] │ ← 自定义标题栏（420 宽）
├──────────────────────────────────────────────────────────────────┤
│  列筛选    行筛选                                                │ ← 页签
├──────────────────────────────────────────────────────────────────┤
│ ┌──────────────────────────────────────────────────────────────┐ │
│ │ 单元格范围: [ A1-A13 或 A1,A3                          ]      │ │ ← 列筛选页（TextEdit 撑满）
│ │ 支持范围格式（A1-A13）和离散格式（A1,A3）                     │ │ ← 灰色说明
│ └──────────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────────┘
   行筛选页：行搜索标题范围 TextEdit（如 A14,B14 / D14-F14）+ 说明
```

---

## 6. 公开 API 与使用方式

### 6.1 公开 API 速查

| 项目 | 签名 | 说明 |
|------|------|------|
| `SettingsPanelState` | `pub struct`（`Default`） | 配置状态容器；`Default` 自动从 yaml 加载 |
| `SettingsPage` | `pub enum { ColumnConfig, RowConfig }` | 插入配置页签 |
| `SearchPage` | `pub enum { ColumnFilter, RowFilter }` | 搜索配置页签 |
| `save_to_file` | `pub fn(&self) -> bool` | 落盘 `insert.column`（保留其它块） |
| `save_search_column` | `pub fn(&self) -> bool` | 落盘 `search.{column,row}`（保留其它块） |
| `draw_settings_panel` | `pub fn(&egui::Context, &mut SettingsPanelState)` | 渲染插入配置弹窗 |
| `draw_search_config_dialog` | `pub fn(&egui::Context, &mut SettingsPanelState)` | 渲染搜索配置弹窗 |

> 模块经 `gui/widgets/mod.rs` 的 `pub mod config; pub use config::*;` 再导出，因此上述项目也可通过
> `crate::gui::widgets::{SettingsPanelState, draw_settings_panel, ...}` 直接引用。

### 6.2 在 `viewer.rs` 中的使用

```rust
// 1) 字段（类型来自 config 模块）
use crate::gui::widgets::{SettingsPanelState, draw_settings_panel, draw_search_config_dialog};

pub struct ExcelViewer {
    pub settings_panel: SettingsPanelState,   // new() 中以 SettingsPanelState::default() 初始化
    // ...
}

// 2) 每帧渲染（ui() 内，菜单栏之后）
draw_settings_panel(&ctx, &mut self.settings_panel);
draw_search_config_dialog(&ctx, &mut self.settings_panel);
```

### 6.3 在 `menu_bar.rs` 中的使用

菜单项通过 `&mut SettingsPanelState` 触发弹窗与页签，不直接调用渲染函数：

```rust
// 配置 → 插入配置 → 列配置
settings_panel.visible = true;
settings_panel.active_page = Some(SettingsPage::ColumnConfig);
// 配置 → 搜索配置
settings_panel.show_search_dialog = true;
```

---

## 附：与原 `viewer.rs` 行为对照

本次抽离为**纯重构**，行为完全不变：

| 关注点 | 迁移前（viewer.rs 内联） | 迁移后（config.rs） |
|--------|--------------------------|----------------------|
| 类型定义 | `SettingsPanelState` / `SettingsPage` / `SearchPage` 在 viewer.rs | 移至 config.rs（同字段、同 derive） |
| 窗口属性 | `title_bar(false)` / 420 宽 / 居中偏移 `(190,80)` | 完全一致 |
| 标题栏 / 页签 / 表单控件 | 内联闭包 | 封装进 `draw_*` 函数，闭包体逐行搬移 |
| `self.settings_panel` 访问 | 直接字段访问 | 函数内改为 `sp` 形参访问 |
| `Window::show(&ctx, ...)` | `ctx` 为 owned，传 `&ctx` | `ctx: &Context` 形参，传 `ctx`（等价） |
| YAML 读写 | `load_from_file` / `save_to_file` / `save_search_column` | 方法签名与实现不变 |
| `Default` 加载时机 | 构造后立即 `load_from_file` | 完全一致 |

> 验证：`cargo check` 通过，无错误、无告警。`viewer.rs` 由 2148 行降至 1731 行（净减约 417 行），
> `ui()` 中配置弹窗的 ~197 行内联代码收敛为两行函数调用；新模块 `config.rs` 为 466 行。

---

*文档基于 `src/gui/widgets/config.rs`（本次新增）及其调用方 `viewer.rs`、`menu_bar.rs` 整理。
主界面整体布局与控制流见 [`viewer.md`](../viewer.md)。*
