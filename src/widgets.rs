//! Власні віджети: плитки вибору, перемикач, кнопка головної дії, прогрес на нотному стані.
//! Усі реагують на клавіатуру (Tab, Enter, Пробіл) і мають видиму рамку фокусу.

use eframe::egui::{self, Align2, CornerRadius, FontId, Rect, Response, Role, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::theme::{self, Palette, RADIUS};

fn r() -> CornerRadius {
    CornerRadius::same(RADIUS)
}

/// Рамка, коли елемент під курсором або у фокусі з клавіатури.
fn highlighted(resp: &Response) -> bool {
    resp.hovered() || resp.has_focus()
}

/// Заливка, колір тексту і рамка для плиток вибору.
fn choice_colors(p: &Palette, selected: bool, resp: &Response) -> (egui::Color32, egui::Color32, egui::Color32, Stroke) {
    if selected {
        let stroke = if highlighted(resp) { Stroke::new(2.0, p.strong) } else { Stroke::new(2.0, p.accent_line) };
        (p.accent, p.on_accent, p.on_accent, stroke)
    } else if highlighted(resp) {
        (p.card_hover, p.text, p.dim, Stroke::new(2.0, p.strong))
    } else {
        (p.card, p.text, p.dim, Stroke::new(1.0, p.border))
    }
}

/// Одна плитка вибору: значення і, за потреби, підпис під ним.
pub fn plate(ui: &mut Ui, selected: bool, big: &str, small: Option<&str>, min_width: f32) -> Response {
    let p = theme::palette(ui.ctx());
    let big_font = FontId::new(18.0, theme::bold());
    let small_font = FontId::new(12.5, theme::bold());
    let text_w = ui.fonts_mut(|f| f.layout_no_wrap(big.to_string(), big_font.clone(), p.text).size().x);
    let height = if small.is_some() { 48.0 } else { 38.0 };
    let size = Vec2::new((text_w + 22.0).max(min_width), height);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    resp.widget_info(|| egui::WidgetInfo::selected(Role::RadioButton, true, selected, big));

    let (fill, fg, small_fg, stroke) = choice_colors(&p, selected, &resp);
    let painter = ui.painter();
    painter.rect(rect, r(), fill, stroke, StrokeKind::Inside);
    match small {
        Some(s) => {
            painter.text(rect.center() - Vec2::new(0.0, 8.0), Align2::CENTER_CENTER, big, big_font, fg);
            painter.text(rect.center() + Vec2::new(0.0, 12.0), Align2::CENTER_CENTER, s, small_font, small_fg);
        }
        None => {
            painter.text(rect.center(), Align2::CENTER_CENTER, big, big_font, fg);
        }
    }
    resp
}

/// Ряд плиток; клік вибирає значення. Повертає `true`, якщо вибір змінився.
pub fn plates<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, String, Option<String>)]) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        for (v, big, small) in options {
            if plate(ui, *value == *v, big, small.as_deref(), 46.0).clicked() && *value != *v {
                *value = *v;
                changed = true;
            }
        }
    });
    changed
}

