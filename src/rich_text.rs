//! Small inline markup used by note bodies.
//!
//! Notes are stored with the markup in their text (so old databases need no
//! schema migration). The editor never shows it: it edits the plain text with a
//! style per character, the way Sticky Notes does, and writes the markup back.

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

impl Style {
    pub fn has(self, action: Action) -> bool {
        match action {
            Action::Bold => self.bold,
            Action::Italic => self.italic,
            Action::Underline => self.underline,
            Action::Strikethrough => self.strikethrough,
            Action::Highlight => self.highlight,
            Action::Code => self.code,
            Action::Clear => self == Style::default(),
        }
    }

    /// With the style flipped; Clear gives no style at all.
    pub fn toggled(mut self, action: Action) -> Style {
        let on = action != Action::Clear && !self.has(action);
        self.set(action, on);
        self
    }

    fn set(&mut self, action: Action, on: bool) {
        match action {
            Action::Bold => self.bold = on,
            Action::Italic => self.italic = on,
            Action::Underline => self.underline = on,
            Action::Strikethrough => self.strikethrough = on,
            Action::Highlight => self.highlight = on,
            Action::Code => self.code = on,
            Action::Clear => *self = Style::default(),
        }
    }
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

/// Markers in the order they're matched: a double marker before a single one.
const MARKERS: [(&str, Action); 6] = [
    ("**", Action::Bold),
    ("__", Action::Underline),
    ("~~", Action::Strikethrough),
    ("==", Action::Highlight),
    ("`", Action::Code),
    ("*", Action::Italic),
];

/// Characters a backslash makes literal.
fn escapable(c: char) -> bool {
    matches!(c, '*' | '_' | '~' | '=' | '`')
}

/// An escape at the start of `rest`: the literal char and the bytes it takes.
/// `\*` is a literal `*`; `\\` is a literal backslash only before another
/// backslash or a marker char, so `\\server\share` written by hand stays as is.
fn escape_at(rest: &str) -> Option<(char, usize)> {
    let mut chars = rest.chars();
    if chars.next()? != '\\' {
        return None;
    }
    match chars.next()? {
        c if escapable(c) => Some((c, 2)),
        '\\' if chars.next().is_some_and(|c| c == '\\' || escapable(c)) => Some(('\\', 2)),
        _ => None,
    }
}

/// The plain text and a style for each of its chars.
pub fn parse(text: &str) -> (String, Vec<Style>) {
    let mut plain = String::with_capacity(text.len());
    let mut styles = Vec::with_capacity(text.len());
    let mut style = Style::default();
    let mut cursor = 0;
    while cursor < text.len() {
        let rest = &text[cursor..];
        let first = rest.chars().next().expect("cursor is inside the text");
        if let Some((c, len)) = escape_at(rest) {
            plain.push(c);
            styles.push(style);
            cursor += len;
            continue;
        }
        if let Some((marker, action)) = MARKERS.iter().find(|(m, _)| rest.starts_with(m)) {
            // A marker without a closing partner is ordinary text. This keeps
            // prose such as "2 * 3" and an unfinished edit visible.
            if style.has(*action) || has_closing(&rest[marker.len()..], marker) {
                style.set(*action, !style.has(*action));
                cursor += marker.len();
                continue;
            }
            for c in marker.chars() {
                plain.push(c);
                styles.push(style);
            }
            cursor += marker.len();
            continue;
        }
        plain.push(first);
        styles.push(style);
        cursor += first.len_utf8();
    }
    (plain, styles)
}

/// Whether `marker` comes later in `rest`, outside escapes.
fn has_closing(rest: &str, marker: &str) -> bool {
    let mut at = 0;
    while at < rest.len() {
        if let Some((_, len)) = escape_at(&rest[at..]) {
            at += len;
        } else if rest[at..].starts_with(marker) {
            return true;
        } else {
            at += rest[at..].chars().next().map_or(1, char::len_utf8);
        }
    }
    false
}

/// Parses the supported markers and returns the visible text in styled runs.
pub fn spans(text: &str) -> Vec<Span> {
    let (plain, styles) = parse(text);
    let mut result: Vec<Span> = Vec::new();
    for (c, style) in plain.chars().zip(styles) {
        match result.last_mut() {
            Some(span) if span.style == style => span.text.push(c),
            _ => result.push(Span { text: c.to_string(), style }),
        }
    }
    result
}

pub fn strip_markup(text: &str) -> String {
    parse(text).0
}

enum Token {
    Mark(&'static str),
    Lit(char),
}

/// The markup for plain text with a style per char; `parse` gives them back.
pub fn serialize(plain: &str, styles: &[Style]) -> String {
    let mut tokens: Vec<Token> = Vec::with_capacity(plain.len() + 8);
    let mut current = Style::default();
    let mut switch = |tokens: &mut Vec<Token>, style: Style| {
        for (marker, action) in MARKERS {
            if style.has(action) != current.has(action) {
                tokens.push(Token::Mark(marker));
            }
        }
        current = style;
    };
    for (i, c) in plain.chars().enumerate() {
        switch(&mut tokens, styles.get(i).copied().unwrap_or_default());
        tokens.push(Token::Lit(c));
    }
    switch(&mut tokens, Style::default());

    // Written back to front, so each char knows what follows it in the output.
    let mut reversed: Vec<char> = Vec::with_capacity(plain.len() + 16);
    let (mut star_later, mut tick_later) = (false, false);
    for (i, token) in tokens.iter().enumerate().rev() {
        let next = reversed.last().copied();
        match *token {
            Token::Mark(m) => reversed.extend(m.chars().rev()),
            Token::Lit(c) => {
                let escape = match c {
                    // A lone '*' or '`' is literal only while none follows, and
                    // a '*' right after an italic marker would make it bold.
                    '*' => star_later || matches!(i.checked_sub(1).map(|p| &tokens[p]), Some(Token::Mark(m)) if m.ends_with('*')),
                    '`' => tick_later,
                    '_' | '~' | '=' => next == Some(c),
                    // A backslash before what an escape would take is doubled.
                    '\\' => next.is_some_and(|n| n == '\\' || escapable(n)),
                    _ => false,
                };
                reversed.push(c);
                if escape {
                    reversed.push('\\');
                }
            }
        }
        let (star, tick) = match *token {
            Token::Mark(m) => (m.contains('*'), m.contains('`')),
            Token::Lit(c) => (c == '*', c == '`'),
        };
        star_later |= star;
        tick_later |= tick;
    }
    reversed.into_iter().rev().collect()
}

/// The markup with whitespace trimmed off the ends of its visible text.
pub fn trim(text: &str) -> String {
    let (plain, styles) = parse(text);
    let start = plain.chars().take_while(|c| c.is_whitespace()).count();
    let len = plain.chars().count();
    let end = len - plain.chars().rev().take_while(|c| c.is_whitespace()).count();
    if start >= end {
        return String::new();
    }
    let trimmed: String = plain.chars().skip(start).take(end - start).collect();
    serialize(&trimmed, &styles[start..end])
}

/// Turns a style on across `range` (chars), or off when all of it has it already.
/// Clear takes every style off.
pub fn toggle(styles: &mut [Style], range: Range<usize>, action: Action) {
    let range = range.start.min(styles.len())..range.end.min(styles.len());
    let on = action != Action::Clear && !styles[range.clone()].iter().all(|s| s.has(action));
    for style in &mut styles[range] {
        style.set(action, on);
    }
}

/// Whether the whole range has the style: what the toolbar shows as pressed.
pub fn all_have(styles: &[Style], range: Range<usize>, action: Action) -> bool {
    let range = range.start.min(styles.len())..range.end.min(styles.len());
    !range.is_empty() && styles[range].iter().all(|s| s.has(action))
}

/// The style typing gets at char `at`: the char before it, else the one after.
pub fn style_at(styles: &[Style], at: usize) -> Style {
    at.checked_sub(1)
        .and_then(|i| styles.get(i))
        .or_else(|| styles.get(at))
        .copied()
        .unwrap_or_default()
}

/// Styles for `new`, an edit of `old`: kept chars keep theirs, inserted chars
/// get `typing` or the style where they went in. `cursor` (chars, after the
/// edit) places an insertion among repeated chars.
pub fn restyle(old: &str, styles: &[Style], new: &str, cursor: Option<usize>, typing: Option<Style>) -> Vec<Style> {
    if old == new {
        return styles.to_vec();
    }
    let old: Vec<char> = old.chars().collect();
    let new: Vec<char> = new.chars().collect();
    let max_suffix = cursor.map_or(new.len(), |c| new.len().saturating_sub(c)).min(old.len()).min(new.len());
    let suffix = old.iter().rev().zip(new.iter().rev()).take(max_suffix).take_while(|(a, b)| a == b).count();
    let max_prefix = old.len().min(new.len()) - suffix;
    let prefix = old.iter().zip(&new).take(max_prefix).take_while(|(a, b)| a == b).count();
    let inserted = new.len() - prefix - suffix;
    let style = typing.unwrap_or_else(|| style_at(styles, prefix));
    let mut result = Vec::with_capacity(new.len());
    result.extend_from_slice(&styles[..prefix.min(styles.len())]);
    result.extend(std::iter::repeat_n(style, inserted));
    result.extend_from_slice(&styles[styles.len().saturating_sub(suffix)..]);
    result.resize(new.len(), Style::default());
    result
}

#[cfg(test)]
mod tests {
    use super::{Action, Style, parse, restyle, serialize, spans, strip_markup, toggle, trim};

