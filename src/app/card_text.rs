//! A card's text as shown (links, lists, a Reference's copy buttons, a
//! Private card's bars) and its editor with the formatting toolbar.

use std::time::Instant;

use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Id, Key, Pos2, Rect, RichText, Sense,
    Stroke, StrokeKind, Ui, pos2, vec2,
};

use crate::card::{self, Card, Kind};
use crate::rich_text;
use crate::theme;
use crate::win;

use super::card_ui::COPIED_FOR;

/// Tags are shown as chips, so hide the `#tag` tokens from the text itself.
fn without_tags(text: &str) -> String {
    text.lines()
        .map(|line| {
            line.split(' ')
                .filter(|w| !(w.starts_with('#') && w.len() > 1))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

/// What a click in a card's text did, besides placing the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BodyHit {
    /// A link opened.
    Link,
    /// A line of a Reference was copied.
    Copied,
    /// The check box on this line was clicked.
    ToggleCheck(usize),
}

/// Room at the right of a Reference's text for the copy buttons of its lines.
const COPY_GUTTER: f32 = 26.0;

/// The date of a reminder, short: "29 сен".
pub(super) fn short_date(unix: i64) -> String {
    const MONTHS: [&str; 12] = ["янв", "фев", "мар", "апр", "мая", "июн", "июл", "авг", "сен", "окт", "ноя", "дек"];
    let (_, m, d) = crate::search::civil_from_days((unix + crate::search::local_offset_secs()).div_euclid(86_400));
    format!("{d} {}", MONTHS[(m - 1).clamp(0, 11) as usize])
}

/// Text of a card, or its editor. `details`: how far the hover details are shown.
pub(super) fn card_body(
    ui: &mut Ui,
    card: &Card,
    editing: Option<&mut String>,
    revealed: bool,
    revealed_text: Option<&str>,
    style: card::CardStyle,
    details: f32,
) -> Option<BodyHit> {
    let base = style.text_size();
    let editor_id = Id::new(("card", card.id)).with("editor");
    // Where the last click in the text landed, in chars of the editor's text.
    let click_id = editor_id.with("click");
    if let Some(buf) = editing {
        // The editor shows the text as it looks, no markup (like Sticky Notes):
        // plain text with a style per char, written back as markup each frame.
        let (mut plain, mut styles) = rich_text::parse(buf);
        let before = plain.clone();
        // Opening: the editor wasn't drawn last frame. A toolbar click takes the
        // focus away for a moment and must not count as opening.
        let pass = ui.ctx().cumulative_pass_nr();
        let shown_id = editor_id.with("shown");
        let opening = ui.data(|d| d.get_temp::<u64>(shown_id)).is_none_or(|last| last + 1 < pass);
        ui.data_mut(|d| d.insert_temp(shown_id, pass));
        if opening {
            // The cursor goes where the text was clicked, else to the end, never
            // where it was the last time this card was edited.
            ui.data_mut(|d| d.remove::<(usize, rich_text::Style)>(editor_id.with("typing")));
            let at = ui.data_mut(|d| d.remove_temp::<usize>(click_id)).unwrap_or_else(|| plain.chars().count());
            let mut state = egui::text_edit::TextEditState::load(ui.ctx(), editor_id).unwrap_or_default();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(at))));
            state.store(ui.ctx(), editor_id);
        }
        let shortcut_action = ui
            .memory(|m| m.has_focus(editor_id))
            .then(|| {
                ui.input(|input| {
                    input.events.iter().find_map(|event| match event {
                        egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. }
                            if modifiers.command && !modifiers.alt => match (key, modifiers.shift) {
                                (Key::B, false) => Some(rich_text::Action::Bold),
                                (Key::I, false) => Some(rich_text::Action::Italic),
                                (Key::U, false) => Some(rich_text::Action::Underline),
                                (Key::S, true) => Some(rich_text::Action::Strikethrough),
                                (Key::H, true) => Some(rich_text::Action::Highlight),
                                (Key::K, true) => Some(rich_text::Action::Code),
                                _ => None,
                            },
                        _ => None,
                    })
                })
            })
            .flatten();
        if let Some(action) = shortcut_action {
            apply_format(ui, editor_id, &mut styles, action);
        }
        let (selection, typing) = format_state(ui, editor_id);
        let typing = typing.filter(|_| selection.is_empty());
        let layout_styles = styles.clone();
        // Code runs take the row height of the card's font, as in the shown
        // text, so opening the editor doesn't reflow the lines.
        let row_height = ui.fonts_mut(|f| f.row_height(&theme::card_font(base)));
        let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
            let text = text.as_str();
            // Mid-edit the text is ahead of the styles; the edit is placed as
            // after the frame, only without the cursor.
            let styles = rich_text::restyle(&before, &layout_styles, text, None, typing);
            let mut job = egui::text::LayoutJob::default();
            let mut from = 0;
            let bytes: Vec<usize> = text.char_indices().map(|(i, _)| i).chain(std::iter::once(text.len())).collect();
            for i in 0..styles.len() {
                if i + 1 == styles.len() || styles[i + 1] != styles[i] {
                    let run = &text[bytes[from]..bytes[i + 1]];
                    let mut format = rich_format(editor_font(base, styles[i]), theme::card_text(), styles[i]);
                    if styles[i].code {
                        format.line_height = Some(row_height);
                        format.valign = Align::Center;
                    }
                    job.append(run, 0.0, format);
                    from = i + 1;
                }
            }
            if job.sections.is_empty() {
                let style = typing.unwrap_or_default();
                job.append("", 0.0, rich_format(editor_font(base, style), theme::card_text(), style));
            }
            job.wrap.max_width = wrap_width;
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let resp = ui.add(
            egui::TextEdit::multiline(&mut plain)
                .id(editor_id)
                .font(theme::card_font(base))
                .text_color(theme::card_text())
                .layouter(&mut layouter)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .desired_rows(3),
        );
        if plain != before {
            let cursor = editor_cursor(ui, editor_id).map(|r| r.end);
            styles = rich_text::restyle(&before, &styles, &plain, cursor, typing);
            // What's typed has the style now; the next char takes it from there.
            ui.data_mut(|d| d.remove::<(usize, rich_text::Style)>(editor_id.with("typing")));
        }
        *buf = rich_text::serialize(&plain, &styles);
        // Also right after a toolbar click, which took the focus for a frame.
        if !resp.has_focus() {
            resp.request_focus();
        }
        return None;
    }
    if let Some(reason) = card.resurface_reason(crate::resurface::unix_now()) {
        ui.label(RichText::new(reason).size(11.5).color(theme::card_dim()));
        ui.add_space(3.0);
    }
    let body = without_tags(if card.kind == Kind::Private && revealed { revealed_text.unwrap_or("") } else { &card.body });
    if card.kind == Kind::Private && !revealed {
        // The heading stays readable (see card::private_label); only the rest is barred.
        let label = (!card.title.is_empty()).then(|| card.title.clone());
        let rest = "••••••••••".to_owned();
        let heading_at = label.as_ref().and_then(|label| {
            let job = egui::text::LayoutJob::simple(label.clone(), theme::card_bold(base + 0.5), theme::card_text(), ui.available_width());
            let (pos, galley, resp) = egui::Label::new(job).wrap().selectable(false).layout_in_ui(ui);
            let at = char_at(ui, &galley, pos, resp.rect);
            ui.painter().galley(pos, galley, theme::card_text());
            at
        });
        let size = if label.is_none() { base } else { base - 1.0 };
        let rest_at = redacted(ui, &rest, theme::card_font(size), theme::card_dim().gamma_multiply(0.45));
        remember_click(ui, click_id, card, label.as_deref(), heading_at, &rest, rest_at);
        return None;
    }
    // A Prompt's first line is its name.
    let (heading, text) = match card.kind {
        Kind::Prompt if card.title.is_empty() => match card::prompt_name(&body) {
            Some((name, rest)) => (Some(name.to_owned()), rest.to_owned()),
            None => (None, body),
        },
        _ => ((!card.title.is_empty()).then(|| card.title.clone()), body),
    };
    let displayed_heading = heading.as_ref().map(|heading| rich_text::strip_markup(heading));
    let heading_at = heading.as_ref().and_then(|heading| {
        let heading = rich_text::strip_markup(heading);
        let job = egui::text::LayoutJob::simple(heading, theme::card_bold(base + 0.5), theme::card_text(), ui.available_width());
        let (pos, galley, resp) = egui::Label::new(job).wrap().selectable(false).layout_in_ui(ui);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, galley.text()));
        let at = char_at(ui, &galley, pos, resp.rect);
        galley_fading(ui, pos, galley, theme::card_text());
        at
    });
    let color = if heading.is_none() { theme::card_text() } else { theme::card_dim() };
    let size = if heading.is_none() { base } else { base - 1.0 };
    // Every row as tall as a row of the card's own font: Consolas' rows are
    // shorter, and a command right under a heading sat on it.
    let row_height = ui.fonts_mut(|f| f.row_height(&theme::card_font(size)));
    let accent = theme::on_card(card.accent());
    let plain = rich_text::strip_markup(&text);
    let plain_lines: Vec<&str> = plain.lines().collect();
    // A line that starts with a list marker shows a bullet or a check box in
    // its place; the text keeps the marker, as Sticky Notes' lists do.
    let marks_of: Vec<Option<(card::ListMark, usize)>> = plain_lines.iter().map(|l| card::list_mark(l)).collect();
    // In a Reference, commands, paths and hosts are set in monospace and copied
    // a line at a time: a gutter at the right holds their copy buttons.
    let technical: Vec<bool> = plain_lines.iter().map(|l| card.kind == Kind::Reference && card::looks_technical(l)).collect();
    if technical.contains(&true) {
        ui.set_max_width(ui.available_width() - COPY_GUTTER);
    }
    // Addresses in link blue and clickable, in any kind; a Prompt's {{variables}} as chips.
    let mut job = egui::text::LayoutJob::default();
    let mut links: Vec<(std::ops::Range<usize>, String)> = Vec::new(); // char ranges in the galley
    let mut marks: Vec<(usize, card::ListMark, usize)> = Vec::new(); // (galley char, mark, line)
    let mut copy_lines: Vec<(usize, String)> = Vec::new(); // (galley char, the line)
    let mut chars = 0;
    let mut line_no = 0;
    let mut line_start = true;
    let mut skip = 0; // marker chars still to leave out of the galley
    let mut lead = 0.0; // space in place of the marker, before the line's first piece
    let mut done = false;
    for span in rich_text::spans(&text) {
        for piece in span.text.split_inclusive('\n') {
            let (line, newline) = piece.strip_suffix('\n').map_or((piece, ""), |line| (line, "\n"));
            if line_start {
                line_start = false;
                (skip, lead, done) = (0, 0.0, false);
                if let Some((mark, n)) = marks_of.get(line_no).copied().flatten() {
                    (skip, lead, done) = (n, mark.indent(), mark == card::ListMark::Done);
                    marks.push((chars, mark, line_no));
                }
                if technical.get(line_no).copied().unwrap_or(false) {
                    copy_lines.push((chars, plain_lines[line_no].trim().to_owned()));
                }
            }
            let mut line = line;
            if skip > 0 {
                let take = skip.min(line.chars().count());
                let byte = line.char_indices().nth(take).map_or(line.len(), |(i, _)| i);
                line = &line[byte..];
                skip -= take;
            }
            let mono = span.style.code || technical.get(line_no).copied().unwrap_or(false);
            // Appends a piece; returns the galley's length so far, in chars.
            let mut append = |s: &str, c: Color32, is_link: bool| {
                let from = chars;
                let mut emit = |s: &str, variable: bool| {
                    let font = if variable {
                        FontId::monospace(size - 1.5)
                    } else if mono {
                        FontId::monospace(size - 1.0)
                    } else if span.style.bold {
                        theme::card_bold(size)
                    } else {
                        theme::card_font(size)
                    };
                    let mut format = rich_format(font, if done { theme::card_muted() } else { c }, span.style);
                    // A variable sits on the line's baseline like the words around it.
                    if mono {
                        format.line_height = Some(row_height);
                        format.valign = Align::Center;
                    }
                    if variable {
                        format.color = accent;
                        format.background = accent.gamma_multiply(0.16);
                    }
                    if done {
                        format.strikethrough = Stroke::new(1.0, theme::card_muted());
                    }
                    job.append(s, std::mem::take(&mut lead), format);
                    chars += s.chars().count();
                };
                if is_link || card.kind != Kind::Prompt {
                    emit(s, false);
                } else {
                    let mut last = 0;
                    for (range, _) in card::prompt_placeholders(s) {
                        emit(&s[last..range.start], false);
                        emit(&s[range.clone()], true);
                        last = range.end;
                    }
                    emit(&s[last..], false);
                }
                from..chars
            };
            let mut rest = line;
            while let Some(start) = [rest.find("https://"), rest.find("http://")].into_iter().flatten().min() {
                let end = rest[start..].find(char::is_whitespace).map_or(rest.len(), |e| start + e);
                // "(see https://x.org)." — the closing punctuation isn't part of the address.
                let url = rest[start..end].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '"', '\'']);
                let end = start + url.len();
                append(&rest[..start], color, false);
                let range = append(url, theme::link(), true);
                links.push((range, url.to_owned()));
                rest = &rest[end..];
            }
            append(rest, color, false);
            if !newline.is_empty() {
                append(newline, color, false);
                line_no += 1;
                line_start = true;
            }
        }
    }
    let (pos, galley, resp) = egui::Label::new(job).wrap().selectable(false).layout_in_ui(ui);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, galley.text()));
    let text_at = char_at(ui, &galley, pos, resp.rect);
    let link = text_at.and_then(|at| links.iter().find(|(range, _)| range.contains(&at)).map(|(_, url)| url.as_str()));
    galley_fading(ui, pos, galley.clone(), color);
    let full = galley.rect.translate(pos.to_vec2());
    let clip = ui.clip_rect();
    let mut hit = None;

    // Bullets and check boxes, in the space left in front of their lines.
    for (at, mark, line) in &marks {
        let cursor = galley.pos_from_cursor(egui::text::CCursor::new(*at)).translate(pos.to_vec2());
        let alpha = fade_alpha(clip, full, cursor);
        if alpha <= 0.0 {
            continue;
        }
        let mut p = ui.painter().with_clip_rect(clip);
        p.multiply_opacity(alpha);
        let center = pos2(cursor.left() - mark.indent() + 7.0, cursor.center().y);
        match mark {
            card::ListMark::Bullet => {
                p.circle_filled(center, 2.5, color);
            }
            card::ListMark::Todo | card::ListMark::Done => {
                let done = *mark == card::ListMark::Done;
                let r = Rect::from_center_size(center, vec2(14.0, 14.0));
                let check = ui.interact(r.expand(3.0), click_id.with(("check", *line)), Sense::click());
                check.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, done, plain_lines.get(*line).copied().unwrap_or("")));
                if done {
                    p.rect_filled(r, CornerRadius::same(4), accent);
                    let on = theme::card_fill(false);
                    p.text(r.center(), Align2::CENTER_CENTER, "\u{E73E}", theme::icons(9.0), Color32::from_rgb(on.r(), on.g(), on.b()));
                } else {
                    let stroke = if check.hovered() { theme::card_text() } else { theme::card_muted() };
                    p.rect_stroke(r, CornerRadius::same(4), Stroke::new(1.5, stroke), StrokeKind::Inside);
                }
                if check.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    hit = Some(BodyHit::ToggleCheck(*line));
                }
            }
        }
    }

    // The copy buttons of a Reference's lines, in the gutter; a line just
    // copied shows a check mark for a moment.
    for (i, (at, line)) in copy_lines.iter().enumerate() {
        let cursor = galley.pos_from_cursor(egui::text::CCursor::new(*at)).translate(pos.to_vec2());
        let alpha = fade_alpha(clip, full, cursor);
        if alpha <= 0.0 {
            continue;
        }
        let button = Rect::from_center_size(pos2(clip.right() - COPY_GUTTER / 2.0, cursor.center().y), vec2(22.0, 20.0));
        let copy = ui.interact(button, click_id.with(("copy-line", i)), Sense::click());
        let copy_id = copy.id;
        copy.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Копировать строку: {line}")));
        let copied = ui.data(|d| d.get_temp::<Instant>(copy.id)).filter(|t| t.elapsed() < COPIED_FOR);
        if let Some(t) = copied {
            ui.ctx().request_repaint_after(COPIED_FOR.saturating_sub(t.elapsed()));
        }
        let shown = if copy.hovered() || copied.is_some() { 1.0 } else { details };
        if shown > 0.0 {
            let mut p = ui.painter().with_clip_rect(clip);
            p.multiply_opacity(alpha * shown);
            if copy.hovered() {
                let row = Rect::from_min_max(pos2(pos.x - 4.0, cursor.top()), pos2(clip.right(), cursor.bottom()));
                p.rect_filled(row, CornerRadius::same(4), theme::wash(14));
                p.rect_filled(button, CornerRadius::same(6), theme::wash(22));
            }
            let (glyph, c) = if copied.is_some() { ("\u{E73E}", theme::SUCCESS) } else { ("\u{E8C8}", theme::card_dim()) };
            p.text(button.center(), Align2::CENTER_CENTER, glyph, theme::icons(11.0), c);
        }
        if copy.on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Копировать строку").clicked() {
            ui.ctx().copy_text(line.clone());
            ui.data_mut(|d| d.insert_temp(copy_id, Instant::now()));
            hit = Some(BodyHit::Copied);
        }
    }

    // The cursor goes where the click landed; the galley's text is what's shown,
    // without the list markers (shown_to_source skips what isn't in it).
    remember_click(ui, click_id, card, displayed_heading.as_deref(), heading_at, galley.text(), text_at);
    if let Some(url) = link {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        if ui.input(|i| i.pointer.primary_clicked()) {
            win::open_url(url);
            hit = Some(BodyHit::Link);
        }
    }
    if card.kind == Kind::Link
        && let Some(domain) = card::link_domain(&card.body)
    {
        ui.add_space(2.0);
        let color = theme::card_muted();
        let galley = ui.painter().layout(domain.to_owned(), FontId::proportional(12.0), color, ui.available_width());
        let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
        galley_fading(ui, rect.min, galley, color);
    }
    hit
}

