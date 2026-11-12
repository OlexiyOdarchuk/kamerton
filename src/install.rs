//! Міні-установка на Windows.
//!
//! Якщо програму запустили не з її постійної теки (наприклад, просто зі «Завантажень»),
//! вона копіює себе в `%LOCALAPPDATA%\Programs\Kamerton`, створює ярлики на робочому столі
//! і в меню «Пуск» і перезапускається звідти. Права адміністратора не потрібні,
//! а автооновлення потім має право замінювати файл.

/// Повертає `true`, якщо запущено встановлену копію і цей процес треба завершити.
#[cfg(windows)]
pub fn ensure_installed() -> bool {
    use std::path::PathBuf;

    if cfg!(debug_assertions) || std::env::var_os("KAMERTON_NO_INSTALL").is_some() {
        return false;
    }
    let (Ok(exe), Some(local)) = (std::env::current_exe(), std::env::var_os("LOCALAPPDATA")) else {
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

/// Ярлики «Камертон» на робочому столі і в меню «Пуск».
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
         }}"
    );
    // -EncodedCommand (UTF-16LE у base64), щоб кирилиця не побилась через кодування консолі.
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let _ = crate::util::command("powershell")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &base64(&utf16)])
        .status();
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
}
