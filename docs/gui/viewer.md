# ExcelViewer 主模块分析（`src/gui/viewer.rs`）

> 本文档基于 `viewer.rs`（约 2148 行）源码梳理，系统阐述 GUI 主模块的定位与依赖、
> 核心类型与控制流、视觉布局、事件交互，以及关键数据流转路径。配套阅读：
> 搜索组件见 [`search.md`](widgets/search.md)。

---

## 1. 模块定位与职责

`viewer.rs` 是 GUI 子系统的**主模块与总控制器**。它实现 eframe 的 `App` trait（入口方法 `fn ui`），
持有应用的**全部可变状态**（`ExcelViewer`），并每帧把状态分派给各 `widgets` 组件绘制。

### 1.1 职责清单

- **应用容器**：`ExcelViewer` 是 eframe 唯一的 `App` 实现者，集中管理 Excel 数据、选中/编辑/拖拽状态、
  各类弹窗状态、撤销栈、隐藏行列集合、授权管理器等。
- **帧调度器**：`eframe::App::ui()` 每帧按固定管线执行 —— 字体设置 → 授权拦截 → 菜单栏 → 浮层 →
  异步结果回收 → 主内容区 → 授权弹窗。
- **状态中转**：把顶层状态以 `&mut` 借用透传给各 `draw_*` 组件，回收组件返回的动作（跳转坐标、
  保存请求、选中变化等）并回写。
- **异步编排**：通过 `std::sync::mpsc` 通道把 Excel 加载 / 保存放到后台线程，主线程每帧 `try_recv` 回收。
- **快捷键与撤销**：`Ctrl+S/Z/C/V`、`Delete`、`Escape` 等全局键绑定，以及多粒度撤销栈。
- **授权门面**：每帧计算 `LicenseStatus`，拦截态下渲染全屏模态遮罩；非拦截态推进试用高水位。

### 1.2 依赖关系

```
                       main.rs
                          │  ExcelViewer::new()
                          ▼
 ┌──────────────────── viewer.rs (ExcelViewer: eframe::App) ────────────────────┐
 │                                                                              │
 │  上游依赖（被它调用）                                                        │
 │   ├─ excel::reader   ExcelData / SheetData / CellData / col_to_letter        │
 │   ├─ excel::formula  evaluate_sheet / evaluate_dependents                    │
 │   ├─ excel::writer   save_to_file                                            │
 │   ├─ gui::state      LoadState                                               │
 │   ├─ gui::fonts      setup_fonts                                             │
 │   ├─ gui::widgets    draw_menu_bar / draw_import_dialog / draw_table_content │
 │   │                  draw_name_box / draw_empty_state / draw_search_window    │
 │   │                  draw_alert_popup / draw_alert_notify_popup / draw_help   │
 │   │                  draw_cond_format_popup / draw_convert_popup             │
 │   │                  draw_license_popup / check_alert_rules / update_alert_* │
 │   ├─ license         LicenseManager / LicenseStatus / time::today_epoch_day  │
 │   └─ util            backup::backup_imported_file（导入备份）/ open::open_in_default_app（点击路径打开）│
 │                                                                              │
 │  下游被依赖（反向引用其类型）                                                │
 │   ├─ menu_bar.rs    引用 SettingsPanelState / SettingsPage                   │
 │   ├─ table.rs       引用 ContextMenuState（右键菜单 + 确认弹窗状态）          │
 │   └─ license_popup  等：纯被 viewer 调用，不反向依赖                          │
 └──────────────────────────────────────────────────────────────────────────────┘
```

> `viewer.rs` 仍被其它 GUI 文件**反向依赖**（共享状态类型）：`ContextMenuState` 在此定义并被
> `table.rs` 引用。**配置相关类型（`SettingsPanelState` / `SettingsPage` / `SearchPage`）及其 UI 已
> 迁移至独立模块 [`gui/widgets/config.rs`](widgets/config.md)**，`menu_bar.rs` 经 `config` 模块引用它们。

> **预警触发检测 + 通知弹窗**（`check_alert_rules` / `draw_alert_notify_popup` / `draw_alert_icon` /
> `filter_by_triggered_rule` / `update_alert_range_expansions_*`）位于 [`gui/widgets/alert_notify.rs`](widgets/alert_notify.md)，
> `viewer.rs` 每帧调用并持有 `alert_notify_state`，`menu_bar.rs` 绘制图标。规则**配置**侧见 [`alert_popup.md`](widgets/alert_popup.md)，
> 通知弹窗的**居中定位实现**见 [`alert_notify.md`](widgets/alert_notify.md) §6。

---

## 2. 代码架构与设计逻辑

### 2.1 核心类型总览

| 类型 | 类别 | 职责 |
|------|------|------|
| `ExcelViewer` | struct（`App` 实现者） | 应用全部状态的"上帝对象"，实现 `eframe::App` |
| `ContextMenuState` | struct | 右键菜单 + 插入/清空确认弹窗的状态（位置、计数、复制选项、确认动作） |
| `FillCommit` | struct（pub） | 填充柄拖拽提交信号：`old_cells` + `old_selected` + `old_range`，由 `draw_table_content` 写入出参 |
| `PendingFill` | struct（pub） | 分批跨帧填充状态：当目标格数超过 FILL_SYNC_THRESHOLD（2000）时激活。字段：`values`（预计算待写入值）、`next_idx`（下一个待写入索引）、`has_formula`（决定重算策略）、`old_cells`（累积旧数据用于撤销）、`prev_selected`/`prev_range`（填充前选区）、`src`/`target`（源选区和目标格） |
| `PasteCommit` | struct（pub） | 粘贴提交信号：`old_cells` + `old_selected` + `old_range`，由 `draw_table_content` 写入出参 |
| `SettingsPanelState` | struct | **已迁移至 [`config.rs`](widgets/config.md)**（插入配置 + 搜索配置 + YAML 持久化） |
| `UndoAction` | enum（私有） | 撤销操作的三粒度：`FullSnapshot` / `CellChange` / `RangeClear` |
| `ContextAction` | enum | 右键菜单动作：插行/插列（上下左右）/ 清空 / 向四方选中 |
| `SettingsPage` / `SearchPage` | enum | **已迁移至 [`config.rs`](widgets/config.md)**：设置面板 / 搜索配置弹窗页签 |
| `LoadState` | enum（`gui::state`） | `Idle` / `Loading` / `Success(ExcelData)` / `Failed(String)` |

