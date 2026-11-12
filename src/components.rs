//! Встановлення та оновлення yt-dlp, ffmpeg і Deno.
//!
//! При кожному запуску у фоні перевіряються останні версії, і якщо щось новіше —
//! докачується. Номери встановлених версій лежать у `bin/versions.json`.
//!
//! Звідки беремо:
//! - yt-dlp — GitHub releases (оновлюється часто, без нього YouTube ламається);
//! - Deno — GitHub releases (yt-dlp потребує JS-рушій для YouTube);
//! - ffmpeg — стабільні релізи: gyan.dev (Windows), martin-riedl.de (Linux, macOS).

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::util::exe_name;
use eframe::egui;

#[derive(Clone, Debug)]
pub enum State {
    /// Іде перевірка або завантаження; текст і прогрес 0..1 (якщо відомий).
    Working(String, Option<f32>),
    /// Усе на місці. Рядок — підсумок версій або попередження.
    Ready(String),
    /// Чогось бракує, і докачати не вдалося.
    Failed(String),
}

#[derive(Clone)]
pub struct Tools {
    pub ytdlp: PathBuf,
    pub ffmpeg: PathBuf,
    pub bin: PathBuf,
    pub deno: PathBuf,
}

impl Tools {
    pub fn new(bin: PathBuf) -> Self {
        Self {
            ytdlp: bin.join(exe_name("yt-dlp")),
            ffmpeg: bin.join(exe_name("ffmpeg")),
            deno: bin.join(exe_name("deno")),
            bin,
        }
    }
}

type Shared = Arc<Mutex<State>>;

fn set(state: &Shared, ctx: &egui::Context, s: State) {
    *state.lock().unwrap() = s;
    ctx.request_repaint();
}

fn agent(follow_redirects: bool) -> ureq::Agent {
    let mut cfg = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .user_agent("kamerton");
    if !follow_redirects {
        cfg = cfg.max_redirects(0).max_redirects_will_error(false);
    }
    cfg.build().into()
}

/// Куди веде редирект (`Location`), не переходячи за ним.
fn redirect_target(url: &str) -> Result<String, String> {
    let resp = agent(false).get(url).call().map_err(|e| e.to_string())?;
    resp.headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .ok_or_else(|| format!("немає редиректу з {url}"))
}

/// Тег останнього релізу на GitHub через редирект `/releases/latest` → `/releases/tag/<тег>`.
/// Так не витрачається ліміт GitHub API.
pub(crate) fn github_latest_tag(repo: &str) -> Result<String, String> {
    let loc = redirect_target(&format!("https://github.com/{repo}/releases/latest"))?;
    loc.rsplit_once("/tag/")
        .map(|(_, t)| t.to_string())
        .ok_or_else(|| format!("не вдалося визначити версію {repo}"))
}

/// Що і звідки качати для одного компонента.
struct Plan {
    /// Ключ у versions.json і назва для користувача.
    name: &'static str,
    version: String,
    downloads: Vec<Download>,
}

struct Download {
    url: String,
    /// `None` — файл і є бінарником; `Some(імена)` — zip, з якого дістаємо ці файли.
    extract: Option<Vec<String>>,
    /// Назва бінарника в `bin`, якщо це не zip.
    save_as: String,
}

fn plan_ytdlp() -> Result<Plan, String> {
    let tag = github_latest_tag("yt-dlp/yt-dlp")?;
    let asset = if cfg!(windows) {
        "yt-dlp.exe"
    } else if cfg!(target_os = "macos") {
        "yt-dlp_macos"
    } else if cfg!(target_arch = "aarch64") {
        "yt-dlp_linux_aarch64"
    } else {
        "yt-dlp_linux"
    };
    Ok(Plan {
        name: "yt-dlp",
        downloads: vec![Download {
            url: format!("https://github.com/yt-dlp/yt-dlp/releases/download/{tag}/{asset}"),
            extract: None,
            save_as: exe_name("yt-dlp"),
        }],
        version: tag,
    })
}

fn plan_deno() -> Result<Plan, String> {
    let tag = github_latest_tag("denoland/deno")?;
    let triple = if cfg!(windows) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") { "aarch64-apple-darwin" } else { "x86_64-apple-darwin" }
    } else if cfg!(target_arch = "aarch64") {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    };
    Ok(Plan {
        name: "Deno",
        downloads: vec![Download {
            url: format!("https://github.com/denoland/deno/releases/download/{tag}/deno-{triple}.zip"),
            extract: Some(vec![exe_name("deno")]),
            save_as: String::new(),
        }],
        version: tag,
    })
}

