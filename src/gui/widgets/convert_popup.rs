//! 转换弹窗组件
//!
//! 负责显示"转换"弹出层，包含文本输入、进度条和开始转换按钮。
//! 解析用户输入的转换规则，将源表单元格按规则映射到目标位置，生成新的 Excel 文件。

use eframe::egui;
use std::path::Path;
use crate::excel::reader::{ExcelData, SheetData, CellRange, col_to_letter};
use crate::excel::formula::letter_to_col;

// ============================================================================
// 状态
// ============================================================================

/// 转换弹窗状态
#[derive(Debug)]
pub struct ConvertPopupState {
    /// 是否显示弹窗
    pub visible: bool,
    /// 多行文本输入框内容
    pub text: String,
    /// 当前进度值（0-100）
    pub progress: f32,
    /// 错误信息
    pub error_message: Option<String>,
    /// 成功信息
    pub success_message: Option<String>,
    /// 是否正在转换
    pub is_converting: bool,
}

impl Default for ConvertPopupState {
    fn default() -> Self {
        Self {
            visible: false,
            text: String::new(),
            progress: 0.0,
            error_message: None,
            success_message: None,
            is_converting: false,
        }
    }
}

// ============================================================================
// 类型定义
// ============================================================================

/// 单元格坐标（1-based）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CellRef {
    col: u32,
    row: u32,
}

impl CellRef {
    fn new(col: u32, row: u32) -> Self {
        Self { col, row }
    }
}

/// 源范围变体
#[derive(Debug, Clone)]
enum SourceRange {
    /// 连续范围：A2:M2 或 A3:A12
    Continuous { start: CellRef, end: CellRef },
    /// 合并单元格对范围：(N1:O1):(BV1:BW1)
    MergedPair {
        start_pair: CellRange,
        end_pair: CellRange,
    },
    /// 带步长范围：(N3+2):BV3
    Stepped {
        start: CellRef,
        step: u32,
        end: CellRef,
    },
    /// 批量列范围：(A:M)3:(A:M)12 — 每列同行号范围
    BatchColumns {
        col_start: u32,
        col_end: u32,
        row_start: u32,
        row_end: u32,
    },
}

/// 目标位置变体
#[derive(Debug, Clone)]
enum TargetPosition {
    /// 简单单元格：A1
    Simple(CellRef),
    /// 合并单元格目标：(B1:C1):
    Merged(CellRange),
    /// 批量目标：B(1:13):C(1:13):
    BatchTarget {
        /// 数值填入列
        value_col: u32,
        /// 合并列（与 value_col 配对合并）
        merge_col: u32,
        /// 目标起始行
        row_start: u32,
        /// 目标结束行
        row_end: u32,
    },
}

/// 填充方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    /// 纵向填充（向下）
    Vertical,
    /// 横向填充（向右）
    Horizontal,
}

/// 解析后的单条规则
#[derive(Debug, Clone)]
struct ParsedRule {
    source_range: SourceRange,
    target_start: TargetPosition,
    direction: Direction,
    line: usize,
}

/// 目标位置项（含合并信息）
struct TargetItem {
    cell: CellRef,
    /// 合并列数（>1 表示需要合并的列数）
    merge_cols: u32,
    /// 合并行数（>1 表示需要合并的行数）
    merge_rows: u32,
}

// ============================================================================
// 规则解析器
// ============================================================================

/// 基于字符指针的规则扫描器
struct RuleScanner {
    text: Vec<char>,
    pos: usize,
    line: usize,
}

impl RuleScanner {
    fn new(text: &str) -> Self {
        Self {
            text: text.chars().collect(),
            pos: 0,
            line: 1,
        }
    }

    fn peek(&self) -> Option<char> {
        self.text.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.text.get(self.pos).copied();
        if let Some(c) = ch {
            if c == '\n' {
                self.line += 1;
            }
            self.pos += 1;
        }
        ch
    }