### 2.2 状态容器 `ExcelViewer`

`ExcelViewer` 是一个**集中式状态对象**（约 40 个字段），按职责分组：

| 分组 | 字段 | 作用 |
|------|------|------|
| 数据 | `excel_data: Option<ExcelData>`、`current_sheet`、`file_path` | 当前工作簿、当前 sheet、源文件路径 |
| 选中/编辑 | `selected_cell`、`selected_range`、`editing_cell`、`edit_value`、`just_entered_edit_mode`、`hovered_cell`、`drag_anchor`、`fill_drag_source`、`pending_fill: Option<PendingFill>`、`shift_click_anchor` | 单格选中、范围选中、正在编辑的格、框选拖拽锚点、填充柄拖拽源锚点、分批跨帧填充状态（目标格数超 FILL_SYNC_THRESHOLD 时激活）、Shift+点击范围选择锚点 |
| 异步 I/O | `load_state`、`rx`、`save_rx`、`saving`、`save_requested`、`save_path` | 加载/保存的后台线程通道与状态 |
| 校验 | `validation_error`、`validation_error_pos`、`original_cell_data`、`pending_formula_save` | 数据有效性校验错误弹窗 + 恢复 + 公式栏待保存 |
| 右键/设置 | `context_menu`、`settings_panel`、`name_box_state` | 各交互面板状态 |
| 撤销 | `undo_stack: Vec<UndoAction>`（私有，深度上限 `MAX_UNDO_DEPTH=20`） | 撤销历史 |
| 行为标志 | `dirty`、`add_column`、`add_row`、`add_column_pending`、`scroll_to_last_col/row` | 脏标记、菜单触发、插入后滚动 |
| 筛选/格式 | `hidden_columns`、`hidden_rows`、`cond_format_popup` | 搜索隐藏集合（搜索写入、表格读取）+ 用户条件格式 |
| 弹窗 | `search_window`、`convert_popup`、`alert_popup`、`help_popup`、`alert_notify_state`、`show_import_dialog` | 各浮窗可见性与内容 |
| 授权 | `license: LicenseManager`、`license_popup` | 离线授权 + 激活/付款弹窗 |

> 设计取舍：所有状态集中在 `ExcelViewer`，使每帧只需一个 `&mut self` 即可把任意子集借用给组件，
> 避免跨组件的回调/事件总线。代价是 `ui()` 体量大（约 1350 行），靠严格的代码分区与"延迟到循环外
> 处理"模式控制借用冲突。

### 2.3 右键菜单 / 确认弹窗状态 `ContextMenuState`

承载两类 UI：① 右键浮层菜单（插行/插列、清空、四方选中）；② 操作确认弹窗（插入列复制选项、
清空确认）。关键字段：

- `visible` / `confirm_visible`：菜单与确认弹窗各自可见性；
- `target_cell`、`position`：操作目标格与弹屏坐标；
- `insert_rows_count` / `insert_cols_count`：插入数量（`DragValue`，可调）；
- `select_down/up/left/right_count`：四方选中数量（`0` = 选到边界）；
- `confirm_action: Option<ContextAction>`、`confirm_established`：待确认动作 + "首帧已建立"标志
  （用于**外部点击关闭**时跳过触发帧，避免一打开就被关掉）；
- `copy_merge/copy_formula/copy_style/copy_value`：插入列时的复制选项。

> **四方选中的合并单元格锚点解析**：右键在合并单元格区域内执行「向下/上/左/右选中」时，
> 会先通过 `get_merged_range(col, row)` 解析合并区域边界，以 `start_col`/`start_row` 作为选中
> 起始锚点、`end_col`/`end_row` 作为扩展基准——例如 D1:E1 合并后在 E1 右键「向右选中 0 列」，
> 选中范围从 D 列（非 E 列）开始向右延伸至边界，且行方向覆盖合并区域全高（第 1 行）。
> 非合并格退化为原逻辑：锚点=点击坐标。此修复解决了右键位置非合并区域左上角时选中范围遗漏
> 合并格起始列/行的问题。

> **两种确认弹窗的视觉区分**：`confirm_action == ClearCell` 的「清空确认弹窗」（`egui::Window::new("clear_confirm")` 分支）
> 采用**红色警示样式**——浅红背景（`#FEECEC`）+ 2px 红色边框（`#D32F2F`）+ 红色加粗标题「⚠ 警告」+ 深红正文（`#961818`），
> 且「确认」按钮为红底（`#D32F2F`）白字的破坏性操作按钮、「取消」为白底柔红描边（`#C85050`）。清空属于不可逆的破坏性操作，
> 故从视觉上强化警示，与插入列的中性确认弹窗区分开，并与下方 §3「保存失败提示框」的红底红边风格一致。
> 插入列确认弹窗（`insert_confirm`）仍保持普通（默认框架）样式。
> **尺寸约束**：清空确认弹窗设 `min_height(80.0).max_height(80.0)` 把窗口高度**固定为 80px**（宽度 `ui.set_width(240.0)`
> 为 240px），避免内容自适应时窗口被竖直拉伸；插入列弹窗用 `set_min_width(360.0)` + `set_height(50.0)`，约束方式不同。

### 2.4 配置面板 —— 已迁移至 `config.rs`

