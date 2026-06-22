//! 单元格填充柄（Fill Handle）填充逻辑。
//!
//! 根据源选区数据类型推断填充序列：
//! - **公式**：按相对引用平移（复用 [`crate::excel::formula::adjust_formula_columns`]
//!   / [`crate::excel::formula::adjust_formula_rows`]，绝对引用 `$` 不变）。
//! - **日期**：按天递增（步长由源序列推断，默认 1 天；结果经
//!   [`ExcelData::format_date`] 格式化）。
//! - **数字**：算术递增（步长由源序列推断，默认 1）；若源序列呈恒定比值则按等比扩展。
//! - **文本**：复制（按源序列取模重复）。
//!
//! 多列/多行源选区按"车道"独立填充：垂直填充时每列各自向下/上扩展，
//! 水平填充时每行各自向右/左扩展。目标格先克隆对应源格（保留字体/底色/边框/对齐/批注等格式），
//! 再覆写 value/formula/raw_number。

use crate::excel::formula::shift_formula_relative;
use crate::excel::reader::{CellData, ExcelData, SheetData};

#[derive(Clone, Copy, PartialEq)]
enum Axis {
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Formula,
    Date,
    Number,
    DateText,
    Text,
}

/// 取单元格的数值：优先 `raw_number`，其次把 `value` 解析为 `f64`。
fn cell_number(c: Option<&CellData>) -> Option<f64> {
    c.and_then(|cell| cell.raw_number.or_else(|| cell.value.trim().parse::<f64>().ok()))
}

/// 清理浮点累加噪声（如 0.1+0.2 的尾数）并格式化为字符串。
fn format_num(v: f64) -> String {
    if !v.is_finite() {
        return format!("{}", v);
    }
    let r = (v * 1e10).round() / 1e10;
    format!("{}", r)
}

// ========== 中文日期文本（如 "08月24号"、"8月24日"、"2024年8月24日"）==========
// 这类值通常是无 number_format 的纯文本，不会被 is_date_format/parse_date_string 识别，
// 故单独识别其模式并按天递增（与 Excel 一致）。

/// 当前年份（用于无年份日期文本的基准年，保证月/年末边界与闰年正确）。
fn current_year() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as f64 / 86400.0)
        .unwrap_or(0.0);
    // 1970-01-01 对应 Excel 序列号 25569
    ExcelData::serial_to_date(days + 25569.0).0
}

/// 解析出的日期文本：解析值 + 格式元数据（用于递增后按原样格式化）。
struct DateText {
    year: u32,
    month: u32,
    day: u32,
    has_year: bool,
    month_pad: bool, // 月是否两位（前导 0）
    day_pad: bool,   // 日/号是否两位
    suffix: char,    // '日' 或 '号'
}

/// 解析中文日期文本：`[YYYY年]?M月D(日|号)`。
///
/// 支持 `"08月24号"`、`"8月24日"`、`"2024年8月24日"` 等；要求末尾为 `日`/`号`。
/// 无年份时取当前年份。两位年份（`<100`）不予识别（歧义）。
fn parse_date_text(s: &str) -> Option<DateText> {
    let s = s.trim();
    let suffix = s.chars().last().filter(|&c| c == '日' || c == '号')?;
    let body = &s[..s.len() - suffix.len_utf8()];
    let body = body.replace("年", "/").replace("月", "/");
    let parts: Vec<&str> = body.split('/').filter(|p| !p.is_empty()).collect();
    let (year_str, month_str, day_str, has_year) = match parts.as_slice() {
        [y, m, d] => (*y, *m, *d, true),
        [m, d] => ("", *m, *d, false),
        _ => return None,
    };
    let month: u32 = month_str.parse().ok()?;
    let day: u32 = day_str.parse().ok()?;
    if month == 0 || month > 12 || day == 0 || day > 31 {
        return None;
    }
    let year = if has_year {
        let y: u32 = year_str.parse().ok()?;
        if y < 100 {
            return None; // 两位年份歧义，不识别
        }
        y
    } else {
        current_year()
    };
    Some(DateText {
        year,
        month,
        day,
        has_year,
        month_pad: month_str.chars().count() == 2,
        day_pad: day_str.chars().count() == 2,
        suffix,
    })
}