    const BOLD: Style = Style { bold: true, italic: false, underline: false, strikethrough: false, highlight: false, code: false };

    #[test]
    fn parses_nested_styles_without_markers() {
        let parsed = spans("a **bold *and italic*** b");
        assert_eq!(parsed[0].text, "a ");
        assert_eq!(parsed[1], super::Span { text: "bold ".into(), style: BOLD });
        assert_eq!(parsed[2].style, Style { bold: true, italic: true, ..Style::default() });
        assert_eq!(strip_markup("a **bold**"), "a bold");
        assert_eq!(strip_markup("2 * 3"), "2 * 3");
    }

    #[test]
    fn toggles_a_selection_and_toggles_it_back() {
        let (plain, mut styles) = parse("hello world");
        toggle(&mut styles, 6..11, Action::Bold);
        assert_eq!(serialize(&plain, &styles), "hello **world**");
        toggle(&mut styles, 6..11, Action::Bold);
        assert_eq!(serialize(&plain, &styles), "hello world");
        // Part of it bold: the whole range becomes bold.
        toggle(&mut styles, 6..8, Action::Bold);
        toggle(&mut styles, 0..11, Action::Bold);
        assert_eq!(serialize(&plain, &styles), "**hello world**");
    }

    #[test]
    fn literal_marker_chars_survive_a_round_trip() {
        for text in ["a * b * c", "snake_case", "x__y", "a == b", "`tick`", r"C:\Users\*", r"\\server\share", "~~~"] {
            let styles = vec![Style::default(); text.chars().count()];
            let markup = serialize(text, &styles);
            assert_eq!(parse(&markup), (text.to_owned(), styles), "{text:?} -> {markup:?}");
        }
        assert_eq!(serialize("snake_case", &[Style::default(); 10]), "snake_case");
    }