> **配置相关代码已抽离到独立模块 [`gui/widgets/config.rs`](widgets/config.md)：**
> `SettingsPanelState` / `SettingsPage` / `SearchPage`（结构与枚举）、插入配置 / 搜索配置弹窗的**全部
> UI 渲染**（选项卡切换、列/行配置、保存功能、保存成功提示）、以及 YAML 持久化（`load_from_file` /
> `save_to_file` / `save_search_column`，读写 `~/.MyExcel/my-excel.yaml`）。
>
> `viewer.rs` 现仅：① 持有 `ExcelViewer.settings_panel: SettingsPanelState` 字段（类型来自 `config` 模块，
> 通过 `use crate::gui::widgets::...SettingsPanelState` 引入）；② 在 `ui()` 中以一行调用
> `draw_settings_panel(&ctx, &mut self.settings_panel)` / `draw_search_config_dialog(...)` 渲染。
> 类型定义、字段、持久化方法、UI 布局与交互详见 [`config.md`](widgets/config.md)。

> 与 `search.rs` 的衔接：`search.rs` 读取 `search.column`/`search.row` 解析为筛选项；`config` 模块的搜索
> 配置弹窗负责**编辑并落盘**这两个键。二者通过同一份 yaml 文件解耦。

### 2.5 撤销模型 `UndoAction`

三粒度，按操作破坏性选择，统一由 `Ctrl+Z`（无 Shift）回放：

```rust
enum UndoAction {
    FullSnapshot { sheet_data: SheetData, sheet_index: usize },   // 插行/插列等结构性操作
    CellChange  { sheet_index, row, col, old_cell, old_selected },// 单格编辑/清空
    RangeClear  { sheet_index, old_cells: Vec<(r,c,old)>, ... },  // 范围清空
}
```

- `push_undo_full/cell/range`：入栈前若 `len >= 20` 则 `remove(0)` 丢弃最旧（FIFO 上限）；
- 三者都是**关联函数**（`Self::push_undo_*(...)`），不借用 `self`，避免与 `excel_data` 的借用冲突；
- 回放（`ui()` 内 `Ctrl+Z`）：`take_undo()` 取出 → 按变体写回 `cells` → 重算公式（结构/范围走
  `evaluate_sheet`，单格走 `evaluate_dependents`）→ 置 `dirty=true`。

**入栈位点（哪些操作可撤销）**：

| 操作 | 粒度 | 入栈位置 |
|------|------|----------|
| 插入行/列（结构性） | `FullSnapshot` | `viewer.rs` 右键菜单确认流程 |
| 右键清空单个单元格 | `CellChange` | `viewer.rs` `ClearCell` 单格分支 |
| 右键清空选中范围 | `RangeClear` | `viewer.rs` `ClearCell` 范围分支 |
| 公式栏编辑提交（`pending_formula_save`） | `CellChange` | `viewer.rs` `pending_formula_save` 处理 |
| **单元格内编辑提交（TextEdit 双击/Enter 编辑 → Enter/Tab/失焦保存）** | `CellChange` | `viewer.rs` `draw_table_content` 返回后，按 `committed_edit` 信号入栈 |
| **填充柄填充（拖拽选区右下角柄 → 释放）** | `RangeClear` | `viewer.rs` 按 `committed_fill`（`FillCommit`）信号入栈；回放恢复 `old_cells` + 选区 |
| **Ctrl+V 粘贴（剪贴板 → 目标区域）** | `RangeClear` | `viewer.rs` 按 `committed_paste`（`PasteCommit`）信号入栈；回放恢复被覆盖格的 `old_cells` + 选区 |

> **单元格内编辑的撤销**：`draw_table_content`（`gui/widgets/table.rs`）本身不接触私有 `undo_stack`，
> 而是在两条保存路径成功写入后置出参 `committed_edit = Some((row, col))`；调用方在 `ScrollArea`
> 返回后据此重建编辑前快照并入栈。旧值取自编辑入口捕获的 `original_cell_data`（编辑只改
> `value`/`formula`，故「当前 cell 克隆 + 回填入口 value/formula」即等价于编辑前快照，规避实时
> 重算对 `cell.value` 的逐帧污染）。仅当 `value`/`formula` 确有变化才入栈。`Ctrl+Z` 回放走
> `CellChange` 分支，由 `evaluate_dependents` 重算。

> **公式位置集精确维护**：`CellChange` 和 `RangeClear` 撤销回放路径现在也会精确维护
> `formula_positions`（基于旧值与新值的公式状态差异，逐格调用 `mark_formula` / `unmark_formula`），
> 确保撤销后公式依赖图与实际数据一致。

> **范围清空的公式图缓存失效**：`ClearCell` 范围清空分支在调用 `evaluate_sheet` 全量重算前，
> 必须先调用 `invalidate_formula_graph` 失效 L2 缓存（`cached_graph_dirty = true`）。否则
> `unmark_formula` 仅移除了 `formula_positions` 索引但未置脏缓存，`build_formula_graph` 会命中
> 缓存的旧公式 AST——被清空格虽 `value`/`formula` 均已 `clear()`，但旧 AST 仍参与拓扑求值并
> 将计算结果写回 `cell.value`，导致「公式已清空但数值残留」的 bug。此修复同时覆盖 Delete 键
> 和右键菜单"清空选中范围"两条入口（二者共用同一确认执行链路）。

### 2.6 `impl ExcelViewer` 方法（加载/保存/撤销）

