//! 状态管理模块
//! 
//! 定义 Excel 查看器的状态类型和相关操作

use crate::excel::reader::ExcelData;

/// 文件加载状态枚举
#[derive(Debug, Clone)]
pub enum LoadState {
    /// 空闲状态，未加载任何文件
    Idle,
    /// 正在加载中
    Loading,
    /// 加载成功，包含 Excel 数据
    #[allow(dead_code)]
    Success(ExcelData),
    /// 加载失败，包含错误信息
    #[allow(dead_code)]
    Failed(String),
}