    fn skip_ws(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == ' ' || ch == '\t' || ch == '\r' {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn expect_char(&mut self, expected: char) -> Result<(), String> {
        self.skip_ws();
        match self.peek() {
            Some(c) if c == expected => {
                self.advance();
                Ok(())
            }
            Some(c) => Err(format!(
                "第{}行: 期望 '{}', 但找到 '{}'",
                self.line, expected, c
            )),
            None => Err(format!(
                "第{}行: 期望 '{}', 但已到文本末尾",
                self.line, expected
            )),
        }
    }

    fn expect_str(&mut self, expected: &str) -> Result<(), String> {
        for ch in expected.chars() {
            self.expect_char(ch)?;
        }
        Ok(())
    }

    /// 解析列字母部分，返回列号（1-based）
    fn parse_col_letters(&mut self) -> Result<String, String> {
        let mut col_str = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphabetic() {
                col_str.push(ch.to_ascii_uppercase());
                self.advance();
            } else {
                break;
            }
        }
        if col_str.is_empty() {
            return Err(format!("第{}行: 期望列字母", self.line));
        }
        Ok(col_str)
    }

    /// 解析行号数字
    fn parse_row_digits(&mut self) -> Result<u32, String> {
        let mut digits = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                digits.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        if digits.is_empty() {
            return Err(format!("第{}行: 期望行号数字", self.line));
        }
        digits
            .parse::<u32>()
            .map_err(|_| format!("第{}行: 无效行号 '{}'", self.line, digits))
    }

    /// 解析单元格引用，如 A1、$B$2
    fn parse_cell_ref(&mut self) -> Result<CellRef, String> {
        // 跳过 $
        if self.peek() == Some('$') {
            self.advance();
        }
        let col_str = self.parse_col_letters()?;
        let col = letter_to_col(&col_str)
            .map_err(|e| format!("第{}行: {}", self.line, e))?;

        // 跳过行号前的 $
        if self.peek() == Some('$') {
            self.advance();
        }
        let row = self.parse_row_digits()?;

        Ok(CellRef { col, row })
    }

    /// 解析简单范围：cell:cell
    fn parse_cell_range(&mut self) -> Result<CellRange, String> {
        let start_cell = self.parse_cell_ref()?;
        self.expect_char(':')?;
        let end_cell = self.parse_cell_ref()?;
        Ok(CellRange::new(
            start_cell.row, start_cell.col,
            end_cell.row, end_cell.col,
        ))
    }

    /// 解析源范围
    fn parse_source_range(&mut self) -> Result<SourceRange, String> {
        self.skip_ws();
        match self.peek() {
            Some('(') => {
                // 保存位置，依次尝试 BatchColumns → MergedPair → Stepped
                let saved_pos = self.pos;
                let saved_line = self.line;

                // 1) BatchColumns: (A:M)3:(A:M)12
                if let Ok(result) = self.try_parse_batch_columns() {
                    return Ok(result);
                }
                self.pos = saved_pos;
                self.line = saved_line;

                // 2) MergedPair: (cell:cell):(cell:cell)
                if let Ok(result) = self.try_parse_merged_pair() {
                    return Ok(result);
                }
                self.pos = saved_pos;
                self.line = saved_line;

                // 3) Stepped: (cell+step):cell
                self.try_parse_stepped()
            }
            Some(ch) if ch.is_ascii_alphabetic() => {
                // Continuous: cell:cell
                let start = self.parse_cell_ref()?;
                self.expect_char(':')?;
                let end = self.parse_cell_ref()?;
                Ok(SourceRange::Continuous { start, end })
            }
            Some(ch) => Err(format!(
                "第{}行: 期望单元格引用或 '(', 但找到 '{}'",
                self.line, ch
            )),
            None => Err(format!("第{}行: 期望源范围，但已到文本末尾", self.line)),
        }
    }

    fn try_parse_merged_pair(&mut self) -> Result<SourceRange, String> {
        self.expect_char('(')?;
        let start_pair = self.parse_cell_range()?;
        self.expect_char(')')?;
        self.expect_char(':')?;
        self.expect_char('(')?;
        let end_pair = self.parse_cell_range()?;
        self.expect_char(')')?;

        Ok(SourceRange::MergedPair {
            start_pair,
            end_pair,
        })
    }

    fn try_parse_stepped(&mut self) -> Result<SourceRange, String> {
        self.expect_char('(')?;
        let start = self.parse_cell_ref()?;
        self.expect_char('+')?;
        let step = self.parse_row_digits()?; // step is a number
        self.expect_char(')')?;
        self.expect_char(':')?;
        let end = self.parse_cell_ref()?;

        if step == 0 {
            return Err(format!("第{}行: 步长不能为 0", self.line));
        }

        Ok(SourceRange::Stepped { start, step, end })
    }

    /// 尝试解析批量列范围：(A:M)3:(A:M)12
    fn try_parse_batch_columns(&mut self) -> Result<SourceRange, String> {
        self.expect_char('(')?;
        let col_start_str = self.parse_col_letters()?;
        self.expect_char(':')?;
        let col_end_str = self.parse_col_letters()?;
        self.expect_char(')')?;
        let row_start = self.parse_row_digits()?;
        self.expect_char(':')?;
        // 第二部分：(A:M)12
        self.expect_char('(')?;
        let col_start2_str = self.parse_col_letters()?;
        self.expect_char(':')?;
        let col_end2_str = self.parse_col_letters()?;
        self.expect_char(')')?;
        let row_end = self.parse_row_digits()?;

        // 校验起止列一致
        if col_start_str != col_start2_str || col_end_str != col_end2_str {
            return Err(format!(
                "第{}行: 批量列范围起止列必须一致",
                self.line
            ));
        }
        let col_start = letter_to_col(&col_start_str)
            .map_err(|e| format!("第{}行: {}", self.line, e))?;
        let col_end = letter_to_col(&col_end_str)
            .map_err(|e| format!("第{}行: {}", self.line, e))?;

        Ok(SourceRange::BatchColumns { col_start, col_end, row_start, row_end })
    }

    /// 解析目标位置
    fn parse_target_start(&mut self) -> Result<TargetPosition, String> {
        self.skip_ws();
        let saved_pos = self.pos;
        let saved_line = self.line;

        // 1) Try Merged target: (cell:cell)
        if self.peek() == Some('(') {
            if let Ok(result) = self.try_parse_merged_target() {
                return Ok(result);
            }
            self.pos = saved_pos;
            self.line = saved_line;
        }

        // 2) Try Batch target: B(1:13):C(1:13):
        //    与 Simple 的区分：列字母后是 '(' 而非数字
        if let Ok(result) = self.try_parse_batch_target() {
            return Ok(result);
        }
        self.pos = saved_pos;
        self.line = saved_line;

        // 3) Simple target: A1
        let cell = self.parse_cell_ref()?;
        Ok(TargetPosition::Simple(cell))
    }

    fn try_parse_merged_target(&mut self) -> Result<TargetPosition, String> {
        self.expect_char('(')?;
        let range = self.parse_cell_range()?;
        self.expect_char(')')?;
        // 不消费末尾的 ':' — 该冒号由 parse_one_rule 统一作为规则分隔符处理

        Ok(TargetPosition::Merged(range))
    }

    /// 尝试解析批量目标，支持两种格式：
    /// - `B(1:13):C(1:13):`  （无外层括号）
    /// - `(B(1:13):C(1:13)):`（有外层括号）
    /// 末尾 ':' 由 parse_one_rule 统一消费
    fn try_parse_batch_target(&mut self) -> Result<TargetPosition, String> {
        let saved_pos = self.pos;
        let saved_line = self.line;

        // 检测可选外层 '('
        let has_outer_paren = self.peek() == Some('(');
        if has_outer_paren {
            // 跳过外层 '('，但不消费——由 expect_char 处理
            // 直接重新解析
            self.pos = saved_pos;
            self.line = saved_line;
            return self.try_parse_batch_target_inner();
        }

        // 无外层括号：列字母开头
        let value_col_str = self.parse_col_letters()?;
        if self.peek() != Some('(') {
            self.pos = saved_pos;
            self.line = saved_line;
            return Err("不是批量目标".to_string());
        }
        self.parse_batch_target_body(value_col_str)
    }

    /// 解析外层带括号的批量目标：(B(1:13):C(1:13)):
    fn try_parse_batch_target_inner(&mut self) -> Result<TargetPosition, String> {
        let saved_pos = self.pos;
        let saved_line = self.line;

        // 可能已有外层 '(' 或需要从字母开始
        if self.peek() == Some('(') {
            self.advance();
            self.skip_ws();
        }

        let value_col_str = self.parse_col_letters()?;
        if self.peek() != Some('(') {
            self.pos = saved_pos;
            self.line = saved_line;
            return Err("不是批量目标".to_string());
        }
        let result = self.parse_batch_target_body(value_col_str)?;

        // 如果有外层括号，消费 ')'
        self.skip_ws();
        if self.peek() == Some(')') {
            self.advance();
        }
        // 末尾 ':' 由 parse_one_rule 统一消费

        Ok(result)
    }

    /// 解析批量目标的核心部分（从组行号到合并列行号）
    fn parse_batch_target_body(&mut self, value_col_str: String) -> Result<TargetPosition, String> {
        let value_col = letter_to_col(&value_col_str)
            .map_err(|e| format!("第{}行: {}", self.line, e))?;

        self.expect_char('(')?;
        let row_start = self.parse_row_digits()?;
        self.expect_char(':')?;
        let row_end = self.parse_row_digits()?;
        self.expect_char(')')?;
        self.expect_char(':')?;
        let merge_col_str = self.parse_col_letters()?;
        self.expect_char('(')?;
        let row_start2 = self.parse_row_digits()?;
        self.expect_char(':')?;
        let row_end2 = self.parse_row_digits()?;
        self.expect_char(')')?;
        // 不消费末尾 ':' — 该冒号由 parse_one_rule 统一消费

        if row_start != row_start2 || row_end != row_end2 {
            return Err(format!("第{}行: 批量目标起止行必须一致", self.line));
        }

        let merge_col = letter_to_col(&merge_col_str)
            .map_err(|e| format!("第{}行: {}", self.line, e))?;

        Ok(TargetPosition::BatchTarget { value_col, merge_col, row_start, row_end })
    }

    /// 解析方向标记
    fn parse_direction(&mut self) -> Result<Direction, String> {
        self.skip_ws();
        match self.peek() {
            Some('|') => {
                self.advance();
                Ok(Direction::Vertical)
            }
            Some('-') => {
                self.advance();
                Ok(Direction::Horizontal)
            }
            Some(c) => Err(format!(
                "第{}行: 期望方向标记 '|' 或 '-', 但找到 '{}'",
                self.line, c
            )),
            None => Err(format!("第{}行: 期望方向标记，但已到文本末尾", self.line)),
        }
    }

    /// 解析单条规则
    fn parse_one_rule(&mut self) -> Result<ParsedRule, String> {
        let line = self.line;
        let source_range = self.parse_source_range()?;

        self.skip_ws();
        self.expect_str("->")?;

        let target_start = self.parse_target_start()?;

        self.expect_char(':')?;
        let direction = self.parse_direction()?;

        self.expect_char('~')?;
        self.expect_char(';')?;

        Ok(ParsedRule {
            source_range,
            target_start,
            direction,
            line,
        })
    }
}

/// 解析所有规则，返回解析结果
fn parse_rules(text: &str) -> Result<Vec<ParsedRule>, String> {
    let mut scanner = RuleScanner::new(text);
    let mut rules = Vec::new();

    loop {
        scanner.skip_ws();
        // 跳过空行
        while scanner.peek() == Some('\n') {
            scanner.advance();
            scanner.skip_ws();
        }
        if scanner.peek().is_none() {
            break;
        }
        rules.push(scanner.parse_one_rule()?);
    }

    if rules.is_empty() {
        return Err("至少需要一条有效规则".to_string());
    }
    Ok(rules)
}

// ============================================================================
// 源范围解析
// ============================================================================

/// 将源范围展开为源单元格列表
fn resolve_source_items(
    range: &SourceRange,
    _sheet: &SheetData,
) -> Result<Vec<CellRef>, String> {
    match range {
        SourceRange::Continuous { start, end } => {
            let mut items = Vec::new();
            let col_range = if start.col <= end.col {
                start.col..=end.col
            } else {
                end.col..=start.col
            };
            let row_range = if start.row <= end.row {
                start.row..=end.row
            } else {
                end.row..=start.row
            };

            if start.row == end.row {
                // 行优先（横向）
                for col in col_range {
                    items.push(CellRef::new(col, start.row));
                }
            } else if start.col == end.col {
                // 列优先（纵向）
                for row in row_range {
                    items.push(CellRef::new(start.col, row));
                }
            } else {
                // 矩形：先行后列
                for row in row_range {
                    for col in col_range.clone() {
                        items.push(CellRef::new(col, row));
                    }
                }
            }
            Ok(items)
        }

        SourceRange::MergedPair {
            start_pair,
            end_pair,
        } => {
            // 每个合并对的宽高
            let pair_width = start_pair.end_col.max(start_pair.start_col)
                - start_pair.end_col.min(start_pair.start_col)
                + 1;
            let pair_height = start_pair.end_row.max(start_pair.start_row)
                - start_pair.end_row.min(start_pair.start_row)
                + 1;

            // 判断遍历方向
            let is_horizontal = end_pair.start_col != start_pair.start_col;
            let forward = if is_horizontal {
                end_pair.start_col > start_pair.start_col
            } else {
                end_pair.start_row > start_pair.start_row
            };

            let mut items = Vec::new();
            let mut col = start_pair.start_col;
            let mut row = start_pair.start_row;

            loop {
                items.push(CellRef::new(col, row));

                // 检查是否已到达终点
                let done = if is_horizontal {
                    if forward { col >= end_pair.start_col } else { col <= end_pair.start_col }
                } else {
                    if forward { row >= end_pair.start_row } else { row <= end_pair.start_row }
                };
                if done {
                    break;
                }

                // 步进
                if is_horizontal {
                    col = if forward { col + pair_width } else { col - pair_width };
                } else {
                    row = if forward { row + pair_height } else { row - pair_height };
                }

                if items.len() > 10000 {
                    return Err("合并单元格范围过大（超过10000项）".to_string());
                }
            }
            Ok(items)
        }

        SourceRange::Stepped { start, step, end } => {
            let mut items = Vec::new();

            if start.row == end.row {
                // 横向步进（列方向）
                let col_range: Vec<u32> = if start.col <= end.col {
                    (start.col..=end.col).step_by(*step as usize).collect()
                } else {
                    (end.col..=start.col)
                        .rev()
                        .step_by(*step as usize)
                        .collect()
                };
                for col in col_range {
                    items.push(CellRef::new(col, start.row));
                }
            } else if start.col == end.col {
                // 纵向步进（行方向）
                let row_range: Vec<u32> = if start.row <= end.row {
                    (start.row..=end.row).step_by(*step as usize).collect()
                } else {
                    (end.row..=start.row)
                        .rev()
                        .step_by(*step as usize)
                        .collect()
                };
                for row in row_range {
                    items.push(CellRef::new(start.col, row));
                }
            } else {
                return Err(format!(
                    "步进范围仅支持单行或单列"
                ));
            }
            Ok(items)
        }

        SourceRange::BatchColumns { .. } => {
            Err("批量列规则应在展开阶段处理".to_string())
        }
    }
}

// ============================================================================
// 目标位置计算
// ============================================================================

/// 根据源项列表和目标起始位置计算目标位置列表
fn compute_targets(
    src_items: &[CellRef],
    target: &TargetPosition,
    dir: Direction,
) -> Vec<TargetItem> {
    let (start_col, start_row, merge_cols, merge_rows) = match target {
        TargetPosition::Simple(r) => (r.col, r.row, 1, 1),
        TargetPosition::Merged(range) => {
            let w = if range.end_col >= range.start_col {
                range.end_col - range.start_col + 1
            } else {
                1
            };
            let h = if range.end_row >= range.start_row {
                range.end_row - range.start_row + 1
            } else {
                1
            };
            (range.start_col, range.start_row, w, h)
        }
        TargetPosition::BatchTarget { .. } => {
            (1, 1, 1, 1) // 批量目标应在展开阶段处理，此处为兜底
        }
    };

    let mut targets = Vec::with_capacity(src_items.len());
    let mut col = start_col;
    let mut row = start_row;

    for _ in src_items {
        targets.push(TargetItem {
            cell: CellRef::new(col, row),
            merge_cols,
            merge_rows,
        });

        match dir {
            Direction::Horizontal => {
                col += merge_cols;
            }
            Direction::Vertical => {
                row += merge_rows;
            }
        }
    }

    targets
}

// ============================================================================
// 公式偏移调整
// ============================================================================

// adjust_formula_by_offset 已移至 crate::excel::formula 模块

// ============================================================================
// 单元格深拷贝
// ============================================================================

/// 深拷贝单个单元格（值、公式、样式）从源位置到目标位置。
/// 从项目自身的 `SheetData` 读取（RGB 颜色已预解析），写入 umya Worksheet，
/// 样式照搬 `writer.rs` 的写入模式。
fn deep_copy_cell(
    src_sheet: &SheetData,
    tgt_ws: &mut umya_spreadsheet::Worksheet,
    src_col: u32,
    src_row: u32,
    tgt_col: u32,
    tgt_row: u32,
    col_offset: i32,
    row_offset: i32,
) {
    // 从项目层的 CellData 读取（样式已解析为 RGB，无主题依赖）
    let cell_data = match src_sheet.get_cell(src_row, src_col) {
        Some(c) => c,
        None => return,
    };

    // --- 值 & 公式 ---
    let tgt_cell = tgt_ws.cell_mut((tgt_col, tgt_row));

    if !cell_data.formula.is_empty() {
        let adjusted =
            crate::excel::formula::adjust_formula_by_offset(&cell_data.formula, col_offset, row_offset);
        tgt_cell.set_formula(adjusted.as_str());
    }

    if !cell_data.value.is_empty() {
        tgt_cell.set_value(cell_data.value.as_str());
    }

    // --- 样式（完全参照 writer.rs 模式） ---
    let tgt_style = tgt_ws.style_mut((tgt_col, tgt_row));

    // 对齐
    {
        let align = tgt_style.alignment_mut();
        align.set_horizontal(match cell_data.alignment.horizontal {
            crate::excel::reader::HorizontalAlignment::General =>
                umya_spreadsheet::HorizontalAlignmentValues::General,
            crate::excel::reader::HorizontalAlignment::Left =>
                umya_spreadsheet::HorizontalAlignmentValues::Left,
            crate::excel::reader::HorizontalAlignment::Center =>
                umya_spreadsheet::HorizontalAlignmentValues::Center,
            crate::excel::reader::HorizontalAlignment::Right =>
                umya_spreadsheet::HorizontalAlignmentValues::Right,
            crate::excel::reader::HorizontalAlignment::Fill =>
                umya_spreadsheet::HorizontalAlignmentValues::Fill,
            crate::excel::reader::HorizontalAlignment::Justify =>
                umya_spreadsheet::HorizontalAlignmentValues::Justify,
            crate::excel::reader::HorizontalAlignment::CenterContinuous =>
                umya_spreadsheet::HorizontalAlignmentValues::CenterContinuous,
            crate::excel::reader::HorizontalAlignment::Distributed =>
                umya_spreadsheet::HorizontalAlignmentValues::Distributed,
        });
        align.set_vertical(match cell_data.alignment.vertical {
            crate::excel::reader::VerticalAlignment::Top =>
                umya_spreadsheet::VerticalAlignmentValues::Top,
            crate::excel::reader::VerticalAlignment::Center =>
                umya_spreadsheet::VerticalAlignmentValues::Center,
            crate::excel::reader::VerticalAlignment::Bottom =>
                umya_spreadsheet::VerticalAlignmentValues::Bottom,
            crate::excel::reader::VerticalAlignment::Justify =>
                umya_spreadsheet::VerticalAlignmentValues::Justify,
            crate::excel::reader::VerticalAlignment::Distributed =>
                umya_spreadsheet::VerticalAlignmentValues::Distributed,
        });
    }

    // 背景颜色（RRGGBB 格式）
    if let Some((r, g, b)) = cell_data.background_color {
        tgt_style.set_background_color(format!("{:02X}{:02X}{:02X}", r, g, b));
    }

    // 字体
    {
        let font = tgt_style.font_mut();
        if let Some(size) = cell_data.font_size {
            font.set_size(size);
        }
        if let Some((r, g, b)) = cell_data.font_color {
            font.color_mut().set_argb_str(format!("FF{:02X}{:02X}{:02X}", r, g, b));
        }
        if cell_data.bold {
            font.set_bold(true);
        }
    }

    // 边框
    {
        let b = tgt_style.borders_mut();
        apply_cellborder(b.left_mut(), &cell_data.borders.left);
        apply_cellborder(b.right_mut(), &cell_data.borders.right);
        apply_cellborder(b.top_mut(), &cell_data.borders.top);
        apply_cellborder(b.bottom_mut(), &cell_data.borders.bottom);
    }

    // 数字格式
    if let Some(ref fmt) = cell_data.number_format {
        tgt_style.number_format_mut().set_format_code(fmt);
    }
}

/// 将项目层的 CellBorder 应用到 umya Border（照搬 writer.rs）
fn apply_cellborder(
    border: &mut umya_spreadsheet::structs::Border,
    cb: &crate::excel::reader::CellBorder,
) {
    if !cb.style.is_empty() {
        border.set_border_style(&cb.style);
    }
    if let Some((r, g, b)) = cb.color {
        let mut color = umya_spreadsheet::structs::Color::default();
        color.set_argb_str(format!("FF{:02X}{:02X}{:02X}", r, g, b));
        border.set_color(color);
    }
}

// ============================================================================
// 文件路径生成
// ============================================================================

/// 生成输出文件路径：{dir}/{stem}-convert-{yyyyMMddHHmmss}.xlsx
fn generate_output_path(file_path: &str) -> Result<String, String> {
    let path = Path::new(file_path);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "无效的文件名".to_string())?;
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("xlsx");
    let dir = path
        .parent()
        .ok_or_else(|| "无效的文件路径".to_string())?;

