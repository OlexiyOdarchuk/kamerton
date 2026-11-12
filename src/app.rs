//! Інтерфейс: зверху посилання, зліва налаштування, по центру завантаження або історія.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Align, Align2, FontId, Layout, Margin, RichText, Stroke, StrokeKind, Vec2};

use crate::components::{self, State, Tools};
use crate::job::{self, FailureKind, Job, JobRef, Status};
use crate::report::{self, Report, ReportState};
use crate::settings::{self, AudioFormat, HistoryEntry, Mode, Settings, ThemeChoice};
use crate::theme::{self, Palette, icon};
use crate::install;
use crate::instance::Instance;
use crate::util::{self, AppDirs};
use crate::updater::{self, UpdateState};
use crate::widgets;

#[derive(PartialEq)]
enum Tab {
    Downloads,
    History,
}

pub struct KamertonApp {
    dirs: AppDirs,
    tools: Tools,
    components: Arc<Mutex<State>>,
    settings: Settings,
    saved_settings: Settings,
    jobs: Vec<JobRef>,
    next_id: u64,
    url_input: String,
    tab: Tab,
    extra_open: bool,
    /// Браузери, з яких можна взяти вхід (визначаються при запуску).
    browsers: Vec<(&'static str, &'static str)>,
    update: Arc<Mutex<UpdateState>>,
    history: Vec<HistoryEntry>,
    done_tx: Sender<HistoryEntry>,
    done_rx: Receiver<HistoryEntry>,
    /// Блокування «лише одна копія»; посилання від інших копій приходять у `inbox_rx`.
    instance: Instance,
    inbox_rx: Receiver<String>,
    /// Чи було вікно у фокусі минулого кадру (щоб помітити повернення до вікна).
    was_focused: bool,
    /// Текст буфера обміну, який уже бачили, щоб не пропонувати те саме двічі.
    clipboard_seen: String,
    /// Посилання з буфера, які пропонуємо додати.
    clipboard_offer: Vec<String>,
    /// Скільки завантажень завершилось, поки вікно було не у фокусі.
    unseen_done: usize,
    /// Чи вже надіслано звіт про невдале встановлення інструментів.
    components_reported: bool,
    #[cfg(debug_assertions)]
    debug_shot_done: bool,
}

const MAX_INFO_FETCHES: usize = 3;
const LOGO: &[u8] = include_bytes!("../assets/icon.png");
/// Мінусовку робить сторонній сайт: вбудована нейромережа зробила б програму важкою й повільною.
const VOCAL_REMOVER: &str = "https://vocalremover.org/";

fn is_audio(path: &std::path::Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    matches!(ext.as_str(), "mp3" | "m4a" | "opus" | "ogg" | "flac" | "wav" | "aac")
}

