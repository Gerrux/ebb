//! Card model and the quick-capture parser.

use egui::{Color32, Pos2, Vec2, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Note,
    Idea,
    Prompt,
    Link,
    Goal,
    Reminder,
    Reference,
    Private,
}

/// Why a card is currently on the layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Placement {
    #[default]
    Manual,
    Pinned,
    Today,
    Rediscover,
    Archive,
}

impl Placement {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Pinned => "pinned",
            Self::Today => "today",
            Self::Rediscover => "rediscover",
            Self::Archive => "archive",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "pinned" => Self::Pinned,
            "today" => Self::Today,
            "rediscover" => Self::Rediscover,
            "archive" => Self::Archive,
            _ => Self::Manual,
        }
    }
}

impl Kind {
    pub const ALL: [Kind; 8] = [
        Kind::Note,
        Kind::Idea,
        Kind::Prompt,
        Kind::Link,
        Kind::Goal,
        Kind::Reminder,
        Kind::Reference,
        Kind::Private,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Note => "note",
            Kind::Idea => "idea",
            Kind::Prompt => "prompt",
            Kind::Link => "link",
            Kind::Goal => "goal",
            Kind::Reminder => "reminder",
            Kind::Reference => "reference",
            Kind::Private => "private",
        }
    }

    pub fn parse(s: &str) -> Kind {
        Kind::ALL.into_iter().find(|k| k.as_str() == s).unwrap_or(Kind::Note)
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Note => "Заметка",
            Kind::Idea => "Идея",
            Kind::Prompt => "Prompt",
            Kind::Link => "Ссылка",
            Kind::Goal => "Цель",
            Kind::Reminder => "Напоминание",
            Kind::Reference => "Reference",
            Kind::Private => "Private",
        }
    }

    /// Segoe Fluent Icons glyph.
    pub fn icon(self) -> &'static str {
        match self {
            Kind::Note => "\u{E70B}",      // QuickNote
            Kind::Idea => "\u{EA80}",      // Lightbulb
            Kind::Prompt => "\u{E99A}",    // Robot
            Kind::Link => "\u{E71B}",      // Link
            Kind::Goal => "\u{E7C1}",      // Flag
            Kind::Reminder => "\u{E823}",  // Clock
            Kind::Reference => "\u{E943}", // Code
            Kind::Private => "\u{E72E}",   // Lock
        }
    }

    /// Readable on the current theme (see theme::adapt).
    pub fn accent(self) -> Color32 {
        crate::theme::adapt(self.raw_accent())
    }

    fn raw_accent(self) -> Color32 {
        match self {
            Kind::Note => Color32::from_rgb(160, 174, 192),
            Kind::Idea => Color32::from_rgb(250, 204, 21),
            Kind::Prompt => Color32::from_rgb(167, 139, 250),
            Kind::Link => Color32::from_rgb(96, 165, 250),
            Kind::Goal => Color32::from_rgb(52, 211, 153),
            Kind::Reminder => Color32::from_rgb(251, 146, 60),
            Kind::Reference => Color32::from_rgb(45, 212, 191),
            Kind::Private => Color32::from_rgb(244, 114, 182),
        }
    }

    /// The digit that switches a selected card to this kind (1–8, in `ALL` order).
    pub fn key(self) -> char {
        let i = Kind::ALL.iter().position(|k| *k == self).unwrap_or(0);
        char::from(b'1' + i as u8)
    }
}

/// A color picked for a card by hand; replaces its kind's accent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tint {
    Yellow,
    Orange,
    Pink,
    Purple,
    Blue,
    Teal,
    Green,
    Gray,
}

impl Tint {
    pub const ALL: [Tint; 8] =
        [Tint::Yellow, Tint::Orange, Tint::Pink, Tint::Purple, Tint::Blue, Tint::Teal, Tint::Green, Tint::Gray];

    pub fn as_str(self) -> &'static str {
        match self {
            Tint::Yellow => "yellow",
            Tint::Orange => "orange",
            Tint::Pink => "pink",
            Tint::Purple => "purple",
            Tint::Blue => "blue",
            Tint::Teal => "teal",
            Tint::Green => "green",
            Tint::Gray => "gray",
        }
    }

    pub fn parse(s: &str) -> Option<Tint> {
        Tint::ALL.into_iter().find(|t| t.as_str() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            Tint::Yellow => "Жёлтый",
            Tint::Orange => "Оранжевый",
            Tint::Pink => "Розовый",
            Tint::Purple => "Фиолетовый",
            Tint::Blue => "Синий",
            Tint::Teal => "Бирюзовый",
            Tint::Green => "Зелёный",
            Tint::Gray => "Серый",
        }
    }

    pub fn color(self) -> Color32 {
        crate::theme::adapt(self.raw_color())
    }

    fn raw_color(self) -> Color32 {
        match self {
            Tint::Yellow => Color32::from_rgb(250, 204, 21),
            Tint::Orange => Color32::from_rgb(251, 146, 60),
            Tint::Pink => Color32::from_rgb(244, 114, 182),
            Tint::Purple => Color32::from_rgb(167, 139, 250),
            Tint::Blue => Color32::from_rgb(96, 165, 250),
            Tint::Teal => Color32::from_rgb(45, 212, 191),
            Tint::Green => Color32::from_rgb(52, 211, 153),
            Tint::Gray => Color32::from_rgb(160, 174, 192),
        }
    }
}

