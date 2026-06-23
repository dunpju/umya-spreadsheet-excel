# 预警消息系统

## 概述

预警消息系统由三个核心模块组成：

| 模块 | 文件 | 职责 |
|------|------|------|
| **规则配置弹窗** | `src/gui/widgets/alert_popup.rs` | 管理预警规则的增删改查，YAML 持久化 |
| **通知图标 + 消息弹窗** | `src/gui/widgets/alert_notify.rs` | 菜单栏图标绘制、规则触发检测、点击过滤、弹窗交互 |
| **集成层** | `src/gui/widgets/menu_bar.rs` / `src/gui/viewer.rs` | 串联各模块，驱动检测循环、范围扩展更新、过滤状态管理 |

---

## 一、数据结构

### 1.1 AlertRule（预警规则）

定义于 `alert_popup.rs`，描述一条预警规则的全部信息：

```rust
pub struct AlertRule {
    pub operator: String,        // 比较运算符：=, !=, >, <, >=, <=
    pub value: String,           // 阈值
    pub message: String,         // 预警消息内容
    pub range: String,           // 应用范围（如 "=B8:~8"）
    pub range_expand_col: u32,    // 固定范围的列扩展偏移量（插入列时累加）
    pub range_expand_row: u32,    // 固定范围的行扩展偏移量（插入行时累加）
}
```

所有规则通过 `AlertPopupState` 管理，支持 YAML 文件加载/保存。

### 1.2 TriggeredRule（已触发规则）

定义于 `alert_notify.rs`，由触发检测函数生成，携带解析后的范围信息：

```rust
pub struct TriggeredRule {
    pub message: String,                          // 规则消息
    pub range: String,                            // 原始范围字符串
    pub operator: String,                         // 规则运算符
    pub value: String,                            // 规则阈值
    pub resolved_range: Option<(u32, u32, u32, u32)>, // 解析后范围: (start_col, start_row, end_col, end_row)
    pub is_horizontal: bool,                       // true=横向（同行多列），false=纵向（同列多行）
}
```

### 1.3 AlertNotifyState（通知弹窗状态）

定义于 `alert_notify.rs`，控制图标显隐、弹窗开关、过滤状态：

```rust
pub struct AlertNotifyState {
    pub visible: bool,                // 弹窗是否可见
    pub triggered_rules: Vec<TriggeredRule>,  // 当前已触发的规则列表
    pub has_triggered: bool,          // 是否有任意规则被触发（控制图标显隐）
    pub is_filtering: bool,           // 当前是否处于过滤状态
    pub collapsed: bool,              // 弹窗是否折叠（默认展开）
}
```

---

## 二、动态显隐逻辑

菜单栏最右侧有一个黄色实心灯泡图标（`draw_alert_icon`），其显隐由 `has_triggered` 字段控制：

- **有任何规则被触发** → 图标显示
- **没有规则被触发** → 图标隐藏

触发检测在 `viewer.rs` 中**每帧执行**（`check_alert_rules`），数据变化后自动更新。

### 图标绘制细节

- 固定 **18×18 像素**，不闪烁（避免因布局尺寸变化导致下方表格震动）
- 由 `circle_filled`（灯泡头部）+ `rect_filled`（颈部 + 底座）三个图形拼合
- 鼠标悬停显示提示：`N 条预警规则已触发，点击查看详情`
- 点击后切换弹窗可见性，每次打开时强制为**展开状态**

---

## 三、规则触发检测

### 3.1 范围解析（`parse_alert_range`）

支持以下格式：

| 格式 | 说明 | 方向 |
|------|------|------|
| `=B8:~8` | 从 B8 开始，同行向右扩展至 `max_col` | 横向 |
| `=B8:D8` | 固定横向范围 B8→D8 | 横向 |
| `=B8:B~` | 从 B8 开始，同列向下扩展至 `max_row` | 纵向 |
| `=B8:B12` | 固定纵向范围 B8→B12 | 纵向 |
| `=B8:~` | 全方向扩展至 (max_col, max_row) | 取决于 start_row==end_row |

动态范围（含 `~`）由 `resolve_dynamic_range` 函数解析为实际单元格坐标：
- `~8` → 替换为 `(max_col)(8)`
- `B~` → 替换为 `B(max_row)`
- `~` → 替换为 `(max_col)(max_row)`

固定范围（无 `~`）额外加上 `range_expand_col` / `range_expand_row` 偏移量，实现插入操作后范围自动扩展。

方向判断规则：`start_row == end_row` → 横向，否则 → 纵向。

