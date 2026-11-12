//! Одне завантаження (картка в списку): отримання інформації про відео і сам процес yt-dlp.

use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use eframe::egui;
use serde_json::Value;

use crate::components::Tools;
use crate::settings::{AudioFormat, HistoryEntry, Mode, Settings};
use crate::util;

pub struct MediaInfo {
    pub title: String,
    pub uploader: Option<String>,
    pub duration: Option<f64>,
    pub is_playlist: bool,
    pub entries: usize,
    pub max_height: Option<u32>,
    /// Найкращий звук у джерелі, наприклад «Opus ~131 кбіт/с».
    pub source_audio: Option<String>,
    pub thumbnail_url: Option<String>,
}

#[derive(Clone, PartialEq)]
pub enum Status {
    /// Чекає черги на отримання інформації.
    NeedInfo,
    FetchingInfo,
    /// Інформація є, можна качати.
    Ready,
    InfoFailed(Failure),
    /// Користувач натиснув «Завантажити», чекає вільного місця.
    Queued,
    Downloading,
    Processing(String),
    Done,
    Failed(Failure),
    Cancelled,
}

impl Status {
    pub fn is_active(&self) -> bool {
        matches!(self, Status::Downloading | Status::Processing(_))
    }
}

#[derive(Clone, PartialEq)]
pub struct Failure {
    /// Зрозуміле пояснення.
    pub message: String,
    /// Оригінальний текст помилки yt-dlp (для підказки).
    pub details: String,
    pub kind: FailureKind,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FailureKind {
    Other,
    /// Сайт пускає лише після входу: картка пропонує взяти вхід з браузера.
    NeedsLogin,
    /// Вхід з вибраного браузера прочитати не вдалося: картка пропонує інший браузер.
    CookiesUnreadable,
}

impl Failure {
    pub fn other(message: impl Into<String>, details: impl Into<String>) -> Self {
        Self { message: message.into(), details: details.into(), kind: FailureKind::Other }
    }
}

#[derive(Default, Clone)]
pub struct Progress {
    pub fraction: Option<f32>,
    pub speed: Option<f64>,
    pub eta: Option<f64>,
    pub total: Option<f64>,
    /// Номер відео в плейлисті та їх кількість.
    pub item: Option<(u32, u32)>,
}

pub struct Job {
    pub id: u64,
    pub url: String,
    pub info: Option<MediaInfo>,
    pub thumb: Option<Arc<[u8]>>,
    pub file_name: String,
    pub trim_open: bool,
    pub trim_from: String,
    pub trim_to: String,
    pub status: Status,
    pub progress: Progress,
    /// Що саме качається («MP3 · 320 кбіт/с»), заповнюється при старті.
    pub summary: String,
    pub files: Vec<PathBuf>,
    pub note: Option<String>,
    pub cancel: Arc<AtomicBool>,
    pub pid: Option<u32>,
}

pub type JobRef = Arc<Mutex<Job>>;

impl Job {
    pub fn new(id: u64, url: String) -> JobRef {
        Arc::new(Mutex::new(Job {
            id,
            url,
            info: None,
            thumb: None,
            file_name: String::new(),
            trim_open: false,
            trim_from: String::new(),
            trim_to: String::new(),
            status: Status::NeedInfo,
            progress: Progress::default(),
            summary: String::new(),
            files: Vec::new(),
            note: None,
            cancel: Arc::new(AtomicBool::new(false)),
            pid: None,
        }))
    }

    pub fn title(&self) -> String {
        self.info.as_ref().map(|i| i.title.clone()).unwrap_or_else(|| self.url.clone())
    }

    /// Перевіряє поля обрізки. `Ok(None)` — обрізка не потрібна.
    pub fn trim_range(&self) -> Result<Option<(f64, Option<f64>)>, &'static str> {
        if !self.trim_open {
            return Ok(None);
        }
        let from = util::parse_time(&self.trim_from).map_err(|_| "Неправильний час початку")?;
        let to = util::parse_time(&self.trim_to).map_err(|_| "Неправильний час кінця")?;
        if from.is_none() && to.is_none() {
            return Ok(None);
        }
        let from = from.unwrap_or(0.0);
        if let Some(to) = to
            && to <= from
        {
            return Err("Кінець має бути пізніше за початок");
        }
        Ok(Some((from, to)))
    }

