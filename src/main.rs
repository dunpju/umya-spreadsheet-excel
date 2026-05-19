use eframe::NativeOptions;

mod excel;
mod gui;

use gui::viewer::ExcelViewer;

fn main() -> eframe::Result<()> {
    let options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Excel Viewer",
        options,
        Box::new(|_cc| Ok(Box::new(ExcelViewer::new()))),
    )
}