impl KamertonApp {
    pub fn new(cc: &eframe::CreationContext<'_>, dirs: AppDirs, instance: Instance) -> Self {
        let ctx = &cc.egui_ctx;
        egui_extras::install_image_loaders(ctx);
        theme::install(ctx);

        let (inbox_tx, inbox_rx) = channel();
        instance.watch(inbox_tx, ctx.clone());
        // Залишки недокачаного з попереднього запуску (інша копія не працює, це гарантує блокування).
        let _ = std::fs::remove_dir_all(dirs.tmp());
        let tools = Tools::new(dirs.bin());
        let settings = Settings::load(&dirs.config.join("settings.json"));
        let history = settings::load_history(&dirs.config.join("history.json"));
        let components = Arc::new(Mutex::new(State::Working("Перевірка компонентів…".into(), None)));
        components::spawn_update(tools.clone(), components.clone(), ctx.clone());
        updater::cleanup_old();
        let update = Arc::new(Mutex::new(UpdateState::Quiet));
        updater::spawn_check(update.clone(), ctx.clone());
        let (done_tx, done_rx) = channel();

        let mut app = Self {
            dirs,
            tools,
            components,
            saved_settings: settings.clone(),
            settings,
            jobs: Vec::new(),
            next_id: 1,
            url_input: String::new(),
            tab: Tab::Downloads,
            extra_open: false,
            browsers: settings::installed_browsers(),
            update,
            history,
            done_tx,
            done_rx,
            instance,
            inbox_rx,
            was_focused: false,
            clipboard_seen: String::new(),
            clipboard_offer: Vec::new(),
            unseen_done: 0,
            components_reported: false,
            #[cfg(debug_assertions)]
            debug_shot_done: false,
        };
        // Посилання, передані при запуску (наприклад, «Відкрити за допомогою…»).
        let args: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with("--")).collect();
        app.add_urls(&args.join(" "));
        app
    }

    fn add_urls(&mut self, text: &str) {
        for url in util::extract_urls(text) {
            let duplicate = self.jobs.iter().any(|j| {
                let j = j.lock().unwrap();
                j.url == url && !matches!(j.status, Status::Done | Status::Cancelled)
            });
            if !duplicate {
                self.jobs.push(Job::new(self.next_id, url));
                self.next_id += 1;
            }
        }
        self.tab = Tab::Downloads;
    }

    fn paste_clipboard(&mut self) {
        if let Ok(text) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
            self.add_urls(&text);
            self.clipboard_seen = text;
            self.clipboard_offer.clear();
        }
    }

    /// Версія програми, система і версії інструментів: перші рядки звіту про помилку.
    /// Версії інструментів і вхід через браузер: частина звіту про помилку.
    fn env_info(&self) -> String {
        let components = match &*self.components.lock().unwrap() {
            State::Ready(s) => s.clone(),
            State::Working(s, _) => format!("ще настроюється: {s}"),
            State::Failed(s) => format!("не встановлено: {s}"),
        };
        let browser = if self.settings.cookies_browser.is_empty() { "ні" } else { &self.settings.cookies_browser };
        format!("{components}; вхід через браузер: {browser}")
    }

    /// Автоматичні звіти: нова помилка на картці або невдале встановлення інструментів.
    /// «Потрібен вхід» і «відео недоступне» не надсилаються: це не збій програми.
    fn auto_reports(&mut self, ctx: &egui::Context) {
        if !self.settings.send_reports || !report::available() {
            return;
        }
        let tools = self.env_info();
        for job in &self.jobs {
            let mut j = job.lock().unwrap();
            let (Status::Failed(f) | Status::InfoFailed(f)) = &j.status else { continue };
            let key = format!("{}|{}", j.url, f.details);
            if j.report_key.as_deref() == Some(key.as_str()) {
                continue;
            }
            let f = f.clone();
            j.report_key = Some(key.clone());
            *j.report.lock().unwrap() = ReportState::NotSent;
            let expected = f.kind != FailureKind::Other
                || f.message.contains("недоступне")
                || f.message.contains("не підтримується")
                || f.message.contains("Потрібної якості");
            if !expected && report::may_auto_send(&key) {
                send_job_report(job_report(&j, &f, &tools, true), &j.report, ctx);
            }
        }
        if !self.components_reported
            && let State::Failed(msg) = &*self.components.lock().unwrap()
        {
            self.components_reported = true;
            if report::may_auto_send(msg) {
                let r = Report::new(tools.clone(), String::new(), "встановлення інструментів".into(), msg.clone(), String::new(), true);
                report::send(r, |_| {});
            }
        }
    }

    /// Якщо в буфері обміну нове посилання, пропонуємо його додати.
    fn check_clipboard(&mut self) {
        let Ok(text) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) else { return };
        if text == self.clipboard_seen {
            return;
        }
        self.clipboard_seen = text.clone();
        // Лише явні посилання з http(s), щоб не чіплятись до будь-якого тексту.
        let known: Vec<String> = self.jobs.iter().map(|j| j.lock().unwrap().url.clone()).collect();
        self.clipboard_offer = util::extract_urls(&text)
            .into_iter()
            .filter(|u| text.contains(u.as_str()) && !known.contains(u))
            .collect();
    }

    fn clipboard_bar(&mut self, ui: &mut egui::Ui) {
        if self.clipboard_offer.is_empty() {
            return;
        }
        let p = theme::palette(ui.ctx());
        ui.add_space(8.0);
        egui::Frame::new()
            .fill(p.card)
            .stroke(Stroke::new(2.0, p.accent_line))
            .corner_radius(theme::RADIUS)
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let n = self.clipboard_offer.len();
                    let text = if n == 1 {
                        "У буфері обміну є посилання:".to_string()
                    } else {
                        format!("У буфері обміну є посилання ({n}):")
                    };
                    ui.label(RichText::new(text).font(FontId::new(16.0, theme::bold())));
                    let mut add = false;
                    let mut dismiss = false;
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Не треба").clicked() {
                            dismiss = true;
                        }
                        if widgets::primary(ui, icon::PLUS, "Додати", true).clicked() {
                            add = true;
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.add(egui::Label::new(RichText::new(&self.clipboard_offer[0]).color(p.dim)).truncate());
                        });
                    });
                    if add {
                        let urls = std::mem::take(&mut self.clipboard_offer).join(" ");
                        self.add_urls(&urls);
                    } else if dismiss {
                        self.clipboard_offer.clear();
                    }
                });
            });
    }

    /// Видалення встановленої копії (лише Windows).
    fn uninstall_now(&mut self, ctx: &egui::Context) {
        if install::confirm_uninstall() {
            for j in &self.jobs {
                j.lock().unwrap().stop();
            }
            self.instance.release();
            install::uninstall(&self.dirs.root());
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn components_ready(&self) -> bool {
        matches!(*self.components.lock().unwrap(), State::Ready(_))
    }

    /// Запускає отримання інформації та завантаження, коли є вільні місця.
    fn schedule(&mut self, ctx: &egui::Context) {
        if !self.components_ready() {
            return;
        }
        let mut fetching = 0;
        let mut active = 0;
        for j in &self.jobs {
            let j = j.lock().unwrap();
            if j.status == Status::FetchingInfo {
                fetching += 1;
            }
            if j.status.is_active() {
                active += 1;
            }
        }
        for job in &self.jobs {
            let status = job.lock().unwrap().status.clone();
            match status {
                Status::NeedInfo if fetching < MAX_INFO_FETCHES => {
                    job::spawn_fetch_info(job.clone(), self.tools.clone(), self.settings.clone(), ctx.clone());
                    fetching += 1;
                }
                Status::Queued if active < self.settings.parallel.max(1) => {
                    job::spawn_download(
                        job.clone(),
                        self.tools.clone(),
                        self.settings.clone(),
                        self.dirs.tmp(),
                        ctx.clone(),
                        self.done_tx.clone(),
                    );
                    active += 1;
                }
                _ => {}
            }
        }
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        // .txt-файл зі списком посилань, перетягнутий у вікно.
        let dropped: Vec<_> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        for path in dropped {
            if let Ok(text) = std::fs::read_to_string(&path) {
                self.add_urls(&text);
            }
        }
        // Ctrl+V, коли курсор не в текстовому полі.
        if !ctx.egui_wants_keyboard_input() {
            let pasted: Vec<String> = ctx.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| if let egui::Event::Paste(t) = e { Some(t.clone()) } else { None })
                    .collect()
            });
            for t in pasted {
                self.add_urls(&t);
            }
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        ui.horizontal(|ui| {
            ui.add(egui::Image::from_bytes("bytes://logo", LOGO).fit_to_exact_size(Vec2::splat(40.0)));
            ui.label(RichText::new("Камертон").font(FontId::new(28.0, theme::display())));
            ui.add_space(14.0);

            let buttons_width = 250.0;
            ui.label(RichText::new(icon::LINK).size(20.0).color(p.dim));
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.url_input)
                    .hint_text("Вставте посилання на відео або музику")
                    .font(FontId::proportional(17.0))
                    .desired_width((ui.available_width() - buttons_width).max(200.0))
                    .margin(Margin::symmetric(10, 9)),
            );
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let typed = !self.url_input.trim().is_empty();
            if widgets::primary(ui, icon::PASTE, "Вставити", true)
                .on_hover_text("Додати посилання з буфера обміну (Ctrl+V)")
                .clicked()
            {
                self.paste_clipboard();
            }
            let add = ui.add_enabled(typed, egui::Button::new(format!("{}  Додати", icon::PLUS)));
            if (add.clicked() || enter) && typed {
                let text = std::mem::take(&mut self.url_input);
                self.add_urls(&text);
                resp.request_focus();
            }
        });
    }

    fn settings_panel(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        let mut uninstall = false;
        let extra_open = &mut self.extra_open;
        let browsers = &self.browsers;
        let s = &mut self.settings;
        let mut scroll = egui::ScrollArea::vertical().auto_shrink([false, false]);
        // Лише для знімків екрана під час розробки: панель завжди прокручена вгору.
        if cfg!(debug_assertions) && std::env::var_os("KAMERTON_SCREENSHOT").is_some() {
            scroll = scroll.vertical_scroll_offset(0.0);
        }
        scroll.show(ui, |ui| {
            widgets::section(ui, "Що завантажити");
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let w = (ui.available_width() - 12.0) / 3.0;
                for (mode, ic, label) in [
                    (Mode::Video, icon::VIDEO, "Відео"),
                    (Mode::Audio, icon::MUSIC, "Звук"),
                    (Mode::VideoNoAudio, icon::NO_AUDIO, "Без звуку"),
                ] {
                    if widgets::mode_tile(ui, s.mode == mode, ic, label, w).clicked() {
                        s.mode = mode;
                    }
                }
            });
            ui.add_space(6.0);

            match s.mode {
                Mode::Video | Mode::VideoNoAudio => {
                    widgets::caption(ui, "Якість");
                    let opts: Vec<_> = settings::VIDEO_HEIGHTS
                        .iter()
                        .map(|&h| {
                            let big = match h {
                                0 => "Макс".to_string(),
                                2160 => "4K".to_string(),
                                h => h.to_string(),
                            };
                            (h, big, None)
                        })
                        .collect();
                    widgets::plates(ui, &mut s.max_height, &opts);
                    if s.mode == Mode::Video {
                        ui.add_space(4.0);
                        widgets::caption(ui, "Звук у відео, кбіт/с");
                        let opts: Vec<_> = settings::VIDEO_AUDIO_BITRATES
                            .iter()
                            .map(|&b| (b, if b == 0 { "Оригінал".into() } else { b.to_string() }, None))
                            .collect();
                        widgets::plates(ui, &mut s.video_audio_bitrate, &opts);
                    }
                    ui.add_space(6.0);
                    widgets::toggle(ui, &mut s.compatible, "Сумісний формат (MP4, H.264)", true);
                    widgets::dim(
                        ui,
                        &p,
                        if s.compatible {
                            "Відкриється на будь-якому телефоні, телевізорі й у редакторі."
                        } else {
                            "Найвища якість (до 4K), але файл WebM/MKV може не відкритися на старих пристроях."
                        },
                    );
                }
                Mode::Audio => {
                    widgets::caption(ui, "Формат");
                    let opts: Vec<_> = AudioFormat::ALL
                        .iter()
                        .map(|&f| {
                            let big = match f {
                                AudioFormat::Original => "Оригінал".to_string(),
                                f => f.ytdlp_name().to_uppercase(),
                            };
                            (f, big, None)
                        })
                        .collect();
                    widgets::plates(ui, &mut s.audio_format, &opts);
                    let note = match s.audio_format {
                        AudioFormat::Mp3 => "Найсумісніший, грає всюди.",
                        AudioFormat::M4a => "AAC, добре звучить на низьких бітрейтах.",
                        AudioFormat::Opus => "Найкраща якість на кілобіт, але не всі плеєри знають.",
                        AudioFormat::Flac => "Без втрат. Якості не додає, якщо джерело стиснене.",
                        AudioFormat::Wav => "Без стиснення, для монтажу. Без обкладинки й тегів.",
                        AudioFormat::Original => "Як у джерелі (зазвичай Opus або AAC), без перекодування.",
                    };
                    widgets::dim(ui, &p, note);

                    let rates = s.audio_format.bitrates();
                    if !rates.is_empty() {
                        if !rates.contains(&s.audio_bitrate) {
                            s.audio_bitrate = if rates.contains(&320) { 320 } else { rates[0] };
                        }
                        ui.add_space(4.0);
                        widgets::caption(ui, "Бітрейт, кбіт/с");
                        let opts: Vec<_> = rates
                            .iter()
                            .map(|&b| if b == 0 { (b, "VBR".into(), Some("~245".into())) } else { (b, b.to_string(), None) })
                            .collect();
                        widgets::plates(ui, &mut s.audio_bitrate, &opts);
                    }
                    if s.audio_format != AudioFormat::Original {
                        ui.add_space(4.0);
                        widgets::caption(ui, "Частота, кГц");
                        let opts = [(0u32, "Як є".to_string(), None), (44100, "44.1".into(), None), (48000, "48".into(), None)];
                        widgets::plates(ui, &mut s.sample_rate, &opts);
                    }
                    ui.add_space(6.0);
                    let can_normalize = s.audio_format != AudioFormat::Original;
                    widgets::toggle(ui, &mut s.normalize, "Вирівняти гучність (−14 LUFS)", can_normalize).on_hover_text(
                        "EBU R128 (loudnorm), true peak −1.5 dBTP.\nЯкщо частота «як є», файл буде 48 кГц.",
                    );
                }
            }
            let wav = s.mode == Mode::Audio && s.audio_format == AudioFormat::Wav;
            let meta_label = if s.mode == Mode::Audio { "Обкладинка і теги" } else { "Теги (назва, автор)" };
            widgets::toggle(ui, &mut s.embed_meta, meta_label, !wav);

            if s.mode == Mode::Audio {
                ui.add_space(14.0);
                widgets::section(ui, "Мінусовка");
                widgets::dim(
                    ui,
                    &p,
                    "Сайт vocalremover.org прибирає голос з пісні або залишає лише голос. \
                     Відкрийте його і перетягніть туди скачаний файл.",
                );
                if widgets::button(ui, icon::LINK, "Відкрити vocalremover.org").clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(VOCAL_REMOVER));
                }
            }

            ui.add_space(14.0);
            widgets::section(ui, "Куди зберегти");
            ui.add(egui::Label::new(RichText::new(s.folder.display().to_string()).size(14.0)).wrap());
            ui.horizontal(|ui| {
                if widgets::button(ui, icon::FOLDER, "Змінити").clicked()
                    && let Some(dir) = rfd::FileDialog::new().set_directory(&s.folder).pick_folder()
                {
                    s.folder = dir;
                }
                if ui.button("Відкрити папку").clicked() {
                    let _ = std::fs::create_dir_all(&s.folder);
                    util::reveal(&s.folder);
                }
            });

            ui.add_space(14.0);
            let caret = if *extra_open { icon::CARET_DOWN } else { icon::CARET_RIGHT };
            if ui
                .add(egui::Button::new(RichText::new(format!("{caret}  Додатково")).font(FontId::new(18.0, theme::display()))).frame(false))
                .clicked()
            {
                *extra_open = !*extra_open;
            }
            if *extra_open {
                widgets::toggle(ui, &mut s.whole_playlist, "Качати весь плейлист", true);
                widgets::dim(ui, &p, "Якщо відео з плейлиста, скачати всі відео з нього.");
                ui.add_space(6.0);
                widgets::caption(ui, "Вхід через браузер");
                widgets::dim(
                    ui,
                    &p,
                    "Для приватних відео, груп у Facebook, Instagram і коли YouTube просить увійти. \
                     Камертон бере вхід з браузера, де ви вже увійшли на сайт. Пароль програма не бачить.",
                );
                let mut opts: Vec<(&str, String, Option<String>)> = vec![("", "Ні".to_string(), None)];
                opts.extend(browsers.iter().map(|(k, n)| (*k, n.to_string(), None)));
                let mut current: &str = opts.iter().find(|(k, _, _)| *k == s.cookies_browser).map(|(k, _, _)| *k).unwrap_or("");
                if widgets::plates(ui, &mut current, &opts) {
                    s.cookies_browser = current.to_string();
                }
                if browsers.is_empty() {
                    widgets::dim(ui, &p, "Не знайдено жодного браузера, з якого можна взяти вхід.");
                }
                ui.add_space(6.0);
                widgets::caption(ui, "Одночасних завантажень");
                let opts: Vec<_> = (1..=5usize).map(|n| (n, n.to_string(), None)).collect();
                widgets::plates(ui, &mut s.parallel, &opts);
                if report::available() {
                    ui.add_space(6.0);
                    widgets::toggle(ui, &mut s.send_reports, "Надсилати звіти про помилки розробнику", true);
                    widgets::dim(
                        ui,
                        &p,
                        "Лише коли щось не вдалося: посилання, текст помилки і версії програм. \
                         Паролів і особистих файлів у звіті немає.",
                    );
                }
                if install::is_installed_copy() {
                    ui.add_space(10.0);
                    if widgets::button(ui, icon::TRASH, "Видалити програму…")
                        .on_hover_text("Скачані відео й музика залишаться на місці")
                        .clicked()
                    {
                        uninstall = true;
                    }
                }
            }
            ui.add_space(12.0);
        });
        if uninstall {
            self.uninstall_now(ui.ctx());
        }
    }

    /// Підсумок налаштувань унизу панелі: що саме буде скачано.
    fn summary_strip(&self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        ui.add_space(8.0);
        ui.label(RichText::new("Буде завантажено").font(FontId::new(13.0, theme::bold())).color(p.dim));
        ui.add(egui::Label::new(RichText::new(self.settings.summary()).font(FontId::new(20.0, theme::display()))).wrap());
        ui.add_space(8.0);
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        let state = self.components.lock().unwrap().clone();
        // Праворуч перемикач теми, решта рядка для стану компонентів.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (choice, ic, label) in [
                (ThemeChoice::Dark, icon::MOON, "Темна тема"),
                (ThemeChoice::Light, icon::SUN, "Світла тема"),
                (ThemeChoice::Auto, icon::AUTO_THEME, "Тема як у системі"),
            ] {
                if widgets::icon_choice(ui, self.settings.theme == choice, ic, label).clicked() {
                    self.settings.theme = choice;
                }
            }
            ui.label(RichText::new("Тема").size(14.0).color(p.dim));
            ui.add_space(12.0);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| self.component_status(ui, &p, state));
        });
    }

    fn component_status(&mut self, ui: &mut egui::Ui, p: &Palette, state: State) {
        match state {
            State::Working(text, progress) => {
                // Встановлення й оновлення компонентів подано як настроювання інструмента.
                ui.spinner();
                ui.label(RichText::new(format!("Настроювання: {text}")).size(15.0));
                if let Some(f) = progress {
                    widgets::progress(ui, Some(f), 180.0, 8.0);
                    ui.label(RichText::new(format!("{:.0}%", f * 100.0)).font(FontId::new(16.0, theme::display())));
                }
            }
            State::Ready(summary) => {
                ui.label(RichText::new(format!("{}  Налаштовано", icon::CHECK)).size(15.0));
                ui.add(egui::Label::new(RichText::new(summary).color(p.dim).size(14.0)).truncate());
            }
            State::Failed(msg) => {
                let first = msg.lines().next().unwrap_or("").to_string();
                ui.label(RichText::new(format!("{}  {first}", icon::WARNING)).color(p.error).size(14.0)).on_hover_text(&msg);
                if widgets::button(ui, icon::RETRY, "Спробувати ще раз").clicked() {
                    *self.components.lock().unwrap() = State::Working("Перевірка компонентів…".into(), None);
                    components::spawn_update(self.tools.clone(), self.components.clone(), ui.ctx().clone());
                }
                // Зі звітами цей випадок надсилається автоматично, інакше його можна скопіювати.
                if !report::available() && widgets::button(ui, icon::COPY, "Скопіювати звіт").clicked() {
                    ui.ctx().copy_text(self.env_info());
                }
            }
        }
    }

    /// Плашка про оновлення програми над вкладками.
    fn update_banner(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        let state = self.update.lock().unwrap().clone();
        let busy = self.jobs.iter().any(|j| {
            let s = &j.lock().unwrap().status;
            s.is_active() || *s == Status::Queued
        });
        let frame = |stroke| {
            egui::Frame::new()
                .fill(p.card)
                .stroke(Stroke::new(2.0, stroke))
                .corner_radius(theme::RADIUS)
                .inner_margin(Margin::symmetric(14, 10))
        };
        match state {
            UpdateState::Quiet => return,
            UpdateState::Downloading(text, progress) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    widgets::dim(ui, &p, format!("{text}…"));
                    if let Some(f) = progress {
                        widgets::progress(ui, Some(f), 140.0, 8.0);
                    }
                });
            }
            UpdateState::Ready(tag) => {
                frame(p.accent_line).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(format!("Встановлено нову версію Камертона ({tag})"))
                                .font(FontId::new(17.0, theme::bold())),
                        );
                        widgets::dim(ui, &p, "Вона запрацює після перезапуску.");
                        let resp = widgets::primary(ui, icon::RETRY, "Перезапустити", !busy);
                        if busy {
                            resp.on_hover_text("Дочекайтесь, поки завершаться завантаження");
                        } else if resp.clicked() {
                            // Нова копія має отримати блокування «одна копія», тому звільняємо його.
                            self.instance.release();
                            updater::restart(ui.ctx());
                        }
                    });
                });
            }
            UpdateState::Failed(msg) => {
                widgets::dim(ui, &p, format!("Не вдалося оновити програму: {msg}"));
            }
        }
        ui.add_space(10.0);
    }

    fn tabs(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        let count = self.jobs.len();
        let downloads = if count > 0 { format!("Завантаження ({count})") } else { "Завантаження".into() };
        for (tab, label) in [(Tab::Downloads, downloads), (Tab::History, "Історія".to_string())] {
            let active = self.tab == tab;
            let color = if active { p.text } else { p.dim };
            let resp = ui.add(
                egui::Button::new(RichText::new(label).font(FontId::new(22.0, theme::display())).color(color)).frame(false),
            );
            if active {
                let r = resp.rect;
                let line = egui::Rect::from_min_max(egui::pos2(r.left() + 4.0, r.bottom() - 1.0), egui::pos2(r.right() - 4.0, r.bottom() + 3.0));
                ui.painter().rect_filled(line, 0.0, p.accent_line);
            }
            if resp.clicked() {
                self.tab = tab;
            }
            ui.add_space(10.0);
        }
    }

    fn downloads_tab(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        if self.jobs.is_empty() {
            self.empty_state(ui);
            return;
        }
        let ready_count = self.jobs.iter().filter(|j| j.lock().unwrap().status == Status::Ready).count();
        let finished = self
            .jobs
            .iter()
            .filter(|j| matches!(j.lock().unwrap().status, Status::Done | Status::Cancelled))
            .count();

        ui.horizontal(|ui| {
            if widgets::primary(ui, icon::DOWNLOAD, &format!("Завантажити все ({ready_count})"), ready_count > 0).clicked() {
                for j in &self.jobs {
                    let mut j = j.lock().unwrap();
                    if j.status == Status::Ready && j.trim_range().is_ok() {
                        j.status = Status::Queued;
                    }
                }
            }
            if ui.add_enabled(finished > 0, egui::Button::new(format!("{}  Прибрати завершені", icon::TRASH))).clicked() {
                self.jobs.retain(|j| !matches!(j.lock().unwrap().status, Status::Done | Status::Cancelled));
            }
        });
        ui.add_space(6.0);

        let ready = self.components_ready();
        let mut remove = Vec::new();
        let mut use_browser = None;
        let browsers = &self.browsers;
        let current = self.settings.cookies_browser.as_str();
        let env_info = self.env_info();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for job in &self.jobs {
                let mut j = job.lock().unwrap();
                match job_card(ui, &p, &mut j, ready, browsers, current, &env_info) {
                    CardAction::Remove => remove.push(j.id),
                    CardAction::UseBrowser(key) => use_browser = Some(key),
                    CardAction::None => {}
                }
                ui.add_space(8.0);
            }
        });
        if let Some(key) = use_browser {
            // Вибраний браузер запам'ятовується для всіх наступних завантажень.
            self.settings.cookies_browser = key.to_string();
        }
        if !remove.is_empty() {
            self.jobs.retain(|j| !remove.contains(&j.lock().unwrap().id));
        }
    }

    fn empty_state(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        ui.add_space((ui.available_height() * 0.22).max(10.0));
        ui.vertical_centered(|ui| {
            ui.add(egui::Image::from_bytes("bytes://logo", LOGO).fit_to_exact_size(Vec2::splat(84.0)));
            ui.add_space(8.0);
            ui.label(RichText::new("Вставте посилання").font(FontId::new(34.0, theme::display())));
            ui.label(
                RichText::new(
                    "YouTube, Facebook, Instagram, TikTok, SoundCloud та сотні інших сайтів.\n\
                     Можна кілька посилань одразу або перетягнути у вікно .txt-файл зі списком.",
                )
                .color(p.dim),
            );
            ui.add_space(10.0);
            if widgets::primary(ui, icon::PASTE, "Вставити з буфера", true).clicked() {
                self.paste_clipboard();
            }
            widgets::dim(ui, &p, "або Ctrl+V у будь-якому місці вікна");

            // Перший запуск: пояснюємо, чому програма щось качає ще до першого відео.
            let first_run = !(self.tools.ytdlp.is_file() && self.tools.ffmpeg.is_file() && self.tools.deno.is_file());
            if first_run && matches!(*self.components.lock().unwrap(), State::Working(..)) {
                ui.add_space(18.0);
                ui.label(RichText::new("Камертон настроюється").font(FontId::new(22.0, theme::display())));
                widgets::dim(
                    ui,
                    &p,
                    "Перший запуск: докачуються інструменти для роботи (yt-dlp, ffmpeg, Deno, близько 100 МБ).\n\
                     Це один раз, далі лише невеликі оновлення. Посилання можна додавати вже зараз.",
                );
            }
        });
    }

    fn history_tab(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        if self.history.is_empty() {
            ui.add_space((ui.available_height() * 0.3).max(10.0));
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Поки порожньо").font(FontId::new(28.0, theme::display())));
                widgets::dim(ui, &p, "Тут з'являтимуться скачані файли, щоб їх легко знайти.");
            });
            return;
        }
        let mut readd = None;
        if widgets::button(ui, icon::TRASH, "Очистити історію").clicked() {
            self.history.clear();
            settings::save_history(&self.dirs.config.join("history.json"), &self.history);
            return;
        }
        ui.add_space(6.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for h in &self.history {
                card_frame(&p).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(RichText::new(&h.title).font(FontId::new(16.5, theme::bold()))).truncate());
                    let files = if h.files > 1 { format!(" · файлів: {}", h.files) } else { String::new() };
                    widgets::dim(ui, &p, format!("{} · {}{files}", h.when, h.summary));
                    ui.horizontal(|ui| {
                        let exists = h.path.as_ref().is_some_and(|p| p.exists());
                        if ui
                            .add_enabled(exists, egui::Button::new(format!("{}  Показати у папці", icon::FOLDER)))
                            .on_disabled_hover_text("Файл переміщено або видалено")
                            .clicked()
                            && let Some(p) = &h.path
                        {
                            util::reveal(p);
                        }
                        if widgets::button(ui, icon::RETRY, "Скачати ще раз").clicked() {
                            readd = Some(h.url.clone());
                        }
                        if widgets::button(ui, icon::COPY, "Копіювати посилання").clicked() {
                            ui.ctx().copy_text(h.url.clone());
                        }
                    });
                });
                ui.add_space(6.0);
            }
        });
        if let Some(url) = readd {
            self.add_urls(&url);
        }
    }
}

