//! One card on the layer: its panel, drag and resize, the meta line with the
//! kind and the hover actions, its menus, and the footer with tags and age.
//! The text inside is `card_text`.

use std::time::{Duration, Instant};

use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Layout, Pos2, Rect, RichText,
    Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, ViewportCommand, pos2, vec2,
};

use crate::card::{self, Card, Kind, MIN_SIZE, Placement, Sides};
use crate::resurface;
use crate::theme;

use super::card_text::{BodyHit, card_body, format_toolbar, short_date};
use super::paint::{card_panel, paint_marker};

pub(super) const HEADER_H: f32 = 30.0;
const FOOTER_H: f32 = 20.0;
/// The line above the text: the kind's mark, and the actions while hovered.
const META_H: f32 = 20.0;
/// How long the copy button shows its check mark.
pub(super) const COPIED_FOR: Duration = Duration::from_millis(1500);
/// "Позже" on a card: days until it comes back.
const SNOOZE_DAYS: [i64; 4] = [1, 3, 7, 30];

pub(super) enum Action {
    Front,
    /// The native floating window finished moving.
    WindowDragStopped,
    Moved,
    TogglePin,
    Copy,
    Detach(Pos2),
    Duplicate,
    Archive,
    Delete,
    StartEdit,
    Reveal,
    SetKind(Kind),
    SetTint(Option<card::Tint>),
    /// Fold the card to one line, or open it.
    ToggleCollapse,
    SetIdeaStatus(Option<card::IdeaStatus>),
    /// Take the tag's `#words` out of the text.
    RemoveTag(String),
    /// Open the tag field, hanging from this point.
    AddTag(Pos2),
    /// Tick or clear the check box on this line of the text.
    ToggleCheck(usize),
    /// Off the layer for this many days.
    Snooze(i64),
    /// The first address in a Link card, in the browser.
    OpenLink,
    /// A Goal or Reminder is done: to the archive.
    Done,
    /// A resurfaced card stays: it's the user's again.
    Keep,
}

fn icon_button(ui: &mut Ui, glyph: &str, tip: &str, color: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 22.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), theme::wash(22));
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        theme::icons(12.5),
        color,
    );
    // Name for screen readers and UI automation (the glyph itself says nothing).
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tip));
    resp.on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(tip)
}

fn age_label(created_at: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let mins = (now - created_at).max(0) / 60;
    match mins {
        0 => "сейчас".into(),
        m if m < 60 => format!("{m} мин"),
        m if m < 60 * 24 => format!("{} ч", m / 60),
        m => format!("{} дн", m / (60 * 24)),
    }
}

/// An idea's statuses, and none.
fn idea_status_menu(
    anchor: &egui::Response,
    popup: Id,
    current: Option<card::IdeaStatus>,
    out: &mut Vec<Action>,
) {
    egui::Popup::menu(anchor)
        .id(popup)
        .align(egui::RectAlign::TOP_START)
        .gap(4.0)
        .width(170.0)
        .show(|ui| {
            ui.spacing_mut().button_padding = vec2(8.0, 5.0);
            let options = card::IdeaStatus::ALL
                .into_iter()
                .map(Some)
                .chain(std::iter::once(None));
            for status in options {
                let label = status.map_or("Без статуса", card::IdeaStatus::label);
                let button = egui::Button::new(RichText::new(label).size(14.0))
                    .selected(status == current)
                    .min_size(vec2(ui.available_width(), 0.0));
                if ui.add(button).clicked() {
                    out.push(Action::SetIdeaStatus(status));
                }
            }
        });
}

/// The action a kind is for: first in the hover pill, in the kind's color.
fn primary_action(card: &Card) -> Option<(&'static str, &'static str, Action)> {
    if card.placement == Placement::Rediscover {
        return Some(("\u{E8FB}", "Оставить на слое", Action::Keep));
    }
    match card.kind {
        Kind::Prompt => Some(("\u{E77F}", "Копировать prompt", Action::Copy)),
        Kind::Reference => Some(("\u{E77F}", "Копировать текст", Action::Copy)),
        Kind::Private => Some(("\u{E77F}", "Скопировать, не показывая", Action::Copy)),
        Kind::Link if card::first_url(&card.body).is_some() => {
            Some(("\u{E8A7}", "Открыть ссылку", Action::OpenLink))
        }
        Kind::Goal | Kind::Reminder if !card.pinned => {
            Some(("\u{E930}", "Сделано: в архив", Action::Done))
        }
        _ => None,
    }
}