/// How visible a row at `r` is inside `clip` when the text `full` runs past
/// it: 0 for a row cut by the edge, then 40 % and 75 % for the two rows before
/// it, 1 for the rest and for text that fits.
fn fade_alpha(clip: Rect, full: Rect, r: Rect) -> f32 {
    let past_bottom = full.bottom() > clip.bottom() + 0.5;
    let past_top = full.top() < clip.top() - 0.5;
    let h = r.height().max(1.0);
    let mut alpha: f32 = 1.0;
    if past_bottom {
        if r.bottom() > clip.bottom() + 0.5 {
            return 0.0;
        }
        alpha = alpha.min(0.4 + 0.35 * (clip.bottom() - r.bottom()) / h);
    }
    if past_top {
        if r.top() < clip.top() - 0.5 {
            return 0.0;
        }
        alpha = alpha.min(0.4 + 0.35 * (r.top() - clip.top()) / h);
    }
    alpha.min(1.0)
}

/// Paints a galley row by row, so text that runs past the card's edge ends on a
/// whole row and fades toward that edge, instead of being cut through a row by
/// the clip. Text that fits is painted as is.
fn galley_fading(ui: &Ui, pos: Pos2, galley: std::sync::Arc<egui::Galley>, color: Color32) {
    let clip = ui.clip_rect();
    let painter = ui.painter();
    let full = galley.rect.translate(pos.to_vec2());
    if full.bottom() <= clip.bottom() + 0.5 && full.top() >= clip.top() - 0.5 {
        painter.galley(pos, galley, color);
        return;
    }
    for row in &galley.rows {
        let r = row.rect().translate(pos.to_vec2());
        let alpha = fade_alpha(clip, full, r);
        if alpha <= 0.0 {
            continue;
        }
        let mut p = painter.with_clip_rect(r.intersect(clip));
        p.multiply_opacity(alpha);
        p.galley(pos, galley.clone(), color);
    }
}

