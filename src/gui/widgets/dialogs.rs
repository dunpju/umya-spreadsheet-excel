//! 对话框组件
//! 
//! 负责显示文件导入等对话框



/// 绘制导入文件对话框
/// 
/// 使用 rfd 库显示原生文件选择对话框
/// 
/// # 参数
/// * `show_import_dialog` - 控制是否显示对话框的可变引用
/// 
/// # 返回值
/// 如果用户选择了文件，返回 Some(文件路径)，否则返回 None
pub fn draw_import_dialog(show_import_dialog: &mut bool) -> Option<String> {
    if *show_import_dialog {
        // 显示文件选择对话框，只允许 xlsx 和 xls 文件
        let result = rfd::FileDialog::new()
            .add_filter("Excel Files", &["xlsx", "xls"])
            .pick_file();
        *show_import_dialog = false;
        result.map(|path| path.to_string_lossy().to_string())
    } else {
        None
    }
}