    pub fn stop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        if let Some(pid) = self.pid {
            util::kill_tree(pid);
        }
        if !self.status.is_active() {
            self.status = Status::Cancelled;
        }
    }
}

/// Аргументи, спільні для отримання інформації і завантаження.
fn base_args(tools: &Tools, s: &Settings) -> Vec<OsString> {
    let mut a: Vec<OsString> = vec![
        "--ignore-config".into(),
        "--color".into(),
        "never".into(),
        "--ffmpeg-location".into(),
        tools.bin.clone().into(),
        "--js-runtimes".into(),
        {
            let mut v = OsString::from("deno:");
            v.push(&tools.deno);
            v
        },
    ];
    if !s.whole_playlist {
        a.push("--no-playlist".into());
    }
    if let Some(browser) = crate::settings::cookies_arg(&s.cookies_browser) {
        a.push("--cookies-from-browser".into());
        a.push(browser.into());
    }
    a
}

fn ytdlp_command(tools: &Tools, args: &[OsString]) -> std::process::Command {
    let mut cmd = util::command(&tools.ytdlp);
    cmd.args(args)
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd
}

/// Перетворює текст помилки yt-dlp на щось зрозуміле людині.
fn explain(stderr: &str) -> Failure {
    let first_error = stderr
        .lines()
        .rev()
        .find(|l| l.starts_with("ERROR:"))
        .or_else(|| stderr.lines().rev().find(|l| !l.trim().is_empty()))
        .unwrap_or("Невідома помилка")
        .trim()
        .to_string();
    let e = stderr.to_lowercase();
    let has = |s: &str| e.contains(s);
    let login = |m: &str| Failure { message: m.into(), details: first_error.clone(), kind: FailureKind::NeedsLogin };
    // Chrome/Edge на Windows шифрують вхід (yt-dlp issue 10927): «Failed to decrypt with DPAPI».
    if has("dpapi")
        || has("app-bound")
        || (has("cookie")
            && (has("decrypt") || has("could not copy") || has("could not find") || has("failed to load")
                || has("unsupported browser") || has("permission denied") || has("database is locked")))
    {
        return Failure {
            message: "Не вдалося взяти вхід з браузера.".into(),
            details: first_error,
            kind: FailureKind::CookiesUnreadable,
        };
    }
    if has("sign in to confirm your age") || has("age-restricted") || has("inappropriate for some users") {
        return login("Відео з віковим обмеженням, потрібен вхід в акаунт.");
    }
    if has("sign in to confirm") || has("not a bot") {
        return login("YouTube просить підтвердити, що ви не бот.");
    }
    if has("private video") || has("members-only") || has("join this channel") {
        return login("Відео доступне лише тим, кому дали доступ.");
    }
    if has("login required") || has("log in") || has("login") || has("sign in") || has("this content isn't available")
        || has("registered users") || has("cookies")
    {
        return login("Сайт показує це лише після входу в акаунт.");
    }
    let message = if has("unsupported url") {
        "Цей сайт або тип посилання не підтримується."
    } else if has("requested format is not available") {
        "Потрібної якості немає. Виберіть іншу якість."
    } else if has("video unavailable") || has("has been removed") || has("http error 404") {
        "Відео недоступне або видалене."
    } else if has("http error 403") {
        "Сайт тимчасово не віддав файл (403). Натисніть «Повторити» трохи згодом."
    } else if has("getaddrinfo") || has("name or service not known") || has("temporary failure in name resolution")
        || has("unable to connect") || has("network is unreachable") || has("timed out")
    {
        "Немає з'єднання з інтернетом або сайт не відповідає."
    } else if has("no space left") {
        "На диску закінчилось місце."
    } else if has("permission denied") || has("access is denied") {
        "Немає доступу до папки збереження. Виберіть іншу папку."
    } else {
        "Не вдалося скачати."
    };
    Failure::other(message, first_error)
}

