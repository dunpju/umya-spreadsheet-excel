//! 导入文件备份工具
//!
//! 用户每次经「文件 → 导入」打开文件时，先把所选文件复制一份到
//! `~/.MyExcel/backup/` 下做快照备份。备份文件命名规则为
//! `原文件名_yyyymmddhhmmss.扩展名`（保留原扩展名）；备份目录不存在时递归创建。
//!
//! 备份属于附加功能：失败时仅向上层返回 `Err`，由调用方决定是否记日志，
//! 但**不应阻断**正常的文件导入与加载流程。

use std::io;
use std::path::{Path, PathBuf};

/// 把用户导入的文件备份到 `~/.MyExcel/backup/`。
///
/// 流程：
/// 1. 解析备份目录 `~/.MyExcel/backup/`（`dirs::home_dir` 不可用时回退到当前目录）；
/// 2. 目录不存在则 `create_dir_all` 递归创建；
/// 3. 拼装备份文件名 `{stem}_{yyyymmddhhmmss}.{ext}`（无扩展名时省略 `.ext`）；
/// 4. `std::fs::copy` 复制源文件到备份路径。
///
/// # 参数
/// * `src` - 用户选择并即将导入的源文件路径
///
/// # 返回
/// 成功返回备份文件的完整路径；失败返回底层 IO 错误（目录创建或文件复制失败）。
///
/// # 示例
/// `template.xlsx` → `~/.MyExcel/backup/template_20260625143005.xlsx`
pub fn backup_imported_file(src: &Path) -> io::Result<PathBuf> {
    // 1. 解析备份目录：~/.MyExcel/backup/（home_dir 不可用时回退当前目录 "."）
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let backup_dir = home.join(".MyExcel").join("backup");

    // 2. 目录不存在则递归创建
    std::fs::create_dir_all(&backup_dir)?;

    // 3. 拼装备份文件名：stem_yyyymmddhhmmss.ext（无扩展名时仅 stem + 时间戳）
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "import".to_string());
    let timestamp = crate::util::date::now_timestamp14();
    let backup_name = match src.extension() {
        Some(ext) => format!("{}_{}.{}", stem, timestamp, ext.to_string_lossy()),
        None => format!("{}_{}", stem, timestamp),
    };
    let backup_path = backup_dir.join(backup_name);

    // 4. 复制源文件到备份路径
    std::fs::copy(src, &backup_path)?;

    Ok(backup_path)
}