fn card_frame(p: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(theme::RADIUS)
        .inner_margin(Margin::same(14))
}

enum CardAction {
    None,
    Remove,
    /// Брати вхід з цього браузера ("" означає без входу).
    UseBrowser(&'static str),
}

/// Малює картку одного завантаження і повертає, що користувач з нею зробив.
fn job_card(
    ui: &mut egui::Ui,
    p: &Palette,
    j: &mut Job,
    components_ready: bool,
    browsers: &[(&'static str, &'static str)],
    current_browser: &str,
    env_info: &str,
) -> CardAction {
    let mut remove = false;
    let mut chosen = None;
    card_frame(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            thumbnail(ui, p, j);
            ui.add_space(6.0);
            ui.vertical(|ui| {
                // Назва і хрестик «прибрати» праворуч в одному рядку.
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        if widgets::icon_button(ui, icon::X, "Прибрати зі списку").clicked() {
                            j.stop();
                            remove = true;
                        }
                        ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                            ui.add(egui::Label::new(RichText::new(j.title()).font(FontId::new(17.0, theme::bold()))).truncate());
                        });
                    });
                });
                card_meta(ui, p, j);
                card_edit(ui, p, j);
                status_block(ui, p, j);
                let failure = match &j.status {
                    Status::Failed(f) | Status::InfoFailed(f) if f.kind != FailureKind::Other => Some(f.kind),
                    _ => None,
                };
                if let Some(kind) = failure {
                    chosen = login_help(ui, p, kind, browsers, current_browser);
                    if chosen.is_some() {
                        // Одразу пробуємо ще раз, уже з входом.
                        j.status = if j.info.is_some() { Status::Queued } else { Status::NeedInfo };
                    }
                }
                card_buttons(ui, j, components_ready, env_info);
            });
        });
    });
    if remove {
        CardAction::Remove
    } else if let Some(key) = chosen {
        CardAction::UseBrowser(key)
    } else {
        CardAction::None
    }
}

