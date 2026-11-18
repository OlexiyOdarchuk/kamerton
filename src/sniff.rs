//! Пошук відеопотоку (HLS, `.m3u8`) на сторінці, де плеєр підвантажує його скриптом
//! і yt-dlp сам його не бачить.
//!
//! Сторінка відкривається у встановленому браузері (Chrome, Edge, Brave, Chromium) без вікна,
//! через Chrome DevTools Protocol. Програма слухає мережеві запити сторінки і всіх її iframe
//! і запам'ятовує перший `.m3u8` разом із заголовками, з якими його запитав плеєр.
//! Відео під DRM так не скачати: yt-dlp на ньому зупиниться, і це правильно.

use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::util;

/// Знайдений потік і те, з чим його треба запитувати.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub url: String,
    pub referer: Option<String>,
    pub user_agent: Option<String>,
    pub title: Option<String>,
    pub thumbnail: Option<String>,
}

#[derive(Debug)]
pub enum SniffError {
    NoBrowser,
    Nothing,
    Failed(String),
}

/// Браузер на основі Chromium, який уміє працювати без вікна.
pub fn find_browser() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        let roots = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"];
        let rels = [
            r"Google\Chrome\Application\chrome.exe",
            r"Microsoft\Edge\Application\msedge.exe",
            r"BraveSoftware\Brave-Browser\Application\brave.exe",
            r"Chromium\Application\chrome.exe",
        ];
        for rel in rels {
            for root in roots {
                if let Some(r) = std::env::var_os(root) {
                    candidates.push(PathBuf::from(r).join(rel));
                }
            }
        }
    } else if cfg!(target_os = "macos") {
        for app in ["Google Chrome", "Microsoft Edge", "Brave Browser", "Chromium"] {
            candidates.push(PathBuf::from(format!("/Applications/{app}.app/Contents/MacOS/{app}")));
        }
    } else {
        let names = [
            "google-chrome-stable",
            "google-chrome",
            "chromium",
            "chromium-browser",
            "microsoft-edge-stable",
            "microsoft-edge",
            "brave-browser",
            "brave",
        ];
        let path = std::env::var_os("PATH").unwrap_or_default();
        for name in names {
            for dir in std::env::split_paths(&path) {
                candidates.push(dir.join(name));
            }
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Чи схоже посилання на HLS-плейлист.
fn is_playlist_url(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or("").to_lowercase();
    path.ends_with(".m3u8") || path.contains(".m3u8/")
}

struct Cdp {
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
    next_id: u64,
}

impl Cdp {
    fn send(&mut self, method: &str, params: Value, session: Option<&str>) -> Result<u64, String> {
        self.next_id += 1;
        let mut msg = json!({ "id": self.next_id, "method": method, "params": params });
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        self.ws.send(Message::text(msg.to_string())).map_err(|e| e.to_string())?;
        Ok(self.next_id)
    }

    /// Наступне повідомлення або `None`, якщо за 200 мс нічого не прийшло.
    fn recv(&mut self) -> Result<Option<Value>, String> {
        match self.ws.read() {
            Ok(Message::Text(t)) => Ok(serde_json::from_str(t.as_str()).ok()),
            Ok(_) => Ok(None),
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) =>
            {
                Ok(None)
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// Чекає відповідь на конкретний запит, а події по дорозі віддає в `on_event`.
    fn call(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&str>,
        on_event: &mut dyn FnMut(&mut Cdp, &Value),
    ) -> Result<Value, String> {
        let id = self.send(method, params, session)?;
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            let Some(msg) = self.recv()? else { continue };
            if msg["id"].as_u64() == Some(id) {
                if let Some(err) = msg.get("error") {
                    return Err(format!("{method}: {err}"));
                }
                return Ok(msg["result"].clone());
            }
            on_event(self, &msg);
        }
        Err(format!("{method}: браузер не відповідає"))
    }
}

/// Запускає браузер без вікна і повертає адресу DevTools.
fn launch(browser: &PathBuf, profile: &PathBuf) -> Result<(std::process::Child, String), String> {
    let mut child = util::command(browser)
        .args([
            "--headless=new",
            "--disable-gpu",
            "--mute-audio",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--autoplay-policy=no-user-gesture-required",
            "--window-size=1280,720",
            "--remote-debugging-port=0",
        ])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("не вдалося запустити браузер: {e}"))?;
    // Браузер пише «DevTools listening on ws://…» у stderr.
    let stderr = child.stderr.take().ok_or("немає stderr")?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Some(i) = line.find("ws://") {
                let _ = tx.send(line[i..].trim().to_string());
            }
        }
    });
    match rx.recv_timeout(Duration::from_secs(20)) {
        Ok(url) => Ok((child, url)),
        Err(_) => {
            let _ = child.kill();
            Err("браузер не запустився без вікна".into())
        }
    }
}