/// How a card shows its kind's (or its own) color on the layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    None,
    Glow,
    StripTop,
    StripLeft,
    Tint,
    Border,
    /// The whole card in a muted shade of the color (Google Keep, Sticky Notes).
    Fill,
    /// A colored outline over a faint tint (Obsidian Canvas).
    Outline,
}

/// Where the kind's icon (which opens the kind menu) sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconSpot {
    BottomRight,
    TopLeft,
    /// Bottom right, only while the card is hovered or selected.
    Hover,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardStyle {
    pub marker: Marker,
    pub icon: IconSpot,
    /// 0–100: how strong the marker's color is.
    pub strength: u8,
    /// A larger icon with thickened strokes.
    pub bold_icon: bool,
    /// Width of the strip markers, in points.
    pub strip: u8,
    /// Corner radius, in points.
    pub radius: u8,
    /// 0–100: how much shadow a card casts.
    pub shadow: u8,
    /// 30–100 %: opacity of the card's background (text stays opaque).
    pub opacity: u8,
    pub font: crate::theme::CardFont,
    /// Text size in half points (29 = 14.5 pt).
    pub text: u8,
    pub background: crate::theme::CardBackground,
}

impl CardStyle {
    pub fn text_size(self) -> f32 {
        f32::from(self.text) / 2.0
    }
}

impl Default for CardStyle {
    fn default() -> Self {
        PRESETS[0].style
    }
}

impl Marker {
    pub const ALL: [Marker; 8] =
        [Marker::None, Marker::Glow, Marker::StripTop, Marker::StripLeft, Marker::Tint, Marker::Border, Marker::Fill, Marker::Outline];

    pub fn key(self) -> &'static str {
        match self {
            Marker::None => "none",
            Marker::Glow => "glow",
            Marker::StripTop => "strip-top",
            Marker::StripLeft => "strip-left",
            Marker::Tint => "tint",
            Marker::Border => "border",
            Marker::Fill => "fill",
            Marker::Outline => "outline",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Marker::None => "Без цвета",
            Marker::Glow => "Свечение из угла",
            Marker::StripTop => "Полоса сверху",
            Marker::StripLeft => "Полоса слева",
            Marker::Tint => "Тонировка",
            Marker::Border => "Цветная рамка",
            Marker::Fill => "Заливка",
            Marker::Outline => "Рамка и тон",
        }
    }
}

impl IconSpot {
    pub const ALL: [IconSpot; 3] = [IconSpot::BottomRight, IconSpot::TopLeft, IconSpot::Hover];

    pub fn key(self) -> &'static str {
        match self {
            IconSpot::BottomRight => "bottom-right",
            IconSpot::TopLeft => "top-left",
            IconSpot::Hover => "hover",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            IconSpot::BottomRight => "Снизу справа",
            IconSpot::TopLeft => "Сверху слева",
            IconSpot::Hover => "Только при наведении",
        }
    }
}

impl CardStyle {
    /// Stored as "key=value" pairs, e.g. "marker=glow icon=bottom-right strength=50 bold=0 strip=5 radius=12";
    /// keys it doesn't know are skipped, missing ones keep the default.
    pub fn to_setting(self) -> String {
        format!(
            "marker={} icon={} strength={} bold={} strip={} radius={} shadow={} opacity={} font={} text={} background={}",
            self.marker.key(),
            self.icon.key(),
            self.strength,
            u8::from(self.bold_icon),
            self.strip,
            self.radius,
            self.shadow,
            self.opacity,
            self.font.key(),
            self.text,
            self.background.key()
        )
    }

    pub fn from_setting(s: &str) -> CardStyle {
        let mut style = CardStyle::default();
        let num = |v: &str| v.parse::<u32>().ok();
        for (key, value) in s.split_whitespace().filter_map(|p| p.split_once('=')) {
            match key {
                "marker" => style.marker = Marker::ALL.into_iter().find(|m| m.key() == value).unwrap_or(style.marker),
                "icon" => style.icon = IconSpot::ALL.into_iter().find(|i| i.key() == value).unwrap_or(style.icon),
                "strength" => style.strength = num(value).map_or(style.strength, |n| n.min(100) as u8),
                "bold" => style.bold_icon = value == "1",
                "strip" => style.strip = num(value).map_or(style.strip, |n| n.clamp(1, 12) as u8),
                "radius" => style.radius = num(value).map_or(style.radius, |n| n.min(20) as u8),
                "shadow" => style.shadow = num(value).map_or(style.shadow, |n| n.min(100) as u8),
                "opacity" => style.opacity = num(value).map_or(style.opacity, |n| n.clamp(30, 100) as u8),
                "font" => {
                    style.font = crate::theme::CardFont::ALL.into_iter().find(|f| f.key() == value).unwrap_or(style.font)
                }
                "text" => style.text = num(value).map_or(style.text, |n| n.clamp(22, 40) as u8),
                "background" => style.background = crate::theme::CardBackground::from_key(value).unwrap_or(style.background),
                _ => {}
            }
        }
        style
    }
}

/// A named look, modeled on an app people may be coming from.
pub struct Preset {
    pub name: &'static str,
    pub hint: &'static str,
    pub style: CardStyle,
}