/// Пояснення і кнопки, коли сайт вимагає входу або браузер не віддав вхід.
fn login_help(
    ui: &mut egui::Ui,
    p: &Palette,
    kind: FailureKind,
    browsers: &[(&'static str, &'static str)],
    current: &str,
) -> Option<&'static str> {
    let mut chosen = None;
    let name = settings::browser_name(current);
    egui::Frame::new()
        .fill(p.bg)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(theme::RADIUS)
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (title, text) = match kind {
                FailureKind::CookiesUnreadable => (
                    format!("Не вдалося взяти вхід з {name}"),
                    "Chrome і Edge на Windows часто не віддають вхід іншим програмам. \
                     Найнадійніше Firefox: увійдіть на сайт у Firefox і виберіть його нижче."
                        .to_string(),
                ),
                _ if current.is_empty() => (
                    "Потрібен вхід в акаунт".to_string(),
                    "1. Відкрийте цей сайт у своєму браузері й увійдіть в акаунт.\n\
                     2. Натисніть нижче свій браузер. Камертон візьме з нього вхід і спробує ще раз. \
                     Пароль програма не бачить."
                        .to_string(),
                ),
                _ => (
                    format!("Навіть із входом через {name} сайт не пускає"),
                    format!(
                        "Перевірте, що в {name} ви увійшли саме в той акаунт, якому доступне це відео, \
                         і спробуйте ще раз. Або виберіть інший браузер."
                    ),
                ),
            };
            ui.label(RichText::new(title).font(FontId::new(17.0, theme::bold())));
            ui.add(egui::Label::new(RichText::new(text).size(15.5)).wrap());
            if browsers.is_empty() {
                widgets::dim(ui, p, "Не знайдено жодного браузера, з якого можна взяти вхід. Встановіть Firefox і увійдіть у ньому на сайт.");
            }
            ui.add_space(4.0);
            if !browsers.is_empty() {
                widgets::caption(ui, "Взяти вхід з браузера:");
            }
            ui.horizontal_wrapped(|ui| {
                for (key, label) in browsers {
                    let skip = kind == FailureKind::CookiesUnreadable && *key == current;
                    if !skip && widgets::accent_outline(ui, "", label, true).clicked() {
                        chosen = Some(*key);
                    }
                }
                if !current.is_empty() && widgets::button(ui, icon::X, "Без входу").clicked() {
                    chosen = Some("");
                }
            });
        });
    chosen
}

