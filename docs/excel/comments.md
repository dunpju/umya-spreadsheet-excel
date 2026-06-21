# 单元格批注（Comment）解析与渲染

> 跨模块专题：解析（`excel::reader`）+ 渲染（`gui::widgets::table`）。本文说明本项目如何读取并展示 Excel 单元格批注，包含 umya-spreadsheet 3.0 的 API、调用示例与注意事项。

## 1. 能力概览

| 能力 | 支持情况 |
|------|----------|
| 读取经典批注（legacy `<comment>`） | ✅ 作者 + 富文本/纯文本 |
| 读取 Office 2019 线程化批注（`<threadedComment>`） | ❌ 本轮未处理 |
| 渲染指示器（右上角红三角） | ✅ Excel 风格 |
| 悬停气泡（作者 + 正文） | ✅ 自动换行 |
| 合并单元格批注 | ✅ 只在合并左上角渲染 |
| 冻结窗格内批注 | ✅ 主网格与冻结区均支持 |
| 编辑/新增批注并写回文件 | ❌ 本轮不做（保存时原始批注自动保留） |

## 2. umya-spreadsheet 3.0 的批注 API

### 2.1 `Worksheet` 级（读入口）

```rust
// 所有经典批注
let comments: &[Comment] = worksheet.comments();

// 按坐标串（如 "A1"）快速查找
let map: HashMap<String, &Comment> = worksheet.comments_to_hashmap();

// 是否存在批注
let has: bool = worksheet.has_comments();

// Office 2019 线程化批注（本未使用）
let threaded: &[ThreadedComment] = worksheet.threaded_comments();
```

### 2.2 `Comment` 结构（字段访问器）

| 方法 | 返回 | 说明 |
|------|------|------|
| `coordinate()` | `&Coordinate` | 坐标，`.col_num()` / `.row_num()` 取 1-based 行列号 |
| `author()` | `&str` | 作者（读取时已由 OOXML `authorId` 解析为名字） |
| `text()` | `&CommentText` | 批注文本（纯文本 + 富文本） |
| `id()` | `&str` | 批注 id |
| `anchor()` / `shape()` | `&Anchor` / `&Shape` | 形状/锚点（位置样式） |

### 2.3 `CommentText` 结构（文本提取）

批注文本可能是纯文本或富文本（作者名通常为首段加粗 run），二者都需处理：

```rust
let mut s = String::new();
if let Some(t) = comment.text().text() {        // 纯文本 <t>
    s.push_str(t.value());
}
if let Some(rt) = comment.text().rich_text() {  // 富文本 <r> run
    for te in rt.rich_text_elements() {
        s.push_str(te.text());
    }
}
```

## 3. 本项目的数据模型与解析

### 3.1 数据结构（`src/excel/reader.rs`）

```rust
/// 单元格批注
pub struct CellComment {
    pub author: String,   // 作者（已由 authorId 解析）
    pub text: String,     // 批注全文（plain + rich 已拼接）
}

pub struct CellData {
    // …其余字段…
    pub comment: Option<CellComment>,  // 新增：无批注时为 None
}
```

> 选择把批注挂在 `CellData` 上（而非 `SheetData` 的独立 `HashMap`），理由：
> - 渲染时 `table.rs` 已按格取 `CellData`，单次查询即可拿到批注；
> - `insert_rows` / `insert_columns` / `append_row` 移动单元格时，批注随单元格自动迁移（与 umya `Comment: AdjustmentCoordinate` 语义一致），无需另写偏移逻辑。

### 3.2 解析流程（`ExcelData::load_from_file`）

在「读取条件格式规则」之后、「`sheets.push(sheet)`」之前，遍历 `worksheet.comments()` 挂载批注：

```rust
for comment in worksheet.comments() {
    let col = comment.coordinate().col_num();
    let row = comment.coordinate().row_num();
    let author = comment.author().to_string();
    let text = extract_comment_text(comment.text());
    // entry().or_insert_with() 兼容「仅有批注、无 <c> 记录」的空单元格
    let cell = sheet.cells.entry((row, col)).or_insert_with(CellData::default);
    cell.comment = Some(CellComment { author, text });
}
```

