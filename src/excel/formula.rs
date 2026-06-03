//! Excel 公式求值器
//!
//! 负责解析和计算 Excel 公式，支持基本运算符、单元格引用、范围引用和常用函数。
//! 使用递归下降解析器将公式字符串解析为 AST，然后通过拓扑排序处理依赖关系并求值。

use std::collections::{HashMap, HashSet, VecDeque};
use crate::excel::reader::{ExcelData, SheetData, col_to_letter};

// ========== AST 类型定义 ==========

/// 公式 AST 节点
#[derive(Debug, Clone)]
pub enum FormulaNode {
    /// 数字字面量
    Number(f64),
    /// 字符串字面量
    String(String),
    /// 布尔值
    Boolean(bool),
    /// 单元格引用 (col, row)，均为 1-based
    CellRef { col: u32, row: u32 },
    /// 范围引用
    RangeRef { start_col: u32, start_row: u32, end_col: u32, end_row: u32 },
    /// 二元运算
    BinaryOp { op: BinOp, left: Box<FormulaNode>, right: Box<FormulaNode> },
    /// 一元运算
    UnaryOp { op: UnOp, operand: Box<FormulaNode> },
    /// 函数调用
    Function { name: String, args: Vec<FormulaNode> },
}

/// 二元运算符
#[derive(Debug, Clone, Copy)]
pub enum BinOp {
    Add, Sub, Mul, Div, Pow,
    Concat,        // &
    Eq, Neq, Lt, Le, Gt, Ge,
}

/// 一元运算符
#[derive(Debug, Clone, Copy)]
pub enum UnOp {
    Negate,   // -
    Percent,  // %
}

/// 公式求值结果
#[derive(Debug, Clone)]
pub enum FormulaValue {
    Number(f64),
    String(String),
    Boolean(bool),
    Error(String),
    Blank,
}

impl FormulaValue {
    /// 转换为显示字符串
    pub fn to_display(&self) -> String {
        match self {
            FormulaValue::Number(n) => format_number(*n),
            FormulaValue::String(s) => s.clone(),
            FormulaValue::Boolean(b) => if *b { "TRUE".to_string() } else { "FALSE".to_string() },
            FormulaValue::Error(e) => e.clone(),
            FormulaValue::Blank => String::new(),
        }
    }

    /// 尝试转换为 f64
    pub fn as_number(&self) -> Option<f64> {
        match self {
            FormulaValue::Number(n) => Some(*n),
            FormulaValue::Boolean(b) => Some(if *b { 1.0 } else { 0.0 }),
            FormulaValue::Blank => None,
            FormulaValue::String(s) => s.parse::<f64>().ok(),
            FormulaValue::Error(_) => None,
        }
    }

    /// 尝试转换为布尔值
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            FormulaValue::Boolean(b) => Some(*b),
            FormulaValue::Number(n) => Some(*n != 0.0),
            FormulaValue::String(s) => {
                let upper = s.to_uppercase();
                match upper.as_str() {
                    "TRUE" => Some(true),
                    "FALSE" => Some(false),
                    _ => None,
                }
            }
            FormulaValue::Blank => Some(false),
            FormulaValue::Error(_) => None,
        }
    }

    fn is_error(&self) -> bool {
        matches!(self, FormulaValue::Error(_))
    }
}

/// 格式化数字：去除不必要的尾零
fn format_number(n: f64) -> String {
    if n.is_nan() { return "#VALUE!".to_string(); }
    if n.is_infinite() { return "#DIV/0!".to_string(); }
    if n == n.trunc() && n.abs() < i64::MAX as f64 {
        format!("{}", n as i64)
    } else {
        // 最多保留10位小数，去除尾零
        let s = format!("{:.10}", n);
        let s = s.trim_end_matches('0');
        s.trim_end_matches('.').to_string()
    }
}

// ========== 词法分析器 ==========

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    StringLit(String),
    Ident(String),     // 函数名或关键字 (SUM, IF, TRUE, FALSE)
    CellRef(String),   // A1, $A$1 等
    Plus, Minus, Star, Slash, Caret, Ampersand, Percent,
    Eq, Lt, Gt, Le, Ge, Neq,
    LParen, RParen, Comma, Colon,
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
}

impl Lexer {
    fn new(input: &str) -> Self {
        Lexer { chars: input.chars().collect(), pos: 0 }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.chars.get(self.pos).copied();
        if ch.is_some() { self.pos += 1; }
        ch
    }

    fn skip_whitespace(&mut self) {
        while self.peek().map_or(false, |c| c.is_whitespace()) {
            self.advance();
        }
    }

    fn tokenize(&mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace();
            match self.peek() {
                None => break,
                Some(ch) => {
                    let tok = match ch {
                        '+' => { self.advance(); Token::Plus }
                        '-' => { self.advance(); Token::Minus }
                        '*' => { self.advance(); Token::Star }
                        '/' => { self.advance(); Token::Slash }
                        '^' => { self.advance(); Token::Caret }
                        '&' => { self.advance(); Token::Ampersand }
                        '%' => { self.advance(); Token::Percent }
                        '(' => { self.advance(); Token::LParen }
                        ')' => { self.advance(); Token::RParen }
                        ',' => { self.advance(); Token::Comma }
                        ':' => { self.advance(); Token::Colon }
                        '=' => { self.advance(); Token::Eq }
                        '<' => {
                            self.advance();
                            if self.peek() == Some('>') { self.advance(); Token::Neq }
                            else if self.peek() == Some('=') { self.advance(); Token::Le }
                            else { Token::Lt }
                        }
                        '>' => {
                            self.advance();
                            if self.peek() == Some('=') { self.advance(); Token::Ge }
                            else { Token::Gt }
                        }
                        '"' => self.lex_string()?,
                        '$' | '_' | 'A'..='Z' => self.lex_ident_or_cellref()?,
                        '0'..='9' | '.' => self.lex_number()?,
                        _ => return Err(format!("无法识别的字符: '{}'", ch)),
                    };
                    tokens.push(tok);
                }
            }
        }
        Ok(tokens)
    }

    fn lex_string(&mut self) -> Result<Token, String> {
        self.advance(); // skip opening "
        let mut s = String::new();
        loop {
            match self.advance() {
                None => return Err("字符串未闭合".to_string()),
                Some('"') => break,
                Some(ch) => s.push(ch),
            }
        }
        Ok(Token::StringLit(s))
    }

    fn lex_number(&mut self) -> Result<Token, String> {
        let mut s = String::new();
        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() || ch == '.' {
                s.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        let n: f64 = s.parse().map_err(|_| format!("无效数字: {}", s))?;
        Ok(Token::Number(n))
    }

    /// 解析标识符或单元格引用
    /// 以 $、_ 或字母开头，可能是：$A$1, A1, SUM, TRUE, _xlfn.IFS 等
    /// 状态机：0=before_col, 1=in_col, 2=after_col, 3=in_row, 10=ident
    fn lex_ident_or_cellref(&mut self) -> Result<Token, String> {
        let mut s = String::new();
        let mut alpha_part = String::new();
        let mut digit_part = String::new();
        let mut state: u8 = 0;
        let mut is_ident = false;

        while let Some(ch) = self.peek() {
            if is_ident {
                // 标识符模式：接受字母、数字、下划线、点
                match ch {
                    'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '.' => {
                        s.push(ch);
                        self.advance();
                    }
                    _ => break,
                }
                continue;
            }

            match ch {
                '$' => {
                    s.push(ch);
                    self.advance();
                    if state == 0 {
                        // 前导 $，仍处于"列前"状态，继续读列字母
                    } else if state == 1 {
                        state = 2; // 列后 $，期待行号
                    } else {
                        break; // 多余的 $
                    }
                }
                'A'..='Z' | 'a'..='z' => {
                    if state <= 1 {
                        let upper = ch.to_ascii_uppercase();
                        s.push(ch);
                        alpha_part.push(upper);
                        self.advance();
                        state = 1; // 正在读列字母
                    } else {
                        break; // 列字母后不能再出现字母
                    }
                }
                '0'..='9' => {
                    s.push(ch);
                    digit_part.push(ch);
                    self.advance();
                    state = 3; // 正在读行号
                }
                '_' | '.' => {
                    // 下划线或点号表示这是标识符（如 _xlfn.IFS）
                    s.push(ch);
                    self.advance();
                    is_ident = true;
                }
                _ => break,
            }
        }

        // 判断是单元格引用还是标识符
        if !is_ident && state == 3 && !alpha_part.is_empty() && !digit_part.is_empty() {
            if alpha_part.len() <= 3 && alpha_part.chars().all(|c| c.is_ascii_uppercase()) {
                return Ok(Token::CellRef(s.to_uppercase()));
            }
        }

        // 是标识符（函数名、TRUE、FALSE 等）
        let upper = s.to_uppercase();
        Ok(Token::Ident(upper))
    }
}