fn thumbnail(ui: &mut egui::Ui, p: &Palette, j: &Job) {
    let size = Vec2::new(176.0, 99.0);
    let rect = match &j.thumb {
        Some(bytes) => {
            ui.add_sized(
                size,
                egui::Image::from_bytes(format!("bytes://thumb-{}", j.id), bytes.clone())
                    .max_size(size)
                    .corner_radius(theme::RADIUS),
            )
            .rect
        }
        None => {
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            ui.painter().rect_filled(rect, theme::RADIUS, p.track);
            if matches!(j.status, Status::NeedInfo | Status::FetchingInfo) {
                ui.put(rect, egui::Spinner::new().size(22.0));
            } else {
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon::VIDEO, FontId::proportional(30.0), p.dim);
            }
            rect
        }
    };
    // Тривалість поверх обкладинки: одразу видно, наскільки довге відео.
    if let Some(d) = j.info.as_ref().and_then(|i| i.duration) {
        let text = util::fmt_duration(d);
        let font = FontId::new(14.0, theme::display());
        let w = ui.fonts_mut(|f| f.layout_no_wrap(text.clone(), font.clone(), p.text).size().x);
        let badge = egui::Rect::from_min_size(rect.right_bottom() - Vec2::new(w + 14.0, 24.0), Vec2::new(w + 10.0, 20.0));
        ui.painter().rect_filled(badge, 3.0, egui::Color32::from_black_alpha(210));
        ui.painter().text(badge.center(), Align2::CENTER_CENTER, text, font, egui::Color32::WHITE);
    }
}