| 方法 | 作用 |
|------|------|
| `new()` | `LicenseManager::load()` + 计算拦截态 → 决定激活弹窗初始可见；初始化全部状态默认值 |
| `start_async_load(path, ctx)` | **导入入口**：`tx`/`rx` 通道、置 `LoadState::Loading` → 开后台线程，线程内**先** `util::backup::backup_imported_file`（备份到 `~/.MyExcel/backup/`，命名 `原文件名_yyyymmddhhmmss.ext`，目录不存在则递归创建，失败仅 `log::warn!` 记日志不阻断）**再** `ExcelData::load_from_file`，完成后 `ctx.request_repaint()`。备份与加载同线程顺序执行，避免阻塞 UI |
| `check_load_result()` | 每帧 `rx.try_recv()`：成功则替换数据、清选中/撤销/隐藏集合、应用用户条件格式、首屏预警检测 |
| `start_async_save(ctx)` | **授权拦截**（拦截态弹激活窗并 return）→ 克隆数据 → **输出路径 = 原文件路径**（`output_path = file_path.clone()`，直接覆盖原文件）→ 开线程 `writer::save_to_file(原路径, 数据, 原路径)`。不再生成带日期后缀的新文件 |
| `check_save_result()` | 每帧 `save_rx.try_recv()`：成功置 `save_path`（= 原文件路径）+ `dirty=false`（并清失败提示）；失败置 `error_message` + `save_failed`（触发居中红色提示框，文案 `保存失败!请检查{原文件路径}文件是否被占用打开`） |
| `push_undo_*` / `take_undo` | 撤销栈入栈/出栈（见 §2.5） |

> 异步模式统一为「**后台线程 + mpsc 通道 + 每帧 try_recv**」：线程完成后 `request_repaint` 唤醒主线程，
> 主线程在 `ui()` 中非阻塞回收。这避免阻塞 UI，也无需 `Arc<Mutex>`（结果只在线程结束后单次传递）。

### 2.7 `eframe::App::ui()` —— 每帧控制流

eframe 0.34 的 `App` trait **要求实现 `fn ui(&mut self, ui, frame)`**（`update` 有默认实现会构造
`egui::Ui` 并调用 `ui`）。`ui()` 是全模块的中枢，固定管线如下：

```
ui(&mut self, ui, frame)                                          // 每帧入口
 │
 ├─ setup_fonts(&ctx)
 ├─ lic_status = license.status(today)                            // 每帧算一次
 │
 ├─【拦截态 is_blocking】► 全屏黑色遮罩(CentralPanel) + "请激活后继续使用"  ── 跳过主界面
 │   └─（else 分支为下方完整主界面）
 │
 ├─ 顶部 Panel "menu_bar" ► draw_menu_bar(...)                     // 文件/编辑/搜索/配置/转换/关于 + 预警图标
 ├─ draw_import_dialog ► 命中 ► start_async_load
 ├─ draw_help_popup / draw_alert_popup
 ├─ check_alert_rules ► 更新 alert_notify_state（每帧）
 ├─ draw_alert_notify_popup / draw_cond_format_popup
 ├─ 条件格式事件驱动：仅当前表 cf_dirty 时 reapply_conditional_formatting + 用户规则（cf_dirty 由 evaluate 在值变化时置位）
 ├─ draw_convert_popup
 ├─ 处理菜单触发的 add_column（复用插入列确认流程）/ add_row（直接 append_row + 公式扩展）
 ├─ draw_settings_panel / draw_search_config_dialog（配置模块 config.rs；由 settings_panel 可见性驱动）
 ├─ draw_search_window（非模态浮窗）
 │
 ├─ check_load_result() / check_save_result()                     // 回收异步结果（保存失败置 save_failed）
 ├─ saving ► request_repaint（驱动 loading 动画）
 ├─ Ctrl+S（dirty && !saving）► start_async_save
 ├─ save_failed.is_some() ► 居中红色 Foreground 浮窗（保存失败提示，"知道了"关闭）
 │
 ├─ 底部 Panel "status_bar"（最底部：源路径 + 保存路径/spinner）
 ├─ 底部 Panel "sheet_bar"（其上：sheet 切换标签，切换时重置选中/隐藏/预警过滤）
 │
 ├─ CentralPanel（主内容区，占剩余空间）：
 │   ├─ pending_undo = (Ctrl+Z && 非编辑) ? take_undo() : None     // 借用 excel_data 前取出；编辑模式守卫
 │   ├─【有数据】
 │   │   ├─ 应用 pending_undo（按 UndoAction 变体回放 + 重算）
 │   │   ├─ Delete 键 ► 有内容则弹清空确认（范围/单格）
 │   │   ├─ 计算 display_text（选中格的公式/日期格式化值，供公式栏）
 │   │   ├─ draw_name_box(...) ► 回收 (跳转坐标, save_clicked)
 │   │   ├─ separator
 │   │   ├─ ScrollArea::both("table_scroll")
 │   │   │     ├─ draw_table_content(...) ► (scroll_target, cell_rect)
 │   │   │     ├─ selected_cell 变化且非拖拽 ► 清 selected_range
 │   │   │     ├─ scroll_to_last_col/row（插入后定位）
 │   │   │     ├─ 数据有效性输入提示弹窗（cell_rect 左下）
 │   │   │     ├─ 数据有效性校验错误弹窗（固定位置，重试/取消恢复）
 │   │   │     ├─ 右键上下文菜单（Area，Foreground）
 │   │   │     └─ 确认弹窗（插入列/清空）
 │   │   ├─ committed_edit（本帧有单元格编辑提交）► 按 original_cell_data 重建编辑前快照入 undo_stack（CellChange）
 │   │   ├─ committed_fill（本帧有填充柄提交）► 构造 RangeClear 入 undo_stack（恢复 old_cells + 选区）
 │   │   ├─ committed_paste（本帧有粘贴提交）► 构造 RangeClear 入 undo_stack（恢复 old_cells + 选区）
 │   │   ├─ pending_formula_save（公式栏 Enter 回写：校验 ► 写格 ► 重算）
 │   │   ├─ pending_fill 接收（draw_table_content 返回的分批填充请求 → self.pending_fill）
 │   │   └─ 分批跨帧填充循环（self.pending_fill: 每帧写入 FILL_BATCH_SIZE 格 + 逐格 mark_formula/unmark_formula + request_repaint；完成后：重算 + 选区更新 + 入撤销栈）
 │   └─【无数据】LoadState ► Loading(维持repaint) / Failed/Idle(draw_empty_state)
 │
 │   【统一加载覆盖层】位于 if/else 之后、CentralPanel 末尾 ►
 │     LoadState::Loading 时无论 excel_data 是否为 Some
 │     （初次导入 / 重新导入），均以 Area+CENTER_CENTER 锚点
 │     + spinner + 绿色文案"正在解析 Excel 样式与公式，请稍候..."居中覆盖旧内容
 │
 ├─ save_requested（延迟保存：excel_data 借用释放后）► start_async_save
 │
 └─ draw_license_popup(每帧) + 非 intercept 时 license.checkpoint(today)   // 推进高水位防回拨
```

