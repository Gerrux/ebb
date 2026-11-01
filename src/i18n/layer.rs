//! The layer: cards, their menus and editors, toasts, the layer menu and debug panel.

use super::common::{en_days, ru_days};
use super::en_plural;

pub struct Layer {
    // Card: hover pill, menus
    pub no_status: &'static str,
    pub keep_on_layer: &'static str,
    pub copy_prompt: &'static str,
    pub copy_text: &'static str,
    pub copy_hidden: &'static str,
    pub open_link: &'static str,
    pub done_archive: &'static str,
    pub unpin: &'static str,
    pub pin_in_place: &'static str,
    pub collapse_to_line: &'static str,
    pub duplicate: &'static str,
    pub delete: &'static str,
    /// Caption of the "Later" menu: the days follow as `resurface::snooze_label`.
    pub put_away_until: &'static str,
    pub color: &'static str,
    /// The tint swatch that means "the kind's own color".
    pub by_kind: &'static str,
    pub expand: &'static str,
    pub more: &'static str,
    pub archive: &'static str,
    pub later: &'static str,
    pub reveal_5s: &'static str,
    pub text_copied: &'static str,
    /// Accessible name of the kind mark, from the kind's label.
    pub kind_aria: fn(&str) -> String,
    /// Hover text of the kind mark, from the kind's label.
    pub change_kind_tip: fn(&str) -> String,
    /// The idea status chip while no status is set.
    pub status: &'static str,
    pub idea_status_aria: fn(&str) -> String,
    pub remove_tag_aria: fn(&str) -> String,
    pub remove_tag: &'static str,
    pub add_tag: &'static str,
    pub collapsed_aria: fn(&str) -> String,

