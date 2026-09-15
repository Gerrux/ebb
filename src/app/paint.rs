//! Painting shared by the layer's windows: the logo, glass panels and buttons,
//! a card's panel and its color marker.

use std::time::Duration;

use egui::{Color32, CornerRadius, Id, Pos2, Rect, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::card;
use crate::theme;

use super::card_ui::HEADER_H;

pub(crate) const PANEL_APPEAR: Duration = Duration::from_millis(200);
pub(super) const PANEL_FADE_OUT: Duration = Duration::from_millis(250);

/// Ease-out cubic on a 0..1 progress (clamped).
pub(crate) fn panel_ease(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// The Ebb logo in a square `rect`, drawn from the geometry of scripts/make-icon.py
/// (a 140-unit square), so it stays sharp at any size. Below ~24 px it keeps
/// only the card and one bold wave, like the small icon sizes.
pub(super) fn paint_logo(painter: &egui::Painter, rect: Rect) {
    const BG: Color32 = Color32::from_rgb(30, 33, 40);
    const CARD: Color32 = Color32::from_rgb(52, 211, 153);
    const INK: Color32 = Color32::from_rgb(11, 59, 43);
    const WAVE: Color32 = Color32::from_rgb(45, 212, 191);
    let k = rect.width() / 140.0;
    let at = |x: f32, y: f32| rect.min + vec2(x, y) * k;
    let rrect = |x0: f32, y0: f32, x1: f32, y1: f32, r: f32, fill: Color32| {
        painter.rect_filled(Rect::from_min_max(at(x0, y0), at(x1, y1)), CornerRadius::same((r * k).round() as u8), fill);
    };
    let wave = |y: f32, width: f32, color: Color32| {
        let points: Vec<Pos2> = (0..=48)
            .map(|i| {
                let x = 22.0 + i as f32 * 2.0;
                at(x, y - 5.0 * (std::f32::consts::PI * (x - 22.0) / 24.0).sin())
            })
            .collect();
        let r = width * k / 2.0;
        painter.circle_filled(points[0], r, color);
        painter.circle_filled(points[points.len() - 1], r, color);
        painter.add(egui::Shape::line(points, Stroke::new(width * k, color)));
    };

    rrect(0.0, 0.0, 140.0, 140.0, 32.0, BG);
    if rect.width() * painter.ctx().pixels_per_point() <= 24.0 {
        rrect(34.0, 22.0, 106.0, 78.0, 12.0, CARD);
        wave(108.0, 13.0, WAVE);
    } else {
        let edge = Rect::from_min_max(at(0.5, 0.5), at(139.5, 139.5));
        painter.rect_stroke(edge, CornerRadius::same((32.0 * k).round() as u8), Stroke::new((1.2 * k).max(0.6), Color32::from_white_alpha(30)), StrokeKind::Inside);
        rrect(42.0, 26.0, 98.0, 70.0, 10.0, CARD);
        rrect(54.0, 40.0, 86.0, 45.0, 2.5, INK);
        rrect(54.0, 51.0, 74.0, 56.0, 2.5, INK);
        wave(90.0, 6.0, WAVE);
        wave(110.0, 6.0, WAVE.gamma_multiply(115.0 / 255.0));
    }
}

/// 0..1 hover progress of a widget, eased over a short time.
pub(super) fn hover_t(ui: &Ui, id: Id, hovered: bool) -> f32 {
    panel_ease(ui.ctx().animate_bool_with_time(id.with("hover"), hovered, 0.14))
}

/// A glass button face at hover progress `t`: fill and edge brighten, a soft
/// shadow comes up under it.
pub(super) fn glass_button(ui: &Ui, rect: Rect, radius: f32, t: f32) {
    let p = ui.painter();
    let r = CornerRadius::same(radius.round() as u8);
    if t > 0.0 {
        p.rect_filled(rect.translate(vec2(0.0, 1.5)).expand(1.0), r, Color32::from_black_alpha((40.0 * t) as u8));
    }
    let fill = theme::glass_fill().lerp_to_gamma(theme::glass_fill_hover(), t);
    let stroke = theme::glass_stroke().lerp_to_gamma(theme::text().gamma_multiply(0.35), t);
    p.rect(rect, r, fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
}

pub(crate) fn glass_panel(ui: &Ui, rect: Rect, hovered: bool) {
    let fill = if hovered { theme::glass_fill_hover() } else { theme::glass_fill() };
    panel(ui, rect, fill, CornerRadius::same(12), 50);
}

/// A card's panel: glass, or the Start/taskbar color (a setting).
pub(super) fn card_panel(ui: &Ui, rect: Rect, hovered: bool, style: card::CardStyle) {
    panel(ui, rect, card_background(theme::card_fill(hovered), style, hovered), CornerRadius::same(style.radius), style.shadow);
}

/// `fill` at the style's opacity; a hovered card is a little more solid.
fn card_background(fill: Color32, style: card::CardStyle, hovered: bool) -> Color32 {
    let alpha = (f32::from(style.opacity) * 2.55 + if hovered { 12.0 } else { 0.0 }).min(255.0) as u8;
    Color32::from_rgba_unmultiplied(fill.r(), fill.g(), fill.b(), alpha)
}

/// `shadow` 0–100; 50 is the soft shadow floating panels always had.
fn panel(ui: &Ui, rect: Rect, fill: Color32, radius: CornerRadius, shadow: u8) {
    let painter = ui.painter();
    if shadow > 0 {
        let k = f32::from(shadow) / 50.0;
        let alpha = if theme::is_light() { 28.0 } else { 70.0 } * k;
        painter.add(
            egui::epaint::Shadow {
                offset: [0, (8.0 * k.min(1.5)).round() as i8],
                blur: (28.0 * k.min(1.5)).round() as u8,
                spread: 0,
                color: Color32::from_black_alpha(alpha.min(160.0) as u8),
            }
            .as_shape(rect, radius),
        );
    }
    painter.rect(rect, radius, fill, Stroke::new(1.0, theme::glass_stroke()), StrokeKind::Inside);
}

/// The card's color, drawn the way the user picked in settings. `strength` 50
/// (the default) is the look each marker was tuned at. Lines and glows take the
/// hue's mark color, fills its surface (see [`card::Tint`]).
pub(crate) fn paint_marker(ui: &Ui, rect: Rect, hue: card::Tint, style: card::CardStyle, hovered: bool) {
    use card::Marker;
    let accent = theme::on_card(hue.color());
    let s = f32::from(style.strength) / 50.0;
    let radius = CornerRadius::same(style.radius);
    let painter = ui.painter();
    // A band along one edge: the rounded rect clipped, so it follows the corners.
    let band = |band: Rect| {
        painter.with_clip_rect(band.intersect(ui.clip_rect())).rect_filled(rect, radius, accent.gamma_multiply((0.85 * s).min(1.0)));
    };
    let w = f32::from(style.strip);
    match style.marker {
        Marker::None => {}
        Marker::Glow => corner_glow(painter, rect, f32::from(style.radius), accent, (0.2 * s).min(0.6)),
        Marker::StripTop => band(Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + w))),
        Marker::StripLeft => band(Rect::from_min_max(rect.min, pos2(rect.min.x + w, rect.max.y))),
        Marker::Tint => {
            painter.rect_filled(rect, radius, accent.gamma_multiply((0.1 * s).min(0.4)));
        }
        Marker::Border => {
            painter.rect_stroke(rect, radius, Stroke::new(1.5, accent.gamma_multiply((0.55 * s).min(1.0))), StrokeKind::Inside);
        }
        Marker::Fill | Marker::Paper => {
            let surface = theme::surface(hue, style.strength);
            painter.rect_filled(rect, radius, card_background(surface, style, false));
            if style.marker == Marker::Paper && hovered {
                // The bar of a Sticky Notes window: the paper a shade deeper,
                // over the meta line.
                let t = if theme::card_is_light() { 0.32 } else { 0.22 };
                let l = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
                let bar = Color32::from_rgb(l(surface.r(), accent.r()), l(surface.g(), accent.g()), l(surface.b(), accent.b()));
                painter
                    .with_clip_rect(Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + HEADER_H)).intersect(ui.clip_rect()))
                    .rect_filled(rect, radius, card_background(bar, style, false));
            } else if hovered {
                painter.rect_filled(rect, radius, theme::wash(10));
            }
        }
        Marker::Outline => {
            painter.rect_filled(rect, radius, accent.gamma_multiply((0.07 * s).min(0.3)));
            painter.rect_stroke(rect, radius, Stroke::new(2.0, accent.gamma_multiply((0.8 * s).min(1.0))), StrokeKind::Inside);
        }
    }
}

