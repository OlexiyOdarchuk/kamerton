//! Налаштування (зберігаються в settings.json) та історія завантажень (history.json).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Video,
    Audio,
    VideoNoAudio,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum AudioFormat {
    Original,
    Mp3,
    M4a,
    Opus,
    Flac,
    Wav,
}

impl AudioFormat {
    pub const ALL: [AudioFormat; 6] = [
        AudioFormat::Mp3,
        AudioFormat::M4a,
        AudioFormat::Opus,
        AudioFormat::Flac,
        AudioFormat::Wav,
        AudioFormat::Original,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AudioFormat::Original => "Оригінал (без конвертації)",
            AudioFormat::Mp3 => "MP3",
            AudioFormat::M4a => "M4A (AAC)",
            AudioFormat::Opus => "Opus",
            AudioFormat::Flac => "FLAC (без втрат)",
            AudioFormat::Wav => "WAV (без стиснення)",
        }
    }

    /// Значення для `--audio-format` у yt-dlp.
    pub fn ytdlp_name(self) -> &'static str {
        match self {
            AudioFormat::Original => "best",
            AudioFormat::Mp3 => "mp3",
            AudioFormat::M4a => "m4a",
            AudioFormat::Opus => "opus",
            AudioFormat::Flac => "flac",
            AudioFormat::Wav => "wav",
        }
    }

    /// Доступні бітрейти (кбіт/с). `0` означає VBR. Порожньо — формат без бітрейту.
    pub fn bitrates(self) -> &'static [u32] {
        match self {
            AudioFormat::Mp3 => &[0, 320, 256, 192, 128],
            AudioFormat::M4a => &[320, 256, 192, 128],
            AudioFormat::Opus => &[256, 192, 160, 128, 96],
            _ => &[],
        }
    }
}

pub fn bitrate_label(b: u32) -> String {
    if b == 0 { "VBR, найвища якість (~245)".into() } else { format!("{b} кбіт/с") }
}

pub const VIDEO_HEIGHTS: [u32; 7] = [0, 2160, 1440, 1080, 720, 480, 360];

pub fn height_label(h: u32) -> String {
    match h {
        0 => "Найкраща".into(),
        2160 => "4K (2160p)".into(),
        h => format!("{h}p"),
    }
}


pub fn sample_rate_label(r: u32) -> String {
    match r {
        0 => "Як в оригіналі".into(),
        r => format!("{:.1} кГц", r as f64 / 1000.0).replace(".0 ", " "),
    }
}

/// Бітрейт звуку всередині відео. `0` — не чіпати (оригінал).
pub const VIDEO_AUDIO_BITRATES: [u32; 5] = [0, 320, 256, 192, 128];

/// Браузери, з яких yt-dlp уміє брати вхід: (ключ, назва, де лежить профіль відносно домашньої теки).
/// Firefox і його похідні першими: на Windows Chrome і Edge шифрують вхід, і його часто не прочитати.
/// Похідні Firefox (Zen, LibreWolf, Floorp, Waterfox) yt-dlp не знає на ім'я, тому для них
/// передається `firefox:<тека профілів>` (див. `cookies_arg`).
const BROWSER_PROFILES: [(&str, &str, &[&str]); 12] = [
    (
        "firefox",
        "Firefox",
        &[
            "AppData/Roaming/Mozilla/Firefox",
            ".mozilla/firefox",
            "snap/firefox/common/.mozilla/firefox",
            ".var/app/org.mozilla.firefox/.mozilla/firefox",
            "Library/Application Support/Firefox",
        ],
    ),
    (
        "zen",
        "Zen",
        &[
            "AppData/Roaming/zen",
            ".zen",
            ".var/app/app.zen_browser.zen/.zen",
            "Library/Application Support/zen",
        ],
    ),
    (
        "librewolf",
        "LibreWolf",
        &[
            "AppData/Roaming/librewolf",
            ".librewolf",
            ".var/app/io.gitlab.librewolf-community/.librewolf",
            "Library/Application Support/librewolf",
        ],
    ),
    ("floorp", "Floorp", &["AppData/Roaming/Floorp", ".floorp", "Library/Application Support/Floorp"]),
    ("waterfox", "Waterfox", &["AppData/Roaming/Waterfox", ".waterfox", "Library/Application Support/Waterfox"]),
    (
        "chrome",
        "Chrome",
        &["AppData/Local/Google/Chrome/User Data", ".config/google-chrome", "Library/Application Support/Google/Chrome"],
    ),
    (
        "edge",
        "Edge",
        &["AppData/Local/Microsoft/Edge/User Data", ".config/microsoft-edge", "Library/Application Support/Microsoft Edge"],
    ),
    (
        "opera",
        "Opera",
        &["AppData/Roaming/Opera Software/Opera Stable", ".config/opera", "Library/Application Support/com.operasoftware.Opera"],
    ),
    (
        "brave",
        "Brave",
        &[
            "AppData/Local/BraveSoftware/Brave-Browser/User Data",
            ".config/BraveSoftware/Brave-Browser",
            "Library/Application Support/BraveSoftware/Brave-Browser",
        ],
    ),
    (
        "vivaldi",
        "Vivaldi",
        &["AppData/Local/Vivaldi/User Data", ".config/vivaldi", "Library/Application Support/Vivaldi"],
    ),
    ("chromium", "Chromium", &[".config/chromium", "Library/Application Support/Chromium"]),
    ("safari", "Safari", &["Library/Safari"]),
];

