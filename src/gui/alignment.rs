//! 对齐方式转换模块
//! 
//! 负责将 Excel 单元格对齐方式转换为 egui 的对齐类型

use eframe::egui;
use crate::excel::reader::{CellAlignment, HorizontalAlignment, VerticalAlignment};

/// 将自定义的单元格对齐方式转换为 egui 的对齐类型
/// 
/// # 参数
/// * `alignment` - 单元格对齐方式
/// 
/// # 返回值
/// 对应的 egui 对齐类型
pub fn alignment_to_egui(alignment: &CellAlignment) -> egui::Align2 {
    match (alignment.horizontal, alignment.vertical) {
        (HorizontalAlignment::Left, VerticalAlignment::Top) => egui::Align2::LEFT_TOP,
        (HorizontalAlignment::Left, VerticalAlignment::Center) => egui::Align2::LEFT_CENTER,
        (HorizontalAlignment::Left, VerticalAlignment::Bottom) => egui::Align2::LEFT_BOTTOM,
        (HorizontalAlignment::Center, VerticalAlignment::Top) => egui::Align2::CENTER_TOP,
        (HorizontalAlignment::Center, VerticalAlignment::Center) => egui::Align2::CENTER_CENTER,
        (HorizontalAlignment::Center, VerticalAlignment::Bottom) => egui::Align2::CENTER_BOTTOM,
        (HorizontalAlignment::Right, VerticalAlignment::Top) => egui::Align2::RIGHT_TOP,
        (HorizontalAlignment::Right, VerticalAlignment::Center) => egui::Align2::RIGHT_CENTER,
        (HorizontalAlignment::Right, VerticalAlignment::Bottom) => egui::Align2::RIGHT_BOTTOM,
        (HorizontalAlignment::CenterContinuous, _) => egui::Align2::CENTER_CENTER,
        (HorizontalAlignment::Fill, _) => egui::Align2::CENTER_CENTER,
        (HorizontalAlignment::Justify, _) => egui::Align2::CENTER_CENTER,
        (HorizontalAlignment::Distributed, _) => egui::Align2::CENTER_CENTER,
        (HorizontalAlignment::General, VerticalAlignment::Top) => egui::Align2::LEFT_TOP,
        (HorizontalAlignment::General, VerticalAlignment::Center) => egui::Align2::LEFT_CENTER,
        (HorizontalAlignment::General, VerticalAlignment::Bottom) => egui::Align2::LEFT_BOTTOM,
        (HorizontalAlignment::General, VerticalAlignment::Justify) => egui::Align2::LEFT_CENTER,
        (HorizontalAlignment::General, VerticalAlignment::Distributed) => egui::Align2::LEFT_CENTER,
        (_, VerticalAlignment::Justify) => egui::Align2::CENTER_CENTER,
        (_, VerticalAlignment::Distributed) => egui::Align2::CENTER_CENTER,
    }
}