### 3.2 比较运算（`compare_values`）

支持以下运算符，优先尝试数值比较（`f64`），回退字符串比较（不区分大小写）：

| 运算符 | 数值比较 | 字符串比较 |
|--------|----------|------------|
| `=` | `(cv - tv).abs() < EPSILON` | `cv == tv`（大小写不敏感） |
| `!=` | `!equal` | `!equal` |
| `>` | `cv > tv` | 字典序比较 |
| `<` | `cv < tv` | 字典序比较 |
| `>=` | `cv >= tv` | 字典序比较 |
| `<=` | `cv <= tv` | 字典序比较 |

### 3.3 检测流程（`check_alert_rules`）

1. 遍历每条 `AlertRule`
2. 调用 `parse_alert_range` 解析范围（传入 `range_expand_col` / `range_expand_row`）
3. 根据方向遍历范围内的单元格：
   - **横向**：同行从 `start_col` 到 `end_col`，调用 `sheet.get_cell(start_row, col)`
   - **纵向**：同列从 `start_row` 到 `end_row`，调用 `sheet.get_cell(row, start_col)`
4. 任一单元格满足比较条件 → 该规则标记为已触发
5. 收集所有已触发规则为 `Vec<TriggeredRule>` 返回

**调用位置**：`viewer.rs` 每帧调用，结果存入 `alert_notify_state.triggered_rules`。

---

## 四、范围自动扩展机制

当用户执行插入列/行、添加列/行操作时，固定范围（无 `~`）需要相应扩展。通过累加偏移量实现：

### 4.1 列扩展（`update_alert_range_expansions_for_col`）

- **跳过**：含 `~` 的动态范围（已自动跟随 `max_col`）
- **条件**：`insert_col >= start_col && insert_col <= end_col + range_expand_col + 1`
- **操作**：`range_expand_col += n`

### 4.2 行扩展（`update_alert_range_expansions_for_row`）

- **跳过**：含 `~` 的动态范围（已自动跟随 `max_row`）
- **条件**：`insert_row >= start_row && insert_row <= end_row + range_expand_row + 1`
- **操作**：`range_expand_row += n`

### 4.3 调用位置（viewer.rs）

| 操作 | 调用 |
|------|------|
| 添加行（菜单） | `update_alert_range_expansions_for_row(rules, new_row, 1, sheet)` |
| 右键菜单 - 上方插入行 | `update_alert_range_expansions_for_row(rules, anchor_row, n, sheet)` |
| 右键菜单 - 下方插入行 | `update_alert_range_expansions_for_row(rules, anchor_row + 1, n, sheet)` |
| 右键菜单 - 左侧插入列 | `update_alert_range_expansions_for_col(rules, anchor_col, m, sheet)` |
| 右键菜单 - 右侧插入列 | `update_alert_range_expansions_for_col(rules, anchor_col + 1, m, sheet)` |
| 确认对话框 - 左侧插入列 | `update_alert_range_expansions_for_col(rules, anchor_col, m, sheet)` |
| 确认对话框 - 右侧插入列 | `update_alert_range_expansions_for_col(rules, anchor_col + 1, m, sheet)` |

### 4.4 示例

- `=B8:~8`（动态）：表格最右列为 AK，实际范围 `B8:AK8`。添加 2 列后，`max_col` 自动变为 AM，实际范围自动变为 `B8:AM8`。
- `=B8:D8`（固定）：在 C 列左侧插入 2 列后，`range_expand_col` 累加 2，实际范围变为 `B8:F8`。

---

## 五、点击过滤逻辑

### 5.1 过滤函数（`filter_by_triggered_rule`）

点击弹窗中的某条预警消息后触发：

1. **清空** `hidden_columns` 和 `hidden_rows`（每次只应用一条规则的过滤）
2. 根据方向遍历范围内单元格：
   - **横向**：不匹配的列加入 `hidden_columns`，空单元格视为不匹配
   - **纵向**：不匹配的行加入 `hidden_rows`，空单元格视为不匹配
3. 调用合并单元格对齐函数：
   - 横向 → `expand_hidden_for_merged_cols`（处理跨列合并）
   - 纵向 → `expand_hidden_for_merged_rows`（处理跨行合并）

### 5.2 合并单元格处理

**跨列合并**（横向过滤时）：
- 检查范围行所在的所有跨列合并区域
- 左上角单元格**可见** → 整个合并范围的所有列设为可见
- 左上角单元格**隐藏** → 整个合并范围的所有列设为隐藏

