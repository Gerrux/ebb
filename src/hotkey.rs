//! Global hotkeys as data: which actions have one, the key combinations, their
//! text form in the settings table, and the defaults. Pure logic with no Win32
//! calls; registering the combinations is the shell thread's job (`shell`).

use std::fmt;

use egui::Key;

/// What a hotkey does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Capture,
    Search,
    Library,
    Layer,
}

impl Action {
    pub const ALL: [Action; 4] = [Action::Capture, Action::Search, Action::Library, Action::Layer];

    pub fn index(self) -> usize {
        self as usize
    }

    /// The settings key: a combo, an empty string for "off", missing for "default".
    pub fn setting_key(self) -> &'static str {
        match self {
            Action::Capture => "hotkey.capture",
            Action::Search => "hotkey.search",
            Action::Library => "hotkey.library",
            Action::Layer => "hotkey.layer",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Action::Capture => "Записать мысль",
            Action::Search => "Поиск",
            Action::Library => "Архив и корзина",
            Action::Layer => "Показать / скрыть слой",
        }
    }

    /// Combinations tried in order when the action is on its default. The first
    /// free one wins. Ctrl+Alt+Space is often taken (PowerToys), Ctrl+Space and
    /// Ctrl+Shift+Space collide with IDE completion; likewise for search. The
    /// layer has no default: it is off until the user picks a combination.
    pub fn defaults(self) -> &'static [Combo] {
        const CAPTURE: [Combo; 3] = [
            Combo::new(true, false, true, false, b'N' as u32),
            Combo::new(false, true, true, false, b'N' as u32),
            Combo::new(false, true, true, false, VK_SPACE),
        ];
        const SEARCH: [Combo; 2] = [
            Combo::new(true, false, true, false, b'F' as u32),
            Combo::new(false, true, true, false, b'F' as u32),
        ];
        const LIBRARY: [Combo; 1] = [Combo::new(true, false, true, false, b'L' as u32)];
        match self {
            Action::Capture => &CAPTURE,
            Action::Search => &SEARCH,
            Action::Library => &LIBRARY,
            Action::Layer => &[],
        }
    }
}

/// Number of ids reserved per action in `WM_HOTKEY`: more than any candidate list.
const ID_STRIDE: i32 = 100;

/// `WM_HOTKEY` id of `action`'s `candidate`-th combination (0 for a custom one).
pub fn hotkey_id(action: Action, candidate: usize) -> i32 {
    (action.index() as i32 + 1) * ID_STRIDE + candidate as i32
}

/// The action a `WM_HOTKEY` id belongs to.
pub fn action_of_id(id: i32) -> Option<Action> {
    let index = usize::try_from(id / ID_STRIDE - 1).ok()?;
    Action::ALL.get(index).copied()
}

// Virtual-key codes (winuser.h); kept here so the module stays free of Win32.
const VK_TAB: u32 = 0x09;
const VK_RETURN: u32 = 0x0D;
const VK_SPACE: u32 = 0x20;
const VK_PRIOR: u32 = 0x21;
const VK_NEXT: u32 = 0x22;
const VK_END: u32 = 0x23;
const VK_HOME: u32 = 0x24;
const VK_LEFT: u32 = 0x25;
const VK_UP: u32 = 0x26;
const VK_RIGHT: u32 = 0x27;
const VK_DOWN: u32 = 0x28;
pub const VK_INSERT: u32 = 0x2D;
pub const VK_DELETE: u32 = 0x2E;
const VK_F1: u32 = 0x70;
const VK_OEM_1: u32 = 0xBA; // ;
const VK_OEM_PLUS: u32 = 0xBB; // =
const VK_OEM_COMMA: u32 = 0xBC;
const VK_OEM_MINUS: u32 = 0xBD;
const VK_OEM_PERIOD: u32 = 0xBE;
const VK_OEM_2: u32 = 0xBF; // /
const VK_OEM_3: u32 = 0xC0; // `
const VK_OEM_4: u32 = 0xDB; // [
const VK_OEM_5: u32 = 0xDC; // \
const VK_OEM_6: u32 = 0xDD; // ]
const VK_OEM_7: u32 = 0xDE; // '