/// JS, що вмикає всі відео на сторінці (без звуку): деякі плеєри просять потік лише після «Play».
const PLAY_JS: &str = "(() => { document.querySelectorAll('video').forEach(v => { v.muted = true; \
    try { v.play().catch(() => {}); } catch (e) {} }); return 1; })()";

const META_JS: &str = "JSON.stringify({ t: document.title, \
    i: (document.querySelector('meta[property=\"og:image\"]') || {}).content || null })";

/// Відкриває сторінку і шукає на ній `.m3u8`. Займає до `timeout`.
pub fn sniff(page: &str, timeout: Duration) -> Result<Found, SniffError> {
    let browser = find_browser().ok_or(SniffError::NoBrowser)?;
    let profile = std::env::temp_dir().join(format!("kamerton-sniff-{}-{}", std::process::id(), rand_suffix()));
    let (mut child, ws_url) = launch(&browser, &profile).map_err(SniffError::Failed)?;
    let result = run(&ws_url, page, timeout);
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&profile);
    result
}

fn rand_suffix() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

fn run(ws_url: &str, page: &str, timeout: Duration) -> Result<Found, SniffError> {
    let fail = |e: String| SniffError::Failed(e);
    let (ws, _) = tungstenite::connect(ws_url).map_err(|e| fail(e.to_string()))?;
    if let MaybeTlsStream::Plain(s) = ws.get_ref() {
        let _ = s.set_read_timeout(Some(Duration::from_millis(200)));
    }
    let mut cdp = Cdp { ws, next_id: 0 };

    // Без «HeadlessChrome» у User-Agent: частина сайтів такий браузер не пускає.
    let version = cdp.call("Browser.getVersion", json!({}), None, &mut |_, _| {}).map_err(fail)?;
    let user_agent = version["userAgent"].as_str().unwrap_or("").replace("HeadlessChrome", "Chrome");

    let target = cdp.call("Target.createTarget", json!({ "url": "about:blank" }), None, &mut |_, _| {}).map_err(fail)?;
    let target_id = target["targetId"].as_str().unwrap_or("").to_string();
    let attached = cdp
        .call("Target.attachToTarget", json!({ "targetId": target_id, "flatten": true }), None, &mut |_, _| {})
        .map_err(fail)?;
    let main = attached["sessionId"].as_str().unwrap_or("").to_string();

    let mut sessions: Vec<String> = vec![main.clone()];
    let mut found: Vec<Found> = Vec::new();
    // Обробник подій: нові iframe, запити до .m3u8.
    let ua = user_agent.clone();
    let on_event = move |cdp: &mut Cdp, msg: &Value, sessions: &mut Vec<String>, found: &mut Vec<Found>| {
        let params = &msg["params"];
        match msg["method"].as_str().unwrap_or("") {
            "Target.attachedToTarget" => {
                let Some(s) = params["sessionId"].as_str() else { return };
                setup_session(cdp, s, &ua);
                let _ = cdp.send("Runtime.runIfWaitingForDebugger", json!({}), Some(s));
                sessions.push(s.to_string());
            }
            "Network.requestWillBeSent" => {
                let url = params["request"]["url"].as_str().unwrap_or("");
                if is_playlist_url(url) && !found.iter().any(|f| f.url == url) {
                    let headers = &params["request"]["headers"];
                    let header = |name: &str| {
                        headers.as_object().and_then(|h| {
                            h.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).and_then(|(_, v)| v.as_str())
                        })
                    };
                    found.push(Found {
                        url: url.to_string(),
                        referer: header("Referer").map(str::to_string).or_else(|| {
                            params["documentURL"].as_str().filter(|d| d.starts_with("http")).map(str::to_string)
                        }),
                        user_agent: header("User-Agent").map(str::to_string),
                        title: None,
                        thumbnail: None,
                    });
                }
            }
            "Network.responseReceived" => {
                let resp = &params["response"];
                let url = resp["url"].as_str().unwrap_or("");
                let mime = resp["mimeType"].as_str().unwrap_or("").to_lowercase();
                if mime.contains("mpegurl") && !found.iter().any(|f| f.url == url) {
                    found.push(Found {
                        url: url.to_string(),
                        referer: None,
                        user_agent: None,
                        title: None,
                        thumbnail: None,
                    });
                }
            }
            _ => {}
        }
    };
    macro_rules! call {
        ($method:expr, $params:expr, $session:expr) => {
            cdp.call($method, $params, $session, &mut |c, m| on_event(c, m, &mut sessions, &mut found))
        };
    }

    setup_session(&mut cdp, &main, &user_agent);
    let _ = call!(
        "Emulation.setDeviceMetricsOverride",
        json!({ "width": 1280, "height": 720, "deviceScaleFactor": 1, "mobile": false }),
        Some(&main)
    );
    call!("Page.navigate", json!({ "url": page }), Some(&main)).map_err(fail)?;

    let start = Instant::now();
    let mut first_found: Option<Instant> = None;
    let mut pokes = 0;
    while start.elapsed() < timeout {
        // Знайшли: ще 2 секунди чекаємо, раптом прийде головний плейлист з якостями.
        if let Some(t) = first_found {
            if t.elapsed() > Duration::from_secs(2) {
                break;
            }
        } else if !found.is_empty() {
            first_found = Some(Instant::now());
        }
        // Нічого не видно: «натискаємо Play» на 4-й і 9-й секунді.
        let due = [4, 9];
        if found.is_empty() && pokes < due.len() && start.elapsed() > Duration::from_secs(due[pokes]) {
            pokes += 1;
            for s in sessions.clone() {
                let _ = cdp.send("Runtime.evaluate", json!({ "expression": PLAY_JS }), Some(&s));
            }
            for kind in ["mousePressed", "mouseReleased"] {
                let _ = cdp.send(
                    "Input.dispatchMouseEvent",
                    json!({ "type": kind, "x": 640, "y": 360, "button": "left", "clickCount": 1 }),
                    Some(&main),
                );
            }
        }
        match cdp.recv() {
            Ok(Some(msg)) => on_event(&mut cdp, &msg, &mut sessions, &mut found),
            Ok(None) => {}
            Err(e) => return Err(fail(e)),
        }
    }

    if found.is_empty() {
        let _ = cdp.send("Browser.close", json!({}), None);
        return Err(SniffError::Nothing);
    }

    // Назва й обкладинка зі сторінки, бо в самого .m3u8 назви немає.
    let meta = call!("Runtime.evaluate", json!({ "expression": META_JS, "returnByValue": true }), Some(&main))
        .ok()
        .and_then(|r| r["result"]["value"].as_str().and_then(|s| serde_json::from_str::<Value>(s).ok()));
    let _ = cdp.send("Browser.close", json!({}), None);

    let mut best = pick(found);
    if best.user_agent.is_none() {
        best.user_agent = Some(user_agent);
    }
    if best.referer.is_none() {
        best.referer = Some(page.to_string());
    }
    if let Some(m) = meta {
        best.title = m["t"].as_str().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
        best.thumbnail = m["i"].as_str().filter(|u| u.starts_with("http")).map(str::to_string);
    }
    Ok(best)
}