// ========== 递归下降解析器 ==========

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<Token> {
        let tok = self.tokens.get(self.pos).cloned();
        if tok.is_some() { self.pos += 1; }
        tok
    }

    fn expect(&mut self, expected: &Token) -> Result<(), String> {
        match self.advance() {
            Some(ref tok) if tok == expected => Ok(()),
            other => Err(format!("期望 {:?}, 得到 {:?}", expected, other)),
        }
    }

    fn parse(&mut self) -> Result<FormulaNode, String> {
        let node = self.parse_comparison()?;
        if self.peek().is_some() {
            return Err(format!("意外的 token: {:?}", self.peek()));
        }
        Ok(node)
    }

    // comparison = addition (('='|'<>'|'<'|'<='|'>'|'>=') addition)*
    fn parse_comparison(&mut self) -> Result<FormulaNode, String> {
        let mut left = self.parse_addition()?;
        loop {
            let op = match self.peek() {
                Some(Token::Eq) => BinOp::Eq,
                Some(Token::Neq) => BinOp::Neq,
                Some(Token::Lt) => BinOp::Lt,
                Some(Token::Le) => BinOp::Le,
                Some(Token::Gt) => BinOp::Gt,
                Some(Token::Ge) => BinOp::Ge,
                _ => break,
            };
            self.advance();
            let right = self.parse_addition()?;
            left = FormulaNode::BinaryOp { op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    // addition = multiplication (('+'|'-') multiplication)*
    fn parse_addition(&mut self) -> Result<FormulaNode, String> {
        let mut left = self.parse_multiplication()?;
        loop {
            let op = match self.peek() {
                Some(Token::Plus) => BinOp::Add,
                Some(Token::Minus) => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_multiplication()?;
            left = FormulaNode::BinaryOp { op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    // multiplication = power (('*'|'/') power)*
    fn parse_multiplication(&mut self) -> Result<FormulaNode, String> {
        let mut left = self.parse_power()?;
        loop {
            let op = match self.peek() {
                Some(Token::Star) => BinOp::Mul,
                Some(Token::Slash) => BinOp::Div,
                _ => break,
            };
            self.advance();
            let right = self.parse_power()?;
            left = FormulaNode::BinaryOp { op, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    // power = concat ('^' concat)*
    fn parse_power(&mut self) -> Result<FormulaNode, String> {
        let mut left = self.parse_concat()?;
        loop {
            if matches!(self.peek(), Some(Token::Caret)) {
                self.advance();
                let right = self.parse_concat()?;
                left = FormulaNode::BinaryOp { op: BinOp::Pow, left: Box::new(left), right: Box::new(right) };
            } else {
                break;
            }
        }
        Ok(left)
    }

    // concat = unary ('&' unary)*
    fn parse_concat(&mut self) -> Result<FormulaNode, String> {
        let mut left = self.parse_unary()?;
        loop {
            if matches!(self.peek(), Some(Token::Ampersand)) {
                self.advance();
                let right = self.parse_unary()?;
                left = FormulaNode::BinaryOp { op: BinOp::Concat, left: Box::new(left), right: Box::new(right) };
            } else {
                break;
            }
        }
        Ok(left)
    }

    // unary = ('-'|'%')? primary
    fn parse_unary(&mut self) -> Result<FormulaNode, String> {
        match self.peek() {
            Some(Token::Minus) => {
                self.advance();
                let operand = self.parse_primary()?;
                Ok(FormulaNode::UnaryOp { op: UnOp::Negate, operand: Box::new(operand) })
            }
            Some(Token::Percent) => {
                self.advance();
                let operand = self.parse_primary()?;
                Ok(FormulaNode::UnaryOp { op: UnOp::Percent, operand: Box::new(operand) })
            }
            _ => self.parse_primary(),
        }
    }

    // primary = function_call | cell_or_range | number | string | boolean | '(' expression ')'
    fn parse_primary(&mut self) -> Result<FormulaNode, String> {
        match self.peek().cloned() {
            Some(Token::LParen) => {
                self.advance();
                let node = self.parse_comparison()?;
                self.expect(&Token::RParen).map_err(|e| format!("括号未闭合: {}", e))?;
                Ok(node)
            }
            Some(Token::Number(n)) => {
                self.advance();
                Ok(FormulaNode::Number(n))
            }
            Some(Token::StringLit(s)) => {
                self.advance();
                Ok(FormulaNode::String(s))
            }
            Some(Token::Ident(name)) => {
                // 检查是否是布尔值
                match name.as_str() {
                    "TRUE" => { self.advance(); return Ok(FormulaNode::Boolean(true)); }
                    "FALSE" => { self.advance(); return Ok(FormulaNode::Boolean(false)); }
                    _ => {}
                }
                // 函数调用
                if matches!(self.tokens.get(self.pos + 1), Some(Token::LParen)) {
                    return self.parse_function(&name);
                }
                // 否则当作未知名称
                self.advance();
                Err(format!("未知名称: {}", name))
            }
            Some(Token::CellRef(_)) => self.parse_cell_or_range(),
            other => Err(format!("意外的 token: {:?}", other)),
        }
    }

    // cell_or_range = CELLREF (':' CELLREF)?
    fn parse_cell_or_range(&mut self) -> Result<FormulaNode, String> {
        let first = match self.advance() {
            Some(Token::CellRef(s)) => s,
            _ => unreachable!(),
        };
        let (col1, row1) = parse_cell_ref_str(&first)?;

        if matches!(self.peek(), Some(Token::Colon)) {
            self.advance();
            let second = match self.advance() {
                Some(Token::CellRef(s)) => s,
                other => return Err(format!("范围引用缺少结束单元格: {:?}", other)),
            };
            let (col2, row2) = parse_cell_ref_str(&second)?;
            return Ok(FormulaNode::RangeRef {
                start_col: col1.min(col2), start_row: row1.min(row2),
                end_col: col1.max(col2), end_row: row1.max(row2),
            });
        }
        Ok(FormulaNode::CellRef { col: col1, row: row1 })
    }

    // function_call = IDENT '(' arg_list? ')'
    fn parse_function(&mut self, name: &str) -> Result<FormulaNode, String> {
        self.advance(); // consume IDENT
        self.advance(); // consume '('
        let mut args = Vec::new();
        if !matches!(self.peek(), Some(Token::RParen)) {
            args.push(self.parse_comparison()?);
            while matches!(self.peek(), Some(Token::Comma)) {
                self.advance();
                args.push(self.parse_comparison()?);
            }
        }
        self.expect(&Token::RParen).map_err(|e| format!("函数 {} 括号未闭合: {}", name, e))?;
        Ok(FormulaNode::Function { name: name.to_uppercase(), args })
    }
}

/// 解析单元格引用字符串（如 "A1", "$A$1"）为 (col, row)
fn parse_cell_ref_str(s: &str) -> Result<(u32, u32), String> {
    let s = s.replace('$', "");
    let mut col_str = String::new();
    let mut row_str = String::new();
    let mut in_digits = false;
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            in_digits = true;
            row_str.push(ch);
        } else if ch.is_ascii_alphabetic() {
            if in_digits { return Err(format!("无效的单元格引用: {}", s)); }
            col_str.push(ch.to_ascii_uppercase());
        }
    }
    if col_str.is_empty() || row_str.is_empty() {
        return Err(format!("无效的单元格引用: {}", s));
    }
    let col = letter_to_col(&col_str)?;
    let row: u32 = row_str.parse().map_err(|_| format!("无效行号: {}", row_str))?;
    Ok((col, row))
}

/// 列字母转列号 (A=1, B=2, ..., Z=26, AA=27)
pub fn letter_to_col(s: &str) -> Result<u32, String> {
    let mut col = 0u32;
    for ch in s.chars() {
        if !ch.is_ascii_alphabetic() { return Err(format!("无效列字母: {}", s)); }
        col = col * 26 + (ch as u32 - 'A' as u32 + 1);
    }
    if col == 0 { return Err("列号不能为0".to_string()); }
    Ok(col)
}

// ========== 公式解析入口 ==========

/// 解析公式字符串为 AST（去掉前导 '='）
pub fn parse_formula(input: &str) -> Result<FormulaNode, String> {
    let trimmed = input.trim();
    let formula_str = if trimmed.starts_with('=') { &trimmed[1..] } else { trimmed };
    // 去除 Excel 隐式交叉运算符 @（如 @IF → IF）
    let formula_str = if formula_str.starts_with('@') { &formula_str[1..] } else { formula_str };
    // 去除 OOXML 新版函数前缀 _xlfn.（如 _xlfn.IFS → IFS）
    let preprocessed = formula_str.replace("_xlfn.", "");
    let mut lexer = Lexer::new(&preprocessed);
    let tokens = lexer.tokenize()?;
    let mut parser = Parser::new(tokens);
    parser.parse()
}

// ========== 公式列偏移调整 ==========

/// 调整公式字符串中的列引用。
///
/// 对 `threshold_col` 及之后的**相对列引用**偏移 `shift` 列（正数右移，负数左移）。
/// 绝对引用（`$A`）不变，行号不变。跳过字符串字面量内的内容。
///
/// # 参数
/// * `formula` - 公式字符串（可含前导 `=`）
/// * `threshold_col` - 列号阈值，仅 >= 此列号的相对引用会被偏移
/// * `shift` - 偏移列数（正数右移，负数左移）
pub fn adjust_formula_columns(formula: &str, threshold_col: u32, shift: i32) -> String {
    if formula.is_empty() || shift == 0 {
        return formula.to_string();
    }
    let chars: Vec<char> = formula.chars().collect();
    let mut result = String::with_capacity(formula.len());
    let mut i = 0;

    // 跳过前导 '=' 或 '@'
    if i < chars.len() && chars[i] == '=' {
        result.push('=');
        i += 1;
    }
    if i < chars.len() && chars[i] == '@' {
        result.push('@');
        i += 1;
    }

    while i < chars.len() {
        let ch = chars[i];

        // 字符串字面量：原样输出
        if ch == '"' {
            result.push(ch);
            i += 1;
            while i < chars.len() {
                result.push(chars[i]);
                if chars[i] == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }

        // 尝试匹配单元格引用：[$]?[A-Za-z]+[$]?[0-9]+
        if ch == '$' || ch.is_ascii_alphabetic() {
            let start = i;
            let mut pos = i;
            let mut col_abs = false; // 列是否绝对引用 ($A)
            let mut col_letters = String::new();
            let mut row_abs = false;  // 行是否绝对引用 ($1)
            let mut row_digits = String::new();
            let mut _state: u8 = 0; // 0=before_col, 1=in_col, 2=after_col, 3=in_row

            // 可选的列绝对前缀 $
            if pos < chars.len() && chars[pos] == '$' {
                col_abs = true;
                pos += 1;
            }

            // 列字母
            let col_start = pos;
            while pos < chars.len() && chars[pos].is_ascii_alphabetic() {
                col_letters.push(chars[pos].to_ascii_uppercase());
                pos += 1;
            }
            if pos == col_start || col_letters.is_empty() {
                // 不是单元格引用（如 $ 后直接跟数字，或是标识符开头）
                // 原样输出从 start 到 pos 的内容
                // 但可能只是个 $ 开头的标识符，继续作为普通字符处理
                result.push(ch);
                i += 1;
                continue;
            }
            _state = 1; // 读完列字母

            // 可选的行绝对前缀 $
            if pos < chars.len() && chars[pos] == '$' {
                row_abs = true;
                pos += 1;
            }

            // 行号数字
            let row_start = pos;
            while pos < chars.len() && chars[pos].is_ascii_digit() {
                row_digits.push(chars[pos]);
                pos += 1;
            }
            if pos == row_start || row_digits.is_empty() {
                // 没有行号 → 不是单元格引用，是函数名或标识符（如 SUM, A1B2 中的前缀）
                // 但需要检查是否是范围引用的一部分，如 A:C
                // 如果后面跟 : 则是列范围，否则是标识符
                // 先按标识符处理，原样输出
                for c in &chars[start..pos] {
                    result.push(*c);
                }
                i = pos;
                continue;
            }

            // 到这里，成功匹配了一个单元格引用
            // 解析列号
            let col_num = match letter_to_col(&col_letters) {
                Ok(c) => c,
                Err(_) => {
                    // 解析失败，原样输出
                    for c in &chars[start..pos] {
                        result.push(*c);
                    }
                    i = pos;
                    continue;
                }
            };

            // 判断是否需要偏移：列号 >= threshold_col（绝对引用也偏移，列插入是结构性操作）
            if col_num >= threshold_col {
                let new_col = (col_num as i32 + shift).max(1) as u32;
                let new_col_str = col_to_letter(new_col);
                // 保留列绝对标记 $
                if col_abs {
                    result.push('$');
                }
                result.push_str(&new_col_str);
                if row_abs {
                    result.push('$');
                }
                result.push_str(&row_digits);
            } else {
                // 原样输出
                for c in &chars[start..pos] {
                    result.push(*c);
                }
            }
            i = pos;
            continue;
        }

        // 其他字符原样输出
        result.push(ch);
        i += 1;
    }

    result
}

// ========== 公式行偏移调整 ==========

/// 调整公式字符串中的行引用。
///
/// 对行号 >= `threshold_row` 的**相对行引用**偏移 `shift` 行（正数下移，负数上移）。
/// 绝对行引用（`$1`）不变，列号不变。跳过字符串字面量内的内容。
///
/// # 参数
/// * `formula` - 公式字符串（可含前导 `=`）
/// * `threshold_row` - 行号阈值，仅 >= 此行号的相对行引用会被偏移
/// * `shift` - 偏移行数（正数下移，负数上移）
pub fn adjust_formula_rows(formula: &str, threshold_row: u32, shift: i32) -> String {
    if formula.is_empty() || shift == 0 {
        return formula.to_string();
    }
    let chars: Vec<char> = formula.chars().collect();
    let mut result = String::with_capacity(formula.len());
    let mut i = 0;

    // 跳过前导 '=' 或 '@'
    if i < chars.len() && chars[i] == '=' {
        result.push('=');
        i += 1;
    }
    if i < chars.len() && chars[i] == '@' {
        result.push('@');
        i += 1;
    }

    while i < chars.len() {
        let ch = chars[i];

        // 字符串字面量：原样输出
        if ch == '"' {
            result.push(ch);
            i += 1;
            while i < chars.len() {
                result.push(chars[i]);
                if chars[i] == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }

        // 尝试匹配单元格引用：[$]?[A-Za-z]+[$]?[0-9]+
        if ch == '$' || ch.is_ascii_alphabetic() {
            let start = i;
            let mut pos = i;
            let mut col_abs = false; // 列是否绝对引用 ($A)
            let mut col_letters = String::new();
            let mut row_abs = false;  // 行是否绝对引用 ($1)
            let mut row_digits = String::new();

            // 可选的列绝对前缀 $
            if pos < chars.len() && chars[pos] == '$' {
                col_abs = true;
                pos += 1;
            }

            // 列字母
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

            // 可选的行绝对前缀 $
            if pos < chars.len() && chars[pos] == '$' {
                row_abs = true;
                pos += 1;
            }

            // 行号数字
            let row_start = pos;
            while pos < chars.len() && chars[pos].is_ascii_digit() {
                row_digits.push(chars[pos]);
                pos += 1;
            }
            if pos == row_start || row_digits.is_empty() {
                // 没有行号 → 不是单元格引用
                for c in &chars[start..pos] {
                    result.push(*c);
                }
                i = pos;
                continue;
            }

            // 到这里，成功匹配了一个单元格引用
            // 解析列号（仅用于验证，列号不变）
            let col_num = match letter_to_col(&col_letters) {
                Ok(c) => c,
                Err(_) => {
                    for c in &chars[start..pos] {
                        result.push(*c);
                    }
                    i = pos;
                    continue;
                }
            };

            // 解析行号
            let row_num: u32 = match row_digits.parse() {
                Ok(r) => r,
                Err(_) => {
                    for c in &chars[start..pos] {
                        result.push(*c);
                    }
                    i = pos;
                    continue;
                }
            };

            // 判断是否需要偏移：行号 >= threshold_row 且行引用不是绝对引用
            // 列号不变（列偏移由 adjust_formula_columns 负责）
            if !row_abs && row_num >= threshold_row {
                let new_row = (row_num as i32 + shift).max(1) as u32;
                // 保留列绝对标记 $
                if col_abs {
                    result.push('$');
                }
                result.push_str(&col_letters);
                if row_abs {
                    result.push('$');
                }
                result.push_str(&new_row.to_string());
            } else {
                // 原样输出
                for c in &chars[start..pos] {
                    result.push(*c);
                }
            }
            i = pos;
            // 消除 "col_num 未使用" 警告
            let _ = col_num;
            continue;
        }

        // 其他字符原样输出
        result.push(ch);
        i += 1;
    }

    result
}

// ========== 求值器 ==========

/// 从 SheetData 获取单元格的值
fn get_cell_value(sheet: &SheetData, row: u32, col: u32) -> FormulaValue {
    match sheet.get_cell(row, col) {
        Some(cell) => {
            if cell.value.is_empty() {
                FormulaValue::Blank
            } else if let Ok(n) = cell.value.parse::<f64>() {
                FormulaValue::Number(n)
            } else if let Some(n) = cell.raw_number {
                // 显示值无法解析为数字时（如日期 "2025/7/1"），
                // 使用原始数值（Excel 日期序列号等）供公式计算
                FormulaValue::Number(n)
            } else if let Some(serial) = ExcelData::parse_date_string(&cell.value) {
                // 值为格式化日期字符串（如 "2026/02/06"），解析回序列号供公式计算。
                // parse_date_string 只匹配严格的三段日期格式，不会误匹配普通文本。
                FormulaValue::Number(serial)
            } else {
                match cell.value.to_uppercase().as_str() {
                    "TRUE" => FormulaValue::Boolean(true),
                    "FALSE" => FormulaValue::Boolean(false),
                    _ => FormulaValue::String(cell.value.clone()),
                }
            }
        }
        None => FormulaValue::Blank,
    }
}

/// 收集范围中的所有值
fn collect_range_values(sheet: &SheetData, start_col: u32, start_row: u32, end_col: u32, end_row: u32) -> Vec<FormulaValue> {
    let mut values = Vec::new();
    for r in start_row..=end_row {
        for c in start_col..=end_col {
            values.push(get_cell_value(sheet, r, c));
        }
    }
    values
}

/// 求值 AST 节点
/// eval_pos: 当前公式所在单元格位置 (row, col)，用于 ROW()、COLUMN() 等函数
fn eval_node(node: &FormulaNode, sheet: &SheetData, eval_pos: (u32, u32)) -> FormulaValue {
    match node {
        FormulaNode::Number(n) => FormulaValue::Number(*n),
        FormulaNode::String(s) => FormulaValue::String(s.clone()),
        FormulaNode::Boolean(b) => FormulaValue::Boolean(*b),
        FormulaNode::CellRef { col, row } => get_cell_value(sheet, *row, *col),
        FormulaNode::RangeRef { start_col, start_row, end_col, end_row } => {
            // 独立的范围引用在表达式上下文中无效（如 =A1:B5），返回错误
            // 但在函数调用中会被特殊处理
            let _ = (start_col, start_row, end_col, end_row);
            FormulaValue::Error("#VALUE!".to_string())
        }
        FormulaNode::UnaryOp { op, operand } => eval_unary(op, operand, sheet, eval_pos),
        FormulaNode::BinaryOp { op, left, right } => eval_binary(op, left, right, sheet, eval_pos),
        FormulaNode::Function { name, args } => eval_function(name, args, sheet, eval_pos),
    }
}

fn eval_unary(op: &UnOp, operand: &FormulaNode, sheet: &SheetData, eval_pos: (u32, u32)) -> FormulaValue {
    let val = eval_node(operand, sheet, eval_pos);
    if val.is_error() { return val; }
    match op {
        UnOp::Negate => {
            match val.as_number() {
                Some(n) => FormulaValue::Number(-n),
                None => FormulaValue::Number(0.0),
            }
        }
        UnOp::Percent => {
            match val.as_number() {
                Some(n) => FormulaValue::Number(n / 100.0),
                None => FormulaValue::Number(0.0),
            }
        }
    }
}

fn eval_binary(op: &BinOp, left: &FormulaNode, right: &FormulaNode, sheet: &SheetData, eval_pos: (u32, u32)) -> FormulaValue {
    let lv = eval_node(left, sheet, eval_pos);
    let rv = eval_node(right, sheet, eval_pos);
    if lv.is_error() { return lv; }
    if rv.is_error() { return rv; }

    match op {
        BinOp::Concat => {
            let ls = val_to_string(&lv);
            let rs = val_to_string(&rv);
            FormulaValue::String(format!("{}{}", ls, rs))
        }
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow => {
            // Excel 行为：空单元格在算术运算中等同于 0
            let ln = match lv.as_number() {
                Some(n) => n,
                None => 0.0,
            };
            let rn = match rv.as_number() {
                Some(n) => n,
                None => 0.0,
            };
            let result = match op {
                BinOp::Add => ln + rn,
                BinOp::Sub => ln - rn,
                BinOp::Mul => ln * rn,
                BinOp::Div => {
                    if rn == 0.0 { return FormulaValue::Error("#DIV/0!".to_string()); }
                    ln / rn
                }
                BinOp::Pow => ln.powf(rn),
                _ => unreachable!(),
            };
            FormulaValue::Number(result)
        }
        BinOp::Eq | BinOp::Neq | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            // 尝试数值比较，否则字符串比较
            let result = match (lv.as_number(), rv.as_number()) {
                (Some(ln), Some(rn)) => {
                    match op {
                        BinOp::Eq => ln == rn,
                        BinOp::Neq => ln != rn,
                        BinOp::Lt => ln < rn,
                        BinOp::Le => ln <= rn,
                        BinOp::Gt => ln > rn,
                        BinOp::Ge => ln >= rn,
                        _ => unreachable!(),
                    }
                }
                _ => {
                    let ls = val_to_string(&lv);
                    let rs = val_to_string(&rv);
                    match op {
                        BinOp::Eq => ls.eq_ignore_ascii_case(&rs),
                        BinOp::Neq => !ls.eq_ignore_ascii_case(&rs),
                        BinOp::Lt => ls < rs,
                        BinOp::Le => ls <= rs,
                        BinOp::Gt => ls > rs,
                        BinOp::Ge => ls >= rs,
                        _ => unreachable!(),
                    }
                }
            };
            FormulaValue::Boolean(result)
        }
    }
}

fn val_to_string(v: &FormulaValue) -> String {
    match v {
        FormulaValue::Blank => String::new(),
        FormulaValue::Boolean(b) => if *b { "TRUE".to_string() } else { "FALSE".to_string() },
        FormulaValue::Number(n) => format_number(*n),
        FormulaValue::String(s) => s.clone(),
        FormulaValue::Error(e) => e.clone(),
    }
}

// ========== 函数实现 ==========

/// 收集参数中的所有值（展开范围引用）
fn collect_args_values(args: &[FormulaNode], sheet: &SheetData, eval_pos: (u32, u32)) -> Vec<FormulaValue> {
    let mut values = Vec::new();
    for arg in args {
        match arg {
            FormulaNode::RangeRef { start_col, start_row, end_col, end_row } => {
                values.extend(collect_range_values(sheet, *start_col, *start_row, *end_col, *end_row));
            }
            _ => values.push(eval_node(arg, sheet, eval_pos)),
        }
    }
    values
}

fn eval_function(name: &str, args: &[FormulaNode], sheet: &SheetData, eval_pos: (u32, u32)) -> FormulaValue {
    match name {
        "SUM" => {
            let values = collect_args_values(args, sheet, eval_pos);
            let mut sum = 0.0;
            for v in &values {
                if v.is_error() { return v.clone(); }
                if let Some(n) = v.as_number() { sum += n; }
            }
            FormulaValue::Number(sum)
        }
        "AVERAGE" => {
            let values = collect_args_values(args, sheet, eval_pos);
            let mut sum = 0.0;
            let mut count = 0u32;
            for v in &values {
                if v.is_error() { return v.clone(); }
                if let Some(n) = v.as_number() { sum += n; count += 1; }
            }
            if count == 0 { FormulaValue::Error("#DIV/0!".to_string()) }
            else { FormulaValue::Number(sum / count as f64) }
        }
        "COUNT" => {
            let values = collect_args_values(args, sheet, eval_pos);
            let mut count = 0u32;
            for v in &values {
                if matches!(v, FormulaValue::Number(_)) { count += 1; }
            }
            FormulaValue::Number(count as f64)
        }
        "MAX" => {
            let values = collect_args_values(args, sheet, eval_pos);
            let mut max: Option<f64> = None;
            for v in &values {
                if v.is_error() { return v.clone(); }
                if let Some(n) = v.as_number() {
                    max = Some(max.map_or(n, |m: f64| m.max(n)));
                }
            }
            max.map_or(FormulaValue::Number(0.0), FormulaValue::Number)
        }
        "MIN" => {
            let values = collect_args_values(args, sheet, eval_pos);
            let mut min: Option<f64> = None;
            for v in &values {
                if v.is_error() { return v.clone(); }
                if let Some(n) = v.as_number() {
                    min = Some(min.map_or(n, |m: f64| m.min(n)));
                }
            }
            min.map_or(FormulaValue::Number(0.0), FormulaValue::Number)
        }
        "IF" => {
            if args.len() < 2 { return FormulaValue::Error("#VALUE!".to_string()); }
            let cond = eval_node(&args[0], sheet, eval_pos);
            let cond_bool = cond.as_bool().unwrap_or(false);
            if cond_bool {
                eval_node(&args[1], sheet, eval_pos)
            } else {
                if args.len() >= 3 { eval_node(&args[2], sheet, eval_pos) }
                else { FormulaValue::Boolean(false) }
            }
        }
        "AND" => {
            let values = collect_args_values(args, sheet, eval_pos);
            for v in &values {
                if v.is_error() { return v.clone(); }
                if !v.as_bool().unwrap_or(false) { return FormulaValue::Boolean(false); }
            }
            FormulaValue::Boolean(true)
        }
        "OR" => {
            let values = collect_args_values(args, sheet, eval_pos);
            for v in &values {
                if v.is_error() { return v.clone(); }
                if v.as_bool().unwrap_or(false) { return FormulaValue::Boolean(true); }
            }
            FormulaValue::Boolean(false)
        }
        "NOT" => {
            if args.len() != 1 { return FormulaValue::Error("#VALUE!".to_string()); }
            let val = eval_node(&args[0], sheet, eval_pos);
            if val.is_error() { return val; }
            FormulaValue::Boolean(!val.as_bool().unwrap_or(false))
        }
        "CONCATENATE" => {
            let mut result = String::new();
            for arg in args {
                match arg {
                    FormulaNode::RangeRef { start_col, start_row, end_col, end_row } => {
                        for v in collect_range_values(sheet, *start_col, *start_row, *end_col, *end_row) {
                            result.push_str(&val_to_string(&v));
                        }
                    }
                    _ => result.push_str(&val_to_string(&eval_node(arg, sheet, eval_pos))),
                }
            }
            FormulaValue::String(result)
        }
        "IFS" => {
            // IFS(cond1, val1, cond2, val2, ...) 返回第一个为真的条件对应的值
            if args.is_empty() || args.len() % 2 != 0 {
                return FormulaValue::Error("#VALUE!".to_string());
            }
            let mut i = 0;
            while i < args.len() {
                let cond = eval_node(&args[i], sheet, eval_pos);
                if cond.as_bool().unwrap_or(false) {
                    return eval_node(&args[i + 1], sheet, eval_pos);
                }
                i += 2;
            }
            FormulaValue::Error("#N/A".to_string())
        }
        "SUMIF" => {
            // SUMIF(range, criteria, [sum_range])
            if args.len() < 2 { return FormulaValue::Error("#VALUE!".to_string()); }
            let criteria_val = eval_node(&args[1], sheet, eval_pos);
            let criteria_str = val_to_string(&criteria_val);
            let check_values = extract_range_values_from_node(&args[0], sheet, eval_pos);
            let sum_values = if args.len() >= 3 {
                extract_range_values_from_node(&args[2], sheet, eval_pos)
            } else {
                check_values.clone()
            };
            let mut sum = 0.0;
            for (i, cv) in check_values.iter().enumerate() {
                if matches_criteria(cv, &criteria_str) {
                    if let Some(sv) = sum_values.get(i) {
                        if let Some(n) = sv.as_number() { sum += n; }
                    }
                }
            }
            FormulaValue::Number(sum)
        }
        "TODAY" => {
            // TODAY() 返回今天日期的 Excel 序列号
            // Excel 序列号：1900-01-01 = 1，Unix epoch 1970-01-01 = 25569
            use std::time::{SystemTime, UNIX_EPOCH};
            let days = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() / 86400;
            FormulaValue::Number(days as f64 + 25569.0)
        }
        "ROW" => {
            if args.is_empty() {
                // ROW() - 返回当前公式所在单元格的行号
                FormulaValue::Number(eval_pos.0 as f64)
            } else {
                // ROW(cell_ref) - 返回引用单元格的行号
                match &args[0] {
                    FormulaNode::CellRef { row, .. } => FormulaValue::Number(*row as f64),
                    FormulaNode::RangeRef { start_row, .. } => FormulaValue::Number(*start_row as f64),
                    _ => FormulaValue::Error("#VALUE!".to_string()),
                }
            }
        }
        _ => FormulaValue::Error(format!("#NAME?")),
    }
}

// ========== SUMIF 辅助函数 ==========

/// 从 AST 节点提取范围值（支持 RangeRef 和普通表达式）
fn extract_range_values_from_node(arg: &FormulaNode, sheet: &SheetData, eval_pos: (u32, u32)) -> Vec<FormulaValue> {
    match arg {
        FormulaNode::RangeRef { start_col, start_row, end_col, end_row } => {
            collect_range_values(sheet, *start_col, *start_row, *end_col, *end_row)
        }
        _ => vec![eval_node(arg, sheet, eval_pos)],
    }
}

/// 判断单元格值是否匹配 SUMIF 条件
/// 支持数值/字符串相等，以及 ">=", "<=", "<>", ">", "<", "=" 前缀比较
fn matches_criteria(value: &FormulaValue, criteria: &str) -> bool {
    // 比较运算符前缀
    if let Some(rest) = criteria.strip_prefix(">=") {
        return compare_value_to_str(value, rest, |a, b| a >= b);
    }
    if let Some(rest) = criteria.strip_prefix("<=") {
        return compare_value_to_str(value, rest, |a, b| a <= b);
    }
    if let Some(rest) = criteria.strip_prefix("<>") {
        return compare_value_to_str(value, rest, |a, b| a != b);
    }
    if let Some(rest) = criteria.strip_prefix(">") {
        return compare_value_to_str(value, rest, |a, b| a > b);
    }
    if let Some(rest) = criteria.strip_prefix("<") {
        return compare_value_to_str(value, rest, |a, b| a < b);
    }
    if let Some(rest) = criteria.strip_prefix("=") {
        return compare_value_to_str(value, rest, |a, b| a == b);
    }
    // 直接相等比较
    let val_str = val_to_string(value);
    val_str.eq_ignore_ascii_case(criteria)
}

/// 数值比较辅助：尝试将值和阈值转为数字比较，否则字符串比较
fn compare_value_to_str(value: &FormulaValue, threshold_str: &str, cmp: fn(f64, f64) -> bool) -> bool {
    if let Some(n) = value.as_number() {
        if let Ok(t) = threshold_str.parse::<f64>() {
            return cmp(n, t);
        }
    }
    let val_str = val_to_string(value);
    cmp_str(&val_str, threshold_str, cmp)
}

fn cmp_str(a: &str, b: &str, cmp: fn(f64, f64) -> bool) -> bool {
    match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(an), Ok(bn)) => cmp(an, bn),
        _ => a.eq_ignore_ascii_case(b),
    }
}

// ========== 依赖分析和拓扑排序 ==========

/// 从 AST 中提取所有被引用的单元格
fn extract_dependencies(node: &FormulaNode) -> HashSet<(u32, u32)> {
    let mut deps = HashSet::new();
    match node {
        FormulaNode::CellRef { col, row } => { deps.insert((*row, *col)); }
        FormulaNode::RangeRef { start_col, start_row, end_col, end_row } => {
            for r in *start_row..=*end_row {
                for c in *start_col..=*end_col {
                    deps.insert((r, c));
                }
            }
        }
        FormulaNode::BinaryOp { left, right, .. } => {
            deps.extend(extract_dependencies(left));
            deps.extend(extract_dependencies(right));
        }
        FormulaNode::UnaryOp { operand, .. } => {
            deps.extend(extract_dependencies(operand));
        }
        FormulaNode::Function { args, .. } => {
            for arg in args { deps.extend(extract_dependencies(arg)); }
        }
        _ => {}
    }
    deps
}

// ========== 顶层 API ==========

/// 解析所有公式单元格，返回 (AST表, 公式位置集合, 正向依赖, 反向依赖)
fn build_formula_graph(sheet: &SheetData) -> (
    HashMap<(u32, u32), FormulaNode>,   // ASTs
    HashSet<(u32, u32)>,                 // formula_positions
    HashMap<(u32, u32), HashSet<(u32, u32)>>, // forward: cell -> its formula deps
    HashMap<(u32, u32), HashSet<(u32, u32)>>, // reverse: any cell -> formula cells depending on it
) {
    let formula_cells: HashMap<(u32, u32), FormulaNode> = sheet.cells.iter()
        .filter(|(_, cell)| !cell.formula.is_empty())
        .filter_map(|(&key, cell)| {
            parse_formula(&cell.formula).ok().map(|ast| (key, ast))
        })
        .collect();

    let formula_positions: HashSet<(u32, u32)> = formula_cells.keys().copied().collect();

    let mut forward_deps: HashMap<(u32, u32), HashSet<(u32, u32)>> = HashMap::new();
    let mut reverse_deps: HashMap<(u32, u32), HashSet<(u32, u32)>> = HashMap::new();

    for (&pos, ast) in &formula_cells {
        let deps = extract_dependencies(ast);
        // 正向依赖：只保留公式间依赖
        let formula_deps: HashSet<(u32, u32)> = deps.intersection(&formula_positions).copied().collect();
        forward_deps.insert(pos, formula_deps);

        // 反向依赖：任何被引用的单元格 -> 引用它的公式单元格
        for &dep in &deps {
            reverse_deps.entry(dep).or_default().insert(pos);
        }
    }

    (formula_cells, formula_positions, forward_deps, reverse_deps)
}

/// 对给定的公式单元格集合进行拓扑排序并求值
fn topo_eval(
    sheet: &mut SheetData,
    formula_cells: &HashMap<(u32, u32), FormulaNode>,
    cells_to_eval: &HashSet<(u32, u32)>,
    forward_deps: &HashMap<(u32, u32), HashSet<(u32, u32)>>,
    reverse_deps: &HashMap<(u32, u32), HashSet<(u32, u32)>>,
) {
    // 在待求值子集中进行拓扑排序
    let mut in_deg: HashMap<(u32, u32), u32> = HashMap::new();
    for &pos in cells_to_eval {
        let deps_in_subset = forward_deps.get(&pos)
            .map(|d| d.intersection(cells_to_eval).count())
            .unwrap_or(0);
        in_deg.insert(pos, deps_in_subset as u32);
    }

    let mut queue: VecDeque<(u32, u32)> = in_deg.iter()
        .filter(|(_, &d)| d == 0)
        .map(|(&pos, _)| pos)
        .collect();

    let mut eval_order = Vec::new();
    while let Some(pos) = queue.pop_front() {
        eval_order.push(pos);
        // 通过反向依赖表直接找到依赖 pos 的公式单元格
        if let Some(dependents) = reverse_deps.get(&pos) {
            for &dep in dependents {
                if !cells_to_eval.contains(&dep) { continue; }
                if let Some(d) = in_deg.get_mut(&dep) {
                    *d -= 1;
                    if *d == 0 { queue.push_back(dep); }
                }
            }
        }
    }

    // 按拓扑顺序求值（传入公式所在单元格位置，供 ROW() 等函数使用）
    for &pos in &eval_order {
        if let Some(ast) = formula_cells.get(&pos) {
            let result = eval_node(ast, sheet, pos);
            if let Some(cell) = sheet.cells.get_mut(&pos) {
                cell.value = result.to_display();
            }
        }
    }

    // 未处理的单元格（循环依赖）标记为 #CIRC!
    let processed: HashSet<(u32, u32)> = eval_order.into_iter().collect();
    for &pos in cells_to_eval {
        if !processed.contains(&pos) {
            if let Some(cell) = sheet.cells.get_mut(&pos) {
                cell.value = "#CIRC!".to_string();
            }
        }
    }
}

/// 求值工作表中的所有公式，将结果写入 cell.value
/// 用于初始加载或公式本身发生变更时
pub fn evaluate_sheet(sheet: &mut SheetData) {
    let (formula_cells, _, forward_deps, reverse_deps) = build_formula_graph(sheet);
    if formula_cells.is_empty() { return; }

    let all_cells: HashSet<(u32, u32)> = formula_cells.keys().copied().collect();
    topo_eval(sheet, &formula_cells, &all_cells, &forward_deps, &reverse_deps);
}

/// 增量求值：仅重新计算受影响的公式单元格
/// 当某个单元格的值发生变化时，只重新计算直接或间接依赖该单元格的公式
///
/// # 参数
/// * `sheet` - 工作表数据
/// * `changed_row` - 发生变化的单元格行号
/// * `changed_col` - 发生变化的单元格列号
pub fn evaluate_dependents(sheet: &mut SheetData, changed_row: u32, changed_col: u32) {
    let (formula_cells, _, forward_deps, reverse_deps) = build_formula_graph(sheet);
    if formula_cells.is_empty() { return; }

    let changed = (changed_row, changed_col);

    // BFS 查找所有受影响的公式单元格
    let mut affected: HashSet<(u32, u32)> = HashSet::new();
    let mut queue: VecDeque<(u32, u32)> = VecDeque::new();

    // 从变更单元格出发，找到直接依赖它的公式单元格
    if let Some(direct_deps) = reverse_deps.get(&changed) {
        for &dep in direct_deps {
            if affected.insert(dep) {
                queue.push_back(dep);
            }
        }
    }

    // 沿公式链传播
    while let Some(pos) = queue.pop_front() {
        if let Some(next_deps) = reverse_deps.get(&pos) {
            for &dep in next_deps {
                if affected.insert(dep) {
                    queue.push_back(dep);
                }
            }
        }
    }

    if affected.is_empty() { return; }

    topo_eval(sheet, &formula_cells, &affected, &forward_deps, &reverse_deps);
}

// ========== 单元格引用解析工具 ==========

/// 解析用户输入的单元格引用字符串（如 "A1"）为 (col, row)
#[allow(dead_code)]
pub fn parse_cell_ref_input(s: &str) -> Option<(u32, u32)> {
    parse_cell_ref_str(s).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::excel::reader::{CellData, SheetData};

    fn make_sheet(cells: Vec<((u32, u32), &str)>) -> SheetData {
        let mut sheet = SheetData::new("Test".to_string());
        for ((row, col), value) in cells {
            sheet.cells.insert((row, col), CellData {
                value: value.to_string(),
                formula: String::new(),
                ..CellData::default()
            });
        }
        sheet
    }

    fn make_formula_sheet(cells: Vec<((u32, u32), &str, &str)>) -> SheetData {
        let mut sheet = SheetData::new("Test".to_string());
        for ((row, col), value, formula) in cells {
            sheet.cells.insert((row, col), CellData {
                value: value.to_string(),
                formula: formula.to_string(),
                ..CellData::default()
            });
        }
        sheet
    }

    /// 创建带日期格式的工作表
    /// cells: ((row, col), value, number_format)
    fn make_date_sheet(cells: Vec<((u32, u32), &str, &str)>) -> SheetData {
        let mut sheet = SheetData::new("Test".to_string());
        for ((row, col), value, number_format) in cells {
            sheet.cells.insert((row, col), CellData {
                value: value.to_string(),
                number_format: Some(number_format.to_string()),
                ..CellData::default()
            });
        }
        sheet
    }

    #[test]
    fn test_eval_date_cell_subtraction() {
        // B5(row=5, col=2) 是日期 "2026/02/06"，B7 公式 =IF(B5="","",B5-TODAY())
        let mut sheet = make_date_sheet(vec![
            ((5, 2), "2026/02/06", "yyyy/m/d"),
            ((7, 2), "", "General"),
        ]);
        sheet.cells.get_mut(&(7, 2)).unwrap().formula = r#"IF(B5="","",B5-TODAY())"#.to_string();
        sheet.max_row = 7;
        sheet.max_col = 2;
        evaluate_sheet(&mut sheet);
        let result = &sheet.cells.get(&(7, 2)).unwrap().value;
        // B5 序列号 45858 减去 TODAY 序列号，结果应为负数（未来日期 - 今天）
        // 不应返回 #VALUE!
        assert_ne!(result, "#VALUE!", "日期单元格减法不应返回 #VALUE!");
        let n: f64 = result.parse().expect("结果应为数字");
        // 2026/02/06 减今天应为一个较大的负数（约 -250 左右）
        assert!(n < 0.0, "2026/02/06 减今天应为负数，实际: {}", n);
    }

    #[test]
    fn test_parse_number() {
        let ast = parse_formula("=42").unwrap();
        assert!(matches!(ast, FormulaNode::Number(42.0)));
    }

    #[test]
    fn test_parse_arithmetic() {
        let ast = parse_formula("=1+2*3").unwrap();
        match ast {
            FormulaNode::BinaryOp { op: BinOp::Add, .. } => {}
            other => panic!("Expected Add, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_cell_ref() {
        let ast = parse_formula("=A1+B2").unwrap();
        match ast {
            FormulaNode::BinaryOp { op: BinOp::Add, left, right } => {
                assert!(matches!(*left, FormulaNode::CellRef { col: 1, row: 1 }));
                assert!(matches!(*right, FormulaNode::CellRef { col: 2, row: 2 }));
            }
            other => panic!("Expected BinaryOp, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_range() {
        let ast = parse_formula("=SUM(A1:A10)").unwrap();
        match ast {
            FormulaNode::Function { name, args } => {
                assert_eq!(name, "SUM");
                assert_eq!(args.len(), 1);
                assert!(matches!(&args[0], FormulaNode::RangeRef { start_col: 1, start_row: 1, end_col: 1, end_row: 10 }));
            }
            other => panic!("Expected Function, got {:?}", other),
        }
    }

    #[test]
    fn test_eval_arithmetic() {
        let sheet = make_sheet(vec![]);
        let ast = parse_formula("=1+2*3").unwrap();
        let result = eval_node(&ast, &sheet, (1, 1));
        assert_eq!(result.to_display(), "7");
    }

    #[test]
    fn test_eval_cell_ref() {
        let sheet = make_sheet(vec![((1, 1), "10"), ((1, 2), "20")]);
        let ast = parse_formula("=A1+B1").unwrap();
        let result = eval_node(&ast, &sheet, (1, 1));
        assert_eq!(result.to_display(), "30");
    }

    #[test]
    fn test_eval_sum() {
        let sheet = make_sheet(vec![
            ((1, 1), "1"), ((2, 1), "2"), ((3, 1), "3"),
            ((4, 1), "4"), ((5, 1), "5"),
        ]);
        let ast = parse_formula("=SUM(A1:A5)").unwrap();
        let result = eval_node(&ast, &sheet, (1, 1));
        assert_eq!(result.to_display(), "15");
    }

    #[test]
    fn test_eval_if() {
        let sheet = make_sheet(vec![((1, 1), "10")]);
        let ast = parse_formula(r#"=IF(A1>5,"big","small")"#).unwrap();
        let result = eval_node(&ast, &sheet, (1, 1));
        assert_eq!(result.to_display(), "big");
    }

    #[test]
    fn test_eval_circular() {
        let mut sheet = make_formula_sheet(vec![
            ((1, 1), "0", "=B1"),
            ((1, 2), "0", "=A1"),
        ]);
        evaluate_sheet(&mut sheet);
        assert_eq!(sheet.cells.get(&(1, 1)).unwrap().value, "#CIRC!");
        assert_eq!(sheet.cells.get(&(1, 2)).unwrap().value, "#CIRC!");
    }

    #[test]
    fn test_eval_chain() {
        let mut sheet = make_formula_sheet(vec![
            ((1, 1), "5", ""),
            ((1, 2), "0", "=A1+1"),
            ((1, 3), "0", "=B1+1"),
        ]);
        evaluate_sheet(&mut sheet);
        assert_eq!(sheet.cells.get(&(1, 2)).unwrap().value, "6");
        assert_eq!(sheet.cells.get(&(1, 3)).unwrap().value, "7");
    }

    #[test]
    fn test_eval_div_zero() {
        let sheet = make_sheet(vec![]);
        let ast = parse_formula("=1/0").unwrap();
        let result = eval_node(&ast, &sheet, (1, 1));
        assert_eq!(result.to_display(), "#DIV/0!");
    }

    #[test]
    fn test_eval_concatenate() {
        let sheet = make_sheet(vec![((1, 1), "hello"), ((1, 2), "world")]);
        let ast = parse_formula("=CONCATENATE(A1,\" \",B1)").unwrap();
        let result = eval_node(&ast, &sheet, (1, 1));
        assert_eq!(result.to_display(), "hello world");
    }

    #[test]
    fn test_evaluate_dependents() {
        // B1=SUM(A1:A5), C1=B1+1
        let mut sheet = make_formula_sheet(vec![
            ((1, 1), "1", ""),   // A1=1
            ((2, 1), "2", ""),   // A2=2
            ((3, 1), "3", ""),   // A3=3
            ((4, 1), "4", ""),   // A4=4
            ((5, 1), "5", ""),   // A5=5
            ((1, 2), "15", "=SUM(A1:A5)"),  // B1=SUM(A1:A5)
            ((1, 3), "16", "=B1+1"),         // C1=B1+1
        ]);
        // 先全量求值
        evaluate_sheet(&mut sheet);
        assert_eq!(sheet.cells.get(&(1, 2)).unwrap().value, "15");
        assert_eq!(sheet.cells.get(&(1, 3)).unwrap().value, "16");

        // 修改 A1 为 10，增量求值
        sheet.cells.get_mut(&(1, 1)).unwrap().value = "10".to_string();
        evaluate_dependents(&mut sheet, 1, 1);

        // A1=10 → SUM(A1:A5)=10+2+3+4+5=24 → C1=24+1=25
        assert_eq!(sheet.cells.get(&(1, 2)).unwrap().value, "24");
        assert_eq!(sheet.cells.get(&(1, 3)).unwrap().value, "25");
    }

    #[test]
    fn test_evaluate_dependents_unaffected() {
        // A1=1, B1=A1+1, C1=99 (no formula)
        // D1=SUM(E1:E5), E1=10
        let mut sheet = make_formula_sheet(vec![
            ((1, 1), "1", ""),          // A1
            ((1, 2), "2", "=A1+1"),     // B1=A1+1
            ((1, 3), "99", ""),         // C1=99 (plain)
            ((1, 4), "0", "=SUM(E1:E5)"), // D1=SUM(E1:E5)
            ((1, 5), "10", ""),         // E1=10
        ]);
        evaluate_sheet(&mut sheet);
        assert_eq!(sheet.cells.get(&(1, 2)).unwrap().value, "2");
        assert_eq!(sheet.cells.get(&(1, 4)).unwrap().value, "10");

        // 修改 E1 为 20，B1 不应受影响
        sheet.cells.get_mut(&(1, 5)).unwrap().value = "20".to_string();
        evaluate_dependents(&mut sheet, 1, 5);

        // D1 应更新，B1 不变
        assert_eq!(sheet.cells.get(&(1, 4)).unwrap().value, "20");
        assert_eq!(sheet.cells.get(&(1, 2)).unwrap().value, "2");
    }

    #[test]
    fn test_parse_dollar_cell_ref() {
        // $C$15 格式
        let ast = parse_formula("=SUM($C$15:$C$45)").unwrap();
        match ast {
            FormulaNode::Function { name, args } => {
                assert_eq!(name, "SUM");
                assert!(matches!(&args[0], FormulaNode::RangeRef { start_col: 3, start_row: 15, end_col: 3, end_row: 45 }));
            }
            other => panic!("Expected Function, got {:?}", other),
        }

        // $A$1 格式
        let ast = parse_formula("=$A$1+B1").unwrap();
        match ast {
            FormulaNode::BinaryOp { left, .. } => {
                assert!(matches!(*left, FormulaNode::CellRef { col: 1, row: 1 }));
            }
            other => panic!("Expected BinaryOp, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_xlfn_prefix() {
        // _xlfn.IFS 应被预处理为 IFS
        let ast = parse_formula(r#"=IF(A1="","",_xlfn.IFS(A1<225,"low",A1>=225,"high"))"#).unwrap();
        match ast {
            FormulaNode::Function { name, .. } => {
                assert_eq!(name, "IF");
            }
            other => panic!("Expected Function, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_at_prefix() {
        // @ 前缀是 Excel 隐式交叉运算符，应被忽略
        let ast = parse_formula("@IF(A1>0,1,0)").unwrap();
        // 确认解析成功
        match ast {
            FormulaNode::Function { ref name, .. } => assert_eq!(name, "IF"),
            other => panic!("Expected Function, got {:?}", other),
        }
    }

    #[test]
    fn test_eval_ifs() {
        let sheet = make_sheet(vec![((1, 1), "100")]);
        let ast = parse_formula(r#"=IFS(A1<50,"small",A1<200,"medium",A1>=200,"large")"#).unwrap();
        let result = eval_node(&ast, &sheet, (1, 1));
        assert_eq!(result.to_display(), "medium");
    }

    #[test]
    fn test_dollar_ref_chain() {
        // B13=SUM($C$15:$C$45), 修改 C20 应触发 B13 重算
        let mut sheet = make_formula_sheet(vec![
            ((15, 3), "10", ""),   // C15=10
            ((16, 3), "20", ""),   // C16=20
            ((17, 3), "30", ""),   // C17=30
            ((20, 3), "40", ""),   // C20=40
            ((13, 2), "0", "SUM($C$15:$C$45)"), // B13=SUM($C$15:$C$45)
        ]);
        evaluate_sheet(&mut sheet);
        assert_eq!(sheet.cells.get(&(13, 2)).unwrap().value, "100"); // 10+20+30+40

        // 修改 C20 为 80，增量求值
        sheet.cells.get_mut(&(20, 3)).unwrap().value = "80".to_string();
        evaluate_dependents(&mut sheet, 20, 3);
        assert_eq!(sheet.cells.get(&(13, 2)).unwrap().value, "140"); // 10+20+30+80
    }

    #[test]
    fn test_eval_today() {
        let sheet = make_sheet(vec![]);
        let ast = parse_formula("=TODAY()").unwrap();
        let result = eval_node(&ast, &sheet, (1, 1));
        // TODAY() 应返回一个大于 45000 的数字（2023年之后的日期序列号）
        match result {
            FormulaValue::Number(n) => assert!(n > 45000.0, "TODAY serial should be > 45000, got {}", n),
            other => panic!("Expected Number, got {:?}", other),
        }
    }

    #[test]
    fn test_eval_row() {
        let sheet = make_sheet(vec![]);

        // ROW() 返回当前公式所在行号
        let ast = parse_formula("=ROW()").unwrap();
        let result = eval_node(&ast, &sheet, (5, 3)); // 第5行第3列
        assert_eq!(result.to_display(), "5");

        // ROW(A10) 返回引用的行号
        let ast = parse_formula("=ROW(A10)").unwrap();
        let result = eval_node(&ast, &sheet, (1, 1));
        assert_eq!(result.to_display(), "10");
    }

    // ========== adjust_formula_rows 测试 ==========

    #[test]
    fn test_adjust_formula_rows_basic() {
        // 在第20行上方插入1行 → 行号 >=20 的相对引用 +1
        assert_eq!(
            adjust_formula_rows("=SUM(B15:B199)", 20, 1),
            "=SUM(B15:B200)"
        );
    }

    #[test]
    fn test_adjust_formula_rows_absolute_row() {
        // 绝对行引用 $15, $199 不应被偏移
        assert_eq!(
            adjust_formula_rows("=SUM(B$15:B$199)", 20, 1),
            "=SUM(B$15:B$199)"
        );
    }

    #[test]
    fn test_adjust_formula_rows_mixed_ref() {
        // 绝对列 + 相对行：$D15:$D199 → $D15:$D200
        assert_eq!(
            adjust_formula_rows("=SUM($D15:$D199)", 20, 1),
            "=SUM($D15:$D200)"
        );
    }

    #[test]
    fn test_adjust_formula_rows_absolute_col_absolute_row() {
        // 完全绝对引用 $D$15:$D$199 → 不变
        assert_eq!(
            adjust_formula_rows("=SUM($D$15:$D$199)", 20, 1),
            "=SUM($D$15:$D$199)"
        );
    }

    #[test]
    fn test_adjust_formula_rows_below_threshold() {
        // 行号 < threshold 的相对引用不变
        assert_eq!(
            adjust_formula_rows("=SUM(A1:A19)", 20, 1),
            "=SUM(A1:A19)"
        );
    }

    #[test]
    fn test_adjust_formula_rows_straddle_threshold() {
        // 起始行 < threshold，结束行 >= threshold → 只偏移结束行
        assert_eq!(
            adjust_formula_rows("=SUM(A15:A25)", 20, 1),
            "=SUM(A15:A26)"
        );
    }

    #[test]
    fn test_adjust_formula_rows_multiple_refs() {
        // 公式中多个引用
        assert_eq!(
            adjust_formula_rows("=A20+B30*C10", 20, 1),
            "=A21+B31*C10"
        );
    }

    #[test]
    fn test_adjust_formula_rows_string_literal() {
        // 字符串内的引用不应被修改
        assert_eq!(
            adjust_formula_rows(r#"=IF(A1="text","A20",B20)"#, 20, 1),
            r#"=IF(A1="text","A20",B21)"#
        );
    }

    #[test]
    fn test_adjust_formula_rows_negative_shift() {
        // 删除行：负偏移
        assert_eq!(
            adjust_formula_rows("=SUM(B15:B200)", 20, -1),
            "=SUM(B15:B199)"
        );
    }

    #[test]
    fn test_adjust_formula_rows_empty() {
        assert_eq!(adjust_formula_rows("", 20, 1), "");
        assert_eq!(adjust_formula_rows("=SUM(A1:A10)", 20, 0), "=SUM(A1:A10)");
    }

    #[test]
    fn test_adjust_formula_rows_at_prefix() {
        // @ 前缀应保留
        assert_eq!(
            adjust_formula_rows("@SUM(A20:A30)", 20, 1),
            "@SUM(A21:A31)"
        );
    }
}