/// Keys with a name or a symbol rather than a letter, digit or F-number.
const NAMED_KEYS: &[(&str, u32)] = &[
    ("Space", VK_SPACE),
    ("Enter", VK_RETURN),
    ("Tab", VK_TAB),
    ("Left", VK_LEFT),
    ("Right", VK_RIGHT),
    ("Up", VK_UP),
    ("Down", VK_DOWN),
    ("Home", VK_HOME),
    ("End", VK_END),
    ("PageUp", VK_PRIOR),
    ("PageDown", VK_NEXT),
    ("Insert", VK_INSERT),
    ("Delete", VK_DELETE),
    (",", VK_OEM_COMMA),
    (".", VK_OEM_PERIOD),
    ("/", VK_OEM_2),
    (";", VK_OEM_1),
    ("[", VK_OEM_4),
    ("]", VK_OEM_6),
    ("-", VK_OEM_MINUS),
    ("=", VK_OEM_PLUS),
    ("`", VK_OEM_3),
    ("\\", VK_OEM_5),
    ("'", VK_OEM_7),
];

/// "N" -> the virtual-key code, case-insensitive; `None` for an unknown name.
fn vk_from_name(name: &str) -> Option<u32> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_ascii_alphabetic() {
            return Some(c.to_ascii_uppercase() as u32);
        }
        if c.is_ascii_digit() {
            return Some(c as u32);
        }
    }
    if let Some(n) = name.strip_prefix(['F', 'f']).and_then(|n| n.parse::<u32>().ok()) {
        // "F+1" and "F01" would parse too; only the canonical spelling counts.
        if (1..=24).contains(&n) && name[1..] == n.to_string() {
            return Some(VK_F1 + n - 1);
        }
    }
    NAMED_KEYS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, vk)| *vk)
}

/// The canonical name of a virtual-key code; `None` for one Ebb doesn't offer.
fn name_of_vk(vk: u32) -> Option<String> {
    match vk {
        0x41..=0x5A | 0x30..=0x39 => char::from_u32(vk).map(String::from),
        0x70..=0x87 => Some(format!("F{}", vk - VK_F1 + 1)),
        _ => NAMED_KEYS.iter().find(|(_, v)| *v == vk).map(|(n, _)| (*n).to_owned()),
    }
}

/// `RegisterHotKey` modifier bits (`MOD_*`).
pub const MOD_ALT: u32 = 0x1;
pub const MOD_CONTROL: u32 = 0x2;
pub const MOD_SHIFT: u32 = 0x4;
pub const MOD_WIN: u32 = 0x8;

/// A key with modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Combo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// Virtual-key code of the non-modifier key.
    pub vk: u32,
}

impl Combo {
    pub const fn new(win: bool, ctrl: bool, alt: bool, shift: bool, vk: u32) -> Self {
        Self { ctrl, alt, shift, win, vk }
    }

    /// Reads "Win+Alt+N"; modifiers and key names are case-insensitive.
    pub fn parse(text: &str) -> Option<Combo> {
        let mut parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let key = parts.pop()?;
        let mut combo = Combo::new(false, false, false, false, vk_from_name(key)?);
        for part in parts {
            let flag = match part.to_ascii_lowercase().as_str() {
                "win" => &mut combo.win,
                "ctrl" | "control" => &mut combo.ctrl,
                "alt" => &mut combo.alt,
                "shift" => &mut combo.shift,
                _ => return None,
            };
            *flag = true;
        }
        Some(combo)
    }

    /// Usable as a global hotkey: Win, or two of Ctrl, Alt and Shift. One of
    /// them alone would take Ctrl+C, Alt+F4 or a capital letter from every program.
    pub fn valid(&self) -> bool {
        let others = [self.ctrl, self.alt, self.shift].into_iter().filter(|on| *on).count();
        (self.win || others >= 2) && name_of_vk(self.vk).is_some()
    }

    /// `RegisterHotKey` modifier bits.
    pub fn modifiers(&self) -> u32 {
        [(self.alt, MOD_ALT), (self.ctrl, MOD_CONTROL), (self.shift, MOD_SHIFT), (self.win, MOD_WIN)]
            .into_iter()
            .filter(|(on, _)| *on)
            .fold(0, |bits, (_, bit)| bits | bit)
    }

    /// What a key press means as a hotkey; `None` for a key Ebb doesn't offer.
    /// The Win key isn't in egui's modifiers, so the caller reads it itself.
    pub fn from_egui(key: Key, ctrl: bool, alt: bool, shift: bool, win: bool) -> Option<Combo> {
        let name = egui_key_name(key)?;
        Some(Combo::new(win, ctrl, alt, shift, vk_from_name(name)?))
    }
}

