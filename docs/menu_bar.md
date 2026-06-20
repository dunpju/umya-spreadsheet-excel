# 菜单栏组件分析（`src/gui/widgets/menu_bar.rs`）

> 本文档基于 `menu_bar.rs`（约 153 行）源码梳理，阐述顶部菜单栏的职责、函数结构、菜单层级、
> 状态管理、已注释功能与改进建议。配套阅读：主模块 [`viewer.md`](./viewer.md)、
> 配置模块 [`config.md`](./config.md)。

---

## 1. 代码设计概述

`menu_bar.rs` 隶属于 `gui::widgets`，是应用**顶部菜单栏**的唯一构建点。它是一个**无状态的纯视图 +
事件分发器**：

- **职责单一**：用 `egui::MenuBar` 绘制「文件 / 编辑 / 搜索 / 配置 / 转换 / 关于」六个顶层菜单及右侧
  预警图标，并把用户的菜单点击**翻译成对共享状态的写入**（置 `visible` / 置命令标志）。
- **不含业务逻辑**：不在本文件里打开文件、执行搜索或渲染弹窗——这些副作用都由 `viewer.rs::ui()`
  在后续帧读取状态后完成。本模块只负责"发信号"。
- **GUI 系统中的定位**：它是用户命令的**入口桥梁**。`ExcelViewer` 每帧调用 `draw_menu_bar`，把各弹窗/
  面板状态以 `&mut` 透传进来；菜单点击修改这些状态，下一帧 `viewer.rs` 据此渲染对应浮层或执行操作。

> 设计取向：菜单栏自身不持有任何状态（无 `struct`/字段），所有状态外部化到 `ExcelViewer`，通过参数
> 传入。这让本模块极易测试与维护——输入只有 `ui` + 一组可变引用 + 两个只读上下文（`has_data`、
> `lic_status`）。

---

## 2. 结构与函数拆解

本文件**不定义任何 `pub struct` / 枚举 / 辅助函数**，仅有一个公开函数，并复用一个外部组件。

### 2.1 `pub fn draw_menu_bar(...)`

```rust
pub fn draw_menu_bar(
    ui: &mut egui::Ui,
    show_import_dialog: &mut bool,
    settings_panel: &mut SettingsPanelState,
    search_window: &mut SearchWindowState,
    add_column: &mut bool,
    add_row: &mut bool,
    has_data: bool,
    convert_popup: &mut ConvertPopupState,
    alert_popup: &mut AlertPopupState,
    _cond_format_popup: &mut CondFormatPopupState,
    help_popup: &mut HelpPopupState,
    alert_notify_state: &mut AlertNotifyState,
    license_popup: &mut LicensePopupState,
    lic_status: &LicenseStatus,
)
```

- **返回值**：单元 `()`。所有副作用通过两种途径产生：① 绘制到 `ui`；② 经 `&mut` 参数写回状态。
- **参数语义**（共 14 个）：

| 参数 | 类型 | 语义 | 读/写 |
|------|------|------|-------|
| `ui` | `&mut egui::Ui` | 宿主 UI 上下文（菜单栏绘制目标） | 写（绘制） |
| `show_import_dialog` | `&mut bool` | 导入对话框可见标志；置 `true` 触发文件选择 | 写 |
| `settings_panel` | `&mut SettingsPanelState` | 配置面板状态（搜索配置弹窗） | 写 `show_search_dialog` |
| `search_window` | `&mut SearchWindowState` | 搜索窗口状态 | 写 `visible`/`collapsed`/`options_loaded` |
| `add_column` / `add_row` | `&mut bool` | 命令标志；置 `true` 由 `viewer.rs` 消费执行插入 | 写 |
| `has_data` | `bool` | 是否已加载 Excel（控制菜单项启用） | 只读 |
| `convert_popup` | `&mut ConvertPopupState` | 转换工具弹窗 | 写 `visible` |
| `alert_popup` | `&mut AlertPopupState` | 预警消息弹窗 | 写 `visible` |
| `_cond_format_popup` | `&mut CondFormatPopupState` | 条件格式弹窗（**当前未用**，前缀 `_`） | 未使用 |
| `help_popup` | `&mut HelpPopupState` | 帮助弹窗 | 写 `visible` |
| `alert_notify_state` | `&mut AlertNotifyState` | 预警通知（图标 + 弹窗） | 传给 `draw_alert_icon` |
| `license_popup` | `&mut LicensePopupState` | 激活/付款弹窗 | 写 `visible` |
| `lic_status` | `&LicenseStatus` | 当前授权状态（驱动"关于"文案与"激活"入口） | 只读 |