/// Look: color marker, icon, strength, bold icon, strip, radius, shadow, opacity, font, text size (half pt), background.
#[allow(clippy::too_many_arguments)]
const fn style(
    marker: Marker,
    icon: IconSpot,
    strength: u8,
    bold_icon: bool,
    strip: u8,
    radius: u8,
    shadow: u8,
    opacity: u8,
    font: crate::theme::CardFont,
    text: u8,
    background: crate::theme::CardBackground,
) -> CardStyle {
    CardStyle { marker, icon, strength, bold_icon, strip, radius, shadow, opacity, font, text, background }
}

use crate::theme::{CardBackground as B, CardFont as F};

pub const PRESETS: [Preset; 8] = [
    Preset {
        name: "Ebb",
        hint: "Стекло и свечение цвета",
        style: style(Marker::Glow, IconSpot::BottomRight, 50, false, 5, 12, 50, 85, F::System, 29, B::Theme),
    },
    Preset {
        name: "Sticky Notes",
        hint: "Цветная шапка заметки",
        style: style(Marker::StripTop, IconSpot::Hover, 100, false, 8, 8, 35, 100, F::System, 29, B::Theme),
    },
    Preset {
        name: "Google Keep",
        hint: "Заливка, плоско, рамка",
        style: style(Marker::Fill, IconSpot::Hover, 50, false, 5, 8, 0, 100, F::System, 28, B::Theme),
    },
    Preset {
        name: "Obsidian Canvas",
        hint: "Рамка и лёгкий тон",
        style: style(Marker::Outline, IconSpot::TopLeft, 60, false, 5, 8, 0, 95, F::System, 28, B::Theme),
    },
    Preset {
        name: "Trello",
        hint: "Цветная метка слева",
        style: style(Marker::StripLeft, IconSpot::TopLeft, 80, false, 4, 6, 20, 100, F::System, 28, B::Theme),
    },
    Preset {
        name: "Бумажный стикер",
        hint: "Заливка, тень, от руки",
        style: style(Marker::Fill, IconSpot::Hover, 70, false, 5, 2, 70, 100, F::SegoePrint, 29, B::Theme),
    },
    Preset {
        name: "Минимализм",
        hint: "Только жирная иконка",
        style: style(Marker::None, IconSpot::BottomRight, 50, true, 5, 12, 20, 85, F::System, 29, B::Theme),
    },
    Preset {
        name: "Windows",
        hint: "Фон как у панели задач",
        style: style(Marker::None, IconSpot::BottomRight, 50, false, 5, 8, 35, 96, F::System, 28, B::Taskbar),
    },
];

#[derive(Clone, Debug)]
pub struct Card {
    pub id: i64,
    pub kind: Kind,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub pinned: bool,
    pub archived: bool,
    pub pos: Pos2,
    pub size: Vec2,
    pub created_at: i64,
    pub review_at: Option<i64>,
    pub placement: Placement,
    /// Picked by hand; `None` takes the kind's color.
    pub tint: Option<Tint>,
}

impl Card {
    pub fn accent(&self) -> Color32 {
        self.tint.map_or(self.kind.accent(), Tint::color)
    }

    /// The caption of a card brought back from the archive.
    pub fn resurface_reason(&self, now: i64) -> Option<String> {
        (self.placement == Placement::Rediscover).then(|| crate::resurface::reason(now, self.created_at, self.review_at))
    }
}

const COMMANDS: &[&str] = &[
    "ssh", "scp", "rsync", "git", "cd", "ls", "curl", "wget", "docker", "kubectl", "helm", "npm", "pnpm", "yarn",
    "npx", "cargo", "pip", "python", "node", "ping", "sudo", "psql", "mysql", "redis-cli", "telnet", "winget",
    "choco", "pwsh", "powershell", "systemctl", "journalctl", "tail", "cat", "export", "set",
];

/// A Reference line to set in monospace: a command, or a short line with a path,
/// address or host in it ("vpn.staging.internal", "10.0.4.12:22", `C:\tools`).
pub fn looks_technical(line: &str) -> bool {
    let line = line.trim();
    let words: Vec<&str> = line.split_whitespace().collect();
    let Some(first) = words.first() else { return false };
    if matches!(*first, "$" | ">") {
        return true;
    }
    let technical = words.iter().any(|w| technical_word(w.trim_end_matches([',', ';', ')'])));
    // "git push", "kubectl get pods -n prod"; not "ping Alex about the release".
    if COMMANDS.contains(&first.to_lowercase().as_str())
        && words.len() > 1
        && (words.len() <= 3 || technical || words.iter().any(|w| w.starts_with('-')))
    {
        return true;
    }
    words.len() <= 6 && technical
}

fn technical_word(w: &str) -> bool {
    if !w.is_ascii() || w.len() < 3 {
        return false;
    }
    let drive = w.as_bytes()[0].is_ascii_alphabetic() && w[1..].starts_with(":\\");
    if w.contains("://") || w.starts_with('/') || w.starts_with("~/") || w.starts_with("./") || drive || w.contains('\\') {
        return true;
    }
    // Keys, hashes, tokens: long runs of letters mixed with digits.
    if w.len() >= 12 && w.bytes().any(|b| b.is_ascii_digit()) && w.bytes().any(|b| b.is_ascii_alphabetic()) {
        return true;
    }
    // host, host:port, user@host, 10.0.4.12
    let host = w.rsplit('@').next().unwrap_or(w);
    let host = match host.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => host,
    };
    let labels: Vec<&str> = host.split('.').collect();
    let ip = labels.len() == 4 && labels.iter().all(|l| !l.is_empty() && l.len() <= 3 && l.bytes().all(|b| b.is_ascii_digit()));
    let name = labels.len() >= 2
        && labels.iter().all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        && labels.last().is_some_and(|l| l.len() >= 2 && l.bytes().all(|b| b.is_ascii_alphabetic()))
        && (labels.len() >= 3 || w.contains(['@', ':']));
    ip || name
}