fn rich_format(font: FontId, color: Color32, style: rich_text::Style) -> egui::TextFormat {
    egui::TextFormat {
        font_id: font,
        color,
        italics: style.italic,
        background: if style.highlight { Color32::from_rgba_unmultiplied(250, 204, 21, 42) } else { Color32::TRANSPARENT },
        underline: if style.underline { Stroke::new(1.0, color) } else { Stroke::NONE },
        strikethrough: if style.strikethrough { Stroke::new(1.0, color) } else { Stroke::NONE },
        ..Default::default()
    }
}

/// Formatting buttons along the bottom of a card being edited. A button shows
/// pressed when the whole selection has its style or, with no selection, when
/// typing at the cursor gets it.
pub(super) fn format_toolbar(ui: &mut Ui, editor_id: Id, buf: &mut String) {
    use rich_text::Action as A;
    const BUTTONS: [(&str, &str, A); 7] = [
        ("\u{E8DD}", "Жирный (Ctrl+B)", A::Bold),
        ("\u{E8DB}", "Курсив (Ctrl+I)", A::Italic),
        ("\u{E8DC}", "Подчёркивание (Ctrl+U)", A::Underline),
        ("\u{EDE0}", "Зачёркивание (Ctrl+Shift+S)", A::Strikethrough),
        ("\u{E7E6}", "Выделение маркером (Ctrl+Shift+H)", A::Highlight),
        ("\u{E943}", "Моноширинный код (Ctrl+Shift+K)", A::Code),
        ("\u{E75C}", "Снять форматирование", A::Clear),
    ];
    let (plain, mut styles) = rich_text::parse(buf);
    let (selection, typing) = format_state(ui, editor_id);
    let at_cursor = typing.unwrap_or_else(|| rich_text::style_at(&styles, selection.start));
    ui.spacing_mut().item_spacing.x = 1.0;
    let mut clicked = None;
    for (glyph, tip, action) in BUTTONS {
        let active = action != A::Clear
            && if selection.is_empty() { at_cursor.has(action) } else { rich_text::all_have(&styles, selection.clone(), action) };
        let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 22.0), Sense::click());
        if active || resp.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(6), theme::wash(if active { 34 } else { 22 }));
        }
        let color = if active { theme::card_text() } else { theme::card_dim() };
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, theme::icons(12.5), color);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, tip));
        if resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tip).clicked() {
            clicked = Some(action);
        }
    }
    if let Some(action) = clicked {
        apply_format(ui, editor_id, &mut styles, action);
        *buf = rich_text::serialize(&plain, &styles);
    }
}