    let ts = format_utc_timestamp();

    let mut filename = format!("{}-convert-{}.{}", stem, ts, ext);
    let mut cnt = 1;
    let mut output = dir.join(&filename);
    while output.exists() {
        filename = format!("{}-convert-{}_{}.{}", stem, ts, cnt, ext);
        output = dir.join(&filename);
        cnt += 1;
    }

    Ok(output.to_string_lossy().to_string())
}

/// 生成 yyyyMMddHHmmss 格式的 UTC 时间戳
fn format_utc_timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    // Howard Hinnant 算法：Unix 时间戳 → UTC 日期
    let days = secs / 86400;
    let sod = secs % 86400;

    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    if m <= 2 {
        y += 1;
    }

    format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}",
        y,
        m,
        d,
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60,
    )
}

// ============================================================================
// 批量规则展开
// ============================================================================

/// 将批量规则展开为等价的逐列规则列表。
/// 非批量规则直接返回原规则的单元素 Vec。
fn expand_batch_rule(rule: &ParsedRule) -> Vec<ParsedRule> {
    match (&rule.source_range, &rule.target_start) {
        (
            SourceRange::BatchColumns { col_start, col_end, row_start, row_end },
            TargetPosition::BatchTarget { value_col, merge_col, row_start: tgt_row_start, row_end: _tgt_row_end },
        ) => {
            let _merge_width = merge_col - value_col + 1;
            let mut expanded = Vec::new();

            for (i, col) in (*col_start..=*col_end).enumerate() {
                let src_start = CellRef::new(col, *row_start);
                let src_end = CellRef::new(col, *row_end);
                let tgt_row = tgt_row_start + i as u32;
                let tgt_range = CellRange::new(
                    tgt_row, *value_col,
                    tgt_row, *merge_col,
                );

                expanded.push(ParsedRule {
                    source_range: SourceRange::Continuous { start: src_start, end: src_end },
                    target_start: TargetPosition::Merged(tgt_range),
                    direction: rule.direction,
                    line: rule.line,
                });
            }
            expanded
        }
        _ => vec![rule.clone()],
    }
}

