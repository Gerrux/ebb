//! Small inline markup used by note bodies.
//!
//! The editor keeps the markup in the note text (which means old databases do
//! not need a schema migration), while the card view renders it as rich text.

use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub highlight: bool,
    pub code: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Highlight,
    Code,
    Clear,
}

impl Action {
    fn marker(self) -> Option<&'static str> {
        match self {
            Self::Bold => Some("**"),
            Self::Italic => Some("*"),
            Self::Underline => Some("__"),
            Self::Strikethrough => Some("~~"),
            Self::Highlight => Some("=="),
            Self::Code => Some("`"),
            Self::Clear => None,
        }
    }

    fn flip(self, style: &mut Style) {
        match self {
            Self::Bold => style.bold = !style.bold,
            Self::Italic => style.italic = !style.italic,
            Self::Underline => style.underline = !style.underline,
            Self::Strikethrough => style.strikethrough = !style.strikethrough,
            Self::Highlight => style.highlight = !style.highlight,
            Self::Code => style.code = !style.code,
            Self::Clear => *style = Style::default(),
        }
    }
}

/// Parses the supported markers and returns only visible text in each span.
pub fn spans(text: &str) -> Vec<Span> {
    let mut result = Vec::new();
    let mut style = Style::default();
    let mut chunk = 0;
    let mut cursor = 0;

    while cursor < text.len() {
        let Some((marker, len, action)) = marker_at(&text[cursor..]) else {
            cursor += text[cursor..].chars().next().map_or(1, char::len_utf8);
            continue;
        };
        // A marker without a closing partner is ordinary text. This keeps
        // prose such as "2 * 3" and an unfinished edit visible.
        if !is_active(action, style) && !text[cursor + len..].contains(marker) {
            cursor += len;
            continue;
        }
        if chunk < cursor {
            result.push(Span {
                text: text[chunk..cursor].to_owned(),
                style,
            });
        }
        action.flip(&mut style);
        cursor += len;
        chunk = cursor;
    }
    if chunk < text.len() {
        result.push(Span {
            text: text[chunk..].to_owned(),
            style,
        });
    }
    result
}

pub fn strip_markup(text: &str) -> String {
    spans(text).into_iter().map(|span| span.text).collect()
}

/// Toggle one style around a selected character range. The returned range is
/// still the user's visible text, so repeated Ctrl+B toggles it back off.
pub fn apply(text: &str, selection: Range<usize>, action: Action) -> (String, Range<usize>) {
    let start = byte_at(text, selection.start);
    let end = byte_at(text, selection.end);
    if start >= end {
        if let Some(marker) = action.marker() {
            let mut output = String::with_capacity(text.len() + marker.len() * 2);
            output.push_str(&text[..start]);
            output.push_str(marker);
            output.push_str(marker);
            output.push_str(&text[start..]);
            let cursor = selection.start + marker.chars().count();
            return (output, cursor..cursor);
        }
        return (text.to_owned(), selection.start..selection.end);
    }

    let selected = &text[start..end];
    if action == Action::Clear {
        let mut left = text[..start].to_owned();
        let mut right = text[end..].to_owned();
        for marker in markers() {
            if left.ends_with(marker) && right.starts_with(marker) {
                left.truncate(left.len() - marker.len());
                right.drain(..marker.len());
            }
        }
        let visible = strip_all_markers(selected);
        let new_start = left.chars().count();
        let new_end = new_start + visible.chars().count();
        left.push_str(&visible);
        left.push_str(&right);
        return (left, new_start..new_end);
    }

    let marker = action.marker().expect("non-clear action has a marker");
    if text[..start].ends_with(marker) && text[end..].starts_with(marker) {
        let mut output = String::with_capacity(text.len() - marker.len() * 2);
        output.push_str(&text[..start - marker.len()]);
        output.push_str(selected);
        output.push_str(&text[end + marker.len()..]);
        let shift = marker.chars().count();
        return (output, selection.start - shift..selection.end - shift);
    }

    let mut output = String::with_capacity(text.len() + marker.len() * 2);
    output.push_str(&text[..start]);
    output.push_str(marker);
    output.push_str(selected);
    output.push_str(marker);
    output.push_str(&text[end..]);
    let shift = marker.chars().count();
    (output, selection.start + shift..selection.end + shift)
}

fn marker_at(text: &str) -> Option<(&'static str, usize, Action)> {
    [
        ("**", Action::Bold),
        ("__", Action::Underline),
        ("~~", Action::Strikethrough),
        ("==", Action::Highlight),
        ("`", Action::Code),
        ("*", Action::Italic),
    ]
    .into_iter()
    .find_map(|(marker, action)| {
        text.starts_with(marker)
            .then_some((marker, marker.len(), action))
    })
}

fn is_active(action: Action, style: Style) -> bool {
    match action {
        Action::Bold => style.bold,
        Action::Italic => style.italic,
        Action::Underline => style.underline,
        Action::Strikethrough => style.strikethrough,
        Action::Highlight => style.highlight,
        Action::Code => style.code,
        Action::Clear => false,
    }
}

fn markers() -> [&'static str; 6] {
    ["**", "__", "~~", "==", "`", "*"]
}

fn strip_all_markers(text: &str) -> String {
    let mut output = text.to_owned();
    for marker in markers() {
        output = output.replace(marker, "");
    }
    output
}

fn byte_at(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(byte, _)| byte)
}

#[cfg(test)]
mod tests {
    use super::{Action, Style, apply, spans, strip_markup};

    #[test]
    fn parses_nested_styles_without_markers() {
        let parsed = spans("a **bold *and italic*** b");
        assert_eq!(parsed[0].text, "a ");
        assert_eq!(
            parsed[1],
            super::Span {
                text: "bold ".into(),
                style: Style {
                    bold: true,
                    ..Style::default()
                }
            }
        );
        assert_eq!(
            parsed[2].style,
            Style {
                bold: true,
                italic: true,
                ..Style::default()
            }
        );
        assert_eq!(strip_markup("a **bold**"), "a bold");
    }

    #[test]
    fn toggles_a_selection_and_toggles_it_back() {
        let (text, range) = apply("hello world", 6..11, Action::Bold);
        assert_eq!(text, "hello **world**");
        assert_eq!(range, 8..13);
        let (text, range) = apply(&text, range, Action::Bold);
        assert_eq!(text, "hello world");
        assert_eq!(range, 6..11);
    }

    #[test]
    fn cursor_action_inserts_a_pair() {
        let (text, range) = apply("abc", 1..1, Action::Underline);
        assert_eq!(text, "a____bc");
        assert_eq!(range, 3..3);
    }
}