fn card_meta(ui: &mut egui::Ui, p: &Palette, j: &Job) {
    let Some(info) = &j.info else {
        widgets::dim(ui, p, &j.url);
        return;
    };
    let mut meta = Vec::new();
    if info.is_playlist {
        meta.push(format!("Плейлист, відео: {}", info.entries));
    }
    if let Some(u) = &info.uploader {
        meta.push(u.clone());
    }
    if let Some(h) = info.max_height {
        meta.push(format!("відео до {h}p"));
    }
    if let Some(a) = &info.source_audio {
        meta.push(format!("звук {a}"));
    }
    widgets::dim(ui, p, meta.join(" · "))
        .on_hover_text("Що доступно в джерелі. Вищий бітрейт при конвертації якості не додасть.");
}

fn card_edit(ui: &mut egui::Ui, p: &Palette, j: &mut Job) {
    let editable = matches!(j.status, Status::Ready | Status::Cancelled | Status::Failed(_) | Status::Done);
    let is_playlist = j.info.as_ref().is_some_and(|i| i.is_playlist);
    if !editable || j.info.is_none() || is_playlist {
        return;
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("Назва файлу").size(14.0).color(p.dim));
        // Місце праворуч під кнопку «Фрагмент» разом з відступами, інакше картка розширюється.
        ui.add(egui::TextEdit::singleline(&mut j.file_name).desired_width((ui.available_width() - 150.0).max(120.0)));
        let label = format!("{}  Фрагмент", icon::SCISSORS);
        ui.toggle_value(&mut j.trim_open, label).on_hover_text("Скачати лише частину: з якого по який момент");
    });
    if j.trim_open {
        ui.horizontal(|ui| {
            ui.label(RichText::new("з").size(14.0).color(p.dim));
            ui.add(egui::TextEdit::singleline(&mut j.trim_from).hint_text("0:00").desired_width(70.0));
            ui.label(RichText::new("до").size(14.0).color(p.dim));
            ui.add(egui::TextEdit::singleline(&mut j.trim_to).hint_text("кінця").desired_width(70.0));
            widgets::dim(ui, p, "наприклад 1:20 або 1:02:03");
        });
        if let Err(e) = j.trim_range() {
            ui.label(RichText::new(format!("{}  {e}", icon::WARNING)).color(p.error).size(14.0));
        }
    }
}

