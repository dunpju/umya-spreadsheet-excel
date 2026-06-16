# 搜索弹窗（search_window）UI 布局结构分析

> 源文件：`src/gui/widgets/search.rs` · 核心函数：`draw_search_window`（行 1031–1304）
> 弹窗类型：**非模态**窗口，可独立于主窗口操作。

---

## 一、窗口层（Window）配置

`draw_search_window` 通过 `egui::Window` 创建一个非模态窗口，配置如下（行 1044–1050）：

| 属性 | 值 | 含义 |
|------|-----|------|
| `title_bar` | `false` | 不使用系统原生标题栏，改用自定义标题栏 |
| `open` | `&mut keep_open` | 受 `keep_open` 控制；关闭时把 `state.visible` 置 `false`（行 1301–1303） |
| `resizable` | `false` | 禁止拉伸，尺寸固定 |
| `collapsible` | `false` | 禁止折叠 |
| `default_pos` | `content_rect().center() - vec2(210, 70)` | 出现在视口中心偏左、偏上一点（窗口 min_width=440，一半约 220） |
| `set_min_width` | `440.0` | 内容区最小宽度 440px（行 1051） |

窗口内部 `ui` 是默认的**垂直（top-down）布局**，所有子区块自上而下堆叠。

---

## 二、组件层级树

```
Window "search_window"  (垂直布局, min_width=440)
│
├── ① 标题栏  ui.horizontal
│   ├── Label "搜索"  (size 13, strong/加粗)
│   ├── [条件: is_searching]
│   │   ├── add_space(16)
│   │   ├── Label "匹配 {matched}/{total} 列"  (size 11, 绿色 0,130,0)
│   │   └── [条件: use_binary_search]
│   │       └── Label "(二分)"  (size 10, 灰色 100,100,100)
│   └── 右侧按钮组  ui.with_layout(right_to_left, Center)
│       ├── Button "✖"            ← 最右
│       ├── Button "🔍 搜索"      ← 左移一位 (enabled = has_col||has_row)
│       └── Button "🔄 重置"      ← 再左移一位 (最左)
│
├── ui.separator  ← 标题栏分隔线
│
├── ② 延迟加载  (非 UI：首次加载 column_options / row_filters)
│
├── ③ 列筛选行  ui.horizontal
│   ├── Label "列筛选:"
│   ├── ComboBox  id="search_column_select", width=166
│   │   └── [下拉] selectable_label × N  "title (cell_ref)"
│   ├── add_space(6)
│   └── TextEdit (singleline)  desired_width=∞, hint="输入搜索关键字..."
│        └── [监听] Enter 键 → 统一执行列筛选+行筛选
│
├── ④ 行筛选区  [条件: !row_filters.is_empty()]
│   ├── add_space(4) → separator → add_space(4)
│   └── for idx in 0..filter_count  (每项一行 ui.horizontal)
│       ├── Label "{title} ({cell_ref}):"
│       ├── TextEdit (singleline)  desired_width=∞
│       │            hint="xxxx 或 'xx1','xx2' 或 'xx3'-'xx4'"
│       │   └── [监听] Enter 键 → 统一执行列筛选+行筛选
│       └── add_space(2)  [条件: 非最后一项]
│   └── [条件: is_row_searching && !row_debug_info空]
│       └── Label 诊断信息  (size 10, 绿色 0,100,0)
│
├── ⑤ 列筛选诊断  [条件: is_searching && !debug_info空]
│   └── Label 诊断信息  (size 10, 灰色 100,100,100)
│
└── ⑥ 底部提示
    ├── add_space(4)
    └── Label "💡 搜索选中列右侧所有列；已排序数据自动启用二分查找"
              (size 10, 灰色 140,140,140)
```

---

## 三、视觉布局图（自上而下）

```
┌──────────────────────────────────────────────────────────────────┐
│  search_window   ← 非模态 / 固定尺寸 / 最小宽度 440px            │
│                                                                  │
│ ┌─── 标题栏 (horizontal) ──────────────────────────────────────┐ │
│ │ 搜索   匹配 8/12 列 (二分)            🔄重置  🔍 搜索  ✖   │ │
│ │ ↑加粗13   ↑绿色11     ↑灰10           └ right_to_left ┘    │ │
│ └──────────────────────────────────────────────────────────────┘ │
│ ════════════════════════ separator ═════════════════════════════ │
│                                                                  │
│ ┌─── 列筛选行 (horizontal) ───────────────────────────────────┐ │
│ │ 列筛选: ┌─────────────────┐   ┌──────────────────────────┐  │ │
│ │         │ 序号 (A1)     ▼ │   │ 输入搜索关键字...        │  │ │
│ │         └ width=166 ─────┘   └ desired_width=∞ ────────┘  │ │
│ └──────────────────────────────────────────────────────────────┘ │
│                                                                  │
│ ════════════════════════ separator ═════════════════════════════ │
│  (以下仅当 row_filters 非空时出现)                              │
│                                                                  │
│ ┌─── 行筛选行 0 (horizontal) ─────────────────────────────────┐ │
│ │ 日期 (A14):   xxxx 或 'xx1','xx2' 或 'xx3'-'xx4'           │ │
│ └──────────────────────────────────────────────────────────────┘ │
│ ┌─── 行筛选行 1 (horizontal) ─────────────────────────────────┐ │
│ │ 入库 (D14):   <desired_width=∞>                            │ │
│ └──────────────────────────────────────────────────────────────┘ │
│   ……（每个 row_filter 渲染一行，行间 add_space 2）……            │
│                                                                  │
│   行筛选[串行]: [日期=A, 入库=D] 行15→...  匹配N行 隐藏M行      │  ← 绿色10
│                                                                  │
│   选中A1 | 行1 B→X 共12列 | 匹配8列 隐藏4列 | 匹配: B='xx'...   │  ← 灰色10
│                                                                  │
│ 💡 搜索选中列右侧所有列；已排序数据自动启用二分查找             │  ← 灰色10
└──────────────────────────────────────────────────────────────────┘
```

