fn main() {
    // 仅在 Windows 目标上嵌入资源（winresource 依赖 rc.exe，非 Windows 无意义）
    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        // 设置图标：必须是 .ico 格式（Windows 资源编译器不支持 SVG）。
        // assets/icon.ico 由 `npm run gen-icon` 从 assets/icon.svg 生成（多分辨率 16~256）。
        res.set_icon("./assets/icon-v3-128.ico");
        res.compile().unwrap();
    }
}