/// 按日期文本的原格式（年/前导零/后缀）格式化。
fn format_date_text(pat: &DateText, y: u32, m: u32, d: u32) -> String {
    let mut s = String::new();
    if pat.has_year {
        s.push_str(&y.to_string());
        s.push('年');
    }
    if pat.month_pad {
        s.push_str(&format!("{:02}", m));
    } else {
        s.push_str(&m.to_string());
    }
    s.push('月');
    if pat.day_pad {
        s.push_str(&format!("{:02}", d));
    } else {
        s.push_str(&d.to_string());
    }
    s.push(pat.suffix);
    s
}


/// 对目标区域执行填充。
///
/// - `src` = `(start_col, start_row, end_col, end_row)` 源选区（内部自动归一化）。
/// - `target` = `(col, row)` 拖拽结束格，决定填充轴向（垂直/水平）与方向（前/后）。
///
/// 返回 `(被覆盖目标格的原始数据, 是否含公式填充)`。原始数据用于撤销；
/// `has_formula` 提示调用方选择全表重算（`evaluate_sheet`）还是逐格增量重算（`evaluate_dependents`）。
pub fn apply_fill(
    sheet: &mut SheetData,
    src: (u32, u32, u32, u32),
    target: (u32, u32),
) -> (Vec<(u32, u32, Option<CellData>)>, bool) {
    let (sc0, sr0, sc1, sr1) = (
        src.0.min(src.2),
        src.1.min(src.3),
        src.0.max(src.2),
        src.1.max(src.3),
    );
    let (tcol, trow) = target;

    // 定轴与方向；target 落在源内则无操作
    let (axis, forward) = if trow > sr1 {
        (Axis::Vertical, true)
    } else if trow < sr0 {
        (Axis::Vertical, false)
    } else if tcol > sc1 {
        (Axis::Horizontal, true)
    } else if tcol < sc0 {
        (Axis::Horizontal, false)
    } else {
        return (Vec::new(), false);
    };

    let mut old_cells: Vec<(u32, u32, Option<CellData>)> = Vec::new();
    let mut has_formula = false;

    // 车道：垂直填充按列、水平填充按行
    let lanes: Vec<u32> = match axis {
        Axis::Vertical => (sc0..=sc1).collect(),
        Axis::Horizontal => (sr0..=sr1).collect(),
    };

    for lane in lanes {
        // 源格坐标（自然序：垂直=top→bottom，水平=left→right），元素为 (col, row)
        let src_pos: Vec<(u32, u32)> = match axis {
            Axis::Vertical => (sr0..=sr1).map(|r| (lane, r)).collect(),
            Axis::Horizontal => (sc0..=sc1).map(|c| (c, lane)).collect(),
        };
        let n = src_pos.len();
        let src_data: Vec<Option<CellData>> = src_pos
            .iter()
            .map(|(c, r)| sheet.get_cell(*r, *c).cloned())
            .collect();

        // 推断类型（首个非空源格）
        let first = src_data.iter().flatten().next();
        let kind = match first {
            Some(c) if !c.formula.is_empty() => Kind::Formula,
            Some(c) if c.number_format.as_deref().map(ExcelData::is_date_format).unwrap_or(false) => {
                Kind::Date
            }
            Some(c) if cell_number(Some(c)).is_some() => Kind::Number,
            Some(c) if parse_date_text(&c.value).is_some() => Kind::DateText,
            _ => Kind::Text,
        };
        if kind == Kind::Formula {
            has_formula = true;
        }

        // 目标格坐标（从源边缘向外，j=0 最近源），元素为 (col, row)
        let target_pos: Vec<(u32, u32)> = match (axis, forward) {
            (Axis::Vertical, true) => ((sr1 + 1)..=trow).map(|r| (lane, r)).collect(),
            (Axis::Vertical, false) => (trow..sr0).rev().map(|r| (lane, r)).collect(),
            (Axis::Horizontal, true) => ((sc1 + 1)..=tcol).map(|c| (c, lane)).collect(),
            (Axis::Horizontal, false) => (tcol..sc0).rev().map(|c| (c, lane)).collect(),
        };

        // 数值序列（数字/日期用）
        let vals: Vec<f64> = src_data
            .iter()
            .map(|c| cell_number(c.as_ref()).unwrap_or(0.0))
            .collect();
        // 日期文本序列号（DateText 用：中文 "M月D日/号" 文本 → 序列号）
        let dt_serials: Vec<f64> = if kind == Kind::DateText {
            src_data
                .iter()
                .map(|c| {
                    c.as_ref()
                        .and_then(|cell| parse_date_text(&cell.value))
                        .map(|p| ExcelData::date_to_serial(p.year, p.month, p.day))
                        .unwrap_or(0.0)
                })
                .collect()
        } else {
            Vec::new()
        };

        for (j, &(tc, tr)) in target_pos.iter().enumerate() {
            let pidx = j % n;
            let (psc, psr) = src_pos[pidx];
            // 捕获旧值（撤销用）
            old_cells.push((tr, tc, sheet.get_cell(tr, tc).cloned()));
            // 新格：克隆 pattern 源格（带格式），再覆写 value/formula/raw_number
            let mut new_cell = src_data[pidx].clone().unwrap_or_default();

            match kind {
                Kind::Formula => {
                    let row_off = tr as i32 - psr as i32;
                    let col_off = tc as i32 - psc as i32;
                    // 复制语义：仅平移相对引用，绝对（$）不变
                    new_cell.formula = shift_formula_relative(&new_cell.formula, col_off, row_off);
                    new_cell.value.clear();
                    new_cell.raw_number = None;
                }
                Kind::Date => {
                    let d = detect_step(&vals).unwrap_or(1.0);
                    let base = if forward { vals[n - 1] } else { vals[0] };
                    let signed_d = if forward { d } else { -d };
                    let serial = base + (j as f64 + 1.0) * signed_d;
                    let fmt = new_cell.number_format.clone().unwrap_or_default();
                    new_cell.value = ExcelData::format_date(serial, &fmt);
                    new_cell.raw_number = Some(serial);
                    new_cell.formula.clear();
                }
                Kind::Number => {
                    let (step, ratio, geom) = detect_number_pattern(&vals);
                    let base = if forward { vals[n - 1] } else { vals[0] };
                    let val = if geom {
                        let r = if forward { ratio } else { 1.0 / ratio };
                        base * r.powi((j as i32) + 1)
                    } else {
                        let signed = if forward { step } else { -step };
                        base + (j as f64 + 1.0) * signed
                    };
                    new_cell.value = format_num(val);
                    new_cell.raw_number = Some(val);
                    new_cell.formula.clear();
                }
                Kind::DateText => {
                    // 中文日期文本（"08月24号" 等）按天递增；步长由源日期序列推断（≥2 格），否则 1 天
                    if let Some(pat) = parse_date_text(&new_cell.value) {
                        let d = detect_step(&dt_serials).unwrap_or(1.0);
                        let base = if forward { dt_serials[n - 1] } else { dt_serials[0] };
                        let signed_d = if forward { d } else { -d };
                        let serial = base + (j as f64 + 1.0) * signed_d;
                        let (yy, mm, dd) = ExcelData::serial_to_date(serial);
                        // 按源格原格式（年/前导零/后缀）输出
                        new_cell.value = format_date_text(&pat, yy, mm, dd);
                        new_cell.raw_number = Some(serial);
                    }
                    new_cell.formula.clear();
                }
                Kind::Text => {
                    // value 已由克隆带入；清公式/数值
                    new_cell.formula.clear();
                    new_cell.raw_number = None;
                }
            }
            sheet.cells.insert((tr, tc), new_cell);
        }
    }

    (old_cells, has_formula)
}