/// Host of the first http(s) URL in the text, without "www.".
pub fn link_domain(text: &str) -> Option<&str> {
    let at = text.find("https://").map(|i| i + 8).or_else(|| text.find("http://").map(|i| i + 7))?;
    let rest = &text[at..];
    let end = rest.find(|c: char| c.is_whitespace() || matches!(c, '/' | '?' | '#' | ')' | ',' | '"')).unwrap_or(rest.len());
    let host = rest[..end].rsplit('@').next().unwrap_or("");
    let host = host.strip_prefix("www.").unwrap_or(host).trim_end_matches(['.', ':']);
    (!host.is_empty()).then_some(host)
}

/// A Prompt's name: its first line, when more text follows and the line is short.
pub fn prompt_name(body: &str) -> Option<(&str, &str)> {
    let body = body.trim_start();
    let (first, rest) = body.split_once('\n')?;
    let (first, rest) = (first.trim(), rest.trim());
    (!first.is_empty() && !rest.is_empty() && first.chars().count() <= 80).then_some((first, rest))
}

/// What a hidden Private card may show: its title, or else the first line when
/// more lines follow ("Wi-Fi офис" above the password). Single-line notes show
/// nothing, since that line may be the secret itself.
pub fn private_label(title: &str, body: &str) -> Option<String> {
    if !title.is_empty() {
        return Some(title.to_owned());
    }
    let mut lines = body.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next().filter(|l| fits_label(l))?;
    lines.next()?;
    Some(first.to_owned())
}

/// The label of a Private card that has nothing to show.
pub const PRIVATE_PLACEHOLDER: &str = "Private";

/// Splits a Private note into the label that stays in the open and the value
/// that is encrypted: the title, else the first line when more lines follow.
/// A lone line may be the secret itself, so it's all value.
pub fn private_parts(title: &str, body: &str) -> (String, String) {
    let title = title.trim();
    if !title.is_empty() {
        return if fits_label(title) {
            (title.to_owned(), body.to_owned())
        } else {
            (PRIVATE_PLACEHOLDER.to_owned(), private_text(title, body))
        };
    }
    match body.trim().split_once('\n') {
        Some((label, secret)) if fits_label(label.trim()) && !secret.trim().is_empty() => {
            (label.trim().to_owned(), secret.trim_start().to_owned())
        }
        _ => (PRIVATE_PLACEHOLDER.to_owned(), body.to_owned()),
    }
}

/// Whether a line may stay in the open as a Private card's label: short, and
/// nothing like a credential — no assignments, keys, long tokens or addresses.
/// Errs on the side of hiding; a hidden label only costs the card its name.
pub fn fits_label(line: &str) -> bool {
    const SECRETS: &[&str] = &["pass", "парол", "token", "токен", "secret", "секрет", "key", "ключ", "begin", "root@", "ssh-"];
    let lower = line.to_lowercase();
    let token_like = |w: &str| {
        w.chars().count() >= 16 && w.chars().any(|c| c.is_ascii_digit()) && w.chars().any(char::is_alphabetic)
    };
    let ip_like = |w: &str| {
        let parts: Vec<&str> = w.trim_matches(|c: char| !c.is_ascii_digit()).split('.').collect();
        parts.len() == 4 && parts.iter().all(|p| !p.is_empty() && p.len() <= 3 && p.chars().all(|c| c.is_ascii_digit()))
    };
    !line.is_empty()
        && line.chars().count() <= 60
        && !line.contains('=')
        && !SECRETS.iter().any(|s| lower.contains(s))
        && !line.split_whitespace().any(|w| token_like(w) || ip_like(w))
}

/// The note as written, back from its label and value: what the editor opens
/// with, and what a card turned from Private into another kind keeps.
pub fn private_text(label: &str, secret: &str) -> String {
    // Without a label of its own, a multi-line value keeps the placeholder as
    // its first line, so saving it again doesn't lay that line open.
    if label == PRIVATE_PLACEHOLDER && !secret.trim().contains('\n') {
        secret.to_owned()
    } else {
        format!("{label}\n{secret}")
    }
}

pub const MIN_SIZE: Vec2 = vec2(200.0, 96.0);
pub const DEFAULT_SIZE: Vec2 = vec2(280.0, 150.0);

/// Result of parsing a quick-capture string.
#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub kind: Kind,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
}

const PREFIXES: &[(&str, Kind)] = &[
    ("идея:", Kind::Idea),
    ("idea:", Kind::Idea),
    ("промпт:", Kind::Prompt),
    ("prompt:", Kind::Prompt),
    ("цель:", Kind::Goal),
    ("goal:", Kind::Goal),
    ("ref:", Kind::Reference),
    ("private:", Kind::Private),
    ("секрет:", Kind::Private),
];

const REFERENCE_WORDS: &[&str] = &[
    "vpn", "ssh", "host", "server", "сервер", "endpoint", "path", "ip", "port", "порт",
];