fn codec_name(acodec: &str) -> String {
    let a = acodec.to_lowercase();
    if a.starts_with("opus") {
        "Opus".into()
    } else if a.starts_with("mp4a") || a == "aac" {
        "AAC".into()
    } else if a.starts_with("vorbis") {
        "Vorbis".into()
    } else if a.starts_with("mp3") {
        "MP3".into()
    } else if a.starts_with("flac") {
        "FLAC".into()
    } else {
        acodec.to_uppercase()
    }
}

fn parse_info(v: &Value) -> MediaInfo {
    let s = |k: &str| v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    let is_playlist = s("_type").as_deref() == Some("playlist");
    let entries = v.get("entries").and_then(Value::as_array);
    let formats: &[Value] = v.get("formats").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
    let fs = |f: &Value, k: &str| f.get(k).and_then(Value::as_str).unwrap_or("").to_string();

    let max_height = formats
        .iter()
        .filter(|f| fs(f, "vcodec") != "none")
        .filter_map(|f| f.get("height").and_then(Value::as_u64))
        .max()
        .map(|h| h as u32);

    let best_audio = formats
        .iter()
        .filter(|f| {
            let a = fs(f, "acodec");
            !a.is_empty() && a != "none" && fs(f, "vcodec") == "none"
        })
        .max_by(|a, b| {
            let abr = |f: &Value| f.get("abr").and_then(Value::as_f64).unwrap_or(0.0);
            abr(a).total_cmp(&abr(b))
        });
    let source_audio = best_audio.map(|f| {
        let codec = codec_name(&fs(f, "acodec"));
        match f.get("abr").and_then(Value::as_f64) {
            Some(abr) if abr > 0.0 => format!("{codec} ~{} кбіт/с", abr.round()),
            _ => codec,
        }
    });

    let thumb_of = |v: &Value| -> Option<String> {
        v.get("thumbnail").and_then(Value::as_str).map(str::to_string).or_else(|| {
            v.get("thumbnails")
                .and_then(Value::as_array)
                .and_then(|t| t.iter().rev().find_map(|t| t.get("url").and_then(Value::as_str)))
                .map(str::to_string)
        })
    };
    let thumbnail_url = thumb_of(v).or_else(|| entries.and_then(|e| e.first()).and_then(thumb_of));

    MediaInfo {
        title: s("title").or_else(|| s("fulltitle")).unwrap_or_else(|| "Без назви".into()),
        uploader: s("uploader").or_else(|| s("channel")).or_else(|| s("uploader_id")),
        duration: v.get("duration").and_then(Value::as_f64),
        is_playlist,
        entries: entries
            .map(Vec::len)
            .or_else(|| v.get("playlist_count").and_then(Value::as_u64).map(|n| n as usize))
            .unwrap_or(0),
        max_height,
        source_audio,
        thumbnail_url,
    }
}

fn fetch_thumbnail(url: &str) -> Option<Arc<[u8]>> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(20)))
        .build()
        .into();
    let mut resp = agent.get(url).call().ok()?;
    let bytes = resp.body_mut().with_config().limit(10_000_000).read_to_vec().ok()?;
    Some(crop_16_9(&bytes).unwrap_or(bytes).into())
}

