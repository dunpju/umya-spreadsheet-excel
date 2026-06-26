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
    /// 批量步进范围：(N(3:12)+2):BV(3:12) — 多行 × 列步进
    BatchStepped {
        col_start: u32,  // N
        row_start: u32,  // 3
        row_end: u32,    // 12
        col_step: u32,   // 2
        col_end: u32,    // BV
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
        value_col: u32,
        merge_col: u32,
        row_start: u32,
        row_end: u32,
    },
    /// 步进目标：(B+2)15 — 目标列按步长递增
    SteppedTarget {
        col_start: u32,  // B
        col_step: u32,   // 2
        row_start: u32,  // 15
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

/// 自定义公式定义
#[derive(Debug, Clone)]
struct CustomFormula {
    /// 目标单元格（如 B12）
    target_cell: CellRef,
    /// 公式文本（~ 为动态行尾占位符，如 SUM(B15:B~)）
    raw_text: String,
}

/// 解析后的单条规则
#[derive(Debug, Clone)]
struct ParsedRule {
    source_range: SourceRange,
    target_start: TargetPosition,
    direction: Direction,
    line: usize,
    /// 可选的自定义公式列表
    custom_formulas: Vec<CustomFormula>,
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
                // 保存位置，依次尝试 BatchStepped → BatchColumns → MergedPair → Stepped
                let saved_pos = self.pos;
                let saved_line = self.line;

                // 1) BatchStepped: (N(3:12)+2):BV(3:12)
                if let Ok(result) = self.try_parse_batch_stepped() {
                    return Ok(result);
                }
                self.pos = saved_pos;
                self.line = saved_line;

                // 2) BatchColumns: (A:M)3:(A:M)12
                if let Ok(result) = self.try_parse_batch_columns() {
                    return Ok(result);
                }
                self.pos = saved_pos;
                self.line = saved_line;

                // 3) MergedPair: (cell:cell):(cell:cell)
                if let Ok(result) = self.try_parse_merged_pair() {
                    return Ok(result);
                }
                self.pos = saved_pos;
                self.line = saved_line;

                // 4) Stepped: (cell+step):cell
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

    /// 尝试解析批量步进源范围：(N(3:12)+2):BV(3:12)
    fn try_parse_batch_stepped(&mut self) -> Result<SourceRange, String> {
        self.expect_char('(')?;
        let col_start_str = self.parse_col_letters()?;
        // 关键区分：列字母后是 '('（行范围）→ BatchStepped
        if self.peek() != Some('(') {
            return Err("不是批量步进范围".to_string());
        }
        self.advance(); // 吃掉 '('
        let row_start = self.parse_row_digits()?;
        self.expect_char(':')?;
        let row_end = self.parse_row_digits()?;
        self.expect_char(')')?;
        self.expect_char('+')?;
        let col_step = self.parse_row_digits()?;
        self.expect_char(')')?;
        self.expect_char(':')?;
        let col_end_str = self.parse_col_letters()?;
        self.expect_char('(')?;
        // 验证结束行范围一致
        let row_start2 = self.parse_row_digits()?;
        self.expect_char(':')?;
        let row_end2 = self.parse_row_digits()?;
        self.expect_char(')')?;
        if row_start != row_start2 || row_end != row_end2 {
            return Err(format!("第{}行: 批量步进起止行必须一致", self.line));
        }
        if col_step == 0 {
            return Err(format!("第{}行: 步长不能为 0", self.line));
        }
        let col_start = letter_to_col(&col_start_str)
            .map_err(|e| format!("第{}行: {}", self.line, e))?;
        let col_end = letter_to_col(&col_end_str)
            .map_err(|e| format!("第{}行: {}", self.line, e))?;
        Ok(SourceRange::BatchStepped { col_start, row_start, row_end, col_step, col_end })
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

        // 1.5) Try Stepped target: (B+2)15
        if self.peek() == Some('(') {
            if let Ok(result) = self.try_parse_stepped_target() {
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

    /// 尝试解析步进目标：(B+2)15
    fn try_parse_stepped_target(&mut self) -> Result<TargetPosition, String> {
        let saved_pos = self.pos;
        let saved_line = self.line;

        self.expect_char('(')?;
        let col_str = self.parse_col_letters()?;
        // 区分：列字母后是 '+' → 步进目标；是 ':' 或 ')' → 其他
        if self.peek() != Some('+') {
            self.pos = saved_pos;
            self.line = saved_line;
            return Err("不是步进目标".to_string());
        }
        self.expect_char('+')?;
        let col_step = self.parse_row_digits()?;
        self.expect_char(')')?;
        let row_start = self.parse_row_digits()?;

        if col_step == 0 {
            return Err(format!("第{}行: 目标步长不能为 0", self.line));
        }

        let col_start = letter_to_col(&col_str)
            .map_err(|e| format!("第{}行: {}", self.line, e))?;

        Ok(TargetPosition::SteppedTarget { col_start, col_step, row_start })
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

        // 可选的 ,formula(cell1=expr1,cell2=expr2,...)
        let custom_formulas = self.parse_optional_custom_formulas()?;

        self.expect_char(':')?;
        let direction = self.parse_direction()?;

        self.expect_char('~')?;
        self.expect_char(';')?;

        Ok(ParsedRule {
            source_range,
            target_start,
            direction,
            line,
            custom_formulas,
        })
    }

    /// 解析可选的 ,formula(T1=f1,T2=f2,...) 部分
    fn parse_optional_custom_formulas(&mut self) -> Result<Vec<CustomFormula>, String> {
        self.skip_ws();
        let saved = (self.pos, self.line);

        // 检测 ",formula(" 前缀
        if self.peek() != Some(',') {
            return Ok(Vec::new());
        }
        self.advance();
        self.skip_ws();

        // 尝试匹配 "formula"
        let mut label = String::new();
        let _label_saved = (self.pos, self.line);
        while let Some(ch) = self.peek() {
            if ch.is_ascii_alphabetic() {
                label.push(ch.to_ascii_lowercase());
                self.advance();
            } else {
                break;
            }
        }
        if label != "formula" {
            // 不是 formula，回退整个 ,
            self.pos = saved.0;
            self.line = saved.1;
            return Ok(Vec::new());
        }

        self.skip_ws();
        if self.peek() != Some('(') {
            self.pos = saved.0;
            self.line = saved.1;
            return Ok(Vec::new());
        }
        self.expect_char('(')?;

        let mut formulas = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(')') {
                self.advance();
                break;
            }
            // 解析 T1=expr1
            let cell_saved = (self.pos, self.line);
            let target_cell = match self.parse_cell_ref() {
                Ok(c) => c,
                Err(_) => {
                    self.pos = cell_saved.0;
                    self.line = cell_saved.1;
                    return Err(format!("第{}行: formula 中期望目标单元格引用", self.line));
                }
            };
            self.skip_ws();
            self.expect_char('=')?;
            // 读取公式直到 ',' 或 ')'
            let raw_text = self.parse_formula_expression()?;
            formulas.push(CustomFormula { target_cell, raw_text });

            self.skip_ws();
            if self.peek() == Some(',') {
                self.advance();
            } else if self.peek() != Some(')') {
                return Err(format!("第{}行: formula 中期望 ',' 或 ')'", self.line));
            }
        }
        Ok(formulas)
    }

    /// 读取公式表达式（直到 ',' 或 ')'，忽略嵌套括号）
    fn parse_formula_expression(&mut self) -> Result<String, String> {
        let mut expr = String::new();
        let mut depth = 0u32;
        while let Some(ch) = self.peek() {
            if ch == '(' {
                depth += 1;
                expr.push(ch);
                self.advance();
            } else if ch == ')' {
                if depth == 0 { break; }
                depth -= 1;
                expr.push(ch);
                self.advance();
            } else if ch == ',' && depth == 0 {
                break;
            } else {
                expr.push(ch);
                self.advance();
            }
        }
        if expr.is_empty() {
            return Err(format!("第{}行: formula 表达式为空", self.line));
        }
        Ok(expr)
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
        SourceRange::BatchStepped { .. } => {
            Err("批量步进规则应在展开阶段处理".to_string())
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
        TargetPosition::SteppedTarget { .. } => {
            (1, 1, 1, 1) // 步进目标应在展开阶段处理，此处为兜底
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
    mapping: &std::collections::HashMap<(u32, u32), (u32, u32)>,
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
        let adjusted = crate::excel::formula::adjust_formula_by_mapping(
            &cell_data.formula,
            mapping,
            col_offset,
            row_offset,
        );
        tgt_cell.set_formula(adjusted.as_str());
        // set_formula_result_default 不会清除公式（set_value 会调用 remove_formula）
        if !cell_data.value.is_empty() {
            tgt_cell.set_formula_result_default(cell_data.value.as_str());
        }
    } else if !cell_data.value.is_empty() {
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

    // --- 批注（Comment） ---
    if let Some(ref comment) = cell_data.comment {
        if !comment.text.is_empty() || !comment.author.is_empty() {
            let mut new_comment = umya_spreadsheet::structs::Comment::default();
            new_comment.new_comment((tgt_col, tgt_row));
            new_comment.set_author(comment.author.as_str());
            new_comment.set_text_string(comment.text.as_str());
            tgt_ws.add_comments(new_comment);
        }
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
            TargetPosition::BatchTarget { value_col, merge_col, row_start: tgt_row_start, row_end: tgt_row_end },
        ) => {
            let merge_width = merge_col - value_col + 1;
            let num_cols = (*col_end - *col_start + 1) as usize;
            let num_src_rows = (*row_end - *row_start + 1) as usize;
            let mut expanded = Vec::with_capacity(num_cols);

            for (i, col) in (*col_start..=*col_end).enumerate() {
                let src_start = CellRef::new(col, *row_start);
                let src_end = CellRef::new(col, *row_end);
                let tgt_row = tgt_row_start + i as u32;
                let tgt_range = CellRange::new(
                    tgt_row, *value_col,
                    tgt_row, *merge_col,
                );

                // 公式横向填充：匹配目标行所属子规则，按源数据行数展开
                let expanded_formulas: Vec<CustomFormula> = rule
                    .custom_formulas
                    .iter()
                    .filter(|cf| {
                        cf.target_cell.row == tgt_row
                        || (i == 0 && (cf.target_cell.row < *tgt_row_start || cf.target_cell.row > *tgt_row_end))
                    })
                    .flat_map(|cf| {
                        (0..num_src_rows).map(move |j| {
                            let col_shift = j as i32 * merge_width as i32;
                            let shifted = shift_formula_columns(&cf.raw_text, col_shift);
                            let new_tgt_col = (cf.target_cell.col as i32 + col_shift).max(1) as u32;
                            CustomFormula {
                                target_cell: CellRef::new(new_tgt_col, cf.target_cell.row),
                                raw_text: shifted,
                            }
                        })
                    })
                    .collect();

                expanded.push(ParsedRule {
                    source_range: SourceRange::Continuous { start: src_start, end: src_end },
                    target_start: TargetPosition::Merged(tgt_range),
                    direction: rule.direction,
                    line: rule.line,
                    custom_formulas: expanded_formulas,
                });
            }
            expanded
        }
        (
            SourceRange::BatchStepped { col_start, row_start, row_end, col_step, col_end },
            TargetPosition::SteppedTarget { col_start: tgt_col_start, col_step: tgt_col_step, row_start: tgt_row_start },
        ) => {
            let (col_start, row_start, row_end, col_step, col_end) =
                (*col_start, *row_start, *row_end, *col_step, *col_end);
            let (tgt_col_start, tgt_col_step, tgt_row_start) =
                (*tgt_col_start, *tgt_col_step, *tgt_row_start);
            let num_rows = row_end - row_start + 1;
            let mut expanded = Vec::with_capacity(num_rows as usize);

            for i in 0..num_rows {
                let src_row = row_start + i;
                let tgt_col = (tgt_col_start as i32 + i as i32 * tgt_col_step as i32).max(1) as u32;

                expanded.push(ParsedRule {
                    source_range: SourceRange::Stepped {
                        start: CellRef::new(col_start, src_row),
                        step: col_step,
                        end: CellRef::new(col_end, src_row),
                    },
                    target_start: TargetPosition::Simple(CellRef::new(tgt_col, tgt_row_start)),
                    direction: rule.direction,
                    line: rule.line,
                    custom_formulas: if i == 0 { rule.custom_formulas.clone() } else { vec![] },
                });
            }
            expanded
        }
        _ => vec![rule.clone()],
    }
}

/// 将公式中所有列引用向右平移 col_shift 列。
/// 支持 `~` 行占位符（如 `B~`），保持绝对引用 `$` 不变。
///
/// 例：`SUM(B15:B~)` + col_shift=2 → `SUM(D15:D~)`
fn shift_formula_columns(formula: &str, col_shift: i32) -> String {
    if formula.is_empty() || col_shift == 0 {
        return formula.to_string();
    }
    let chars: Vec<char> = formula.chars().collect();
    let mut result = String::with_capacity(formula.len());
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];

        // 字符串字面量：原样输出
        if ch == '"' {
            result.push(ch);
            i += 1;
            while i < chars.len() {
                result.push(chars[i]);
                if chars[i] == '"' { i += 1; break; }
                i += 1;
            }
            continue;
        }

        // 尝试匹配单元格/范围引用：[$]?[A-Za-z]+[$]?[~0-9]+
        if ch == '$' || ch.is_ascii_alphabetic() {
            let start = i;
            let mut pos = i;
            let mut col_abs = false;
            let mut col_letters = String::new();
            let mut row_abs = false;
            let mut row_text = String::new();

            if pos < chars.len() && chars[pos] == '$' {
                col_abs = true;
                pos += 1;
            }

            let col_start = pos;
            while pos < chars.len() && chars[pos].is_ascii_alphabetic() {
                col_letters.push(chars[pos].to_ascii_uppercase());
                pos += 1;
            }
            if pos == col_start || col_letters.is_empty() {
                result.push(ch);
                i += 1;
                continue;
            }

            if pos < chars.len() && chars[pos] == '$' {
                row_abs = true;
                pos += 1;
            }

            // 行部分：数字 或 ~ 占位符
            let row_start = pos;
            while pos < chars.len() && (chars[pos].is_ascii_digit() || chars[pos] == '~') {
                row_text.push(chars[pos]);
                pos += 1;
            }
            if pos == row_start || row_text.is_empty() {
                for c in &chars[start..pos] { result.push(*c); }
                i = pos;
                continue;
            }

            // 解析列号并偏移
            let col_num = match letter_to_col(&col_letters) {
                Ok(c) => c,
                Err(_) => {
                    for c in &chars[start..pos] { result.push(*c); }
                    i = pos;
                    continue;
                }
            };
            let new_col = (col_num as i32 + col_shift).max(1) as u32;

            if col_abs { result.push('$'); }
            result.push_str(&col_to_letter(new_col));
            if row_abs { result.push('$'); }
            result.push_str(&row_text);
            i = pos;
            continue;
        }

        result.push(ch);
        i += 1;
    }
    result
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
    let expanded_rules_list: Vec<ParsedRule> = rules
        .iter()
        .flat_map(|r| expand_batch_rule(r))
        .collect();

    // 后续使用 expanded_rules_list

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

    for rule in &expanded_rules_list {
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

    // 6. 构建源→目标全局映射表（用于公式引用自动调整）
    let mut cell_mapping: std::collections::HashMap<(u32, u32), (u32, u32)> =
        std::collections::HashMap::new();
    for plan in &plans {
        for (src, tgt) in &plan.mappings {
            cell_mapping.insert((src.col, src.row), (tgt.cell.col, tgt.cell.row));
        }
    }

    // 7. 处理每条规则的映射：从项目层 SheetData 读取（RGB 已解析），写入输出 Workbook
    let mut processed = 0usize;

    for plan in &plans {
        for (src, tgt) in &plan.mappings {
            let col_offset = tgt.cell.col as i32 - src.col as i32;
            let row_offset = tgt.cell.row as i32 - src.row as i32;

            deep_copy_cell(
                sheet, out_ws,
                src.col, src.row,
                tgt.cell.col, tgt.cell.row,
                &cell_mapping,
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

    // 8. 应用自定义公式：~ 替换为输出表最大行号
    let max_row = out_ws.highest_row().max(1);
    for rule in &expanded_rules_list {
        for cf in &rule.custom_formulas {
            let formula_text = cf.raw_text.replace('~', &max_row.to_string());
            let cell = out_ws.cell_mut((cf.target_cell.col, cf.target_cell.row));
            cell.set_formula(formula_text.as_str());
        }
    }

    // 8.5 迁移条件格式规则（映射源 sqref → 目标 sqref）
    if let Ok(src_book) = umya_spreadsheet::reader::xlsx::read(Path::new(file_path)) {
        if let Some(src_ws) = src_book.sheet_collection().get(current_sheet) {
            for cf in src_ws.conditional_formatting_collection() {
                let mut mapped_seq = umya_spreadsheet::structs::SequenceOfReferences::default();
                let mut mapped_sqref_parts = Vec::new();

                for range in cf.sequence_of_references().range_collection() {
                    if let (Some(sc), Some(sr), Some(ec), Some(er)) = (
                        range.coordinate_start_col().map(|c| c.num()),
                        range.coordinate_start_row().map(|r| r.num()),
                        range.coordinate_end_col().map(|c| c.num()),
                        range.coordinate_end_row().map(|r| r.num()),
                    ) {
                        // 找到源范围内所有有映射的单元格的目标位置
                        let mut tgt_cols: Vec<u32> = Vec::new();
                        let mut tgt_rows: Vec<u32> = Vec::new();
                        for r in sr..=er {
                            for c in sc..=ec {
                                if let Some(&(tc, tr)) = cell_mapping.get(&(c, r)) {
                                    tgt_cols.push(tc);
                                    tgt_rows.push(tr);
                                }
                            }
                        }
                        if !tgt_cols.is_empty() && !tgt_rows.is_empty() {
                            tgt_cols.sort();
                            tgt_rows.sort();
                            let part = format!(
                                "{}{}:{}{}",
                                col_to_letter(tgt_cols[0]),
                                tgt_rows[0],
                                col_to_letter(*tgt_cols.last().unwrap()),
                                *tgt_rows.last().unwrap(),
                            );
                            mapped_sqref_parts.push(part);
                        }
                    }
                }
                if !mapped_sqref_parts.is_empty() {
                    let new_sqref = mapped_sqref_parts.join(" ");
                    mapped_seq.set_sqref(&new_sqref);

                    // 创建新的 ConditionalFormatting 并添加到输出
                    let mut new_cf = umya_spreadsheet::structs::ConditionalFormatting::default();
                    new_cf.set_sequence_of_references(mapped_seq);
                    for rule in cf.conditional_collection() {
                        new_cf.add_conditional_collection(rule.clone());
                    }
                    out_ws.add_conditional_formatting_collection(new_cf);
                }
            }
        }
    }

    // 9. 写入输出文件
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
        .resizable(true)
        .default_size(egui::vec2(440.0, 330.0))
        .open(&mut keep_open)
        .show(ctx, |ui| {
            ui.set_min_width(440.0);
            ui.set_min_height(260.0);

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

            // 中间区域：多行文本输入框（固定可视高度——不随窗口/内容撑高）
            //
            // 关键：`desired_rows` 与 `max_height` 都是【固定常量】，绝不依赖 `available_height`。
            // 之前的 `desired_rows = ceil(available_height / 行高)` 会形成正反馈：
            //   窗口越高 → 可用越高 → 行数越多 → TextEdit 越高 → 内容 min 越大 →
            //   egui 把可缩放 Window 撑大去容纳内容 → 可用更高 → …… 直至屏幕高度
            //   （实测空内容就把弹窗撑到 ~800px）。ScrollArea 也救不了，因为它的 min 跟着内容走。
            // 固定后：≤ TEXT_ROWS 行正好填满默认窗口（无间隙）；超过则 ScrollArea 在 TEXT_MAX_H 封顶、滚动，不撑高。
            const TEXT_ROWS: usize = 12;
            const TEXT_MAX_H: f32 = 230.0;
            let text_changed = egui::ScrollArea::vertical()
                .max_height(TEXT_MAX_H)
                .max_width(ui.available_width())
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut state.text)
                            .hint_text("请输入转换规则...")
                            .desired_width(f32::INFINITY)
                            .desired_rows(TEXT_ROWS),
                    )
                    .changed()
                })
                .inner;

            // 文本变化时清除状态消息
            if text_changed {
                state.error_message = None;
                state.success_message = None;
            }

            ui.separator();

            // 预解析规则、判断按钮可用性 & 实时错误（不依赖 ui，提前计算）
            let parse_result = if state.text.trim().is_empty() {
                None
            } else {
                Some(parse_rules(&state.text))
            };
            let has_valid_rules = parse_result.as_ref().is_some_and(|r| r.is_ok());
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

            // 底部行：进度条（宽度随窗口自适应）+ 开始转换按钮（右对齐）
            ui.horizontal(|ui| {
                let avail_w = ui.available_width();
                ui.add(
                    egui::ProgressBar::new(state.progress / 100.0)
                        .desired_width((avail_w - 96.0).max(40.0))
                        .text(format!("{:.0}%", state.progress)),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let convert_btn = ui.add_enabled(can_convert, egui::Button::new("开始转换"));

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

    #[test]
    fn test_parse_batch_stepped_basic() {
        let text = "(N(3:12)+2):BV(3:12)->(B+2)15:|~;";
        let rules = parse_rules(text).unwrap();
        assert_eq!(rules.len(), 1);
        let r = &rules[0];
        assert!(matches!(r.source_range, SourceRange::BatchStepped { .. }));
        assert!(matches!(r.target_start, TargetPosition::SteppedTarget { .. }));
        assert_eq!(r.direction, Direction::Vertical);
    }

    #[test]
    fn test_batch_stepped_expansion() {
        let text = "(N(3:12)+2):BV(3:12)->(B+2)15:|~;";
        let rules = parse_rules(text).unwrap();
        let expanded: Vec<ParsedRule> = rules.iter().flat_map(|r| expand_batch_rule(r)).collect();
        // 10 行 (3..12) → 10 条逐行规则
        assert_eq!(expanded.len(), 10);

        // 第一条：行3 → (N3+2):BV3->B15:|~;
        let first = &expanded[0];
        if let SourceRange::Stepped { start, step, end } = &first.source_range {
            assert_eq!(start.col, 14); // N
            assert_eq!(start.row, 3);
            assert_eq!(*step, 2);
            assert_eq!(end.col, 74); // BV
        } else { panic!("Expected Stepped"); }
        if let TargetPosition::Simple(cell) = &first.target_start {
            assert_eq!(cell.col, 2); // B
            assert_eq!(cell.row, 15);
        } else { panic!("Expected Simple"); }

        // 最后一条：行12 → (N12+2):BV12->T15:|~;
        let last = &expanded[9];
        if let SourceRange::Stepped { start, .. } = &last.source_range {
            assert_eq!(start.row, 12);
        } else { panic!("Expected Stepped"); }
        if let TargetPosition::Simple(cell) = &last.target_start {
            assert_eq!(cell.col, 20); // T = 20 (B+9*2 = 2+18 = 20)
            assert_eq!(cell.row, 15);
        } else { panic!("Expected Simple"); }
    }

    #[test]
    fn test_parse_user_rules_with_batch_stepped() {
        let text = "A2:M2->A1:|~;
(N1:O1):(BV1:BW1)->A15:|~;
(A:M)3:(A:M)12->(B(1:13):C(1:13)):-~;
N2:BW2->B14:-~;
(N(3:12)+2):BV(3:12)->(B+2)15:|~;
(O3+2):BW3->C15:|~;";
        let rules = parse_rules(text).unwrap();
        assert_eq!(rules.len(), 6);
        let expanded: Vec<ParsedRule> = rules.iter().flat_map(|r| expand_batch_rule(r)).collect();
        // 4 条普通 + 13 (BatchColumns) + 10 (BatchStepped) = 27
        assert_eq!(expanded.len(), 27);
    }

    #[test]
    fn test_formula_mapping_cross_column_reference() {
        // 模拟: (A:M)3:(A:M)12 -> (B(1:13):C(1:13)):-~;
        // G3(7,3)→B7(2,7), E3(5,3)→B5(2,5)
        // G3 公式 =E3-TODAY() 应变成 =B5-TODAY()
        let mut mapping: std::collections::HashMap<(u32, u32), (u32, u32)> =
            std::collections::HashMap::new();
        // G3 → B7
        mapping.insert((7, 3), (2, 7));
        // E3 → B5
        mapping.insert((5, 3), (2, 5));

        let adjusted = crate::excel::formula::adjust_formula_by_mapping(
            "=E3-TODAY()",
            &mapping,
            -5,  // fallback: G3→B7 col offset
            4,   // fallback: G3→B7 row offset
        );
        assert_eq!(adjusted, "=B5-TODAY()");
    }

    #[test]
    fn test_formula_mapping_fallback_for_unmapped_ref() {
        // 引用不在映射表中 → 使用 fallback 偏移
        let mapping: std::collections::HashMap<(u32, u32), (u32, u32)> =
            std::collections::HashMap::new();
        let adjusted = crate::excel::formula::adjust_formula_by_mapping(
            "=E3*2",
            &mapping,
            -5,
            4,
        );
        // E3: col=5+(-5)=0→A(1), row=3+4=7 → A7
        assert_eq!(adjusted, "=A7*2");
    }

    #[test]
    fn test_formula_mapping_absolute_ref_preserved() {
        // $E$3 在映射表中 → 保持绝对引用不变
        let mut mapping: std::collections::HashMap<(u32, u32), (u32, u32)> =
            std::collections::HashMap::new();
        mapping.insert((5, 3), (2, 5));
        let adjusted = crate::excel::formula::adjust_formula_by_mapping(
            "=$E$3",
            &mapping,
            0,
            0,
        );
        // $E$3: 两者都绝对，即使有映射也不变
        assert_eq!(adjusted, "=$E$3");
    }

    // ---- 自定义公式测试 ----

    #[test]
    fn test_parse_rule_with_custom_formula() {
        let text = "(A:M)3:(A:M)12->(B(1:13):C(1:13)),formula(B12=SUM(B15:B~),B13=SUM($C$15:$C$~)):-~;";
        let rules = parse_rules(text).unwrap();
        assert_eq!(rules.len(), 1);
        let r = &rules[0];
        assert_eq!(r.custom_formulas.len(), 2);
        assert_eq!(r.custom_formulas[0].target_cell.col, 2); // B
        assert_eq!(r.custom_formulas[0].target_cell.row, 12);
        assert_eq!(r.custom_formulas[0].raw_text, "SUM(B15:B~)");
        assert_eq!(r.custom_formulas[1].target_cell.col, 2); // B
        assert_eq!(r.custom_formulas[1].target_cell.row, 13);
        assert_eq!(r.custom_formulas[1].raw_text, "SUM($C$15:$C$~)");
    }

    #[test]
    fn test_parse_rule_without_custom_formula_still_works() {
        // 不含 formula 的规则应正常解析
        let text = "(A:M)3:(A:M)12->(B(1:13):C(1:13)):-~;";
        let rules = parse_rules(text).unwrap();
        assert_eq!(rules.len(), 1);
        assert!(rules[0].custom_formulas.is_empty());
    }

    #[test]
    fn test_parse_simple_rule_with_custom_formula() {
        let text = "A2:M2->A1:|~;";
        let rules = parse_rules(text).unwrap();
        assert_eq!(rules.len(), 1);
        assert!(rules[0].custom_formulas.is_empty());
    }

    #[test]
    fn test_parse_custom_formula_with_nested_parens() {
        let text = "(A:M)3:(A:M)12->(B(1:13):C(1:13)),formula(B12=SUM(B15:B~)+AVERAGE(D15:D~)):-~;";
        let rules = parse_rules(text).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].custom_formulas[0].raw_text, "SUM(B15:B~)+AVERAGE(D15:D~)");
    }

    // ---- 公式列偏移测试 ----

    #[test]
    fn test_shift_formula_columns_basic() {
        assert_eq!(shift_formula_columns("SUM(B15:B~)", 2), "SUM(D15:D~)");
        assert_eq!(shift_formula_columns("B12=SUM(B15:B~)", 2), "D12=SUM(D15:D~)");
    }

    #[test]
    fn test_shift_formula_columns_absolute() {
        assert_eq!(
            shift_formula_columns("SUM($C$15:$C$~)", 2),
            "SUM($E$15:$E$~)"
        );
    }

    #[test]
    fn test_shift_formula_columns_zero() {
        assert_eq!(shift_formula_columns("SUM(B15:B~)", 0), "SUM(B15:B~)");
    }

    #[test]
    fn test_batch_rule_expands_custom_formulas() {
        // 批量规则 + 自定义公式 → 公式按源数据行数横向展开到匹配的子规则
        // (A:C)3:(A:C)12 → 3列 × 10行源数据，目标行1-3，公式在行1和行2
        let text = "(A:C)3:(A:C)12->(B(1:3):C(1:3)),formula(B1=SUM(B15:B~),B2=SUM($C$15:$C$~)):-~;";
        let rules = parse_rules(text).unwrap();
        let expanded: Vec<ParsedRule> = rules.iter().flat_map(|r| expand_batch_rule(r)).collect();
        // A,B,C = 3 列 → 3 条展开规则
        assert_eq!(expanded.len(), 3);

        // 子规则 0 (A, tgt_row=1): B1 公式横向填充 10 份
        let r0 = &expanded[0];
        assert_eq!(r0.custom_formulas.len(), 10); // 仅 B1，填充 10 列
        assert_eq!(r0.custom_formulas[0].target_cell.col, 2); // B1
        assert_eq!(r0.custom_formulas[0].raw_text, "SUM(B15:B~)");
        // 第 10 份 (j=9, col_shift=18): B+18=20=T1
        assert_eq!(r0.custom_formulas[9].target_cell.col, 20); // T
        assert_eq!(r0.custom_formulas[9].raw_text, "SUM(T15:T~)");

        // 子规则 1 (B, tgt_row=2): B2 公式横向填充 10 份
        let r1 = &expanded[1];
        assert_eq!(r1.custom_formulas.len(), 10); // 仅 B2，填充 10 列
        assert_eq!(r1.custom_formulas[0].target_cell.col, 2); // B2
        assert_eq!(r1.custom_formulas[0].raw_text, "SUM($C$15:$C$~)");
        // 第 10 份 (j=9, col_shift=18): 目标列 B(2)+18=20=T2，公式引用 C(3)+18=21=U
        assert_eq!(r1.custom_formulas[9].target_cell.col, 20); // T2
        assert_eq!(r1.custom_formulas[9].raw_text, "SUM($U$15:$U$~)");

        // 子规则 2 (C, tgt_row=3): 无匹配公式
        assert_eq!(expanded[2].custom_formulas.len(), 0);
    }

    #[test]
    fn test_batch_rule_with_13_cols_formula_expansion() {
        // 完整 13 列场景，公式在 B12/B13（在目标行范围 1-13 内）
        // 源 10 行，公式横向填充 10 份
        let text = "(A:M)3:(A:M)12->(B(1:13):C(1:13)),formula(B12=SUM(B15:B~),B13=SUM($C$15:$C$~)):-~;";
        let rules = parse_rules(text).unwrap();
        let expanded: Vec<ParsedRule> = rules.iter().flat_map(|r| expand_batch_rule(r)).collect();
        assert_eq!(expanded.len(), 13);

        // 子规则 11 (L, tgt_row=12): B12 公式横向填充 10 份
        let r11 = &expanded[11];
        assert_eq!(r11.custom_formulas.len(), 10);
        assert_eq!(r11.custom_formulas[0].target_cell.col, 2); // B12
        assert_eq!(r11.custom_formulas[9].target_cell.col, 20); // T = B(2) + 9*2 = 20
        assert_eq!(r11.custom_formulas[0].raw_text, "SUM(B15:B~)");
        assert!(r11.custom_formulas[9].raw_text.contains("SUM(T15:T~)"),
            "Expected T in formula, got: {}", r11.custom_formulas[9].raw_text);

        // 子规则 12 (M, tgt_row=13): B13 公式横向填充 10 份
        let r12 = &expanded[12];
        assert_eq!(r12.custom_formulas.len(), 10);
        assert_eq!(r12.custom_formulas[0].target_cell.col, 2); // B13
        assert_eq!(r12.custom_formulas[9].target_cell.col, 20); // T = B(2) + 9*2 = 20，公式引用 U = C(3) + 18 = 21
        assert_eq!(r12.custom_formulas[0].raw_text, "SUM($C$15:$C$~)");
        assert!(r12.custom_formulas[9].raw_text.contains("SUM($U$15:$U$~)"),
            "Expected U in formula, got: {}", r12.custom_formulas[9].raw_text);

        // 子规则 0-10 无匹配公式
        for i in 0..11 {
            assert_eq!(expanded[i].custom_formulas.len(), 0, "sub-rule {} should have no formulas", i);
        }
    }

    #[test]
    fn test_batch_rule_formula_horizontal_fill_matches_data_extent() {
        // 模拟用户场景：公式横向填充应覆盖数据实际分布范围，而非批量列数
        // (A:C)3:(A:C)5 -> 3列 × 3行源数据(行3,4,5)，目标行1-3
        // 每列源数据有 3 行 → 横向填充 3 个单元格
        // merge_width=2 → 列偏移 0, 2, 4 → 覆盖列 2,4,6 (B,D,F)
        let text = "(A:C)3:(A:C)5->(B(1:3):C(1:3)),formula(B1=COLUMN()/2):-~;";
        let rules = parse_rules(text).unwrap();
        let expanded: Vec<ParsedRule> = rules.iter().flat_map(|r| expand_batch_rule(r)).collect();
        assert_eq!(expanded.len(), 3);

        // 子规则 0 (A, tgt_row=1): B1 匹配 row=1，横向填充 3 份
        let r0 = &expanded[0];
        assert_eq!(r0.custom_formulas.len(), 3,
            "公式应在子规则0中按源数据行数(3)横向填充");
        // j=0: B1(col=2)
        assert_eq!(r0.custom_formulas[0].target_cell.col, 2);
        assert_eq!(r0.custom_formulas[0].target_cell.row, 1);
        // j=1: D1(col=4)
        assert_eq!(r0.custom_formulas[1].target_cell.col, 4);
        assert_eq!(r0.custom_formulas[1].target_cell.row, 1);
        // j=2: F1(col=6)
        assert_eq!(r0.custom_formulas[2].target_cell.col, 6);
        assert_eq!(r0.custom_formulas[2].target_cell.row, 1);

        // 子规则 1,2: 无匹配公式 (公式仅在 row=1，不在 row=2,3)
        assert_eq!(expanded[1].custom_formulas.len(), 0);
        assert_eq!(expanded[2].custom_formulas.len(), 0);

        // 关键验证：公式填充到了数据末尾列 F(6)，而非仅到批量列数边界 C(2) 或 D(4)
        // 旧行为：公式仅填充 3 份(batch列数)，列偏移 0,2,4 → 最后到 F1(col=6)，碰巧和正确结果一致
        // 但用更大的源行数验证：
        // (A:C)3:(A:C)12 → 源 10 行，batch 3 列，公式应填充 10 份而非 3 份
        let text2 = "(A:C)3:(A:C)12->(B(1:3):C(1:3)),formula(B1=COLUMN()/2):-~;";
        let rules2 = parse_rules(text2).unwrap();
        let expanded2: Vec<ParsedRule> = rules2.iter().flat_map(|r| expand_batch_rule(r)).collect();
        assert_eq!(expanded2.len(), 3);

        let r0_2 = &expanded2[0];
        assert_eq!(r0_2.custom_formulas.len(), 10,
            "公式应按源数据行数(10)填充，而非批量列数(3)");
        // 最后一份在 col=2+9*2=20=T1
        assert_eq!(r0_2.custom_formulas[9].target_cell.col, 20); // T1
        assert_eq!(r0_2.custom_formulas[9].target_cell.row, 1);
    }
}