fn plan_ffmpeg() -> Result<Plan, String> {
    if cfg!(windows) {
        // Збірка gyan.dev, але з її дзеркала на GitHub: у ~40 разів швидше за сам gyan.dev,
        // а .7z важить 35 МБ замість 114 МБ у .zip.
        let version = github_latest_tag("GyanD/codexffmpeg")?;
        Ok(Plan {
            name: "ffmpeg",
            downloads: vec![Download {
                url: format!(
                    "https://github.com/GyanD/codexffmpeg/releases/download/{version}/ffmpeg-{version}-essentials_build.7z"
                ),
                extract: Some(vec!["ffmpeg.exe".into(), "ffprobe.exe".into()]),
                save_as: String::new(),
            }],
            version,
        })
    } else {
        let os = if cfg!(target_os = "macos") { "macos" } else { "linux" };
        let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "amd64" };
        let base = "https://ffmpeg.martin-riedl.de";
        let mut downloads = Vec::new();
        let mut version = String::new();
        for tool in ["ffmpeg", "ffprobe"] {
            // Редирект виду /download/linux/amd64/1789931100_9.0.2/ffmpeg.zip
            let loc = redirect_target(&format!("{base}/redirect/latest/{os}/{arch}/release/{tool}.zip"))?;
            if version.is_empty() {
                version = loc
                    .rsplit('/')
                    .nth(1)
                    .and_then(|seg| seg.split_once('_'))
                    .map(|(_, v)| v.to_string())
                    .unwrap_or_else(|| loc.clone());
            }
            let url = if loc.starts_with("http") { loc } else { format!("{base}{loc}") };
            downloads.push(Download { url, extract: Some(vec![tool.into()]), save_as: String::new() });
        }
        Ok(Plan { name: "ffmpeg", version, downloads })
    }
}

