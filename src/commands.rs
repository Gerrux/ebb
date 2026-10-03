//! The command palette's data: which commands exist, when they apply, how a
//! typed query finds them, and the list of recently used ones. Pure logic, no
//! egui widgets and no Win32 calls; drawing is `bar`'s job, running a command
//! is the root viewport's (`app`).
//!
//! Matching is a cheap fuzzy match over the title and synonyms (Russian and
//! English): a substring beats a subsequence, a word start beats the middle of
//! a word, a whole-word match beats a prefix. The registry is a few dozen
//! entries, so every keystroke may scan all of it.

/// How many recently used commands lead the list for an empty query.
pub const RECENT_MAX: usize = 5;

/// The layer's dimming (`layer.tint`) is an alpha 0..=TINT_MAX; the palette
/// speaks percent.
pub const TINT_MAX: u8 = 220;

macro_rules! command_ids {
    ($($variant:ident => $key:literal),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum CommandId {
            $($variant),*
        }

        impl CommandId {
            pub const ALL: &'static [CommandId] = &[$(CommandId::$variant),*];

            /// Stable name for the `palette.recent` setting; never renamed.
            pub fn key(self) -> &'static str {
                match self {
                    $(CommandId::$variant => $key),*
                }
            }
        }
    };
}

command_ids! {
    NewNote => "new_note",
    Search => "search",
    ShowIdeas => "show_ideas",
    ShowPrompts => "show_prompts",
    ShowGoals => "show_goals",
    ShowReminders => "show_reminders",
    ShowLinks => "show_links",
    OpenReview => "open_review",
    OpenArchive => "open_archive",
    OpenTrash => "open_trash",
    OpenSettings => "open_settings",
    ToggleLayer => "toggle_layer",
    TogglePinBottom => "toggle_pin_bottom",
    ToggleCurtain => "toggle_curtain",
    MoveToMonitor => "move_to_monitor",
    Dimming => "dimming",
    NextBackdrop => "next_backdrop",
    ImportSticky => "import_sticky",
    ToggleAutostart => "toggle_autostart",
    BackupNow => "backup_now",
    Quit => "quit",
    ArchiveCard => "archive_card",
    TogglePinCard => "toggle_pin_card",
}

impl CommandId {
    pub fn from_key(key: &str) -> Option<CommandId> {
        Self::ALL.iter().copied().find(|id| id.key() == key)
    }

    /// Where the command takes the bar itself, instead of the layer: the bar
    /// switches in place and stays open.
    pub fn bar_target(self) -> Option<BarTarget> {
        Some(match self {
            CommandId::NewNote => BarTarget::Capture,
            CommandId::Search => BarTarget::Search(""),
            // Type filters are plain words in a query (see `search`).
            CommandId::ShowIdeas => BarTarget::Search("идеи "),
            CommandId::ShowPrompts => BarTarget::Search("промпты "),
            CommandId::ShowGoals => BarTarget::Search("цели "),
            CommandId::ShowReminders => BarTarget::Search("напоминания "),
            CommandId::ShowLinks => BarTarget::Search("ссылки "),
            _ => return None,
        })
    }
}

/// A mode of the bar a command can open, with the search text it starts with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarTarget {
    Capture,
    Search(&'static str),
}

/// What a command needs typed after its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arg {
    None,
    /// A whole number in `min..=max`; `hint` tells the user what it is.
    Number { hint: &'static str, min: i64, max: i64 },
}

pub struct Command {
    pub id: CommandId,
    /// Russian name, without the current state.
    pub title: &'static str,
    /// Other words the command is found by, Russian and English.
    pub synonyms: &'static [&'static str],
    pub arg: Arg,
}

/// Upper bound the registry gives the monitor number; the real one is the
/// number of connected monitors (see [`search`]).
const MONITORS_MAX: i64 = 16;

