# 名称框组件分析（`src/gui/widgets/names_box.rs`）

> 本文档基于 `names_box.rs`（约 303 行）源码梳理，阐述名称框 + 公式栏组件的架构设计、类型/函数分工、
> 与外部的交互、视觉布局与交互逻辑。配套阅读：主模块 [`viewer.md`](../../viewer.md)。
>
> **术语说明**：本组件是 **egui 立即模式（immediate-mode）** 实现，**不存在** `NamesBox` /
> `NamesBoxBuilder` 这类保留式 Widget 结构，也不直接持有 `Spreadsheet` / `Worksheet` 对象。下文按
> 实际代码（一个状态结构 `NameBoxState` + 一个绘制函数 `draw_name_box` + 一个解析助手）展开分析。

---

## 1. 代码设计概述

### 1.1 模块定位与职责

`names_box.rs` 隶属于 `gui::widgets`，实现 Excel 风格的**名称框 + 公式栏**——即表格网格正上方那一行：
显示当前选中单元格坐标、支持输入坐标快速跳转、编辑当前单元格公式、以及一个保存按钮。

职责拆分：

- **显示**：把外部传入的选中坐标（`selected_cell`）格式化为 `"A1"` 展示；把选中格的公式/值展示到公式栏。
- **输入**：名称框接受坐标输入并跳转；公式栏接受公式/值输入并回写。
- **命令**：保存按钮触发保存（实际由 `viewer.rs` 执行）。
- **占位**：下拉菜单"定义名称.../管理名称..."（名称管理功能未实现，见 §1.7）。

### 1.2 设计范式：立即模式，非保留式 Widget

egui 是立即模式 GUI：**没有跨帧存活的 Widget 对象**，每帧由 `viewer.rs::ui()` 调用一次 `draw_name_box`
重新构建整行 UI。因此：

- **没有 `NamesBox` Widget 结构、没有 Builder 模式**（与某些保留式框架不同）；
- 需要跨帧保留的状态（输入文本、焦点、下拉开关）收敛进独立的状态结构 `NameBoxState`，由 `ExcelViewer`
  持有，每帧以 `&mut` 传入；
- "绘制"与"事件处理"在同一函数内交织完成（读 `ui.input` + 写 `&mut state`）。

### 1.3 结构体与函数清单

| 项 | 可见性 | 类别 | 职责 |
|----|--------|------|------|
| `NameBoxState` | `pub struct`（`#[derive(Clone)]`） | 状态容器 | 跨帧保留名称框/公式栏的全部可变状态 |
| `impl Default for NameBoxState` | `pub`（trait 实现） | 构造 | 提供默认值（空文本、下拉关、固定 `input_id`） |
| `draw_name_box(...)` | `pub fn` | 绘制入口 | 每帧渲染整行并处理交互，返回跳转/保存信号 |
| `parse_cell_reference(input)` | `fn`（私有） | 纯解析助手 | `"A1"`/`"AA100"` → `(col, row)`，失败返回 `None` |

> 本文件**仅定义一个结构体**。没有枚举、没有 trait 实现（除 `Default`）、没有其它辅助函数。

### 1.4 `NameBoxState` 字段

| 字段 | 类型 | 作用 |
|------|------|------|
| `input_text` | `String` | 名称框输入/显示文本（如 `"A1"`） |
| `formula_text` | `String` | 公式栏输入/显示文本 |
| `show_dropdown` | `bool` | 名称框下拉菜单是否展开 |
| `has_focus` | `bool` | 名称框输入框当前是否有焦点（用于反向同步保护） |
| `formula_has_focus` | `bool` | 公式栏输入框当前是否有焦点 |
| `input_id` | `egui::Id` | 名称框输入框固定 ID（`"name_box_input"`），用于稳定焦点/光标状态 |

`input_id` 固定为 `egui::Id::new("name_box_input")`：egui 默认按内容哈希生成控件 ID，而名称框文本会随
选中格变化，固定 ID 可避免 ID 漂移导致的光标/焦点丢失。

### 1.5 公共 API 与私有助手的分工

- **公共**（`NameBoxState` + `draw_name_box`）：是对外契约，由 `viewer.rs` 消费——`ExcelViewer` 持有
  `name_box_state: NameBoxState` 字段，并在 `ui()` 中调用 `draw_name_box`。
