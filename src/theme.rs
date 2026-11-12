//! Візуальна мова «Камертона»: нотний папір, сині чорнила, червоне лише для помилок.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Shadow, Stroke, TextStyle, Theme,
};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq)]
pub struct Palette {
    /// Фон вікна.
    pub bg: Color32,
    /// Бічна панель, верхня та нижня смуги.
    pub panel: Color32,
    /// Картки, кнопки.
    pub card: Color32,
    pub card_hover: Color32,
    pub border: Color32,
    pub input: Color32,
    pub text: Color32,
    pub dim: Color32,
    /// Сині чорнила: заливка вибраного й головної дії.
    pub accent: Color32,
    pub accent_hover: Color32,
    /// Текст на заливці акценту.
    pub on_accent: Color32,
    /// Тонкі акцентні лінії (прогрес, підкреслення вкладки, рамка вибраного).
    /// У темній темі світліші, бо заливка на темному фоні має замалий контраст.
    pub accent_line: Color32,
    /// Рамка наведення й фокусу з клавіатури.
    pub strong: Color32,
    /// Лінії нотного стану в смузі прогресу.
    pub staff: Color32,
    pub track: Color32,
    /// Лише помилки, щоб їх не сплутати з вибраним.
    pub error: Color32,
}

pub const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x15, 0x14, 0x13),
    panel: Color32::from_rgb(0x1A, 0x19, 0x17),
    card: Color32::from_rgb(0x1E, 0x1C, 0x1A),
    card_hover: Color32::from_rgb(0x27, 0x25, 0x22),
    border: Color32::from_rgb(0x38, 0x34, 0x2F),
    input: Color32::from_rgb(0x10, 0x0F, 0x0E),
    text: Color32::from_rgb(0xEF, 0xE8, 0xDA),
    dim: Color32::from_rgb(0xA8, 0x9F, 0x90),
    accent: Color32::from_rgb(0x2E, 0x4F, 0xA6),
    accent_hover: Color32::from_rgb(0x3A, 0x5D, 0xB8),
    on_accent: Color32::from_rgb(0xFF, 0xFD, 0xF8),
    accent_line: Color32::from_rgb(0x93, 0xAA, 0xF0),
    strong: Color32::from_rgb(0xEF, 0xE8, 0xDA),
    staff: Color32::from_rgb(0x4A, 0x45, 0x3F),
    track: Color32::from_rgb(0x2B, 0x28, 0x25),
    error: Color32::from_rgb(0xFF, 0x7B, 0x6E),
};

pub const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xF6, 0xF1, 0xE7),
    panel: Color32::from_rgb(0xFB, 0xF8, 0xF1),
    card: Color32::from_rgb(0xFF, 0xFD, 0xF8),
    card_hover: Color32::from_rgb(0xF3, 0xEE, 0xE2),
    border: Color32::from_rgb(0xD9, 0xD0, 0xBF),
    input: Color32::from_rgb(0xFF, 0xFD, 0xF8),
    text: Color32::from_rgb(0x1C, 0x1A, 0x17),
    dim: Color32::from_rgb(0x63, 0x5C, 0x51),
    accent: Color32::from_rgb(0x23, 0x3F, 0x8C),
    accent_hover: Color32::from_rgb(0x1B, 0x33, 0x75),
    on_accent: Color32::from_rgb(0xFF, 0xFD, 0xF8),
    accent_line: Color32::from_rgb(0x23, 0x3F, 0x8C),
    strong: Color32::from_rgb(0x1C, 0x1A, 0x17),
    staff: Color32::from_rgb(0xCF, 0xC6, 0xB5),
    track: Color32::from_rgb(0xE6, 0xDF, 0xD1),
    error: Color32::from_rgb(0xB3, 0x26, 0x1E),
};

pub fn palette(ctx: &egui::Context) -> Palette {
    match ctx.theme() {
        Theme::Dark => DARK,
        Theme::Light => LIGHT,
    }
}

/// Fixel Display Bold: заголовки.
pub fn display() -> FontFamily {
    FontFamily::Name("display".into())
}

/// Fixel Text Bold.
pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

pub const RADIUS: u8 = 6;