/// Налаштування сесії сторінки чи iframe: мережа, User-Agent, автопідключення вкладених iframe.
fn setup_session(cdp: &mut Cdp, session: &str, user_agent: &str) {
    let s = Some(session);
    let _ = cdp.send("Network.enable", json!({}), s);
    let _ = cdp.send("Page.enable", json!({}), s);
    let _ = cdp.send("Network.setUserAgentOverride", json!({ "userAgent": user_agent }), s);
    let _ = cdp.send(
        "Target.setAutoAttach",
        json!({ "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true }),
        s,
    );
}

/// З кількох знайдених плейлистів беремо головний (з переліком якостей), якщо він є.
fn pick(found: Vec<Found>) -> Found {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    for f in &found {
        let mut req = agent.get(&f.url);
        if let Some(r) = &f.referer {
            req = req.header("Referer", r);
        }
        if let Some(ua) = &f.user_agent {
            req = req.header("User-Agent", ua);
        }
        let master = req
            .call()
            .ok()
            .and_then(|mut r| r.body_mut().with_config().limit(2_000_000).read_to_string().ok())
            .is_some_and(|body| body.contains("#EXT-X-STREAM-INF"));
        if master {
            return f.clone();
        }
    }
    found.into_iter().next().expect("found не порожній")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlist_urls() {
        assert!(is_playlist_url("https://cdn.example.com/a/master.m3u8?token=1"));
        assert!(is_playlist_url("https://x/HLS/INDEX.M3U8"));
        assert!(!is_playlist_url("https://x/video.mp4"));
        assert!(!is_playlist_url("https://x/page?file=a.m3u8"));
    }

    /// `cargo test sniff_live -- --ignored --nocapture` (потрібен Chrome, Edge, Brave або Chromium).
    #[test]
    #[ignore]
    fn sniff_live() {
        let page = std::env::var("KAMERTON_SNIFF_URL").unwrap_or_else(|_| "https://hlsjs.video-dev.org/demo/".into());
        let f = sniff(&page, Duration::from_secs(25)).expect("не знайшло");
        println!("{f:#?}");
        assert!(is_playlist_url(&f.url));
    }
}