### 2.2 复用的外部组件

- `draw_alert_icon(ui, alert_notify_state)`（来自 `alert_notify` 模块，经 `crate::gui::widgets::draw_alert_icon`
  引入）：绘制菜单栏最右侧的预警铃铛图标。本文件不实现它，只调用。

> 无辅助函数：所有菜单构建以闭包内联在 `draw_menu_bar` 中（见 §3）。

---

## 3. 菜单层级与 UI 布局

整体由 `egui::MenuBar::new().ui(ui, |ui| { ... })` 构建一条**水平菜单栏**：顶层菜单按代码顺序自左向右
排列，末尾用 `with_layout(right_to_left)` 把预警图标推到最右端。

### 3.1 菜单树与触发行为

| 顶层菜单 | 子项 | 触发行为（点击后） | 启用条件 |
|----------|------|---------------------|----------|
| **文件** | 导入 | `*show_import_dialog = true` | 始终 |
| **编辑** | 添加列 | `*add_column = true`（viewer 消费 → 插入列确认流程） | `has_data` |
|          | 添加行 | `*add_row = true`（viewer 消费 → `append_row` + 公式扩展） | `has_data` |
| **搜索** | 搜索 | `search_window.visible=true`；`collapsed=false`；`options_loaded=false` | `has_data` |
| **配置** | ~~插入配置~~（列配置/行配置） | **已注释**（见 §5） | — |
|          | 搜索配置 | `settings_panel.show_search_dialog = true` | 始终 |
|          | 预警消息 | `alert_popup.visible = true` | 始终 |
|          | ~~条件格式~~ | **已注释**（见 §5） | — |
| **转换** | 转换工具 | `convert_popup.visible = true` | 始终 |
| **关于** | 版本/邮箱 label | 展示 `My Excel v{CARGO_PKG_VERSION} @ 2026 ...`（版本号在编译时从 `Cargo.toml` 的 `package.version` 注入） | 始终 |
|          | 授权状态 label | 按 `lic_status` 显示试用剩余/已授权/到期/异常 | 始终 |
|          | 激活 | `license_popup.visible = true` | 试用期内（剩余>0） |
|          | 帮助 | `help_popup.visible = true` | 始终 |
| （右侧） | 🔔 预警图标 | `draw_alert_icon(...)` | 始终 |

### 3.2 布局示意

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ 文件 ▾   编辑 ▾   搜索 ▾   配置 ▾   转换 ▾   关于 ▾              🔔           │
│  导入     添加列    搜索     搜索配置   转换工具   My Excel v0.1.0...         │
│           添加行             预警消息             试用剩余 N 天               │
│                              (插入配置-)          激活                        │
│                              (条件格式-)          帮助                        │
└──────────────────────────────────────────────────────────────────────────────┘
   ▲ 顶层菜单自左向右；预警图标用 right_to_left 布局钉在右端
   △ 已注释项不会渲染