- **私有**（`parse_cell_reference`）：纯函数、无 UI 依赖、无副作用，仅服务于 `draw_name_box` 内的
  Enter 跳转判定。单独抽出便于复用与测试（坐标解析是独立关注点）。

### 1.6 与其它"组件"的交互关系（参数解耦）

> 用户题面提到 `Spreadsheet` / `Worksheet`——本组件**并不直接引用这些类型**。与工作表/表格数据的
> 交互完全通过 `viewer.rs` 以**值参数**中转，`names_box` 自身不触碰 `ExcelData` / `SheetData`。

数据流（以 `draw_name_box` 的参数为契约）：

| 方向 | 参数 | 含义 |
|------|------|------|
| viewer → names_box | `selected_cell: Option<(u32,u32)>` | 当前选中格（驱动名称框显示） |
| viewer → names_box | `formula: Option<&str>` | 当前格的显示文本（公式或日期格式化值，由 viewer 计算好） |
| viewer → names_box | `max_col` / `max_row` | 当前工作表边界（用于跳转越界校验） |
| viewer → names_box | `dirty: bool` | 是否有未保存变更（驱动保存按钮启用态） |
| names_box → viewer | 返回 `Option<(u32,u32)>` | 名称框 Enter 跳转目标（viewer 据此设 `selected_cell`） |
| names_box → viewer | 返回 `bool` | 保存按钮是否被点击（viewer 据此触发异步保存） |
| names_box → viewer | `pending_save: &mut Option<String>` | 公式栏 Enter 的待写入值（输出参数；viewer 据此写格 + 重算） |

外部依赖仅一个：`crate::excel::reader::col_to_letter`（列号 → 字母，用于显示）。**不依赖** `ExcelData`、
`SheetData`、表格渲染器等——这是一个纯粹的输入/显示表面，所有数据绑定都由 `viewer.rs` 完成。

### 1.7 状态管理与事件处理流程

**状态归属**：所有状态在 `NameBoxState`（由 `ExcelViewer` 拥有），`draw_name_box` 经 `&mut` 修改；
函数返回值与输出参数把"用户意图"回传 viewer。

**双向绑定（焦点保护）**：名称框/公式栏的显示值由 viewer 提供的 `selected_cell` / `formula` 驱动，但
**仅当对应输入框无焦点时才覆盖**（`if !state.has_focus` / `if !state.formula_has_focus`），避免用户正在
输入时被反向同步打断。

**事件流程**：

```
每帧 draw_name_box(ui, state, selected_cell, formula, max_col, max_row, pending_save, dirty)
 │
 ├─【名称框输入框】
 │   ├─ Ctrl+A ──► 手动设置全选光标（CCursorRange）
 │   ├─ Enter ──► parse_cell_reference ──► 越界校验(col<=max_col && row<=max_row) ──► result=Some
 │   └─ has_focus 记录（供反向同步判定）
 │
 ├─【▼ 下拉按钮】clicked ──► toggle show_dropdown
 │   └─ 展开时：Area(Foreground)+Frame::popup 渲染"定义名称.../管理名称..."
 │      ├─ 项点击 ──► 仅关闭下拉（名称管理未实现，占位）
 │      ├─ 点击菜单/按钮外部 ──► 关闭
 │      └─ Escape ──► 关闭
 │
 ├─【公式栏】（left_to_right：fx 按钮 | 分隔线 | 公式输入框）
 │   ├─ Ctrl+A ──► 全选
 │   ├─ Enter（非空）──► *pending_save = Some(formula_text)
 │   └─ formula_has_focus 记录
 │
 ├─【保存按钮】（right_to_left，dirty 启用）clicked ──► save_clicked=true
 │
 ├─【反向同步】（无焦点时）input_text ◄── col_to_letter(col)+row；formula_text ◄── formula
 │
 └─ 返回 (result, save_clicked)
```

---

## 2. 视觉布局与 UI 结构

### 2.1 布局层次

整行是一个 `ui.horizontal(...)`，内部从左到右依次排列：**名称框 → ▼ 下拉 → 竖分隔线 → 公式栏组 →
保存按钮**。公式栏与保存按钮各自用子布局（`left_to_right` / `right_to_left`）控制对齐。

> 题面提到的 `Scrollbar` / `List` 等嵌套子 Widget 在本组件中**不存在**——这是一行扁平的单行输入控件，
> 无滚动、无列表。