/// Обрізає обкладинку по центру до 16:9 і зменшує, щоб усі картки були однакові.
/// Старі відео мають обкладинки 4:3 з чорними смугами, вони б стояли з відступом.
fn crop_16_9(bytes: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(bytes).ok()?;
    let (w, h) = (img.width(), img.height());
    let (cw, ch) = if w * 9 > h * 16 { (h * 16 / 9, h) } else { (w, w * 9 / 16) };
    let cropped = img.crop_imm((w - cw) / 2, (h - ch) / 2, cw, ch);
    let small = cropped.resize_exact(352, 198, image::imageops::FilterType::Triangle);
    let mut out = std::io::Cursor::new(Vec::new());
    small.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

/// Отримує назву, тривалість, доступні якості та обкладинку.
pub fn spawn_fetch_info(job: JobRef, tools: Tools, settings: Settings, ctx: egui::Context) {
    let url = {
        let mut j = job.lock().unwrap();
        j.status = Status::FetchingInfo;
        j.url.clone()
    };
    std::thread::spawn(move || {
        let mut args = base_args(&tools, &settings);
        args.extend(["-J".into(), "--flat-playlist".into(), "--".into(), url.into()]);
        let output = ytdlp_command(&tools, &args).output();
        let result = match output {
            Ok(out) if out.status.success() => serde_json::from_slice::<Value>(&out.stdout)
                .map(|v| parse_info(&v))
                .map_err(|e| Failure::other("Не вдалося розібрати відповідь yt-dlp.", e.to_string())),
            Ok(out) => Err(explain(&String::from_utf8_lossy(&out.stderr))),
            Err(e) => Err(Failure::other("Не вдалося запустити yt-dlp.", e.to_string())),
        };
        let thumb_url = match &result {
            Ok(info) => info.thumbnail_url.clone(),
            Err(_) => None,
        };
        {
            let mut j = job.lock().unwrap();
            match result {
                Ok(info) => {
                    if j.file_name.is_empty() && !info.is_playlist {
                        j.file_name = info.title.clone();
                    }
                    j.info = Some(info);
                    j.status = Status::Ready;
                }
                Err(f) => j.status = Status::InfoFailed(f),
            }
        }
        ctx.request_repaint();
        if let Some(u) = thumb_url
            && let Some(bytes) = fetch_thumbnail(&u)
        {
            job.lock().unwrap().thumb = Some(bytes);
            ctx.request_repaint();
        }
    });
}

/// Аргументи формату та обробки залежно від налаштувань.
fn format_args(s: &Settings, trim: Option<(f64, Option<f64>)>) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    let mut push = |items: &[&str]| a.extend(items.iter().map(|s| s.to_string()));
    let h = if s.max_height > 0 { format!("[height<={}]", s.max_height) } else { String::new() };

    match s.mode {
        Mode::Video => {
            push(&["-f", &format!("bv*{h}+ba/b{h}/bv*+ba/b")]);
            if s.compatible {
                push(&["-S", "vcodec:h264,res,acodec:aac", "--merge-output-format", "mp4"]);
            }
            if s.embed_meta {
                push(&["--embed-metadata"]);
                if s.compatible {
                    push(&["--embed-thumbnail", "--convert-thumbnails", "jpg"]);
                }
            }
        }
        Mode::VideoNoAudio => {
            push(&["-f", &format!("bv{h}/bv*{h}/bv/b")]);
            if s.compatible {
                push(&["-S", "vcodec:h264,res", "--remux-video", "mp4"]);
            }
            if s.embed_meta {
                push(&["--embed-metadata"]);
            }
        }
        Mode::Audio => {
            let f = s.audio_format;
            push(&["-f", "ba/b", "-x", "--audio-format", f.ytdlp_name()]);
            let rates = f.bitrates();
            if !rates.is_empty() {
                let b = if rates.contains(&s.audio_bitrate) { s.audio_bitrate } else { rates[0] };
                let q = if b == 0 { "0".to_string() } else { format!("{b}K") };
                push(&["--audio-quality", &q]);
            }
            if f != AudioFormat::Original {
                let mut pp = Vec::new();
                if s.normalize {
                    pp.push("-af loudnorm=I=-14:TP=-1.5:LRA=11".to_string());
                }
                // loudnorm внутрішньо працює на 192 кГц, тому частоту треба задати явно.
                let rate = if s.sample_rate > 0 { s.sample_rate } else if s.normalize { 48000 } else { 0 };
                if rate > 0 {
                    pp.push(format!("-ar {rate}"));
                }
                if !pp.is_empty() {
                    push(&["--postprocessor-args", &format!("ExtractAudio:{}", pp.join(" "))]);
                }
            }
            if s.embed_meta && f != AudioFormat::Wav {
                push(&["--embed-metadata", "--embed-thumbnail", "--convert-thumbnails", "jpg"]);
            }
        }
    }

    if let Some((from, to)) = trim {
        let to = to.map(|t| t.to_string()).unwrap_or_else(|| "inf".into());
        // Без --force-keyframes-at-cuts розріз іде по найближчій точці пошуку
        // і фрагмент виходить на кілька секунд довшим.
        push(&["--download-sections", &format!("*{from}-{to}"), "--force-keyframes-at-cuts"]);
    }
    a
}

