#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

use eframe::egui;
use rupora::{
    RuporaApp, diagnostics,
    instance::{InstanceCoordinator, InstanceRole},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    diagnostics::install_panic_hook();
    diagnostics::append_event(
        "INFO",
        &format!(
            "starting RUPORA {} on {}-{}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    )
    .ok();

    let startup_files = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    let instance = match InstanceCoordinator::acquire("RUPORA", &startup_files) {
        Ok(InstanceRole::Primary(instance)) => instance,
        Ok(InstanceRole::Secondary) => return Ok(()),
        Err(error) => {
            diagnostics::append_event("ERROR", &format!("instance startup failed: {error}")).ok();
            return Err(std::io::Error::other(error).into());
        }
    };

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
        Box::new(move |creation_context| {
            Ok(Box::new(RuporaApp::new_with_instance(
                creation_context,
                startup_files,
                instance,
            )))
        }),
    )?;
    Ok(())
}