/// The rest of the actions: pin, copy (when the kind's own action isn't copy),
/// fold, a copy of the card, and delete. A pinned (`locked`) card can't be
/// deleted from here, only unpinned.
fn more_menu(
    anchor: &egui::Response,
    popup: Id,
    locked: bool,
    with_copy: bool,
    out: &mut Vec<Action>,
) {
    egui::Popup::menu(anchor)
        .id(popup)
        .align(egui::RectAlign::BOTTOM_END)
        .gap(4.0)
        .width(190.0)
        .show(|ui| {
            ui.spacing_mut().button_padding = vec2(8.0, 5.0);
            let item = |ui: &mut Ui, glyph: &str, label: &str, color: Color32| -> bool {
                let mut job = egui::text::LayoutJob::default();
                let format = |font: FontId, color: Color32| egui::TextFormat {
                    font_id: font,
                    color,
                    valign: Align::Center,
                    ..Default::default()
                };
                job.append(glyph, 0.0, format(theme::icons(13.0), color));
                job.append(
                    label,
                    10.0,
                    format(FontId::proportional(14.0), theme::text()),
                );
                ui.add(egui::Button::new(job).min_size(vec2(ui.available_width(), 0.0)))
                    .clicked()
            };
            let (glyph, label) = if locked {
                ("\u{E77A}", "Открепить")
            } else {
                ("\u{E718}", "Закрепить на месте")
            };
            if item(ui, glyph, label, theme::dim()) {
                out.push(Action::TogglePin);
            }
            if with_copy && item(ui, "\u{E77F}", "Копировать текст", theme::dim()) {
                out.push(Action::Copy);
            }
            ui.separator();
            if item(ui, "\u{E70E}", "Свернуть в строку", theme::dim()) {
                out.push(Action::ToggleCollapse);
            }
            if item(ui, "\u{E8C8}", "Дублировать", theme::dim()) {
                out.push(Action::Duplicate);
            }
            if locked {
                return;
            }
            ui.separator();
            if item(ui, "\u{E74D}", "Удалить", theme::muted()) {
                out.push(Action::Delete);
            }
        });
}

/// "Позже": which morning the card comes back on.
fn snooze_menu(anchor: &egui::Response, popup: Id, out: &mut Vec<Action>) {
    egui::Popup::menu(anchor)
        .id(popup)
        .align(egui::RectAlign::BOTTOM_END)
        .gap(4.0)
        .width(170.0)
        .show(|ui| {
            ui.spacing_mut().button_padding = vec2(8.0, 5.0);
            ui.add(
                egui::Label::new(
                    RichText::new("Убрать и вернуть")
                        .size(12.0)
                        .color(theme::muted()),
                )
                .selectable(false),
            );
            for days in SNOOZE_DAYS {
                let label = resurface::snooze_label(days);
                let mut chars = label.chars();
                let label: String = chars
                    .next()
                    .into_iter()
                    .flat_map(char::to_uppercase)
                    .chain(chars)
                    .collect();
                let button = egui::Button::new(RichText::new(label).size(14.0))
                    .min_size(vec2(ui.available_width(), 0.0));
                if ui.add(button).clicked() {
                    out.push(Action::Snooze(days));
                }
            }
        });
}

/// Shows the check mark on a card's copy button.
pub(super) fn mark_copied(ui: &Ui, card: i64) {
    ui.data_mut(|d| d.insert_temp(Id::new(("card", card)).with("copied"), Instant::now()));
}

/// Menu under a card's kind: every kind (with its digit key), then the card's color.
/// `below`: the anchor is at the top of the card, so the menu opens downwards.
/// A floating card has only its own small viewport, so its menu must scroll instead
/// of being clipped when the card is shorter than the list.
fn kind_menu(
    anchor: &egui::Response,
    card: &Card,
    out: &mut Vec<Action>,
    below: bool,
    floating: bool,
) {
    egui::Popup::menu(anchor)
        .align(if below {
            egui::RectAlign::BOTTOM_START
        } else {
            egui::RectAlign::TOP_END
        })
        .gap(4.0)
        .width(236.0)
        .show(|ui| {
            ui.spacing_mut().button_padding = vec2(8.0, 5.0);
            let mut contents = |ui: &mut Ui| {
                for kind in Kind::ALL {
                    let mut job = egui::text::LayoutJob::default();
                    let format = |font: FontId, color: Color32| egui::TextFormat {
                        font_id: font,
                        color,
                        valign: Align::Center,
                        ..Default::default()
                    };
                    job.append(kind.icon(), 0.0, format(theme::icons(13.0), kind.accent()));
                    job.append(
                        kind.label(),
                        10.0,
                        format(FontId::proportional(14.0), theme::text()),
                    );
                    let button = egui::Button::new(job)
                        .selected(kind == card.kind)
                        .shortcut_text(RichText::new(kind.key().to_string()).size(12.5))
                        .min_size(vec2(ui.available_width(), 0.0));
                    if ui.add(button).clicked() {
                        out.push(Action::SetKind(kind));
                    }
                }
                ui.separator();
                ui.add(
                    egui::Label::new(RichText::new("Цвет").size(12.0).color(theme::muted()))
                        .selectable(false),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(2.0, 4.0);
                    let swatches =
                        std::iter::once(None).chain(card::Tint::ALL.into_iter().map(Some));
                    for tint in swatches {
                        let (rect, resp) =
                            ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
                        let color = tint.map_or(card.kind.accent(), card::Tint::color);
                        let p = ui.painter();
                        if tint.is_none() {
                            // "By kind": a ring in the kind's color.
                            p.circle_stroke(rect.center(), 6.0, Stroke::new(2.0, color));
                        } else {
                            p.circle_filled(rect.center(), 7.0, color);
                        }
                        if tint == card.tint {
                            p.circle_stroke(rect.center(), 9.5, Stroke::new(1.5, theme::text()));
                        } else if resp.hovered() {
                            p.circle_stroke(rect.center(), 9.5, Stroke::new(1.0, theme::muted()));
                        }
                        let label = tint.map_or("По типу", card::Tint::label);
                        resp.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label)
                        });
                        if resp
                            .on_hover_text(label)
                            .on_hover_cursor(CursorIcon::PointingHand)
                            .clicked()
                        {
                            out.push(Action::SetTint(tint));
                        }
                    }
                });
                ui.separator();
                let (glyph, label) = if card.collapsed {
                    ("\u{E70D}", "Развернуть")
                } else {
                    ("\u{E70E}", "Свернуть в строку")
                };
                let mut job = egui::text::LayoutJob::default();
                let format = |font: FontId, color: Color32| egui::TextFormat {
                    font_id: font,
                    color,
                    valign: Align::Center,
                    ..Default::default()
                };
                job.append(glyph, 0.0, format(theme::icons(13.0), theme::muted()));
                job.append(
                    label,
                    10.0,
                    format(FontId::proportional(14.0), theme::text()),
                );
                if ui
                    .add(egui::Button::new(job).min_size(vec2(ui.available_width(), 0.0)))
                    .clicked()
                {
                    out.push(Action::ToggleCollapse);
                }
            };
            if floating {
                egui::ScrollArea::vertical()
                    .id_salt(("kind-menu-scroll", card.id))
                    .auto_shrink([false, false])
                    .show(ui, contents);
            } else {
                contents(ui);
            }
        });
}