/// Додаткова обробка через ffmpeg після yt-dlp: зміна бітрейту звуку у відео
/// або видалення звукової доріжки. Файл замінюється на місці.
fn ffmpeg_post(tools: &Tools, path: &Path, s: &Settings) -> Result<(), String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let codec_args: Vec<String> = match s.mode {
        Mode::VideoNoAudio => vec!["-an".into()],
        Mode::Video if s.video_audio_bitrate > 0 => {
            let codec = if ext == "webm" { "libopus" } else { "aac" };
            vec!["-c:a".into(), codec.into(), "-b:a".into(), format!("{}k", s.video_audio_bitrate)]
        }
        _ => return Ok(()),
    };
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("video");
    let tmp = path.with_file_name(format!("{stem}.kamerton-tmp.{ext}"));
    let out = util::command(&tools.ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(path)
        .args(["-map", "0:v", "-map", "0:a?", "-c", "copy"])
        .args(&codec_args)
        .arg(&tmp)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn pp_label(name: &str) -> &'static str {
    match name {
        "Merger" => "Склеювання відео і звуку…",
        "ExtractAudio" => "Конвертація звуку…",
        "EmbedThumbnail" | "ThumbnailsConvertor" => "Додавання обкладинки…",
        "Metadata" => "Запис тегів…",
        "MoveFiles" => "Збереження…",
        "VideoRemuxer" => "Перепакування…",
        _ => "Обробка…",
    }
}

fn num(s: Option<&str>) -> Option<f64> {
    s.and_then(|s| s.parse::<f64>().ok())
}

