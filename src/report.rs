//! Звіти про помилки розробнику.
//!
//! Звіт іде на воркер Cloudflare, який пересилає його в Telegram (токен бота лишається
//! у секретах воркера, у програмі його немає). Адреса воркера вставляється під час
//! збірки з GitHub Secrets (`KAMERTON_REPORT_URL`), тож у збірках без неї звіти
//! недоступні і програма пропонує скопіювати звіт.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;

const ENDPOINT: Option<&str> = option_env!("KAMERTON_REPORT_URL");

/// Скільки звітів можна надіслати автоматично за один запуск програми.
const AUTO_LIMIT: usize = 5;

static SENT: Mutex<Option<HashSet<String>>> = Mutex::new(None);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ReportState {
    NotSent,
    Sending,
    Sent,
    Failed,
}

#[derive(Serialize, Clone)]
pub struct Report {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub version: &'static str,
    pub os: String,
    /// Версії yt-dlp, ffmpeg, Deno і чи ввімкнено вхід через браузер.
    pub tools: String,
    pub url: String,
    /// Що качалось: «MP3 · 320 кбіт/с».
    pub summary: String,
    pub message: String,
    /// Хвіст виводу yt-dlp.
    pub log: String,
    /// Надіслано автоматично (а не кнопкою).
    pub auto: bool,
}

impl Report {
    pub fn new(tools: String, url: String, summary: String, message: String, log: String, auto: bool) -> Self {
        Self {
            kind: "kamerton-error",
            version: env!("CARGO_PKG_VERSION"),
            os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            tools: sanitize(&tools),
            url,
            summary,
            message,
            log: sanitize(&log),
            auto,
        }
    }

    /// Текстова версія, коли надсилання недоступне і звіт можна лише скопіювати.
    pub fn as_text(&self) -> String {
        format!(
            "Камертон {}, {}\n{}\nПосилання: {}\nЩо качалось: {}\nПомилка: {}\n\n--- yt-dlp ---\n{}",
            self.version, self.os, self.tools, self.url, self.summary, self.message, self.log
        )
    }
}

pub fn available() -> bool {
    ENDPOINT.is_some_and(|u| u.starts_with("https://"))
}

/// Прибирає з тексту домашню теку (у шляхах є ім'я користувача) і лишає кінець довгого логу.
pub fn sanitize(text: &str) -> String {
    let mut text = text.to_string();
    if let Some(home) = directories::BaseDirs::new().map(|b| b.home_dir().display().to_string())
        && home.len() > 3
    {
        text = text.replace(&home, "~");
        // yt-dlp на Windows інколи пише шляхи з прямими скісними.
        text = text.replace(&home.replace('\\', "/"), "~");
    }
    let count = text.chars().count();
    if count > 2500 {
        text = format!("…{}", text.chars().skip(count - 2500).collect::<String>());
    }
    text
}

/// Чи можна надіслати ще один автоматичний звіт з таким змістом (без повторів і в межах ліміту).
pub fn may_auto_send(key: &str) -> bool {
    let mut guard = SENT.lock().unwrap();
    let sent = guard.get_or_insert_with(HashSet::new);
    if sent.len() >= AUTO_LIMIT || sent.contains(key) {
        return false;
    }
    sent.insert(key.to_string());
    true
}

/// Надсилає звіт у фоні; `done` отримує результат.
pub fn send(report: Report, done: impl FnOnce(bool) + Send + 'static) {
    post(report, done);
}

/// Повідомлення про перший запуск: лише версія і система, без жодного ідентифікатора.
#[derive(Serialize)]
struct FirstLaunch {
    #[serde(rename = "type")]
    kind: &'static str,
    version: &'static str,
    os: String,
    /// Програмою вже користувались до цієї версії (є старі налаштування).
    existing: bool,
}

/// Один раз повідомляє про перший запуск. Позначка `marker` з'являється лише після
/// успішного надсилання, тож без інтернету спроба повториться наступного разу.
pub fn first_launch(marker: PathBuf, existing: bool) {
    if marker.exists() {
        return;
    }
    let msg = FirstLaunch {
        kind: "kamerton-install",
        version: env!("CARGO_PKG_VERSION"),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        existing,
    };
    post(msg, move |ok| {
        if ok {
            let _ = std::fs::write(&marker, env!("CARGO_PKG_VERSION"));
        }
    });
}

fn post<T: Serialize + Send + 'static>(body: T, done: impl FnOnce(bool) + Send + 'static) {
    let Some(endpoint) = ENDPOINT.filter(|_| available()) else {
        done(false);
        return;
    };
    std::thread::spawn(move || {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .build()
            .into();
        let ok = serde_json::to_string(&body).is_ok_and(|body| {
            agent.post(endpoint).header("Content-Type", "application/json").send(body.as_str()).is_ok()
        });
        done(ok);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_home_and_limits_length() {
        let home = directories::BaseDirs::new().unwrap().home_dir().display().to_string();
        let s = sanitize(&format!("ERROR: {home}/Music/x.mp3"));
        assert!(!s.contains(&home));
        assert!(s.contains("~/Music/x.mp3"));
        assert!(sanitize(&"а".repeat(5000)).chars().count() <= 2501);
    }

    #[test]
    fn auto_limit() {
        assert!(may_auto_send("a"));
        assert!(!may_auto_send("a"));
        for i in 0..10 {
            may_auto_send(&i.to_string());
        }
        assert!(!may_auto_send("ще один"));
    }

    #[test]
    fn json_shape() {
        let r = Report::new("yt-dlp 1".into(), "https://x".into(), "MP3".into(), "збій".into(), "log".into(), true);
        let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(v["type"], "kamerton-error");
        assert_eq!(v["auto"], true);
    }
}

/// Живий тест: надсилає один звіт через справжній воркер.
/// `KAMERTON_REPORT_URL=https://… cargo test report_live -- --ignored`
#[cfg(test)]
mod live {
    #[test]
    #[ignore]
    fn report_live() {
        assert!(super::available(), "збірка без KAMERTON_REPORT_URL");
        let r = super::Report::new(
            "тест".into(),
            "https://example.com/тест".into(),
            "тест".into(),
            "Тестовий звіт, перевірка зв'язку".into(),
            "ERROR: це лише тест".into(),
            false,
        );
        let (tx, rx) = std::sync::mpsc::channel();
        super::send(r, move |ok| tx.send(ok).unwrap());
        assert!(rx.recv_timeout(std::time::Duration::from_secs(30)).unwrap(), "воркер не прийняв звіт");
    }

    /// `KAMERTON_REPORT_URL=https://… cargo test first_launch_live -- --ignored`
    #[test]
    #[ignore]
    fn first_launch_live() {
        assert!(super::available(), "збірка без KAMERTON_REPORT_URL");
        let marker = std::env::temp_dir().join(format!("kamerton-first-launch-{}", std::process::id()));
        super::first_launch(marker.clone(), false);
        let start = std::time::Instant::now();
        while !marker.exists() && start.elapsed().as_secs() < 30 {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        assert!(marker.exists(), "воркер не прийняв повідомлення");
        let _ = std::fs::remove_file(marker);
    }
}
