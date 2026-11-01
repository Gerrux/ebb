//! Words shared by several windows: names of kinds, statuses, colors and looks, counts,
//! dates and ages.

use super::{en_plural, ru_plural};

pub struct Common {
    /// Indexed like `card::Kind::ALL`.
    pub kinds: [&'static str; 8],
    /// Indexed like `card::IdeaStatus::ALL`.
    pub idea_statuses: [&'static str; 4],
    /// Indexed like `card::Tint::ALL`.
    pub tints: [&'static str; 8],
    /// Indexed like `card::Marker::ALL`.
    pub markers: [&'static str; 9],
    /// Indexed like `card::IconSpot::ALL`.
    pub icon_spots: [&'static str; 3],
    /// Indexed like `card::PRESETS`.
    pub preset_names: [&'static str; 8],
    pub preset_hints: [&'static str; 8],
    /// Indexed like `theme::ThemeMode::ALL`.
    pub theme_modes: [&'static str; 3],
    /// `theme::CardFont::SegoePrint`; other fonts are shown by name.
    pub font_segoe_print: &'static str,
    /// Indexed like `win::Backdrop::ALL`.
    pub backdrops: [&'static str; 3],
    /// Indexed like `hotkey::Action::ALL`.
    pub hotkey_actions: [&'static str; 5],
    /// Monday first.
    pub weekdays: [&'static str; 7],
    /// "23 дня" / "23 days".
    pub days: fn(i64) -> String,
    /// "5 заметок" / "5 notes".
    pub notes: fn(usize) -> String,
    /// How long ago a card was written, from minutes: "сейчас", "5 мин", "3 ч", "23 дн".
    pub age_minutes: fn(i64) -> String,
    /// How long ago, from days, for lists: "сегодня", "вчера", "23 дн", "1 мес", "2 г".
    pub age_days: fn(i64) -> String,
    /// A date without the year, from month (1–12) and day: "29 сен" / "Sep 29".
    pub short_date: fn(u32, u32) -> String,
    /// An action that failed, from its verb phrase: "Не удалось {what}".
    pub failed: fn(&str) -> String,
    /// Shown when a window can't open its own connection to the database.
    pub store_failed: &'static str,
}

pub(crate) fn ru_days(n: i64) -> &'static str {
    ru_plural(n, "день", "дня", "дней")
}

pub(crate) fn en_days(n: i64) -> &'static str {
    en_plural(n, "day", "days")
}

pub(crate) fn ru_notes(n: usize) -> &'static str {
    ru_plural(n as i64, "заметка", "заметки", "заметок")
}

pub(crate) fn en_notes(n: usize) -> &'static str {
    en_plural(n as i64, "note", "notes")
}

pub const RU: Common = Common {
    kinds: ["Заметка", "Идея", "Prompt", "Ссылка", "Цель", "Напоминание", "Reference", "Private"],
    idea_statuses: ["Потенциал", "Может быть", "Изучить", "Важно"],
    tints: ["Жёлтый", "Оранжевый", "Розовый", "Фиолетовый", "Синий", "Бирюзовый", "Зелёный", "Серый"],
    markers: [
        "Без цвета",
        "Свечение из угла",
        "Полоса сверху",
        "Полоса слева",
        "Тонировка",
        "Цветная рамка",
        "Заливка",
        "Бумага с шапкой",
        "Рамка и тон",
    ],
    icon_spots: ["Значок", "Точка и название", "Только при наведении"],
    preset_names: [
        "Ebb",
        "Sticky Notes",
        "Google Keep",
        "Obsidian Canvas",
        "Trello",
        "Бумажный стикер",
        "Минимализм",
        "Windows",
    ],
    preset_hints: [
        "Стекло и свечение цвета",
        "Цветная бумага, шапка при наведении",
        "Пастель днём, ночные тона в темноте",
        "Рамка и лёгкий тон",
        "Цветная метка слева",
        "Заливка, тень, от руки",
        "Только жирная иконка",
        "Фон как у панели задач",
    ],
    theme_modes: ["Как в Windows", "Тёмная", "Светлая"],
    font_segoe_print: "Segoe Print (рукописный)",
    backdrops: ["Размытие фона", "Размытие Windows (серое, когда слой неактивен)", "Без размытия"],
    hotkey_actions: ["Записать мысль", "Поиск", "Архив и корзина", "Показать / скрыть слой", "Команды"],
    weekdays: ["Понедельник", "Вторник", "Среда", "Четверг", "Пятница", "Суббота", "Воскресенье"],
    days: |n| format!("{n} {}", ru_days(n)),
    notes: |n| format!("{n} {}", ru_notes(n)),
    age_minutes: |m| match m {
        ..=0 => "сейчас".into(),
        m if m < 60 => format!("{m} мин"),
        m if m < 60 * 24 => format!("{} ч", m / 60),
        m => format!("{} дн", m / (60 * 24)),
    },
    age_days: |d| match d {
        ..=0 => "сегодня".into(),
        1 => "вчера".into(),
        d if d < 30 => format!("{d} дн"),
        d if d < 365 => format!("{} мес", d / 30),
        d => format!("{} г", d / 365),
    },
    short_date: |m, d| {
        const MONTHS: [&str; 12] = ["янв", "фев", "мар", "апр", "мая", "июн", "июл", "авг", "сен", "окт", "ноя", "дек"];
        format!("{d} {}", MONTHS[(m.clamp(1, 12) - 1) as usize])
    },
    failed: |what| format!("Не удалось {what}"),
    store_failed: "База заметок недоступна",
};

pub const EN: Common = Common {
    kinds: ["Note", "Idea", "Prompt", "Link", "Goal", "Reminder", "Reference", "Private"],
    idea_statuses: ["Potential", "Maybe", "Explore", "Important"],
    tints: ["Yellow", "Orange", "Pink", "Purple", "Blue", "Teal", "Green", "Gray"],
    markers: [
        "No color",
        "Corner glow",
        "Strip on top",
        "Strip on the left",
        "Tint",
        "Colored border",
        "Fill",
        "Paper with a header",
        "Border and tint",
    ],
    icon_spots: ["Icon", "Dot and name", "Only on hover"],
    preset_names: [
        "Ebb",
        "Sticky Notes",
        "Google Keep",
        "Obsidian Canvas",
        "Trello",
        "Paper sticker",
        "Minimal",
        "Windows",
    ],
    preset_hints: [
        "Glass and a glow of color",
        "Colored paper, header on hover",
        "Pastel by day, night tones in the dark",
        "A border and a light tint",
        "A colored label on the left",
        "Fill, shadow, handwriting",
        "Just a bold icon",
        "Background like the taskbar",
    ],
    theme_modes: ["Same as Windows", "Dark", "Light"],
    font_segoe_print: "Segoe Print (handwriting)",
    backdrops: ["Background blur", "Windows blur (gray while the layer is inactive)", "No blur"],
    hotkey_actions: ["Capture a thought", "Search", "Archive and trash", "Show / hide the layer", "Commands"],
    weekdays: ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"],
    days: |n| format!("{n} {}", en_days(n)),
    notes: |n| format!("{n} {}", en_notes(n)),
    age_minutes: |m| match m {
        ..=0 => "now".into(),
        m if m < 60 => format!("{m} min"),
        m if m < 60 * 24 => format!("{} h", m / 60),
        m => format!("{} d", m / (60 * 24)),
    },
    age_days: |d| match d {
        ..=0 => "today".into(),
        1 => "yesterday".into(),
        d if d < 30 => format!("{d} d"),
        d if d < 365 => format!("{} mo", d / 30),
        d => format!("{} y", d / 365),
    },
    short_date: |m, d| {
        const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        format!("{} {d}", MONTHS[(m.clamp(1, 12) - 1) as usize])
    },
    failed: |what| format!("Couldn't {what}"),
    store_failed: "The notes database is unavailable",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_ages() {
        assert_eq!((RU.days)(23), "23 дня");
        assert_eq!((EN.days)(23), "23 days");
        assert_eq!((EN.days)(1), "1 day");
        assert_eq!((RU.notes)(21), "21 заметка");
        assert_eq!((EN.notes)(1), "1 note");
        assert_eq!((EN.notes)(5), "5 notes");
        assert_eq!([(RU.age_minutes)(0), (RU.age_minutes)(5), (RU.age_minutes)(180), (RU.age_minutes)(23 * 1440)], [
            "сейчас", "5 мин", "3 ч", "23 дн"
        ]);
        assert_eq!([(EN.age_minutes)(0), (EN.age_minutes)(5), (EN.age_minutes)(60), (EN.age_minutes)(23 * 1440)], [
            "now", "5 min", "1 h", "23 d"
        ]);
        assert_eq!([(RU.age_days)(0), (RU.age_days)(1), (RU.age_days)(23), (RU.age_days)(40), (RU.age_days)(800)], [
            "сегодня", "вчера", "23 дн", "1 мес", "2 г"
        ]);
        assert_eq!([(EN.age_days)(0), (EN.age_days)(1), (EN.age_days)(23), (EN.age_days)(40), (EN.age_days)(800)], [
            "today", "yesterday", "23 d", "1 mo", "2 y"
        ]);
        assert_eq!((RU.short_date)(11, 14), "14 ноя");
        assert_eq!((EN.short_date)(11, 14), "Nov 14");
        assert_eq!((EN.failed)("save the note"), "Couldn't save the note");
    }
}