/// The key behind a clipboard event: egui-winit turns Ctrl+C/X/V (with any other
/// modifiers), Ctrl+Insert, Shift+Insert and Shift+Delete into `Copy`, `Cut` and
/// `Paste` instead of key events. `insert_down`/`delete_down` tell the Insert and
/// Delete spellings apart. `None` for any other event.
pub fn clipboard_key(event: &egui::Event, insert_down: bool, delete_down: bool) -> Option<Key> {
    match event {
        egui::Event::Copy => Some(if insert_down { Key::Insert } else { Key::C }),
        egui::Event::Paste(_) => Some(if insert_down { Key::Insert } else { Key::V }),
        egui::Event::Cut => Some(if delete_down { Key::Delete } else { Key::X }),
        _ => None,
    }
}

/// Canonical name for an egui key (digits from the numpad arrive as the same
/// keys as the main row).
fn egui_key_name(key: Key) -> Option<&'static str> {
    Some(match key {
        Key::Space => "Space",
        Key::Enter => "Enter",
        Key::Tab => "Tab",
        Key::ArrowLeft => "Left",
        Key::ArrowRight => "Right",
        Key::ArrowUp => "Up",
        Key::ArrowDown => "Down",
        Key::Home => "Home",
        Key::End => "End",
        Key::PageUp => "PageUp",
        Key::PageDown => "PageDown",
        Key::Insert => "Insert",
        Key::Delete => "Delete",
        Key::Comma => ",",
        Key::Period => ".",
        Key::Slash => "/",
        Key::Semicolon => ";",
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::Minus => "-",
        Key::Equals => "=",
        Key::Backtick => "`",
        Key::Backslash => "\\",
        Key::Quote => "'",
        // Letters, digits and F-keys are named the way `vk_from_name` reads them.
        other => {
            let name = other.name();
            let single = name.len() == 1 && name.as_bytes()[0].is_ascii_alphanumeric();
            let f_key = name.strip_prefix('F').is_some_and(|n| n.parse::<u32>().is_ok());
            if single || f_key {
                name
            } else {
                return None;
            }
        }
    })
}

impl fmt::Display for Combo {
    /// Canonical order: Win+Ctrl+Alt+Shift+Key.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [(self.win, "Win"), (self.ctrl, "Ctrl"), (self.alt, "Alt"), (self.shift, "Shift")] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        match name_of_vk(self.vk) {
            Some(name) => f.write_str(&name),
            None => write!(f, "0x{:02X}", self.vk),
        }
    }
}

/// What the settings ask of one action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wanted {
    /// The action's default candidates, first free one.
    Default,
    Off,
    Custom(Combo),
}

impl Wanted {
    /// The settings value: `None` = no row (default), `Some("")` = off.
    pub fn setting_value(self) -> Option<String> {
        match self {
            Wanted::Default => None,
            Wanted::Off => Some(String::new()),
            Wanted::Custom(c) => Some(c.to_string()),
        }
    }
}

/// Reads a stored setting: missing or unreadable means the default.
pub fn wanted(setting: Option<&str>) -> Wanted {
    match setting {
        None => Wanted::Default,
        Some(s) if s.trim().is_empty() => Wanted::Off,
        Some(s) => Combo::parse(s).filter(Combo::valid).map_or(Wanted::Default, Wanted::Custom),
    }
}

/// Every action's wish, indexed by [`Action::index`].
pub type Config = [Wanted; 4];

/// The order to bind actions in: chosen combinations first, so a default's
/// fallback candidate can't take one the user gave another action (say, when
/// the first candidate is held by another program at startup).
pub fn registration_order(config: &Config) -> [Action; 4] {
    let mut order = Action::ALL;
    order.sort_by_key(|a| !matches!(config[a.index()], Wanted::Custom(_)));
    order
}