static COMMANDS: [Command; 23] = [
    Command {
        id: CommandId::NewNote,
        title: "Новая заметка",
        synonyms: &["записать мысль", "захват", "добавить", "создать", "capture", "new note"],
        arg: Arg::None,
    },
    Command {
        id: CommandId::Search,
        title: "Поиск заметок",
        synonyms: &["найти", "искать", "find", "search"],
        arg: Arg::None,
    },
    Command {
        id: CommandId::ShowIdeas,
        title: "Показать идеи",
        synonyms: &["идеи", "ideas"],
        arg: Arg::None,
    },
    Command {
        id: CommandId::ShowPrompts,
        title: "Показать промпты",
        synonyms: &["промпты", "prompts"],
        arg: Arg::None,
    },
    Command {
        id: CommandId::ShowGoals,
        title: "Показать цели",
        synonyms: &["цели", "goals"],
        arg: Arg::None,
    },
    Command {
        id: CommandId::ShowReminders,
        title: "Показать напоминания",
        synonyms: &["напоминания", "напомнить", "reminders"],
        arg: Arg::None,
    },
    Command {
        id: CommandId::ShowLinks,
        title: "Показать ссылки",
        synonyms: &["ссылки", "links"],
        arg: Arg::None,
    },
    Command {
        id: CommandId::OpenReview,
        title: "Еженедельный обзор",
        synonyms: &["обзор", "неделя", "review", "weekly", "weekly review"],
        arg: Arg::None,
    },
    Command {
        id: CommandId::OpenArchive,
        title: "Открыть архив",
        synonyms: &["архив", "archive", "библиотека", "library"],
        arg: Arg::None,
    },
    Command {
        id: CommandId::OpenTrash,
        title: "Открыть корзину",
        synonyms: &["корзина", "удалённые", "удаленные", "trash", "bin"],
        arg: Arg::None,
    },
    Command {
        id: CommandId::OpenSettings,
        title: "Настройки",
        synonyms: &["параметры", "settings", "preferences", "options"],
        arg: Arg::None,
    },
    Command {
        id: CommandId::ToggleLayer,
        title: "Слой",
        synonyms: &["показать", "скрыть", "layer", "show", "hide"],
        arg: Arg::None,
    },
    Command {
        id: CommandId::TogglePinBottom,
        title: "Слой под окнами",
        synonyms: &["под окнами", "рабочий стол", "bottom", "desktop", "pin"],
        arg: Arg::None,
    },
    Command {
        id: CommandId::ToggleCurtain,
        title: "Свернуть слой",
        synonyms: &["свернуть", "развернуть", "штора", "collapse", "expand", "curtain"],
        arg: Arg::None,
    },
    Command {
        id: CommandId::MoveToMonitor,
        title: "Переместить слой на монитор",
        synonyms: &["монитор", "экран", "monitor", "display", "screen"],
        arg: Arg::Number { hint: "номер монитора", min: 1, max: MONITORS_MAX },
    },
    Command {
        id: CommandId::Dimming,
        title: "Затемнение слоя",
        synonyms: &["затемнение", "прозрачность", "тень", "dim", "tint", "opacity", "transparency"],
        arg: Arg::Number { hint: "0–100", min: 0, max: 100 },
    },
    Command {
        id: CommandId::NextBackdrop,
        title: "Фон слоя: следующий",
        synonyms: &["фон", "акрил", "размытие", "backdrop", "acrylic", "blur"],
        arg: Arg::None,
    },
    Command {
        id: CommandId::ImportSticky,
        title: "Импорт из Sticky Notes",
        synonyms: &["импорт", "стикеры", "sticky", "import"],
        arg: Arg::None,
    },
    Command {
        id: CommandId::ToggleAutostart,
        title: "Запускать при входе",
        synonyms: &["автозапуск", "запуск", "autostart", "startup", "logon", "login"],
        arg: Arg::None,
    },
    Command {
        id: CommandId::BackupNow,
        title: "Сделать резервную копию",
        synonyms: &["бэкап", "копия", "backup"],
        arg: Arg::None,
    },
    Command {
        id: CommandId::Quit,
        title: "Выйти из Ebb",
        synonyms: &["выход", "закрыть", "quit", "exit"],
        arg: Arg::None,
    },
    Command {
        id: CommandId::ArchiveCard,
        title: "Выбранная карточка: убрать в архив",
        synonyms: &["убрать", "сделано", "done"],
        arg: Arg::None,
    },
    Command {
        id: CommandId::TogglePinCard,
        title: "Выбранная карточка: закрепить",
        synonyms: &["закрепить", "открепить", "pin", "unpin"],
        arg: Arg::None,
    },
];

pub fn command(id: CommandId) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.id == id)
}