/// Встановлені браузери (ключ, назва) у порядку надійності.
pub fn installed_browsers() -> Vec<(&'static str, &'static str)> {
    let Some(home) = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()) else {
        return Vec::new();
    };
    BROWSER_PROFILES
        .iter()
        .filter(|(_, _, dirs)| dirs.iter().any(|d| home.join(d).is_dir()))
        .map(|(key, name, _)| (*key, *name))
        .collect()
}

/// Значення для `--cookies-from-browser`. Для похідних Firefox: `firefox:<тека профілів>`,
/// yt-dlp сам знайде в ній найсвіжіший cookies.sqlite.
pub fn cookies_arg(key: &str) -> Option<String> {
    const FIREFOX_FORKS: [&str; 4] = ["zen", "librewolf", "floorp", "waterfox"];
    if key.is_empty() {
        return None;
    }
    if !FIREFOX_FORKS.contains(&key) {
        return Some(key.to_string());
    }
    let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
    let (_, _, dirs) = BROWSER_PROFILES.iter().find(|(k, _, _)| *k == key)?;
    let dir = dirs.iter().map(|d| home.join(d)).find(|p| p.is_dir())?;
    Some(format!("firefox:{}", dir.display()))
}

pub fn browser_name(key: &str) -> &'static str {
    BROWSER_PROFILES.iter().find(|(k, _, _)| *k == key).map(|(_, n, _)| *n).unwrap_or("браузер")
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Settings {
    pub folder: PathBuf,
    pub mode: Mode,
    /// Максимальна висота відео, 0 — найкраща доступна.
    pub max_height: u32,
    /// Віддавати перевагу H.264/AAC у MP4, щоб файл відкривався всюди.
    pub compatible: bool,
    pub video_audio_bitrate: u32,
    pub audio_format: AudioFormat,
    pub audio_bitrate: u32,
    pub sample_rate: u32,
    pub normalize: bool,
    pub embed_meta: bool,
    pub whole_playlist: bool,
    pub cookies_browser: String,
    pub parallel: usize,
    pub theme: ThemeChoice,
    /// Автоматично надсилати звіти про помилки розробнику.
    pub send_reports: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeChoice {
    /// Як у системі.
    Auto,
    Light,
    Dark,
}

impl Default for Settings {
    fn default() -> Self {
        let folder = directories::UserDirs::new()
            .and_then(|u| u.download_dir().map(Path::to_path_buf).or(Some(u.home_dir().to_path_buf())))
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            folder,
            mode: Mode::Video,
            max_height: 1080,
            compatible: true,
            video_audio_bitrate: 0,
            audio_format: AudioFormat::Mp3,
            audio_bitrate: 320,
            sample_rate: 0,
            normalize: false,
            embed_meta: true,
            whole_playlist: false,
            cookies_browser: String::new(),
            parallel: 2,
            theme: ThemeChoice::Auto,
            send_reports: true,
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, s);
        }
    }

    /// Короткий опис того, що буде скачано: «MP3 · 320 кбіт/с».
    pub fn summary(&self) -> String {
        match self.mode {
            Mode::Video | Mode::VideoNoAudio => {
                let mut s = format!("Відео · {}", height_label(self.max_height));
                if self.mode == Mode::VideoNoAudio {
                    s.push_str(" · без звуку");
                } else if self.video_audio_bitrate > 0 {
                    s.push_str(&format!(" · звук {} кбіт/с", self.video_audio_bitrate));
                }
                if self.compatible {
                    s.push_str(" · MP4");
                }
                s
            }
            Mode::Audio => {
                let f = self.audio_format;
                let mut s = match f {
                    AudioFormat::Original => "Звук · оригінал".to_string(),
                    _ => f.label().split(' ').next().unwrap_or("").to_string(),
                };
                if !f.bitrates().is_empty() {
                    s.push_str(" · ");
                    s.push_str(&bitrate_label(self.audio_bitrate));
                }
                if f != AudioFormat::Original && self.sample_rate > 0 {
                    s.push_str(" · ");
                    s.push_str(&sample_rate_label(self.sample_rate));
                }
                if self.normalize && f != AudioFormat::Original {
                    s.push_str(" · −14 LUFS");
                }
                s
            }
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HistoryEntry {
    pub title: String,
    pub url: String,
    pub path: Option<PathBuf>,
    pub files: usize,
    pub when: String,
    pub summary: String,
}

pub fn load_history(path: &Path) -> Vec<HistoryEntry> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_history(path: &Path, history: &[HistoryEntry]) {
    if let Ok(s) = serde_json::to_string_pretty(history) {
        let _ = std::fs::write(path, s);
    }
}