文本提取助手（私有）：

```rust
fn extract_comment_text(ct: &umya_spreadsheet::structs::CommentText) -> String {
    let mut s = String::new();
    if let Some(t) = ct.text() { s.push_str(t.value()); }
    if let Some(rt) = ct.rich_text() {
        for te in rt.rich_text_elements() { s.push_str(te.text()); }
    }
    s
}
```

## 4. 渲染（`src/gui/widgets/table.rs`）

采用 Excel 风格：**右上角红色三角指示器** + **鼠标悬停弹出气泡**。

### 4.1 指示器（红三角）

模块级私有函数 `draw_comment_indicator(painter, x, y, width)`，用 `egui::Shape::convex_polygon` 在单元格右上角画 ~7px 红色实心三角。调用点：

- **主网格非冻结区**：在「第二遍内容绘制」之后单独遍历可见单元格，合并非左上角跳过（只在合并左上角画三角）。
- **冻结区**：在 `draw_frozen_cell` 闭包末尾调用，覆盖冻结列/行内的带批注单元格。

### 4.2 悬停气泡

在所有单元格绘制完成后（编辑框之前）一次性指针检测：

1. `response.hovered() && !response.dragged() && editing_cell.is_none() && !validation_error_active`
2. 屏幕坐标 → 单元格：复用与点击相同的冻结区感知坐标转换（`partition_point` 二分查找累积数组）。
3. 命中带批注的单元格（合并单元格自动取左上角）→ 用 `painter.layout_job(LayoutJob::simple(...))` 生成自动换行的 galley，绘制淡黄背景（`#FFFFE0`）+ 边框 + 作者（小号灰）+ 正文（黑色）。
4. 定位在指针右下方，越界自动向左/上翻转并夹紧到视口。

```rust
let author_galley = painter.layout_job(
    egui::text::LayoutJob::simple(comment.author.clone(), egui::FontId::proportional(11.0), gray, 300.0),
);
let body_galley = painter.layout_job(
    egui::text::LayoutJob::simple(comment.text.clone(), egui::FontId::proportional(13.0), black, 300.0),
);
// …计算尺寸 → painter.rect_filled/rect_stroke → painter.galley(…)
```

## 5. 使用方法

1. 用任意工具（Excel / WPS / LibreOffice）在某 `.xlsx` 单元格上添加批注（含作者与文本）。
2. 运行 GUI 打开该文件：
   ```bash
   cargo run
   ```
3. 有批注的单元格右上角出现红色小三角；鼠标悬停弹出「作者 + 正文」气泡。
4. 冻结窗格内、合并单元格（左上角）的批注同样显示。

## 6. 注意事项 / 边界

- **经典批注无时间戳**：OOXML `<comment>` 仅 `authorId` + 文本，不含日期/时间。需要时间戳的「线程化批注」（Office 2019/365）本轮未支持。
- **编辑不写回**：`writer.rs` 保存时重读原始文件，**原始批注在 round-trip 中自动保留**；但用户在 UI 中新增/修改批注不会写入新文件（超出本轮范围）。如需写回，需在 `writer.rs` 把 `CellData.comment` 重建为 `umya_spreadsheet::Comment` 并 `set_comments` / `add_comments`。
- **行列插入时批注跟随**：批注存在 `CellData` 上，插入行/列时随单元格整体移动（内存模型正确）。但保存时原始文件坐标不变（受上一条写回限制影响）。
- **隐藏行/列**：批注三角仅在可见单元格绘制，与现有隐藏跳过逻辑一致。
- **合并单元格**：批注只在合并左上角渲染三角与气泡（与 Excel 行为一致）。
- **空批注**：文本为空时气泡正文显示「（空批注）」占位。

## 7. 相关文档

- [`reader.md`](./reader.md) —— `CellData` / `CellComment` / `load_from_file` 数据模型与解析。
- [`../gui/widgets/table.md`](../gui/widgets/table.md) —— `draw_table_content` 渲染逻辑（含批注指示器与悬停气泡章节）。