fn status_block(ui: &mut egui::Ui, p: &Palette, j: &Job) {
    match &j.status {
        Status::NeedInfo => {
            widgets::dim(ui, p, "Очікує черги…");
        }
        Status::FetchingInfo => {
            ui.horizontal(|ui| {
                ui.spinner();
                widgets::dim(ui, p, "Отримання інформації про відео…");
            });
        }
        Status::Ready => {}
        Status::Queued => {
            widgets::dim(ui, p, "У черзі, почнеться, щойно звільниться місце.");
        }
        Status::Downloading => {
            let pr = &j.progress;
            ui.horizontal(|ui| {
                let pct = pr.fraction.map(|f| format!("{:.0}%", f * 100.0)).unwrap_or_default();
                let pct_w = 74.0;
                widgets::progress(ui, pr.fraction, (ui.available_width() - pct_w - 8.0).max(80.0), 28.0);
                ui.label(RichText::new(pct).font(FontId::new(30.0, theme::display())));
            });
            let mut parts = Vec::new();
            if let Some((i, n)) = pr.item {
                parts.push(format!("відео {i} з {n}"));
            }
            if let Some(t) = pr.total {
                parts.push(util::fmt_bytes(t));
            }
            if let Some(s) = pr.speed {
                parts.push(format!("{}/с", util::fmt_bytes(s)));
            }
            if let Some(e) = pr.eta {
                parts.push(format!("ще {}", util::fmt_duration(e)));
            }
            let line = if parts.is_empty() { "Підготовка…".to_string() } else { parts.join(" · ") };
            widgets::dim(ui, p, format!("{line}   |   {}", j.summary));
        }
        Status::Processing(what) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(what);
            });
            widgets::dim(ui, p, &j.summary);
        }
        Status::Done => {
            let name = match j.files.len() {
                0 => String::new(),
                1 => j.files[0].file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                n => format!("файлів: {n}"),
            };
            ui.label(RichText::new(format!("{}  Готово · {}", icon::CHECK, j.summary)).font(FontId::new(17.0, theme::display())));
            widgets::dim(ui, p, name);
            if let Some(note) = &j.note {
                ui.label(RichText::new(format!("{}  {note}", icon::WARNING)).size(14.0));
            }
        }
        Status::Failed(f) | Status::InfoFailed(f) => {
            ui.add(egui::Label::new(RichText::new(format!("{}  {}", icon::WARNING, f.message)).color(p.error)).wrap());
            ui.add(egui::Label::new(RichText::new(&f.details).size(12.5).color(p.dim)).truncate())
                .on_hover_text(&f.details);
        }
        Status::Cancelled => {
            widgets::dim(ui, p, "Зупинено. Недокачані файли видалено.");
        }
    }
}

/// Звіт про помилку картки: що, де і як саме не вдалося.
fn job_report(j: &Job, f: &job::Failure, tools: &str, auto: bool) -> Report {
    let summary = if j.summary.is_empty() { "отримання інформації про відео".to_string() } else { j.summary.clone() };
    Report::new(tools.to_string(), j.url.clone(), summary, f.message.clone(), f.log.clone(), auto)
}

/// Надсилає звіт і показує на картці, як це пройшло.
fn send_job_report(report: Report, state: &Arc<Mutex<ReportState>>, ctx: &egui::Context) {
    *state.lock().unwrap() = ReportState::Sending;
    let (state, ctx) = (state.clone(), ctx.clone());
    report::send(report, move |ok| {
        *state.lock().unwrap() = if ok { ReportState::Sent } else { ReportState::Failed };
        ctx.request_repaint();
    });
}

fn report_controls(ui: &mut egui::Ui, j: &mut Job, f: &job::Failure, tools: &str) {
    let p = theme::palette(ui.ctx());
    if !report::available() {
        // Збірка без адреси для звітів: можна лише скопіювати текст.
        if widgets::button(ui, icon::COPY, "Скопіювати звіт").clicked() {
            ui.ctx().copy_text(job_report(j, f, tools, false).as_text());
        }
        return;
    }
    let state = *j.report.lock().unwrap();
    match state {
        ReportState::NotSent | ReportState::Failed => {
            if widgets::button(ui, icon::SEND, "Надіслати звіт розробнику")
                .on_hover_text("Опис помилки піде розробнику, щоб він міг розібратися")
                .clicked()
            {
                send_job_report(job_report(j, f, tools, false), &j.report, ui.ctx());
            }
            if state == ReportState::Failed {
                widgets::dim(ui, &p, "Не вдалося надіслати, перевірте інтернет");
            }
        }
        ReportState::Sending => {
            ui.spinner();
            widgets::dim(ui, &p, "Надсилання звіту…");
        }
        ReportState::Sent => {
            ui.label(RichText::new(format!("{}  Звіт надіслано розробнику", icon::CHECK)).size(15.0));
        }
    }
}

fn card_buttons(ui: &mut egui::Ui, j: &mut Job, components_ready: bool, tools: &str) {
    // Переносимо кнопки на новий рядок, якщо вікно вузьке, замість того щоб вилазити за картку.
    ui.horizontal_wrapped(|ui| {
        if let Status::Failed(f) | Status::InfoFailed(f) = &j.status {
            let f = f.clone();
            report_controls(ui, j, &f, tools);
        }
        match &j.status {
            Status::Ready => {
                if widgets::accent_outline(ui, icon::DOWNLOAD, "Завантажити", j.trim_range().is_ok()).clicked() {
                    j.status = Status::Queued;
                }
            }
            Status::Queued | Status::Downloading | Status::Processing(_) => {
                if widgets::button(ui, icon::STOP, "Стоп").clicked() {
                    j.stop();
                }
            }
            Status::InfoFailed(_) => {
                if ui.add_enabled(components_ready, egui::Button::new(format!("{}  Спробувати ще раз", icon::RETRY))).clicked() {
                    j.status = Status::NeedInfo;
                }
            }
            Status::Failed(_) | Status::Cancelled => {
                if ui.add_enabled(j.trim_range().is_ok(), egui::Button::new(format!("{}  Повторити", icon::RETRY))).clicked() {
                    j.status = Status::Queued;
                }
            }
            Status::Done => {
                if let Some(f) = j.files.first().cloned() {
                    if j.files.len() == 1 && widgets::button(ui, icon::PLAY, "Відкрити").clicked() {
                        util::open(&f);
                    }
                    if widgets::button(ui, icon::FOLDER, "Показати у папці").clicked() {
                        util::reveal(&f);
                    }
                    if is_audio(&f)
                        && widgets::button(ui, icon::MUSIC, "Мінусовка")
                            .on_hover_text("Відкриє vocalremover.org і покаже файл у папці, щоб перетягнути його на сайт")
                            .clicked()
                    {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(VOCAL_REMOVER));
                        util::reveal(&f);
                    }
                }
                if widgets::button(ui, icon::RETRY, "Скачати ще раз").on_hover_text("З поточними налаштуваннями").clicked() {
                    j.status = Status::Queued;
                }
            }
            Status::NeedInfo | Status::FetchingInfo => {}
        }
    });
}