**跨行合并**（纵向过滤时）：
- 检查范围列所在的所有跨行合并区域
- 左上角单元格**可见** → 整个合并范围的所有行设为可见
- 左上角单元格**隐藏** → 整个合并范围的所有行设为隐藏

### 5.3 过滤生效机制

`hidden_columns` 和 `hidden_rows` 存储在 `ExcelViewer` 中，传递给 `draw_table_content`（`table.rs`），在以下渲染阶段跳过隐藏的列/行：

- Pass 1（单元格背景绘制）
- Pass 2（单元格内容绘制）
- 冻结区域顶部（列标题 + 数据行）
- 冻结区域左侧（行号 + 数据行）
- 四角区域（冻结行列交叉）
- 列累计宽度计算

### 5.4 重置

`reset_filter` 函数清空 `hidden_columns`、`hidden_rows`，并设置 `is_filtering = false`。由以下操作触发：
- 弹窗 **「✖ 关闭」** 按钮
- 弹窗 **「🔄 重置」** 按钮
- **切换工作表**时（`viewer.rs` 自动调用 `hidden_columns.clear()` + `hidden_rows.clear()`）

---

## 六、预警消息弹窗（`draw_alert_notify_popup`）

### 6.1 布局结构

```
┌──────────────────────────────┐
│ ▼  ⚠ 预警消息      🔄 重置 ✖│  ← 自定义标题栏（可点击折叠/展开）
│──────────────────────────────│
│ 规则消息1（红色文字，可点击）  │  ← 滚动列表区（最大高度 180px）
│ 规则消息2（红色文字，可点击）  │
│ 规则消息3（红色文字，可点击）  │
│                              │
│ 💡 点击预警消息过滤...        │  ← 底部固定提示行（始终可见）
└──────────────────────────────┘
```

### 6.2 弹窗特性

- **宽度**：固定 300px
- **位置**：屏幕（视口）**水平垂直居中**（`.anchor(Align2::CENTER_CENTER, Vec2::ZERO)`，窗口中心对齐屏幕中心；每帧重新锚定，故展开/折叠动画改变高度时仍保持居中）。早期版本用 `default_pos(content_rect().right_center() - vec2(320,0))` 定位在屏幕右侧居中、整体偏下，现已改为居中
- **不可调整大小**、**不可拖拽**（`anchor` 会自动置 `movable=false`）、**无标题栏**、**不可折叠**（通过 `egui::Window` 属性控制）
- **展开/折叠动画**：使用 `egui::animate_value_with_time`（200ms）+ smoothstep 缓动函数，避免手动逐帧驱动造成的卡顿
- **折叠时**：仅显示标题栏，点击标题栏展开

### 6.3 规则列表项

- 所有预警消息文字为**红色**（`Color32::RED`），字号 12px
- 有自定义消息时显示自定义消息；无自定义消息时自动生成：`规则N: [operator] [value]  (范围: [range])`
- 悬停提示：`点击过滤 | [operator] [value] | 横向/纵向 | 范围: [range]`
- 无触发规则时显示灰色占位文本：`暂无触发的预警规则`

---

## 七、菜单栏集成

### 7.1 入口

菜单栏中「预警消息」按钮（`menu_bar.rs`）打开规则配置弹窗（`alert_popup`）。

### 7.2 图标位置

菜单栏最右侧使用 `right_to_left` 布局，调用 `draw_alert_icon` 绘制警示灯泡图标。

---

## 八、状态生命周期

| 事件 | 处理 |
|------|------|
| **每帧渲染** | `check_alert_rules` 检测规则触发 → 更新 `has_triggered` / `triggered_rules` |
| **打开文件** | 清空 `hidden_columns` / `hidden_rows` |
| **切换工作表** | 清空 `hidden_columns` / `hidden_rows`，重置 `alert_notify_state` 全部字段 |
| **插入/添加列** | `update_alert_range_expansions_for_col` 更新偏移量 |
| **插入/添加行** | `update_alert_range_expansions_for_row` 更新偏移量 |
| **点击预警消息** | `filter_by_triggered_rule` 执行过滤 → 设置 `is_filtering = true` |
| **点击重置/关闭** | `reset_filter` 清空隐藏集合 + `is_filtering = false` |

---

## 九、约束条件

所有上述改动不得破坏菜单栏及表格现有的任何功能，包括但不限于：
- 其他菜单项的正常点击
- 弹窗交互
- 表格数据操作（冻结区域、搜索过滤等共用 `hidden_columns` / `hidden_rows`，互不冲突）