/// `magnet`: every card's rect on the layer, to stick to while dragging.
#[allow(clippy::too_many_arguments)]
pub(super) fn card_ui(
    ui: &mut Ui,
    origin: Pos2,
    area: Vec2,
    card: &mut Card,
    hovered: bool,
    mut editing: Option<&mut String>,
    revealed: bool,
    revealed_text: Option<&str>,
    active: bool,
    magnet: Option<&[(i64, Rect)]>,
    style: card::CardStyle,
    // The card lives in a borderless viewport: its header moves the window,
    // not the card inside it.
    window_drag: bool,
) -> Vec<Action> {
    let mut out = Vec::new();
    let id = Id::new(("card", card.id));
    // A collapsed card is one line at its full width, drawn at that height; its
    // size is put back at the end, so it opens to what it was.
    let full_size = card.size;
    if card.collapsed {
        card.size = card.shown_size();
    }
    let rect = Rect::from_min_size(origin + card.pos.to_vec2(), card.size);

    // Registration order = hit-test priority: later widgets sit on top.
    // The background senses drags too, though it doesn't move the card: egui
    // hands a press to the topmost click widget and, separately, the topmost
    // drag widget, so a click-only background let the header or edge of a card
    // underneath catch the drag and come to the front.
    let bg = ui.interact(rect, id.with("bg"), Sense::click_and_drag());
    // Keep the native window drag out of the resize strips. Otherwise the header
    // wins the top/left overlap before the custom resize response is examined.
    const EDGE: f32 = 12.0;
    const CORNER: f32 = 24.0;
    let pressed_on_edge = ui.input(|i| i.pointer.press_origin()).is_some_and(|p| {
        p.x <= rect.left() + EDGE
            || p.x >= rect.right() - EDGE
            || p.y <= rect.top() + EDGE
            || p.y >= rect.bottom() - EDGE
    });
    let header_rect = if window_drag && !card.collapsed {
        Rect::from_min_max(
            pos2(rect.left() + EDGE, rect.top() + EDGE),
            pos2(
                rect.right() - EDGE,
                (rect.top() + HEADER_H + 6.0).min(rect.bottom()),
            ),
        )
    } else {
        Rect::from_min_size(rect.min, vec2(rect.width(), HEADER_H + 6.0))
    };
    // A pinned card is locked in place: no moving, no resizing.
    let locked = card.pinned;
    // A collapsed card is nearly all header: the header takes its click too
    // (it's on top of the background and would swallow it).
    let header_sense = if window_drag && pressed_on_edge {
        Sense::click()
    } else if window_drag {
        Sense::click_and_drag()
    } else {
        match (locked, card.collapsed) {
            (true, false) => Sense::hover(),
            (true, true) => Sense::click(),
            (false, false) => Sense::drag(),
            (false, true) => Sense::click_and_drag(),
        }
    };
    let drag = ui.interact(header_rect, id.with("drag"), header_sense);
    // Resize handles on every edge and corner; corners last so they win where they overlap.
    let (l, r, t, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
    let side = |left, right, top, bottom| Sides {
        left,
        right,
        top,
        bottom,
    };
    let handles = [
        (
            side(true, false, false, false),
            Rect::from_min_max(pos2(l, t), pos2(l + EDGE, b)),
        ),
        (
            side(false, true, false, false),
            Rect::from_min_max(pos2(r - EDGE, t), pos2(r, b)),
        ),
        (
            side(false, false, true, false),
            Rect::from_min_max(pos2(l, t), pos2(r, t + EDGE)),
        ),
        (
            side(false, false, false, true),
            Rect::from_min_max(pos2(l, b - EDGE), pos2(r, b)),
        ),
        (
            side(true, false, true, false),
            Rect::from_min_max(pos2(l, t), pos2(l + CORNER, t + CORNER)),
        ),
        (
            side(false, true, true, false),
            Rect::from_min_max(pos2(r - CORNER, t), pos2(r, t + CORNER)),
        ),
        (
            side(true, false, false, true),
            Rect::from_min_max(pos2(l, b - CORNER), pos2(l + CORNER, b)),
        ),
        (
            side(false, true, false, true),
            Rect::from_min_max(pos2(r - CORNER, b - CORNER), pos2(r, b)),
        ),
    ];
    let handles: Vec<(Sides, egui::Response)> = handles
        .into_iter()
        .filter(|_| !locked && !card.collapsed)
        .enumerate()
        .map(|(i, (sides, area))| {
            (
                sides,
                ui.interact(area, id.with(("resize", i)), Sense::drag()),
            )
        })
        .collect();
    // The last registered handle under the pointer is the one egui hit.
    let resize = handles
        .iter()
        .rev()
        .find(|(_, h)| h.dragged() || h.drag_started() || h.drag_stopped());
    let resize_hover = handles
        .iter()
        .rev()
        .find(|(_, h)| h.hovered())
        .map(|(s, _)| *s);
    let corner_hovered = resize_hover.is_some_and(|s| s.right && s.bottom);

    let native_resize_id = id.with("window-resize");
    if window_drag && let Some((sides, handle)) = resize {
        if handle.drag_started() {
            ui.data_mut(|data| data.insert_temp(native_resize_id, true));
            ui.ctx().send_viewport_cmd(ViewportCommand::BeginResize(
                sides.resize_direction(),
            ));
        }
        if handle.drag_stopped() {
            ui.data_mut(|data| data.remove::<bool>(native_resize_id));
            out.push(Action::Moved);
        }
    }

    if window_drag
        && drag.drag_started()
        && !pressed_on_edge
        && !resize.is_some_and(|(_, h)| h.drag_started())
    {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if window_drag && drag.drag_stopped() {
        out.push(Action::WindowDragStopped);
    }

    if bg.clicked()
        || bg.double_clicked()
        || drag.clicked()
        || drag.drag_started()
        || resize.is_some_and(|(_, h)| h.drag_started())
    {
        out.push(Action::Front);
    }
    // One click edits; a Private card takes a double click, so a stray click
    // doesn't lay its text open.
    // Pushed once the body is drawn: a click on a link there opens it instead.
    let edit_gesture = if card.kind == Kind::Private {
        bg.double_clicked()
    } else {
        bg.clicked()
    };
    // A collapsed card opens on a click instead.
    if card.collapsed && (bg.clicked() || drag.clicked()) {
        out.push(Action::ToggleCollapse);
    }
    let start_edit = edit_gesture && editing.is_none() && !card.collapsed;

    // The rect at the start of the gesture and the pointer's total movement live in
    // memory, so a stuck edge follows the pointer again once it moves past the snap
    // distance, and shrinking below the minimum doesn't lose track of the pointer.
    let raw_id = id.with("raw");
    let started = !window_drag
        && (drag.drag_started() || resize.is_some_and(|(_, h)| h.drag_started()));
    if started {
        ui.data_mut(|d| {
            d.insert_temp(
                raw_id,
                (Rect::from_min_size(card.pos, card.size), Vec2::ZERO),
            )
        });
    }
    // Alt places the card freely.
    let snapping = magnet.is_some() && !ui.input(|i| i.modifiers.alt);
    let bounds = Rect::from_min_size(Pos2::ZERO, area);
    let others: Vec<Rect> = match magnet {
        Some(all) if snapping && (drag.dragged() || drag.drag_stopped() || resize.is_some()) => all
            .iter()
            .filter(|(cid, _)| *cid != card.id)
            .map(|(_, r)| *r)
            .collect(),
        _ => Vec::new(),
    };
    let mut snapped: Option<card::Snapped> = None;
    let detach_id = id.with("detach");
    let resizing = resize
        .filter(|(_, h)| !window_drag && h.dragged())
        .map(|(s, h)| (*s, h.drag_delta()));
    if (!window_drag && drag.dragged()) || resizing.is_some() {
        let (start, mut total) = ui
            .data(|d| d.get_temp::<(Rect, Vec2)>(raw_id))
            .unwrap_or((Rect::from_min_size(card.pos, card.size), Vec2::ZERO));
        total += resizing.map_or(drag.drag_delta(), |(_, d)| d);
        ui.data_mut(|d| d.insert_temp(raw_id, (start, total)));
        let raw = match resizing {
            None => {
                let moved = start.translate(total);
                let dragged_out = moved.min.x < 0.0
                    || moved.min.y < 0.0
                    || moved.max.x > area.x
                    || moved.max.y > area.y;
                if dragged_out {
                    ui.data_mut(|d| d.insert_temp(detach_id, moved.min));
                } else {
                    ui.data_mut(|d| d.remove::<Pos2>(detach_id));
                }
                let min = pos2(
                    moved.min.x.clamp(0.0, (area.x - moved.width()).max(0.0)),
                    moved.min.y.clamp(0.0, (area.y - moved.height()).max(0.0)),
                );
                Rect::from_min_size(min, moved.size())
            }
            Some((sides, _)) => sides.resize(start, total, bounds),
        };
        let s = match (snapping, resizing) {
            (false, _) => card::Snapped {
                rect: raw,
                guides: Vec::new(),
                x: false,
                y: false,
            },
            (true, None) => card::snap_move(raw, &others, bounds),
            (true, Some((sides, _))) => card::snap_resize(raw, &others, bounds, sides),
        };
        card.pos = s.rect.min;
        card.size = s.rect.size();
        snapped = Some(s);
    }
    let detached = (!window_drag && drag.drag_stopped())
        .then(|| ui.data(|d| d.get_temp::<Pos2>(detach_id)))
        .flatten();
    let stopped_resize = resize
        .filter(|(_, h)| !window_drag && h.drag_stopped())
        .map(|(s, _)| *s);
    if drag.drag_stopped() || stopped_resize.is_some() {
        // Edges that stuck stay put; the rest go to the 8 pt grid.
        let rect = Rect::from_min_size(card.pos, card.size);
        let s = match (snapping, stopped_resize) {
            (false, _) => None,
            (true, None) => Some(card::snap_move(rect, &others, bounds)),
            (true, Some(sides)) => Some(card::snap_resize(rect, &others, bounds, sides)),
        };
        let (sx, sy) = s.as_ref().map_or((false, false), |s| (s.x, s.y));
        let grid = |v: f32, stuck: bool| if stuck { v } else { (v / 8.0).round() * 8.0 };
        let rect = match stopped_resize {
            None => Rect::from_min_size(
                pos2(grid(rect.min.x, sx), grid(rect.min.y, sy)),
                rect.size(),
            ),
            Some(sides) => {
                let mut g = rect;
                if sides.left {
                    g.min.x = grid(g.min.x, sx).min(g.max.x - MIN_SIZE.x)
                }
                if sides.right {
                    g.max.x = grid(g.max.x, sx).max(g.min.x + MIN_SIZE.x)
                }
                if sides.top {
                    g.min.y = grid(g.min.y, sy).min(g.max.y - MIN_SIZE.y)
                }
                if sides.bottom {
                    g.max.y = grid(g.max.y, sy).max(g.min.y + MIN_SIZE.y)
                }
                g
            }
        };
        card.pos = rect.min;
        card.size = rect.size();
        ui.data_mut(|d| d.remove::<(Rect, Vec2)>(raw_id));
        ui.data_mut(|d| d.remove::<Pos2>(detach_id));
        out.push(Action::Moved);
    }
    if let Some(pos) = detached {
        out.push(Action::Detach(pos));
    }
    if !window_drag && drag.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    } else if drag.hovered() && resize_hover.is_none() && !locked {
        ui.ctx().set_cursor_icon(CursorIcon::Grab);
    }
    if let Some(sides) = resize.map(|(s, _)| *s).or(resize_hover) {
        ui.ctx().set_cursor_icon(sides.cursor());
    }

    let rect = Rect::from_min_size(origin + card.pos.to_vec2(), card.size);
    card_panel(ui, rect, hovered, style);
    if let Some(s) = &snapped {
        // Above every card, not just the ones painted before this one.
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            id.with("guides"),
        ));
        let stroke = Stroke::new(1.0, card.accent().gamma_multiply(0.8));
        for [a, b] in &s.guides {
            painter.line_segment([origin + a.to_vec2(), origin + b.to_vec2()], stroke);
        }
    }

    let inner = rect.shrink2(vec2(14.0, 10.0));
    let meta = Rect::from_min_size(inner.min, vec2(inner.width(), META_H));
    let footer = Rect::from_min_max(pos2(inner.left(), inner.bottom() - FOOTER_H), inner.max);
    let body = Rect::from_min_max(
        pos2(inner.left(), meta.bottom() + 4.0),
        pos2(inner.right(), footer.top() - 2.0),
    );
    let accent = theme::on_card(card.accent());

    paint_marker(ui, rect, card.hue(), style, hovered);

    // Tags and age only while the card is hovered, edited or selected; at rest a
    // card is its text, its color marker (a setting) and the kind's mark.
    let details = ui.ctx().animate_bool_with_time(
        id.with("details"),
        hovered || active || editing.is_some(),
        0.15,
    );

    // The hover actions stay while one of their menus is open, or the menu would
    // lose the button it hangs from as the pointer moves onto it.
    let snooze_popup = id.with("snooze");
    let more_popup = id.with("more");
    let hovered = hovered
        || egui::Popup::is_id_open(ui.ctx(), snooze_popup)
        || egui::Popup::is_id_open(ui.ctx(), more_popup);

    // Meta line, right: while hovered, a pill of actions: the kind's own first and
    // in its color, then "Позже" and "В архив" (off the layer, back later), and
    // the rest (pin, copy, fold, delete) under "Ещё". At rest, the pin of a
    // pinned card.
    let mut pill_left = meta.right();
    if hovered {
        let primary = primary_action(card);
        let primary_is_copy = matches!(primary, Some((_, _, Action::Copy)));
        // "Сделано" already archives a goal or reminder: no second archive button.
        let primary_is_done = matches!(primary, Some((_, _, Action::Done)));
        // A collapsed card has one action: open.
        let n = if card.collapsed {
            1
        } else {
            1 // more
                + usize::from(primary.is_some())
                + usize::from(card.kind == Kind::Private)
                + usize::from(!locked) // later
                + usize::from(!locked && !primary_is_done) // archive
        };
        let width = n as f32 * 24.0 + (n as f32 - 1.0) * 2.0 + 8.0;
        let pill = Rect::from_min_max(
            pos2(meta.right() + 4.0 - width, meta.center().y - 13.0),
            pos2(meta.right() + 4.0, meta.center().y + 13.0),
        );
        pill_left = pill.left();
        let p = ui.painter();
        let shadow = Color32::from_black_alpha(if theme::is_light() { 24 } else { 70 });
        p.add(
            egui::epaint::Shadow {
                offset: [0, 2],
                blur: 10,
                spread: 0,
                color: shadow,
            }
            .as_shape(pill, CornerRadius::same(7)),
        );
        p.rect(
            pill,
            CornerRadius::same(7),
            theme::glass_fill_hover(),
            Stroke::new(1.0, theme::glass_stroke()),
            StrokeKind::Inside,
        );
        // The pill is the theme's glass whatever the card's background is painted
        // with, so its icons take the theme's colors, not the card's.
        let pill_accent = card.accent();
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(pill.shrink2(vec2(4.0, 2.0)))
                .layout(Layout::right_to_left(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if card.collapsed {
                    if icon_button(ui, "\u{E70D}", "Развернуть", theme::dim()).clicked() {
                        out.push(Action::ToggleCollapse);
                    }
                    return;
                }
                let more = icon_button(ui, "\u{E712}", "Ещё", theme::dim());
                more_menu(&more, more_popup, locked, !primary_is_copy, &mut out);
                if !locked {
                    if !primary_is_done
                        && icon_button(ui, "\u{E7B8}", "В архив", theme::dim()).clicked()
                    {
                        out.push(Action::Archive);
                    }
                    let later = icon_button(ui, "\u{E708}", "Позже", theme::dim());
                    snooze_menu(&later, snooze_popup, &mut out);
                }
                if card.kind == Kind::Private
                    && icon_button(ui, "\u{E890}", "Показать на 5 секунд", theme::dim()).clicked()
                {
                    out.push(Action::Reveal);
                }
                // The check mark on the kind's copy button says the text is on the
                // clipboard (copy from "Ещё" has only the toast).
                let copied_id = id.with("copied");
                let copied = ui
                    .data(|d| d.get_temp::<Instant>(copied_id))
                    .filter(|t| primary_is_copy && t.elapsed() < COPIED_FOR);
                if let Some(t) = copied {
                    ui.ctx()
                        .request_repaint_after(COPIED_FOR.saturating_sub(t.elapsed()));
                }
                if let Some((glyph, tip, action)) = primary {
                    let is_copy = matches!(action, Action::Copy);
                    let (glyph, tip, color) = if is_copy && copied.is_some() {
                        ("\u{E73E}", "Текст скопирован", theme::SUCCESS)
                    } else {
                        (glyph, tip, pill_accent)
                    };
                    // Its own face: a wash of the kind's color under the glyph.
                    let next = ui.available_rect_before_wrap();
                    let face = Rect::from_min_max(
                        pos2(next.right() - 24.0, next.center().y - 11.0),
                        pos2(next.right(), next.center().y + 11.0),
                    );
                    ui.painter().rect_filled(
                        face,
                        CornerRadius::same(6),
                        pill_accent.gamma_multiply(0.16),
                    );
                    if icon_button(ui, glyph, tip, color).clicked() {
                        out.push(action);
                    }
                }
            },
        );
    } else if card.pinned {
        ui.painter().text(
            pos2(meta.right(), meta.center().y),
            Align2::RIGHT_CENTER,
            "\u{E840}",
            theme::icons(11.0),
            accent,
        );
    }

    // Meta line, left: the kind, a dot and its name (or its glyph, a setting),
    // which opens the menu to change the kind. A plain note has nothing to say
    // there and shows it only on hover.
    let glyph_mode = style.icon == card::IconSpot::BottomRight;
    let plain_note = card.kind == Kind::Note && card.tint.is_none();
    let mark_shown = if card.collapsed {
        1.0
    } else if plain_note || style.icon == card::IconSpot::Hover {
        details
    } else {
        1.0
    };
    let mark_font = if glyph_mode {
        theme::icons(if style.bold_icon { 14.0 } else { 12.0 })
    } else {
        theme::semibold(11.5)
    };
    let mark_text = match (glyph_mode, card.kind, card.review_at) {
        (true, ..) => card.kind.icon().to_owned(),
        // A reminder says when it's due.
        (false, Kind::Reminder, Some(at)) => {
            format!("{} \u{B7} {}", card.kind.label(), short_date(at))
        }
        _ => card.kind.label().to_owned(),
    };
    let mark_galley = ui.painter().layout_no_wrap(mark_text, mark_font, accent);
    let dot_w = if glyph_mode { 0.0 } else { 13.0 };
    let mark_w = (dot_w + mark_galley.size().x + 12.0).min((pill_left - meta.left()).max(20.0));
    let mark_rect = Rect::from_min_size(
        pos2(meta.left() - 6.0, meta.center().y - 10.0),
        vec2(mark_w, 20.0),
    );
    let sense = if mark_shown > 0.5 {
        Sense::click()
    } else {
        Sense::hover()
    };
    let kind = ui.interact(mark_rect, id.with("kind"), sense);
    kind.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("Тип: {}", card.kind.label()),
        )
    });
    let menu_open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&kind));
    let mut painter = ui
        .painter()
        .with_clip_rect(mark_rect.intersect(ui.clip_rect()));
    painter.multiply_opacity(mark_shown);
    if kind.hovered() && mark_shown > 0.5 || menu_open {
        painter.rect_filled(mark_rect, CornerRadius::same(6), theme::wash(22));
    }
    let mut x = mark_rect.left() + 6.0;
    if !glyph_mode {
        painter.circle_filled(pos2(x + 3.5, mark_rect.center().y), 3.5, accent);
        x += dot_w;
    }
    painter.galley(
        pos2(x, mark_rect.center().y - mark_galley.size().y / 2.0),
        mark_galley,
        accent,
    );
    if kind.clicked() {
        out.push(Action::Front);
    }
    let kind = if mark_shown > 0.5 {
        kind.on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(format!("{} — сменить тип", card.kind.label()))
    } else {
        kind
    };
    kind_menu(&kind, card, &mut out, true, window_drag);

    // Footer while editing: the formatting toolbar in place of tags and age. Drawn
    // before the body, so a click applies in the same frame the editor reads it.
    // A Private card's editor is plain text (the secret is stored verbatim): no toolbar.
    let toolbar_shown = editing.is_some() && card.kind != Kind::Private;
    if let Some(buf) = editing.as_deref_mut().filter(|_| toolbar_shown) {
        let bar = footer;
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(bar)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.set_clip_rect(bar.intersect(ui.clip_rect()));
                format_toolbar(ui, id.with("editor"), buf);
            },
        );
    }

    // Body: scrolls when the text doesn't fit; the floating bar shows only on hover.
    let hit = if card.collapsed {
        None
    } else {
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(body)
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ui.set_clip_rect(body.intersect(ui.clip_rect()));
                // A thin bar over the text, only while the pointer is on the card,
                // like the scrollbars of Windows 11; not egui's solid gutter.
                ui.spacing_mut().scroll = egui::style::ScrollStyle {
                    bar_width: 6.0,
                    floating_width: 3.0,
                    ..egui::style::ScrollStyle::floating()
                };
                egui::ScrollArea::vertical()
                    .id_salt(id.with("scroll"))
                    .auto_shrink([false, false])
                    .max_height(body.height())
                    .show(ui, |ui| {
                        card_body(ui, card, editing, revealed, revealed_text, style, details)
                    })
                    .inner
            },
        )
        .inner
    };
    // A click on a link, a check box or a copy button in the text is that, not
    // the start of editing.
    if start_edit && hit.is_none() {
        out.push(Action::StartEdit);
    }
    if let Some(BodyHit::ToggleCheck(line)) = hit {
        out.push(Action::ToggleCheck(line));
    }

    // Footer: tags and age.
    let mut painter = ui.painter().with_clip_rect(footer);
    // While editing, the toolbar has the footer.
    painter.multiply_opacity(if toolbar_shown || card.collapsed {
        0.0
    } else {
        details
    });
    let age = painter.text(
        pos2(footer.right(), footer.center().y),
        Align2::RIGHT_CENTER,
        age_label(card.created_at),
        FontId::proportional(11.5),
        theme::card_muted(),
    );
    let mut x = footer.left();
    // An idea's status leads the footer: always there once set (it's what the
    // idea is), offered with the other details while it isn't.
    if card.kind == Kind::Idea && !card.collapsed && !toolbar_shown {
        let popup = id.with("idea-status");
        let menu_open = egui::Popup::is_id_open(ui.ctx(), popup);
        let shown = if card.idea_status.is_some() || menu_open {
            1.0
        } else {
            details
        };
        if shown > 0.0 {
            let (text, color) = match card.idea_status {
                Some(s) => (s.label(), s.color()),
                None => ("Статус", theme::card_muted()),
            };
            let galley =
                ui.painter()
                    .layout_no_wrap(text.to_owned(), FontId::proportional(11.5), color);
            let chip = Rect::from_min_size(
                pos2(x, footer.center().y - galley.size().y / 2.0 - 2.0),
                galley.size() + vec2(12.0, 4.0),
            );
            let resp = ui.interact(
                chip,
                popup.with("chip"),
                if shown > 0.5 {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            resp.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    format!("Статус идеи: {text}"),
                )
            });
            let mut p = ui.painter().with_clip_rect(footer);
            p.multiply_opacity(shown);
            let fill = if resp.hovered() || menu_open {
                theme::wash(30)
            } else {
                theme::chip_fill()
            };
            p.rect_filled(chip, CornerRadius::same(6), fill);
            if card.idea_status.is_some() {
                p.rect_stroke(
                    chip,
                    CornerRadius::same(6),
                    Stroke::new(1.0, color.gamma_multiply(0.6)),
                    StrokeKind::Inside,
                );
            }
            p.galley(chip.min + vec2(6.0, 2.0), galley, color);
            let resp = if shown > 0.5 {
                resp.on_hover_cursor(CursorIcon::PointingHand)
            } else {
                resp
            };
            idea_status_menu(&resp, popup, card.idea_status, &mut out);
            x = chip.right() + 4.0;
        }
    }
    // Tags. While the details show, a click on a chip takes the tag out of the
    // text and "+" adds one; a Private card's text is its secret, not edited here.
    let editable_tags =
        card.kind != Kind::Private && !toolbar_shown && !card.collapsed && details > 0.5;
    const PLUS_W: f32 = 22.0;
    let room = age.left() - 8.0 - if editable_tags { PLUS_W + 4.0 } else { 0.0 };
    for tag in &card.tags {
        let galley = painter.layout_no_wrap(
            format!("#{tag}"),
            FontId::proportional(11.5),
            theme::card_dim(),
        );
        let cross = if editable_tags { 12.0 } else { 0.0 };
        let chip = Rect::from_min_size(
            pos2(x, footer.center().y - galley.size().y / 2.0 - 2.0),
            galley.size() + vec2(12.0 + cross, 4.0),
        );
        if chip.right() > room {
            break;
        }
        let resp = editable_tags
            .then(|| ui.interact(chip, id.with(("tag", tag.as_str())), Sense::click()));
        let chip_hovered = resp.as_ref().is_some_and(egui::Response::hovered);
        painter.rect_filled(
            chip,
            CornerRadius::same(6),
            if chip_hovered {
                theme::wash(30)
            } else {
                theme::chip_fill()
            },
        );
        painter.galley(chip.min + vec2(6.0, 2.0), galley, theme::card_dim());
        if let Some(resp) = resp {
            let color = if chip_hovered {
                theme::card_text()
            } else {
                theme::card_muted()
            };
            painter.text(
                pos2(chip.right() - 9.0, chip.center().y),
                Align2::CENTER_CENTER,
                "\u{E711}",
                theme::icons(7.5),
                color,
            );
            resp.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    format!("Убрать тег #{tag}"),
                )
            });
            if resp
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text("Убрать тег")
                .clicked()
            {
                out.push(Action::RemoveTag(tag.clone()));
            }
        }
        x = chip.right() + 4.0;
    }
    if editable_tags {
        let plus = Rect::from_min_size(pos2(x, footer.center().y - 9.0), vec2(PLUS_W, 18.0));
        let resp = ui.interact(plus, id.with("add-tag"), Sense::click());
        painter.rect_filled(
            plus,
            CornerRadius::same(6),
            if resp.hovered() {
                theme::wash(30)
            } else {
                theme::chip_fill()
            },
        );
        painter.text(
            plus.center(),
            Align2::CENTER_CENTER,
            "\u{E710}",
            theme::icons(9.5),
            theme::card_dim(),
        );
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Добавить тег")
        });
        if resp
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Добавить тег")
            .clicked()
        {
            out.push(Action::AddTag(plus.left_bottom()));
        }
    }

    // The resize grip, where a card can be resized.
    if hovered && !locked && !card.collapsed {
        let p = ui.painter();
        let c = rect.max - vec2(6.0, 6.0);
        let stroke = Stroke::new(1.2, theme::wash(if corner_hovered { 120 } else { 50 }));
        p.line_segment([c - vec2(8.0, 0.0), c - vec2(0.0, 8.0)], stroke);
        p.line_segment([c - vec2(4.0, 0.0), c - vec2(0.0, 4.0)], stroke);
    }

    if card.collapsed {
        let line = card.collapsed_line();
        // Clear of the open button and the pin while those show.
        let right = if hovered {
            pill_left - 8.0
        } else if card.pinned {
            meta.right() - 22.0
        } else {
            inner.right()
        };
        let left = mark_rect.right() + 2.0;
        let mut job = egui::text::LayoutJob::single_section(
            line.clone(),
            egui::TextFormat {
                font_id: theme::card_font(style.text_size()),
                color: theme::card_text(),
                ..Default::default()
            },
        );
        job.wrap = egui::text::TextWrapping {
            max_width: (right - left).max(0.0),
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        ui.painter().galley(
            pos2(left, rect.center().y - galley.size().y / 2.0),
            galley,
            theme::card_text(),
        );
        bg.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Свёрнута: {line}"))
        });
        card.size = full_size;
    }

    out
}