pub fn parse_capture(input: &str) -> Parsed {
    let text = input.trim();
    let lower = text.to_lowercase();

    let mut kind = Kind::Note;
    let mut rest = text;
    for (prefix, k) in PREFIXES {
        if lower.starts_with(prefix) {
            kind = *k;
            // Prefixes are ASCII or Cyrillic; slice by char count to stay on a boundary.
            let n = prefix.chars().count();
            let byte = text.char_indices().nth(n).map_or(text.len(), |(i, _)| i);
            rest = text[byte..].trim_start();
            break;
        }
    }

    if kind == Kind::Note {
        let words: Vec<&str> = lower
            .split(|c: char| !c.is_alphanumeric() && c != '.' && c != ':' && c != '/')
            .filter(|w| !w.is_empty())
            .collect();
        if lower.contains("http://") || lower.contains("https://") {
            kind = Kind::Link;
        } else if is_reminder(&lower) {
            kind = Kind::Reminder;
        } else if words.iter().any(|w| REFERENCE_WORDS.contains(w)) {
            kind = Kind::Reference;
        }
    }

    let tags = rest
        .split_whitespace()
        .filter_map(|w| w.strip_prefix('#'))
        .filter(|t| !t.is_empty())
        .map(|t| t.trim_end_matches([',', '.', ';']).to_lowercase())
        .collect();

    // No automatic title: the first line is often not one, and splitting it off
    // changes how the note reads. The text is kept as written.
    Parsed {
        kind,
        title: String::new(),
        body: rest.to_owned(),
        tags,
    }
}

fn is_reminder(lower: &str) -> bool {
    if lower.starts_with("напомни") || lower.starts_with("remind") {
        return true;
    }
    const UNITS: &[&str] = &["дн", "день", "недел", "месяц", "час", "day", "week", "month"];
    let after = lower
        .split("через ")
        .nth(1)
        .or_else(|| lower.split("in ").nth(1));
    after.is_some_and(|a| a.split_whitespace().take(2).any(|w| UNITS.iter().any(|u| w.starts_with(u))))
}

/// Place a new card at the first free slot of a coarse grid.
pub fn free_slot(cards: &[Card], area: Vec2) -> Pos2 {
    let step = vec2(DEFAULT_SIZE.x + 16.0, DEFAULT_SIZE.y + 16.0);
    let origin = pos2(32.0, 72.0);
    let cols = ((area.x - origin.x) / step.x).floor().max(1.0) as usize;
    let rows = ((area.y - origin.y) / step.y).floor().max(1.0) as usize;
    for row in 0..rows {
        for col in 0..cols {
            let p = origin + vec2(col as f32 * step.x, row as f32 * step.y);
            let slot = egui::Rect::from_min_size(p, DEFAULT_SIZE);
            if !cards
                .iter()
                .filter(|c| !c.archived)
                .any(|c| egui::Rect::from_min_size(c.pos, c.size).intersects(slot))
            {
                return p;
            }
        }
    }
    // A full layer: a cascade, each card a step past the last one placed there,
    // so several cards placed in a row don't hide one another.
    (0..10)
        .map(|k| origin + vec2(24.0, 24.0) * k as f32)
        .find(|p| !cards.iter().any(|c| !c.archived && c.pos == *p))
        .unwrap_or(origin)
}

/// Where a copy of `of` goes: right against it, else below, left or above, the
/// first spot that is on the layer and free; else the first free slot.
pub fn beside(cards: &[Card], of: &Card, area: Vec2) -> Pos2 {
    let layer = egui::Rect::from_min_size(Pos2::ZERO, area);
    let (w, h) = (of.size.x, of.size.y);
    [vec2(w, 0.0), vec2(0.0, h), vec2(-w, 0.0), vec2(0.0, -h)]
        .into_iter()
        .map(|d| of.pos + d)
        .find(|&p| {
            // Shrunk a little: touching a neighbour's edge is fine.
            let slot = egui::Rect::from_min_size(p, of.size).shrink(0.5);
            layer.contains_rect(slot)
                && !cards
                    .iter()
                    .filter(|c| !c.archived)
                    .any(|c| egui::Rect::from_min_size(c.pos, c.size).intersects(slot))
        })
        .unwrap_or_else(|| free_slot(cards, area))
}

/// Space between cards that stick side by side: none, they sit edge to edge.
pub const SNAP_GAP: f32 = 0.0;
/// How close an edge has to come before it sticks.
pub const SNAP_DISTANCE: f32 = 10.0;
/// Cards at most this far apart (across the snapping axis) align their edges.
const ALIGN_REACH: f32 = 64.0;
/// Margins of the layer that edges stick to (the header takes the top).
const LAYER_MARGIN: egui::Margin = egui::Margin { left: 32, right: 32, top: 72, bottom: 32 };

/// A card rect after snapping, with a guide line per axis that stuck.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapped {
    pub rect: egui::Rect,
    pub guides: Vec<[Pos2; 2]>,
    pub x: bool,
    pub y: bool,
}

/// Best candidate on one axis: (offset to apply, guide line).
type Candidate = Option<(f32, [Pos2; 2])>;

fn consider(best: &mut Candidate, from: f32, to: f32, guide: impl FnOnce(f32) -> [Pos2; 2]) {
    let d = to - from;
    if d.abs() <= SNAP_DISTANCE && best.is_none_or(|(b, _)| d.abs() < b.abs()) {
        *best = Some((d, guide((from + to) / 2.0)));
    }
}