### 2.2 整体布局图

```
┌────────────┬─────┬───┬──────────────────────────────────────┬───────────────┐
│ 名称框 80  │ ▼   │fx │  公式栏 desired_width=400             │  💾 保存      │  ← ui.horizontal
│ (跳转输入) │     │   │  (Enter → pending_save)               │ (dirty 启用)  │
└────────────┴─────┴───┴──────────────────────────────────────┴───────────────┘
      ▲           ▲                                              ▲
      │           └ 下拉展开时：                                 └ right_to_left 子布局
      │              Area(Foreground)+Frame::popup                钉在最右
      │              ┌────────────────────┐
      │              │ 定义名称...        │  min_width=150
      │              │ ─────────────────  │
      │              │ 管理名称...        │
      │              └────────────────────┘
      │              fixed_pos = ▼ 按钮 left_bottom + (0,2)
      └ TextEdit.singleline, id 固定, desired_width=80, hint "名称框"
```

各区块：

| 区块 | 控件 | 宽度/约束 | 绑定状态/动作 |
|------|------|-----------|----------------|
| 名称框 | `TextEdit::singleline` | `desired_width(80.0)`，固定 `input_id` | `input_text`；Enter 跳转 |
| 下拉触发 | `Button::new("▼").small()` | `min_size(20.0, 0.0)` | toggle `show_dropdown` |
| 下拉浮层 | `Area` + `Frame::popup` | `min_width(150.0)`，`fixed_pos` | 占位项（仅关闭） |
| 竖分隔 | `Separator::vertical()` | — | 视觉分隔 |
| fx 按钮 | `Button::new("fx").small()` | — | **无 click 处理（装饰）** |
| 公式栏 | `TextEdit::singleline` | `desired_width(400.0)` | `formula_text`；Enter → `pending_save` |
| 保存按钮 | `Button`（`add_enabled(dirty)`） | — | `save_clicked` |

### 2.3 尺寸计算与动态布局策略

- **固定宽度**：名称框 `80.0`、公式栏 `400.0`（`desired_width`，egui 会以此为期望值，实际可随容器伸缩）。
- **右对齐保存按钮**：在同一个 `horizontal` 内再开一个 `right_to_left` 子布局，egui 会把该子布局分配到
  行末右侧区域，从而把保存按钮钉在最右——这是 egui 中实现"一行内左/右两端控件"的惯用手法。
- **下拉定位**：`fixed_pos(dropdown_response.rect.left_bottom() + vec2(0,2))`——锚定在 ▼ 按钮左下角
  下方 2px，不随滚动/窗口变化漂移；`order(Foreground)` 保证浮在表格之上。
- **字体**：统一取 `TextStyle::Body` 的 `font_id`，clone 后分别用于两个输入框，保证视觉一致。
- **无动态尺寸**：除保存按钮的启用/配色随 `dirty` 变化外，布局尺寸是静态的，不随内容自适应。

### 2.4 用户交互处理

| 交互 | 对象 | 处理逻辑 |
|------|------|----------|
| 输入 + Enter | 名称框 | `parse_cell_reference` 解析 → 越界校验（`col<=max_col && row<=max_row`）→ 返回跳转目标；非法/越界则忽略 |
| Ctrl+A | 名称框 / 公式栏 | 手动构造 `CCursorRange::two(0, len)` 并 `TextEdit::store_state`（egui 单行输入默认 Ctrl+A 行为不理想，故显式实现全选） |
| 点击 ▼ | 下拉按钮 | 翻转 `show_dropdown` |
| 点击菜单项 | 下拉浮层 | 仅 `show_dropdown=false`（名称管理未实现） |
| 点击外部 | 下拉浮层 | `pointer.any_click()` 且 `hover_pos` 不在菜单/按钮 rect 内 → 关闭 |
| Escape | 下拉浮层 | `key_pressed(Escape)` → 关闭下拉 |
| 输入 + Enter | 公式栏 | 非空则 `*pending_save = Some(formula_text)`（viewer 消费写格） |
| 点击保存 | 保存按钮 | `save_clicked=true`（仅 `dirty` 时可点） |
| 悬停提示（dirty 时） | 保存按钮 | `on_hover_text("Ctrl+S")`：蓝色激活态悬停显示快捷键提示 |