/// A soft wash of `color` spreading from the top left corner of a rounded card.
/// egui has no radial gradient: a grid mesh with per-vertex alpha, its outer
/// vertices pulled inside the rounded corners so nothing spills past them.
fn corner_glow(painter: &egui::Painter, rect: Rect, radius: f32, color: Color32, strength: f32) {
    const STEP: f32 = 14.0;
    let reach = (rect.width().max(rect.height()) * 0.75).min(240.0);
    let area = Rect::from_min_size(rect.min, vec2(reach.min(rect.width()), reach.min(rect.height())));
    let (nx, ny) = ((area.width() / STEP).ceil() as u32, (area.height() / STEP).ceil() as u32);
    let inside = |p: Pos2| {
        // Nearest point of the rounded rect: clamp into the corner's circle.
        let c = pos2(p.x.clamp(rect.left() + radius, rect.right() - radius), p.y.clamp(rect.top() + radius, rect.bottom() - radius));
        let d = p - c;
        if d.length() > radius { c + d.normalized() * radius } else { p }
    };
    let mut mesh = egui::Mesh::default();
    for j in 0..=ny {
        for i in 0..=nx {
            let p = pos2(
                (area.left() + i as f32 * STEP).min(area.right()),
                (area.top() + j as f32 * STEP).min(area.bottom()),
            );
            let p = inside(p);
            let t = (1.0 - (p - rect.min).length() / reach).max(0.0);
            mesh.colored_vertex(p, color.gamma_multiply(strength * t * t));
        }
    }
    let row = nx + 1;
    for j in 0..ny {
        for i in 0..nx {
            let k = j * row + i;
            mesh.add_triangle(k, k + 1, k + row);
            mesh.add_triangle(k + 1, k + row + 1, k + row);
        }
    }
    painter.add(mesh);
}