/// Запускає завантаження. Налаштування беруться на момент старту.
pub fn spawn_download(
    job: JobRef,
    tools: Tools,
    settings: Settings,
    tmp_root: PathBuf,
    ctx: egui::Context,
    done_tx: Sender<HistoryEntry>,
) {
    let (url, id, template, trim, cancel) = {
        let mut j = job.lock().unwrap();
        let is_playlist = j.info.as_ref().is_some_and(|i| i.is_playlist);
        let template = if is_playlist {
            "%(playlist_title|Плейлист)s/%(playlist_index)03d - %(title)s.%(ext)s".to_string()
        } else {
            let name = util::sanitize_filename(&j.file_name);
            let name = if name.is_empty() { "%(title)s".to_string() } else { name };
            format!("{name}.%(ext)s")
        };
        let trim = j.trim_range().ok().flatten();
        j.cancel.store(false, Ordering::SeqCst);
        j.status = Status::Downloading;
        j.progress = Progress::default();
        j.files.clear();
        j.note = None;
        j.summary = settings.summary();
        if trim.is_some() {
            j.summary.push_str(" · фрагмент");
        }
        (j.url.clone(), j.id, template, trim, j.cancel.clone())
    };

    std::thread::spawn(move || {
        let tmp_dir = tmp_root.join(format!("{}-{id}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp_dir);
        let _ = std::fs::create_dir_all(&settings.folder);

        let mut args = base_args(&tools, &settings);
        let mut temp_path = OsString::from("temp:");
        temp_path.push(&tmp_dir);
        let mut home_path = OsString::from("home:");
        home_path.push(&settings.folder);
        args.extend([
            "--newline".into(),
            "--progress".into(),
            "--no-mtime".into(),
            "--retries".into(),
            "10".into(),
            "--fragment-retries".into(),
            "10".into(),
            "--progress-template".into(),
            "download:KACHPROG %(progress.downloaded_bytes)s %(progress.total_bytes)s \
             %(progress.total_bytes_estimate)s %(progress.speed)s %(progress.eta)s \
             %(info.playlist_index)s %(info.n_entries)s"
                .into(),
            "--progress-template".into(),
            "postprocess:KACHPP %(progress.postprocessor)s".into(),
            "--print".into(),
            "after_move:KACHFILE %(filepath)s".into(),
            "-P".into(),
            home_path,
            "-P".into(),
            temp_path,
            "-o".into(),
            template.into(),
        ]);
        args.extend(format_args(&settings, trim).into_iter().map(OsString::from));
        args.extend(["--".into(), url.clone().into()]);

        let finish = |status: Status| {
            job.lock().unwrap().status = status;
            ctx.request_repaint();
        };

        let mut child = match ytdlp_command(&tools, &args).spawn() {
            Ok(c) => c,
            Err(e) => {
                finish(Status::Failed(Failure::other("Не вдалося запустити yt-dlp.", e.to_string())));
                return;
            }
        };
        job.lock().unwrap().pid = Some(child.id());

        // stderr читаємо окремо, щоб процес не завис на заповненому буфері.
        let stderr = child.stderr.take().unwrap();
        let err_thread = std::thread::spawn(move || {
            let mut lines: Vec<String> = Vec::new();
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                lines.push(line);
                if lines.len() > 200 {
                    lines.drain(..100);
                }
            }
            lines.join("\n")
        });

        let stdout = child.stdout.take().unwrap();
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let mut j = job.lock().unwrap();
            if let Some(rest) = line.strip_prefix("KACHPROG ") {
                let p: Vec<&str> = rest.split_whitespace().collect();
                let done = num(p.first().copied());
                let total = num(p.get(1).copied()).or(num(p.get(2).copied()));
                j.progress.total = total;
                j.progress.fraction = match (done, total) {
                    (Some(d), Some(t)) if t > 0.0 => Some((d / t).clamp(0.0, 1.0) as f32),
                    _ => None,
                };
                j.progress.speed = num(p.get(3).copied());
                j.progress.eta = num(p.get(4).copied());
                j.progress.item = match (num(p.get(5).copied()), num(p.get(6).copied())) {
                    (Some(i), Some(n)) => Some((i as u32, n as u32)),
                    _ => None,
                };
                if !cancel.load(Ordering::SeqCst) {
                    j.status = Status::Downloading;
                }
            } else if let Some(name) = line.strip_prefix("KACHPP ") {
                if !cancel.load(Ordering::SeqCst) {
                    j.status = Status::Processing(pp_label(name.trim()).into());
                }
            } else if let Some(path) = line.strip_prefix("KACHFILE ") {
                let p = PathBuf::from(path.trim());
                if !j.files.contains(&p) {
                    j.files.push(p);
                }
            }
            drop(j);
            ctx.request_repaint();
        }

        let exit = child.wait();
        let stderr_text = err_thread.join().unwrap_or_default();
        job.lock().unwrap().pid = None;
        let _ = std::fs::remove_dir_all(&tmp_dir);

        if cancel.load(Ordering::SeqCst) {
            finish(Status::Cancelled);
            return;
        }
        let ok = exit.map(|s| s.success()).unwrap_or(false);
        let files = job.lock().unwrap().files.clone();
        if !ok && files.is_empty() {
            finish(Status::Failed(explain(&stderr_text)));
            return;
        }

        // Своя обробка ffmpeg (бітрейт звуку у відео / прибрати звук).
        let needs_post = settings.mode == Mode::VideoNoAudio
            || (settings.mode == Mode::Video && settings.video_audio_bitrate > 0);
        let mut note = None;
        if needs_post {
            job.lock().unwrap().status = Status::Processing(if settings.mode == Mode::VideoNoAudio {
                "Видалення звукової доріжки…".into()
            } else {
                "Зміна бітрейту звуку…".into()
            });
            ctx.request_repaint();
            for f in &files {
                if let Err(e) = ffmpeg_post(&tools, f, &settings) {
                    note = Some(format!("Файл збережено, але обробка не вдалася: {e}"));
                }
            }
        }
        if !ok {
            // Частина плейлиста скачалась, частина — ні.
            note = Some(format!("Скачано не все: {}", explain(&stderr_text).message));
        }

        let entry = {
            let mut j = job.lock().unwrap();
            j.note = note;
            j.status = Status::Done;
            HistoryEntry {
                title: j.title(),
                url: j.url.clone(),
                path: j.files.first().cloned(),
                files: j.files.len(),
                when: chrono::Local::now().format("%d.%m.%Y %H:%M").to_string(),
                summary: j.summary.clone(),
            }
        };
        let _ = done_tx.send(entry);
        ctx.request_repaint();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Повна перевірка з мережею: ставить компоненти і качає коротке відео.
    /// Запуск: `cargo test -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn end_to_end() {
        use crate::components::{State, spawn_update};
        let root = std::env::current_dir().unwrap().join("target/e2e");
        let out = root.join("out");
        let _ = std::fs::remove_dir_all(&out);
        let tools = Tools::new(root.join("bin"));
        let ctx = egui::Context::default();
        let state = Arc::new(Mutex::new(State::Working(String::new(), None)));
        spawn_update(tools.clone(), state.clone(), ctx.clone());
        loop {
            match state.lock().unwrap().clone() {
                State::Ready(s) => {
                    println!("компоненти: {s}");
                    break;
                }
                State::Failed(e) => panic!("{e}"),
                State::Working(..) => {}
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }

        let url = "https://www.youtube.com/watch?v=jNQXAC9IVRw";
        let cases = [
            ("mp3", Settings { mode: Mode::Audio, audio_format: AudioFormat::Mp3, audio_bitrate: 192, normalize: true, ..Settings::default() }, Some((2.0, Some(8.0)))),
            ("video", Settings { mode: Mode::Video, max_height: 360, video_audio_bitrate: 128, ..Settings::default() }, None),
            ("noaudio", Settings { mode: Mode::VideoNoAudio, max_height: 240, ..Settings::default() }, None),
        ];
        for (name, mut settings, trim) in cases {
            settings.folder = out.join(name);
            let job = Job::new(1, url.into());
            spawn_fetch_info(job.clone(), tools.clone(), settings.clone(), ctx.clone());
            wait(&job, |s| !matches!(s, Status::FetchingInfo));
            {
                let mut j = job.lock().unwrap();
                assert!(j.status == Status::Ready, "інформація не отримана");
                let info = j.info.as_ref().unwrap();
                println!("[{name}] {} · {:?} · {:?} · {:?}", info.title, info.duration, info.max_height, info.source_audio);
                if let Some((from, to)) = trim {
                    j.trim_open = true;
                    j.trim_from = from.to_string();
                    j.trim_to = to.unwrap().to_string();
                }
            }
            let (tx, rx) = std::sync::mpsc::channel();
            spawn_download(job.clone(), tools.clone(), settings, root.join("tmp"), ctx.clone(), tx);
            wait(&job, |s| matches!(s, Status::Done | Status::Failed(_) | Status::Cancelled));
            let j = job.lock().unwrap();
            if let Status::Failed(f) = &j.status {
                panic!("[{name}] {} / {}", f.message, f.details);
            }
            println!("[{name}] файли: {:?}, примітка: {:?}", j.files, j.note);
            assert!(j.files.iter().all(|f| f.is_file()));
            assert!(rx.try_recv().is_ok());
        }
    }

    fn wait(job: &JobRef, done: impl Fn(&Status) -> bool) {
        for _ in 0..1200 {
            if done(&job.lock().unwrap().status) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        panic!("занадто довго");
    }

    #[test]
    fn explains_errors() {
        let f = explain("WARNING: x\nERROR: [youtube] abc: Private video. Sign in if you've been granted access");
        assert_eq!(f.kind, FailureKind::NeedsLogin);
        assert!(f.details.starts_with("ERROR:"));

        let f = explain("ERROR: [facebook] 123: This content isn't available right now");
        assert_eq!(f.kind, FailureKind::NeedsLogin);

        let f = explain("ERROR: Failed to decrypt with DPAPI. See https://github.com/yt-dlp/yt-dlp/issues/10927 for more info");
        assert_eq!(f.kind, FailureKind::CookiesUnreadable);

        let f = explain("ERROR: Unsupported URL: https://example.com/");
        assert_eq!(f.kind, FailureKind::Other);
    }

    #[test]
    fn audio_args() {
        let s = Settings {
            mode: Mode::Audio,
            audio_format: AudioFormat::Mp3,
            audio_bitrate: 320,
            normalize: true,
            ..Settings::default()
        };
        let a = format_args(&s, Some((80.0, None)));
        let joined = a.join(" ");
        assert!(joined.contains("--audio-format mp3 --audio-quality 320K"));
        assert!(joined.contains("ExtractAudio:-af loudnorm=I=-14:TP=-1.5:LRA=11 -ar 48000"));
        assert!(joined.contains("--download-sections *80-inf"));
        assert!(joined.contains("--force-keyframes-at-cuts"));
    }
}
