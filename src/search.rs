//! Search query understanding: free text plus filters written in plain words.
//!
//! "идеи прошлого месяца" → kind = Idea, created in the previous calendar month;
//! "что я писал про onboarding" → text "onboarding"; "#pricing" → tag filter.
//! Text terms become FTS5 prefix queries; Russian words lose a common inflection
//! ending first ("онбординга" → "онбординг*"), a cheap stand-in for stemming.

use crate::card::Kind;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    /// Terms as typed (lowercased), for the substring fallback.
    pub words: Vec<String>,
    /// Stemmed terms for FTS prefix matching.
    pub terms: Vec<String>,
    pub tags: Vec<String>,
    pub kind: Option<Kind>,
    /// `[from, to)` in Unix seconds, on `created_at`, with a label for the UI.
    pub range: Option<(i64, i64, String)>,
}

impl Query {
    /// FTS5 MATCH expression, or None when there is nothing to match on.
    pub fn fts_expression(&self) -> Option<String> {
        let quote = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
        let mut parts: Vec<String> = self.terms.iter().map(|t| format!("{}*", quote(t))).collect();
        parts.extend(self.tags.iter().map(|t| format!("tags : {}*", quote(t))));
        (!parts.is_empty()).then(|| parts.join(" AND "))
    }

    /// Understood filters, shown as chips so the user sees how the query was read.
    pub fn chips(&self) -> Vec<String> {
        let mut chips = Vec::new();
        if let Some(kind) = self.kind {
            chips.push(kind.label().to_owned());
        }
        if let Some((_, _, label)) = &self.range {
            chips.push(label.clone());
        }
        chips.extend(self.tags.iter().map(|t| format!("#{t}")));
        chips
    }
}

const STOP_WORDS: &[&str] = &[
    "что", "я", "мы", "мне", "мой", "мои", "моих", "писал", "писала", "писали", "записывал", "записывала", "про",
    "о", "об", "обо", "для", "по", "все", "всё", "где", "как", "это", "было", "были", "найди", "найти", "покажи",
    "заметки", "заметку", "заметок", "заметка", "the", "about", "my", "notes", "note", "find", "show", "за", "на",
    "в", "во", "с", "и", "or", "and", "in", "for", "of",
];