/// Велика плитка режиму: іконка і підпис.
pub fn mode_tile(ui: &mut Ui, selected: bool, icon: &str, label: &str, width: f32) -> Response {
    let p = theme::palette(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 74.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    resp.widget_info(|| egui::WidgetInfo::selected(Role::RadioButton, true, selected, label));
    let (fill, fg, _, stroke) = choice_colors(&p, selected, &resp);
    let painter = ui.painter();
    painter.rect(rect, r(), fill, stroke, StrokeKind::Inside);
    painter.text(rect.center() - Vec2::new(0.0, 11.0), Align2::CENTER_CENTER, icon, FontId::proportional(26.0), fg);
    painter.text(rect.center() + Vec2::new(0.0, 20.0), Align2::CENTER_CENTER, label, FontId::new(15.5, theme::bold()), fg);
    resp
}

/// Кнопка головної дії: заливка синіми чорнилами.
pub fn primary(ui: &mut Ui, icon: &str, text: &str, enabled: bool) -> Response {
    let p = theme::palette(ui.ctx());
    let font = FontId::new(16.5, theme::bold());
    let label = format!("{icon}  {text}");
    let text_size = ui.fonts_mut(|f| f.layout_no_wrap(label.clone(), font.clone(), p.text).size());
    let size = Vec2::new(text_size.x + 30.0, text_size.y.max(40.0));
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    resp.widget_info(|| egui::WidgetInfo::labeled(Role::Button, enabled, text));
    let resp = if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
    let (fill, fg, stroke) = if !enabled {
        (p.card, p.dim, Stroke::new(1.0, p.border))
    } else if highlighted(&resp) {
        (p.accent_hover, p.on_accent, Stroke::new(2.0, p.strong))
    } else {
        (p.accent, p.on_accent, Stroke::new(2.0, p.accent_line))
    };
    let painter = ui.painter();
    painter.rect(rect, r(), fill, stroke, StrokeKind::Inside);
    painter.text(rect.center(), Align2::CENTER_CENTER, label, font, fg);
    resp
}

/// Дія на картці: контур і текст синіми чорнилами. Головна кнопка екрана лишається одна.
/// Порожня `icon` дає кнопку лише з текстом.
pub fn accent_outline(ui: &mut Ui, icon: &str, text: &str, enabled: bool) -> Response {
    let p = theme::palette(ui.ctx());
    let font = FontId::new(16.5, theme::bold());
    let label = if icon.is_empty() { text.to_string() } else { format!("{icon}  {text}") };
    let text_size = ui.fonts_mut(|f| f.layout_no_wrap(label.clone(), font.clone(), p.text).size());
    let size = Vec2::new(text_size.x + 30.0, text_size.y.max(38.0));
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    resp.widget_info(|| egui::WidgetInfo::labeled(Role::Button, enabled, text));
    let resp = if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
    let (fill, fg, stroke) = if !enabled {
        (p.card, p.dim, Stroke::new(1.0, p.border))
    } else if highlighted(&resp) {
        (p.accent, p.on_accent, Stroke::new(2.0, p.strong))
    } else {
        (p.card, p.accent_line, Stroke::new(2.0, p.accent_line))
    };
    let painter = ui.painter();
    painter.rect(rect, r(), fill, stroke, StrokeKind::Inside);
    painter.text(rect.center(), Align2::CENTER_CENTER, label, font, fg);
    resp
}

/// Звичайна кнопка з іконкою.
pub fn button(ui: &mut Ui, icon: &str, text: &str) -> Response {
    ui.add(egui::Button::new(format!("{icon}  {text}")))
}

/// Перемикач з підписом. Клік по підпису теж перемикає.
pub fn toggle(ui: &mut Ui, on: &mut bool, label: &str, enabled: bool) -> Response {
    let p = theme::palette(ui.ctx());
    let font = FontId::proportional(16.5);
    let wrap = ui.available_width() - 56.0;
    let galley = ui.fonts_mut(|f| f.layout(label.to_string(), font, p.text, wrap));
    let height = galley.size().y.max(26.0);
    let size = Vec2::new(ui.available_width(), height);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, mut resp) = ui.allocate_exact_size(size, sense);
    resp.widget_info(|| egui::WidgetInfo::selected(Role::CheckBox, enabled, *on, label));
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let track = Rect::from_min_size(rect.left_top() + Vec2::new(0.0, (height - 22.0) / 2.0), Vec2::new(42.0, 22.0));
    let (track_fill, knob_fill) = if *on { (p.accent, p.on_accent) } else { (p.track, p.dim) };
    let stroke = if highlighted(&resp) && enabled {
        Stroke::new(2.0, p.strong)
    } else if *on {
        Stroke::new(1.5, p.accent_line)
    } else {
        Stroke::new(1.0, p.border)
    };
    let painter = ui.painter();
    let alpha = if enabled { 1.0 } else { 0.4 };
    painter.rect(track, CornerRadius::same(11), track_fill.gamma_multiply(alpha), stroke, StrokeKind::Inside);
    let x = if *on { track.right() - 11.0 } else { track.left() + 11.0 };
    painter.circle_filled(egui::pos2(x, track.center().y), 7.0, knob_fill.gamma_multiply(alpha));
    let text_color = if enabled { p.text } else { p.dim };
    painter.galley(rect.left_top() + Vec2::new(54.0, (height - galley.size().y) / 2.0), galley, text_color);
    resp
}

/// Мелодія для прогресу: позиції нот у півкроках від нижньої лінії (0) до верхньої (8).
const MELODY: [i32; 16] = [2, 4, 3, 5, 4, 6, 5, 7, 6, 4, 5, 3, 4, 2, 3, 1];

