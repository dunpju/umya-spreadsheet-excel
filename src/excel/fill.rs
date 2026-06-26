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
use std::collections::HashSet;

/// 双击填充柄自动填充时，单次写入单元格数的安全上限。
///
/// 双击自动填充的目标范围由「相邻连续数据」的边界决定，通常远小于此值；该上限仅作兜底，
/// 防止在异常超大表（如相邻列有数十万行连续数据）上单帧海量写入导致 UI 卡顿。
/// 超出时会把目标夹紧到此上限对应的行列数（见 [`compute_autofill_target`]）。
pub const AUTO_FILL_MAX_CELLS: u32 = 50_000;

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
    /// 数字型日期文本：无 number_format、非中文，但 `value` 形如 `2026/2/7`、`2026-01-26`
    /// （斜杠/连字符分隔、年开头）。按天递增，输出沿用源格式的分隔符与前导零。
    DateNum,
    Text,
}

/// 取单元格的数值：优先 `raw_number`，其次把 `value` 解析为 `f64`。
fn cell_number(c: Option<&CellData>) -> Option<f64> {
    c.and_then(|cell| cell.raw_number.or_else(|| cell.value.trim().parse::<f64>().ok()))
}

/// 该格是否是某个合并区域的"非左上角"部分。
///
/// 合并区域的值只存于左上角；其余部分（非左上角）无独立数据。填充时必须跳过这些位置，
/// 否则它们会被当作空/0 污染序列步长推断——例如 AJ1:AK1 合并值 18 水平填充时，
/// AK1（非左上角）为空→0，使序列 [18,0] 步长算成 -18，结果错误地变成 -18。
fn is_merged_part(sheet: &SheetData, col: u32, row: u32) -> bool {
    sheet
        .get_merged_range(col, row)
        .map_or(false, |mr| !mr.is_top_left(col, row))
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

// ========== 数字型日期（斜杠/连字符分隔，年开头，如 "2026/2/7"、"2026-01-26"）==========
// 这类值无 number_format、非中文，不会被 is_date_format/parse_date_text 识别，
// 故单独识别其模式并按天递增（与 Excel 一致）。与中文日期文本（DateText）对称。

/// 解析出的数字型日期：序列号 + 格式码。
///
/// `fmt` 记录分隔符与前导零（如 `yyyy/m/d`、`yyyy-mm-dd`），供递增后经
/// [`ExcelData::format_date`] 按源格原样回填。
struct NumericDate {
    serial: f64,
    fmt: String,
}

/// 解析数字型日期：`YYYY<M><D>`，分隔符为 `/` 或 `-`（不可混用）。
///
/// 仅识别**年开头**且年份为 4 位（≥1000）的日期，避免与 `m/d/yyyy`、两位年份等歧义。
/// 无效日期（如 `2026/2/30`）经序列号往返校验后不予识别。
fn parse_numeric_date(s: &str) -> Option<NumericDate> {
    let s = s.trim();
    let sep = if s.contains('/') && !s.contains('-') {
        '/'
    } else if s.contains('-') && !s.contains('/') {
        '-'
    } else {
        return None; // 不含分隔符或混用 → 非数字日期
    };
    let parts: Vec<&str> = s.split(sep).collect();
    if parts.len() != 3 {
        return None;
    }
    let year: u32 = parts[0].parse().ok()?;
    if parts[0].len() != 4 || year < 1000 {
        return None; // 年必须 4 位且在最前
    }
    let month: u32 = parts[1].parse().ok()?;
    let day: u32 = parts[2].parse().ok()?;
    if month == 0 || month > 12 || day == 0 || day > 31 {
        return None;
    }
    let serial = ExcelData::date_to_serial(year, month, day);
    // 往返校验：拒绝 2026/2/30 这类不存在的日期（溢出到下月）
    if ExcelData::serial_to_date(serial) != (year, month, day) {
        return None;
    }
    let m_code = if parts[1].len() == 2 { "mm" } else { "m" };
    let d_code = if parts[2].len() == 2 { "dd" } else { "d" };
    let fmt = format!("yyyy{sep}{m_code}{sep}{d_code}");
    Some(NumericDate { serial, fmt })
}

/// 对目标区域执行填充（同步，直接写入 sheet）。
///
/// - `src` = `(start_col, start_row, end_col, end_row)` 源选区（内部自动归一化）。
/// - `target` = `(col, row)` 拖拽结束格，决定填充轴向（垂直/水平）与方向（前/后）。
///
/// 返回 `(被覆盖目标格的原始数据, 是否含公式填充)`。原始数据用于撤销；
/// `has_formula` 提示调用方选择重算策略：含公式走全量重算（`evaluate_sheet`），
/// 仅值走**批量**增量重算（`evaluate_dependents_many`，一次建图；勿逐格调 `evaluate_dependents`，
/// 大表上为 K × O(2M) 会卡顿）。
#[cfg(test)]
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

    // 定轴与方向：按行列偏移量取大者作为填充方向（与 Excel 一致）。
    // 优先取偏移更大的方向，避免填充柄微小垂直误触（填充柄 2px 伸出选区底部、
    // 合并单元格场景下 cell_at 落入下一行）将水平拖拽误判为垂直填充。
    let row_offset = if trow > sr1 {
        trow - sr1
    } else if trow < sr0 {
        sr0 - trow
    } else {
        0
    };
    let col_offset = if tcol > sc1 {
        tcol - sc1
    } else if tcol < sc0 {
        sc0 - tcol
    } else {
        0
    };
    if row_offset == 0 && col_offset == 0 {
        return (Vec::new(), false); // target 落在源内则无操作
    }
    let (axis, forward) = if row_offset >= col_offset {
        if trow > sr1 {
            (Axis::Vertical, true)
        } else {
            (Axis::Vertical, false)
        }
    } else {
        if tcol > sc1 {
            (Axis::Horizontal, true)
        } else {
            (Axis::Horizontal, false)
        }
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
        // 合并单元格感知：跳过合并区域的非左上角格（其值由左上角代表），使一个合并单元格
        // 在源序列中只占一个元素——否则合并体内的空格会被当作 0，污染序列步长推断
        // （如 AJ1:AK1 合并值 18 水平填充，AK1 空格→0 使步长算成 -18、结果变成 -18）。
        let src_pos: Vec<(u32, u32)> = {
            let raw: Vec<(u32, u32)> = match axis {
                Axis::Vertical => (sr0..=sr1).map(|r| (lane, r)).collect(),
                Axis::Horizontal => (sc0..=sc1).map(|c| (c, lane)).collect(),
            };
            raw.into_iter()
                .filter(|&(c, r)| !is_merged_part(sheet, c, r))
                .collect()
        };
        let n = src_pos.len();
        if n == 0 {
            continue; // 合并感知过滤后无有效源格，跳过该车道
        }
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
            Some(c) if parse_numeric_date(&c.value).is_some() => Kind::DateNum,
            _ => Kind::Text,
        };
        if kind == Kind::Formula {
            has_formula = true;
        }

        // 目标格坐标（从源边缘向外，j=0 最近源），元素为 (col, row)
        // 合并单元格感知：只在合并区域左上角写入，跳过非左上角（不向合并体内塞隐藏值）。
        let target_pos: Vec<(u32, u32)> = {
            let raw: Vec<(u32, u32)> = match (axis, forward) {
                (Axis::Vertical, true) => ((sr1 + 1)..=trow).map(|r| (lane, r)).collect(),
                (Axis::Vertical, false) => (trow..sr0).rev().map(|r| (lane, r)).collect(),
                (Axis::Horizontal, true) => ((sc1 + 1)..=tcol).map(|c| (c, lane)).collect(),
                (Axis::Horizontal, false) => (tcol..sc0).rev().map(|c| (c, lane)).collect(),
            };
            raw.into_iter()
                .filter(|&(c, r)| !is_merged_part(sheet, c, r))
                .collect()
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
        // 数字型日期序列号（DateNum 用："2026/2/7"、"2026-01-26" → 序列号）
        let dn_serials: Vec<f64> = if kind == Kind::DateNum {
            src_data
                .iter()
                .map(|c| {
                    c.as_ref()
                        .and_then(|cell| parse_numeric_date(&cell.value))
                        .map(|nd| nd.serial)
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
            let old_cell = sheet.get_cell(tr, tc).cloned();
            old_cells.push((tr, tc, old_cell));
            // 新格：克隆 pattern 源格（带格式），再覆写 value/formula/raw_number
            let mut new_cell = src_data[pidx].clone().unwrap_or_default();

            match kind {
                Kind::Formula => {
                    let row_off = tr as i32 - psr as i32;
                    let col_off = tc as i32 - psc as i32;
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
                    if let Some(pat) = parse_date_text(&new_cell.value) {
                        let d = detect_step(&dt_serials).unwrap_or(1.0);
                        let base = if forward { dt_serials[n - 1] } else { dt_serials[0] };
                        let signed_d = if forward { d } else { -d };
                        let serial = base + (j as f64 + 1.0) * signed_d;
                        let (yy, mm, dd) = ExcelData::serial_to_date(serial);
                        new_cell.value = format_date_text(&pat, yy, mm, dd);
                        new_cell.raw_number = Some(serial);
                    }
                    new_cell.formula.clear();
                }
                Kind::DateNum => {
                    if let Some(nd) = parse_numeric_date(&new_cell.value) {
                        let d = detect_step(&dn_serials).unwrap_or(1.0);
                        let base = if forward { dn_serials[n - 1] } else { dn_serials[0] };
                        let signed_d = if forward { d } else { -d };
                        let serial = base + (j as f64 + 1.0) * signed_d;
                        new_cell.value = ExcelData::format_date(serial, &nd.fmt);
                        new_cell.raw_number = Some(serial);
                    }
                    new_cell.formula.clear();
                }
                Kind::Text => {
                    new_cell.formula.clear();
                    new_cell.raw_number = None;
                }
            }
            sheet.cells.insert((tr, tc), new_cell);
        }
    }

    if has_formula {
        crate::excel::formula::invalidate_formula_graph(sheet);
    }

    (old_cells, has_formula)
}

// ========== 预计算填充值（只读，不写入 sheet）==========

/// 预计算的所有填充目标格值（只读，无副作用）。
///
/// 由 [`compute_fill_values`] 生成，供分批跨帧填充逐批写入 sheet。
/// 逻辑与 [`apply_fill`] 完全一致（车道推断/Kind 检测/步长计算/合并感知），
/// 仅把 `sheet.cells.insert` 替换为收集到 `Vec`。
#[derive(Clone)]
pub struct FillValues {
    /// 待写入的目标格列表 `(row, col, new_cell_data)`。
    pub cells: Vec<(u32, u32, CellData)>,
    /// 目标中是否含公式填充（决定重算策略：公式→`evaluate_sheet`，仅值→`evaluate_dependents_many`）。
    pub has_formula: bool,
}

/// 预计算填充值（只读，不修改 sheet）。
///
/// 复用 [`apply_fill`] 的全部推断逻辑，但不执行 `HashMap::insert`——
/// 而是把每个目标格的 `(row, col, CellData)` 收集到 [`FillValues`] 中，
/// 供调用方按批次写入 sheet（分帧填充）。
///
/// 返回 `None` 表示 target 落在源内（无填充操作）。
pub fn compute_fill_values(
    sheet: &SheetData,
    src: (u32, u32, u32, u32),
    target: (u32, u32),
) -> Option<FillValues> {
    let (sc0, sr0, sc1, sr1) = (
        src.0.min(src.2),
        src.1.min(src.3),
        src.0.max(src.2),
        src.1.max(src.3),
    );
    let (tcol, trow) = target;

    // 定轴与方向：按行列偏移量取大者作为填充方向（与 Excel 一致）。
    // 优先取偏移更大的方向，避免填充柄微小垂直误触将水平拖拽误判为垂直填充。
    let row_offset = if trow > sr1 {
        trow - sr1
    } else if trow < sr0 {
        sr0 - trow
    } else {
        0
    };
    let col_offset = if tcol > sc1 {
        tcol - sc1
    } else if tcol < sc0 {
        sc0 - tcol
    } else {
        0
    };
    if row_offset == 0 && col_offset == 0 {
        return None; // target 落在源内则无操作
    }
    let (axis, forward) = if row_offset >= col_offset {
        if trow > sr1 {
            (Axis::Vertical, true)
        } else {
            (Axis::Vertical, false)
        }
    } else {
        if tcol > sc1 {
            (Axis::Horizontal, true)
        } else {
            (Axis::Horizontal, false)
        }
    };

    let mut has_formula = false;
    let mut all_cells: Vec<(u32, u32, CellData)> = Vec::new();

    // 车道：垂直填充按列、水平填充按行
    let lanes: Vec<u32> = match axis {
        Axis::Vertical => (sc0..=sc1).collect(),
        Axis::Horizontal => (sr0..=sr1).collect(),
    };

    for lane in lanes {
        // 源格坐标（合并感知，与 apply_fill 一致）
        let src_pos: Vec<(u32, u32)> = {
            let raw: Vec<(u32, u32)> = match axis {
                Axis::Vertical => (sr0..=sr1).map(|r| (lane, r)).collect(),
                Axis::Horizontal => (sc0..=sc1).map(|c| (c, lane)).collect(),
            };
            raw.into_iter()
                .filter(|&(c, r)| !is_merged_part(sheet, c, r))
                .collect()
        };
        let n = src_pos.len();
        if n == 0 {
            continue; // 合并感知过滤后无有效源格，跳过该车道
        }
        let src_data: Vec<Option<CellData>> = src_pos
            .iter()
            .map(|(c, r)| sheet.get_cell(*r, *c).cloned())
            .collect();

        // 推断类型
        let first = src_data.iter().flatten().next();
        let kind = match first {
            Some(c) if !c.formula.is_empty() => Kind::Formula,
            Some(c) if c.number_format.as_deref().map(ExcelData::is_date_format).unwrap_or(false) => {
                Kind::Date
            }
            Some(c) if cell_number(Some(c)).is_some() => Kind::Number,
            Some(c) if parse_date_text(&c.value).is_some() => Kind::DateText,
            Some(c) if parse_numeric_date(&c.value).is_some() => Kind::DateNum,
            _ => Kind::Text,
        };
        if kind == Kind::Formula {
            has_formula = true;
        }

        // 目标格坐标（合并感知）
        let target_pos: Vec<(u32, u32)> = {
            let raw: Vec<(u32, u32)> = match (axis, forward) {
                (Axis::Vertical, true) => ((sr1 + 1)..=trow).map(|r| (lane, r)).collect(),
                (Axis::Vertical, false) => (trow..sr0).rev().map(|r| (lane, r)).collect(),
                (Axis::Horizontal, true) => ((sc1 + 1)..=tcol).map(|c| (c, lane)).collect(),
                (Axis::Horizontal, false) => (tcol..sc0).rev().map(|c| (c, lane)).collect(),
            };
            raw.into_iter()
                .filter(|&(c, r)| !is_merged_part(sheet, c, r))
                .collect()
        };

        // 数值序列
        let vals: Vec<f64> = src_data
            .iter()
            .map(|c| cell_number(c.as_ref()).unwrap_or(0.0))
            .collect();
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
        // 数字型日期序列号（DateNum 用："2026/2/7"、"2026-01-26" → 序列号）
        let dn_serials: Vec<f64> = if kind == Kind::DateNum {
            src_data
                .iter()
                .map(|c| {
                    c.as_ref()
                        .and_then(|cell| parse_numeric_date(&cell.value))
                        .map(|nd| nd.serial)
                        .unwrap_or(0.0)
                })
                .collect()
        } else {
            Vec::new()
        };

        for (j, &(tc, tr)) in target_pos.iter().enumerate() {
            let pidx = j % n;
            let (psc, psr) = src_pos[pidx];
            let mut new_cell = src_data[pidx].clone().unwrap_or_default();

            match kind {
                Kind::Formula => {
                    let row_off = tr as i32 - psr as i32;
                    let col_off = tc as i32 - psc as i32;
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
                    if let Some(pat) = parse_date_text(&new_cell.value) {
                        let d = detect_step(&dt_serials).unwrap_or(1.0);
                        let base = if forward { dt_serials[n - 1] } else { dt_serials[0] };
                        let signed_d = if forward { d } else { -d };
                        let serial = base + (j as f64 + 1.0) * signed_d;
                        let (yy, mm, dd) = ExcelData::serial_to_date(serial);
                        new_cell.value = format_date_text(&pat, yy, mm, dd);
                        new_cell.raw_number = Some(serial);
                    }
                    new_cell.formula.clear();
                }
                Kind::DateNum => {
                    if let Some(nd) = parse_numeric_date(&new_cell.value) {
                        let d = detect_step(&dn_serials).unwrap_or(1.0);
                        let base = if forward { dn_serials[n - 1] } else { dn_serials[0] };
                        let signed_d = if forward { d } else { -d };
                        let serial = base + (j as f64 + 1.0) * signed_d;
                        new_cell.value = ExcelData::format_date(serial, &nd.fmt);
                        new_cell.raw_number = Some(serial);
                    }
                    new_cell.formula.clear();
                }
                Kind::Text => {
                    new_cell.formula.clear();
                    new_cell.raw_number = None;
                }
            }
            all_cells.push((tr, tc, new_cell));
        }
    }

    if all_cells.is_empty() {
        None
    } else {
        Some(FillValues { cells: all_cells, has_formula })
    }
}

/// 分批跨帧填充的每帧写入上限（格数）。
///
/// 2000 格 × ~1.5μs/格 ≈ 3ms/帧，远低于 16ms 帧预算，UI 保持流畅。
pub const FILL_BATCH_SIZE: usize = 2000;

/// 低于此目标格数的填充走同步路径（单帧完成）；超过则启用分批跨帧模式。
pub const FILL_SYNC_THRESHOLD: usize = 2000;

// ========== 双击填充柄自动填充：目标边界推断 ==========

/// 检查 `(col, row)` 处是否有有效数据（合并感知）。
///
/// 若该位置属于某个合并区域，则以其左上角的值/公式为准（合并区域的数据只存于左上角，
/// 非左上角无独立数据）。"有效" = 存在单元格且 `value` 或 `formula` 非空。
fn cell_occupied(sheet: &SheetData, col: u32, row: u32) -> bool {
    let (c, r) = match sheet.get_merged_range(col, row) {
        Some(mr) => (mr.start_col, mr.start_row),
        None => (col, row),
    };
    sheet
        .get_cell(r, c)
        .map_or(false, |cell| !cell.value.is_empty() || !cell.formula.is_empty())
}

/// 从 `from_row` 起沿 `col` 列向下扫描连续非空格，返回最末非空行号。
///
/// - 合并感知：遇到合并区域时，若其左上角有数据则整个行跨度（`start_row..=end_row`）视为连续占据，
///   并跳到 `end_row + 1` 继续；否则视为空隙，终止扫描。
/// - 隐藏行透明：遇到隐藏行不中断连续性、也不计入占据（跳过）。
/// - `from_row` 本身即空（或被隐藏后的首个非隐藏位为空）时返回 `None`（该方向无边界）。
/// - 受 `max_row` 约束，防止无限扫描。
fn scan_down(
    sheet: &SheetData,
    col: u32,
    from_row: u32,
    max_row: u32,
    hidden_rows: &HashSet<u32>,
) -> Option<u32> {
    let mut r = from_row;
    let mut last: Option<u32> = None;
    while r <= max_row {
        if hidden_rows.contains(&r) {
            r = r.saturating_add(1);
            continue;
        }
        if let Some(mr) = sheet.get_merged_range(col, r) {
            if cell_occupied(sheet, mr.start_col, mr.start_row) {
                last = Some(mr.end_row.max(r));
                r = mr.end_row.saturating_add(1);
                continue;
            } else {
                break;
            }
        }
        if cell_occupied(sheet, col, r) {
            last = Some(r);
            r = r.saturating_add(1);
        } else {
            break;
        }
    }
    last
}

/// 从 `from_col` 起沿 `row` 行向右扫描连续非空格，返回最末非空列号（语义对称于 [`scan_down`]）。
fn scan_right(
    sheet: &SheetData,
    row: u32,
    from_col: u32,
    max_col: u32,
    hidden_cols: &HashSet<u32>,
) -> Option<u32> {
    let mut c = from_col;
    let mut last: Option<u32> = None;
    while c <= max_col {
        if hidden_cols.contains(&c) {
            c = c.saturating_add(1);
            continue;
        }
        if let Some(mr) = sheet.get_merged_range(c, row) {
            if cell_occupied(sheet, mr.start_col, mr.start_row) {
                last = Some(mr.end_col.max(c));
                c = mr.end_col.saturating_add(1);
                continue;
            } else {
                break;
            }
        }
        if cell_occupied(sheet, c, row) {
            last = Some(c);
            c = c.saturating_add(1);
        } else {
            break;
        }
    }
    last
}

/// 双击填充柄自动填充：根据源选区推断填充方向与「相邻连续数据」边界，返回供 [`apply_fill`] 的目标格。
///
/// 复用既有 [`apply_fill`]（已具备序列推断与合并感知），本函数只负责算出"填到哪一格"。
///
/// # 方向推断（按源选区朝向）
/// - 横向线（多列单行）→ 仅向右；纵向线（多行单列）→ 仅向下（与 Excel 一致）。
/// - 单格/方块 → 默认向下，无相邻数据时回退向右；都无则返回 `None`（不填充，避免误操作）。
/// - 方向明确的选区（横向线/纵向线）**不回退另一方向**，避免横向选区误触纵向填充。
///
/// # 边界判定（仿 Excel「双击填充柄填充到相邻连续数据末尾」）
/// - **向下**：从源末行下一行（`sr1+1`）起，**仅沿源列**（`sc0..=sc1`）查找第一个有数据的行，
///   从该行起向下扫描连续非空格，末行即目标行。可跳过起始空行；源列全空时兜底填充到 `max_row`。
/// - **向右**：从源末列右一列（`sc1+1`）起，在候选行（源行优先 → 向上 ≤10 行 → 向下 ≤10 行）
///   中取首个有数据的行作锚点，向右扫描连续非空格，末列即目标列。
/// - **单格/方块**：方向不明确，允许搜索相邻列/行以推断边界（扩展搜索）。
/// - 合并感知 / 隐藏行列透明（详见 [`scan_down`] / [`scan_right`]）。
///
/// # 安全上限
/// 若按边界算出的总填充格数（车道 × 沿轴长度）超过 [`AUTO_FILL_MAX_CELLS`]，
/// 则把沿轴长度夹紧到上限对应的行列数，避免单帧海量写入阻塞 UI。
///
/// # 参数
/// - `src` = `(start_col, start_row, end_col, end_row)` 源选区（内部自动归一化）。
/// - `hidden_cols` / `hidden_rows`：隐藏列/行集合（边界扫描时透明跳过）。
///
/// # 返回
/// `Some((target_col, target_row))` 目标格，可直接传给 [`apply_fill`]；`None` 表示无相邻数据、不填充。
pub fn compute_autofill_target(
    sheet: &SheetData,
    src: (u32, u32, u32, u32),
    hidden_cols: &HashSet<u32>,
    hidden_rows: &HashSet<u32>,
) -> Option<(u32, u32)> {
    let (sc0, sr0, sc1, sr1) = (
        src.0.min(src.2),
        src.1.min(src.3),
        src.0.max(src.2),
        src.1.max(src.3),
    );
    let max_row = sheet.max_row;
    let max_col = sheet.max_col;

    let horizontal_line = sc1 > sc0 && sr1 == sr0;
    let vertical_line = sr1 > sr0 && sc1 == sc0;
    // 朝向决定首选方向：横向线→右，纵向线→下，单格/方块→默认下（允许回退）

    // 向下：仅以源列作为锚点列，从源末行下一行起向下扫描，跳过起始空行
    let try_down = || -> Option<(u32, u32)> {
        let from_row = sr1 + 1;
        if from_row > max_row {
            return None;
        }
        // 在源列中查找 from_row 起第一个有数据的行作为扫描起点；
        // 若源列从 from_row 起全空（如源末尾恰好是表格最底部数据），则兜底填充到 max_row
        let last_row =
            if let Some(anchor_col) = (sc0..=sc1).find(|&c| {
                (from_row..=max_row).any(|r| cell_occupied(sheet, c, r))
            }) {
                let start_row = (from_row..=max_row)
                    .find(|&r| cell_occupied(sheet, anchor_col, r))?;
                scan_down(sheet, anchor_col, start_row, max_row, hidden_rows)?
            } else {
                // 源列无现有数据可作锚点 → 填充到表格末行
                max_row
            };
        let lanes = sc1 - sc0 + 1; // 垂直填充的车道数 = 源列数
        let extent = last_row - sr1; // >= 1
        let total = (lanes as u64) * (extent as u64);
        let extent = if total > AUTO_FILL_MAX_CELLS as u64 {
            ((AUTO_FILL_MAX_CELLS as u64) / (lanes.max(1) as u64)).max(1) as u32
        } else {
            extent
        };
        // target_row > sr1（extent>=1），apply_fill 据此判为「垂直向前」
        Some((sc0, sr1 + extent))
    };

    // ===== 横向填充：允许搜索相邻行（数据延伸行常在源行之外）=====

    // 向下（扩展版）：源列优先，再向左、向右各搜索至多 10 列
    let try_down_expanded = || -> Option<(u32, u32)> {
        if sr1 >= max_row {
            return None;
        }
        let from_row = sr1 + 1;
        let anchor_col = {
            let src_cols = sc0..=sc1;
            let left = (1.max(sc0.saturating_sub(10))..sc0).rev();
            let right = (sc1 + 1)..=(sc1 + 10).min(max_col);
            src_cols.chain(left).chain(right).find(|&c| cell_occupied(sheet, c, from_row))?
        };
        let last_row = scan_down(sheet, anchor_col, from_row, max_row, hidden_rows)?;
        let lanes = sc1 - sc0 + 1;
        let extent = last_row - sr1;
        let total = (lanes as u64) * (extent as u64);
        let extent = if total > AUTO_FILL_MAX_CELLS as u64 {
            ((AUTO_FILL_MAX_CELLS as u64) / (lanes.max(1) as u64)).max(1) as u32
        } else {
            extent
        };
        Some((sc0, sr1 + extent))
    };

    // 向右（扩展版）：源行优先，再向上、向下各搜索至多 10 行
    let try_right_expanded = || -> Option<(u32, u32)> {
        if sc1 >= max_col {
            return None;
        }
        let from_col = sc1 + 1;
        let anchor_row = {
            let src_rows = sr0..=sr1;
            let above = (1.max(sr0.saturating_sub(10))..sr0).rev();
            let below = (sr1 + 1)..=(sr1 + 10).min(max_row);
            src_rows.chain(above).chain(below).find(|&r| cell_occupied(sheet, from_col, r))?
        };
        let last_col = scan_right(sheet, anchor_row, from_col, max_col, hidden_cols)?;
        let lanes = sr1 - sr0 + 1;
        let extent = last_col - sc1;
        let total = (lanes as u64) * (extent as u64);
        let extent = if total > AUTO_FILL_MAX_CELLS as u64 {
            ((AUTO_FILL_MAX_CELLS as u64) / (lanes.max(1) as u64)).max(1) as u32
        } else {
            extent
        };
        Some((sc1 + extent, sr0))
    };

    // 方向明确的选区不回退另一方向（与 Excel 一致）：
    // 纵向线→仅向下，锚点限定于源列（用户需求：不搜索相邻列）
    // 横向线→仅向右，锚点允许搜索相邻行（数据延伸行常在源行之外）
    // 单格/方块→默认向下，允许回退向右（均扩展搜索相邻行列）
    if horizontal_line {
        try_right_expanded()
    } else if vertical_line {
        try_down()
    } else {
        try_down_expanded().or_else(try_right_expanded)
    }
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
            formula_positions: HashSet::new(),
            formula_positions_dirty: true,
            cached_graph: None,
            cached_graph_dirty: true,
        }
    }

    fn put(sheet: &mut SheetData, col: u32, row: u32, value: &str) {
        let mut c = CellData::default();
        c.value = value.to_string();
        c.raw_number = value.trim().parse::<f64>().ok();
        sheet.cells.insert((row, col), c);
    }

    /// 存入纯文本值（无 raw_number、无 number_format），模拟编辑器对无格式单元格的保存路径。
    fn put_text(sheet: &mut SheetData, col: u32, row: u32, value: &str) {
        let mut c = CellData::default();
        c.value = value.to_string();
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

    // ========== 数字型日期文本（DateNum：斜杠/连字符分隔）==========

    #[test]
    fn fill_numeric_date_slash_single() {
        // 单格 "2026/2/7" 向下填 → 按天递增，保持斜杠/无前导零格式
        let mut s = empty_sheet();
        put_text(&mut s, 1, 1, "2026/2/7");
        let _ = apply_fill(&mut s, (1, 1, 1, 1), (1, 3));
        assert_eq!(val(&s, 1, 2), "2026/2/8");
        assert_eq!(val(&s, 1, 3), "2026/2/9");
    }

    #[test]
    fn fill_numeric_date_slash_two_step() {
        // 两格 "2026/2/7"、"2026/2/8" 推断步长 1 → 2026/2/9、2026/2/10
        let mut s = empty_sheet();
        put_text(&mut s, 1, 1, "2026/2/7");
        put_text(&mut s, 1, 2, "2026/2/8");
        let _ = apply_fill(&mut s, (1, 1, 1, 2), (1, 4));
        assert_eq!(val(&s, 1, 3), "2026/2/9");
        assert_eq!(val(&s, 1, 4), "2026/2/10");
    }

    #[test]
    fn fill_numeric_date_dash_padded() {
        // "2026-01-26"（连字符 + 前导零）→ 保持原格式递增
        let mut s = empty_sheet();
        put_text(&mut s, 1, 1, "2026-01-26");
        let _ = apply_fill(&mut s, (1, 1, 1, 1), (1, 3));
        assert_eq!(val(&s, 1, 2), "2026-01-27");
        assert_eq!(val(&s, 1, 3), "2026-01-28");
    }

    #[test]
    fn fill_numeric_date_dash_month_boundary() {
        // 月末跨月：前导零格式保持，跨到下月
        let mut s = empty_sheet();
        put_text(&mut s, 1, 1, "2026-01-30");
        put_text(&mut s, 1, 2, "2026-01-31");
        let _ = apply_fill(&mut s, (1, 1, 1, 2), (1, 3));
        assert_eq!(val(&s, 1, 3), "2026-02-01");
    }

    #[test]
    fn fill_numeric_date_invalid_not_date() {
        // 不存在的日期 "2026/2/30" 不应被当作日期递增 → 按文本复制
        let mut s = empty_sheet();
        put_text(&mut s, 1, 1, "2026/2/30");
        let _ = apply_fill(&mut s, (1, 1, 1, 1), (1, 2));
        assert_eq!(val(&s, 1, 2), "2026/2/30"); // 复制而非 "2026/2/31"
    }

    #[test]
    fn fill_numeric_date_sets_raw_number() {
        // DateNum 填充应写入序列号（raw_number），便于后续按数值处理
        let mut s = empty_sheet();
        put_text(&mut s, 1, 1, "2026/2/7");
        let _ = apply_fill(&mut s, (1, 1, 1, 1), (1, 2));
        let serial = s.get_cell(2, 1).and_then(|c| c.raw_number);
        assert!(serial.is_some(), "raw_number 应被设置");
        // 序列号往返：2026/2/8
        assert_eq!(ExcelData::serial_to_date(serial.unwrap()), (2026, 2, 8));
    }

    #[test]
    fn fill_merged_horizontal_not_polluted_by_empty_body() {
        // 报修用例（等价缩小列号）：A1:B1 合并值 18，向右填充到 C1（C1:D1 合并）应为 19。
        // 修复前：B1（合并非左上角）无数据→被当作 0，序列 [18,0] 步长算成 -18，结果 C1=-18。
        let mut s = empty_sheet();
        s.merged_cells
            .push(crate::excel::reader::CellRange::new(1, 1, 1, 2)); // A1:B1
        s.merged_cells
            .push(crate::excel::reader::CellRange::new(1, 3, 1, 4)); // C1:D1
        s.rebuild_merge_index();
        put(&mut s, 1, 1, "18"); // A1（合并左上角）= 18
        // 源选区 A1:B1，向右填到 C1
        let _ = apply_fill(&mut s, (1, 1, 2, 1), (3, 1));
        assert_eq!(val(&s, 3, 1), "19"); // C1 = 19（递增 +1）
    }

    #[test]
    fn fill_merged_target_writes_only_top_left() {
        // 拖到目标合并格的非左上角（D1，属 C1:D1 合并）时，只在左上角 C1 写入，D1 不被塞值。
        let mut s = empty_sheet();
        s.merged_cells
            .push(crate::excel::reader::CellRange::new(1, 1, 1, 2)); // A1:B1
        s.merged_cells
            .push(crate::excel::reader::CellRange::new(1, 3, 1, 4)); // C1:D1
        s.rebuild_merge_index();
        put(&mut s, 1, 1, "18");
        let _ = apply_fill(&mut s, (1, 1, 2, 1), (4, 1)); // 拖到 D1（C1:D1 的非左上角）
        assert_eq!(val(&s, 3, 1), "19"); // C1（左上角）= 19
        assert_eq!(val(&s, 4, 1), ""); // D1 不被写入
    }

    // ========== 报修：合并单元格填充柄越界到非目标行 ==========
    // 场景：AH–AM 共6列两两合并（AH+AI、AJ+AK、AL+AM），每行独立合并。
    // 第9行 AH9:AI9=49.19、AJ9:AK9=50.19；选中后向右拖拽填充柄至 AL9:AM9。
    // Bug：AL11:AM11（第11行的合并单元格）也被错误填充为 51.19。
    // 列号映射：AH=34, AI=35, AJ=36, AK=37, AL=38, AM=39

    /// 辅助：存入含 raw_number 的数值格（模拟真实数据路径——编辑器存数值时会同时设 raw_number）
    fn put_num(sheet: &mut SheetData, col: u32, row: u32, value: f64) {
        let mut c = CellData::default();
        c.value = format_num(value);
        c.raw_number = Some(value);
        sheet.cells.insert((row, col), c);
    }

    #[test]
    fn fill_horizontal_merged_no_spill_to_other_rows_via_apply_fill() {
        // 场景：AH9:AI9=49.19, AJ9:AK9=50.19（每行独立合并），向右拖拽至 AL9:AM9
        let mut s = empty_sheet();
        // 第9行合并：AH9:AI9, AJ9:AK9, AL9:AM9
        s.merged_cells.push(crate::excel::reader::CellRange::new(9, 34, 9, 35)); // AH9:AI9
        s.merged_cells.push(crate::excel::reader::CellRange::new(9, 36, 9, 37)); // AJ9:AK9
        s.merged_cells.push(crate::excel::reader::CellRange::new(9, 38, 9, 39)); // AL9:AM9
        // 第11行独立合并：AL11:AM11（应不被填充影响）
        s.merged_cells.push(crate::excel::reader::CellRange::new(11, 38, 11, 39)); // AL11:AM11
        s.rebuild_merge_index();
        put_num(&mut s, 34, 9, 49.19); // AH9
        put_num(&mut s, 36, 9, 50.19); // AJ9

        // 源选区 AH9:AK9(34..37)，向右拖拽至 AM9(39)
        let (old_cells, has_formula) = apply_fill(&mut s, (34, 9, 37, 9), (39, 9));

        // AL9（合并左上角）= 51.19（49.19→50.19→51.19，等差步长1）
        assert_eq!(val(&s, 38, 9), "51.19", "AL9 应为 51.19");
        // AM9（合并非左上角）应保持空
        assert_eq!(val(&s, 39, 9), "", "AM9 不应被写入（合并非左上角）");
        // AL11 和 AM11 应完全不受影响（它们是第11行的独立合并）
        assert!(!s.cells.contains_key(&(11, 38)), "AL11 不应被填充");
        assert!(!s.cells.contains_key(&(11, 39)), "AM11 不应被填充");
        assert!(!has_formula, "纯数值填充不应标记 has_formula");
        // old_cells 应只包含 AL9 的旧值（目标只写入一个单元格）
        assert_eq!(old_cells.len(), 1, "应只覆盖一个目标格 AL9");
        assert_eq!(old_cells[0].0, 9, "old_cells[0] row");
        assert_eq!(old_cells[0].1, 38, "old_cells[0] col");
        assert!(old_cells[0].2.is_none(), "old_cells[0] 原值应为空");
    }

    #[test]
    fn fill_horizontal_merged_no_spill_to_other_rows_via_compute_fill_values() {
        // 与上例相同场景，但走 compute_fill_values（实际运行时表.rs 使用的路径）
        let mut s = empty_sheet();
        s.merged_cells.push(crate::excel::reader::CellRange::new(9, 34, 9, 35)); // AH9:AI9
        s.merged_cells.push(crate::excel::reader::CellRange::new(9, 36, 9, 37)); // AJ9:AK9
        s.merged_cells.push(crate::excel::reader::CellRange::new(9, 38, 9, 39)); // AL9:AM9
        s.merged_cells.push(crate::excel::reader::CellRange::new(11, 38, 11, 39)); // AL11:AM11
        s.rebuild_merge_index();
        put_num(&mut s, 34, 9, 49.19); // AH9
        put_num(&mut s, 36, 9, 50.19); // AJ9

        let fv = compute_fill_values(&s, (34, 9, 37, 9), (39, 9));
        assert!(fv.is_some(), "应产生填充值");
        let fv = fv.unwrap();
        assert!(!fv.has_formula);

        // 应只生成一个目标格：AL9(row=9, col=38)
        assert_eq!(fv.cells.len(), 1, "应只生成一个目标格（AL9），而非 {} 个", fv.cells.len());
        let (row, col, ref cell) = fv.cells[0];
        assert_eq!(row, 9, "目标行应为9");
        assert_eq!(col, 38, "目标列应为38(AL)");
        assert_eq!(cell.value, "51.19");

        // 其他行不应出现任何目标格
        let has_row11 = fv.cells.iter().any(|&(r, _, _)| r == 11);
        assert!(!has_row11, "第11行不应出现任何目标格");
    }

    // ========== 报修变体：垂直合并（AH–AM 列两两合并，且各列合并跨越多行）==========
    // 场景：AH9:AI11 合并=49.19，AJ9:AK11 合并=50.19，AL9:AM11 合并（待填充）。
    // 选中两个合并格后向右拖拽填充柄 → 实际是水平填充，但源选区因合并感知而行车道=[9,10,11]，
    // lanes 10/11 的 src_pos 全是 merged part 被 skip，仅 lane 9 产生 AL9。
    // 但因为 AL9:AM11 是单个合并格，写入 AL9 即覆盖整列（包含 row 11），用户感知为"row11 被污染"。

    #[test]
    fn fill_horizontal_when_source_merges_span_three_rows() {
        let mut s = empty_sheet();
        // 列对合并（跨 rows 9-11）
        s.merged_cells.push(crate::excel::reader::CellRange::new(9, 34, 11, 35)); // AH9:AI11
        s.merged_cells.push(crate::excel::reader::CellRange::new(9, 36, 11, 37)); // AJ9:AK11
        s.merged_cells.push(crate::excel::reader::CellRange::new(9, 38, 11, 39)); // AL9:AM11（目标）
        s.rebuild_merge_index();
        put_num(&mut s, 34, 9, 49.19); // AH9（合并左上角）
        put_num(&mut s, 36, 9, 50.19); // AJ9（合并左上角）

        // 用户选中两个合并格，selected_range 展开到合并边界 → (34,9,37,11)
        // 实际传给 apply_fill 的源是 selected_range 归一化后的值
        let (old_cells, _) = apply_fill(&mut s, (34, 9, 37, 11), (39, 9));

        // AL9（AL9:AM11 的左上角）= 51.19
        assert_eq!(val(&s, 38, 9), "51.19", "AL9 应为 51.19");
        // AM9（合并非左上角）不应被独立写入
        assert_eq!(val(&s, 39, 9), "", "AM9 不应被独立写入");
        // 关键：row 10/11 不应有独立的 cell 写入（它们属于同一合并，值由 AL9 体现）
        assert!(!s.cells.contains_key(&(10, 38)), "AL10 不应被独立写入");
        assert!(!s.cells.contains_key(&(11, 38)), "AL11 不应被独立写入");
        // 仅一个目标格被写入
        assert_eq!(old_cells.len(), 1, "只应覆盖 AL9 一个目标格");
    }

    // ========== 报修核心场景：填充柄拖拽误触垂直填充 ==========
    // Bug 根因假设：填充柄位于选区右下角（AK9 底部边缘），其 5×5px 方块有 2px 伸出选区外
    // （即 row 9 → row 10 边界外）。用户拖拽时 click 点落入 row 10，cell_at 返回 trow=10 或 11。
    // 此时 trow > sr1（9），apply_fill 判为 Vertical 而非 Horizontal，导致向下方多行写入。

    #[test]
    fn fill_vertical_mistrigger_when_target_row_below_source() {
        // 源 AH9:AI9=49.19, AJ9:AK9=50.19；每行独立合并（含 row 10/11）
        let mut s = empty_sheet();
        // row 9 合并
        s.merged_cells.push(crate::excel::reader::CellRange::new(9, 34, 9, 35)); // AH9:AI9
        s.merged_cells.push(crate::excel::reader::CellRange::new(9, 36, 9, 37)); // AJ9:AK9
        // row 10 合并
        s.merged_cells.push(crate::excel::reader::CellRange::new(10, 34, 10, 35)); // AH10:AI10
        s.merged_cells.push(crate::excel::reader::CellRange::new(10, 36, 10, 37)); // AJ10:AK10
        // row 11 合并（报修中提及的 AL11:AM11）
        s.merged_cells.push(crate::excel::reader::CellRange::new(11, 38, 11, 39)); // AL11:AM11
        s.rebuild_merge_index();
        put_num(&mut s, 34, 9, 49.19); // AH9
        put_num(&mut s, 36, 9, 50.19); // AJ9

        // 模拟拖拽到 row 10, col 38（填充柄延伸 2px 导致 cell_at 误解为行 10）
        // apply_fill 会判为 Vertical forward
        let (old_cells, _) = apply_fill(&mut s, (34, 9, 37, 9), (38, 10));

        // row 10 应被填充：lane 34(AH) → AH10 = 49.19+1 = 50.19
        assert_eq!(val(&s, 34, 10), "50.19", "AH10 应为 50.19（lane=AH, j=0）");
        // lane 36(AJ) → AJ10 = 50.19+1 = 51.19
        assert_eq!(val(&s, 36, 10), "51.19", "AJ10 应为 51.19（lane=AJ, j=0）");
        // row 11 不应被填充（target 仅到 row 10）
        assert!(!s.cells.contains_key(&(11, 34)), "AH11 不应被填充");
        assert!(!s.cells.contains_key(&(11, 36)), "AJ11 不应被填充");
        assert!(!s.cells.contains_key(&(11, 38)), "AL11 不应被填充（target 只到 row 10）");
        // AL:AM 列的 row 9 不被填充（源不含 AL/AM 列）
        assert_eq!(val(&s, 38, 9), "", "AL9 不应被填充（垂直填充不含 AL 列）");
        // old_cells 应包含 AH10 和 AJ10（两个目标格）
        assert_eq!(old_cells.len(), 2, "应覆盖 AH10 和 AJ10");
    }

    #[test]
    fn fill_vertical_mistrigger_target_row_11() {
        // 同上场景，但拖拽到 row 11, col 39 → 也会充填 row 10（range 10..=11）
        let mut s = empty_sheet();
        s.merged_cells.push(crate::excel::reader::CellRange::new(9, 34, 9, 35));
        s.merged_cells.push(crate::excel::reader::CellRange::new(9, 36, 9, 37));
        s.merged_cells.push(crate::excel::reader::CellRange::new(10, 34, 10, 35));
        s.merged_cells.push(crate::excel::reader::CellRange::new(10, 36, 10, 37));
        s.merged_cells.push(crate::excel::reader::CellRange::new(11, 34, 11, 35));
        s.merged_cells.push(crate::excel::reader::CellRange::new(11, 36, 11, 37));
        s.merged_cells.push(crate::excel::reader::CellRange::new(11, 38, 11, 39)); // AL11:AM11
        s.rebuild_merge_index();
        put_num(&mut s, 34, 9, 49.19);
        put_num(&mut s, 36, 9, 50.19);

        // 拖拽到 row 11, col 39（落点即为 AM11，但 apply_fill 因竖向优先判为 Vertical）
        let (old_cells, _) = apply_fill(&mut s, (34, 9, 37, 9), (39, 11));

        // row 10 被填
        assert_eq!(val(&s, 34, 10), "50.19", "AH10");
        assert_eq!(val(&s, 36, 10), "51.19", "AJ10");
        // row 11 被填
        assert_eq!(val(&s, 34, 11), "51.19", "AH11 = 49.19+2");
        assert_eq!(val(&s, 36, 11), "52.19", "AJ11 = 50.19+2");
        // AL11 不受影响（AL 列不在源 lanes 34..=37 中）
        assert!(!s.cells.contains_key(&(11, 38)), "AL11 不应被填充（源不含 AL 列）");
        // 共 4 个目标格：AH10,AJ10,AH11,AJ11
        assert_eq!(old_cells.len(), 4);
    }

    /// 验证 compute_fill_values 在垂直合并场景也不产生额外行的目标格
    #[test]
    fn compute_fill_values_when_source_merges_span_three_rows() {
        let mut s = empty_sheet();
        s.merged_cells.push(crate::excel::reader::CellRange::new(9, 34, 11, 35)); // AH9:AI11
        s.merged_cells.push(crate::excel::reader::CellRange::new(9, 36, 11, 37)); // AJ9:AK11
        s.merged_cells.push(crate::excel::reader::CellRange::new(9, 38, 11, 39)); // AL9:AM11（目标）
        s.rebuild_merge_index();
        put_num(&mut s, 34, 9, 49.19);
        put_num(&mut s, 36, 9, 50.19);

        let fv = compute_fill_values(&s, (34, 9, 37, 11), (39, 9)).unwrap();
        assert_eq!(fv.cells.len(), 1, "应只生成一个目标格，实际 {} 个: {:?}", fv.cells.len(), fv.cells.iter().map(|(r,c,_)| format!("({},{})", r, c)).collect::<Vec<_>>());
        assert_eq!(fv.cells[0].0, 9, "目标行应为 9");
        assert_eq!(fv.cells[0].1, 38, "目标列应为 38(AL)");

        // 不应出现 row 10/11 的目标格
        let rows: Vec<u32> = fv.cells.iter().map(|(r, _, _)| *r).collect();
        assert!(!rows.contains(&10) && !rows.contains(&11), "不应包含 row 10/11，实际行: {:?}", rows);
    }

    // ========== 双击自动填充目标推断（compute_autofill_target）==========

    fn empty_hidden() -> (HashSet<u32>, HashSet<u32>) {
        (HashSet::new(), HashSet::new())
    }

    #[test]
    fn autofill_horizontal_to_adjacent_boundary() {
        // 报修横向用例（缩小列号）：A1:B1 合并=17、C1:D1 合并=18（源 A1:D1，横向线）；
        // 相邻下方第 2 行 E2:G2 有数据、H2 空 → 边界到 G(7)。
        let mut s = empty_sheet();
        s.merged_cells.push(crate::excel::reader::CellRange::new(1, 1, 1, 2)); // A1:B1
        s.merged_cells.push(crate::excel::reader::CellRange::new(1, 3, 1, 4)); // C1:D1
        s.rebuild_merge_index();
        put(&mut s, 1, 1, "17"); // A1（合并左上角）
        put(&mut s, 3, 1, "18"); // C1（合并左上角）
        put(&mut s, 5, 2, "x"); // E2（相邻行）
        put(&mut s, 6, 2, "y"); // F2（相邻行）
        put(&mut s, 7, 2, "z"); // G2（相邻行）
        // H2 空
        let (hc, hr) = empty_hidden();
        let target = compute_autofill_target(&s, (1, 1, 4, 1), &hc, &hr);
        assert_eq!(target, Some((7, 1))); // 填到 G1
        // 实际填充：E1,F1,G1 = 19,20,21
        let _ = apply_fill(&mut s, (1, 1, 4, 1), target.unwrap());
        assert_eq!(val(&s, 5, 1), "19");
        assert_eq!(val(&s, 6, 1), "20");
        assert_eq!(val(&s, 7, 1), "21");
    }

    #[test]
    fn autofill_vertical_datetext_to_boundary() {
        // 报修纵向用例（缩小行号）：A1="08月17号"、A2="08月18号"（源 A1:A2，纵向线）；
        // 源列 A 列 A3:A6 有数据、A7 空 → 边界到第 6 行。
        let mut s = empty_sheet();
        put(&mut s, 1, 1, "08月17号");
        put(&mut s, 1, 2, "08月18号");
        put(&mut s, 1, 3, "p"); // A3（源列内）
        put(&mut s, 1, 4, "q"); // A4（源列内）
        put(&mut s, 1, 5, "r"); // A5（源列内）
        put(&mut s, 1, 6, "s"); // A6（源列内）
        // A7 空
        let (hc, hr) = empty_hidden();
        let target = compute_autofill_target(&s, (1, 1, 1, 2), &hc, &hr);
        assert_eq!(target, Some((1, 6))); // 填到 A6
        let _ = apply_fill(&mut s, (1, 1, 1, 2), target.unwrap());
        assert_eq!(val(&s, 1, 3), "08月19号");
        assert_eq!(val(&s, 1, 4), "08月20号");
        assert_eq!(val(&s, 1, 5), "08月21号");
        assert_eq!(val(&s, 1, 6), "08月22号");
    }

    #[test]
    fn autofill_merged_adjacent_boundary() {
        // 相邻数据本身含合并：源 A1=5（单格，默认向下），相邻左列无数据、右列 B 起有合并块。
        // B2:C2 合并="m"（跨两列），D2="n"，E2 空 → 边界到 D(4)。
        let mut s = empty_sheet();
        put(&mut s, 1, 1, "5"); // A1 源
        s.merged_cells.push(crate::excel::reader::CellRange::new(2, 2, 2, 3)); // B2:C2
        s.rebuild_merge_index();
        put(&mut s, 2, 2, "m"); // B2（合并左上角）
        put(&mut s, 4, 2, "n"); // D2
        // E2 空
        let (hc, hr) = empty_hidden();
        // 源 A1 单格 → 默认向下。相邻列：左=无（col 0），右=B(2)。B2 占据 → 向下扫 B 列。
        // B2 合并跨到 C 行(同 row2)，D2 占据，E2 空 → 边界 row=2。但源末行 sr1=1，from_row=2。
        // scan_down(B, 2)：B2 合并（行跨度仅 row2）→last=2；D2 不在 B 列……
        // 注意：scan_down 沿「单列 B」向下，B2 占据 row2，B3 空 → last=2。extent=2-1=1 → 目标 (1,2)。
        let target = compute_autofill_target(&s, (1, 1, 1, 1), &hc, &hr);
        assert_eq!(target, Some((1, 2)));
    }

    #[test]
    fn autofill_merged_adjacent_horizontal() {
        // 横向：相邻行含合并。A1:B1（合并=1）、C1:D1（合并=2）（横向线）；
        // 相邻下方 row2：E2:F2 合并="a"、G2="b"、H2 空 → 边界 G(7)。
        let mut s = empty_sheet();
        s.merged_cells.push(crate::excel::reader::CellRange::new(1, 1, 1, 2)); // A1:B1
        s.merged_cells.push(crate::excel::reader::CellRange::new(1, 3, 1, 4)); // C1:D1
        s.merged_cells.push(crate::excel::reader::CellRange::new(2, 5, 2, 6)); // E2:F2
        s.rebuild_merge_index();
        put(&mut s, 1, 1, "1");
        put(&mut s, 3, 1, "2");
        put(&mut s, 5, 2, "a"); // E2 合并左上角（相邻行）
        put(&mut s, 7, 2, "b"); // G2（相邻行）
        let (hc, hr) = empty_hidden();
        let target = compute_autofill_target(&s, (1, 1, 4, 1), &hc, &hr);
        // scan_right(row2, from=5)：E2 合并跨到 F(6)→last=6；G2→last=7；H2 空 → 边界 7
        assert_eq!(target, Some((7, 1)));
    }

    #[test]
    fn autofill_horizontal_no_fallback_to_vertical() {
        // 横向线选区：右侧无数据、左侧下方有数据 → 不应回退纵向填充（与 Excel 一致）
        let mut s = empty_sheet();
        put(&mut s, 1, 1, "17"); // A1（源）
        put(&mut s, 2, 1, "18"); // B1（源），形成横向线 A1:B1
        put(&mut s, 1, 2, "x"); // A2（左侧下方有数据，try_down 会找到）
        // C1 空（右侧无数据，try_right 返回 None）
        let (hc, hr) = empty_hidden();
        // 横向线不回退：右侧无数据 → None（不是回退到纵向填到 A2）
        assert_eq!(compute_autofill_target(&s, (1, 1, 2, 1), &hc, &hr), None);
    }

    #[test]
    fn autofill_horizontal_expanded_anchor_search() {
        // 横向线选区（含合并）：源行(1)的 from_col 为空，但第 3 行有数据
        // → 扩大锚点搜索范围应能发现 row3 的数据并正确横向填充。
        let mut s = empty_sheet();
        s.merged_cells.push(crate::excel::reader::CellRange::new(1, 1, 1, 2)); // A1:B1 合并=17
        s.merged_cells.push(crate::excel::reader::CellRange::new(1, 3, 1, 4)); // C1:D1 合并=18
        s.rebuild_merge_index();
        put(&mut s, 1, 1, "17"); // A1（合并左上角）
        put(&mut s, 3, 1, "18"); // C1（合并左上角）
        // row1 col5 空, row2 col5 空, row3 col5-col7 有数据
        put(&mut s, 5, 3, "x"); // E3
        put(&mut s, 6, 3, "y"); // F3
        put(&mut s, 7, 3, "z"); // G3
        let (hc, hr) = empty_hidden();
        // 扩大搜索应找到 row3 作为锚点 → scan_right 到 G(7) → target=(7,1)
        let target = compute_autofill_target(&s, (1, 1, 4, 1), &hc, &hr);
        assert_eq!(target, Some((7, 1)));
        // 验证实际填充值：合并感知过滤后源=[(A1,17),(C1,18)] → 19,20,21
        let _ = apply_fill(&mut s, (1, 1, 4, 1), target.unwrap());
        assert_eq!(val(&s, 5, 1), "19");
        assert_eq!(val(&s, 6, 1), "20");
        assert_eq!(val(&s, 7, 1), "21");
    }

    #[test]
    fn autofill_vertical_no_fallback_to_horizontal() {
        // 纵向线选区：下方无数据、右侧有数据 → 不应回退横向填充
        let mut s = empty_sheet();
        s.max_row = 2; // 源末行=max_row，无空间填充
        put(&mut s, 1, 1, "10"); // A1（源）
        put(&mut s, 1, 2, "20"); // A2（源），形成纵向线 A1:A2
        put(&mut s, 2, 1, "x"); // B1（右侧有数据，但纵向线不回退横向）
        // from_row=3 > max_row=2，无空间填充
        let (hc, hr) = empty_hidden();
        assert_eq!(compute_autofill_target(&s, (1, 1, 1, 2), &hc, &hr), None);
    }

    #[test]
    fn autofill_vertical_source_only_anchor() {
        // 纵向线选区：源列 A 列 A3:A6 有数据 → 仅沿源列扫描到第 6 行。
        // 验证锚点搜索限定于源列（不搜索非源列数据）。
        let mut s = empty_sheet();
        put(&mut s, 1, 1, "08月17号"); // A1（源）
        put(&mut s, 1, 2, "08月18号"); // A2（源），形成纵向线 A1:A2
        put(&mut s, 1, 3, "a"); // A3（源列内）
        put(&mut s, 1, 4, "b"); // A4（源列内）
        put(&mut s, 1, 5, "c"); // A5（源列内）
        put(&mut s, 1, 6, "d"); // A6（源列内）
        // 同时在 col3 放数据——但纵向线不应搜索非源列，应忽略
        put(&mut s, 3, 3, "ignored"); // 非源列，应被忽略
        let (hc, hr) = empty_hidden();
        let target = compute_autofill_target(&s, (1, 1, 1, 2), &hc, &hr);
        assert_eq!(target, Some((1, 6)));
        let _ = apply_fill(&mut s, (1, 1, 1, 2), target.unwrap());
        assert_eq!(val(&s, 1, 3), "08月19号");
        assert_eq!(val(&s, 1, 4), "08月20号");
        assert_eq!(val(&s, 1, 5), "08月21号");
        assert_eq!(val(&s, 1, 6), "08月22号");
    }

    #[test]
    fn autofill_vertical_datetext_exact_user_scenario() {
        // 报修用例（精确行号）：A38="08月17号"、A39="08月18号"（纵向线 A38:A39）；
        // 源列 A 列 A40:A44 有数据、A45 空 → 边界到第 44 行。
        // 预期：A40..A44 = 08月19号..08月23号（共 5 格）
        let mut s = empty_sheet();
        s.max_row = 100;
        put(&mut s, 1, 38, "08月17号");
        put(&mut s, 1, 39, "08月18号");
        // 源列 A 的数据延伸到第 44 行
        for r in 40..=44 {
            put(&mut s, 1, r, "x");
        }
        let (hc, hr) = empty_hidden();
        let target = compute_autofill_target(&s, (1, 38, 1, 39), &hc, &hr);
        assert_eq!(target, Some((1, 44))); // 填到 A44
        // 执行填充并验证
        let _ = apply_fill(&mut s, (1, 38, 1, 39), target.unwrap());
        assert_eq!(val(&s, 1, 40), "08月19号");
        assert_eq!(val(&s, 1, 41), "08月20号");
        assert_eq!(val(&s, 1, 42), "08月21号");
        assert_eq!(val(&s, 1, 43), "08月22号");
        assert_eq!(val(&s, 1, 44), "08月23号");
        // 确认末格下一格没有被意外填充
        assert!(!s.cells.contains_key(&(45, 1)));
    }

    #[test]
    fn autofill_numeric_date_slash_user_scenario() {
        // 报修用例：A1="2026/2/7"、A2="2026/2/8"（纵向线 A1:A2，纯文本无 number_format）；
        // 源列 A 列 A3:A6 有数据、A7 空 → 双击填充柄边界到第 6 行。
        // 预期：A3..A6 = 2026/2/9 .. 2026/2/12（按天递增，保持斜杠格式）
        let mut s = empty_sheet();
        s.max_row = 100;
        put_text(&mut s, 1, 1, "2026/2/7");
        put_text(&mut s, 1, 2, "2026/2/8");
        for r in 3..=6 {
            put(&mut s, 1, r, "x"); // 源列相邻数据，设定边界
        }
        let (hc, hr) = empty_hidden();
        let target = compute_autofill_target(&s, (1, 1, 1, 2), &hc, &hr);
        assert_eq!(target, Some((1, 6))); // 填到 A6
        let _ = apply_fill(&mut s, (1, 1, 1, 2), target.unwrap());
        assert_eq!(val(&s, 1, 3), "2026/2/9");
        assert_eq!(val(&s, 1, 4), "2026/2/10");
        assert_eq!(val(&s, 1, 5), "2026/2/11");
        assert_eq!(val(&s, 1, 6), "2026/2/12");
    }

    #[test]
    fn autofill_vertical_skip_empty_from_row() {
        // 纵向线 A38:A39，源列 A 的 from_row(A40) 为空，但 A41:A44 有数据
        // → 应跳过空行 A40，从 A41 开始扫描连续数据到 A44。
        let mut s = empty_sheet();
        s.max_row = 100;
        put(&mut s, 1, 38, "08月17号");
        put(&mut s, 1, 39, "08月18号");
        // A40 故意留空（不 put）
        for r in 41..=44 {
            put(&mut s, 1, r, "x");
        }
        let (hc, hr) = empty_hidden();
        let target = compute_autofill_target(&s, (1, 38, 1, 39), &hc, &hr);
        assert_eq!(target, Some((1, 44))); // 填到 A44
        let _ = apply_fill(&mut s, (1, 38, 1, 39), target.unwrap());
        assert_eq!(val(&s, 1, 40), "08月19号");
        assert_eq!(val(&s, 1, 41), "08月20号");
        assert_eq!(val(&s, 1, 42), "08月21号");
        assert_eq!(val(&s, 1, 43), "08月22号");
        assert_eq!(val(&s, 1, 44), "08月23号");
    }

    #[test]
    fn autofill_vertical_to_max_row_when_no_anchor_data() {
        // 报修用例：A44="08月23号"、A45="08月24号"，max_row=47，
        // A46/A47 为空（待填充），双击填充柄应兜底填充到表格末行 A47。
        let mut s = empty_sheet();
        s.max_row = 47;
        put(&mut s, 1, 44, "08月23号");
        put(&mut s, 1, 45, "08月24号");
        // A46、A47 为空——源列从 from_row=46 起无现有数据
        let (hc, hr) = empty_hidden();
        let target = compute_autofill_target(&s, (1, 44, 1, 45), &hc, &hr);
        assert_eq!(target, Some((1, 47))); // 兜底填到 max_row=47
        let _ = apply_fill(&mut s, (1, 44, 1, 45), target.unwrap());
        assert_eq!(val(&s, 1, 46), "08月25号");
        assert_eq!(val(&s, 1, 47), "08月26号");
    }

    #[test]
    fn autofill_no_adjacent_data_returns_none() {
        // 源周围无相邻数据 → 不填充
        let mut s = empty_sheet();
        put(&mut s, 1, 1, "5");
        let (hc, hr) = empty_hidden();
        assert_eq!(compute_autofill_target(&s, (1, 1, 1, 1), &hc, &hr), None);
    }

    #[test]
    fn autofill_cap_clamps_extent() {
        // 相邻列连续数据远超上限 → 目标被夹紧到 AUTO_FILL_MAX_CELLS 个格
        let mut s = empty_sheet();
        s.max_row = 100_000;
        put(&mut s, 1, 1, "5"); // A1 源
        for r in 2..=60_000 {
            put(&mut s, 2, r, "x"); // B 列连续数据
        }
        let (hc, hr) = empty_hidden();
        let target = compute_autofill_target(&s, (1, 1, 1, 1), &hc, &hr);
        // 车道=1，夹紧后 extent = AUTO_FILL_MAX_CELLS = 50000 → 目标行 = 1 + 50000
        assert_eq!(target, Some((1, 1 + AUTO_FILL_MAX_CELLS)));
    }

    #[test]
    fn autofill_hidden_rows_transparent() {
        // 相邻列中间夹隐藏行不中断边界扫描
        let mut s = empty_sheet();
        put(&mut s, 1, 1, "5"); // A1 源
        put(&mut s, 2, 2, "a");
        put(&mut s, 2, 3, "b");
        put(&mut s, 2, 5, "c"); // row4 隐藏，但 row5 仍应被扫到
        let hc = HashSet::new();
        let mut hr = HashSet::new();
        hr.insert(4);
        let target = compute_autofill_target(&s, (1, 1, 1, 1), &hc, &hr);
        // B2,B3 占据；row4 隐藏跳过；B5 占据 → 边界 row=5（隐藏行透明，连续性不断）
        assert_eq!(target, Some((1, 5)));
    }

    // ========== compute_fill_values 测试（预计算填充值，只读）==========

    #[test]
    fn compute_fill_values_number_vertical() {
        // 预计算 + 手动写入，结果与 apply_fill 一致
        let mut s1 = empty_sheet();
        put(&mut s1, 1, 1, "5");
        let fv = compute_fill_values(&s1, (1, 1, 1, 1), (1, 4));
        assert!(fv.is_some());
        let fv = fv.unwrap();
        assert_eq!(fv.cells.len(), 3);
        assert_eq!(fv.has_formula, false);
        assert_eq!(fv.cells[0].2.value, "6"); // A2
        assert_eq!(fv.cells[1].2.value, "7"); // A3
        assert_eq!(fv.cells[2].2.value, "8"); // A4
    }

    #[test]
    fn compute_fill_values_matches_apply_fill() {
        // 预计算写入后与 apply_fill 的结果完全一致
        let mut s1 = empty_sheet();
        put(&mut s1, 1, 1, "2");
        put(&mut s1, 1, 2, "4");
        // apply_fill 路径
        let mut s2 = empty_sheet();
        put(&mut s2, 1, 1, "2");
        put(&mut s2, 1, 2, "4");
        let _ = apply_fill(&mut s2, (1, 1, 1, 2), (1, 5));
        // compute_fill_values 路径
        let fv = compute_fill_values(&s1, (1, 1, 1, 2), (1, 5)).unwrap();
        for &(r, c, ref cell) in &fv.cells {
            s1.cells.insert((r, c), cell.clone());
        }
        // 逐格对比
        for r in 1..=5 {
            assert_eq!(val(&s1, 1, r), val(&s2, 1, r), "row {} mismatch", r);
        }
    }

    #[test]
    fn compute_fill_values_horizontal() {
        let mut s = empty_sheet();
        put(&mut s, 1, 1, "10");
        let fv = compute_fill_values(&s, (1, 1, 1, 1), (4, 1)).unwrap();
        assert_eq!(fv.cells.len(), 3);
        assert_eq!(fv.cells[0].2.value, "11"); // B1
        assert_eq!(fv.cells[1].2.value, "12"); // C1
        assert_eq!(fv.cells[2].2.value, "13"); // D1
    }

    #[test]
    fn compute_fill_values_date_text() {
        let mut s = empty_sheet();
        put(&mut s, 1, 1, "08月24号");
        let fv = compute_fill_values(&s, (1, 1, 1, 1), (1, 2)).unwrap();
        assert_eq!(fv.cells.len(), 1);
        assert_eq!(fv.cells[0].2.value, "08月25号");
    }

    #[test]
    fn compute_fill_values_date_num() {
        // 数字型日期文本预计算：与 apply_fill 一致（斜杠/连字符均覆盖）
        let mut s = empty_sheet();
        put_text(&mut s, 1, 1, "2026/2/7");
        put_text(&mut s, 1, 2, "2026/2/8");
        let fv = compute_fill_values(&s, (1, 1, 1, 2), (1, 4)).unwrap();
        assert_eq!(fv.cells.len(), 2);
        assert_eq!(fv.cells[0].2.value, "2026/2/9");
        assert_eq!(fv.cells[1].2.value, "2026/2/10");
        assert!(fv.cells[0].2.raw_number.is_some());

        // 连字符 + 前导零格式保持
        let mut s2 = empty_sheet();
        put_text(&mut s2, 1, 1, "2026-01-26");
        let fv2 = compute_fill_values(&s2, (1, 1, 1, 1), (1, 2)).unwrap();
        assert_eq!(fv2.cells[0].2.value, "2026-01-27");
    }

    #[test]
    fn compute_fill_values_merged_not_polluted() {
        // 合并单元格场景：预计算与 apply_fill 一致
        let mut s1 = empty_sheet();
        s1.merged_cells.push(crate::excel::reader::CellRange::new(1, 1, 1, 2)); // A1:B1
        s1.merged_cells.push(crate::excel::reader::CellRange::new(1, 3, 1, 4)); // C1:D1
        s1.rebuild_merge_index();
        put(&mut s1, 1, 1, "18");
        let fv = compute_fill_values(&s1, (1, 1, 2, 1), (3, 1)).unwrap();
        // 只在 C1（合并左上角）生成一个值
        assert_eq!(fv.cells.len(), 1);
        assert_eq!(fv.cells[0].0, 1); // row
        assert_eq!(fv.cells[0].1, 3); // col = C
        assert_eq!(fv.cells[0].2.value, "19");
    }

    #[test]
    fn compute_fill_values_formula() {
        let mut s = empty_sheet();
        let mut c = CellData::default();
        c.formula = "=A1+B1".to_string();
        put_cell(&mut s, 3, 1, c); // C1 = =A1+B1
        let fv = compute_fill_values(&s, (3, 1, 3, 1), (3, 3)).unwrap();
        assert_eq!(fv.has_formula, true);
        assert_eq!(fv.cells[0].2.formula, "=A2+B2");
        assert_eq!(fv.cells[1].2.formula, "=A3+B3");
    }

    #[test]
    fn compute_fill_values_target_inside_source_returns_none() {
        let mut s = empty_sheet();
        put(&mut s, 1, 1, "5");
        // target 落在源内 → None
        assert!(compute_fill_values(&s, (1, 1, 3, 3), (2, 2)).is_none());
    }

    /// 大表性能基准（忽略，手动跑）：`cargo test bench_autofill_large -- --nocapture --ignored`
    /// 测量双击自动填充各阶段在 5000×500（~250 万格）表上的耗时。
    /// 现包含 compute_fill_values（预计算路径）与 apply_fill（同步路径）的对比。
    #[test]
    #[ignore]
    fn bench_autofill_large() {
        use std::time::Instant;
        let rows: u32 = 5000;
        let cols: u32 = 500;
        let mut s = empty_sheet();
        s.max_row = rows;
        s.max_col = cols;
        // 填满数据（模拟满表 cells HashMap）
        let t0 = Instant::now();
        for r in 1..=rows {
            for c in 1..=cols {
                s.cells.insert(
                    (r, c),
                    CellData {
                        value: "1".to_string(),
                        raw_number: Some(1.0),
                        ..Default::default()
                    },
                );
            }
        }
        println!("construct {}x{} (cells={}) : {:?}", rows, cols, s.cells.len(), t0.elapsed());

        // 源 A1=1, A2=2（纵向线）；相邻 B 列满数据 → 向下填到 row=rows
        let hidden = HashSet::new();
        let t1 = Instant::now();
        let target = compute_autofill_target(&s, (1, 1, 1, 2), &hidden, &hidden);
        println!("compute_autofill_target : {:?} -> {:?}", t1.elapsed(), target);

        let target = target.expect("应有目标");

        // 预计算路径（分批填充用）
        let t2 = Instant::now();
        let fv = compute_fill_values(&s, (1, 1, 1, 2), target);
        println!("compute_fill_values ({:?} cells) : {:?}", fv.as_ref().map(|v| v.cells.len()), t2.elapsed());

        // 同步路径（apply_fill）
        let t3 = Instant::now();
        let (old, has_f) = apply_fill(&mut s, (1, 1, 1, 2), target);
        println!("apply_fill ({} cells)    : {:?} has_formula={}", old.len(), t3.elapsed(), has_f);

        let t4 = Instant::now();
        crate::excel::formula::evaluate_dependents_many(&mut s, old.iter().map(|(r, c, _)| (*r, *c)));
        println!("evaluate_dependents_many : {:?}", t4.elapsed());
        println!("(总单元格数 = {})", s.cells.len());
    }
}