/// Reads all four settings through `read` (one lookup per action).
pub fn load(read: impl Fn(&str) -> Option<String>) -> Config {
    Action::ALL.map(|a| wanted(read(a.setting_key()).as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn combo(text: &str) -> Combo {
        Combo::parse(text).unwrap()
    }

    #[test]
    fn default_names_round_trip() {
        for name in ["Win+Alt+N", "Ctrl+Alt+N", "Ctrl+Alt+Space", "Win+Alt+F", "Ctrl+Alt+F", "Win+Alt+L"] {
            assert_eq!(combo(name).to_string(), name);
        }
    }

    #[test]
    fn defaults_are_the_documented_lists() {
        let names = |a: Action| a.defaults().iter().map(Combo::to_string).collect::<Vec<_>>();
        assert_eq!(names(Action::Capture), ["Win+Alt+N", "Ctrl+Alt+N", "Ctrl+Alt+Space"]);
        assert_eq!(names(Action::Search), ["Win+Alt+F", "Ctrl+Alt+F"]);
        assert_eq!(names(Action::Library), ["Win+Alt+L"]);
        assert!(names(Action::Layer).is_empty());
        assert!(Action::ALL.iter().flat_map(|a| a.defaults()).all(Combo::valid));
    }

    #[test]
    fn display_uses_the_canonical_modifier_order() {
        assert_eq!(combo("shift+alt+ctrl+win+k").to_string(), "Win+Ctrl+Alt+Shift+K");
        assert_eq!(combo(" ctrl + alt + f12 ").to_string(), "Ctrl+Alt+F12");
        assert_eq!(combo("CONTROL+space").to_string(), "Ctrl+Space");
    }

    #[test]
    fn parse_rejects_garbage() {
        for text in ["", "+", "Ctrl+", "Ctrl+Alt", "Ctrl+Foo+N", "Ctrl+F25", "Ctrl+F0", "Ctrl+F01", "Ctrl+NN", "Ctrl+Esc", "Ctrl++N"] {
            assert_eq!(Combo::parse(text), None, "{text}");
        }
    }

    #[test]
    fn every_offered_key_round_trips() {
        let mut names: Vec<String> = ('A'..='Z').chain('0'..='9').map(String::from).collect();
        names.extend((1..=24).map(|n| format!("F{n}")));
        names.extend(NAMED_KEYS.iter().map(|(n, _)| (*n).to_owned()));
        for name in names {
            let c = combo(&format!("Ctrl+Alt+{name}"));
            assert_eq!(c.to_string(), format!("Ctrl+Alt+{name}"));
            assert!(c.valid());
        }
    }

    #[test]
    fn virtual_key_codes() {
        assert_eq!(combo("Ctrl+A").vk, 0x41);
        assert_eq!(combo("Ctrl+0").vk, 0x30);
        assert_eq!(combo("Ctrl+F1").vk, 0x70);
        assert_eq!(combo("Ctrl+F24").vk, 0x87);
        assert_eq!(combo("Ctrl+,").vk, 0xBC);
        assert_eq!(combo("Ctrl+=").vk, 0xBB);
        assert_eq!(combo("Ctrl+PageDown").vk, 0x22);
    }

    #[test]
    fn modifier_bits() {
        assert_eq!(combo("Win+Alt+N").modifiers(), MOD_WIN | MOD_ALT);
        assert_eq!(combo("Ctrl+Shift+N").modifiers(), MOD_CONTROL | MOD_SHIFT);
    }

    #[test]
    fn validity() {
        assert!(combo("Win+N").valid());
        assert!(combo("Ctrl+Alt+N").valid());
        assert!(combo("Ctrl+Shift+N").valid());
        assert!(combo("Alt+Shift+N").valid());
        // One of these alone is some program's own shortcut.
        assert!(!combo("Ctrl+C").valid());
        assert!(!combo("Alt+F4").valid());
        assert!(!combo("Shift+N").valid());
        assert!(!combo("N").valid());
        assert!(!Combo::new(false, true, true, false, 0x1B).valid());
    }

    #[test]
    fn egui_keys_map_to_virtual_keys() {
        let map = |k| Combo::from_egui(k, true, false, false, false);
        assert_eq!(map(Key::A).map(|c| c.vk), Some(0x41));
        assert_eq!(map(Key::Num7).map(|c| c.vk), Some(0x37));
        assert_eq!(map(Key::F5).map(|c| c.vk), Some(0x74));
        assert_eq!(map(Key::Space).map(|c| c.vk), Some(VK_SPACE));
        assert_eq!(map(Key::Comma).map(|c| c.vk), Some(VK_OEM_COMMA));
        assert_eq!(map(Key::Backtick).map(|c| c.vk), Some(VK_OEM_3));
        assert_eq!(map(Key::Escape), None);
        assert_eq!(map(Key::Backspace), None);
        let c = Combo::from_egui(Key::N, true, true, true, true).unwrap();
        assert_eq!(c.to_string(), "Win+Ctrl+Alt+Shift+N");
    }

    #[test]
    fn every_egui_key_that_maps_is_offered_by_name() {
        let mapped: Vec<_> = Key::ALL.iter().filter_map(|k| Combo::from_egui(*k, true, true, false, false)).collect();
        assert!(mapped.len() >= 36 + 24 + 13);
        for c in mapped {
            assert!(c.valid(), "{c}");
            assert_eq!(Combo::parse(&c.to_string()), Some(c));
        }
    }

    #[test]
    fn wanted_reads_settings() {
        assert_eq!(wanted(None), Wanted::Default);
        assert_eq!(wanted(Some("")), Wanted::Off);
        assert_eq!(wanted(Some("  ")), Wanted::Off);
        assert_eq!(wanted(Some("Win+Alt+N")), Wanted::Custom(combo("Win+Alt+N")));
        assert_eq!(wanted(Some("nonsense")), Wanted::Default);
        // Parses, but would grab a bare key from every program.
        assert_eq!(wanted(Some("Shift+N")), Wanted::Default);
        assert_eq!(wanted(Some("N")), Wanted::Default);
    }

    #[test]
    fn wanted_writes_settings() {
        assert_eq!(Wanted::Default.setting_value(), None);
        assert_eq!(Wanted::Off.setting_value().as_deref(), Some(""));
        let custom = Wanted::Custom(combo("ctrl+alt+k"));
        assert_eq!(custom.setting_value().as_deref(), Some("Ctrl+Alt+K"));
        assert_eq!(wanted(custom.setting_value().as_deref()), custom);
    }

    #[test]
    fn load_reads_one_setting_per_action() {
        let config = load(|key| match key {
            "hotkey.capture" => Some("Ctrl+Alt+K".to_owned()),
            "hotkey.search" => Some(String::new()),
            _ => None,
        });
        assert_eq!(config, [Wanted::Custom(combo("Ctrl+Alt+K")), Wanted::Off, Wanted::Default, Wanted::Default]);
    }

    #[test]
    fn ids_identify_the_action() {
        for a in Action::ALL {
            for candidate in 0..a.defaults().len().max(1) {
                let id = hotkey_id(a, candidate);
                assert!(id > 0);
                assert_eq!(action_of_id(id), Some(a));
            }
        }
        assert_eq!(action_of_id(0), None);
        assert_eq!(action_of_id(99), None);
        assert_eq!(action_of_id(500), None);
    }

    #[test]
    fn custom_combinations_are_bound_before_defaults() {
        let layer = Wanted::Custom(combo("Ctrl+Alt+N"));
        let order = registration_order(&[Wanted::Default, Wanted::Off, Wanted::Default, layer]);
        assert_eq!(order, [Action::Layer, Action::Capture, Action::Search, Action::Library]);
        let all_default = registration_order(&[Wanted::Default; 4]);
        assert_eq!(all_default, Action::ALL);
    }

    #[test]
    fn clipboard_events_record_their_key() {
        let paste = egui::Event::Paste("x".into());
        assert_eq!(clipboard_key(&egui::Event::Copy, false, false), Some(Key::C));
        assert_eq!(clipboard_key(&egui::Event::Copy, true, false), Some(Key::Insert));
        assert_eq!(clipboard_key(&paste, false, false), Some(Key::V));
        assert_eq!(clipboard_key(&paste, true, false), Some(Key::Insert));
        assert_eq!(clipboard_key(&egui::Event::Cut, false, false), Some(Key::X));
        assert_eq!(clipboard_key(&egui::Event::Cut, false, true), Some(Key::Delete));
        assert_eq!(clipboard_key(&egui::Event::Text("c".into()), false, false), None);
        // Ctrl+Alt+C is what a user would press for "capture".
        let c = Combo::from_egui(clipboard_key(&egui::Event::Copy, false, false).unwrap(), true, true, false, false);
        assert_eq!(c.map(|c| c.to_string()).as_deref(), Some("Ctrl+Alt+C"));
    }

    #[test]
    fn setting_keys_are_distinct() {
        let mut keys: Vec<_> = Action::ALL.iter().map(|a| a.setting_key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), Action::ALL.len());
    }
}