**焦点追踪**：每帧把 `input_response.has_focus()` / `formula_response.has_focus()` 写回 `state.has_focus` /
`state.formula_has_focus`，作为反向同步（viewer→显示）的"免打扰"门控。

### 2.5 样式与外观配置

- **保存按钮配色**（随 `dirty` 双态）：

  | 状态 | 文字色 | 填充色 | 含义 |
  |------|--------|--------|------|
  | `dirty=true` | 白 `#FFFFFF` | 蓝 `#0070C0`（与单元格选中色一致） | 高亮"可保存" |
  | `dirty=false` | 灰 `#A0A0A0` | 浅灰 `#DCDCDC` | 灰显 + `add_enabled` 禁用 |

- **下拉浮层**：`Frame::popup(ui.style())` 沿用 egui 默认弹出框样式（浅底 + 阴影 + 圆角）。
- **分隔**：区段间用 `Separator::vertical()` 竖线视觉分组（名称框/下拉 ‖ 公式栏组；fx ‖ 公式输入）。
- **字体**：统一 `TextStyle::Body`；标题性提示用 `hint_text`（"名称框" / "输入公式..."）。
- **按钮**：`▼` 与 `fx` 用 `.small()` 紧凑尺寸。

---

## 3. "保存"按钮业务逻辑详解

> **重要澄清（请先读这段）**：`names_box.rs` 中的"保存"按钮**本身几乎不含业务逻辑**——点击只置一个布尔
> 标志 `save_clicked=true` 并随返回值传出。真正的保存流程（路径生成、授权校验、异步落盘、错误回收、
> 状态更新）**全部位于 [`viewer.rs`](../../src/gui/viewer.rs)**，详见 [`viewer.md`](../../viewer.md) §5.4
> 保存流。此外，该按钮保存的是**整个工作簿文件**，与"Excel 名称管理器"**没有任何交互**——名称管理在本
> 项目中是未实现的占位功能（见 §3.6）。下文按「触发 / 校验 / 保存流程 / 界面状态 / 异常处理 / 名称管理
> 交互」逐项**如实**说明，对代码中并不存在的部分会明确标注，避免误导。

### 3.1 触发条件

按钮在 `draw_name_box` 内以 `add_enabled` 渲染（`names_box.rs:257-268`）：

```rust
let save_btn = ui.add_enabled(
    dirty,                                              // ← 可点击前置条件
    egui::Button::new(btn_text).fill(/* dirty 双态配色 */),
);
if save_btn.clicked() {
    save_clicked = true;                                // ← 唯一副作用
}
```

- **`dirty=false`**：按钮禁用（灰显、不可点击）。
- **`dirty=true`**：可点击；点击 → `save_clicked=true`，经返回值 `(Option<_>, bool)` 传回 `viewer.rs`。
- **等价入口**：`Ctrl+S`（`viewer.rs:731`）也调用同一个 `start_async_save`，与按钮殊途同归。按钮在
  蓝色激活态（`dirty`）悬停时会以 `on_hover_text("Ctrl+S")` 显示该快捷键，向用户暴露等价入口。

即触发条件 = `dirty==true` 且用户点击保存按钮（或按 `Ctrl+S`）。

### 3.2 输入校验规则

> **结论：保存按钮路径上没有任何"输入校验"。**

- 按钮不读取 `input_text` / `formula_text`，不校验任何单元格内容；
- 项目中的**数据有效性校验**（`SheetData::validate_cell` / `validation_error`）发生在**单元格编辑提交时**
  （公式栏 Enter / 双击编辑提交，由 `viewer.rs` 处理），**与保存按钮无关**；
- 保存流程里唯一的"校验门控"是**授权状态**：`start_async_save` 首行检查 `license.status(...).is_blocking()`
  （`viewer.rs:493-497`）——这是**权限校验**，不是数据校验。

所以「能否点保存」的唯一前置条件是 `dirty`；点下后也不会对工作簿内容做任何合法性校验。

### 3.3 数据保存流程（跨 `names_box.rs` → `viewer.rs`）

按钮只是整条链路的**第①步**：