/// What the app looks like right now; decides which commands apply and how
/// their titles read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ctx {
    /// A card is selected on the layer.
    pub has_active_card: bool,
    pub active_pinned: bool,
    pub layer_visible: bool,
    /// The layer is rolled up into its tab.
    pub collapsed: bool,
    pub pin_bottom: bool,
    /// Whether Ebb runs at logon; `None` when not known without asking COM.
    pub autostart: Option<bool>,
    pub monitors: usize,
    /// The layer's dimming as stored (alpha, see [`TINT_MAX`]).
    pub tint: u8,
}

pub fn tint_from_percent(percent: i64) -> u8 {
    ((percent.clamp(0, 100) * i64::from(TINT_MAX) + 50) / 100) as u8
}

pub fn percent_of_tint(tint: u8) -> i64 {
    (i64::from(tint.min(TINT_MAX)) * 100 + i64::from(TINT_MAX) / 2) / i64::from(TINT_MAX)
}

/// Whether the command makes sense now; the others are hidden.
pub fn available(id: CommandId, ctx: &Ctx) -> bool {
    match id {
        CommandId::ArchiveCard | CommandId::TogglePinCard => ctx.has_active_card,
        // Rolling a hidden layer up is meaningless.
        CommandId::ToggleCurtain => ctx.layer_visible,
        // With one monitor there is nowhere to go.
        CommandId::MoveToMonitor => ctx.monitors >= 2,
        _ => true,
    }
}

/// The command's name with its current state, as the list shows it.
pub fn title(id: CommandId, ctx: &Ctx) -> String {
    let base = command(id).map_or("", |c| c.title);
    let on_off = |on: bool| if on { "выключить" } else { "включить" };
    match id {
        // A rolled-up layer is brought back, not hidden (see `toggle_layer`).
        CommandId::ToggleLayer => {
            (if ctx.layer_visible && !ctx.collapsed { "Скрыть слой" } else { "Показать слой" }).to_owned()
        }
        CommandId::TogglePinBottom => format!("{base}: {}", on_off(ctx.pin_bottom)),
        CommandId::ToggleCurtain => (if ctx.collapsed { "Развернуть слой" } else { "Свернуть слой" }).to_owned(),
        CommandId::Dimming => format!("{base}: {}%", percent_of_tint(ctx.tint)),
        CommandId::ToggleAutostart => match ctx.autostart {
            Some(on) => format!("{base}: {}", on_off(on)),
            None => format!("{base}: переключить"),
        },
        CommandId::TogglePinCard => {
            (if ctx.active_pinned { "Выбранная карточка: открепить" } else { base }).to_owned()
        }
        _ => base.to_owned(),
    }
}

/// One line of the palette's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub id: CommandId,
    /// With the current state, e.g. "Слой под окнами: выключить".
    pub title: String,
    /// The number typed after the name, when it is valid for the command.
    pub arg: Option<i64>,
    /// The command takes a number and none valid was given: Enter asks for it
    /// instead of running.
    pub needs_arg: bool,
}

/// Lowercase with "ё" as "е", as chars.
fn fold(text: &str) -> Vec<char> {
    text.chars().flat_map(char::to_lowercase).map(|c| if c == 'ё' { 'е' } else { c }).collect()
}

fn word_start(hay: &[char], at: usize) -> bool {
    at == 0 || !hay[at - 1].is_alphanumeric()
}

/// How well `needle` matches `hay` (both folded); `None` when it doesn't.
fn score(needle: &[char], hay: &[char]) -> Option<i32> {
    if needle.is_empty() {
        return Some(0);
    }
    let n = needle.len();
    if n > hay.len() {
        return None;
    }
    let mut best = None;
    for at in 0..=hay.len() - n {
        if hay[at..at + n] != *needle {
            continue;
        }
        let found = match (at, word_start(hay, at)) {
            (0, _) if n == hay.len() => 120,
            (0, _) => 100,
            (_, true) => 80,
            // A single letter in the middle of a word is noise.
            _ if n == 1 => continue,
            _ => 60,
        };
        best = best.max(Some(found));
    }
    if best.is_some() || n < 2 {
        return best;
    }
    // A subsequence: every letter in order, with a bonus for word starts and
    // for letters that follow each other.
    let (mut points, mut from) = (10, 0);
    let mut previous = None;
    for c in needle {
        let at = from + hay[from..].iter().position(|h| h == c)?;
        if word_start(hay, at) {
            points += 4;
        }
        if previous == Some(at.wrapping_sub(1)) {
            points += 3;
        }
        previous = Some(at);
        from = at + 1;
    }
    Some(points.min(49))
}

