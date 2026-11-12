//! Міні-установка на Windows.
//!
//! Якщо програму запустили не з її постійної теки (наприклад, просто зі «Завантажень»),
//! вона копіює себе в `%LOCALAPPDATA%\Programs\Kamerton`, створює ярлики на робочому столі
//! і в меню «Пуск» і перезапускається звідти. Права адміністратора не потрібні,
//! а автооновлення потім має право замінювати файл.
//!
//! На Linux програма лише додає себе в меню програм (див. [`register_desktop_entry`]).

/// Повертає `true`, якщо запущено встановлену копію і цей процес треба завершити.
#[cfg(windows)]
pub fn ensure_installed() -> bool {
    use std::path::PathBuf;

    if cfg!(debug_assertions) || std::env::var_os("KAMERTON_NO_INSTALL").is_some() {
        return false;
    }
    let (Ok(exe), Some(local)) = (crate::updater::exe_path().ok_or(()), std::env::var_os("LOCALAPPDATA")) else {
        return false;
    };
    let dir = PathBuf::from(local).join("Programs").join("Kamerton");
    let target = dir.join("kamerton.exe");
    let same = |a: &std::path::Path, b: &std::path::Path| {
        let norm = |p: &std::path::Path| {
            std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().to_lowercase()
        };
        norm(a) == norm(b)
    };
    if same(&exe, &target) {
        return false;
    }
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    // Якщо встановлена копія вже запущена, файл зайнятий: тоді просто відкриваємо її.
    let copied = std::fs::copy(&exe, &target).is_ok();
    if !target.is_file() {
        return false;
    }
    if copied {
        create_shortcuts(&target);
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    std::process::Command::new(&target).args(args).current_dir(&dir).spawn().is_ok()
}

#[cfg(not(windows))]
pub fn ensure_installed() -> bool {
    false
}

/// Linux: пункт «Камертон» у меню програм (`~/.local/share/applications`) і значок у темі
/// `hicolor`. `Exec` вказує на поточний файл програми, тож запис працює і для `cargo install`,
/// і для завантаженого файлу. Якщо файл програми видалити, `TryExec` ховає пункт з меню.
#[cfg(target_os = "linux")]
pub fn register_desktop_entry() {
    if cfg!(debug_assertions) || std::env::var_os("KAMERTON_NO_INSTALL").is_some() {
        return;
    }
    let (Some(exe), Some(base)) = (crate::updater::exe_path(), directories::BaseDirs::new()) else {
        return;
    };
    let share = base.data_dir();
    let icons = share.join("icons").join("hicolor");
    write_if_changed(&icons.join("scalable/apps/kamerton.svg"), include_bytes!("../assets/icon.svg"));
    write_if_changed(&icons.join("256x256/apps/kamerton.png"), include_bytes!("../assets/icon.png"));
    write_if_changed(&share.join("applications/kamerton.desktop"), desktop_entry(&exe).as_bytes());
}

#[cfg(not(target_os = "linux"))]
pub fn register_desktop_entry() {}

#[cfg(any(target_os = "linux", test))]
fn desktop_entry(exe: &std::path::Path) -> String {
    let exe = exe.to_string_lossy();
    // Рядок у .desktop: `\` подвоюється. В Exec ще й лапки з екрануванням `"` `` ` `` `$` `\`, а `%` подвоюється.
    let string = |s: &str| s.replace('\\', "\\\\");
    let mut quoted = String::new();
    for c in exe.chars() {
        match c {
            '"' | '`' | '$' | '\\' => quoted.extend(['\\', c]),
            '%' => quoted.push_str("%%"),
            _ => quoted.push(c),
        }
    }
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Камертон\n\
         GenericName=Завантаження відео і музики\n\
         Comment=Завантаження відео і музики з YouTube, Facebook, Instagram, TikTok, SoundCloud та інших сайтів\n\
         TryExec={}\n\
         Exec=\"{}\" %U\n\
         Icon=kamerton\n\
         Terminal=false\n\
         StartupNotify=true\n\
         StartupWMClass=kamerton\n\
         Categories=AudioVideo;\n\
         Keywords=youtube;video;audio;music;download;відео;музика;завантаження;\n",
        string(&exe),
        string(&quoted),
    )
}

#[cfg(target_os = "linux")]
fn write_if_changed(path: &std::path::Path, data: &[u8]) {
    if std::fs::read(path).is_ok_and(|old| old == data) {
        return;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, data);
}

/// Ключ реєстру, через який програма видна в «Програмах та можливостях».
#[cfg(windows)]
const UNINSTALL_KEY: &str = r"HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Kamerton";