fn editor_font(base: f32, style: rich_text::Style) -> FontId {
    if style.code {
        FontId::monospace(base - 1.0)
    } else if style.bold {
        theme::card_bold(base)
    } else {
        theme::card_font(base)
    }
}

/// The editor's selection in chars, sorted; empty at the cursor.
fn editor_cursor(ui: &Ui, editor_id: Id) -> Option<std::ops::Range<usize>> {
    let state = egui::text_edit::TextEditState::load(ui.ctx(), editor_id)?;
    let range = state.cursor.char_range()?.as_sorted_char_range();
    Some(range.start.0..range.end.0)
}

/// The selection, and the style picked for typing at the cursor while it stays there.
fn format_state(ui: &Ui, editor_id: Id) -> (std::ops::Range<usize>, Option<rich_text::Style>) {
    let selection = editor_cursor(ui, editor_id).unwrap_or(0..0);
    let typing = ui
        .data(|d| d.get_temp::<(usize, rich_text::Style)>(editor_id.with("typing")))
        .filter(|(at, _)| selection.is_empty() && *at == selection.start)
        .map(|(_, style)| style);
    (selection, typing)
}

/// Toggles a style on the selection or, with none, for what's typed next.
fn apply_format(ui: &Ui, editor_id: Id, styles: &mut [rich_text::Style], action: rich_text::Action) {
    let (selection, typing) = format_state(ui, editor_id);
    if selection.is_empty() {
        let style = typing.unwrap_or_else(|| rich_text::style_at(styles, selection.start)).toggled(action);
        ui.data_mut(|d| d.insert_temp(editor_id.with("typing"), (selection.start, style)));
    } else {
        rich_text::toggle(styles, selection, action);
    }
}