> **借用冲突的解法**：`ui()` 大量使用"先在闭包外收集动作（`pending_action`/`pending_undo`/`save_requested`），
> 释放 `excel_data` 借用后再执行"的模式；撤销的 `push_undo_*` 设计为关联函数同理。`activate_cb` 闭包
> 只捕获 `self.license`，与 `&mut self.license_popup` 是 edition 2021 的不相交字段借用。

---

## 3. 视觉布局与 UI 结构

窗口由 **顶部菜单栏 + 中央主内容区 + 底部双状态栏** 构成，外加一组**前景浮层**。
egui 中 `TopBottomPanel` 按代码顺序从下往上堆叠（先 `show` 的 bottom 面板贴最底），`CentralPanel`
填充剩余空间。

### 3.1 整体窗口分层

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│ 文件  编辑  搜索  配置  转换  关于                                  🔔 预警     │ ① 顶部 menu_bar
├──────────────────────────────────────────────────────────────────────────────────┤
│ [  名称框  ▼ ] │ fx │ =SUM(A1:A10)                              [ 💾 保存 ]   │ ② 名称框行
│                │    │                                              (dirty 高亮) │   (draw_name_box)
├──────────────────────────────────────────────────────────────────────────────────┤
│      │   A      B      C      D      E      ……                       │           │ ③ 列标题(frozen)
│   ───┼───────────────────────────────────────────────────             │           │
│    1 │  cell   cell   cell   cell   cell   ……     ← 行号列(frozen)    │           │ ④ 表格主体
│    2 │  cell   cell   cell   cell   cell   ……                         │  CentralPanel
│    ⋮ │  ……                                            双向滚动 + 冻结窗格     │  ScrollArea::both
│      │                                                                 │           │   (虚拟渲染)
├──────────────────────────────────────────────────────────────────────────────────┤
│  Sheet1 │ Sheet2 │ Sheet3                                                       │ ⑤ sheet_bar
├──────────────────────────────────────────────────────────────────────────────────┤
│ E:\dir\template.xlsx                       E:\dir\template.xlsx（绿色）         │ ⑥ status_bar
└──────────────────────────────────────────────────────────────────────────────────┘
        ▲ 保存完成：右侧绿色显示**原文件路径**（直接覆盖，不再生成日期后缀新文件）；保存中显示 spinner + "正在保存..."
        ▲ 绿色路径**可点击**：点击用系统默认程序打开该文件（`util::open::open_in_default_app`），悬停手型光标 + 提示

 浮层（egui::Area / Window，Order::Foreground，按需显示）：
   设置面板* │ 搜索配置* │ 搜索窗口(非模态) │ 右键菜单 │ 确认弹窗(插入列/清空)   (* 渲染自 config.rs)
   预警消息 │ 预警通知 │ 帮助 │ 转换工具 │ 条件格式 │ 激活/付款
   数据有效性输入提示 │ 数据有效性校验错误(重试/取消)
```

### 3.2 名称框行（`draw_name_box`）细分

水平排列（左侧 `left_to_right` + 最右保存按钮 `right_to_left`）：

```
┌──────────┬───┬───┬──────────────────────────────────┬──────────────┐
│ 名称框80 │ ▼ │fx │  公式输入框 desired_width=400    │  💾 保存      │
│ (跳转)   │   │   │  (Enter 写入选中格)              │ (dirty 启用) │
└──────────┴───┴───┴──────────────────────────────────┴──────────────┘
   ▼ 下拉：定义名称... / 管理名称...      公式栏无焦点时随选中格自动同步显示值/公式
```

- 名称框输入 `"A1"`/`"AA100"` 回车 → 解析为 `(col,row)` 跳转（`parse_cell_reference`，越界忽略）；
- 公式栏：选中格变化时自动回填；用户回车写入 `pending_formula_save`（见 §5.3）。
- 保存按钮：`dirty` 时蓝色高亮、可点击，**悬停显示 `Ctrl+S` 快捷键提示**（`on_hover_text`）；点击或按 `Ctrl+S` 均触发保存（见 §5.4）。

> 名称框组件的完整结构、状态字段、交互逻辑与视觉布局详见 [`names_box.md`](widgets/names_box.md)。

### 3.3 浮层窗口清单

| 浮层 | 触发 | 渲染方式 | 关键约束 |
|------|------|----------|----------|
| 设置面板「插入配置」 | 配置 → 插入配置 | `Window`（无标题栏，420 宽，居中） | 列配置/行配置两个页签 |
| 搜索配置 | 配置 → 搜索配置 | `Window`（420 宽） | 列筛选/行筛选页签，保存写 yaml |
| 搜索窗口 | 搜索 → 搜索 | 非模态 `Window`（520 宽，可折叠） | 详见 [`search.md`](widgets/search.md) |
| 右键菜单 | 表格右键 | `Area`（Foreground） | 220 宽，外部点击/Escape 关闭 |
| 保存失败提示框 | 保存 `Err`（按钮 / Ctrl+S） | `Window`（Foreground，居中） | 红底红边，文案"保存失败!请检查{路径}文件是否被占用打开"，"知道了"关闭 |
| 确认弹窗 | 插入列/清空 | `Window`（Foreground，fixed_pos） | 插入列=普通样式（min 360 宽）；**清空=红色警示样式**（240 宽 × **80 高固定**，红边/浅红底/⚠警告标题/红底确认按钮，破坏性操作）。首帧 established 后才检测外部点击 |
| 预警消息 / 通知 | 配置/自动 | `Window`/图标 | 写回隐藏行列集合 |
| 帮助 / 转换 / 条件格式 | 菜单 | `Window` | — |
| 激活/付款弹窗 | 试用/拦截 | `Window`（模态遮罩） | 拦截态不可关闭 |
| 数据有效性输入提示 | 选中带提示格 | `Area`（Foreground，格左下） | 黄底 `#FFFFE1` |
| 数据有效性校验错误 | 校验失败 | `Area`（Foreground，固定位置） | 重试/取消（恢复原值） |