```

### 3.3 关键交互细节

- **`ui.close()`**：每个可点击菜单项在 `clicked()` 后调用，**立即收起下拉菜单**（egui 惯例，避免菜单
  在动作执行后仍悬停）。
- **`add_enabled(has_data, ...)`**：编辑/搜索菜单在未加载文件时**灰显禁用**，防止空操作。
- **搜索菜单的强制重置**：每次打开都 `collapsed=false` + `options_loaded=false`——不沿用用户上次的折叠
  状态，并强制下一帧重新从 yaml 加载下拉选项。
- **"激活"入口的短路渲染**：`if in_trial && ui.button("激活").clicked()` —— `in_trial` 为假时由于短路
  求值**根本不调用 `ui.button`**，即该菜单项不渲染（而非渲染后禁用）。

---

## 4. 状态管理

本模块自身**无状态**；所有状态都是 `ExcelViewer` 的字段，经 `&mut` 参数传入。菜单交互对这些状态的读写
遵循两种统一模式。

### 4.1 "打开浮层"模式（visibility-driven popup）

绝大多数菜单项的副作用是**置某个 `*.visible = true`**：

```
菜单点击 ──► X_popup.visible = true ──► viewer.rs::ui() 下一帧读取并渲染 X 浮层
```

涉及字段：`search_window.visible`、`settings_panel.show_search_dialog`、`alert_popup.visible`、
`convert_popup.visible`、`license_popup.visible`、`help_popup.visible`、`*show_import_dialog`。

> 这是一种**声明式可见性**：菜单不直接创建窗口，只翻转一个布尔，由 `viewer.rs` 集中决定何时/如何渲染。
> 多个浮窗可同时可见（无互斥），状态彼此独立。

### 4.2 "命令标志"模式（command flag）

编辑菜单的 `添加列/行` 用**一次性命令标志**：

```
菜单点击 ──► *add_column = true ──► viewer.rs::ui() 读取后执行插入，并复位 *add_column = false
```

`viewer.rs` 在 `ui()` 中检测 `self.add_column`，触发插入列确认流程（复用右键菜单的确认弹窗），完成后
清零标志。这是"事件 → 标志 → 帧消费"的解耦，避免菜单闭包直接操作数据。

### 4.3 只读上下文

- `has_data: bool`：决定编辑/搜索菜单是否启用；
- `lic_status: &LicenseStatus`：驱动"关于"菜单的授权文案（`match` 全枚举）与"激活"入口是否渲染。

### 4.4 涉及状态字段汇总

| 字段 | 所属 | 写入方（本模块） | 消费方 |
|------|------|------------------|--------|
| `*show_import_dialog` | ExcelViewer | 文件→导入 | viewer（`draw_import_dialog`） |
| `*add_column` / `*add_row` | ExcelViewer | 编辑菜单 | viewer（插入流程，消费后清零） |
| `search_window.{visible,collapsed,options_loaded}` | SearchWindowState | 搜索菜单 | `draw_search_window` |
| `settings_panel.show_search_dialog` | SettingsPanelState | 配置→搜索配置 | `draw_search_config_dialog` |
| `alert_popup.visible` | AlertPopupState | 配置→预警消息 | `draw_alert_popup` |
| `convert_popup.visible` | ConvertPopupState | 转换→转换工具 | `draw_convert_popup` |
| `license_popup.visible` | LicensePopupState | 关于→激活 | `draw_license_popup` |
| `help_popup.visible` | HelpPopupState | 关于→帮助 | `draw_help_popup` |
| `alert_notify_state` | AlertNotifyState | （透传给图标） | `draw_alert_icon` |

---

## 5. 已注释 / 未完成功能

文件中有两处被注释的菜单项，均为**有意停用的 UI 入口**：

### 5.1 「插入配置」子菜单（列配置 / 行配置）

```rust
// 暂时未使用，先注释
// ui.menu_button("插入配置", |ui| {
//     if ui.button("列配置").clicked() { ... settings_panel.active_page = Some(SettingsPage::ColumnConfig); }
//     if ui.button("行配置").clicked()  { ... settings_panel.active_page = Some(SettingsPage::RowConfig); }
// });
```

- **设计意图**：打开「插入配置」面板并定位到列配置/行配置页签（编辑合并参数与复制选项）。
- **当前状态**：停用。根因是底层的合并配置功能**未完成**——`SettingsPanelState` 的合并参数
  （`merge_col_*`/`merge_row_*`）虽有 UI 与 yaml 持久化，但**无任何消费者**（详见
  [`config.md`](./config.md) 及前序分析）。配套地，导入已收窄为只引入 `SettingsPanelState`
  （`SettingsPage` 被移除，恢复时需加回，已在源码注释中标注）。

### 5.2 「条件格式」菜单项

```rust
// 使用原Excel表格条件格式功能，所以隐藏菜单功能
// if ui.button("条件格式").clicked() {
//     ui.close();
//     _cond_format_popup.visible = true;
// }
```

- **设计意图**：打开自定义条件格式弹窗（`cond_format_popup`）。
- **当前状态**：停用。原因是应用改为**直接使用 Excel 文件自带的条件格式**（`viewer.rs` 每帧
  `reapply_conditional_formatting`），自定义编辑入口不再需要。参数 `_cond_format_popup` 仍保留在签名中
  （前缀 `_` 表示未使用），以维持调用点签名稳定。

> 此外「行配置」页签在 `config.rs` 中也仅显示"功能开发中..."占位，与「插入配置」整体未完成一致。

---

## 6. 设计特点与改进建议

### 6.1 优点

- **无状态单函数**：全部状态外部化，函数行为完全由入参决定，易读、易测。
- **统一的"置 visible / 置标志"模式**：打开浮层与触发命令两种范式贯穿全文件，认知负担低。
- **正确的启用控制**：`add_enabled(has_data, ...)` 在无数据时禁用编辑/搜索，防止无效操作。
- **优雅的授权门面**：`in_trial && ui.button("激活")` 用短路求值实现"条件渲染"，且拦截态由 `viewer`
  自动弹模态，菜单无需重复处理。
- **参数文档齐全**：`draw_menu_bar` 的 doc-comment 逐项说明参数用途。

### 6.2 改进建议

| # | 问题 | 建议 |
|---|------|------|
| 1 | **参数过多（14 个）**，调用点（`viewer.rs:480` 附近）一行极长、易错 | 聚合为一个 `MenuBarCtx<'a>` 结构体（持有所有 `&mut` + 只读字段），`draw_menu_bar(ui, &mut ctx)`；签名与调用点都大幅简化 |
| 2 | **每项都是 `if ui.button(X).clicked() { ui.close(); Y.visible=true; }` 样板** | 抽一个 helper，如 `fn open_popup(ui, label, target: &mut bool)`（注意 egui 闭包借用，可传 `&mut` 或返回 `bool` 由调用方赋值），减少重复 |
| 3 | **每个浮窗各自一个 `visible` 布尔**，分散在各 state 结构，隐含"多窗同显" | 评估引入 `ActivePanel` 枚举做单点互斥管理（会改变现有"可同时多开"行为，需确认产品诉求后再改） |
| 4 | ~~硬编码版本号~~ / 邮箱 / 菜单文案 | ✅ 版本号已改用 `env!("CARGO_PKG_VERSION")` 编译时注入（不再硬编码）；邮箱与菜单文案仍硬编码，可后续集中为常量便于 i18n |
| 5 | **两处长期注释代码**（插入配置、条件格式） | 若短期内不恢复，建议**彻底删除**（git 可追溯历史）或改用 `#[cfg(feature = "...")]` feature flag，保持源码整洁；当前"注释 + 导入说明"是可接受的临时态 |
| 6 | **"添加列/行"与右键插入是两条路径** | 两者最终都走 `insert_columns`/`append_row`，但菜单经 `add_column` 标志、右键经 `ContextMenuState`，存在行为分叉风险；可考虑统一为同一入口/同一确认流程 |
| 7 | **"关于"菜单混合静态 label 与可点击 button** | 语义上"关于"宜纯展示；"激活/帮助"可独立入口或保留——属偏好取舍，非缺陷 |
| 8 | **`_cond_format_popup` 保留空占位参数** | 若确认条件格式入口不再恢复，可从签名移除该参数并同步更新调用点，减少无效透传 |

---

*文档基于 `src/gui/widgets/menu_bar.rs`（截至当前，含「插入配置」「条件格式」已注释的状态）整理。
各菜单项实际触发的浮层渲染与命令消费逻辑见 [`viewer.md`](./viewer.md) §2.7 控制流。*