/// Іконки Phosphor Bold (лише ті, що потрібні). Жирна вага, бо дрібні тонкі іконки погано видно.
pub mod icon {
    pub const VIDEO: &str = "\u{e792}";
    pub const MUSIC: &str = "\u{e340}";
    pub const NO_AUDIO: &str = "\u{e4dc}";
    pub const LINK: &str = "\u{e2e2}";
    pub const PASTE: &str = "\u{e198}";
    pub const PLUS: &str = "\u{e3d4}";
    pub const DOWNLOAD: &str = "\u{e20c}";
    pub const STOP: &str = "\u{e46c}";
    pub const RETRY: &str = "\u{e036}";
    pub const FOLDER: &str = "\u{e256}";
    pub const X: &str = "\u{e4f6}";
    pub const SCISSORS: &str = "\u{eae0}";
    pub const CHECK: &str = "\u{e182}";
    pub const WARNING: &str = "\u{e4e2}";
    pub const PLAY: &str = "\u{e3d0}";
    pub const TRASH: &str = "\u{e4a6}";
    pub const COPY: &str = "\u{e1ca}";
    pub const CARET_DOWN: &str = "\u{e136}";
    pub const CARET_RIGHT: &str = "\u{e13a}";
    pub const SUN: &str = "\u{e472}";
    pub const MOON: &str = "\u{e330}";
    pub const AUTO_THEME: &str = "\u{e18c}";
    pub const SEND: &str = "\u{e398}";
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, bytes: &'static [u8]| {
        fonts.font_data.insert(name.to_string(), Arc::new(FontData::from_static(bytes)));
    };
    add("text", include_bytes!("../assets/fonts/FixelText-Medium.ttf"));
    add("text-bold", include_bytes!("../assets/fonts/FixelText-Bold.ttf"));
    add("display", include_bytes!("../assets/fonts/FixelDisplay-Bold.ttf"));
    add("icons", include_bytes!("../assets/fonts/Phosphor-Bold.ttf"));

    // Стандартні шрифти egui лишаються запасними (символи, яких немає в основних).
    let fallback = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let chain = |names: &[&str]| -> Vec<String> {
        names.iter().map(|s| s.to_string()).chain(fallback.iter().cloned()).collect()
    };
    fonts.families.insert(FontFamily::Proportional, chain(&["text", "icons"]));
    fonts.families.insert(bold(), chain(&["text-bold", "icons"]));
    fonts.families.insert(display(), chain(&["display", "text-bold", "icons"]));
    ctx.set_fonts(fonts);

    ctx.style_mut_of(Theme::Dark, |s| apply(s, &DARK, true));
    ctx.style_mut_of(Theme::Light, |s| apply(s, &LIGHT, false));
}

fn apply(style: &mut egui::Style, p: &Palette, dark: bool) {
    // Великий кегль: легко читати без окулярів і на ноутбуці.
    style.text_styles.insert(TextStyle::Body, FontId::proportional(17.0));
    style.text_styles.insert(TextStyle::Button, FontId::new(16.5, bold()));
    style.text_styles.insert(TextStyle::Heading, FontId::new(24.0, display()));
    style.text_styles.insert(TextStyle::Small, FontId::proportional(14.5));

    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.interact_size.y = 34.0;
    style.spacing.scroll.bar_width = 8.0;

    let v = &mut style.visuals;
    *v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let r = CornerRadius::same(RADIUS);
    v.panel_fill = p.panel;
    v.window_fill = p.card;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = r;
    v.menu_corner_radius = r;
    v.window_shadow = Shadow::NONE;
    v.popup_shadow = Shadow::NONE;
    v.extreme_bg_color = p.input;
    v.text_edit_bg_color = Some(p.input);
    v.faint_bg_color = p.card;
    v.hyperlink_color = p.accent_line;
    v.error_fg_color = p.error;
    v.warn_fg_color = p.text;
    v.override_text_color = Some(p.text);
    v.weak_text_color = Some(p.dim);
    v.selection.bg_fill = p.accent_line.gamma_multiply(if dark { 0.35 } else { 0.25 });
    // Рамка поля вводу у фокусі.
    v.selection.stroke = Stroke::new(2.0, p.strong);
    v.text_cursor.stroke = Stroke::new(2.0, p.text);

    let w = &mut v.widgets;
    for (state, fill, stroke) in [
        (&mut w.noninteractive, p.card, Stroke::new(1.0, p.border)),
        (&mut w.inactive, p.card, Stroke::new(1.0, p.border)),
        // Наведення і фокус з клавіатури: товста рамка, добре видно.
        (&mut w.hovered, p.card_hover, Stroke::new(2.0, p.strong)),
        (&mut w.active, p.track, Stroke::new(2.0, p.strong)),
        (&mut w.open, p.card_hover, Stroke::new(2.0, p.strong)),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = stroke;
        state.fg_stroke = Stroke::new(1.5, p.text);
        state.corner_radius = r;
        state.expansion = 0.0;
    }
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
}