    #[test]
    fn random_texts_and_styles_round_trip() {
        let alphabet: Vec<char> = r"ab *_~=`\".chars().chain(['\n', 'я']).collect();
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for _ in 0..20_000 {
            let len = next(12);
            let text: String = (0..len).map(|_| alphabet[next(alphabet.len())]).collect();
            let styles: Vec<Style> = (0..len)
                .map(|_| {
                    let bits = next(64);
                    Style {
                        bold: bits & 1 != 0,
                        italic: bits & 2 != 0,
                        underline: bits & 4 != 0,
                        strikethrough: bits & 8 != 0,
                        highlight: bits & 16 != 0,
                        code: bits & 32 != 0,
                    }
                })
                .collect();
            let markup = serialize(&text, &styles);
            assert_eq!(parse(&markup), (text.clone(), styles), "{text:?} -> {markup:?}");
        }
    }

    #[test]
    fn typing_takes_the_style_before_the_cursor() {
        let (plain, styles) = parse("**ab**c");
        let styles2 = restyle(&plain, &styles, "abXc", Some(3), None);
        assert_eq!(serialize("abXc", &styles2), "**abX**c");
        // A style switched on at the cursor applies to what's typed there.
        let styles3 = restyle(&plain, &styles, "abcY", Some(4), Some(BOLD));
        assert_eq!(serialize("abcY", &styles3), "**ab**c**Y**");
        // Repeated chars: the cursor says which one was inserted.
        let (plain, styles) = parse("a**a**");
        let styles4 = restyle(&plain, &styles, "aaa", Some(1), Some(Style::default()));
        assert_eq!(serialize("aaa", &styles4), "aa**a**");
        // Deleting keeps the rest.
        let styles5 = restyle(&plain, &styles, "a", Some(1), None);
        assert_eq!(serialize("a", &styles5), "a");
    }

    #[test]
    fn trims_the_visible_text() {
        assert_eq!(trim("  **bold  **\n"), "**bold**");
        assert_eq!(trim(" ** ** "), "");
    }
}