/// The query split into its text and a trailing whole number.
fn split_number(query: &str) -> (&str, Option<i64>) {
    let query = query.trim_end();
    let (text, last) = match query.rsplit_once(char::is_whitespace) {
        Some((text, last)) => (text.trim_end(), last),
        None => ("", query),
    };
    let (negative, digits) = match last.strip_prefix('-') {
        Some(digits) => (true, digits),
        None => (false, last),
    };
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        // Too long for i64 is still a number, just out of every range.
        let n = last.parse().unwrap_or(if negative { i64::MIN } else { i64::MAX });
        return (text, Some(n));
    }
    (query, None)
}

/// The commands `query` finds, best first. `query` is what follows the `>`.
/// An empty query lists the recent commands (most recent first), then the rest.
pub fn search(query: &str, ctx: &Ctx, recent: &[CommandId]) -> Vec<Match> {
    let query = query.trim_start();
    let shown = |id: CommandId, arg: Option<i64>, needs_arg: bool| Match { id, title: title(id, ctx), arg, needs_arg };

    if query.trim().is_empty() {
        let mut ids: Vec<CommandId> = recent.iter().copied().filter(|id| available(*id, ctx)).take(RECENT_MAX).collect();
        let lead = ids.len();
        let rest: Vec<CommandId> =
            COMMANDS.iter().map(|c| c.id).filter(|id| available(*id, ctx) && !ids[..lead].contains(id)).collect();
        ids.extend(rest);
        return ids.into_iter().map(|id| shown(id, None, command(id).is_some_and(|c| c.arg != Arg::None))).collect();
    }

    let mut found: Vec<(i32, Match)> = Vec::new();
    for c in COMMANDS.iter().filter(|c| available(c.id, ctx)) {
        // A trailing number is the argument of a command that takes one; for
        // the others it's just part of the text (and matches nothing).
        let (text, number) = match c.arg {
            Arg::Number { .. } => split_number(query),
            Arg::None => (query, None),
        };
        let needle = fold(text);
        let from_title = score(&needle, &fold(&title(c.id, ctx))).map(|s| s + 4);
        let from_synonym = c.synonyms.iter().filter_map(|s| score(&needle, &fold(s))).max();
        let Some(best) = from_title.max(from_synonym) else { continue };
        let (arg, needs_arg) = match c.arg {
            Arg::None => (None, false),
            Arg::Number { min, max, .. } => {
                // Monitors: as many as are connected.
                let max = if c.id == CommandId::MoveToMonitor { ctx.monitors as i64 } else { max };
                // Out of range is not clamped: "затемнение 400" is a typo, not 100.
                let valid = number.filter(|n| (min..=max).contains(n));
                (valid, valid.is_none())
            }
        };
        found.push((best, shown(c.id, arg, needs_arg)));
    }
    // Stable: equal scores stay in registry order.
    found.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    found.into_iter().map(|(_, m)| m).collect()
}

/// Moves `id` to the front of the recent list; at most [`RECENT_MAX`] stay.
pub fn push_recent(recent: &mut Vec<CommandId>, id: CommandId) {
    recent.retain(|r| *r != id);
    recent.insert(0, id);
    recent.truncate(RECENT_MAX);
}

/// The `palette.recent` setting: ids, most recent first, separated by commas.
pub fn encode_recent(recent: &[CommandId]) -> String {
    recent.iter().map(|id| id.key()).collect::<Vec<_>>().join(",")
}