/// Span of two ranges, for guide lines.
fn span(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    (a.0.min(b.0), a.1.max(b.1))
}

/// Candidates for moving edges; `edges` says which of left/right (or top/bottom)
/// are free to move. `x` picks the axis.
fn axis(rect: egui::Rect, others: &[egui::Rect], bounds: egui::Rect, x: bool, edges: (bool, bool)) -> Candidate {
    // (low edge, high edge) along the axis, and the range across it.
    let along = |r: egui::Rect| if x { (r.left(), r.right()) } else { (r.top(), r.bottom()) };
    let across = |r: egui::Rect| if x { (r.top(), r.bottom()) } else { (r.left(), r.right()) };
    let line = |at: f32, (a, b): (f32, f32)| if x { [pos2(at, a), pos2(at, b)] } else { [pos2(a, at), pos2(b, at)] };
    let (lo, hi) = along(rect);
    let cross = across(rect);
    let mut best = None;
    for &o in others {
        let (olo, ohi) = along(o);
        let ocross = across(o);
        let s = span(cross, ocross);
        let gap_across = (ocross.0 - cross.1).max(cross.0 - ocross.1);
        // Side by side: they overlap across the axis.
        if gap_across < 0.0 {
            if edges.0 {
                consider(&mut best, lo, ohi + SNAP_GAP, |at| line(at, s));
            }
            if edges.1 {
                consider(&mut best, hi, olo - SNAP_GAP, |at| line(at, s));
            }
        }
        // Aligned: one above the other (or near), same edge.
        if gap_across <= ALIGN_REACH {
            if edges.0 {
                consider(&mut best, lo, olo, |at| line(at, s));
            }
            if edges.1 {
                consider(&mut best, hi, ohi, |at| line(at, s));
            }
        }
    }
    let (blo, bhi) = if x {
        (bounds.left() + LAYER_MARGIN.left as f32, bounds.right() - LAYER_MARGIN.right as f32)
    } else {
        (bounds.top() + LAYER_MARGIN.top as f32, bounds.bottom() - LAYER_MARGIN.bottom as f32)
    };
    let whole = across(bounds);
    if edges.0 {
        consider(&mut best, lo, blo, |_| line(blo, whole));
    }
    if edges.1 {
        consider(&mut best, hi, bhi, |_| line(bhi, whole));
    }
    best
}

/// Sticks a dragged card to the edges of the others and of the layer: next to a
/// card (edge to edge, [`SNAP_GAP`]), or aligned with its edge.
pub fn snap_move(rect: egui::Rect, others: &[egui::Rect], bounds: egui::Rect) -> Snapped {
    let dx = axis(rect, others, bounds, true, (true, true));
    let dy = axis(rect, others, bounds, false, (true, true));
    let shift = vec2(dx.map_or(0.0, |d| d.0), dy.map_or(0.0, |d| d.0));
    // Guides were computed before the shift across the other axis; move them along.
    let guides = dx
        .map(|(_, g)| g.map(|p| p + vec2(0.0, shift.y)))
        .into_iter()
        .chain(dy.map(|(_, g)| g.map(|p| p + vec2(shift.x, 0.0))))
        .collect();
    Snapped { rect: rect.translate(shift), guides, x: dx.is_some(), y: dy.is_some() }
}

/// Which edges of a card a resize handle moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sides {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

impl Sides {
    /// `start` with these edges moved by `delta`; the opposite edges stay, the
    /// size never drops below [`MIN_SIZE`] and moved edges stay on the layer.
    pub fn resize(self, start: egui::Rect, delta: Vec2, bounds: egui::Rect) -> egui::Rect {
        let mut r = start;
        if self.left {
            r.min.x = (r.min.x + delta.x).max(bounds.left()).min(r.max.x - MIN_SIZE.x);
        }
        if self.right {
            r.max.x = (r.max.x + delta.x).max(r.min.x + MIN_SIZE.x);
        }
        if self.top {
            r.min.y = (r.min.y + delta.y).max(bounds.top()).min(r.max.y - MIN_SIZE.y);
        }
        if self.bottom {
            r.max.y = (r.max.y + delta.y).max(r.min.y + MIN_SIZE.y);
        }
        r
    }

    pub fn cursor(self) -> egui::CursorIcon {
        use egui::CursorIcon::*;
        match (self.left || self.right, self.top || self.bottom) {
            (true, false) => ResizeHorizontal,
            (false, true) => ResizeVertical,
            _ if (self.left && self.top) || (self.right && self.bottom) => ResizeNwSe,
            _ => ResizeNeSw,
        }
    }
}

