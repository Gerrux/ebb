//! UI strings in the active language.
//!
//! Every language is one complete table: [`Strings`] and its per-area parts, each with a
//! `RU` and an `EN` constant. A missing translation is a compile error, a lookup is a
//! pointer read: no files, no parsing, no allocation before the first frame. Parameterized
//! phrases (counts, plurals, dates) are plain `fn` fields, since word order and plural
//! rules differ between languages.
//!
//! The active language is one process-wide atomic: windows on other threads (bar, tray
//! menu) read it too, and everything drawn per frame follows a change at once. It starts
//! as Russian, so tests that don't set it see the strings Ebb always had; `main` sets it
//! from the Windows UI language, then the app from the `app.language` setting. Tests of English call
//! [`strings`]`(Lang::En)` directly instead of flipping the global.
//!
//! Only what Ebb shows is here. Words Ebb *reads* (search filters, `идея:` prefixes,
//! "через неделю") stay where they are parsed: those accept both languages at once.

use std::sync::atomic::{AtomicU8, Ordering};

pub mod bar;
pub mod common;
pub mod layer;
pub mod library;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Ru,
    En,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::Ru, Lang::En];

    pub fn key(self) -> &'static str {
        match self {
            Lang::Ru => "ru",
            Lang::En => "en",
        }
    }

    /// `None`: follow Windows (`system`, absent or unknown).
    pub fn from_key(s: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.key() == s)
    }

    /// The language's own name, the same in every UI language.
    pub fn name(self) -> &'static str {
        match self {
            Lang::Ru => "Русский",
            Lang::En => "English",
        }
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn lang() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        1 => Lang::En,
        _ => Lang::Ru,
    }
}

pub fn set(lang: Lang) {
    LANG.store(lang as u8, Ordering::Relaxed);
}

/// The language a choice from Settings means on this machine.
pub fn resolve(choice: Option<Lang>) -> Lang {
    choice.unwrap_or_else(system)
}

/// Russian when Windows shows its UI in Russian, English otherwise.
pub fn system() -> Lang {
    // SAFETY: no arguments; only reads the user's UI language.
    let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    from_langid(id)
}

/// LANGID → language: the primary language is the low 10 bits, `LANG_RUSSIAN` is 0x19.
fn from_langid(id: u16) -> Lang {
    if id & 0x3FF == 0x19 { Lang::Ru } else { Lang::En }
}

pub struct Strings {
    pub common: common::Common,
    pub layer: layer::Layer,
    pub bar: bar::Bar,
    pub library: library::Library,
}

static RU: Strings = Strings { common: common::RU, layer: layer::RU, bar: bar::RU, library: library::RU };
static EN: Strings = Strings { common: common::EN, layer: layer::EN, bar: bar::EN, library: library::EN };

/// The table of the active language.
pub fn s() -> &'static Strings {
    strings(lang())
}

pub fn strings(lang: Lang) -> &'static Strings {
    match lang {
        Lang::Ru => &RU,
        Lang::En => &EN,
    }
}

/// The name of `v` from `names`, which lists them in the order of `all` (an enum's `ALL`).
pub(crate) fn name_of<T: PartialEq + Copy, const N: usize>(all: &[T; N], v: T, names: &[&'static str; N]) -> &'static str {
    names[all.iter().position(|a| *a == v).unwrap_or(0)]
}

/// Russian plural form: `one` for 1, 21, 31…; `few` for 2–4, 22–24…; `many` for the rest
/// (including 11–14).
pub(crate) fn ru_plural(n: i64, one: &'static str, few: &'static str, many: &'static str) -> &'static str {
    let n = n.unsigned_abs();
    match (n % 10, n % 100) {
        (_, 11..=14) => many,
        (1, _) => one,
        (2..=4, _) => few,
        _ => many,
    }
}

/// English plural form: `one` only for exactly 1.
pub(crate) fn en_plural(n: i64, one: &'static str, other: &'static str) -> &'static str {
    if n == 1 { one } else { other }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn langid_primary_language() {
        assert_eq!(from_langid(0x0419), Lang::Ru); // ru-RU
        assert_eq!(from_langid(0x0819), Lang::Ru); // ru-MD
        assert_eq!(from_langid(0x0409), Lang::En); // en-US
        assert_eq!(from_langid(0x0422), Lang::En); // uk-UA
        assert_eq!(from_langid(0x0407), Lang::En); // de-DE
    }

    #[test]
    fn keys_round_trip() {
        for l in Lang::ALL {
            assert_eq!(Lang::from_key(l.key()), Some(l));
        }
        assert_eq!(Lang::from_key("system"), None);
        assert_eq!(Lang::from_key(""), None);
    }

    #[test]
    fn plurals() {
        let ru = |n| ru_plural(n, "день", "дня", "дней");
        assert_eq!([ru(1), ru(2), ru(5), ru(11), ru(12), ru(21), ru(23), ru(111), ru(0)], [
            "день", "дня", "дней", "дней", "дней", "день", "дня", "дней", "дней"
        ]);
        assert_eq!([en_plural(1, "day", "days"), en_plural(0, "day", "days"), en_plural(21, "day", "days")], [
            "day", "days", "days"
        ]);
    }
}
