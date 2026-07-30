#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use rupora::RuporaApp;

fn main() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("RUPORA")
        .with_inner_size([1240.0, 820.0])
        .with_min_inner_size([760.0, 520.0]);

    if let Ok(icon) =
        eframe::icon_data::from_png_bytes(include_bytes!("../../src-tauri/icons/icon.png"))
    {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::run_native(
        "RUPORA",
        options,
        Box::new(|creation_context| Ok(Box::new(RuporaApp::new(creation_context)))),
    )
}