/// What the editor opens with: the note as written, an old separate title as its first line.
pub(super) fn edit_text(card: &Card) -> String {
    if card.title.is_empty() { card.body.clone() } else { format!("{}\n{}", card.title, card.body) }
}

/// The char under the pointer in a galley painted at `pos` over `rect`.
/// Not `Response::hover_pos`: while the card's background is being clicked egui
/// hovers nothing else, so a label would never see the click.
fn char_at(ui: &Ui, galley: &egui::Galley, pos: Pos2, rect: Rect) -> Option<usize> {
    let p = ui.input(|i| i.pointer.interact_pos()).filter(|_| ui.rect_contains_pointer(rect))?;
    Some(galley.cursor_from_pos(p - pos).index.0)
}

/// On a click in a card's text, keeps where it landed in the editor's text, for
/// the cursor when the click opens the editor. `heading_at`/`text_at`: chars in
/// the shown heading and text.
fn remember_click(
    ui: &Ui,
    click_id: Id,
    card: &Card,
    heading: Option<&str>,
    heading_at: Option<usize>,
    text: &str,
    text_at: Option<usize>,
) {
    if !ui.input(|i| i.pointer.primary_clicked()) {
        return;
    }
    let shown = match heading {
        Some(h) => format!("{h}\n{text}"),
        None => text.to_owned(),
    };
    let heading_len = heading.map_or(0, |h| h.chars().count() + 1);
    let at = match (heading_at, text_at) {
        (Some(at), _) => at,
        (None, Some(at)) => heading_len + at,
        (None, None) => return ui.data_mut(|d| d.remove::<usize>(click_id)),
    };
    // The editor shows the text without markup.
    let at = shown_to_source(&shown, at, &rich_text::strip_markup(&edit_text(card)));
    ui.data_mut(|d| d.insert_temp(click_id, at));
}