### 3.4 区域 ↔ 状态字段映射表

| 区域 | UI 元素 | 绑定状态 / 动作 |
|------|---------|----------------|
| 菜单栏 | 文件/编辑/搜索/配置/转换/关于 | 各 `*_popup.visible` / `add_column`/`add_row` / `show_import_dialog` |
| 菜单栏右 | 🔔 预警图标 | `alert_notify_state` |
| 名称框行 | 名称框输入 | `name_box_state.input_text` → `selected_cell` 跳转 |
| 名称框行 | 公式输入框 | `name_box_state.formula_text` → `pending_formula_save` |
| 名称框行 | 💾 保存 | `save_requested`（dirty 启用） |
| 表格主体 | 单元格网格 | `selected_cell`/`selected_range`/`editing_cell`/`edit_value`/`hidden_*`/`shift_click_anchor` |
| sheet 栏 | sheet 标签 | `current_sheet`（切换重置选中/隐藏/预警过滤） |
| 状态栏 | 源路径 / 保存路径 | `file_path`（灰，左）/ `save_path`（绿，右，**可点击 → `util::open` 打开**）/ `saving` |

---

## 4. 事件处理与交互逻辑

事件分**表格内（`table.rs::draw_table_content`）**与**全局（`viewer.rs::ui`）**两层处理。
表格交互的核心是一块覆盖整表的 `ui.interact(rect, "table_interaction", Sense::click_and_drag())`。

### 4.1 鼠标交互（冻结感知的命中测试）

点击→单元格的换算在**累积宽度/高度数组**上做二分（`partition_point`），并区分冻结区/非冻结区
两套坐标系：

```rust
// 累积数组（隐藏列宽贡献为 0），保证索引与 partition_point 点击检测一致
col_cumulative_width[col]   // 前缀和：列 1..col 的宽度
row_cumulative_height[row]

// 点击命中（冻结区用视口相对坐标，非冻结区用表格内容坐标）
let click_x = if in_frozen_left { pos.x - viewport.min.x } else { pos.x - tl_x };
let col = col_cumulative_width.partition_point(|&w| w <= click_x) - 1;
```

| 事件 | 检测 | 行为 |
|------|------|------|
| 左键单击（数据格） | `response.clicked()` | 更新 `selected_cell`（`col>0 && row>0`）；`request_focus`；同时更新 `shift_click_anchor`；双击进入编辑 |
| **左键单击行号** | `response.clicked()` + `col==0` | 选中整行：`selected_range = (1, row, max_col, row)`；`selected_cell = (1, row)`；清除编辑态 |
| **左键单击列号** | `response.clicked()` + `row==0` | 选中整列：`selected_range = (col, 1, col, max_row)`；`selected_cell = (col, 1)`；清除编辑态 |
| Shift+左键单击 | `response.clicked()` + `modifiers.shift` | 从 `shift_click_anchor` 到目标格计算矩形范围 → `selected_range`；活动单元格保持不变；不触发双击编辑 |
| 左键双击 | `response.double_clicked()` | 进入编辑：合并格取左上角；`edit_value` 取公式或显示值；保存原始数据备恢复 |
| 右键单击 | `response.secondary_clicked()` | 打开右键菜单，记录 `target_cell` 与默认插入数（`default_insert_count`） |
| 拖拽选择 | `drag_started()` / `dragged()` | 锚点扩展到所在合并区；拖动时锚点格与当前格各自展开到合并区边界取并集 → `selected_range` |
| 校验弹窗在 | `validation_error_active` | 上述点击/右键/拖拽**全部禁用**，强制先处理校验错误 |
| **悬停溢出单元格** | `response.hovered()` + `overflow_cells` | 若悬停格文本超出列宽且无批注，在指针旁弹出浅灰底 tooltip 完整展示文本内容（最大宽度 400px，自动换行） |

> 合并单元格对齐：拖拽/选中遇到合并区域会"吸附"到整块边界（`expand_to_merge` / `get_merged_range`），
> 避免选中半个合并块。

> **溢出裁剪与 tooltip**：`draw_table_content` 在内容渲染 pass 中测量每个非空单元格文本宽度，
> 超出 `cell_width - 8px` 记入 `overflow_cells` 集合；绘制时通过 `painter.set_clip_rect` 裁剪到
> 单元格边界。悬停检测复用冻结感知坐标系统，与批注气泡共享同一检测块，合并单元格自动解
> 析到左上角。详见 [`table.md`](widgets/table.md) §2.19。

### 4.2 键盘交互

**表格内导航**（仅当表格拥有焦点、非编辑态）：

| 键 | 行为 |
|----|------|
| `←/→/↑/↓` | 单格移动 `selected_cell`，越界自动 `scroll_to_rect`（冻结区偏移补偿）；同步更新 `shift_click_anchor` |
| `Tab` / `Shift+Tab` | 编辑态下提交并右/左移；非编辑态下右/左移选中；同步更新 `shift_click_anchor` |
| `Enter` | 非编辑态进入编辑；编辑态提交 |
| `Ctrl+C` | 非编辑态下复制选中单元格/范围 → TSV → 系统剪贴板（`ctx.copy_text()`） |
| `Ctrl+V` | 非编辑态下粘贴（`Event::Paste`）→ 解析 TSV → 写入 cells → 重算 → 更新选区（详见 [`table.md`](widgets/table.md) §2.17） |