    // Card text and editor
    pub copy_line_aria: fn(&str) -> String,
    pub copy_line: &'static str,
    /// Formatting toolbar, in the order bold, italic, underline, strikethrough,
    /// highlight, code, clear.
    pub format_tips: [&'static str; 7],
    pub private_label_hint: &'static str,
    pub private_secret_hint: &'static str,
    pub new_tag_hint: &'static str,
    /// Heading of the Prompt variables dialog.
    pub fill_prompt: &'static str,
    pub copy: &'static str,
    pub cancel: &'static str,

    // Toasts
    pub done_archived: &'static str,
    pub archived: &'static str,
    /// "Type: Note → Idea", from the two kind labels.
    pub kind_changed: fn(&str, &str) -> String,
    /// From `store::TRASH_DAYS`.
    pub trashed: fn(i64) -> String,
    pub stays_on_layer: &'static str,
    pub tag_removed: fn(&str) -> String,
    /// "Back tomorrow", from `resurface::snooze_label`.
    pub snoozed_until: fn(&str) -> String,
    pub card_style_toast: fn(&str) -> String,
    pub undo: &'static str,
    pub autostart_on: &'static str,
    pub autostart_off: &'static str,
    pub backup_saved: &'static str,

    // Verb phrases for `common.failed`: "Couldn't {…}"
    pub do_erase_secret: &'static str,
    pub do_discard_empty: &'static str,
    pub do_create_note: &'static str,
    pub do_change_kind: &'static str,
    pub do_make_copy: &'static str,
    pub do_trash: &'static str,
    pub do_change_status: &'static str,
    pub do_collapse: &'static str,
    pub do_expand: &'static str,
    pub do_snooze: &'static str,
    pub do_save_capture: &'static str,
    pub do_save_changes: &'static str,
    pub do_undo: &'static str,
    pub do_change_autostart: &'static str,
    pub do_backup: &'static str,
    pub do_open_db: &'static str,
    pub do_read_notes: &'static str,

    // Hotkey errors in Settings
    /// From the label of the action that holds the combination.
    pub hotkey_taken_by: fn(&str) -> String,
    pub hotkey_taken_other: &'static str,

    // Layer header, menu, tab
    /// Card count, capture key, search key.
    pub header_line: fn(usize, &str, &str) -> String,
    pub new_note: &'static str,
    pub menu: &'static str,
    pub hide_layer: &'static str,
    pub send_to_background: &'static str,
    pub find: &'static str,
    pub weekly_review: &'static str,
    pub import_sticky: &'static str,
    pub settings: &'static str,
    pub collapse_up: &'static str,
    pub collapse_down: &'static str,
    pub exit: &'static str,
    pub expand_ebb: &'static str,

    // Debug panel (F1)
    /// A duration in milliseconds.
    pub ms: fn(f64) -> String,
    pub dbg_first_frame_process: &'static str,
    pub dbg_first_frame_main: &'static str,
    pub dbg_hotkey_latency: &'static str,
    pub dbg_memory: &'static str,
    pub dbg_frames: &'static str,
    pub dbg_backdrop_monitor: &'static str,

    // Review invitation
    pub start: &'static str,
    pub not_now: &'static str,

    // Startup errors
    /// Failed action (already phrased), the database error, the database path.
    pub startup_error: fn(&str, &str, &str) -> String,
    /// The newer schema version in the file, the newest this build knows.
    pub db_too_new: fn(i64, i64) -> String,
}

pub const RU: Layer = Layer {
    no_status: "Без статуса",
    keep_on_layer: "Оставить на слое",
    copy_prompt: "Копировать prompt",
    copy_text: "Копировать текст",
    copy_hidden: "Скопировать, не показывая",
    open_link: "Открыть ссылку",
    done_archive: "Сделано: в архив",
    unpin: "Открепить",
    pin_in_place: "Закрепить на месте",
    collapse_to_line: "Свернуть в строку",
    duplicate: "Дублировать",
    delete: "Удалить",
    put_away_until: "Убрать и вернуть",
    color: "Цвет",
    by_kind: "По типу",
    expand: "Развернуть",
    more: "Ещё",
    archive: "В архив",
    later: "Позже",
    reveal_5s: "Показать на 5 секунд",
    text_copied: "Текст скопирован",
    kind_aria: |kind| format!("Тип: {kind}"),
    change_kind_tip: |kind| format!("{kind} — сменить тип"),
    status: "Статус",
    idea_status_aria: |text| format!("Статус идеи: {text}"),
    remove_tag_aria: |tag| format!("Убрать тег #{tag}"),
    remove_tag: "Убрать тег",
    add_tag: "Добавить тег",
    collapsed_aria: |line| format!("Свёрнута: {line}"),

    copy_line_aria: |line| format!("Копировать строку: {line}"),
    copy_line: "Копировать строку",
    format_tips: [
        "Жирный (Ctrl+B)",
        "Курсив (Ctrl+I)",
        "Подчёркивание (Ctrl+U)",
        "Зачёркивание (Ctrl+Shift+S)",
        "Выделение маркером (Ctrl+Shift+H)",
        "Моноширинный код (Ctrl+Shift+K)",
        "Снять форматирование",
    ],
    private_label_hint: "Метка — видна на карточке",
    private_secret_hint: "Секрет — хранится зашифрованным",
    new_tag_hint: "новый тег",
    fill_prompt: "Подставить в prompt",
    copy: "Копировать",
    cancel: "Отмена",

    done_archived: "Сделано, карточка в архиве",
    archived: "Карточка в архиве",
    kind_changed: |old, new| format!("Тип: {old} → {new}"),
    trashed: |days| format!("Карточка в корзине, {days} {} можно вернуть", ru_days(days)),
    stays_on_layer: "Останется на слое",
    tag_removed: |tag| format!("Тег #{tag} убран"),
    snoozed_until: |when| format!("Вернётся {when}"),
    card_style_toast: |label| format!("Вид карточек: {label} (F4 — дальше)"),
    undo: "Отменить",
    autostart_on: "Запускать при входе: включено",
    autostart_off: "Запускать при входе: выключено",
    backup_saved: "Резервная копия сохранена",

    do_erase_secret: "стереть секрет",
    do_discard_empty: "убрать пустую заметку",
    do_create_note: "создать заметку",
    do_change_kind: "сменить тип",
    do_make_copy: "создать копию",
    do_trash: "убрать в корзину",
    do_change_status: "поменять статус идеи",
    do_collapse: "свернуть карточку",
    do_expand: "развернуть карточку",
    do_snooze: "отложить карточку",
    do_save_capture: "сохранить заметку, текст остался в окне захвата",
    do_save_changes: "сохранить изменения",
    do_undo: "отменить",
    do_change_autostart: "изменить автозапуск",
    do_backup: "сделать резервную копию",
    do_open_db: "открыть базу заметок",
    do_read_notes: "прочитать заметки",

    hotkey_taken_by: |label| format!("Уже занято действием «{label}»"),
    hotkey_taken_other: "Занято другой программой",

    header_line: |n, capture, search| {
        format!("{n} карточек  ·  {capture} — записать  ·  {search} — найти  ·  F1 — debug")
    },
    new_note: "Новая заметка",
    menu: "Меню",
    hide_layer: "Скрыть слой",
    send_to_background: "Убрать на фон",
    find: "Найти",
    weekly_review: "Еженедельный обзор",
    import_sticky: "Импорт из Sticky Notes…",
    settings: "Настройки…",
    collapse_up: "Свернуть вверх",
    collapse_down: "Свернуть вниз",
    exit: "Выход",
    expand_ebb: "Развернуть Ebb",

    ms: |v| format!("{v:.0} мс"),
    dbg_first_frame_process: "Первый кадр от старта процесса",
    dbg_first_frame_main: "Первый кадр от main()",
    dbg_hotkey_latency: "Хоткей → кадр окна захвата",
    dbg_memory: "Память: working set / private",
    dbg_frames: "Кадров отрисовано",
    dbg_backdrop_monitor: "Фон (F2) / монитор (F3)",

    start: "Начать",
    not_now: "Не сейчас",

    startup_error: |failed, error, path| {
        format!("{failed}:\n{error}\n\n{path}\n\nЗаметки не тронуты. Проверь, что диск доступен и на нём есть место, и запусти Ebb снова.")
    },
    db_too_new: |version, latest| {
        format!("база заметок создана более новой версией Ebb (схема {version}, эта версия знает до {latest}). Обнови Ebb.")
    },
};

pub const EN: Layer = Layer {
    no_status: "No status",
    keep_on_layer: "Keep on the layer",
    copy_prompt: "Copy prompt",
    copy_text: "Copy text",
    copy_hidden: "Copy without showing",
    open_link: "Open link",
    done_archive: "Done: archive",
    unpin: "Unpin",
    pin_in_place: "Pin in place",
    collapse_to_line: "Collapse to one line",
    duplicate: "Duplicate",
    delete: "Delete",
    put_away_until: "Put away until",
    color: "Color",
    by_kind: "By type",
    expand: "Expand",
    more: "More",
    archive: "Archive",
    later: "Later",
    reveal_5s: "Show for 5 seconds",
    text_copied: "Text copied",
    kind_aria: |kind| format!("Type: {kind}"),
    change_kind_tip: |kind| format!("{kind} — change type"),
    status: "Status",
    idea_status_aria: |text| format!("Idea status: {text}"),
    remove_tag_aria: |tag| format!("Remove tag #{tag}"),
    remove_tag: "Remove tag",
    add_tag: "Add tag",
    collapsed_aria: |line| format!("Collapsed: {line}"),

    copy_line_aria: |line| format!("Copy line: {line}"),
    copy_line: "Copy line",
    format_tips: [
        "Bold (Ctrl+B)",
        "Italic (Ctrl+I)",
        "Underline (Ctrl+U)",
        "Strikethrough (Ctrl+Shift+S)",
        "Highlight (Ctrl+Shift+H)",
        "Monospace code (Ctrl+Shift+K)",
        "Clear formatting",
    ],
    private_label_hint: "Label — shown on the card",
    private_secret_hint: "Secret — stored encrypted",
    new_tag_hint: "new tag",
    fill_prompt: "Fill in the prompt",
    copy: "Copy",
    cancel: "Cancel",

    done_archived: "Done, card archived",
    archived: "Card archived",
    kind_changed: |old, new| format!("Type: {old} → {new}"),
    trashed: |days| format!("Card moved to trash, you can restore it for {days} {}", en_days(days)),
    stays_on_layer: "Stays on the layer",
    tag_removed: |tag| format!("Tag #{tag} removed"),
    snoozed_until: |when| format!("Back {when}"),
    card_style_toast: |label| format!("Card style: {label} (F4 — next)"),
    undo: "Undo",
    autostart_on: "Start when I sign in: on",
    autostart_off: "Start when I sign in: off",
    backup_saved: "Backup saved",

    do_erase_secret: "erase the secret",
    do_discard_empty: "discard the empty note",
    do_create_note: "create the note",
    do_change_kind: "change the type",
    do_make_copy: "make a copy",
    do_trash: "move it to trash",
    do_change_status: "change the idea status",
    do_collapse: "collapse the card",
    do_expand: "expand the card",
    do_snooze: "snooze the card",
    do_save_capture: "save the note, the text is still in the capture window",
    do_save_changes: "save the changes",
    do_undo: "undo",
    do_change_autostart: "change autostart",
    do_backup: "make a backup",
    do_open_db: "open the notes database",
    do_read_notes: "read the notes",

    hotkey_taken_by: |label| format!("Already used by “{label}”"),
    hotkey_taken_other: "Used by another program",

    header_line: |n, capture, search| {
        format!(
            "{n} {}  ·  {capture} — capture  ·  {search} — find  ·  F1 — debug",
            en_plural(n as i64, "card", "cards")
        )
    },
    new_note: "New note",
    menu: "Menu",
    hide_layer: "Hide the layer",
    send_to_background: "Send to background",
    find: "Find",
    weekly_review: "Weekly review",
    import_sticky: "Import from Sticky Notes…",
    settings: "Settings…",
    collapse_up: "Collapse up",
    collapse_down: "Collapse down",
    exit: "Exit",
    expand_ebb: "Expand Ebb",

    ms: |v| format!("{v:.0} ms"),
    dbg_first_frame_process: "First frame from process start",
    dbg_first_frame_main: "First frame from main()",
    dbg_hotkey_latency: "Hotkey → capture window frame",
    dbg_memory: "Memory: working set / private",
    dbg_frames: "Frames drawn",
    dbg_backdrop_monitor: "Backdrop (F2) / monitor (F3)",

    start: "Start",
    not_now: "Not now",

    startup_error: |failed, error, path| {
        format!("{failed}:\n{error}\n\n{path}\n\nYour notes are untouched. Check that the disk is available and has free space, then start Ebb again.")
    },
    db_too_new: |version, latest| {
        format!("the notes database was created by a newer version of Ebb (schema {version}, this version supports up to {latest}). Update Ebb.")
    },
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_phrases() {
        assert_eq!((EN.kind_aria)("Idea"), "Type: Idea");
        assert_eq!((EN.change_kind_tip)("Idea"), "Idea — change type");
        assert_eq!((EN.idea_status_aria)("Maybe"), "Idea status: Maybe");
        assert_eq!((EN.remove_tag_aria)("work"), "Remove tag #work");
        assert_eq!((EN.collapsed_aria)("Buy milk"), "Collapsed: Buy milk");
        assert_eq!((EN.copy_line_aria)("abc"), "Copy line: abc");
        assert_eq!((EN.kind_changed)("Note", "Idea"), "Type: Note → Idea");
        assert_eq!((EN.tag_removed)("work"), "Tag #work removed");
        assert_eq!((EN.snoozed_until)("tomorrow"), "Back tomorrow");
        assert_eq!((EN.snoozed_until)("in a week"), "Back in a week");
        assert_eq!((EN.card_style_toast)("Tint"), "Card style: Tint (F4 — next)");
        assert_eq!((EN.hotkey_taken_by)("Search"), "Already used by “Search”");
        assert_eq!((EN.ms)(14.2), "14 ms");
        assert_eq!((RU.ms)(14.2), "14 мс");
    }

    #[test]
    fn trash_notice_counts_days() {
        assert_eq!((RU.trashed)(30), "Карточка в корзине, 30 дней можно вернуть");
        assert_eq!((EN.trashed)(30), "Card moved to trash, you can restore it for 30 days");
        assert_eq!((EN.trashed)(1), "Card moved to trash, you can restore it for 1 day");
    }

    #[test]
    fn header_line_lists_keys() {
        assert_eq!(
            (RU.header_line)(6, "Win+Alt+N", "Win+Alt+F"),
            "6 карточек  ·  Win+Alt+N — записать  ·  Win+Alt+F — найти  ·  F1 — debug"
        );
        assert_eq!(
            (EN.header_line)(6, "Win+Alt+N", "Win+Alt+F"),
            "6 cards  ·  Win+Alt+N — capture  ·  Win+Alt+F — find  ·  F1 — debug"
        );
        assert!((EN.header_line)(1, "a", "b").starts_with("1 card  ·"));
    }

    #[test]
    fn startup_messages() {
        let msg = (EN.startup_error)("Couldn't open the notes database", "disk full", "C:\\Ebb\\ebb.db");
        assert!(msg.starts_with("Couldn't open the notes database:\ndisk full\n\nC:\\Ebb\\ebb.db\n\n"), "{msg}");
        assert!(msg.ends_with("start Ebb again."), "{msg}");
        assert_eq!(
            (EN.db_too_new)(5, 4),
            "the notes database was created by a newer version of Ebb (schema 5, this version supports up to 4). Update Ebb."
        );
        assert!((RU.db_too_new)(5, 4).contains("более новой версией Ebb"));
    }

    #[test]
    fn failed_phrases_compose() {
        let failed = super::super::common::EN.failed;
        assert_eq!(failed(EN.do_trash), "Couldn't move it to trash");
        assert_eq!(failed(EN.do_backup), "Couldn't make a backup");
        let failed = super::super::common::RU.failed;
        assert_eq!(failed(RU.do_change_autostart), "Не удалось изменить автозапуск");
    }
}