fn load_versions(bin: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(bin.join("versions.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_versions(bin: &Path, v: &BTreeMap<String, String>) {
    if let Ok(s) = serde_json::to_string_pretty(v) {
        let _ = std::fs::write(bin.join("versions.json"), s);
    }
}

/// Качає файл у `dest`, оновлюючи прогрес.
pub(crate) fn download_file(url: &str, dest: &Path, mut progress: impl FnMut(u64, Option<u64>)) -> Result<(), String> {
    let resp = agent(true).get(url).call().map_err(|e| format!("{url}: {e}"))?;
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let mut reader = resp.into_body().into_reader();
    let mut file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("обрив з'єднання: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        progress(done, total);
    }
    file.flush().map_err(|e| e.to_string())?;
    if let Some(t) = total
        && done != t
    {
        return Err("файл скачався не повністю".into());
    }
    Ok(())
}

/// Дістає з zip файли з потрібними іменами (де б вони не лежали всередині) у `bin`.
fn extract_from_zip(zip_path: &Path, names: &[String], bin: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut found = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        let base = entry.name().map_err(|e| e.to_string())?.rsplit('/').next().unwrap_or("").to_string();
        if !names.contains(&base) {
            continue;
        }
        let tmp = bin.join(format!("{base}.new"));
        let mut out = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        drop(out);
        install_binary(&tmp, &bin.join(&base))?;
        found += 1;
    }
    if found < names.len() {
        return Err(format!("в архіві не знайдено {}", names.join(", ")));
    }
    Ok(())
}

/// Те саме для .7z (ffmpeg на Windows).
#[cfg(windows)]
fn extract_from_7z(archive_path: &Path, names: &[String], bin: &Path) -> Result<(), String> {
    let mut archive = sevenz_rust2::ArchiveReader::open(archive_path, sevenz_rust2::Password::empty())
        .map_err(|e| e.to_string())?;
    let mut found = Vec::new();
    let mut failure = None;
    archive
        .for_each_entries(|entry, reader| {
            let base = entry.name().rsplit(['/', '\\']).next().unwrap_or("").to_string();
            if entry.is_directory() || !names.contains(&base) {
                return Ok(true);
            }
            let tmp = bin.join(format!("{base}.new"));
            let result = std::fs::File::create(&tmp)
                .and_then(|mut out| std::io::copy(reader, &mut out).map(|_| ()))
                .map_err(|e| e.to_string())
                .and_then(|_| install_binary(&tmp, &bin.join(&base)));
            match result {
                Ok(()) => found.push(base),
                Err(e) => failure = Some(e),
            }
            Ok(failure.is_none())
        })
        .map_err(|e| e.to_string())?;
    if let Some(e) = failure {
        return Err(e);
    }
    if found.len() < names.len() {
        return Err(format!("в архіві не знайдено {}", names.join(", ")));
    }
    Ok(())
}

#[cfg(not(windows))]
fn extract_from_7z(_: &Path, _: &[String], _: &Path) -> Result<(), String> {
    Err(".7z використовується лише на Windows".into())
}

/// Робить файл виконуваним і атомарно ставить на місце старого.
fn install_binary(tmp: &Path, dest: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(tmp, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    std::fs::rename(tmp, dest).map_err(|e| format!("не вдалося замінити {}: {e}", dest.display()))
}

fn apply(plan: &Plan, bin: &Path, state: &Shared, ctx: &egui::Context) -> Result<(), String> {
    let count = plan.downloads.len();
    for (i, d) in plan.downloads.iter().enumerate() {
        let part = if count > 1 { format!(" ({}/{count})", i + 1) } else { String::new() };
        let label = format!("Завантаження {} {}{part}", plan.name, plan.version);
        set(state, ctx, State::Working(format!("{label}…"), Some(0.0)));
        let tmp = bin.join(format!("download-{}-{i}.tmp", plan.name));
        let mut last = 0.0f32;
        let res = download_file(&d.url, &tmp, |done, total| {
            if let Some(t) = total {
                let f = done as f32 / t as f32;
                if f - last >= 0.01 {
                    last = f;
                    set(state, ctx, State::Working(format!("{label}…"), Some(f)));
                }
            }
        });
        if let Err(e) = res {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        let res = match &d.extract {
            None => install_binary(&tmp, &bin.join(&d.save_as)),
            Some(names) => {
                set(state, ctx, State::Working(format!("Розпакування {}…", plan.name), None));
                let r = if d.url.ends_with(".7z") {
                    extract_from_7z(&tmp, names, bin)
                } else {
                    extract_from_zip(&tmp, names, bin)
                };
                let _ = std::fs::remove_file(&tmp);
                r
            }
        };
        res?;
    }
    Ok(())
}

/// Перевіряє й оновлює всі компоненти. Запускається у фоновому потоці.
pub fn spawn_update(tools: Tools, state: Shared, ctx: egui::Context) {
    std::thread::spawn(move || {
        let bin = tools.bin.clone();
        if let Err(e) = std::fs::create_dir_all(&bin) {
            set(&state, &ctx, State::Failed(format!("Не вдалося створити папку {}: {e}", bin.display())));
            return;
        }
        let mut versions = load_versions(&bin);
        let present = |p: &Path| p.is_file();
        let items: [(&str, fn() -> Result<Plan, String>, &Path); 3] = [
            ("yt-dlp", plan_ytdlp, &tools.ytdlp),
            ("ffmpeg", plan_ffmpeg, &tools.ffmpeg),
            ("Deno", plan_deno, &tools.deno),
        ];
        let mut problems = Vec::new();
        for (name, make_plan, path) in items {
            set(&state, &ctx, State::Working(format!("Перевірка оновлень {name}…"), None));
            let result = make_plan().and_then(|plan| {
                let installed = versions.get(plan.name).map(String::as_str);
                if installed == Some(plan.version.as_str()) && present(path) {
                    return Ok(());
                }
                apply(&plan, &bin, &state, &ctx)?;
                versions.insert(plan.name.to_string(), plan.version.clone());
                save_versions(&bin, &versions);
                Ok(())
            });
            if let Err(e) = result {
                problems.push((name, present(path), e));
            }
        }

        let missing: Vec<_> = problems.iter().filter(|(_, ok, _)| !ok).collect();
        if !missing.is_empty() {
            let details: Vec<String> = missing.iter().map(|(n, _, e)| format!("{n}: {e}")).collect();
            set(
                &state,
                &ctx,
                State::Failed(format!(
                    "Не вдалося встановити потрібні компоненти. Перевірте інтернет і спробуйте ще раз.\n{}",
                    details.join("\n")
                )),
            );
            return;
        }
        let summary = ["yt-dlp", "ffmpeg", "Deno"]
            .iter()
            .map(|n| format!("{n} {}", versions.get(*n).map(String::as_str).unwrap_or("?")))
            .collect::<Vec<_>>()
            .join(" · ");
        let summary = if problems.is_empty() {
            summary
        } else {
            format!("{summary} (не вдалося перевірити оновлення)")
        };
        set(&state, &ctx, State::Ready(summary));
    });
}