**全局快捷键**（`ui()` 顶层，`ui.input`）：

| 键 | 位置 | 行为 |
|----|------|------|
| `Ctrl+S` | `ui()` | `dirty && !saving` → `start_async_save` |
| `Ctrl+C` | `draw_table_content` | 非编辑态下序列化选中单元格为 TSV → `ctx.copy_text()` 系统剪贴板 |
| `Ctrl+V` | `draw_table_content` | 非编辑态下检测 `Event::Paste` → 解析 TSV → 写入 cells → 重算 → `committed_paste` 撤销信号 |
| `Ctrl+Z`（无 Shift） | CentralPanel | `take_undo()` 回放（借出在 `excel_data` 之前）；**带 `editing_cell.is_none()` 守卫**——编辑模式下不触发，把 `Ctrl+Z` 留给输入框文本内撤销，避免弹出栈中无关动作 |
| `Delete` | CentralPanel | 有内容的选中格/范围 → 弹清空确认（区分范围/单格） |
| `Escape` | 各弹窗 | 关闭右键菜单/确认弹窗/名称框下拉 |
| `Ctrl+A` | 名称框/公式栏输入框 | 全选文本（手动设置光标 `CCursorRange`） |

### 4.3 滚动

- **双向滚动**：主表格用 `ScrollArea::both().id_salt("table_scroll")`（注释说明：嵌套 ScrollArea 会让
  `scroll_to_rect` 无法同时作用两个方向，故用单一双向滚动区）。
- **自动滚动**：① 键盘导航越界 → `ui.scroll_to_rect(target)`；② 插入列/行后 `scroll_to_last_col/row`
  滚到内容区右/下边缘，使新增行列进入可视区。
- **冻结窗格**：`sheet.frozen_rows/frozen_cols` 定义固定表头（`frozen_top_height`/`frozen_left_width`
  从累积数组推导），渲染时主网格跳过冻结行列，由冻结覆盖层单独绘制。

> 项目未实现缩放（zoom / `pixels_per_point`）；缩放/触控板手势走 egui 默认行为。

### 4.4 弹窗"外部点击关闭"机制

右键菜单、名称框下拉、确认弹窗都采用同一套**首帧豁免**模式，避免弹窗刚弹出就被"触发它那次点击"
误关：

```rust
let is_established = self.context_menu.confirm_established;   // 读旧值
self.context_menu.confirm_established = true;                  // 本帧置 true
...
if is_established {                                            // 仅从次帧起检测
    if pointer.any_click() && !menu_rect.contains(hover_pos()) { close(); }
}
```

---

## 5. 关键数据流

### 5.1 文件加载流（import → async → render）

```
菜单"导入" ──► draw_import_dialog(返回 path)
   │
   ▼ start_async_load(path, ctx)
   ├─ (tx, rx)=channel; self.rx=Some(rx); load_state=Loading
   └─ spawn thread（顺序执行，避免阻塞 UI）:
        ├─ util::backup::backup_imported_file(path)  // 先备份到 ~/.MyExcel/backup/（原文件名_yyyymmddhhmmss.ext，失败仅记日志）
        └─ ExcelData::load_from_file(path) ──► tx.send(Ok/Err) ──► ctx.request_repaint()
        │
        ▼ 每帧 check_load_result() ── rx.try_recv()
        ├─ Ok(data): excel_data=Some(data); 清选中/撤销/隐藏集合;
        │            apply_user_cond_format_rules; check_alert_rules(首屏预警) ──► render
        └─ Err(e):   error_message=Some(e); load_state=Failed ──► draw_empty_state
```

### 5.2 渲染流（虚拟渲染）

```
ui() CentralPanel
 │
 ▼ ScrollArea::both("table_scroll")
     ▼ draw_table_content(ui, excel_data, current_sheet, &mut 选中/编辑..., &hidden_*, &mut shift_click_anchor, &mut committed_paste)
        ├─ 构建累积宽高数组（隐藏行列贡献 0）
        ├─ viewport.clip_rect() ± margin ──► partition_point 得可见行列范围（虚拟化）
        ├─ painter 绘制：背景 / 冻结表头 / 可见单元格（值/格式/合并/对齐/条件格式）
        ├─ ui.interact(整表 rect, click_and_drag) 处理点击/右键/拖拽
        └─ 返回 (scroll_target, 选中格 rect) 供外层弹窗定位
```

> 性能要点：① 累积数组 + `partition_point` 把命中检测与可见范围都做到 O(log n)；② 隐藏列宽贡献 0，
> 使索引在搜索隐藏后仍正确；③ 仅渲染 `clip_rect ± 100px margin` 内的单元格。

### 5.3 编辑流（double-click / 公式栏 → 校验 → 重算）

```
双击单元格 / 公式栏 Enter
   │
   ├─【双击】editing_cell=Some(左上角); edit_value=公式或显示值; original_cell_data=原始值
   │
   ▼ 提交（in-cell Enter 或 pending_formula_save）
   ├─ 值以 '=' 开头 ──► cell.formula=value; evaluate_sheet(全表重算)
   └─ 普通值 ──► sheet.validate_cell(col,row,value)
        ├─ 失败: validation_error=Some(标题,消息); 固定位置弹窗(重试/取消恢复原值)
        └─ 通过: 日期格式则 parse_date_string 转序列号; cell.value=值; evaluate_dependents(依赖重算)
   ▼ dirty=true ──► 名称框行"💾 保存"高亮
```

### 5.4 保存流（Ctrl+S / 保存按钮 → 异步 → 状态栏 + 失败红色提示框）