/// Ярлики «Камертон» на робочому столі і в меню «Пуск» та запис у «Програмах та можливостях».
#[cfg(windows)]
fn create_shortcuts(target: &std::path::Path) {
    let target = target.display().to_string().replace('\'', "''");
    let dir = std::path::Path::new(&target).parent().map(|d| d.display().to_string()).unwrap_or_default();
    let script = format!(
        "$w = New-Object -ComObject WScript.Shell\n\
         foreach ($d in @([Environment]::GetFolderPath('Desktop'), [Environment]::GetFolderPath('Programs'))) {{\n\
           $l = $w.CreateShortcut((Join-Path $d 'Камертон.lnk'))\n\
           $l.TargetPath = '{target}'\n\
           $l.WorkingDirectory = '{dir}'\n\
           $l.Description = 'Завантаження відео і музики'\n\
           $l.Save()\n\
         }}\n\
         $k = '{UNINSTALL_KEY}'\n\
         New-Item -Path $k -Force | Out-Null\n\
         Set-ItemProperty -Path $k -Name DisplayName -Value 'Камертон'\n\
         Set-ItemProperty -Path $k -Name DisplayIcon -Value '{target}'\n\
         Set-ItemProperty -Path $k -Name InstallLocation -Value '{dir}'\n\
         Set-ItemProperty -Path $k -Name UninstallString -Value '\"{target}\" --uninstall'\n\
         Set-ItemProperty -Path $k -Name NoModify -Value 1 -Type DWord\n\
         Set-ItemProperty -Path $k -Name NoRepair -Value 1 -Type DWord"
    );
    let _ = powershell(&script).status();
}

/// PowerShell без вікна. Скрипт передається як -EncodedCommand (UTF-16LE у base64),
/// щоб кирилиця не побилась через кодування консолі.
#[cfg(windows)]
fn powershell(script: &str) -> std::process::Command {
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut cmd = crate::util::command("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &base64(&utf16)]);
    cmd
}

/// Чи це встановлена копія (видалення має сенс лише для неї).
#[cfg(windows)]
pub fn is_installed_copy() -> bool {
    let (Ok(exe), Some(local)) = (crate::updater::exe_path().ok_or(()), std::env::var_os("LOCALAPPDATA")) else {
        return false;
    };
    let target = std::path::PathBuf::from(local).join("Programs").join("Kamerton").join("kamerton.exe");
    exe.to_string_lossy().eq_ignore_ascii_case(&target.to_string_lossy())
}

#[cfg(not(windows))]
pub fn is_installed_copy() -> bool {
    false
}

/// Видаляє програму: ярлики, запис у реєстрі, теку програми і завантажені інструменти,
/// налаштування та історію. Скачані відео й музика лишаються.
/// Працює у фоновому PowerShell, який чекає, поки ця копія закриється.
#[cfg(windows)]
pub fn uninstall(app_data: &std::path::Path) {
    let Ok(exe) = crate::updater::exe_path().ok_or(()) else { return };
    let q = |p: &std::path::Path| p.display().to_string().replace('\'', "''");
    let install_dir = exe.parent().map(q).unwrap_or_default();
    let data = q(app_data);
    let script = format!(
        "Start-Sleep -Seconds 2\n\
         foreach ($d in @([Environment]::GetFolderPath('Desktop'), [Environment]::GetFolderPath('Programs'))) {{\n\
           Remove-Item -LiteralPath (Join-Path $d 'Камертон.lnk') -Force -ErrorAction SilentlyContinue\n\
         }}\n\
         Remove-Item -Path '{UNINSTALL_KEY}' -Recurse -Force -ErrorAction SilentlyContinue\n\
         Remove-Item -LiteralPath '{data}' -Recurse -Force -ErrorAction SilentlyContinue\n\
         Remove-Item -LiteralPath '{install_dir}' -Recurse -Force -ErrorAction SilentlyContinue"
    );
    let _ = powershell(&script).spawn();
}

#[cfg(not(windows))]
pub fn uninstall(_: &std::path::Path) {}

/// Запит підтвердження у звичайному вікні Windows.
pub fn confirm_uninstall() -> bool {
    rfd::MessageDialog::new()
        .set_title("Видалити Камертон")
        .set_description(
            "Буде видалено програму, ярлики, завантажені інструменти, налаштування та історію.\n\
             Скачані відео й музика залишаться на місці.",
        )
        .set_buttons(rfd::MessageButtons::YesNo)
        .set_level(rfd::MessageLevel::Warning)
        .show()
        == rfd::MessageDialogResult::Yes
}

#[cfg(any(windows, test))]
fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_reference() {
        let utf16: Vec<u8> = "Камертон ok".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(super::base64(&utf16), "GgQwBDwENQRABEIEPgQ9BCAAbwBrAA==");
        assert_eq!(super::base64(b"ab"), "YWI=");
        assert_eq!(super::base64(b"abc"), "YWJj");
    }

    #[test]
    fn desktop_entry_quotes_exec() {
        let e = super::desktop_entry(std::path::Path::new("/home/u/my apps/100%/kamerton"));
        assert!(e.contains("\nExec=\"/home/u/my apps/100%%/kamerton\" %U\n"));
        assert!(e.contains("\nTryExec=/home/u/my apps/100%/kamerton\n"));
        let e = super::desktop_entry(std::path::Path::new("/a\\b/$x"));
        assert!(e.contains("\nExec=\"/a\\\\\\\\b/\\\\$x\" %U\n"));
    }
}
