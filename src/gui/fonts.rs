//! 字体加载模块
//! 
//! 负责加载和设置中文字体，确保中文能在界面中正常显示

use eframe::egui;

/// 加载中文字体
/// 
/// 尝试从 Windows 系统字体目录加载常用中文字体
/// 
/// # 返回值
/// 成功返回 (字体名称, 字体数据)，失败返回 None
pub fn load_chinese_font() -> Option<(String, Vec<u8>)> {
    // 常用中文字体路径列表
    let font_paths = vec![
        r"C:\Windows\Fonts\msyh.ttc",    // 微软雅黑
        r"C:\Windows\Fonts\msyh.ttf",    // 微软雅黑
        r"C:\Windows\Fonts\simhei.ttf",  // 黑体
        r"C:\Windows\Fonts\simkai.ttf",  // 楷体
    ];

    // 遍历尝试加载字体
    for font_path in font_paths {
        if let Ok(font_data) = std::fs::read(font_path) {
            // 根据路径判断字体名称
            if font_path.contains("msyh") {
                return Some(("Microsoft YaHei".to_string(), font_data));
            } else if font_path.contains("simhei") {
                return Some(("SimHei".to_string(), font_data));
            } else if font_path.contains("simkai") {
                return Some(("SimKai".to_string(), font_data));
            }
        }
    }
    None
}

/// 设置中文字体
/// 
/// 将中文字体添加到 egui 的字体系统中，确保中文能正常显示
/// 
/// # 参数
/// * `ctx` - egui 上下文
pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    
    // 尝试加载中文字体
    if let Some((ref font_name, font_data)) = load_chinese_font() {
        // 添加字体数据
        fonts.font_data.insert(
            font_name.clone(),
            egui::FontData::from_owned(font_data),
        );
        
        // 将字体插入到字体族的首位，优先使用
        fonts.families.get_mut(&egui::FontFamily::Proportional).unwrap().insert(0, font_name.clone());
        fonts.families.get_mut(&egui::FontFamily::Monospace).unwrap().insert(0, font_name.clone());
        
        // 应用字体设置
        ctx.set_fonts(fonts);
    }
}