```
Ctrl+S（dirty&&!saving&&有数据）/ 名称框"💾 保存" ──► save_requested=true
   │
   ▼（excel_data 借用释放后）start_async_save(ctx)
   ├─ 授权拦截? ─是► license_popup.visible=true; return
   ├─ output_path = file_path.clone()  // 直接覆盖原文件路径（不再生成日期后缀新文件）
   ├─ pending_save_path = Some(output_path.clone())  // 记录在途路径（= 原路径），供失败提示
   ├─ excel_data.clone(); saving=true; (tx,rx)=channel
   └─ spawn thread: writer::save_to_file(原路径, 数据, 原路径) ──► tx.send ──► request_repaint
        │   （writer 内部 read(原路径)→apply→write(原路径)：原文件先完整读入内存再覆盖，安全）
        ▼ 每帧 check_save_result() ── save_rx.try_recv()（并 take pending_save_path）
        ├─ Ok(path): save_path=Some(path)(=原路径); dirty=false; save_failed=None ──► 状态栏绿色（原路径）
        └─ Err(e):   error_message=Some(e);
                     save_failed=Some("保存失败!请检查{原文件路径}文件是否被占用打开")
                        └─► ui() 渲染居中红色 Foreground 浮窗（"知道了"关闭）
```

> 关键点：① **保存校验点前置**（拦截态禁保存）；② **副本落盘**（克隆 `ExcelData` 给后台线程，
> 主线程继续编辑不阻塞）；③ **直接覆盖原文件**（`output_path = file_path`，不再生成带日期后缀的新文件，
> 原"日期后缀新文件"逻辑 `generate_save_path` 已移除）；④ **失败反馈**：`Err` 时除内部
> `error_message` 外，置 `save_failed` 触发**居中红色提示框**（`egui::Window` + `Frame::popup` 红底红边，
> 区别于状态栏文字），文案 `保存失败!请检查{原文件路径}文件是否被占用打开`；重试成功（`Ok`）会清除该框。
> **两种触发方式（点"保存"按钮 / `Ctrl+S`）都汇入 `start_async_save` → `check_save_result`，失败提示对二者均生效。**
> `{原文件路径}` 取自在途保存的输出路径（`pending_save_path`，与原文件相同）；写盘失败（原文件被占用打开）
> 或重读原文件失败（文件被占用）都会触发——见 `excel::writer::save_to_file` 的两处 `map_err`。

> **绿色路径可点击打开**：保存成功后状态栏右侧的绿色 `save_path` 文本由普通 `Label` 改为
> `Label::sense(Sense::click())`，`Response::clicked()` 时调用 [`util::open::open_in_default_app`](../util/open.md)
> 用系统默认程序打开该文件（Windows 走 `ShellExecuteW`，UTF-16 原生支持任意 Unicode/空格路径，非阻塞）。
> 悬停显示手型光标 + "点击用系统默认程序打开"提示。`on_hover_*` 消费 `Response` 并返回 `Self`，故需链式重绑定后再 `.clicked()`。

### 5.5 搜索 / 筛选流（隐藏集合驱动渲染）

```
搜索窗口（draw_search_window，详见 gui/widgets/search.md）
   ├─ execute_multi_column_search ──► hidden_columns   // 列筛选
   └─ execute_row_search           ──► hidden_rows     // 行筛选
        │
        ▼ 透传 &mut hidden_columns / hidden_rows 到 draw_table_content
        ▼ 渲染时：隐藏列/行宽度贡献为 0 → 既不绘制也不占位 → 视觉上"折叠"
   切换 sheet / 重置搜索 ► hidden_*.clear() + options_loaded=false
```

### 5.6 撤销流

```
结构性/单格/范围操作（插行/插列/清空/编辑）
   ▼ push_undo_full / push_undo_cell / push_undo_range（关联函数，不借 self）
   undo_stack.push(...)，len>20 则丢弃最旧
        │
        ▼ Ctrl+Z（无 Shift）── take_undo()
        ▼ 按变体写回 cells（FullSnapshot 整表替换 / CellChange 单格 / RangeClear 多格）
        ▼ 重算（结构·范围→evaluate_sheet；单格→evaluate_dependents）；dirty=true
```

---

## 附：设计要点汇总

| 主题 | 做法 | 价值 |
|------|------|------|
| 集中式状态 | 所有状态集中在 `ExcelViewer` | 单一 `&mut self` 即可分派，无需事件总线 |
| 异步 I/O | 后台线程 + mpsc + 每帧 `try_recv` | 不阻塞 UI，无需锁 |
| 借用冲突 | 关联函数入栈 / 闭包外收集动作 / 延迟执行 | 在巨型 `ui()` 内维持可借用性 |
| 虚拟渲染 | 累积数组 + `partition_point` + clip_rect 裁剪 | 大表 O(log n) 命中与可见性判定 |
| 冻结窗格 | 单一双向 `ScrollArea` + 冻结覆盖层 | 避免 `scroll_to_rect` 双向失效 |
| 多粒度撤销 | FullSnapshot/CellChange/RangeClear | 按破坏性选粒度，节省内存 |
| 隐藏集合 | 搜索写入 `hidden_*`、表格读取 | 搜索与渲染解耦，行列宽贡献 0 |
| 弹窗关闭 | 首帧 `established` 豁免 + 外部点击检测 | 避免触发点击误关弹窗 |
| 配置持久化 | `~/.MyExcel/my-excel.yaml`（保留其它块） | 插入/搜索配置可复用，与 `search.rs` 解耦 |
| 授权门面 | 每帧 `status` + 拦截态模态 + `checkpoint` | 校验点分散到核心功能（保存等），防时钟回拨 |
| 分批跨帧填充 | `PendingFill` + 每帧写入 FILL_BATCH_SIZE 格 + 逐格 mark_formula/unmark_formula | 大范围填充不阻塞 UI，维持公式位置集精确 |

---

*文档基于 `src/gui/viewer.rs`（截至当前 master）及其直接依赖（`widgets/table.rs`、`names_box.rs`、
`menu_bar.rs`、`gui::state`、`license`）整理。表格内部渲染、搜索/筛选、预警、转换、条件格式的细节
分别见 `table.rs` / [`search.md`](widgets/search.md) / 各组件源码注释。*