/// Прогрес. Велика версія: нотний стан, по якому з'являються нотки й підстрибують хвилею;
/// скільки нот, стільки й скачано. Мала (менше 16 px): проста смуга.
/// Невідомий розмір: група нот біжить по стану, показуючи, що процес живий.
pub fn progress(ui: &mut Ui, fraction: Option<f32>, width: f32, height: f32) {
    let p = theme::palette(ui.ctx());
    let t = ui.input(|i| i.time) as f32;

    if height < 16.0 {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        let (from, to) = match fraction {
            Some(f) => (0.0, f.clamp(0.0, 1.0)),
            None => {
                ui.ctx().request_repaint();
                let h = (t * 0.5).fract();
                ((h * 1.25 - 0.25).max(0.0), (h * 1.25).min(1.0))
            }
        };
        let x = |f: f32| rect.left() + rect.width() * f;
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(2), p.track);
        let fill = Rect::from_min_max(egui::pos2(x(from), rect.top()), egui::pos2(x(to), rect.bottom()));
        painter.rect_filled(fill, CornerRadius::same(2), p.accent_line);
        return;
    }

    // Над і під станом лишаємо місце для штилів і стрибків.
    let gap = height / 4.0;
    let (outer, _) = ui.allocate_exact_size(Vec2::new(width, height + gap * 6.0), Sense::hover());
    let staff = Rect::from_min_size(outer.min + Vec2::new(0.0, gap * 3.0), Vec2::new(width, height));
    let painter = ui.painter();
    for i in 0..5 {
        painter.hline(staff.x_range(), staff.top() + gap * i as f32, Stroke::new(1.0, p.staff));
    }

    let step = gap * 2.8;
    let count = ((staff.width() - gap * 2.0) / step).floor().max(1.0) as usize;
    let (rx, ry) = (gap * 0.62, gap * 0.46);
    for i in 0..count {
        let x = staff.left() + gap * 1.2 + i as f32 * step;
        let pos = (x - staff.left()) / staff.width();
        let visible = match fraction {
            Some(f) => pos <= f,
            None => {
                let head = (t * 0.4).fract() * 1.3;
                pos <= head && pos >= head - 0.3
            }
        };
        if !visible {
            continue;
        }
        let pitch = MELODY[i % MELODY.len()];
        // Хвиля стрибків: кожна наступна нота стрибає трохи пізніше за попередню.
        let hop = (t * 5.0 - i as f32 * 0.7).sin().max(0.0) * gap * 0.9;
        let center = egui::pos2(x, staff.bottom() - pitch as f32 * gap / 2.0 - hop);
        painter.add(egui::Shape::Ellipse(egui::epaint::EllipseShape::filled(center, Vec2::new(rx, ry), p.accent_line)));
        // Штиль: у нижніх нот угору праворуч, у верхніх униз ліворуч, як у нотному записі.
        let stem = Stroke::new(1.6, p.accent_line);
        if pitch < 4 {
            let sx = center.x + rx - 0.8;
            painter.line_segment([egui::pos2(sx, center.y), egui::pos2(sx, center.y - gap * 3.2)], stem);
        } else {
            let sx = center.x - rx + 0.8;
            painter.line_segment([egui::pos2(sx, center.y), egui::pos2(sx, center.y + gap * 3.2)], stem);
        }
    }
    ui.ctx().request_repaint();
}

/// Кнопка лише з іконкою. Підпис показується підказкою і читається екранними читачами.
pub fn icon_button(ui: &mut Ui, icon: &str, label: &str) -> Response {
    let p = theme::palette(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(30.0, 30.0), Sense::click());
    let resp = resp.on_hover_text(label).on_hover_cursor(egui::CursorIcon::PointingHand);
    resp.widget_info(|| egui::WidgetInfo::labeled(Role::Button, true, label));
    let painter = ui.painter();
    if highlighted(&resp) {
        painter.rect(rect, r(), p.card_hover, Stroke::new(2.0, p.strong), StrokeKind::Inside);
    }
    let fg = if highlighted(&resp) { p.text } else { p.dim };
    painter.text(rect.center(), Align2::CENTER_CENTER, icon, FontId::proportional(18.0), fg);
    resp
}

/// Маленька кнопка-іконка з вибором (перемикач теми).
pub fn icon_choice(ui: &mut Ui, selected: bool, icon: &str, label: &str) -> Response {
    let p = theme::palette(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(36.0, 30.0), Sense::click());
    let resp = resp.on_hover_text(label).on_hover_cursor(egui::CursorIcon::PointingHand);
    resp.widget_info(|| egui::WidgetInfo::selected(Role::RadioButton, true, selected, label));
    let (fill, fg, _, stroke) = choice_colors(&p, selected, &resp);
    let painter = ui.painter();
    painter.rect(rect, r(), fill, stroke, StrokeKind::Inside);
    painter.text(rect.center(), Align2::CENTER_CENTER, icon, FontId::proportional(18.0), fg);
    resp
}

/// Заголовок секції.
pub fn section(ui: &mut Ui, title: &str) {
    ui.label(egui::RichText::new(title).font(FontId::new(22.0, theme::display())));
}

/// Підзаголовок над рядом плиток.
pub fn caption(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(FontId::new(15.0, theme::bold())).color(theme::palette(ui.ctx()).dim));
}

pub fn dim(ui: &mut Ui, p: &Palette, text: impl Into<String>) -> Response {
    ui.add(egui::Label::new(egui::RichText::new(text.into()).color(p.dim).size(15.0)).wrap())
}