// ============================================================================
// 转换执行器
// ============================================================================

/// 执行转换
fn execute_convert(
    state: &mut ConvertPopupState,
    excel_data: &ExcelData,
    file_path: &str,
    current_sheet: usize,
) -> Result<(), String> {
    // 1. 解析规则
    let rules = parse_rules(&state.text)?;

    // 1.5 展开批量规则 → 等价逐列规则
    let rules: Vec<ParsedRule> = rules
        .iter()
        .flat_map(|r| expand_batch_rule(r))
        .collect();

    // 2. 获取当前工作表
    let sheet = excel_data
        .get_sheet(current_sheet)
        .ok_or_else(|| "当前工作表无效".to_string())?;

    // 3. 预计算所有映射
    struct RulePlan {
        mappings: Vec<(CellRef, TargetItem)>,
    }

    let mut plans: Vec<RulePlan> = Vec::new();
    let mut total_items = 0usize;

    for rule in &rules {
        let src_items = resolve_source_items(&rule.source_range, sheet)?;
        let targets = compute_targets(&src_items, &rule.target_start, rule.direction);

        if src_items.len() != targets.len() {
            return Err(format!(
                "第{}行: 源项数({})与目标项数({})不匹配",
                rule.line,
                src_items.len(),
                targets.len(),
            ));
        }

        let mappings: Vec<(CellRef, TargetItem)> = src_items
            .into_iter()
            .zip(targets.into_iter())
            .collect();
        total_items += mappings.len();
        plans.push(RulePlan { mappings });
    }

    if total_items == 0 {
        return Err("没有需要复制的单元格".to_string());
    }

    // 4. 生成输出路径
    let output_path = generate_output_path(file_path)?;

    // 5. 创建全新的输出 Workbook，只写入规则指定的单元格
    let mut out_book = umya_spreadsheet::new_file();
    let sheet_name = sheet.name.clone();
    if let Ok(ws) = out_book.sheet_mut(0) {
        ws.set_name(&sheet_name);
    }

    let out_ws = out_book.sheet_mut(0)
        .map_err(|e| format!("无法获取输出工作表: {}", e))?;

    // 6. 处理每条规则的映射：从项目层 SheetData 读取（RGB 已解析），写入输出 Workbook
    let mut processed = 0usize;

    for plan in &plans {
        for (src, tgt) in &plan.mappings {
            let col_offset = tgt.cell.col as i32 - src.col as i32;
            let row_offset = tgt.cell.row as i32 - src.row as i32;

            deep_copy_cell(
                sheet, out_ws,
                src.col, src.row,
                tgt.cell.col, tgt.cell.row,
                col_offset, row_offset,
            );

            // 如果目标是合并单元格，添加合并
            if tgt.merge_cols > 1 || tgt.merge_rows > 1 {
                let end_col = tgt.cell.col + tgt.merge_cols - 1;
                let end_row = tgt.cell.row + tgt.merge_rows - 1;
                let range_str = format!(
                    "{}{}:{}{}",
                    col_to_letter(tgt.cell.col),
                    tgt.cell.row,
                    col_to_letter(end_col),
                    end_row,
                );
                out_ws.add_merge_cells(&range_str);
            }

            processed += 1;
            state.progress = (processed as f32 / total_items as f32) * 100.0;
        }
    }

    // 8. 写入输出文件
    umya_spreadsheet::writer::xlsx::write(&out_book, Path::new(&output_path))
        .map_err(|e| format!("写入文件失败: {}", e))?;

    Ok(())
}