fn kind_word(w: &str) -> Option<Kind> {
    Some(match w {
        "идея" | "идеи" | "идей" | "идею" | "idea" | "ideas" => Kind::Idea,
        "промпт" | "промпты" | "промптов" | "промпта" | "prompt" | "prompts" => Kind::Prompt,
        "ссылка" | "ссылки" | "ссылок" | "ссылку" | "link" | "links" => Kind::Link,
        "цель" | "цели" | "целей" | "goal" | "goals" => Kind::Goal,
        "напоминание" | "напоминания" | "напоминаний" | "reminder" | "reminders" => Kind::Reminder,
        "справка" | "справки" | "reference" | "references" => Kind::Reference,
        "секрет" | "секреты" | "секретов" | "private" | "приватные" => Kind::Private,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Dates (proleptic Gregorian, days since 1970-01-01)
// ---------------------------------------------------------------------------

pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Seconds to add to UTC to get local time, from the current time zone.
pub fn local_offset_secs() -> i64 {
    use windows::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};
    use windows::Win32::System::Time::SystemTimeToFileTime;
    unsafe {
        let (local, utc) = (GetLocalTime(), GetSystemTime());
        let (mut fl, mut fu) = Default::default();
        if SystemTimeToFileTime(&local, &mut fl).is_err() || SystemTimeToFileTime(&utc, &mut fu).is_err() {
            return 0;
        }
        let v = |f: windows::Win32::Foundation::FILETIME| ((f.dwHighDateTime as i64) << 32) | f.dwLowDateTime as i64;
        // Round to the minute: the two calls are a few microseconds apart.
        ((v(fl) - v(fu)) as f64 / 10_000_000.0 / 60.0).round() as i64 * 60
    }
}

/// Recognizes a date phrase at `words[i..]`; returns (words consumed, range in local days, label).
fn date_phrase(words: &[&str], i: usize, today: i64) -> Option<(usize, i64, i64, String)> {
    let w = words[i];
    let next = words.get(i + 1).copied().unwrap_or("");
    let (y, m, _) = civil_from_days(today);
    let month_start = |y: i64, m: i64| days_from_civil(y, m, 1);
    let monday = today - (today + 3).rem_euclid(7);
    let this_or_last = |w: &str| {
        if w.starts_with("эт") || w == "this" {
            Some(false)
        } else if w.starts_with("прошл") || w == "last" {
            Some(true)
        } else {
            None
        }
    };
    let unit = |w: &str| {
        if w.starts_with("недел") || w.starts_with("week") {
            Some('w')
        } else if w.starts_with("месяц") || w.starts_with("month") {
            Some('m')
        } else if w.starts_with("год") || w == "году" || w.starts_with("year") {
            Some('y')
        } else {
            None
        }
    };
    match w {
        "сегодня" | "today" => return Some((1, today, today + 1, "сегодня".into())),
        "вчера" | "yesterday" => return Some((1, today - 1, today, "вчера".into())),
        _ => {}
    }
    if let (Some(last), Some(u)) = (this_or_last(w), unit(next)) {
        let (from, to, label) = match (u, last) {
            ('w', false) => (monday, today + 1, "эта неделя"),
            ('w', true) => (monday - 7, monday, "прошлая неделя"),
            ('m', false) => (month_start(y, m), today + 1, "этот месяц"),
            ('m', true) => {
                let (py, pm) = if m == 1 { (y - 1, 12) } else { (y, m - 1) };
                (month_start(py, pm), month_start(y, m), "прошлый месяц")
            }
            ('y', false) => (days_from_civil(y, 1, 1), today + 1, "этот год"),
            (_, _) => (days_from_civil(y - 1, 1, 1), days_from_civil(y, 1, 1), "прошлый год"),
        };
        return Some((2, from, to, label.into()));
    }
    // "за неделю" / "за месяц": the last 7 / 30 days (the "за" is a stop word).
    if let Some(u) = unit(w).filter(|_| i > 0 && words[i - 1] == "за") {
        let days = match u {
            'w' => 7,
            'm' => 30,
            _ => 365,
        };
        return Some((1, today - days + 1, today + 1, format!("последние {days} дн")));
    }
    if w.len() == 4 && w.chars().all(|c| c.is_ascii_digit()) {
        let year: i64 = w.parse().ok()?;
        if (2000..=2099).contains(&year) {
            return Some((1, days_from_civil(year, 1, 1), days_from_civil(year + 1, 1, 1), w.into()));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Terms
// ---------------------------------------------------------------------------

const RU_ENDINGS: &[&str] = &[
    "ами", "ями", "ого", "его", "ому", "ему", "ыми", "ими", "ах", "ях", "ов", "ев", "ей", "ой", "ий", "ый", "ая",
    "яя", "ое", "ее", "ые", "ие", "ую", "юю", "ом", "ем", "ам", "ям", "а", "я", "о", "е", "ы", "и", "у", "ю", "ь", "й",
];

fn stem(word: &str) -> String {
    let is_cyrillic = word.chars().any(|c| ('а'..='я').contains(&c) || c == 'ё');
    let len = word.chars().count();
    if !is_cyrillic || len < 5 {
        return word.to_owned();
    }
    for ending in RU_ENDINGS {
        if let Some(base) = word.strip_suffix(ending) {
            if base.chars().count() >= 4 {
                return base.to_owned();
            }
        }
    }
    word.to_owned()
}

pub fn parse(input: &str, now: i64, utc_offset: i64) -> Query {
    let lower = input.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '?' | '!' | '"' | '«' | '»' | '(' | ')'))
        .filter(|w| !w.is_empty())
        .collect();
    let today = (now + utc_offset).div_euclid(86_400);

    let mut q = Query::default();
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        if let Some(tag) = w.strip_prefix('#').filter(|t| !t.is_empty()) {
            q.tags.push(tag.to_owned());
            i += 1;
            continue;
        }
        if q.range.is_none() {
            if let Some((n, from, to, label)) = date_phrase(&words, i, today) {
                q.range = Some((from * 86_400 - utc_offset, to * 86_400 - utc_offset, label));
                i += n;
                continue;
            }
        }
        if q.kind.is_none() {
            if let Some(kind) = kind_word(w) {
                q.kind = Some(kind);
                i += 1;
                continue;
            }
        }
        if !STOP_WORDS.contains(&w) {
            // FTS5's unicode61 tokenizer splits on punctuation; do the same so
            // "vpn.staging" becomes two prefix terms instead of an impossible one.
            for part in w.split(|c: char| !c.is_alphanumeric()).filter(|p| !p.is_empty()) {
                q.words.push(part.to_owned());
                q.terms.push(stem(part));
            }
        }
        i += 1;
    }
    q
}

// ---------------------------------------------------------------------------
// Snippets
// ---------------------------------------------------------------------------

pub const MARK_START: char = '\u{1}';
pub const MARK_END: char = '\u{2}';

/// A one-line excerpt of `body` of at most ~`max_chars` characters, starting a
/// little before the first matching word; matches are wrapped in
/// [`MARK_START`]…[`MARK_END`].
pub fn snippet(body: &str, q: &Query, max_chars: usize) -> String {
    // Very long notes: a match past this point shows the beginning instead.
    let body = &body[..body.floor_char_boundary(body.len().min(64_000))];
    let mut flat = String::with_capacity(body.len());
    for c in body.chars() {
        let c = if c.is_whitespace() { ' ' } else { c };
        if !(c == ' ' && flat.ends_with(' ')) {
            flat.push(c);
        }
    }
    let flat = flat.trim();

    let mut spans: Vec<(usize, usize, bool)> = Vec::new();
    let is_match = |w: &str| {
        let lw = w.to_lowercase().replace('ё', "е");
        q.terms.iter().any(|t| lw.starts_with(t.as_str()))
            || q.words.iter().any(|t| t.chars().count() >= 3 && lw.contains(t.as_str()))
    };
    let mut start = None;
    for (i, c) in flat.char_indices().chain([(flat.len(), ' ')]) {
        if c.is_alphanumeric() {
            start.get_or_insert(i);
        } else if let Some(s) = start.take() {
            spans.push((s, i, is_match(&flat[s..i])));
        }
    }

    // Window: from a few words before the first match, up to max_chars.
    let first = spans.iter().position(|s| s.2);
    let mut begin = 0;
    if let Some(m) = first {
        let lead = max_chars / 4;
        let mut k = m;
        while k > 0 && flat[spans[k - 1].0..spans[m].0].chars().count() <= lead {
            k -= 1;
        }
        begin = spans[k].0;
    }
    let mut end = begin;
    let mut used = 0;
    for &(_, e, _) in spans.iter().filter(|s| s.0 >= begin) {
        used += flat[end..e].chars().count();
        if used > max_chars && end > begin {
            break;
        }
        end = e;
    }
    if spans.is_empty() {
        end = flat.len();
    }

    let mut out = String::new();
    if begin > 0 {
        out.push('…');
    }
    let mut pos = begin;
    for &(s, e, _) in spans.iter().filter(|s| s.0 >= begin && s.1 <= end && s.2) {
        out.push_str(&flat[pos..s]);
        out.push(MARK_START);
        out.push_str(&flat[s..e]);
        out.push(MARK_END);
        pos = e;
    }
    out.push_str(&flat[pos..end]);
    if end < flat.len() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_highlights_stems_around_first_match() {
        let q = parse("онбординга", NOW, MSK);
        let body = format!("{} Попробовать Онбординг без\nрегистрации, онбординга хватит", "слово ".repeat(40));
        let s = snippet(&body, &q, 60);
        assert!(s.starts_with('…'), "{s}");
        assert!(s.contains("\u{1}Онбординг\u{2}"), "{s}");
        assert!(s.chars().filter(|c| *c != MARK_START && *c != MARK_END).count() <= 64, "{s}");
        assert_eq!(snippet("short text", &parse("zzz", NOW, MSK), 60), "short text");
    }

    // 2026-09-13 12:00 UTC, a Sunday; Moscow time (+3 h).
    const NOW: i64 = 1_789_300_800;
    const MSK: i64 = 3 * 3600;

    fn day(y: i64, m: i64, d: i64) -> i64 {
        days_from_civil(y, m, d) * 86_400 - MSK
    }

    #[test]
    fn civil_round_trip() {
        for z in [-1000, 0, 19_000, 20_709, 60_000] {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
        }
        assert_eq!(civil_from_days(NOW / 86_400), (2026, 9, 13));
    }

    #[test]
    fn plain_text_and_stop_words() {
        let q = parse("что я писал про onboarding", NOW, MSK);
        assert_eq!(q.terms, ["onboarding"]);
        assert!(q.kind.is_none() && q.range.is_none());
        assert_eq!(q.fts_expression().unwrap(), "\"onboarding\"*");
    }

    #[test]
    fn kind_and_stemming() {
        let q = parse("prompts для анализа", NOW, MSK);
        assert_eq!(q.kind, Some(Kind::Prompt));
        assert_eq!(q.terms, ["анализ"]);
        assert_eq!(parse("онбординга", NOW, MSK).terms, ["онбординг"]);
        assert_eq!(parse("vpn", NOW, MSK).terms, ["vpn"]);
    }

    #[test]
    fn previous_month() {
        let q = parse("идеи прошлого месяца", NOW, MSK);
        assert_eq!(q.kind, Some(Kind::Idea));
        assert!(q.terms.is_empty());
        let (from, to, _) = q.range.unwrap();
        assert_eq!((from, to), (day(2026, 8, 1), day(2026, 9, 1)));
    }

    #[test]
    fn weeks_days_years() {
        // Sunday 2026-09-13: this week started Monday 2026-09-07.
        let (from, to, _) = parse("на этой неделе", NOW, MSK).range.unwrap();
        assert_eq!((from, to), (day(2026, 9, 7), day(2026, 9, 14)));
        let (from, to, _) = parse("прошлой неделе", NOW, MSK).range.unwrap();
        assert_eq!((from, to), (day(2026, 8, 31), day(2026, 9, 7)));
        let (from, _, _) = parse("вчера", NOW, MSK).range.unwrap();
        assert_eq!(from, day(2026, 9, 12));
        let (from, to, _) = parse("pricing за месяц", NOW, MSK).range.unwrap();
        assert_eq!((from, to), (day(2026, 8, 15), day(2026, 9, 14)));
        let q = parse("vpn 2024", NOW, MSK);
        assert_eq!(q.range.unwrap().0, day(2024, 1, 1));
        assert_eq!(q.terms, ["vpn"]);
    }

    #[test]
    fn tags_and_punctuation() {
        let q = parse("#pricing vpn.staging \"quoted\"", NOW, MSK);
        assert_eq!(q.tags, ["pricing"]);
        assert_eq!(q.terms, ["vpn", "staging", "quoted"]);
        assert_eq!(
            q.fts_expression().unwrap(),
            "\"vpn\"* AND \"staging\"* AND \"quoted\"* AND tags : \"pricing\"*"
        );
    }
}
