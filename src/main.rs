// На Windows не показувати чорне вікно консолі поруч з програмою.
#![windows_subsystem = "windows"]

mod app;
mod components;
mod install;
mod job;
mod settings;
mod theme;
mod updater;
mod util;
mod widgets;

use eframe::egui;

fn main() -> eframe::Result {
    // Windows: перший запуск зі «Завантажень» встановлює програму і відкриває встановлену копію.
    if install::ensure_installed() {
        return Ok(());
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Камертон")
        .with_app_id("kamerton")
        .with_inner_size([1040.0, 720.0])
        .with_min_inner_size([800.0, 540.0]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("Камертон", options, Box::new(|cc| Ok(Box::new(app::KamertonApp::new(cc)))))
}