/// 从数值序列推断算术步长（单元素返回 `None`，多元素取首两个之差）。
fn detect_step(vals: &[f64]) -> Option<f64> {
    if vals.len() < 2 {
        return None;
    }
    Some(vals[1] - vals[0])
}

/// 推断数字序列：返回 `(步长, 比值, 是否等比)`。
///
/// 与 Excel 默认一致——**优先等差**：序列呈恒定差时按等差扩展（如 `2,4 → 6,8`）。
/// 仅当序列**不**呈恒定差、却呈恒定比值（且无 0、比值 ≠ 1）时才判为等比
/// （如 `2,4,8 → 16,32`）。单元素默认步长 1。
fn detect_number_pattern(vals: &[f64]) -> (f64, f64, bool) {
    if vals.len() >= 2 {
        let d0 = vals[1] - vals[0];
        let arith = vals.windows(2).all(|w| (w[1] - w[0] - d0).abs() < 1e-9);
        if arith {
            return (d0, 0.0, false);
        }
        if vals.iter().all(|v| v.abs() > 1e-12) {
            let r = vals[1] / vals[0];
            let geom = vals.windows(2).all(|w| (w[1] / w[0] - r).abs() < 1e-9);
            if geom && (r - 1.0).abs() > 1e-9 {
                return (0.0, r, true);
            }
        }
        // 既非严格等差也非等比：退化为首差等差
        return (d0, 0.0, false);
    }
    (1.0, 0.0, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::excel::reader::CellData;
    use std::collections::HashMap;

    /// 构造一个最小可用的 SheetData（仅 cells + 边界）。
    fn empty_sheet() -> SheetData {
        SheetData {
            name: "test".to_string(),
            cells: HashMap::new(),
            merged_cells: Vec::new(),
            max_row: 100,
            max_col: 100,
            column_widths: HashMap::new(),
            row_heights: HashMap::new(),
            frozen_rows: 0,
            frozen_cols: 0,
            data_validations: Vec::new(),
            merge_index: HashMap::new(),
            conditional_rules: Vec::new(),
            cf_dirty: false,
        }
    }

    fn put(sheet: &mut SheetData, col: u32, row: u32, value: &str) {
        let mut c = CellData::default();
        c.value = value.to_string();
        c.raw_number = value.trim().parse::<f64>().ok();
        sheet.cells.insert((row, col), c);
    }

    fn put_cell(sheet: &mut SheetData, col: u32, row: u32, cell: CellData) {
        sheet.cells.insert((row, col), cell);
    }

    fn val(sheet: &SheetData, col: u32, row: u32) -> String {
        sheet.get_cell(row, col).map(|c| c.value.clone()).unwrap_or_default()
    }

    fn formula(sheet: &SheetData, col: u32, row: u32) -> String {
        sheet.get_cell(row, col).map(|c| c.formula.clone()).unwrap_or_default()
    }

    #[test]
    fn fill_single_number_increments() {
        let mut s = empty_sheet();
        put(&mut s, 1, 1, "5");
        // 源 A1，向下填到 A4
        let _ = apply_fill(&mut s, (1, 1, 1, 1), (1, 4));
        assert_eq!(val(&s, 1, 2), "6");
        assert_eq!(val(&s, 1, 3), "7");
        assert_eq!(val(&s, 1, 4), "8");
    }

    #[test]
    fn fill_arithmetic_sequence() {
        let mut s = empty_sheet();
        put(&mut s, 1, 1, "1");
        put(&mut s, 1, 2, "2");
        put(&mut s, 1, 3, "3");
        // 源 A1:A3，向下填到 A6 → 4,5,6
        let _ = apply_fill(&mut s, (1, 1, 1, 3), (1, 6));
        assert_eq!(val(&s, 1, 4), "4");
        assert_eq!(val(&s, 1, 5), "5");
        assert_eq!(val(&s, 1, 6), "6");
    }

    #[test]
    fn fill_arithmetic_step2() {
        let mut s = empty_sheet();
        put(&mut s, 1, 1, "2");
        put(&mut s, 1, 2, "4");
        let _ = apply_fill(&mut s, (1, 1, 1, 2), (1, 4));
        assert_eq!(val(&s, 1, 3), "6");
        assert_eq!(val(&s, 1, 4), "8");
    }

    #[test]
    fn fill_geometric_sequence() {
        let mut s = empty_sheet();
        put(&mut s, 1, 1, "2");
        put(&mut s, 1, 2, "4");
        put(&mut s, 1, 3, "8");
        // 2,4,8：非恒定差、恒定比值 2 → 等比 → 16,32
        let _ = apply_fill(&mut s, (1, 1, 1, 3), (1, 5));
        assert_eq!(val(&s, 1, 4), "16");
        assert_eq!(val(&s, 1, 5), "32");
    }

    #[test]
    fn fill_horizontal() {
        let mut s = empty_sheet();
        put(&mut s, 1, 1, "10");
        // 源 A1，向右填到 D1 → 11,12,13
        let _ = apply_fill(&mut s, (1, 1, 1, 1), (4, 1));
        assert_eq!(val(&s, 2, 1), "11");
        assert_eq!(val(&s, 3, 1), "12");
        assert_eq!(val(&s, 4, 1), "13");
    }

    #[test]
    fn fill_backward_up() {
        let mut s = empty_sheet();
        put(&mut s, 1, 3, "3");
        // 源 A3，向上填到 A1 → 2,1
        let _ = apply_fill(&mut s, (1, 3, 1, 3), (1, 1));
        assert_eq!(val(&s, 1, 2), "2");
        assert_eq!(val(&s, 1, 1), "1");
    }

    #[test]
    fn fill_text_copies() {
        let mut s = empty_sheet();
        put(&mut s, 1, 1, "hello");
        let _ = apply_fill(&mut s, (1, 1, 1, 1), (1, 3));
        assert_eq!(val(&s, 1, 2), "hello");
        assert_eq!(val(&s, 1, 3), "hello");
    }

    #[test]
    fn fill_formula_shifts_relative_only() {
        let mut s = empty_sheet();
        let mut c = CellData::default();
        c.formula = "=A1+B1".to_string();
        put_cell(&mut s, 3, 1, c); // C1 = =A1+B1
        let (old, has_formula) = apply_fill(&mut s, (3, 1, 3, 1), (3, 3));
        // 向下填到 C3：相对引用随行偏移
        assert_eq!(formula(&s, 3, 2), "=A2+B2");
        assert_eq!(formula(&s, 3, 3), "=A3+B3");
        assert!(has_formula);
        assert!(old.iter().any(|(r, c, _)| *r == 2 && *c == 3));
    }

    #[test]
    fn fill_formula_keeps_absolute() {
        let mut s = empty_sheet();
        let mut c = CellData::default();
        c.formula = "=$A$1+B1".to_string();
        put_cell(&mut s, 3, 1, c);
        let _ = apply_fill(&mut s, (3, 1, 3, 1), (3, 2));
        // $A$1 不变，B1 → B2
        assert_eq!(formula(&s, 3, 2), "=$A$1+B2");
    }

    #[test]
    fn fill_returns_old_cells_for_undo() {
        let mut s = empty_sheet();
        put(&mut s, 1, 1, "1");
        put(&mut s, 1, 2, "existing"); // 将被覆盖
        let (old, _) = apply_fill(&mut s, (1, 1, 1, 1), (1, 3));
        // old_cells 记录 A2 原值 "existing"
        let a2_old = old.iter().find(|(r, c, _)| *r == 2 && *c == 1);
        assert_eq!(a2_old.unwrap().2.as_ref().unwrap().value, "existing");
    }

    #[test]
    fn fill_date_text_increments_hao() {
        // "08月24号" → "08月25号"（报修用例）
        let mut s = empty_sheet();
        put(&mut s, 1, 45, "08月24号");
        let _ = apply_fill(&mut s, (1, 45, 1, 45), (1, 46));
        assert_eq!(val(&s, 1, 46), "08月25号");
    }

    #[test]
    fn fill_date_text_increments_multi() {
        let mut s = empty_sheet();
        put(&mut s, 1, 1, "8月24日");
        let _ = apply_fill(&mut s, (1, 1, 1, 1), (1, 4));
        assert_eq!(val(&s, 1, 2), "8月25日");
        assert_eq!(val(&s, 1, 3), "8月26日");
        assert_eq!(val(&s, 1, 4), "8月27日");
    }

    #[test]
    fn fill_date_text_with_year() {
        let mut s = empty_sheet();
        put(&mut s, 1, 1, "2024年8月24日");
        let _ = apply_fill(&mut s, (1, 1, 1, 1), (1, 2));
        assert_eq!(val(&s, 1, 2), "2024年8月25日");
    }

    #[test]
    fn fill_date_text_month_boundary() {
        // 月末跨月：前导零与后缀保持原格式
        let mut s = empty_sheet();
        put(&mut s, 1, 1, "08月31号");
        let _ = apply_fill(&mut s, (1, 1, 1, 1), (1, 2));
        assert_eq!(val(&s, 1, 2), "09月01号");
    }

    #[test]
    fn fill_plain_text_not_treated_as_date() {
        // 非日期文本仍按复制处理
        let mut s = empty_sheet();
        put(&mut s, 1, 1, "hello");
        let _ = apply_fill(&mut s, (1, 1, 1, 1), (1, 3));
        assert_eq!(val(&s, 1, 2), "hello");
        assert_eq!(val(&s, 1, 3), "hello");
    }
}