```
① names_box.rs:257-268   save_btn.clicked() ──► save_clicked = true
② viewer.rs:939-940      if save_clicked { self.save_requested = true; }      // 记录请求
③ viewer.rs:1755-1757    if self.save_requested {
                            self.save_requested = false;
                            start_async_save(ctx.clone());                      // 延后执行
                         }
④ viewer.rs:498  start_async_save:
     ├─ 授权拦截（blocking ──► 弹激活窗并 return）
     ├─ output_path  = generate_save_path()   // stem_YYYYMMDD.ext（Howard Hinnant 算法，无 chrono）
     ├─ pending_save_path = Some(output_path.clone())   // 记录在途路径，供失败提示
     ├─ original_path = file_path
     ├─ excel_data    = self.excel_data.clone()
     ├─ saving=true; (tx,rx)=channel; save_rx=Some(rx)
     └─ spawn 线程: writer::save_to_file(original, data, output)
                       └─► tx.send(Ok(path) | Err(e)) ──► ctx.request_repaint()
⑤ viewer.rs:723 + 538  check_save_result（每帧）: rx.try_recv()（并 take pending_save_path）
     ├─ Ok(path) ──► save_path = Some(path); dirty = false; save_failed = None   // 重试成功清提示
     └─ Err(e)   ──► error_message = Some(e);
                   save_failed = Some("保存失败!请检查{output_path}文件是否被占用打开")
     saving=false; save_rx=None
⑥ ui(): save_failed.is_some() ──► 渲染居中红色 Foreground 浮窗（"保存失败!请检查{路径}..."，"知道了"关闭）
```

要点：

- **延迟执行（②→③）**：按钮回调发生在 `draw_name_box` 内（此时 `viewer.rs` 仍持有 `excel_data` 借用），
  无法立即 `start_async_save`（它需要 `excel_data.clone()`）。故先置 `save_requested` 标志，待 CentralPanel
  闭包结束、借用释放后，在 `viewer.rs:1755` 才真正执行——这是 viewer 处理借用冲突的惯用"命令标志"模式。
- **副本落盘**：后台线程持有 `excel_data` 的克隆，主线程可继续编辑，UI 不阻塞。
- **输出带日期后缀的新文件**（`stem_YYYYMMDD.ext`，`generate_save_path`，`viewer.rs:463-489`），**不改原模板**。

### 3.4 界面状态变化

按钮自身（`names_box.rs`）：

- `dirty` 驱动**配色与启用**（见 §2.5 配色表）：`dirty=true` 白字蓝底高亮、可点；`dirty=false` 灰字浅灰底、禁用。
- 点击瞬间仅有 egui 按钮默认的按压视觉，无额外反馈。

保存流程驱动的界面变化（`viewer.rs` / 底部状态栏）：

| 阶段 | 状态字段 | 界面表现 |
|------|----------|----------|
| 保存中 | `saving=true` | 状态栏右侧 spinner + "正在保存..."；每帧 `request_repaint` 驱动动画 |
| 成功 | `save_path=Some(path)`、`dirty=false` | 状态栏显示**绿色**新路径（带日期后缀）；保存按钮变灰禁用 |
| 失败 | `error_message=Some(e)`、`save_failed=Some(...)` | 弹出**居中红色提示框**（Foreground 浮窗，非状态栏文字）"保存失败!请检查{路径}文件是否被占用打开"，点"知道了"关闭 |
| （始终） | 名称框/公式栏文本 | 不受保存影响 |

### 3.5 异常处理机制

- **授权拦截**：拦截态点保存 → **不保存**，弹出激活/付款模态（`license_popup.visible=true`）后 `return`
  （`viewer.rs:500-503`）。
- **路径缺失**：`file_path` 为 `None` 或 `generate_save_path()` 返回 `None` → 静默 `return`，不保存
  （`viewer.rs:504-515`）。
- **写盘失败（红色提示框，本次新增反馈）**：`writer::save_to_file` 返回 `Err(e)`（重读原文件失败 = 模板被占用；
  写入输出失败 = 输出文件被占用）→ `check_save_result`（`viewer.rs:538`）置 `error_message=Some(e)` **并**
  `save_failed=Some("保存失败!请检查{output_path}文件是否被占用打开")`；`ui()` 据此渲染**居中红色提示框**
  （`egui::Window` + `Frame::popup` 红底红边，Foreground 浮窗，**区别于状态栏文字**），点"知道了"关闭。
  **两种触发方式（点"保存"按钮 / Ctrl+S）都汇入 `start_async_save` → `check_save_result`，故对二者均生效。**