/// Where char `at` of `shown` sits in `source`. The shown text is the source with
/// bits left out (tags, trimmed space), so its chars are matched in order.
fn shown_to_source(shown: &str, at: usize, source: &str) -> usize {
    let mut rest = source.chars().enumerate();
    let mut end = 0;
    for c in shown.chars().take(at) {
        match rest.by_ref().find(|&(_, s)| s == c) {
            Some((i, _)) => end = i + 1,
            None => return source.chars().count(),
        }
    }
    end
}

/// Text laid out as usual but painted as one bar per word: the note keeps its
/// shape (line and word lengths, paragraphs) without a readable letter.
/// Returns the char under the pointer, like [`char_at`].
fn redacted(ui: &mut Ui, text: &str, font: FontId, color: Color32) -> Option<usize> {
    let galley = ui.painter().layout(text.to_owned(), font, color, ui.available_width());
    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    let at = char_at(ui, &galley, rect.min, rect);
    let p = ui.painter();
    for row in &galley.rows {
        let r = row.rect().translate(rect.min.to_vec2());
        let y = (r.top() + r.height() * 0.28)..=(r.bottom() - r.height() * 0.2);
        let bar = |(a, b): (f32, f32)| p.rect_filled(Rect::from_x_y_ranges((r.left() + a)..=(r.left() + b), y.clone()), 2.0, color);
        let mut run: Option<(f32, f32)> = None;
        for g in &row.glyphs {
            if g.chr.is_whitespace() {
                run.take().map(bar);
            } else {
                run = Some((run.map_or(g.pos.x, |(a, _)| a), g.pos.x + g.advance_width));
            }
        }
        run.map(bar);
    }
    at
}
