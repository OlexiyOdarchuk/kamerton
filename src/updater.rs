//! Автооновлення самої програми з GitHub releases.
//!
//! При запуску у фоні перевіряється останній реліз репозиторію. Якщо він новіший,
//! файл для цієї системи докачується поруч, поточний exe перейменовується в `.old`,
//! а новий стає на його місце (запущений exe можна перейменувати навіть на Windows).
//! Нова версія починає працювати після перезапуску; `.old` видаляється при наступному старті.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use eframe::egui;

use crate::components::{download_file, github_latest_tag};

/// Репозиторій, з якого беруться релізи.
pub const REPO: &str = "OlexiyOdarchuk/kamerton";

#[derive(Clone, Debug)]
pub enum UpdateState {
    /// Перевірка ще йде або оновлень немає: нічого не показуємо.
    Quiet,
    Downloading(String, Option<f32>),
    /// Нову версію встановлено, вона запрацює після перезапуску.
    Ready(String),
    /// Не вдалося встановити (наприклад, програма лежить у теці без права запису).
    Failed(String),
}

/// Назва файлу в релізі для цієї системи.
pub fn asset_name() -> Option<&'static str> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some("kamerton-windows-x86_64.exe")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("kamerton-linux-x86_64")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("kamerton-macos-aarch64")
    } else {
        None
    }
}

/// «v1.2.3» → (1, 2, 3). Нечислові частини ігноруються.
fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let mut parts = v.trim().trim_start_matches('v').split('.').map(|p| {
        p.chars().take_while(char::is_ascii_digit).collect::<String>().parse::<u32>().ok()
    });
    Some((parts.next()??, parts.next().flatten().unwrap_or(0), parts.next().flatten().unwrap_or(0)))
}

pub fn is_newer(remote: &str, current: &str) -> bool {
    match (parse_version(remote), parse_version(current)) {
        (Some(r), Some(c)) => r > c,
        _ => false,
    }
}

fn old_path(exe: &Path) -> PathBuf {
    exe.with_extension("old")
}

/// Прибирає файл попередньої версії, що лишився після оновлення.
pub fn cleanup_old() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(old_path(&exe));
    }
}

fn set(state: &Arc<Mutex<UpdateState>>, ctx: &egui::Context, s: UpdateState) {
    *state.lock().unwrap() = s;
    ctx.request_repaint();
}

pub fn spawn_check(state: Arc<Mutex<UpdateState>>, ctx: egui::Context) {
    // Під час розробки не підміняємо зібраний бінарник.
    if cfg!(debug_assertions) {
        return;
    }
    std::thread::spawn(move || {
        if let Err(e) = check_and_install(&state, &ctx) {
            set(&state, &ctx, UpdateState::Failed(e));
        }
    });
}

fn check_and_install(state: &Arc<Mutex<UpdateState>>, ctx: &egui::Context) -> Result<(), String> {
    let Some(asset) = asset_name() else { return Ok(()) };
    // Немає інтернету або релізів: тихо працюємо на поточній версії.
    let Ok(tag) = github_latest_tag(REPO) else { return Ok(()) };
    if !is_newer(&tag, env!("CARGO_PKG_VERSION")) {
        return Ok(());
    }

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let new = exe.with_extension("new");
    let url = format!("https://github.com/{REPO}/releases/download/{tag}/{asset}");
    let label = format!("Завантаження нової версії {tag}");
    set(state, ctx, UpdateState::Downloading(label.clone(), Some(0.0)));
    let mut last = 0.0f32;
    let result = download_file(&url, &new, |done, total| {
        if let Some(t) = total {
            let f = done as f32 / t as f32;
            if f - last >= 0.02 {
                last = f;
                set(state, ctx, UpdateState::Downloading(label.clone(), Some(f)));
            }
        }
    });
    if result.is_err() {
        // Реліз ще збирається, файлу для цієї системи немає або обірвався інтернет:
        // тихо спробуємо наступного запуску.
        let _ = std::fs::remove_file(&new);
        set(state, ctx, UpdateState::Quiet);
        return Ok(());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    let old = old_path(&exe);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|e| format!("немає права замінити програму: {e}"))?;
    if let Err(e) = std::fs::rename(&new, &exe) {
        // Повертаємо стару версію на місце.
        let _ = std::fs::rename(&old, &exe);
        return Err(format!("не вдалося встановити оновлення: {e}"));
    }
    set(state, ctx, UpdateState::Ready(tag));
    Ok(())
}

/// Запускає нову версію і закриває поточну.
pub fn restart(ctx: &egui::Context) {
    if let Ok(exe) = std::env::current_exe()
        && std::process::Command::new(exe).spawn().is_ok()
    {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(is_newer("v0.1.10", "0.1.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("v0.0.9", "0.1.0"));
        assert!(!is_newer("nightly", "0.1.0"));
    }
}