// ============================================================================
// 绘制函数
// ============================================================================

/// 绘制转换弹窗
///
/// # 参数
/// * `ctx` - egui 上下文
/// * `state` - 转换弹窗状态的可变引用
/// * `excel_data` - 当前加载的 Excel 数据
/// * `file_path` - 当前文件路径
/// * `current_sheet` - 当前活动工作表索引
pub fn draw_convert_popup(
    ctx: &egui::Context,
    state: &mut ConvertPopupState,
    excel_data: Option<&ExcelData>,
    file_path: Option<&str>,
    current_sheet: usize,
) {
    if !state.visible {
        return;
    }

    let mut keep_open = true;

    egui::Window::new("转换工具")
        .id(egui::Id::new("convert_popup"))
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .open(&mut keep_open)
        .show(ctx, |ui| {
            ui.set_min_width(440.0);
            ui.set_max_width(440.0);

            // ══════ 自定义标题栏 ══════
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("转换工具").size(13.0).strong());
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

            // 中间区域：多行文本输入框
            let text_changed = egui::ScrollArea::vertical()
                .max_height(124.0)
                .max_width(ui.available_width())
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let response = ui.add(
                        egui::TextEdit::multiline(&mut state.text)
                            .hint_text("请输入转换规则...")
                            .desired_width(f32::INFINITY)
                            .desired_rows(7),
                    );
                    response.changed()
                })
                .inner;

            // 文本变化时清除状态消息
            if text_changed {
                state.error_message = None;
                state.success_message = None;
            }

            ui.separator();

            // 底部行：进度条 + 开始转换按钮
            ui.horizontal(|ui| {
                ui.add(
                    egui::ProgressBar::new(state.progress / 100.0)
                        .desired_width(370.0)
                        .text(format!("{:.0}%", state.progress)),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // 判断按钮是否可用：实时解析规则并显示错误
                    let parse_result = if state.text.trim().is_empty() {
                        None
                    } else {
                        Some(parse_rules(&state.text))
                    };

                    let has_valid_rules = parse_result
                        .as_ref()
                        .is_some_and(|r| r.is_ok());

                    // 若解析失败，实时显示错误信息
                    if let Some(Err(ref e)) = parse_result {
                        state.error_message = Some(e.clone());
                    } else if has_valid_rules {
                        // 规则有效时清除之前的解析错误
                        if state.error_message.as_ref().is_some_and(|m| m.contains("行")) {
                            state.error_message = None;
                        }
                    }

                    let can_convert = !state.is_converting
                        && excel_data.is_some()
                        && file_path.is_some()
                        && has_valid_rules;

                    let convert_btn =
                        ui.add_enabled(can_convert, egui::Button::new("开始转换"));

                    if convert_btn.clicked() {
                        state.error_message = None;
                        state.success_message = None;
                        state.progress = 0.0;

                        if let (Some(data), Some(path)) = (excel_data, file_path) {
                            match execute_convert(state, data, path, current_sheet) {
                                Ok(()) => {
                                    state.progress = 100.0;
                                    state.success_message =
                                        Some("转换完成！文件已生成在同目录下".to_string());
                                }
                                Err(e) => {
                                    state.error_message = Some(e);
                                }
                            }
                        }
                    }
                });
            });

            // 状态消息
            if let Some(ref err) = state.error_message {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(err)
                        .color(egui::Color32::RED)
                        .size(11.0),
                );
            }
            if let Some(ref msg) = state.success_message {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(msg)
                        .color(egui::Color32::GREEN)
                        .size(11.0),
                );
            }
        });

    if !keep_open {
        state.visible = false;
    }
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rule1_simple_continuous() {
        let text = "A2:M2->A1:|~;";
        let rules = parse_rules(text).unwrap();
        assert_eq!(rules.len(), 1);
        let r = &rules[0];
        assert!(matches!(r.source_range, SourceRange::Continuous { .. }));
        assert!(matches!(r.target_start, TargetPosition::Simple(_)));
        assert_eq!(r.direction, Direction::Vertical);
    }

    #[test]
    fn test_parse_rule2_merged_pair() {
        let text = "(N1:O1):(BV1:BW1)->A15:|~;";
        let rules = parse_rules(text).unwrap();
        assert_eq!(rules.len(), 1);
        let r = &rules[0];
        assert!(matches!(r.source_range, SourceRange::MergedPair { .. }));
        assert_eq!(r.direction, Direction::Vertical);
    }

    #[test]
    fn test_parse_rule3_merged_target() {
        let text = "A3:A12->(B1:C1):-~;";
        let rules = parse_rules(text).unwrap();
        assert_eq!(rules.len(), 1);
        let r = &rules[0];
        assert!(matches!(r.source_range, SourceRange::Continuous { .. }));
        assert!(matches!(r.target_start, TargetPosition::Merged(_)));
        assert_eq!(r.direction, Direction::Horizontal);
    }

    #[test]
    fn test_parse_rule4_horizontal() {
        let text = "N1:BW1->B14:-~;";
        let rules = parse_rules(text).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].direction, Direction::Horizontal);
    }

    #[test]
    fn test_parse_rule5_stepped() {
        let text = "(N3+2):BV3->B15:|~;";
        let rules = parse_rules(text).unwrap();
        assert_eq!(rules.len(), 1);
        let r = &rules[0];
        assert!(matches!(r.source_range, SourceRange::Stepped { .. }));
        assert_eq!(r.direction, Direction::Vertical);
    }

    #[test]
    fn test_parse_all_five_rules_multiline() {
        let text = "A2:M2->A1:|~;
(N1:O1):(BV1:BW1)->A15:|~;
A3:A12->(B1:C1):-~;
N1:BW1->B14:-~;
(N3+2):BV3->B15:|~;";
        let rules = parse_rules(text).unwrap();
        assert_eq!(rules.len(), 5);
    }

    #[test]
    fn test_parse_all_five_rules_compact_no_trailing_newline() {
        let text = "A2:M2->A1:|~;(N1:O1):(BV1:BW1)->A15:|~;A3:A12->(B1:C1):-~;N1:BW1->B14:-~;(N3+2):BV3->B15:|~;";
        let rules = parse_rules(text).unwrap();
        assert_eq!(rules.len(), 5);
    }

    #[test]
    fn test_parse_batch_columns_rule() {
        let text = "(A:M)3:(A:M)12->B(1:13):C(1:13):-~;";
        let rules = parse_rules(text).unwrap();
        assert_eq!(rules.len(), 1);
        let r = &rules[0];
        assert!(matches!(r.source_range, SourceRange::BatchColumns { .. }));
        assert!(matches!(r.target_start, TargetPosition::BatchTarget { .. }));
        assert_eq!(r.direction, Direction::Horizontal);
    }

    #[test]
    fn test_batch_rule_expansion() {
        let text = "(A:M)3:(A:M)12->B(1:13):C(1:13):-~;";
        let rules = parse_rules(text).unwrap();
        let expanded: Vec<ParsedRule> = rules.iter().flat_map(|r| expand_batch_rule(r)).collect();
        // 13 列 (A..M) → 13 条逐列规则
        assert_eq!(expanded.len(), 13);
        // 每条应是 Continuous + Merged + Horizontal
        for (i, r) in expanded.iter().enumerate() {
            assert!(matches!(r.source_range, SourceRange::Continuous { .. }));
            assert!(matches!(r.target_start, TargetPosition::Merged(_)));
            assert_eq!(r.direction, Direction::Horizontal);
            // 验证行号递增
            if let TargetPosition::Merged(ref range) = r.target_start {
                assert_eq!(range.start_row, (i + 1) as u32);
            }
        }
    }

    #[test]
    fn test_parse_batch_rule_compact() {
        let text = "(A:M)3:(A:M)12->B(1:13):C(1:13):-~;";
        let rules = parse_rules(text).unwrap();
        let expanded: Vec<ParsedRule> = rules.iter().flat_map(|r| expand_batch_rule(r)).collect();
        // 应等价的13条逐列规则: A3:A12->(B1:C1):-~; 到 M3:M12->(B13:C13):-~;
        assert_eq!(expanded.len(), 13);
        // 第一条: A3:A12->(B1:C1):-~;
        let first = &expanded[0];
        if let SourceRange::Continuous { start, end } = &first.source_range {
            assert_eq!(start.col, 1); // A
            assert_eq!(start.row, 3);
            assert_eq!(end.col, 1); // A
            assert_eq!(end.row, 12);
        } else {
            panic!("Expected Continuous");
        }
        if let TargetPosition::Merged(range) = &first.target_start {
            assert_eq!(range.start_col, 2); // B
            assert_eq!(range.end_col, 3);   // C
            assert_eq!(range.start_row, 1);
        } else {
            panic!("Expected Merged");
        }
        // 最后一条: M3:M12->(B13:C13):-~;
        let last = &expanded[12];
        if let SourceRange::Continuous { start, end } = &last.source_range {
            assert_eq!(start.col, 13); // M
            assert_eq!(start.row, 3);
            assert_eq!(end.col, 13); // M
            assert_eq!(end.row, 12);
        } else {
            panic!("Expected Continuous");
        }
        if let TargetPosition::Merged(range) = &last.target_start {
            assert_eq!(range.start_col, 2); // B
            assert_eq!(range.end_col, 3);   // C
            assert_eq!(range.start_row, 13);
        } else {
            panic!("Expected Merged");
        }
    }

    #[test]
    fn test_parse_batch_target_outer_parens() {
        // 外层括号格式：(B(1:13):C(1:13)):
        let text = "(A:M)3:(A:M)12->(B(1:13):C(1:13)):-~;";
        let rules = parse_rules(text).unwrap();
        assert_eq!(rules.len(), 1);
        assert!(matches!(rules[0].target_start, TargetPosition::BatchTarget { .. }));
        let expanded: Vec<ParsedRule> = rules.iter().flat_map(|r| expand_batch_rule(r)).collect();
        assert_eq!(expanded.len(), 13);
        if let TargetPosition::Merged(range) = &expanded[0].target_start {
            assert_eq!(range.start_row, 1);
            assert_eq!(range.start_col, 2);
            assert_eq!(range.end_col, 3);
        } else {
            panic!("Expected Merged");
        }
    }

    #[test]
    fn test_parse_user_six_rules() {
        let text = "A2:M2->A1:|~;
(N1:O1):(BV1:BW1)->A15:|~;
(A:M)3:(A:M)12->(B(1:13):C(1:13)):-~;
N2:BW2->B14:-~;
(N3+2):BV3->B15:|~;
(O3+2):BW3->C15:|~;";
        let rules = parse_rules(text).unwrap();
        assert_eq!(rules.len(), 6);
        let expanded: Vec<ParsedRule> = rules.iter().flat_map(|r| expand_batch_rule(r)).collect();
        assert_eq!(expanded.len(), 18);
    }
}
