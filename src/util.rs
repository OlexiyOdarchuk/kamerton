//! Дрібні допоміжні функції: шляхи, запуск процесів без консолі, форматування.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Папки програми: `bin` для yt-dlp/ffmpeg/deno, `tmp` для недокачаного,
/// налаштування та історія поруч.
pub struct AppDirs {
    pub data: PathBuf,
    pub config: PathBuf,
}

impl AppDirs {
    pub fn new() -> Self {
        let pd = directories::ProjectDirs::from("", "", "Kamerton");
        let (data, config) = match pd {
            Some(p) => (p.data_dir().to_path_buf(), p.config_dir().to_path_buf()),
            None => {
                let base = std::env::current_dir().unwrap_or_default().join("kamerton-data");
                (base.clone(), base)
            }
        };
        let _ = std::fs::create_dir_all(&data);
        let _ = std::fs::create_dir_all(&config);
        Self { data, config }
    }

    /// Спільна тека налаштувань і даних програми (на Windows `%APPDATA%\Kamerton`).
    pub fn root(&self) -> PathBuf {
        match (self.data.parent(), self.config.parent()) {
            (Some(a), Some(b)) if a == b => a.to_path_buf(),
            _ => self.data.clone(),
        }
    }

    pub fn bin(&self) -> PathBuf {
        self.data.join("bin")
    }

    pub fn tmp(&self) -> PathBuf {
        self.data.join("tmp")
    }
}

pub fn exe_name(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

/// `Command`, який на Windows не відкриває чорне вікно консолі.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    // Статичний ffmpeg не знає, де в системі лежать сертифікати, і падає на https.
    #[cfg(unix)]
    if std::env::var_os("SSL_CERT_FILE").is_none() {
        const BUNDLES: [&str; 5] = [
            "/etc/ssl/certs/ca-certificates.crt",
            "/etc/pki/tls/certs/ca-bundle.crt",
            "/etc/ssl/ca-bundle.pem",
            "/etc/ssl/cert.pem",
            "/usr/local/etc/openssl/cert.pem",
        ];
        if let Some(b) = BUNDLES.iter().find(|b| Path::new(b).is_file()) {
            cmd.env("SSL_CERT_FILE", b);
        }
    }
    cmd
}

/// Зупиняє процес разом з усіма дочірніми (yt-dlp запускає ffmpeg і deno).
pub fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let _ = command("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status();
    }
    #[cfg(unix)]
    {
        // Процес запускається у власній групі (див. job.rs), тож вбиваємо всю групу.
        let _ = Command::new("kill")
            .args(["-s", "TERM", "--", &format!("-{pid}")])
            .status();
    }
}

/// Відкрити файловий менеджер і по можливості виділити файл.
pub fn reveal(path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if path.is_file() {
            let _ = Command::new("explorer")
                .raw_arg(format!("/select,\"{}\"", path.display()))
                .spawn();
        } else {
            let _ = Command::new("explorer").arg(path).spawn();
        }
    }
    #[cfg(target_os = "macos")]
    {
        if path.is_file() {
            let _ = Command::new("open").arg("-R").arg(path).spawn();
        } else {
            let _ = Command::new("open").arg(path).spawn();
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if path.is_file() {
            let uri = format!("file://{}", path.display());
            let ok = Command::new("dbus-send")
                .args([
                    "--session",
                    "--dest=org.freedesktop.FileManager1",
                    "--type=method_call",
                    "/org/freedesktop/FileManager1",
                    "org.freedesktop.FileManager1.ShowItems",
                    &format!("array:string:{uri}"),
                    "string:",
                ])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok {
                return;
            }
        }
        let dir = if path.is_file() { path.parent().unwrap_or(path) } else { path };
        let _ = Command::new("xdg-open").arg(dir).spawn();
    }
}

/// Відкрити файл програмою за замовчуванням (плеєром).
pub fn open(path: &Path) {
    #[cfg(windows)]
    {
        let _ = Command::new("explorer").arg(path).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = Command::new("xdg-open").arg(path).spawn();
    }
}

/// Витягує посилання з довільного тексту (по одному на рядок або через пробіл).
pub fn extract_urls(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter_map(|t| {
            let t = t.trim_matches(|c: char| c == '"' || c == '\'' || c == '<' || c == '>');
            if t.starts_with("http://") || t.starts_with("https://") {
                Some(t.to_string())
            } else if t.contains('.') && t.contains('/') && !t.contains("://") {
                Some(format!("https://{t}"))
            } else {
                None
            }
        })
        .collect()
}

/// «1:20», «80», «1:02:03» → секунди. Порожній рядок → None.
pub fn parse_time(s: &str) -> Result<Option<f64>, ()> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let mut total = 0.0;
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() > 3 {
        return Err(());
    }
    for p in parts {
        let v: f64 = p.trim().replace(',', ".").parse().map_err(|_| ())?;
        if v < 0.0 {
            return Err(());
        }
        total = total * 60.0 + v;
    }
    Ok(Some(total))
}

pub fn fmt_duration(secs: f64) -> String {
    let s = secs.round() as u64;
    let (h, m, s) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

pub fn fmt_bytes(b: f64) -> String {
    const KB: f64 = 1024.0;
    if b >= KB * KB * KB {
        format!("{:.1} ГБ", b / KB / KB / KB)
    } else if b >= KB * KB {
        format!("{:.1} МБ", b / KB / KB)
    } else {
        format!("{:.0} КБ", b / KB)
    }
}

/// Прибирає з назви файлу символи, заборонені у Windows, і екранує `%` для шаблону yt-dlp.
pub fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    cleaned.trim().trim_end_matches('.').replace('%', "%%")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_parsing() {
        assert_eq!(parse_time(""), Ok(None));
        assert_eq!(parse_time("80"), Ok(Some(80.0)));
        assert_eq!(parse_time("1:20"), Ok(Some(80.0)));
        assert_eq!(parse_time("1:02:03"), Ok(Some(3723.0)));
        assert!(parse_time("abc").is_err());
    }

    #[test]
    fn urls() {
        let v = extract_urls("дивись https://youtu.be/abc і\nwww.facebook.com/watch?v=1");
        assert_eq!(v, vec!["https://youtu.be/abc", "https://www.facebook.com/watch?v=1"]);
    }

    #[test]
    fn filenames() {
        assert_eq!(sanitize_filename("AC/DC: Live 100%"), "AC_DC_ Live 100%%");
    }
}