/// Reads the setting; ids that no longer exist are dropped.
pub fn decode_recent(text: &str) -> Vec<CommandId> {
    let mut recent = Vec::new();
    for id in text.split(',').filter_map(|key| CommandId::from_key(key.trim())) {
        if !recent.contains(&id) {
            recent.push(id);
        }
    }
    recent.truncate(RECENT_MAX);
    recent
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Ctx {
        Ctx { layer_visible: true, pin_bottom: true, monitors: 2, tint: 70, ..Ctx::default() }
    }

    fn ids(query: &str, ctx: &Ctx) -> Vec<CommandId> {
        search(query, ctx, &[]).into_iter().map(|m| m.id).collect()
    }

    #[test]
    fn registry_has_one_entry_per_command() {
        assert_eq!(COMMANDS.len(), CommandId::ALL.len());
        for id in CommandId::ALL {
            assert_eq!(COMMANDS.iter().filter(|c| c.id == *id).count(), 1, "{id:?}");
        }
        let mut keys: Vec<_> = CommandId::ALL.iter().map(|id| id.key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), CommandId::ALL.len());
        for id in CommandId::ALL {
            assert_eq!(CommandId::from_key(id.key()), Some(*id));
        }
        // The empty query lists the registry in its own order.
        let order: Vec<_> = COMMANDS.iter().map(|c| c.id).collect();
        let mut all = ctx();
        (all.has_active_card, all.monitors) = (true, 2);
        assert_eq!(ids("", &all), order);
    }

    #[test]
    fn keys_are_snake_case() {
        for id in CommandId::ALL {
            assert!(id.key().bytes().all(|b| b.is_ascii_lowercase() || b == b'_'), "{}", id.key());
        }
    }

    #[test]
    fn prefix_beats_subsequence() {
        let found = ids("настр", &ctx());
        assert_eq!(found.first(), Some(&CommandId::OpenSettings));
        // "нст" is only a subsequence of "Настройки", but still finds it.
        assert!(ids("нстр", &ctx()).contains(&CommandId::OpenSettings));
        let (sub, prefix) = (score(&fold("нстк"), &fold("Настройки")), score(&fold("наст"), &fold("Настройки")));
        assert!(prefix > sub && sub.is_some());
    }

    #[test]
    fn word_start_beats_the_middle_of_a_word() {
        let word = score(&fold("слой"), &fold("Показать слой"));
        let inner = score(&fold("лой"), &fold("Показать слой"));
        assert!(word > inner && inner.is_some());
        // A letter typed alone only matches where a word begins.
        assert!(score(&fold("о"), &fold("Открыть корзину")).is_some());
        assert_eq!(score(&fold("н"), &fold("Корзина")), None);
        // Subsequence: word starts count.
        let spread = score(&fold("пс"), &fold("Показать слой"));
        let inside = score(&fold("пс"), &fold("Копировать с"));
        assert!(spread > inside);
    }

    #[test]
    fn whole_word_beats_prefix() {
        assert!(score(&fold("архив"), &fold("архив")) > score(&fold("архив"), &fold("архивация")));
    }

    #[test]
    fn synonyms_find_commands() {
        // The spec's example: "архив" opens the archive, ahead of the card's.
        let mut with_card = ctx();
        with_card.has_active_card = true;
        assert_eq!(ids("архив", &with_card).first(), Some(&CommandId::OpenArchive));
        assert!(ids("архив", &with_card).contains(&CommandId::ArchiveCard));
        assert_eq!(ids("trash", &ctx()).first(), Some(&CommandId::OpenTrash));
        assert_eq!(ids("backup", &ctx()).first(), Some(&CommandId::BackupNow));
        assert_eq!(ids("sticky", &ctx()).first(), Some(&CommandId::ImportSticky));
        assert_eq!(ids("корзина", &ctx()).first(), Some(&CommandId::OpenTrash));
        assert_eq!(ids("ИДЕИ", &ctx()).first(), Some(&CommandId::ShowIdeas));
        assert_eq!(ids("удалённые", &ctx()).first(), Some(&CommandId::OpenTrash));
        assert_eq!(ids("удаленные", &ctx()).first(), Some(&CommandId::OpenTrash));
        assert!(ids("zzzz", &ctx()).is_empty());
    }

    #[test]
    fn number_argument_is_split_off() {
        let found = search("затемнение 40", &ctx(), &[]);
        let first = &found[0];
        assert_eq!((first.id, first.arg, first.needs_arg), (CommandId::Dimming, Some(40), false));
        assert_eq!(first.title, "Затемнение слоя: 32%");
        // Without a number the command asks for one.
        let bare = &search("затемнение", &ctx(), &[])[0];
        assert_eq!((bare.id, bare.arg, bare.needs_arg), (CommandId::Dimming, None, true));
        // A number alone doesn't match everything: only commands that take one.
        let numbers: Vec<_> = ids("40", &ctx());
        assert_eq!(numbers, [CommandId::MoveToMonitor, CommandId::Dimming]);
        // Digits in the middle are not an argument.
        assert!(ids("затемнение 4 0", &ctx()).is_empty());
        assert_eq!(split_number("слой 12"), ("слой", Some(12)));
        assert_eq!(split_number("12"), ("", Some(12)));
        assert_eq!(split_number("слой"), ("слой", None));
        assert_eq!(split_number("слой 1x"), ("слой 1x", None));
        assert_eq!(split_number("99999999999999999999"), ("", Some(i64::MAX)));
        assert_eq!(split_number("слой -3"), ("слой", Some(-3)));
        assert_eq!(split_number("слой -"), ("слой -", None));
    }

    #[test]
    fn negative_and_huge_numbers_ask_for_a_valid_one() {
        // Out of range like "затемнение 400": the command stays listed and asks.
        for query in ["затемнение 99999999999999999999", "затемнение -5", "монитор -99999999999999999999"] {
            let m = &search(query, &ctx(), &[])[0];
            assert!(matches!(m.id, CommandId::Dimming | CommandId::MoveToMonitor), "{query}");
            assert_eq!((m.arg, m.needs_arg), (None, true), "{query}");
        }
    }

    #[test]
    fn number_out_of_range_is_rejected() {
        for query in ["затемнение 101", "затемнение 400", "монитор 0", "монитор 3"] {
            let m = &search(query, &ctx(), &[])[0];
            assert_eq!((m.arg, m.needs_arg), (None, true), "{query}");
        }
        for (query, id, n) in [
            ("затемнение 0", CommandId::Dimming, 0),
            ("затемнение 100", CommandId::Dimming, 100),
            ("монитор 1", CommandId::MoveToMonitor, 1),
            ("монитор 2", CommandId::MoveToMonitor, 2),
        ] {
            let m = &search(query, &ctx(), &[])[0];
            assert_eq!((m.id, m.arg, m.needs_arg), (id, Some(n), false), "{query}");
        }
    }

    #[test]
    fn tint_converts_to_percent_and_back() {
        assert_eq!(tint_from_percent(0), 0);
        assert_eq!(tint_from_percent(100), TINT_MAX);
        assert_eq!(tint_from_percent(500), TINT_MAX);
        assert_eq!(tint_from_percent(-3), 0);
        for percent in 0..=100 {
            assert_eq!(percent_of_tint(tint_from_percent(percent)), percent);
        }
        assert_eq!(percent_of_tint(255), 100);
    }

    #[test]
    fn inapplicable_commands_are_hidden() {
        let mut c = ctx();
        let all = |c: &Ctx| ids("", c);
        assert!(!all(&c).contains(&CommandId::ArchiveCard));
        assert!(!all(&c).contains(&CommandId::TogglePinCard));
        assert!(ids("карточка", &c).is_empty());
        c.has_active_card = true;
        assert!(all(&c).contains(&CommandId::ArchiveCard));
        assert!(ids("карточка", &c).contains(&CommandId::TogglePinCard));
        c.monitors = 1;
        assert!(!all(&c).contains(&CommandId::MoveToMonitor));
        c.layer_visible = false;
        assert!(!all(&c).contains(&CommandId::ToggleCurtain));
    }

    #[test]
    fn titles_show_the_state() {
        let mut c = ctx();
        assert_eq!(title(CommandId::TogglePinBottom, &c), "Слой под окнами: выключить");
        c.pin_bottom = false;
        assert_eq!(title(CommandId::TogglePinBottom, &c), "Слой под окнами: включить");
        assert_eq!(title(CommandId::ToggleAutostart, &c), "Запускать при входе: переключить");
        c.autostart = Some(true);
        assert_eq!(title(CommandId::ToggleAutostart, &c), "Запускать при входе: выключить");
        assert_eq!(title(CommandId::ToggleLayer, &c), "Скрыть слой");
        c.layer_visible = false;
        assert_eq!(title(CommandId::ToggleLayer, &c), "Показать слой");
        assert_eq!(title(CommandId::ToggleCurtain, &c), "Свернуть слой");
        c.collapsed = true;
        assert_eq!(title(CommandId::ToggleCurtain, &c), "Развернуть слой");
        // Rolled up, the layer's toggle brings it back instead of hiding it.
        c.layer_visible = true;
        assert_eq!(title(CommandId::ToggleLayer, &c), "Показать слой");
        assert_eq!(title(CommandId::TogglePinCard, &c), "Выбранная карточка: закрепить");
        c.active_pinned = true;
        assert_eq!(title(CommandId::TogglePinCard, &c), "Выбранная карточка: открепить");
        // The state is searchable too.
        assert_eq!(ids("выключить", &ctx()).first(), Some(&CommandId::TogglePinBottom));
    }

    #[test]
    fn empty_query_leads_with_recent_commands() {
        let recent = [CommandId::Quit, CommandId::OpenTrash, CommandId::ArchiveCard];
        let found: Vec<_> = search("", &ctx(), &recent).into_iter().map(|m| m.id).collect();
        // The card command isn't available: skipped, not shown at its old place.
        assert_eq!(&found[..2], [CommandId::Quit, CommandId::OpenTrash]);
        assert!(!found.contains(&CommandId::ArchiveCard));
        // No repeats, and the rest follows in registry order.
        assert_eq!(found.iter().filter(|id| **id == CommandId::Quit).count(), 1);
        assert_eq!(found[2], CommandId::NewNote);
        // A recent command that takes a number asks for it.
        let numbered = search("", &ctx(), &[CommandId::Dimming]);
        assert_eq!((numbered[0].id, numbered[0].needs_arg), (CommandId::Dimming, true));
    }

    #[test]
    fn recent_list_is_most_recent_first_without_repeats_and_capped() {
        let mut recent = Vec::new();
        for id in [CommandId::Quit, CommandId::Search, CommandId::Quit, CommandId::OpenTrash] {
            push_recent(&mut recent, id);
        }
        assert_eq!(recent, [CommandId::OpenTrash, CommandId::Quit, CommandId::Search]);
        for id in CommandId::ALL {
            push_recent(&mut recent, *id);
        }
        assert_eq!(recent.len(), RECENT_MAX);
        assert_eq!(recent[0], *CommandId::ALL.last().unwrap());
    }

    #[test]
    fn recent_list_round_trips_through_the_setting() {
        let recent = vec![CommandId::BackupNow, CommandId::Dimming, CommandId::OpenArchive];
        let text = encode_recent(&recent);
        assert_eq!(text, "backup_now,dimming,open_archive");
        assert_eq!(decode_recent(&text), recent);
        assert_eq!(encode_recent(&[]), "");
        assert!(decode_recent("").is_empty());
    }

    #[test]
    fn unknown_recent_ids_are_ignored() {
        assert_eq!(decode_recent("gone,quit,,quit, search ,x"), [CommandId::Quit, CommandId::Search]);
        let many = CommandId::ALL.iter().map(|id| id.key()).collect::<Vec<_>>().join(",");
        assert_eq!(decode_recent(&many).len(), RECENT_MAX);
    }

    #[test]
    fn search_filters_open_the_bar_in_search() {
        assert_eq!(CommandId::ShowIdeas.bar_target(), Some(BarTarget::Search("идеи ")));
        assert_eq!(CommandId::NewNote.bar_target(), Some(BarTarget::Capture));
        assert_eq!(CommandId::OpenArchive.bar_target(), None);
        // The words are the ones the note search understands as a type filter.
        for (id, kind) in [
            (CommandId::ShowIdeas, crate::card::Kind::Idea),
            (CommandId::ShowPrompts, crate::card::Kind::Prompt),
            (CommandId::ShowGoals, crate::card::Kind::Goal),
            (CommandId::ShowReminders, crate::card::Kind::Reminder),
            (CommandId::ShowLinks, crate::card::Kind::Link),
        ] {
            let Some(BarTarget::Search(text)) = id.bar_target() else { panic!("{id:?}") };
            assert_eq!(crate::search::parse_local(text, 0).kind, Some(kind), "{id:?}");
        }
    }

    #[test]
    fn filtering_the_whole_registry_is_cheap() {
        let c = Ctx { has_active_card: true, ..ctx() };
        let started = std::time::Instant::now();
        for query in ["а", "арх", "затемнение 40", "слой под окнами", "zzzzzz"] {
            for _ in 0..100 {
                std::hint::black_box(search(query, &c, &[]));
            }
        }
        // 500 searches; one is well under a millisecond even unoptimized.
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
    }
}