/// Sticks the edges of a card being resized that the handle moves.
pub fn snap_resize(rect: egui::Rect, others: &[egui::Rect], bounds: egui::Rect, sides: Sides) -> Snapped {
    // Moving the low edge by d shrinks the rect by d; the high edge grows it.
    let fits = |c: Candidate, low: bool, len: f32, min: f32| c.filter(|(d, _)| if low { len - d } else { len + d } >= min);
    let dx = fits(axis(rect, others, bounds, true, (sides.left, sides.right)), sides.left, rect.width(), MIN_SIZE.x);
    let dy = fits(axis(rect, others, bounds, false, (sides.top, sides.bottom)), sides.top, rect.height(), MIN_SIZE.y);
    let mut out = rect;
    if let Some((d, _)) = dx {
        if sides.left { out.min.x += d } else { out.max.x += d }
    }
    if let Some((d, _)) = dy {
        if sides.top { out.min.y += d } else { out.max.y += d }
    }
    let guides = dx.map(|(_, g)| g).into_iter().chain(dy.map(|(_, g)| g)).collect();
    Snapped { rect: out, guides, x: dx.is_some(), y: dy.is_some() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f32, y: f32, w: f32, h: f32) -> egui::Rect {
        egui::Rect::from_min_size(pos2(x, y), vec2(w, h))
    }

    const BOUNDS: egui::Rect = egui::Rect { min: pos2(0.0, 0.0), max: pos2(2000.0, 1200.0) };

    #[test]
    fn snaps_beside_a_card_with_a_gap() {
        let other = r(400.0, 300.0, 280.0, 150.0);
        // Right of it, 6 pt too far.
        let s = snap_move(r(686.0 + SNAP_GAP, 330.0, 280.0, 150.0), &[other], BOUNDS);
        assert_eq!(s.rect.left(), 680.0 + SNAP_GAP);
        assert!(s.x);
        assert_eq!(s.guides.len(), 1);
    }

    #[test]
    fn aligns_edges_when_stacked() {
        let other = r(400.0, 300.0, 280.0, 150.0);
        // Below it, left edges 7 pt apart, top 5 pt from the gap.
        let s = snap_move(r(407.0, 455.0 + SNAP_GAP, 200.0, 100.0), &[other], BOUNDS);
        assert_eq!(s.rect.min, pos2(400.0, 450.0 + SNAP_GAP));
        assert!(s.x && s.y);
    }

    #[test]
    fn far_cards_and_free_space_do_not_stick() {
        let other = r(400.0, 300.0, 280.0, 150.0);
        let rect = r(900.0, 700.0, 280.0, 150.0);
        let s = snap_move(rect, &[other], BOUNDS);
        assert_eq!(s.rect, rect);
        assert!(s.guides.is_empty());
    }

    #[test]
    fn sticks_to_layer_margins() {
        let s = snap_move(r(36.0, 500.0, 280.0, 150.0), &[], BOUNDS);
        assert_eq!(s.rect.left(), 32.0);
    }

    #[test]
    fn resize_snaps_only_the_moving_edges_and_keeps_min_size() {
        let br = Sides { right: true, bottom: true, ..Default::default() };
        let other = r(700.0, 300.0, 280.0, 150.0);
        let s = snap_resize(r(400.0, 320.0, 290.0, 120.0), &[other], BOUNDS, br);
        assert_eq!(s.rect.min, pos2(400.0, 320.0));
        assert_eq!(s.rect.right(), 700.0 - SNAP_GAP);
        // Bottom aligns with the neighbour's.
        assert_eq!(s.rect.bottom(), 450.0);

        let tiny = r(400.0, 320.0, MIN_SIZE.x + 2.0, 120.0);
        let s = snap_resize(tiny, &[r(400.0 + MIN_SIZE.x + 2.0 + SNAP_GAP - 8.0, 300.0, 100.0, 150.0)], BOUNDS, br);
        assert!(s.rect.width() >= MIN_SIZE.x);
    }

    fn card_at(id: i64, x: f32, y: f32) -> Card {
        Card {
            id,
            kind: Kind::Note,
            title: String::new(),
            body: String::new(),
            tags: Vec::new(),
            pinned: false,
            archived: false,
            pos: pos2(x, y),
            size: DEFAULT_SIZE,
            created_at: 0,
            review_at: None,
            placement: Placement::Manual,
            tint: None,
        }
    }

    #[test]
    fn technical_lines() {
        for line in [
            "ssh deploy@10.0.4.12",
            "vpn staging vpn.staging.internal",
            "C:\\tools\\bin",
            "/etc/nginx/nginx.conf",
            "db: postgres.local:5432",
            "$ cargo build --release",
            "https://egui.rs",
            "3f9a0c7e41b2d85e6a1f",
        ] {
            assert!(looks_technical(line), "{line}");
        }
        for line in ["просто мысль", "т.е. позже", "e.g. later", "ping Alex about the release", "Wi-Fi офис", ""] {
            assert!(!looks_technical(line), "{line}");
        }
    }

    #[test]
    fn link_domains() {
        assert_eq!(link_domain("демо https://www.egui.rs/#demo"), Some("egui.rs"));
        assert_eq!(link_domain("(http://user@host.dev:8080/x)"), Some("host.dev:8080"));
        assert_eq!(link_domain("без ссылки"), None);
    }

    #[test]
    fn prompt_names() {
        assert_eq!(prompt_name("Ревью кода\nПосмотри на {{diff}}"), Some(("Ревью кода", "Посмотри на {{diff}}")));
        assert_eq!(prompt_name("одна строка"), None);
    }

    #[test]
    fn card_style_round_trips_and_survives_junk() {
        let s = CardStyle { radius: 4, shadow: 10, opacity: 60, font: crate::theme::CardFont::Georgia, text: 32, background: crate::theme::CardBackground::Custom([255, 242, 171]), ..PRESETS[3].style };
        assert_eq!(CardStyle::from_setting(&s.to_setting()), s);
        let junk = CardStyle::from_setting("marker=what strength=999 color=red radius");
        assert_eq!(junk, CardStyle { strength: 100, ..CardStyle::default() });
    }

    #[test]
    fn kind_keys_and_tints_round_trip() {
        assert_eq!(Kind::Note.key(), '1');
        assert_eq!(Kind::Private.key(), '8');
        for t in Tint::ALL {
            assert_eq!(Tint::parse(t.as_str()), Some(t));
        }
    }

    #[test]
    fn duplicate_goes_right_against_the_original_or_below() {
        let area = vec2(1920.0, 1080.0);
        let a = card_at(1, 400.0, 300.0);
        assert_eq!(beside(&[a.clone()], &a, area), pos2(400.0 + DEFAULT_SIZE.x, 300.0));
        // Right is taken: below.
        let b = card_at(2, 400.0 + DEFAULT_SIZE.x, 300.0);
        assert_eq!(beside(&[a.clone(), b], &a, area), pos2(400.0, 300.0 + DEFAULT_SIZE.y));
        // At the right edge of the layer: not off-screen.
        let edge = card_at(3, area.x - DEFAULT_SIZE.x, 300.0);
        assert_eq!(beside(&[edge.clone()], &edge, area), pos2(edge.pos.x, 300.0 + DEFAULT_SIZE.y));
    }

    #[test]
    fn resize_from_the_left_and_top_moves_those_edges() {
        let tl = Sides { left: true, top: true, ..Default::default() };
        let start = r(400.0, 400.0, 280.0, 150.0);
        let grown = tl.resize(start, vec2(-50.0, -30.0), BOUNDS);
        assert_eq!((grown.min, grown.max), (pos2(350.0, 370.0), start.max));
        // Can't shrink past the minimum: the far edges stay where they were.
        let shrunk = tl.resize(start, vec2(500.0, 500.0), BOUNDS);
        assert_eq!(shrunk.size(), MIN_SIZE);
        assert_eq!(shrunk.max, start.max);

        // Left edge 6 pt from a neighbour's right edge sticks to it.
        let left = Sides { left: true, ..Default::default() };
        let s = snap_resize(r(286.0, 400.0, 300.0, 150.0), &[r(0.0, 380.0, 280.0, 150.0)], BOUNDS, left);
        assert_eq!(s.rect.left(), 280.0);
        assert_eq!(s.rect.right(), 586.0);
    }

    #[test]
    fn detects_kinds() {
        assert_eq!(parse_capture("идея: добавить weekly recap").kind, Kind::Idea);
        assert_eq!(parse_capture("идея: добавить weekly recap").body, "добавить weekly recap");
        assert_eq!(parse_capture("vpn staging vpn.staging.internal").kind, Kind::Reference);
        assert_eq!(
            parse_capture("через две недели проверить новую pricing модель").kind,
            Kind::Reminder
        );
        assert_eq!(parse_capture("https://egui.rs demo").kind, Kind::Link);
        assert_eq!(parse_capture("просто мысль").kind, Kind::Note);
    }

    #[test]
    fn private_label_never_shows_a_lone_line() {
        assert_eq!(private_label("", "Wi-Fi офис\nguest / pass").as_deref(), Some("Wi-Fi офис"));
        assert_eq!(private_label("", "hunter2"), None);
        assert_eq!(private_label("Title", "x").as_deref(), Some("Title"));
    }

    #[test]
    fn free_slot_on_a_full_layer_cascades() {
        let area = vec2(400.0, 300.0);
        let mut cards = vec![card_at(1, 0.0, 0.0)];
        cards[0].size = area;
        let first = free_slot(&cards, area);
        cards.push(card_at(2, first.x, first.y));
        let second = free_slot(&cards, area);
        assert_ne!(first, second);
    }

    #[test]
    fn labels_that_look_like_credentials_stay_hidden() {
        for ok in ["Wi-Fi офис", "ALL MY SSH", "host: nl-home", "Домашнее задание на 3 декабря:"] {
            assert!(fits_label(ok), "{ok}");
        }
        for secret in [
            "$env:DB_PASSWORD='x'",
            "ssh root@10.0.0.1",
            "IPv4: 85.31.45.44",
            "-----BEGIN CERTIFICATE-----",
            "export TAVILY_API_KEY",
            "sk-52f6ed63b1a4f789ffd",
            "пароль от роутера",
        ] {
            assert!(!fits_label(secret), "{secret}");
        }
        assert_eq!(private_parts("", "ssh root@10.0.0.1\npass").0, PRIVATE_PLACEHOLDER);
        assert_eq!(private_label("", "token=abc\nmore"), None);
    }

    #[test]
    fn private_parts_round_trip_through_the_editor() {
        for text in ["Wi-Fi офис\nguest / pass", "hunter2", "Private\nline one\nline two"] {
            let (label, secret) = private_parts("", text);
            assert_eq!(private_parts("", &private_text(&label, &secret)), (label, secret), "{text}");
        }
        assert_eq!(private_parts("", "hunter2"), (PRIVATE_PLACEHOLDER.to_owned(), "hunter2".to_owned()));
        assert_eq!(private_text("Wi-Fi", "pass"), "Wi-Fi\npass");
    }

    #[test]
    fn extracts_tags_and_keeps_text_as_written() {
        let p = parse_capture("Pricing\nпопробовать annual plan #product #pricing.");
        assert_eq!(p.title, "");
        assert_eq!(p.body, "Pricing\nпопробовать annual plan #product #pricing.");
        assert_eq!(p.tags, vec!["product", "pricing"]);
    }
}