- **重试自清除**：用户关闭占用文件后再次保存，若返回 `Ok` 则 `save_failed=None` 自动收起提示框。
- **无自动重试**：失败不自动重试，需用户手动再次触发保存。
- **名称框侧无任何异常处理**：按钮点击"不会失败"（只是置一个布尔）。

### 3.6 与"Excel 名称管理器"的交互

> **没有交互。本项目不存在 Excel 名称管理器功能。**

- 名称框下拉里的「定义名称...」「管理名称...」（`names_box.rs:183-189`）是**占位 UI**：点击仅
  `show_dropdown=false`，未实现任何名称的增删改查，也**不读写工作簿的命名区域（defined names）**。
- 「保存」按钮保存的是**整个工作簿**（所有 sheet 的单元格、公式、样式、合并、数据有效性等，经
  `writer::save_to_file`），与"名称管理"是**两件相互独立的事**——请勿把保存按钮误当作名称管理的一部分。
- 若未来实现名称管理，它应挂在下拉菜单项下，与保存按钮无耦合。

### 3.7 完整时序（按钮 → 落盘）

```
用户（dirty=true）── 点 [💾 保存] ──► names_box: save_clicked = true
      │
      ▼ viewer.rs
save_requested = true ──（excel_data 借用释放后）──► start_async_save(ctx)
      │
      ├─ license blocking? ──是──► 弹激活窗，结束（不保存）
      │
      └─ 否：saving=true；pending_save_path=Some(output)；spawn 线程 ──► writer::save_to_file ──► tx.send(Ok|Err)
                                                                     │
      每帧 check_save_result ◄── try_recv（take pending_save_path）──┘
            ├─ Ok(path) ──► 状态栏绿色路径；dirty=false；save_failed=None；按钮灰显
            └─ Err(e)   ──► error_message + save_failed ──► 居中红色提示框
                          （"保存失败!请检查{output}文件是否被占用打开"，"知道了"关闭）
```

> 与 [`viewer.md`](../../viewer.md) 的关系：本节只刻画按钮侧的"发信号"；授权门面、日期后缀路径、异步通道、
> 状态栏反馈的完整实现均在 `viewer.rs`，详见 [`viewer.md`](../../viewer.md) §2.6（方法表）与 §5.4（保存流）。

---

## 附：未完成 / 占位功能与改进建议

### 占位功能（当前无实际行为）

| 项 | 现状 | 说明 |
|----|------|------|
| "定义名称..." / "管理名称..." | 点击仅关闭下拉 | 名称管理功能未实现，下拉为占位 UI |
| "fx" 按钮 | 无 `clicked` 处理 | 纯装饰，未接入函数插入向导 |

### 改进建议

| # | 问题 | 建议 |
|---|------|------|
| 1 | **参数较多（8 个）**，且 `pending_save` 用输出参数而非返回值 | 可把返回值扩为 `(jump, save_clicked, pending_save)` 三元组，或聚合一个 `NameBoxOutcome` 结构体，消除输出参数、降低调用方心智负担 |
| 2 | **名称框与公式栏的 Ctrl+A 全选逻辑重复**（两段几乎相同的 `load_state`/`store_state` 代码） | 抽私有助手 `fn select_all(ui, response, text)`，消除重复 |
| 3 | **"定义名称.../管理名称..."与"fx"为死 UI** | 要么实现，要么移除/隐藏，避免误导用户；至少补 TODO 注释 |
| 4 | **下拉关闭采用"每帧检测 any_click + hover_pos"** | egui 提供 `ui.menu_button` / `ComboBox` 等自带"外部点击关闭"的组件，可改用以减少手写关闭逻辑与首帧豁免隐患 |
| 5 | **硬编码宽度（80/400/150）** | 提为具名常量（`NAME_BOX_WIDTH` 等），便于统一调整 |
| 6 | **`input_id` 固定字符串** | 当前合理（防 ID 漂移）；若未来同屏多实例需改成构造时传入唯一 ID |

---

*文档基于 `src/gui/widgets/names_box.rs`（截至当前 master）整理。名称框的调用上下文（选中格、公式显示值、
`dirty`、保存/跳转消费）见 [`viewer.md`](../../viewer.md) §2.7 控制流与 §5 数据流。*