---

## 四、各区域尺寸约束 / 排列方式汇总

| 区块 | 布局方向 | 关键尺寸 | 备注 |
|------|----------|----------|------|
| 标题栏 | 水平 | "搜索" label + `right_to_left` 子区占满右侧 | 按钮组用 `right_to_left` 让按钮靠右贴边 |
| 按钮组 | 水平(右→左) | 自适应 | 添加顺序 ✖→🔍→🔄，故视觉左→右为 `🔄 🔍 ✖` |
| ComboBox | 下拉 | `width=166.0` 固定 | 固定宽，保证下拉不被挤压 |
| 列筛选 TextEdit | 水平 | `desired_width=∞` | 撑满 440−166−6−label 后剩余宽度 |
| 行筛选 TextEdit | 水平 | `desired_width=∞` | 每行各一个，撑满 label 右侧 |
| 行间距 | — | `add_space(2)` | 仅非末项；区块间用 `add_space(4)+separator+add_space(4)` |

---

## 五、交互区域与行为划分

### 1. 标题栏右侧三个按钮（行 1080–1133）

| 按钮 | 触发条件 | 行为 |
|------|----------|------|
| `✖` | 点击 | `state.visible = false`（关闭弹窗） |
| `🔍 搜索` | `has_col_input \|\| has_row_input` 时才可点 | **统一执行**：列筛选(`execute_search`)+行筛选(`execute_row_search`)；无输入的分支自动 `clear()` 旧结果 |
| `🔄 重置` | 点击 | 清空 `hidden_columns` / `hidden_rows`、所有关键字、统计与诊断 |

> `has_col_input` = 有数据 && 选中列有效 && `search_keyword` 非空；
> `has_row_input` = 有数据 && 任一 `row_filter.is_active()`。

### 2. 列筛选下拉框（行 1161–1186）

- 点击选项 → 更新 `selected_index`，并**自动重置**：清空 `hidden_columns/rows`、`search_keyword`、所有行筛选关键字、统计（行 1171–1183），即切换列即恢复表格。

### 3. 两个 TextEdit 输入框的 Enter 键（行 1196、1238）

- 任意输入框聚焦时按 **Enter** → 触发与「🔍 搜索」按钮**相同**的统一搜索逻辑，并在执行后 `response.surrender_focus()` 失去焦点。
- 这两段逻辑是重复的（列筛选框 + 每个 row_filter 框各一份），实际效果一致。

### 4. 诊断 / 提示信息（只读）

| 信息 | 触发条件 | 颜色 / 字号 | 内容 |
|------|----------|-------------|------|
| 标题栏「匹配 x/y 列」 | `is_searching` | 绿色(0,130,0) / 11 | 列筛选统计 |
| 「(二分)」 | `use_binary_search` | 灰(100,100,100) / 10 | 自动启用二分查找标记 |
| 行筛选诊断 | `is_row_searching && 非空` | 绿(0,100,0) / 10 | 模式标签（二分/并行/串行）+ 命中统计 |
| 列筛选诊断 | `is_searching && 非空` | 灰(100,100,100) / 10 | 选中列、范围、隐藏/匹配数、采样值 |
| 底部提示 | 常驻 | 灰(140,140,140) / 10 | 固定说明文字 |

---

## 六、结构性细节

1. **统一搜索模型**：列筛选与行筛选**完全解耦但共用一个「搜索」入口**——按钮和 Enter 都会先做列筛选（基于列隐藏集），再做行筛选（基于行隐藏集），两者结果在主表格上叠加。
2. **延迟加载**（行 1139）：`options_loaded` 标志保证 `column_options` 和 `row_filters` 仅在窗口首次渲染 / 切表后加载一次，避免每帧重新解析 YAML。
3. **诊断信息双来源**：`debug_info`（列）与 `row_debug_info`（行）来自 `execute_search` / `execute_row_search` 内部生成，UI 只负责按颜色显示。
4. **可优化点**：列筛选 TextEdit 的 Enter 处理（行 1196–1217）与每个 row_filter 的 Enter 处理（行 1238–1259）逻辑**完全相同**，属于重复代码，可抽成一个内部辅助函数（如 `try_run_search(state, excel_data, sheet, hidden_columns, hidden_rows)`）。
