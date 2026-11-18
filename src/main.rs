// На Windows не показувати чорне вікно консолі поруч з програмою.
#![windows_subsystem = "windows"]

mod app;
mod components;
mod install;
mod instance;
mod job;
mod report;
mod settings;
mod sniff;
mod theme;
mod updater;
mod util;
mod widgets;

use eframe::egui;

fn main() -> eframe::Result {
    // Запам'ятати шлях до програми, поки оновлення ще не перейменувало файл.
    let _ = updater::exe_path();
    let uninstall = std::env::args().any(|a| a == "--uninstall");
    // Windows: перший запуск зі «Завантажень» встановлює програму і відкриває встановлену копію.
    if !uninstall && install::ensure_installed() {
        return Ok(());
    }

    let dirs = util::AppDirs::new();
    let instance = match instance::acquire(&dirs.data) {
        instance::Acquire::First(i) => i,
        // Вже відкрито: та копія сама вийде на передній план і додасть посилання.
        instance::Acquire::Second => {
            if uninstall {
                rfd::MessageDialog::new()
                    .set_title("Камертон")
                    .set_description("Спершу закрийте Камертон, а потім спробуйте видалити ще раз.")
                    .show();
            }
            return Ok(());
        }
    };

    // Видалення з «Програм та можливостей» Windows.
    if uninstall {
        if install::confirm_uninstall() {
            drop(instance);
            install::uninstall(&dirs.root());
        }
        return Ok(());
    }

    install::register_desktop_entry();

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Камертон")
        .with_app_id("kamerton")
        .with_inner_size([1040.0, 720.0])
        .with_min_inner_size([800.0, 540.0]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "Камертон",
        options,
        Box::new(move |cc| Ok(Box::new(app::KamertonApp::new(cc, dirs, instance)))),
    )
}