impl eframe::App for KamertonApp {
    /// Те, що має працювати й тоді, коли вікно згорнуте (eframe тоді не викликає `ui`):
    /// черга завантажень, історія, посилання від інших копій, сигнал «Готово».
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);

        while let Ok(entry) = self.done_rx.try_recv() {
            self.history.insert(0, entry);
            self.history.truncate(500);
            settings::save_history(&self.dirs.config.join("history.json"), &self.history);
            if !focused {
                // Вікно згорнуте або позаду: блимаємо іконкою на панелі задач і пишемо в заголовку.
                self.unseen_done += 1;
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("Готово ({}) · Камертон", self.unseen_done)));
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                    egui::UserAttentionType::Informational,
                ));
            }
        }
        if focused && self.unseen_done > 0 {
            self.unseen_done = 0;
            ctx.send_viewport_cmd(egui::ViewportCommand::Title("Камертон".into()));
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Reset));
        }

        // Хтось запустив програму вдруге: показуємо це вікно і додаємо передані посилання.
        let mut show = false;
        while let Ok(line) = self.inbox_rx.try_recv() {
            self.add_urls(&line);
            show = true;
        }
        if show {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        self.schedule(ctx);
        self.auto_reports(ctx);
        if self.settings != self.saved_settings {
            self.settings.save(&self.dirs.config.join("settings.json"));
            self.saved_settings = self.settings.clone();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let wanted = match self.settings.theme {
            ThemeChoice::Auto => egui::ThemePreference::System,
            ThemeChoice::Light => egui::ThemePreference::Light,
            ThemeChoice::Dark => egui::ThemePreference::Dark,
        };
        if ctx.options(|o| o.theme_preference) != wanted {
            ctx.set_theme(wanted);
        }
        let p = theme::palette(&ctx);

        self.handle_input(&ctx);
        // Повернулась до вікна: можливо, в буфері нове посилання.
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        if focused && !self.was_focused {
            self.check_clipboard();
        }
        self.was_focused = focused;

        let bar = |fill| egui::Frame::new().fill(fill).inner_margin(Margin::symmetric(16, 10));
        egui::Panel::top("top")
            .frame(bar(p.panel).stroke(Stroke::new(1.0, p.border)))
            .show(ui, |ui| {
                self.top_bar(ui);
                self.clipboard_bar(ui);
            });
        egui::Panel::bottom("status")
            .frame(bar(p.panel).inner_margin(Margin::symmetric(16, 6)))
            .show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("rack")
            .resizable(false)
            .exact_size(330.0)
            .frame(egui::Frame::new().fill(p.panel).inner_margin(Margin::same(16)))
            .show(ui, |ui| {
                egui::Panel::bottom("summary")
                    .frame(egui::Frame::new().fill(p.panel))
                    .show(ui, |ui| self.summary_strip(ui));
                egui::CentralPanel::no_frame().show(ui, |ui| self.settings_panel(ui));
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.bg).inner_margin(Margin::symmetric(20, 14)))
            .show(ui, |ui| {
                self.update_banner(ui);
                ui.horizontal(|ui| self.tabs(ui));
                ui.add_space(10.0);
                match self.tab {
                    Tab::Downloads => self.downloads_tab(ui),
                    Tab::History => self.history_tab(ui),
                }
            });

        // Підказка під час перетягування файлу у вікно.
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let rect = ctx.content_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop")));
            painter.rect_filled(rect, 0.0, p.bg.gamma_multiply(0.94));
            painter.rect_stroke(rect.shrink(16.0), theme::RADIUS, Stroke::new(3.0, p.strong), StrokeKind::Inside);
            painter.text(rect.center() - Vec2::new(0.0, 16.0), Align2::CENTER_CENTER, "Відпустіть файл", FontId::new(36.0, theme::display()), p.text);
            painter.text(
                rect.center() + Vec2::new(0.0, 22.0),
                Align2::CENTER_CENTER,
                "Посилання з .txt додадуться до списку",
                FontId::proportional(17.0),
                p.dim,
            );
        }

        #[cfg(debug_assertions)]
        self.debug_screenshot(&ctx);

        // Поки щось качається, оновлюємо екран хоча б раз на секунду.
        if self.jobs.iter().any(|j| j.lock().unwrap().status.is_active()) {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
    }
}

#[cfg(debug_assertions)]
impl KamertonApp {
    /// Лише для розробки: `KAMERTON_SCREENSHOT=shot.png KAMERTON_SCREENSHOT_AT=8`
    /// зберігає знімок вікна через вказану кількість секунд і закриває програму.
    /// `KAMERTON_DEBUG_START=1` одразу запускає всі готові завантаження.
    /// `KAMERTON_DEBUG_THEME=light|dark` примусово вмикає тему.
    fn debug_screenshot(&mut self, ctx: &egui::Context) {
        match std::env::var("KAMERTON_DEBUG_THEME").as_deref() {
            Ok("light") => self.settings.theme = ThemeChoice::Light,
            Ok("dark") => self.settings.theme = ThemeChoice::Dark,
            _ => {}
        }
        if std::env::var_os("KAMERTON_DEBUG_EXTRA").is_some() {
            self.extra_open = true;
        }
        if std::env::var_os("KAMERTON_DEBUG_START").is_some() {
            for j in &self.jobs {
                let mut j = j.lock().unwrap();
                if j.status == Status::Ready {
                    j.status = Status::Queued;
                }
            }
        }
        let Some(path) = std::env::var_os("KAMERTON_SCREENSHOT") else { return };
        if self.debug_shot_done {
            return;
        }
        let at: f64 = std::env::var("KAMERTON_SCREENSHOT_AT").ok().and_then(|v| v.parse().ok()).unwrap_or(8.0);
        if ctx.input(|i| i.time) < at {
            ctx.request_repaint_after(std::time::Duration::from_millis(300));
            return;
        }
        self.debug_shot_done = true;
        let ctx2 = ctx.clone();
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
            let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
            if let Some(buf) = image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, rgba) {
                let _ = buf.save(&path);
            }
            ctx2.send_viewport_cmd(egui::ViewportCommand::Close);
        })));
    }
}
