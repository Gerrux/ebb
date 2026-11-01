//! Library window: archive, trash, weekly review, import review, settings; the Sticky Notes
//! import panel.

use super::common::{en_days, en_notes, ru_days};
use super::ru_plural;

pub struct Library {
    /// Settings: the language choice, before the language names.
    pub language: &'static str,
    /// Settings: follow the Windows UI language.
    pub language_system: &'static str,

    // Hotkey recording hints
    pub hotkey_needs_modifiers: &'static str,
    pub hotkey_key_unavailable: &'static str,

    // Header
    pub title_library: &'static str,
    pub title_settings: &'static str,
    pub title_welcome: &'static str,
    pub tab_archive: fn(i64) -> String,
    pub tab_trash: fn(i64) -> String,
    pub tab_review: &'static str,
    pub tab_import: fn(usize) -> String,
    pub tab_settings: &'static str,
    pub close: &'static str,
    pub close_hint: &'static str,

    // Weekly review
    pub review_empty_title: &'static str,
    pub review_empty_text: &'static str,
    pub review_done_title: &'static str,
    pub review_undo_hint: &'static str,
    pub review_why_reminder: &'static str,
    pub review_why_import: &'static str,
    pub review_why_old: &'static str,
    /// How long ago the card was written, from whole days: "записано 5 дней назад".
    pub written_ago: fn(i64) -> String,
    /// "3 из 15": the current card and the total.
    pub review_position: fn(usize, usize) -> String,
    /// Buttons, indexed like the review's action list (keep, archive, later, pin, goal, delete).
    pub review_actions: [&'static str; 6],
    /// What the summary calls each action, same order.
    pub review_outcomes: [&'static str; 6],
    pub review_hint: &'static str,
    /// The invitation on the layer, from the card count and minutes.
    pub invite_text: fn(usize, usize) -> String,

    // Import review
    /// A notice followed by the undo hint.
    pub with_undo_hint: fn(&str) -> String,
    pub import_apply_failed: &'static str,
    pub import_undone: fn(usize) -> String,
    pub import_undo_failed: &'static str,
    pub import_left: fn(usize) -> String,
    pub import_undo_last: &'static str,
    pub import_sorted_title: &'static str,
    pub import_sorted_text: &'static str,
    /// Indexed like `sticky::ImportGroup::ALL`.
    pub import_groups: [&'static str; 7],
    /// Cards in a group and how many of them are on the layer.
    pub import_count_on_layer: fn(usize, usize) -> String,
    pub import_hint_secrets: &'static str,
    pub import_hint_old: &'static str,
    pub import_hint_other: &'static str,
    /// From the kind's name.
    pub import_hint_kind: fn(&str) -> String,
    pub import_accept: fn(&str) -> String,
    pub import_keep: &'static str,
    /// The notice after a group stays as it is, from "5 заметок".
    pub import_kept: fn(&str) -> String,
    pub import_archive_all: &'static str,
    pub import_archived: fn(&str) -> String,
    pub import_trash_all: &'static str,
    /// From the notes ("5 заметок") and the days the trash keeps them.
    pub import_trashed: fn(&str, i64) -> String,
    pub import_collapse: &'static str,
    pub import_expand: &'static str,
    pub import_kind_set: fn(&str) -> String,
    /// How long ago a listed card changed, from whole days.
    pub import_item_age: fn(i64) -> String,

    // Archive and trash
    pub search_trash: &'static str,
    pub search_archive: &'static str,
    pub empty_trash: &'static str,
    pub empty_trash_confirm: &'static str,
    pub trash_emptied: &'static str,
    pub trash_empty_failed: &'static str,
    pub count_in_trash: fn(i64) -> String,
    pub count_in_archive: fn(i64) -> String,
    /// A search result count: found, total.
    pub found: fn(usize, i64) -> String,
    /// From the days the trash keeps notes.
    pub trash_keeps: fn(i64) -> String,
    pub trash_is_empty: &'static str,
    pub archive_is_empty: &'static str,
    pub nothing_found: &'static str,
    pub list_hint_trash: &'static str,
    pub list_hint_archive: &'static str,
    pub returned_to_layer: &'static str,
    pub moved_to_trash: &'static str,
    pub move_to_trash_failed: &'static str,
    pub restored_to_archive: &'static str,
    pub restored_to_layer: &'static str,
    pub restore_failed: &'static str,
    pub purged: &'static str,
    pub purge_failed: &'static str,
    pub purge_confirm_button: &'static str,
    pub purge_button: &'static str,
    pub restore_button: &'static str,
    pub delete_button: &'static str,
    pub to_layer_button: &'static str,
    /// In the trash, when a row isn't hovered: days since deleted, days left.
    pub trash_row_age: fn(i64, i64) -> String,
    pub was_in_archive: &'static str,
    pub was_on_layer: &'static str,

    // Settings: monitor map
    pub monitor_name: fn(usize) -> String,
    pub monitor_primary: &'static str,
    // Settings: card background
    pub bg_theme: &'static str,
    pub bg_taskbar: &'static str,
    pub bg_custom: &'static str,
    pub bg_custom_note: &'static str,
    pub bg_taskbar_note: &'static str,
    // Settings: hotkey rows
    pub hotkey_press: &'static str,
    pub hotkey_off: &'static str,
    pub hotkey_taken: fn(&str) -> String,
    pub hotkey_none_free: &'static str,
    pub hotkey_reset: &'static str,
    pub hotkey_reset_hint: &'static str,
    pub hotkey_turn_off: &'static str,

    // Settings: welcome and screen
    pub welcome_text: &'static str,
    pub welcome_note: &'static str,
    pub section_screen: &'static str,
    pub screen_note: &'static str,
    // Settings: layer
    pub section_layer: &'static str,
    pub on_top: &'static str,
    pub on_top_note: &'static str,
    pub esc_caption: &'static str,
    pub esc_background: &'static str,
    pub esc_hide: &'static str,
    pub collapse_caption: &'static str,
    pub collapse_top: &'static str,
    pub collapse_bottom: &'static str,
    pub collapse_note: &'static str,
    // Settings: appearance
    pub section_appearance: &'static str,
    pub theme: &'static str,
    pub card_looks: &'static str,
    pub custom_style_note: &'static str,
    pub customize: &'static str,
    pub marker_caption: &'static str,
    pub intensity: &'static str,
    pub strip_width: &'static str,
    pub corner_radius: &'static str,
    pub shadow: &'static str,
    pub opacity: &'static str,
    pub text_font: &'static str,
    pub text_size: &'static str,
    pub icon_caption: &'static str,
    pub big_icon: &'static str,
    pub card_background: &'static str,
    pub appearance_note: &'static str,
    // Settings: cards, Private, review
    pub section_cards: &'static str,
    pub snap: &'static str,
    pub snap_note: &'static str,
    pub section_private: &'static str,
    pub hide_from_capture: &'static str,
    pub hide_from_capture_note: &'static str,
    pub section_review: &'static str,
    pub review_invite: &'static str,
    pub review_when: &'static str,
    pub review_invite_note: &'static str,
    // Settings: startup
    pub section_startup: &'static str,
    pub open_on_launch: &'static str,
    pub autostart: &'static str,
    pub autostart_note: &'static str,
    pub autostart_other_copy: fn(&str) -> String,
    pub autostart_unknown: fn(&str) -> String,
    pub autostart_fix: &'static str,
    pub autostart_fix_hint: &'static str,
    // Settings: backdrop and hotkeys
    pub section_backdrop: &'static str,
    pub backdrop_caption: &'static str,
    pub dimming: &'static str,
    pub section_hotkeys: &'static str,
    pub hotkeys_note: &'static str,
    // Settings: import and data
    pub section_import: &'static str,
    pub import_sticky: &'static str,
    pub sort_out_import: fn(usize) -> String,
    pub import_note: &'static str,
    pub section_data: &'static str,
    /// Cards on the layer, in the archive, in the trash.
    pub data_counts: fn(i64, i64, i64) -> String,
    pub open_folder: &'static str,
    pub last_backup: fn(&str) -> String,
    pub no_backup_yet: &'static str,
    pub backup_note: &'static str,
    pub backup_now: &'static str,
    pub show_backups: &'static str,
    pub backup_saved: &'static str,
    pub backup_failed: fn(&str) -> String,
    pub backup_busy: &'static str,

    // Sticky Notes import panel
    pub sticky_title: &'static str,
    pub sticky_not_found: &'static str,
    /// Already imported and changed in Ebb, empty, duplicates.
    pub sticky_nothing_new: fn(usize, usize, usize) -> String,
    pub sticky_read_failed: fn(&str) -> String,
    pub sticky_import_failed: fn(&str) -> String,
    pub sticky_undo_failed: fn(&str) -> String,
    /// Importable, total.
    pub sticky_found: fn(usize, usize) -> String,
    /// Open notes, placed on their own monitor, placed next to it.
    pub sticky_to_layer: fn(usize, usize, usize) -> String,
    /// Archived, not changed for a long time.
    pub sticky_to_archive: fn(usize, usize) -> String,
    pub sticky_colors: fn(usize) -> String,
    /// Empty, duplicates.
    pub sticky_skip: fn(usize, usize) -> String,
    /// Replaced, kept.
    pub sticky_previous: fn(usize, usize) -> String,
    pub sticky_readonly: &'static str,
    pub sticky_import: &'static str,
    pub sticky_not_now: &'static str,
    pub sticky_never: &'static str,
    pub sticky_done_title: &'static str,
    /// Importable, on the layer, in the archive.
    pub sticky_done_counts: fn(usize, usize, usize) -> String,
    pub sticky_replaced: fn(usize) -> String,
    pub sticky_originals: &'static str,
    pub sticky_sort_out: &'static str,
    pub sticky_sort_out_hint: &'static str,
    pub sticky_done_button: &'static str,
    pub sticky_undo: &'static str,
}

fn ru_cards(n: usize) -> &'static str {
    ru_plural(n as i64, "заметку", "заметки", "заметок")
}

pub const RU: Library = Library {
    language: "Язык",
    language_system: "Как в Windows",

    hotkey_needs_modifiers: "Нужен Win или два из Ctrl, Alt, Shift",
    hotkey_key_unavailable: "Эту клавишу нельзя использовать",

    title_library: "Библиотека",
    title_settings: "Настройки Ebb",
    title_welcome: "Добро пожаловать в Ebb",
    tab_archive: |n| format!("Архив {n}"),
    tab_trash: |n| format!("Корзина {n}"),
    tab_review: "Обзор",
    tab_import: |n| format!("Импорт {n}"),
    tab_settings: "Настройки",
    close: "Закрыть",
    close_hint: "Закрыть (Esc)",

    review_empty_title: "Для обзора пока нет карточек",
    review_empty_text: "Старые и отложенные мысли появятся здесь позже.",
    review_done_title: "Обзор завершён",
    review_undo_hint: "Ctrl+Z — вернуть последнюю карточку",
    review_why_reminder: "Напоминание",
    review_why_import: "Импорт, ещё не открывал",
    review_why_old: "Давно не открывали",
    written_ago: |d| match d {
        ..=0 => "записано сегодня".to_owned(),
        n => format!("записано {n} {} назад", ru_days(n)),
    },
    review_position: |n, total| format!("{n} из {total}"),
    review_actions: ["← Оставить", "→ В архив", "↓ Позже", "↑ Закрепить", "G  Сделать целью", "Del  В корзину"],
    review_outcomes: ["оставлено", "в архиве", "отложено", "закреплено", "стали целями", "в корзине"],
    review_hint: "«Оставить» не спросит о карточке 8 недель, «Позже» — 2 недели · Ctrl+Z — отменить",
    invite_text: |cards, minutes| format!("Разобрать {cards} {} · ~{minutes} мин", ru_cards(cards)),

    with_undo_hint: |done| format!("{done} · Ctrl+Z — отменить"),
    import_apply_failed: "Не получилось, ничего не изменено",
    import_undone: |n| format!("Отменено: {}", (super::common::RU.notes)(n)),
    import_undo_failed: "Отменить не получилось",
    import_left: |n| format!("Из Sticky Notes осталось разобрать {}. Решение для группы — одной кнопкой, Ctrl+Z отменяет.", (super::common::RU.notes)(n)),
    import_undo_last: "Отменить последнее",
    import_sorted_title: "Импорт разобран",
    import_sorted_text: "Все заметки из Sticky Notes на своих местах. Их найдёт поиск.",
    import_groups: [
        "Похоже на пароли и доступы",
        "Ссылки",
        "Промпты",
        "Идеи",
        "Справка: команды, хосты, пути",
        "Давно не менялись",
        "Остальное",
    ],
    import_count_on_layer: |all, on_layer| format!("{all} · {on_layer} на слое"),
    import_hint_secrets: "Станут Private: значение зашифруется и пропадёт из поиска, останется подпись.",
    import_hint_old: "Не менялись больше года. В архиве их по-прежнему найдёт поиск.",
    import_hint_other: "Обычные заметки. Оставь как есть или убери в архив.",
    import_hint_kind: |kind| format!("Похоже на тип «{kind}»."),
    import_accept: |kind| format!("Принять: {kind}"),
    import_keep: "Оставить как есть",
    import_kept: |n| format!("{n} оставлены как есть"),
    import_archive_all: "Всё в архив",
    import_archived: |n| format!("{n} в архиве"),
    import_trash_all: "Всё в корзину",
    import_trashed: |n, days| format!("{n} в корзине, {} можно вернуть", (super::common::RU.days)(days)),
    import_collapse: "Свернуть",
    import_expand: "Посмотреть",
    import_kind_set: |kind| format!("Тип: {kind}"),
    import_item_age: |d| match d {
        ..=0 => "сегодня".to_owned(),
        d => format!("{d} {} назад", ru_days(d)),
    },

    search_trash: "Найти в корзине",
    search_archive: "Найти в архиве: текст, #тег, «ссылки прошлого года»",
    empty_trash: "Очистить корзину",
    empty_trash_confirm: "Точно очистить? Нажми ещё раз",
    trash_emptied: "Корзина очищена",
    trash_empty_failed: "Не удалось очистить корзину",
    count_in_trash: |total| format!("{total} в корзине"),
    count_in_archive: |total| format!("{total} в архиве"),
    found: |n, total| format!("найдено {n} из {total}"),
    trash_keeps: |days| format!("удалённое хранится {}", (super::common::RU.days)(days)),
    trash_is_empty: "Корзина пуста",
    archive_is_empty: "В архиве пусто. Сюда уходят карточки со слоя и импорт.",
    nothing_found: "Ничего не нашлось",
    list_hint_trash: "↑↓ выбор · Enter — восстановить · Delete — удалить навсегда · Ctrl+Tab — вкладка · Esc — закрыть",
    list_hint_archive: "↑↓ выбор · Enter — на слой · Delete — в корзину · Ctrl+Tab — вкладка · Esc — закрыть",
    returned_to_layer: "Возвращено на слой",
    moved_to_trash: "Перемещено в корзину",
    move_to_trash_failed: "Не удалось переместить в корзину",
    restored_to_archive: "Восстановлено в архив",
    restored_to_layer: "Восстановлено на слой",
    restore_failed: "Не удалось восстановить",
    purged: "Удалено навсегда",
    purge_failed: "Не удалось удалить",
    purge_confirm_button: "Точно? Ещё раз",
    purge_button: "Удалить навсегда",
    restore_button: "Восстановить",
    delete_button: "В корзину",
    to_layer_button: "На слой",
    trash_row_age: |deleted, left| {
        if deleted == 0 {
            format!("удалено сегодня · ещё {left} дн")
        } else {
            format!("удалено {deleted} дн назад · ещё {left} дн")
        }
    },
    was_in_archive: "был в архиве",
    was_on_layer: "был на слое",

    monitor_name: |n| format!("Монитор {n}"),
    monitor_primary: " · основной",
    bg_theme: "По теме",
    bg_taskbar: "Как панель задач",
    bg_custom: "Свой цвет",
    bg_custom_note: "Цвет текста подбирается сам: тёмный на светлом фоне, светлый на тёмном.",
    bg_taskbar_note: "Берётся из «Параметры → Персонализация → Цвета» и меняется вместе с Windows.",
    hotkey_press: "Нажмите сочетание…",
    hotkey_off: "выключено",
    hotkey_taken: |c| format!("занято: {c}"),
    hotkey_none_free: "нет свободного сочетания",
    hotkey_reset: "Сбросить",
    hotkey_reset_hint: "Вернуть стандартное сочетание",
    hotkey_turn_off: "Выключить",

    welcome_text: "Ebb кладёт заметки на отдельный экран — под окнами, как обои. Выберите, где им лежать; остальное можно не трогать.",
    welcome_note: "Настройки всегда открываются из меню значка Ebb в трее.",
    section_screen: "Экран слоя",
    screen_note: "Кликните по монитору — слой переедет туда сразу.",
    section_layer: "Слой",
    on_top: "Показывать заметки поверх окон",
    on_top_note: "Когда выключено, заметки лежат под окнами как обои. Включите, чтобы работать с ними прямо на экране.",
    esc_caption: "Esc и повторный клик в трее",
    esc_background: "Убрать слой на фон",
    esc_hide: "Скрыть слой",
    collapse_caption: "Свернуть слой к краю",
    collapse_top: "Сверху — чёлкой у края экрана",
    collapse_bottom: "Снизу — плашкой над панелью задач",
    collapse_note: "Ручка на краю слоя сворачивает его, и рабочий стол под ним снова доступен; клик по свёрнутому слою возвращает его.",
    section_appearance: "Оформление",
    theme: "Тема",
    card_looks: "Вид карточек",
    custom_style_note: "Свой вариант: настроен вручную.",
    customize: "Настроить вручную",
    marker_caption: "Цвет типа на карточке",
    intensity: "Интенсивность",
    strip_width: "Толщина полосы",
    corner_radius: "Скругление углов",
    shadow: "Тень",
    opacity: "Непрозрачность фона",
    text_font: "Шрифт текста",
    text_size: "Размер текста",
    icon_caption: "Метка типа",
    big_icon: "Крупный значок",
    card_background: "Фон карточек",
    appearance_note: "Слой меняется сразу. F4 на слое перебирает варианты цвета. Клик по метке типа на карточке меняет тип и цвет.",
    section_cards: "Карточки",
    snap: "Магнит: прилипать к краям соседних карточек",
    snap_note: "Удерживайте Alt при перетаскивании, чтобы поставить карточку свободно.",
    section_private: "Private",
    hide_from_capture: "Скрывать от записи экрана",
    hide_from_capture_note: "Пока секрет показан, слой не попадает в скриншоты, запись и демонстрацию экрана.",
    section_review: "Обзор",
    review_invite: "Приглашать к обзору раз в неделю",
    review_when: "Когда",
    review_invite_note: "На слое появится приглашение разобрать заметки. Через три дня оно уходит само; если разбирать почти нечего, не появляется.",
    section_startup: "Запуск",
    open_on_launch: "Открывать это окно при запуске Ebb из ярлыка",
    autostart: "Запускать при входе в Windows",
    autostart_note: "Через Планировщик заданий: стартует сразу после входа, без задержки автозагрузки.",
    autostart_other_copy: |command| format!("Запускается другая копия: {command}"),
    autostart_unknown: |e| format!("Не удалось узнать: {e}"),
    autostart_fix: "Исправить",
    autostart_fix_hint: "Перенастроить задание на эту копию",
    section_backdrop: "Внешний вид",
    backdrop_caption: "Фон",
    dimming: "Затемнение",
    section_hotkeys: "Горячие клавиши",
    hotkeys_note: "Нажмите на сочетание и введите новое: нужен Win или два из Ctrl, Alt, Shift; Esc отменяет. Пока сочетание не выбрано, берётся первое свободное из стандартных.",
    section_import: "Импорт",
    import_sticky: "Импорт из Sticky Notes…",
    sort_out_import: |n| format!("Разобрать импорт ({n})"),
    import_note: "Открытые заметки встанут на слой как на экране, остальные уйдут в архив. Повторный импорт заменит прошлый, изменённые тобой карточки останутся.",
    section_data: "Данные",
    data_counts: |layer, archived, trashed| format!("На слое {layer}, в архиве {archived}, в корзине {trashed}."),
    open_folder: "Открыть папку",
    last_backup: |date| format!("Последняя копия: {date}"),
    no_backup_yet: "Последняя копия: ещё не делалась",
    backup_note: "Раз в неделю, хранятся 4 последние. Значения Private в копии читаются только под этим профилем Windows.",
    backup_now: "Сделать копию сейчас",
    show_backups: "Показать копии",
    backup_saved: "Копия сохранена",
    backup_failed: |e| format!("Не удалось: {e}"),
    backup_busy: "Копирую…",

    sticky_title: "Импорт из Sticky Notes",
    sticky_not_found: "Sticky Notes не найдены на этом компьютере.",
    sticky_nothing_new: |already, empty, duplicates| {
        format!("Нечего импортировать: {already} изменены в Ebb и оставлены как есть, пустых {empty}, дубликатов {duplicates}.")
    },
    sticky_read_failed: |e| format!("Не удалось прочитать Sticky Notes: {e}"),
    sticky_import_failed: |e| format!("Импорт не удался: {e}"),
    sticky_undo_failed: |e| format!("Отмена не удалась: {e}"),
    sticky_found: |importable, total| format!("Нашлось {} для импорта из {total}.", (super::common::RU.notes)(importable)),
    sticky_to_layer: |open, in_place, nearby| {
        format!("На слой: {open} открытых, как на экране ({in_place} на свои места, {nearby} рядом).")
    },
    sticky_to_archive: |archived, old| format!("В архив: {archived} (давно не менялись: {old}). Их найдёт поиск."),
    sticky_colors: |n| format!("Цвета заметок сохраню: {n}."),
    sticky_skip: |empty, duplicates| format!("Пропущу: пустых {empty}, дубликатов {duplicates}."),
    sticky_previous: |replaced, kept| format!("Прошлый импорт: заменю {replaced}, оставлю изменённые тобой ({kept})."),
    sticky_readonly: "Sticky Notes не меняются: читается копия базы.",
    sticky_import: "Импортировать",
    sticky_not_now: "Не сейчас",
    sticky_never: "Не предлагать",
    sticky_done_title: "Sticky Notes импортированы",
    sticky_done_counts: |importable, on_layer, archived| {
        format!("{}: {on_layer} на слое, {archived} в архиве.", (super::common::RU.notes)(importable))
    },
    sticky_replaced: |n| format!("Заменено из прошлого импорта: {n}."),
    sticky_originals: "Оригиналы в Sticky Notes остались на месте.",
    sticky_sort_out: "Разобрать",
    sticky_sort_out_hint: "Группы: пароли, ссылки, промпты, идеи… — решение одной кнопкой",
    sticky_done_button: "Готово",
    sticky_undo: "Отменить импорт",
};

pub const EN: Library = Library {
    language: "Language",
    language_system: "Same as Windows",

    hotkey_needs_modifiers: "Needs Win or two of Ctrl, Alt, Shift",
    hotkey_key_unavailable: "This key can't be used",

    title_library: "Library",
    title_settings: "Ebb settings",
    title_welcome: "Welcome to Ebb",
    tab_archive: |n| format!("Archive {n}"),
    tab_trash: |n| format!("Trash {n}"),
    tab_review: "Review",
    tab_import: |n| format!("Import {n}"),
    tab_settings: "Settings",
    close: "Close",
    close_hint: "Close (Esc)",

    review_empty_title: "No cards to review yet",
    review_empty_text: "Old and snoozed thoughts will show up here later.",
    review_done_title: "Review finished",
    review_undo_hint: "Ctrl+Z — bring back the last card",
    review_why_reminder: "Reminder",
    review_why_import: "Imported, never opened",
    review_why_old: "Not opened in a long time",
    written_ago: |d| match d {
        ..=0 => "written today".to_owned(),
        n => format!("written {n} {} ago", en_days(n)),
    },
    review_position: |n, total| format!("{n} of {total}"),
    review_actions: ["← Keep", "→ Archive", "↓ Later", "↑ Pin", "G  Make a goal", "Del  Delete"],
    review_outcomes: ["kept", "archived", "snoozed", "pinned", "made goals", "deleted"],
    review_hint: "“Keep” won't ask about a card for 8 weeks, “Later” for 2 weeks · Ctrl+Z — undo",
    invite_text: |cards, minutes| format!("Sort out {cards} {} · ~{minutes} min", en_notes(cards)),

    with_undo_hint: |done| format!("{done} · Ctrl+Z — undo"),
    import_apply_failed: "Couldn't do it, nothing was changed",
    import_undone: |n| format!("Undone: {}", (super::common::EN.notes)(n)),
    import_undo_failed: "Couldn't undo",
    import_left: |n| format!("{} from Sticky Notes left to sort out. One button decides for a whole group, Ctrl+Z undoes it.", (super::common::EN.notes)(n)),
    import_undo_last: "Undo the last one",
    import_sorted_title: "Import sorted out",
    import_sorted_text: "All the notes from Sticky Notes are in their places. Search will find them.",
    import_groups: [
        "Look like passwords and credentials",
        "Links",
        "Prompts",
        "Ideas",
        "Reference: commands, hosts, paths",
        "Not changed for a long time",
        "Everything else",
    ],
    import_count_on_layer: |all, on_layer| format!("{all} · {on_layer} on the layer"),
    import_hint_secrets: "Will become Private: the value is encrypted and leaves search, the label stays.",
    import_hint_old: "Not changed for over a year. Search still finds them in the archive.",
    import_hint_other: "Ordinary notes. Keep them as they are or move them to the archive.",
    import_hint_kind: |kind| format!("Looks like the “{kind}” kind."),
    import_accept: |kind| format!("Accept: {kind}"),
    import_keep: "Keep as they are",
    import_kept: |n| format!("{n} kept as they are"),
    import_archive_all: "Archive all",
    import_archived: |n| format!("{n} archived"),
    import_trash_all: "Delete all",
    import_trashed: |n, days| format!("{n} moved to the trash, can be restored for {}", (super::common::EN.days)(days)),
    import_collapse: "Collapse",
    import_expand: "View",
    import_kind_set: |kind| format!("Kind: {kind}"),
    import_item_age: |d| match d {
        ..=0 => "today".to_owned(),
        d => format!("{d} {} ago", en_days(d)),
    },

    search_trash: "Search the trash",
    search_archive: "Search the archive: text, #tag, “links from last year”",
    empty_trash: "Empty trash",
    empty_trash_confirm: "Really empty it? Click again",
    trash_emptied: "Trash emptied",
    trash_empty_failed: "Couldn't empty the trash",
    count_in_trash: |total| format!("{total} in the trash"),
    count_in_archive: |total| format!("{total} in the archive"),
    found: |n, total| format!("found {n} of {total}"),
    trash_keeps: |days| format!("deleted notes are kept for {}", (super::common::EN.days)(days)),
    trash_is_empty: "The trash is empty",
    archive_is_empty: "The archive is empty. Cards from the layer and imports end up here.",
    nothing_found: "Nothing found",
    list_hint_trash: "↑↓ select · Enter — restore · Delete — delete forever · Ctrl+Tab — tab · Esc — close",
    list_hint_archive: "↑↓ select · Enter — to the layer · Delete — to the trash · Ctrl+Tab — tab · Esc — close",
    returned_to_layer: "Returned to the layer",
    moved_to_trash: "Moved to the trash",
    move_to_trash_failed: "Couldn't move to the trash",
    restored_to_archive: "Restored to the archive",
    restored_to_layer: "Restored to the layer",
    restore_failed: "Couldn't restore",
    purged: "Deleted forever",
    purge_failed: "Couldn't delete",
    purge_confirm_button: "Sure? Click again",
    purge_button: "Delete forever",
    restore_button: "Restore",
    delete_button: "Delete",
    to_layer_button: "To the layer",
    trash_row_age: |deleted, left| {
        if deleted == 0 {
            format!("deleted today · {left} d left")
        } else {
            format!("deleted {deleted} d ago · {left} d left")
        }
    },
    was_in_archive: "was in the archive",
    was_on_layer: "was on the layer",

    monitor_name: |n| format!("Monitor {n}"),
    monitor_primary: " · primary",
    bg_theme: "Match theme",
    bg_taskbar: "Like the taskbar",
    bg_custom: "Custom color",
    bg_custom_note: "The text color is picked automatically: dark on a light background, light on a dark one.",
    bg_taskbar_note: "Taken from Settings → Personalization → Colors and changes along with Windows.",
    hotkey_press: "Press a shortcut…",
    hotkey_off: "off",
    hotkey_taken: |c| format!("taken: {c}"),
    hotkey_none_free: "no free shortcut",
    hotkey_reset: "Reset",
    hotkey_reset_hint: "Restore the default shortcut",
    hotkey_turn_off: "Turn off",

    welcome_text: "Ebb puts your notes on a separate screen, under your windows like wallpaper. Choose where they should live; you can leave the rest alone.",
    welcome_note: "Settings always open from the Ebb tray icon menu.",
    section_screen: "Layer screen",
    screen_note: "Click a monitor and the layer moves there right away.",
    section_layer: "Layer",
    on_top: "Show notes on top of windows",
    on_top_note: "When off, notes sit under windows like wallpaper. Turn it on to work with them right on the screen.",
    esc_caption: "Esc and a second click on the tray icon",
    esc_background: "Send the layer to the back",
    esc_hide: "Hide the layer",
    collapse_caption: "Collapse the layer to an edge",
    collapse_top: "Top — a notch at the screen edge",
    collapse_bottom: "Bottom — a tab above the taskbar",
    collapse_note: "The handle on the layer's edge collapses it and frees the desktop beneath; clicking the collapsed layer brings it back.",
    section_appearance: "Appearance",
    theme: "Theme",
    card_looks: "Card style",
    custom_style_note: "Custom style: set up by hand.",
    customize: "Customize",
    marker_caption: "Kind color on the card",
    intensity: "Intensity",
    strip_width: "Strip width",
    corner_radius: "Corner radius",
    shadow: "Shadow",
    opacity: "Background opacity",
    text_font: "Text font",
    text_size: "Text size",
    icon_caption: "Kind mark",
    big_icon: "Large icon",
    card_background: "Card background",
    appearance_note: "The layer updates right away. F4 on the layer cycles through colors. Clicking the kind mark on a card changes its kind and color.",
    section_cards: "Cards",
    snap: "Snap to the edges of neighboring cards",
    snap_note: "Hold Alt while dragging to place a card freely.",
    section_private: "Private",
    hide_from_capture: "Hide from screen recording",
    hide_from_capture_note: "While a secret is shown, the layer stays out of screenshots, recordings and screen sharing.",
    section_review: "Review",
    review_invite: "Invite me to a review once a week",
    review_when: "When",
    review_invite_note: "An invitation to sort out notes appears on the layer. It goes away on its own after three days; it doesn't appear if there's hardly anything to sort out.",
    section_startup: "Startup",
    open_on_launch: "Open this window when Ebb is launched from a shortcut",
    autostart: "Start when I sign in to Windows",
    autostart_note: "Via Task Scheduler: starts right after sign-in, without the startup delay.",
    autostart_other_copy: |command| format!("Another copy starts: {command}"),
    autostart_unknown: |e| format!("Couldn't check: {e}"),
    autostart_fix: "Fix",
    autostart_fix_hint: "Point the task at this copy",
    section_backdrop: "Backdrop",
    backdrop_caption: "Background",
    dimming: "Dimming",
    section_hotkeys: "Hotkeys",
    hotkeys_note: "Click a shortcut and type a new one: it needs Win or two of Ctrl, Alt, Shift; Esc cancels. Until you pick one, the first free default is used.",
    section_import: "Import",
    import_sticky: "Import from Sticky Notes…",
    sort_out_import: |n| format!("Sort out the import ({n})"),
    import_note: "Open notes will land on the layer as they are on screen, the rest go to the archive. A repeat import replaces the previous one; cards you've edited stay.",
    section_data: "Data",
    data_counts: |layer, archived, trashed| format!("{layer} on the layer, {archived} in the archive, {trashed} in the trash."),
    open_folder: "Open folder",
    last_backup: |date| format!("Last backup: {date}"),
    no_backup_yet: "Last backup: none yet",
    backup_note: "Weekly; the last 4 are kept. Private values in a backup can only be read under this Windows profile.",
    backup_now: "Back up now",
    show_backups: "Show backups",
    backup_saved: "Backup saved",
    backup_failed: |e| format!("Failed: {e}"),
    backup_busy: "Copying…",

    sticky_title: "Import from Sticky Notes",
    sticky_not_found: "Sticky Notes wasn't found on this computer.",
    sticky_nothing_new: |already, empty, duplicates| {
        format!("Nothing to import: {already} edited in Ebb and kept as they are, {empty} empty, {duplicates} duplicates.")
    },
    sticky_read_failed: |e| format!("Couldn't read Sticky Notes: {e}"),
    sticky_import_failed: |e| format!("Import failed: {e}"),
    sticky_undo_failed: |e| format!("Undo failed: {e}"),
    sticky_found: |importable, total| format!("Found {} to import out of {total}.", (super::common::EN.notes)(importable)),
    sticky_to_layer: |open, in_place, nearby| {
        format!("To the layer: {open} open ones, as on screen ({in_place} in place, {nearby} nearby).")
    },
    sticky_to_archive: |archived, old| format!("To the archive: {archived} (not changed for a long time: {old}). Search will find them."),
    sticky_colors: |n| format!("Note colors will be kept: {n}."),
    sticky_skip: |empty, duplicates| format!("Will skip: {empty} empty, {duplicates} duplicates."),
    sticky_previous: |replaced, kept| format!("Previous import: will replace {replaced}, will keep the ones you edited ({kept})."),
    sticky_readonly: "Sticky Notes stays untouched: a copy of its database is read.",
    sticky_import: "Import",
    sticky_not_now: "Not now",
    sticky_never: "Don't offer again",
    sticky_done_title: "Sticky Notes imported",
    sticky_done_counts: |importable, on_layer, archived| {
        format!("{}: {on_layer} on the layer, {archived} in the archive.", (super::common::EN.notes)(importable))
    },
    sticky_replaced: |n| format!("Replaced from the previous import: {n}."),
    sticky_originals: "The originals in Sticky Notes were left in place.",
    sticky_sort_out: "Sort out",
    sticky_sort_out_hint: "Groups: passwords, links, prompts, ideas… — one button decides for a group",
    sticky_done_button: "Done",
    sticky_undo: "Undo the import",
};

#[cfg(test)]
mod tests {
    use crate::i18n::{Lang, strings};

    #[test]
    fn english_counts_and_plurals() {
        let en = &strings(Lang::En).library;
        assert_eq!((en.tab_archive)(5), "Archive 5");
        assert_eq!((en.tab_trash)(0), "Trash 0");
        assert_eq!((en.tab_import)(12), "Import 12");
        assert_eq!((en.review_position)(3, 15), "3 of 15");
        assert_eq!((en.count_in_trash)(1), "1 in the trash");
        assert_eq!((en.found)(2, 10), "found 2 of 10");
        assert_eq!((en.sort_out_import)(4), "Sort out the import (4)");
        assert_eq!((en.data_counts)(1, 2, 3), "1 on the layer, 2 in the archive, 3 in the trash.");
        assert_eq!((en.monitor_name)(2), "Monitor 2");
        let notes = |n| (en.import_undone)(n);
        assert_eq!([notes(1), notes(2), notes(5), notes(21)], ["Undone: 1 note", "Undone: 2 notes", "Undone: 5 notes", "Undone: 21 notes"]);
        let left = |n| (en.import_left)(n);
        assert_eq!(left(1), "1 note from Sticky Notes left to sort out. One button decides for a whole group, Ctrl+Z undoes it.");
        assert_eq!((en.sticky_found)(21, 30), "Found 21 notes to import out of 30.");
        assert_eq!((en.sticky_found)(1, 1), "Found 1 note to import out of 1.");
        assert_eq!((en.sticky_done_counts)(2, 1, 1), "2 notes: 1 on the layer, 1 in the archive.");
    }

    #[test]
    fn english_ages_and_countdowns() {
        let en = &strings(Lang::En).library;
        assert_eq!([(en.written_ago)(0), (en.written_ago)(1), (en.written_ago)(5), (en.written_ago)(21)], [
            "written today",
            "written 1 day ago",
            "written 5 days ago",
            "written 21 days ago"
        ]);
        assert_eq!([(en.import_item_age)(0), (en.import_item_age)(1), (en.import_item_age)(23)], [
            "today",
            "1 day ago",
            "23 days ago"
        ]);
        assert_eq!((en.trash_row_age)(0, 30), "deleted today · 30 d left");
        assert_eq!((en.trash_row_age)(3, 27), "deleted 3 d ago · 27 d left");
        assert_eq!((en.trash_keeps)(30), "deleted notes are kept for 30 days");
        assert_eq!((en.trash_keeps)(1), "deleted notes are kept for 1 day");
        assert_eq!((en.import_trashed)("5 notes", 30), "5 notes moved to the trash, can be restored for 30 days");
    }

    #[test]
    fn english_invite_text() {
        let en = &strings(Lang::En).library;
        let invite = |cards, minutes| (en.invite_text)(cards, minutes);
        assert_eq!(invite(1, 1), "Sort out 1 note · ~1 min");
        assert_eq!(invite(2, 1), "Sort out 2 notes · ~1 min");
        assert_eq!(invite(5, 2), "Sort out 5 notes · ~2 min");
        assert_eq!(invite(21, 6), "Sort out 21 notes · ~6 min");
    }

    #[test]
    fn english_import_phrases() {
        let en = &strings(Lang::En).library;
        assert_eq!((en.with_undo_hint)("5 notes archived"), "5 notes archived · Ctrl+Z — undo");
        assert_eq!((en.import_count_on_layer)(7, 3), "7 · 3 on the layer");
        assert_eq!((en.import_hint_kind)("Link"), "Looks like the “Link” kind.");
        assert_eq!((en.sticky_to_layer)(10, 7, 3), "To the layer: 10 open ones, as on screen (7 in place, 3 nearby).");
        assert_eq!((en.sticky_skip)(1, 2), "Will skip: 1 empty, 2 duplicates.");
        assert_eq!(en.import_groups.len(), crate::sticky::ImportGroup::ALL.len());
        assert_eq!(en.review_actions.len(), en.review_outcomes.len());
    }

    #[test]
    fn russian_phrases_keep_their_wording() {
        let ru = &strings(Lang::Ru).library;
        assert_eq!((ru.invite_text)(21, 6), "Разобрать 21 заметку · ~6 мин");
        assert_eq!((ru.written_ago)(0), "записано сегодня");
        assert_eq!((ru.written_ago)(5), "записано 5 дней назад");
        assert_eq!((ru.import_item_age)(22), "22 дня назад");
        assert_eq!((ru.trash_row_age)(3, 27), "удалено 3 дн назад · ещё 27 дн");
        assert_eq!((ru.import_undone)(11), "Отменено: 11 заметок");
        assert_eq!((ru.trash_keeps)(30), "удалённое хранится 30 дней");
    }
}
