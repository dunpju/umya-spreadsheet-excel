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

    // 版本号变更时重新运行本脚本
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");

    // 说明：版本化产物（my-excel-<version>.exe）由根目录的 `build.bat` 在 `cargo build --release`
    // 完成后复制生成。此处不在 build.rs 内派生后台进程——那会因继承 cargo 的 stdout 管道而
    // 确定性地挂起 cargo（已验证），故改用构建后批处理这一可靠方式。
}
