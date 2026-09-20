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
    Desktop,
}

impl Placement {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Pinned => "pinned",
            Self::Today => "today",
            Self::Rediscover => "rediscover",
            Self::Archive => "archive",
            Self::Desktop => "desktop",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "pinned" => Self::Pinned,
            "today" => Self::Today,
            "rediscover" => Self::Rediscover,
            "archive" => Self::Archive,
            "desktop" => Self::Desktop,
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
        Kind::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .unwrap_or(Kind::Note)
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

    /// The kind's color: one of the tints, so a kind and a color picked by hand
    /// are painted the same way.
    pub fn tint(self) -> Tint {
        match self {
            Kind::Note => Tint::Gray,
            Kind::Idea => Tint::Yellow,
            Kind::Prompt => Tint::Purple,
            Kind::Link => Tint::Blue,
            Kind::Goal => Tint::Green,
            Kind::Reminder => Tint::Orange,
            Kind::Reference => Tint::Teal,
            Kind::Private => Tint::Pink,
        }
    }

    /// The kind's mark color, readable on the current theme (see theme::adapt).
    pub fn accent(self) -> Color32 {
        self.tint().color()
    }

    /// The digit that switches a selected card to this kind (1–8, in `ALL` order).
    pub fn key(self) -> char {
        let i = Kind::ALL.iter().position(|k| *k == self).unwrap_or(0);
        char::from(b'1' + i as u8)
    }
}

/// Where an idea stands (product.txt §4). Kept in `cards.meta`, so it survives
/// a change of kind and comes back when the card is an idea again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdeaStatus {
    Potential,
    Maybe,
    Explore,
    Important,
}

impl IdeaStatus {
    pub const ALL: [IdeaStatus; 4] = [
        IdeaStatus::Potential,
        IdeaStatus::Maybe,
        IdeaStatus::Explore,
        IdeaStatus::Important,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            IdeaStatus::Potential => "potential",
            IdeaStatus::Maybe => "maybe",
            IdeaStatus::Explore => "explore",
            IdeaStatus::Important => "important",
        }
    }

    pub fn parse(s: &str) -> Option<IdeaStatus> {
        IdeaStatus::ALL.into_iter().find(|v| v.as_str() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            IdeaStatus::Potential => "Потенциал",
            IdeaStatus::Maybe => "Может быть",
            IdeaStatus::Explore => "Изучить",
            IdeaStatus::Important => "Важно",
        }
    }

    /// Readable on a card (see theme::on_card).
    pub fn color(self) -> Color32 {
        let tint = match self {
            IdeaStatus::Potential => Tint::Blue,
            IdeaStatus::Maybe => Tint::Gray,
            IdeaStatus::Explore => Tint::Teal,
            IdeaStatus::Important => Tint::Orange,
        };
        crate::theme::on_card(tint.color())
    }
}

/// A card's color: its kind's, or one picked by hand. Two palettes per color:
/// the *mark* (dot, glyph, strip, border, glow, the primary button), and the
/// *surface* the whole card is filled with, which is not the mark washed over
/// the glass (that came out muddy) but a color of its own: the paper of Sticky
/// Notes and Keep in the light theme, a night shade like Keep's in the dark one.
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
    pub const ALL: [Tint; 8] = [
        Tint::Yellow,
        Tint::Orange,
        Tint::Pink,
        Tint::Purple,
        Tint::Blue,
        Tint::Teal,
        Tint::Green,
        Tint::Gray,
    ];

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

    /// The mark color, readable on the current theme.
    pub fn color(self) -> Color32 {
        crate::theme::adapt(self.raw_color())
    }

    /// The surface color: paper on a light card, a night shade on a dark one.
    pub fn surface(self, light: bool) -> Color32 {
        let [r, g, b] = if light { self.paper() } else { self.night() };
        Color32::from_rgb(r, g, b)
    }

    /// Sticky Notes' paper colors (yellow, green, pink, purple, blue, gray) and
    /// Keep's peach and teal.
    fn paper(self) -> [u8; 3] {
        match self {
            Tint::Yellow => [255, 242, 171],
            Tint::Orange => [255, 217, 184],
            Tint::Pink => [255, 204, 229],
            Tint::Purple => [231, 207, 255],
            Tint::Blue => [205, 233, 255],
            Tint::Teal => [197, 236, 236],
            Tint::Green => [203, 241, 196],
            Tint::Gray => [237, 238, 241],
        }
    }

    /// Dark shades that still read as the color, and stay mostly grey so the
    /// layer doesn't turn into a paint box at night. Gray is the glass itself.
    fn night(self) -> [u8; 3] {
        match self {
            Tint::Yellow => [64, 59, 40],
            Tint::Orange => [68, 52, 40],
            Tint::Pink => [66, 44, 56],
            Tint::Purple => [54, 48, 74],
            Tint::Blue => [36, 50, 70],
            Tint::Teal => [34, 58, 60],
            Tint::Green => [38, 60, 46],
            Tint::Gray => [43, 47, 54],
        }
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
    /// The whole card in the color's surface: paper by day, a night shade in
    /// the dark (Google Keep).
    Fill,
    /// A colored outline over a faint tint (Obsidian Canvas).
    Outline,
    /// Paper, and while the card is hovered a darker band along the top, the
    /// way a Sticky Notes window shows its bar.
    Paper,
}

/// How the kind is marked in a card's meta line (the mark opens the kind menu).
/// The keys keep their old names, from when the mark was an icon in a corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconSpot {
    /// The kind's glyph alone.
    BottomRight,
    /// A dot in the kind's color and the kind's name.
    TopLeft,
    /// The dot and name, only while the card is hovered or selected.
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
    pub const ALL: [Marker; 9] = [
        Marker::None,
        Marker::Glow,
        Marker::StripTop,
        Marker::StripLeft,
        Marker::Tint,
        Marker::Border,
        Marker::Fill,
        Marker::Paper,
        Marker::Outline,
    ];

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
            Marker::Paper => "paper",
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
            Marker::Paper => "Бумага с шапкой",
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
            IconSpot::BottomRight => "Значок",
            IconSpot::TopLeft => "Точка и название",
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
                "marker" => {
                    style.marker = Marker::ALL
                        .into_iter()
                        .find(|m| m.key() == value)
                        .unwrap_or(style.marker)
                }
                "icon" => {
                    style.icon = IconSpot::ALL
                        .into_iter()
                        .find(|i| i.key() == value)
                        .unwrap_or(style.icon)
                }
                "strength" => {
                    style.strength = num(value).map_or(style.strength, |n| n.min(100) as u8)
                }
                "bold" => style.bold_icon = value == "1",
                "strip" => style.strip = num(value).map_or(style.strip, |n| n.clamp(1, 12) as u8),
                "radius" => style.radius = num(value).map_or(style.radius, |n| n.min(20) as u8),
                "shadow" => style.shadow = num(value).map_or(style.shadow, |n| n.min(100) as u8),
                "opacity" => {
                    style.opacity = num(value).map_or(style.opacity, |n| n.clamp(30, 100) as u8)
                }
                "font" => {
                    style.font = crate::theme::CardFont::ALL
                        .into_iter()
                        .find(|f| f.key() == value)
                        .unwrap_or(style.font)
                }
                "text" => style.text = num(value).map_or(style.text, |n| n.clamp(22, 40) as u8),
                "background" => {
                    style.background =
                        crate::theme::CardBackground::from_key(value).unwrap_or(style.background)
                }
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
    CardStyle {
        marker,
        icon,
        strength,
        bold_icon,
        strip,
        radius,
        shadow,
        opacity,
        font,
        text,
        background,
    }
}

use crate::theme::{CardBackground as B, CardFont as F};

pub const PRESETS: [Preset; 8] = [
    Preset {
        name: "Ebb",
        hint: "Стекло и свечение цвета",
        style: style(
            Marker::Glow,
            IconSpot::TopLeft,
            50,
            false,
            5,
            12,
            50,
            85,
            F::System,
            29,
            B::Theme,
        ),
    },
    Preset {
        name: "Sticky Notes",
        hint: "Цветная бумага, шапка при наведении",
        style: style(
            Marker::Paper,
            IconSpot::Hover,
            50,
            false,
            8,
            8,
            35,
            100,
            F::System,
            29,
            B::Theme,
        ),
    },
    Preset {
        name: "Google Keep",
        hint: "Пастель днём, ночные тона в темноте",
        style: style(
            Marker::Fill,
            IconSpot::Hover,
            50,
            false,
            5,
            8,
            0,
            100,
            F::System,
            28,
            B::Theme,
        ),
    },
    Preset {
        name: "Obsidian Canvas",
        hint: "Рамка и лёгкий тон",
        style: style(
            Marker::Outline,
            IconSpot::TopLeft,
            60,
            false,
            5,
            8,
            0,
            95,
            F::System,
            28,
            B::Theme,
        ),
    },
    Preset {
        name: "Trello",
        hint: "Цветная метка слева",
        style: style(
            Marker::StripLeft,
            IconSpot::TopLeft,
            80,
            false,
            4,
            6,
            20,
            100,
            F::System,
            28,
            B::Theme,
        ),
    },
    Preset {
        name: "Бумажный стикер",
        hint: "Заливка, тень, от руки",
        style: style(
            Marker::Fill,
            IconSpot::Hover,
            60,
            false,
            5,
            2,
            70,
            100,
            F::SegoePrint,
            29,
            B::Theme,
        ),
    },
    Preset {
        name: "Минимализм",
        hint: "Только жирная иконка",
        style: style(
            Marker::None,
            IconSpot::BottomRight,
            50,
            true,
            5,
            12,
            20,
            85,
            F::System,
            29,
            B::Theme,
        ),
    },
    Preset {
        name: "Windows",
        hint: "Фон как у панели задач",
        style: style(
            Marker::None,
            IconSpot::TopLeft,
            50,
            false,
            5,
            8,
            35,
            96,
            F::System,
            28,
            B::Taskbar,
        ),
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
    /// Folded to one line on the layer; `size` stays what it opens to.
    pub collapsed: bool,
    /// Set while it was an idea; shown only while it is one.
    pub idea_status: Option<IdeaStatus>,
}

/// Height of a collapsed card.
pub const COLLAPSED_H: f32 = 40.0;

impl Card {
    /// The card's color: picked by hand, else its kind's.
    pub fn hue(&self) -> Tint {
        self.tint.unwrap_or(self.kind.tint())
    }

    /// The mark color of [`Card::hue`].
    /// The size the card takes on the layer: one line while collapsed.
    pub fn shown_size(&self) -> Vec2 {
        if self.collapsed {
            vec2(self.size.x, COLLAPSED_H)
        } else {
            self.size
        }
    }

    /// The line a collapsed card shows: the first line of its text as it reads;
    /// for a Private card, its label (the layer never holds the value).
    pub fn collapsed_line(&self) -> String {
        if self.kind == Kind::Private {
            return if self.title.is_empty() {
                PRIVATE_PLACEHOLDER.to_owned()
            } else {
                self.title.clone()
            };
        }
        let text = if self.title.is_empty() {
            self.body.clone()
        } else {
            format!("{}\n{}", self.title, self.body)
        };
        crate::rich_text::strip_markup(&text)
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or_default()
            .to_owned()
    }

    pub fn accent(&self) -> Color32 {
        self.hue().color()
    }

    /// The caption of a card brought back from the archive, or of one on the
    /// layer whose reminder has come due (the Today part of spec 01).
    pub fn resurface_reason(&self, now: i64) -> Option<String> {
        if self.placement == Placement::Rediscover {
            return Some(crate::resurface::reason(
                now,
                self.created_at,
                self.review_at,
            ));
        }
        self.due_reminder(now)
            .map(|at| crate::resurface::reminder_reason(now, at))
    }

    /// When this card's reminder came due, if it has and nobody has seen it since.
    pub fn due_reminder(&self, now: i64) -> Option<i64> {
        self.review_at.filter(|at| *at <= now && !self.archived)
    }
}

const COMMANDS: &[&str] = &[
    "ssh",
    "scp",
    "rsync",
    "git",
    "cd",
    "ls",
    "curl",
    "wget",
    "docker",
    "kubectl",
    "helm",
    "npm",
    "pnpm",
    "yarn",
    "npx",
    "cargo",
    "pip",
    "python",
    "node",
    "ping",
    "sudo",
    "psql",
    "mysql",
    "redis-cli",
    "telnet",
    "winget",
    "choco",
    "pwsh",
    "powershell",
    "systemctl",
    "journalctl",
    "tail",
    "cat",
    "export",
    "set",
];

/// A Reference line to set in monospace: a command, or a short line with a path,
/// address or host in it ("vpn.staging.internal", "10.0.4.12:22", `C:\tools`).
pub fn looks_technical(line: &str) -> bool {
    let line = line.trim();
    let words: Vec<&str> = line.split_whitespace().collect();
    let Some(first) = words.first() else {
        return false;
    };
    if matches!(*first, "$" | ">") {
        return true;
    }
    let technical = words
        .iter()
        .any(|w| technical_word(w.trim_end_matches([',', ';', ')'])));
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
    if w.contains("://")
        || w.starts_with('/')
        || w.starts_with("~/")
        || w.starts_with("./")
        || drive
        || w.contains('\\')
    {
        return true;
    }
    // Keys, hashes, tokens: long runs of letters mixed with digits.
    if w.len() >= 12
        && w.bytes().any(|b| b.is_ascii_digit())
        && w.bytes().any(|b| b.is_ascii_alphabetic())
    {
        return true;
    }
    // host, host:port, user@host, 10.0.4.12
    let host = w.rsplit('@').next().unwrap_or(w);
    let host = match host.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => host,
    };
    let labels: Vec<&str> = host.split('.').collect();
    let ip = labels.len() == 4
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.len() <= 3 && l.bytes().all(|b| b.is_ascii_digit()));
    let name = labels.len() >= 2
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        && labels
            .last()
            .is_some_and(|l| l.len() >= 2 && l.bytes().all(|b| b.is_ascii_alphabetic()))
        && (labels.len() >= 3 || w.contains(['@', ':']));
    ip || name
}

/// Host of the first http(s) URL in the text, without "www.".
pub fn link_domain(text: &str) -> Option<&str> {
    let at = text
        .find("https://")
        .map(|i| i + 8)
        .or_else(|| text.find("http://").map(|i| i + 7))?;
    let rest = &text[at..];
    let end = rest
        .find(|c: char| c.is_whitespace() || matches!(c, '/' | '?' | '#' | ')' | ',' | '"'))
        .unwrap_or(rest.len());
    let host = rest[..end].rsplit('@').next().unwrap_or("");
    let host = host
        .strip_prefix("www.")
        .unwrap_or(host)
        .trim_end_matches(['.', ':']);
    (!host.is_empty()).then_some(host)
}

/// The first http(s) address in the text, without the punctuation after it.
pub fn first_url(text: &str) -> Option<&str> {
    let at = [text.find("https://"), text.find("http://")]
        .into_iter()
        .flatten()
        .min()?;
    let rest = &text[at..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some(rest[..end].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '"', '\'']))
}

/// A Prompt's name: its first line, when more text follows and the line is short.
pub fn prompt_name(body: &str) -> Option<(&str, &str)> {
    let body = body.trim_start();
    let (first, rest) = body.split_once('\n')?;
    let (first, rest) = (first.trim(), rest.trim());
    (!first.is_empty() && !rest.is_empty() && first.chars().count() <= 80).then_some((first, rest))
}

/// `{{name}}` placeholders in a Prompt: the byte range of each and its name.
/// A name is what's between the braces, trimmed: one line, no braces, ≤ 40 chars.
pub(crate) fn prompt_placeholders(text: &str) -> Vec<(std::ops::Range<usize>, &str)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(open) = text[from..].find("{{").map(|i| from + i) {
        let Some(close) = text[open + 2..].find("}}").map(|i| open + 2 + i) else {
            break;
        };
        let inner = &text[open + 2..close];
        let name = inner.trim();
        let valid =
            !name.is_empty() && name.chars().count() <= 40 && !inner.contains(['{', '}', '\n']);
        if valid {
            found.push((open..close + 2, name));
            from = close + 2;
        } else {
            // "{{{x}}}" or a stray "{{": look again one brace on.
            from = open + 1;
        }
    }
    found
}

/// A Prompt's variables, each once, in the order they first appear.
pub fn prompt_variables(text: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for (_, name) in prompt_placeholders(text) {
        if !names.iter().any(|n| n == name) {
            names.push(name.to_owned());
        }
    }
    names
}

/// The Prompt with each `{{name}}` replaced by its value; names without one stay as written.
pub fn fill_prompt(text: &str, values: &[(String, String)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (range, name) in prompt_placeholders(text) {
        if let Some((_, value)) = values.iter().find(|(n, _)| n == name) {
            out.push_str(&text[last..range.start]);
            out.push_str(value);
            last = range.end;
        }
    }
    out.push_str(&text[last..]);
    out
}

/// A card's tags: the `#words` of its text (without markup), lowercased, once
/// each, trailing `,.;` dropped.
pub fn tags_of(plain: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for tag in plain.split_whitespace().filter_map(tag_word) {
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    tags
}

/// The tag a word stands for, if it's a `#tag`.
fn tag_word(word: &str) -> Option<String> {
    let tag = word
        .strip_prefix('#')?
        .trim_end_matches([',', '.', ';'])
        .to_lowercase();
    (!tag.is_empty()).then_some(tag)
}

/// A tag typed into the tag field: without `#`, lowercased, words joined by `-`
/// (a tag lives in the text as one word). None when nothing usable is left.
pub fn normalize_tag(input: &str) -> Option<String> {
    let words: Vec<&str> = input
        .split(|c: char| c.is_whitespace() || c == '#')
        .filter(|w| !w.is_empty())
        .collect();
    let tag = words
        .join("-")
        .trim_end_matches([',', '.', ';'])
        .to_lowercase();
    (!tag.is_empty() && tag.chars().count() <= 40).then_some(tag)
}

/// The text with `#tag` added: on the last line if that line is only tags,
/// else on a line of its own. Unchanged if the tag is there already.
pub fn with_tag(text: &str, tag: &str) -> String {
    let plain = crate::rich_text::strip_markup(text);
    if tags_of(&plain).iter().any(|t| t == tag) {
        return text.to_owned();
    }
    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        return format!("#{tag}");
    }
    let last_line_is_tags = plain
        .trim_end()
        .lines()
        .last()
        .is_some_and(|l| l.split_whitespace().all(|w| tag_word(w).is_some()));
    if last_line_is_tags {
        format!("{trimmed} #{tag}")
    } else {
        format!("{trimmed}\n#{tag}")
    }
}

/// The text without its `#tag` words (styles of the rest kept). A space goes
/// with each; a line left empty goes too.
pub fn without_tag(text: &str, tag: &str) -> String {
    let (plain, styles) = crate::rich_text::parse(text);
    let chars: Vec<char> = plain.chars().collect();
    let mut keep = vec![true; chars.len()];
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
        let word: String = chars[start..i].iter().collect();
        if tag_word(&word).is_some_and(|t| t == tag) {
            let (mut from, mut to) = (start, i);
            if from > 0 && chars[from - 1] == ' ' {
                from -= 1;
            } else if to < chars.len() && chars[to] == ' ' {
                to += 1;
            }
            keep[from..to].iter_mut().for_each(|k| *k = false);
        }
    }
    // A line that had a tag taken out and holds nothing else now goes, with its line break.
    let mut line_start = 0;
    for end in (0..=chars.len()).filter(|&j| j == chars.len() || chars[j] == '\n') {
        let line = line_start..end;
        let touched = keep[line.clone()].iter().any(|k| !k);
        let empty = line.clone().all(|j| !keep[j] || chars[j].is_whitespace());
        if touched && empty {
            line.clone().for_each(|j| keep[j] = false);
            if end < chars.len() {
                keep[end] = false;
            } else if line_start > 0 {
                keep[line_start - 1] = false;
            }
        }
        line_start = end + 1;
    }
    let (plain, styles): (String, Vec<crate::rich_text::Style>) = chars
        .iter()
        .zip(styles)
        .zip(&keep)
        .filter(|(_, k)| **k)
        .map(|((c, s), _)| (*c, s))
        .unzip();
    crate::rich_text::serialize(&plain, &styles)
}

/// Tags to offer while typing `typed`: the most used first, starting with what's
/// typed, none the card has already.
pub fn suggest_tags(
    counts: &[(String, i64)],
    typed: &str,
    have: &[String],
    limit: usize,
) -> Vec<String> {
    let prefix = typed.trim().trim_start_matches('#').to_lowercase();
    counts
        .iter()
        .filter(|(t, _)| t.starts_with(&prefix) && !have.contains(t) && *t != prefix)
        .take(limit)
        .map(|(t, _)| t.clone())
        .collect()
}

/// What copying a Prompt takes: its text without the name line.
pub fn prompt_text(title: &str, body: &str) -> String {
    let full = if title.is_empty() {
        body.to_owned()
    } else {
        format!("{title}\n{body}")
    };
    let full = crate::rich_text::strip_markup(&full);
    match prompt_name(&full) {
        Some((_, rest)) => rest.to_owned(),
        None => full.trim().to_owned(),
    }
}

/// A list marker at the start of a line: a bullet, or a check box, the way
/// Sticky Notes' lists and Markdown's task lists are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListMark {
    Bullet,
    Todo,
    Done,
}

impl ListMark {
    /// Space in front of the line, where the mark is drawn in place of the marker.
    pub fn indent(self) -> f32 {
        match self {
            ListMark::Bullet => 16.0,
            ListMark::Todo | ListMark::Done => 22.0,
        }
    }
}

/// The marker a line starts with, and how many chars it takes (the space after
/// it included). A lone "-" isn't a bullet; "- [ ]" with nothing after it is a
/// check box.
pub fn list_mark(line: &str) -> Option<(ListMark, usize)> {
    let l = line.trim_start();
    let pad = line.chars().count() - l.chars().count();
    for (marker, mark) in [
        ("- [ ]", ListMark::Todo),
        ("* [ ]", ListMark::Todo),
        ("- [x]", ListMark::Done),
        ("- [X]", ListMark::Done),
        ("* [x]", ListMark::Done),
        ("* [X]", ListMark::Done),
    ] {
        if let Some(rest) = l.strip_prefix(marker)
            && (rest.is_empty() || rest.starts_with(' '))
        {
            return Some((
                mark,
                pad + marker.len() + usize::from(rest.starts_with(' ')),
            ));
        }
    }
    for marker in ["- ", "* ", "\u{2022} "] {
        if l.starts_with(marker) && l.len() > marker.len() {
            return Some((ListMark::Bullet, pad + marker.chars().count()));
        }
    }
    None
}

/// The text with the check box on line `line` (counted in the text as written;
/// markup adds no lines) ticked or cleared.
pub fn toggle_check(text: &str, line: usize) -> String {
    text.split_inclusive('\n')
        .enumerate()
        .map(|(i, l)| {
            if i != line {
                return l.to_owned();
            }
            if let Some(at) = l.find("[ ]") {
                format!("{}[x]{}", &l[..at], &l[at + 3..])
            } else if let Some(at) = l.find("[x]").or_else(|| l.find("[X]")) {
                format!("{}[ ]{}", &l[..at], &l[at + 3..])
            } else {
                l.to_owned()
            }
        })
        .collect()
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
    const SECRETS: &[&str] = &[
        "pass",
        "парол",
        "token",
        "токен",
        "secret",
        "секрет",
        "key",
        "ключ",
        "begin",
        "root@",
        "ssh-",
    ];
    let lower = line.to_lowercase();
    let token_like = |w: &str| {
        w.chars().count() >= 16
            && w.chars().any(|c| c.is_ascii_digit())
            && w.chars().any(char::is_alphabetic)
    };
    let ip_like = |w: &str| {
        let parts: Vec<&str> = w
            .trim_matches(|c: char| !c.is_ascii_digit())
            .split('.')
            .collect();
        parts.len() == 4
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.len() <= 3 && p.chars().all(|c| c.is_ascii_digit()))
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
    "vpn",
    "ssh",
    "host",
    "server",
    "сервер",
    "endpoint",
    "path",
    "ip",
    "port",
    "порт",
];

/// Looks like a credential: passwords, tokens, keys.
pub fn looks_secret(text: &str) -> bool {
    let lower = text.to_lowercase();
    const LABELS: &[&str] = &[
        "пароль",
        "password",
        "passwd",
        "pass",
        "pwd",
        "логин",
        "login",
        "token",
        "токен",
        "api key",
        "api_key",
        "apikey",
        "secret",
        "секрет",
        "private key",
        "pin",
        "пин",
    ];
    for label in LABELS {
        for (start, _) in lower.match_indices(label) {
            let before = lower[..start].chars().next_back();
            let end = start + label.len();
            let after = lower[end..].chars().next();
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_')
                || after.is_some_and(|c| c.is_alphanumeric() || c == '_')
            {
                continue;
            }

            let suffix = &lower[end..];
            let explicit =
                suffix.trim_start().starts_with(':') || suffix.trim_start().starts_with('=');
            let value = suffix
                .trim_start_matches(|c: char| c == ':' || c == '=' || c.is_whitespace())
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches(|c: char| c == '"' || c == '\'');
            let length = value.chars().count();
            let plausible_value = length >= 8
                || (length >= 5 && value.chars().any(|c| c.is_ascii_digit()))
                || (length >= 3 && value.chars().all(|c| c.is_ascii_digit()));
            if (explicit && !value.is_empty()) || plausible_value {
                return true;
            }
        }
    }
    // OpenAI-style keys: sk- followed by a long run of key characters.
    lower.match_indices("sk-").any(|(i, _)| {
        lower[i + 3..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .count()
            >= 20
    })
}

pub fn parse_capture(input: &str) -> Parsed {
    parse_capture_with(input, true)
}

/// `detect_secret` off: the user took the Private chip off a note that only
/// looked like a credential; an explicit `private:` prefix still counts.
pub fn parse_capture_with(input: &str, detect_secret: bool) -> Parsed {
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
        // A credential typed without a prefix is still encrypted, not indexed.
        if detect_secret && looks_secret(&lower) {
            kind = Kind::Private;
        } else if lower.contains("http://") || lower.contains("https://") {
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
    const UNITS: &[&str] = &[
        "дн",
        "день",
        "недел",
        "месяц",
        "час",
        "day",
        "week",
        "month",
    ];
    let after = lower
        .split("через ")
        .nth(1)
        .or_else(|| lower.split("in ").nth(1));
    after.is_some_and(|a| {
        a.split_whitespace()
            .take(2)
            .any(|w| UNITS.iter().any(|u| w.starts_with(u)))
    })
}

/// Place a new card at the first free slot of a coarse grid.
pub fn free_slot(cards: &[Card], area: Vec2) -> Pos2 {
    free_slot_for(cards, DEFAULT_SIZE, area)
}

/// A free grid slot for a card of `size` (a card back from the archive keeps
/// the size it had, so the slot must hold that, not the default).
pub fn free_slot_for(cards: &[Card], size: Vec2, area: Vec2) -> Pos2 {
    let step = vec2(DEFAULT_SIZE.x + 16.0, DEFAULT_SIZE.y + 16.0);
    let origin = pos2(32.0, 72.0);
    let cols = ((area.x - origin.x) / step.x).floor().max(1.0) as usize;
    let rows = ((area.y - origin.y) / step.y).floor().max(1.0) as usize;
    for row in 0..rows {
        for col in 0..cols {
            let p = origin + vec2(col as f32 * step.x, row as f32 * step.y);
            let slot = egui::Rect::from_min_size(p, size);
            // Past the layer's edge only when nothing larger could fit at all.
            if (slot.max.x > area.x || slot.max.y > area.y) && (col > 0 || row > 0) {
                continue;
            }
            if !cards
                .iter()
                .filter(|c| !c.archived)
                .any(|c| egui::Rect::from_min_size(c.pos, c.shown_size()).intersects(slot))
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
                    .any(|c| egui::Rect::from_min_size(c.pos, c.shown_size()).intersects(slot))
        })
        .unwrap_or_else(|| free_slot_for(cards, of.size, area))
}

/// Space between cards that stick side by side: none, they sit edge to edge.
pub const SNAP_GAP: f32 = 0.0;
/// How close an edge has to come before it sticks.
pub const SNAP_DISTANCE: f32 = 10.0;
/// Cards at most this far apart (across the snapping axis) align their edges.
const ALIGN_REACH: f32 = 64.0;
/// Margins of the layer that edges stick to (the header takes the top).
const LAYER_MARGIN: egui::Margin = egui::Margin {
    left: 32,
    right: 32,
    top: 72,
    bottom: 32,
};

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
fn axis(
    rect: egui::Rect,
    others: &[egui::Rect],
    bounds: egui::Rect,
    x: bool,
    edges: (bool, bool),
) -> Candidate {
    // (low edge, high edge) along the axis, and the range across it.
    let along = |r: egui::Rect| {
        if x {
            (r.left(), r.right())
        } else {
            (r.top(), r.bottom())
        }
    };
    let across = |r: egui::Rect| {
        if x {
            (r.top(), r.bottom())
        } else {
            (r.left(), r.right())
        }
    };
    let line = |at: f32, (a, b): (f32, f32)| {
        if x {
            [pos2(at, a), pos2(at, b)]
        } else {
            [pos2(a, at), pos2(b, at)]
        }
    };
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
        (
            bounds.left() + LAYER_MARGIN.left as f32,
            bounds.right() - LAYER_MARGIN.right as f32,
        )
    } else {
        (
            bounds.top() + LAYER_MARGIN.top as f32,
            bounds.bottom() - LAYER_MARGIN.bottom as f32,
        )
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
    Snapped {
        rect: rect.translate(shift),
        guides,
        x: dx.is_some(),
        y: dy.is_some(),
    }
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
            r.min.x = (r.min.x + delta.x)
                .max(bounds.left())
                .min(r.max.x - MIN_SIZE.x);
        }
        if self.right {
            r.max.x = (r.max.x + delta.x).clamp(r.min.x + MIN_SIZE.x, bounds.right());
        }
        if self.top {
            r.min.y = (r.min.y + delta.y)
                .max(bounds.top())
                .min(r.max.y - MIN_SIZE.y);
        }
        if self.bottom {
            r.max.y = (r.max.y + delta.y).clamp(r.min.y + MIN_SIZE.y, bounds.bottom());
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

    pub fn resize_direction(self) -> egui::ResizeDirection {
        use egui::ResizeDirection::*;
        match (self.left, self.right, self.top, self.bottom) {
            (true, false, true, false) => NorthWest,
            (false, true, true, false) => NorthEast,
            (true, false, false, true) => SouthWest,
            (false, true, false, true) => SouthEast,
            (true, false, false, false) => West,
            (false, true, false, false) => East,
            (false, false, true, false) => North,
            (false, false, false, true) => South,
            _ => unreachable!("resize handle must move at least one edge"),
        }
    }
}

/// Sticks the edges of a card being resized that the handle moves.
pub fn snap_resize(
    rect: egui::Rect,
    others: &[egui::Rect],
    bounds: egui::Rect,
    sides: Sides,
) -> Snapped {
    // Moving the low edge by d shrinks the rect by d; the high edge grows it.
    let fits = |c: Candidate, low: bool, len: f32, min: f32| {
        c.filter(|(d, _)| if low { len - d } else { len + d } >= min)
    };
    let dx = fits(
        axis(rect, others, bounds, true, (sides.left, sides.right)),
        sides.left,
        rect.width(),
        MIN_SIZE.x,
    );
    let dy = fits(
        axis(rect, others, bounds, false, (sides.top, sides.bottom)),
        sides.top,
        rect.height(),
        MIN_SIZE.y,
    );
    let mut out = rect;
    if let Some((d, _)) = dx {
        if sides.left {
            out.min.x += d
        } else {
            out.max.x += d
        }
    }
    if let Some((d, _)) = dy {
        if sides.top {
            out.min.y += d
        } else {
            out.max.y += d
        }
    }
    let guides = dx
        .map(|(_, g)| g)
        .into_iter()
        .chain(dy.map(|(_, g)| g))
        .collect();
    Snapped {
        rect: out,
        guides,
        x: dx.is_some(),
        y: dy.is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f32, y: f32, w: f32, h: f32) -> egui::Rect {
        egui::Rect::from_min_size(pos2(x, y), vec2(w, h))
    }

    const BOUNDS: egui::Rect = egui::Rect {
        min: pos2(0.0, 0.0),
        max: pos2(2000.0, 1200.0),
    };

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
        let br = Sides {
            right: true,
            bottom: true,
            ..Default::default()
        };
        let other = r(700.0, 300.0, 280.0, 150.0);
        let s = snap_resize(r(400.0, 320.0, 290.0, 120.0), &[other], BOUNDS, br);
        assert_eq!(s.rect.min, pos2(400.0, 320.0));
        assert_eq!(s.rect.right(), 700.0 - SNAP_GAP);
        // Bottom aligns with the neighbour's.
        assert_eq!(s.rect.bottom(), 450.0);

        let tiny = r(400.0, 320.0, MIN_SIZE.x + 2.0, 120.0);
        let s = snap_resize(
            tiny,
            &[r(
                400.0 + MIN_SIZE.x + 2.0 + SNAP_GAP - 8.0,
                300.0,
                100.0,
                150.0,
            )],
            BOUNDS,
            br,
        );
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
            collapsed: false,
            idea_status: None,
        }
    }

    #[test]
    fn collapsed_card_shows_its_first_line_and_takes_one_row() {
        let mut card = card_at(1, 0.0, 0.0);
        card.body = "\n  **Купить** молоко  \nи хлеб".to_owned();
        card.collapsed = true;
        assert_eq!(card.collapsed_line(), "Купить молоко");
        assert_eq!(card.shown_size(), vec2(DEFAULT_SIZE.x, COLLAPSED_H));
        // The slot under a collapsed card is free past its one row.
        let below = free_slot_for(
            std::slice::from_ref(&card),
            vec2(DEFAULT_SIZE.x, 40.0),
            vec2(DEFAULT_SIZE.x + 64.0, 400.0),
        );
        assert!(below.y < DEFAULT_SIZE.y, "{below:?}");
        card.kind = Kind::Private;
        card.title = "Wi-Fi офис".to_owned();
        card.body.clear();
        assert_eq!(card.collapsed_line(), "Wi-Fi офис");
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
        for line in [
            "просто мысль",
            "т.е. позже",
            "e.g. later",
            "ping Alex about the release",
            "Wi-Fi офис",
            "",
        ] {
            assert!(!looks_technical(line), "{line}");
        }
    }

    #[test]
    fn link_domains() {
        assert_eq!(
            first_url("см. (https://egui.rs/x). и всё"),
            Some("https://egui.rs/x")
        );
        assert_eq!(first_url("без ссылки"), None);
        assert_eq!(
            link_domain("демо https://www.egui.rs/#demo"),
            Some("egui.rs")
        );
        assert_eq!(
            link_domain("(http://user@host.dev:8080/x)"),
            Some("host.dev:8080")
        );
        assert_eq!(link_domain("без ссылки"), None);
    }

    #[test]
    fn tags_come_from_the_text_once_each() {
        assert_eq!(
            tags_of("#Идея про #pricing, и снова #идея. # и #"),
            ["идея", "pricing"]
        );
        assert_eq!(
            normalize_tag("  #Новый Тег, "),
            Some("новый-тег".to_owned())
        );
        assert_eq!(normalize_tag(" # "), None);
    }

    #[test]
    fn adding_a_tag_writes_it_into_the_text() {
        assert_eq!(with_tag("", "дом"), "#дом");
        assert_eq!(with_tag("Купить молоко\n", "дом"), "Купить молоко\n#дом");
        assert_eq!(
            with_tag("Купить молоко\n#еда", "дом"),
            "Купить молоко\n#еда #дом"
        );
        assert_eq!(with_tag("Купить #Дом молоко", "дом"), "Купить #Дом молоко");
        // After a styled end the tag is plain text.
        assert_eq!(
            crate::rich_text::strip_markup(&with_tag("**важно**", "дом")),
            "важно\n#дом"
        );
    }

    #[test]
    fn removing_a_tag_takes_its_words_out_and_keeps_styles() {
        assert_eq!(
            without_tag("Купить #дом молоко #Дом,", "дом"),
            "Купить молоко"
        );
        assert_eq!(
            without_tag("Купить молоко\n#дом\nпотом", "дом"),
            "Купить молоко\nпотом"
        );
        assert_eq!(
            without_tag("Купить молоко\n#еда #дом", "дом"),
            "Купить молоко\n#еда"
        );
        assert_eq!(without_tag("#дом", "дом"), "");
        let styled = crate::rich_text::serialize(
            "жирный #дом текст",
            &[crate::rich_text::Style {
                bold: true,
                ..Default::default()
            }; 17],
        );
        let out = without_tag(&styled, "дом");
        let (plain, styles) = crate::rich_text::parse(&out);
        assert_eq!(plain, "жирный текст");
        assert!(styles.iter().all(|s| s.bold));
        // Words that only look alike stay.
        assert_eq!(without_tag("#домик и #дом2", "дом"), "#домик и #дом2");
    }

    #[test]
    fn suggestions_follow_what_is_typed_by_use() {
        let counts = [
            ("работа".to_owned(), 9),
            ("рецепт".to_owned(), 4),
            ("дом".to_owned(), 7),
        ];
        assert_eq!(
            suggest_tags(&counts, "", &[], 5),
            ["работа", "рецепт", "дом"]
        );
        assert_eq!(suggest_tags(&counts, "#Ре", &[], 5), ["рецепт"]);
        assert_eq!(
            suggest_tags(&counts, "р", &["работа".to_owned()], 5),
            ["рецепт"]
        );
    }

    #[test]
    fn prompt_names() {
        assert_eq!(
            prompt_name("Ревью кода\nПосмотри на {{diff}}"),
            Some(("Ревью кода", "Посмотри на {{diff}}"))
        );
        assert_eq!(prompt_name("одна строка"), None);
    }

    #[test]
    fn prompt_variables_are_found_once_and_filled() {
        let text = "Напиши о {{topic}} в тоне {{ tone }}. Ещё раз: {{topic}}. {{}} и {{\nнет}} — не переменные.";
        assert_eq!(prompt_variables(text), ["topic", "tone"]);
        let values = [
            ("topic".to_owned(), "SQLite".to_owned()),
            ("tone".to_owned(), "сухом".to_owned()),
        ];
        assert_eq!(
            fill_prompt(text, &values),
            "Напиши о SQLite в тоне сухом. Ещё раз: SQLite. {{}} и {{\nнет}} — не переменные."
        );
        // A variable without a value stays as written; extra braces around one are kept.
        assert_eq!(fill_prompt("{{a}} {{b}}", &values[..0]), "{{a}} {{b}}");
        assert_eq!(prompt_variables("{{{x}}}"), ["x"]);
        assert_eq!(
            fill_prompt("{{{x}}}", &[("x".to_owned(), "1".to_owned())]),
            "{1}"
        );
        assert!(prompt_variables("{{ не закрыта").is_empty());
    }

    #[test]
    fn list_markers_are_found_and_check_boxes_toggle() {
        assert_eq!(list_mark("- [ ] купить молоко"), Some((ListMark::Todo, 6)));
        assert_eq!(list_mark("  * [X] сделано"), Some((ListMark::Done, 8)));
        assert_eq!(list_mark("- [ ]"), Some((ListMark::Todo, 5)));
        assert_eq!(list_mark("- пункт"), Some((ListMark::Bullet, 2)));
        assert_eq!(list_mark("\u{2022} пункт"), Some((ListMark::Bullet, 2)));
        assert_eq!(list_mark("-"), None);
        assert_eq!(list_mark("- [y] нет"), Some((ListMark::Bullet, 2)));
        assert_eq!(list_mark("просто текст"), None);
        let text = "план\n- [ ] **молоко**\n- [x] хлеб";
        assert_eq!(toggle_check(text, 1), "план\n- [x] **молоко**\n- [x] хлеб");
        assert_eq!(toggle_check(text, 2), "план\n- [ ] **молоко**\n- [ ] хлеб");
        assert_eq!(toggle_check(text, 0), text);
    }

    #[test]
    fn copying_a_prompt_leaves_out_its_name() {
        assert_eq!(
            prompt_text("", "Ревью кода\nПосмотри на **{{diff}}**"),
            "Посмотри на {{diff}}"
        );
        assert_eq!(prompt_text("", "Переведи {{text}}"), "Переведи {{text}}");
    }

    #[test]
    fn card_style_round_trips_and_survives_junk() {
        let s = CardStyle {
            radius: 4,
            shadow: 10,
            opacity: 60,
            font: crate::theme::CardFont::Georgia,
            text: 32,
            background: crate::theme::CardBackground::Custom([255, 242, 171]),
            ..PRESETS[3].style
        };
        assert_eq!(CardStyle::from_setting(&s.to_setting()), s);
        let junk = CardStyle::from_setting("marker=what strength=999 color=red radius");
        assert_eq!(
            junk,
            CardStyle {
                strength: 100,
                ..CardStyle::default()
            }
        );
    }

    #[test]
    fn kind_keys_and_tints_round_trip() {
        assert_eq!(Placement::parse("desktop"), Placement::Desktop);
        assert_eq!(Kind::Note.key(), '1');
        assert_eq!(Kind::Private.key(), '8');
        for t in Tint::ALL {
            assert_eq!(Tint::parse(t.as_str()), Some(t));
        }
        // Every kind has a tint of its own, so its color and a picked one are painted alike.
        let mut tints: Vec<Tint> = Kind::ALL.iter().map(|k| k.tint()).collect();
        tints.dedup();
        assert_eq!(tints.len(), Kind::ALL.len());
        // The paper is Sticky Notes' yellow, the night shade is dark.
        assert_eq!(Tint::Yellow.surface(true), Color32::from_rgb(255, 242, 171));
        assert!(Tint::Yellow.surface(false).r() < 100);
        assert_eq!(Marker::ALL.iter().filter(|m| m.key() == "paper").count(), 1);
    }

    #[test]
    fn duplicate_goes_right_against_the_original_or_below() {
        let area = vec2(1920.0, 1080.0);
        let a = card_at(1, 400.0, 300.0);
        assert_eq!(
            beside(&[a.clone()], &a, area),
            pos2(400.0 + DEFAULT_SIZE.x, 300.0)
        );
        // Right is taken: below.
        let b = card_at(2, 400.0 + DEFAULT_SIZE.x, 300.0);
        assert_eq!(
            beside(&[a.clone(), b], &a, area),
            pos2(400.0, 300.0 + DEFAULT_SIZE.y)
        );
        // At the right edge of the layer: not off-screen.
        let edge = card_at(3, area.x - DEFAULT_SIZE.x, 300.0);
        assert_eq!(
            beside(&[edge.clone()], &edge, area),
            pos2(edge.pos.x, 300.0 + DEFAULT_SIZE.y)
        );
    }

    #[test]
    fn resize_from_the_left_and_top_moves_those_edges() {
        let tl = Sides {
            left: true,
            top: true,
            ..Default::default()
        };
        assert_eq!(tl.resize_direction(), egui::ResizeDirection::NorthWest);
        let start = r(400.0, 400.0, 280.0, 150.0);
        let grown = tl.resize(start, vec2(-50.0, -30.0), BOUNDS);
        assert_eq!((grown.min, grown.max), (pos2(350.0, 370.0), start.max));
        // Can't shrink past the minimum: the far edges stay where they were.
        let shrunk = tl.resize(start, vec2(500.0, 500.0), BOUNDS);
        assert_eq!(shrunk.size(), MIN_SIZE);
        assert_eq!(shrunk.max, start.max);

        // Left edge 6 pt from a neighbour's right edge sticks to it.
        let left = Sides {
            left: true,
            ..Default::default()
        };
        assert_eq!(left.resize_direction(), egui::ResizeDirection::West);
        let s = snap_resize(
            r(286.0, 400.0, 300.0, 150.0),
            &[r(0.0, 380.0, 280.0, 150.0)],
            BOUNDS,
            left,
        );
        assert_eq!(s.rect.left(), 280.0);
        assert_eq!(s.rect.right(), 586.0);
    }

    #[test]
    fn resize_does_not_pass_the_layer_edge() {
        let bottom_right = Sides {
            right: true,
            bottom: true,
            ..Default::default()
        };
        assert_eq!(
            bottom_right.resize(r(1700.0, 1000.0, 200.0, 100.0), vec2(500.0, 500.0), BOUNDS),
            r(1700.0, 1000.0, 300.0, 200.0)
        );
    }

    #[test]
    fn detects_kinds() {
        assert_eq!(
            parse_capture("идея: добавить weekly recap").kind,
            Kind::Idea
        );
        assert_eq!(
            parse_capture("идея: добавить weekly recap").body,
            "добавить weekly recap"
        );
        assert_eq!(
            parse_capture("vpn staging vpn.staging.internal").kind,
            Kind::Reference
        );
        assert_eq!(
            parse_capture("через две недели проверить новую pricing модель").kind,
            Kind::Reminder
        );
        assert_eq!(parse_capture("https://egui.rs demo").kind, Kind::Link);
        assert_eq!(parse_capture("просто мысль").kind, Kind::Note);
        // A credential without a prefix is Private unless the user took the chip off.
        assert_eq!(parse_capture("wifi пароль qwerty123").kind, Kind::Private);
        assert_eq!(
            parse_capture_with("wifi пароль qwerty123", false).kind,
            Kind::Note
        );
        assert_eq!(
            parse_capture_with("секрет: qwerty123", false).kind,
            Kind::Private
        );
        assert_eq!(
            parse_capture("идея: сменить пароль на роутере").kind,
            Kind::Idea
        );
        assert!(!looks_secret("идея: сменить пароль на роутере"));
        assert!(looks_secret("password: hunter2"));
    }

    #[test]
    fn private_label_never_shows_a_lone_line() {
        assert_eq!(
            private_label("", "Wi-Fi офис\nguest / pass").as_deref(),
            Some("Wi-Fi офис")
        );
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
    fn free_slot_holds_a_card_larger_than_default() {
        let area = vec2(1200.0, 800.0);
        // The first grid slot is free for a default card, but a card twice as
        // wide placed there would run into the card in the second column.
        let cards = vec![card_at(1, 32.0 + DEFAULT_SIZE.x + 16.0, 72.0)];
        let big = vec2(DEFAULT_SIZE.x * 2.0, DEFAULT_SIZE.y);
        let p = free_slot_for(&cards, big, area);
        let slot = egui::Rect::from_min_size(p, big);
        assert!(!egui::Rect::from_min_size(cards[0].pos, cards[0].size).intersects(slot));
        assert!(slot.max.x <= area.x && slot.max.y <= area.y);
    }

    #[test]
    fn labels_that_look_like_credentials_stay_hidden() {
        for ok in [
            "Wi-Fi офис",
            "ALL MY SSH",
            "host: nl-home",
            "Домашнее задание на 3 декабря:",
        ] {
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
        assert_eq!(
            private_parts("", "ssh root@10.0.0.1\npass").0,
            PRIVATE_PLACEHOLDER
        );
        assert_eq!(private_label("", "token=abc\nmore"), None);
    }

    #[test]
    fn private_parts_round_trip_through_the_editor() {
        for text in [
            "Wi-Fi офис\nguest / pass",
            "hunter2",
            "Private\nline one\nline two",
        ] {
            let (label, secret) = private_parts("", text);
            assert_eq!(
                private_parts("", &private_text(&label, &secret)),
                (label, secret),
                "{text}"
            );
        }
        assert_eq!(
            private_parts("", "hunter2"),
            (PRIVATE_PLACEHOLDER.to_owned(), "hunter2".to_owned())
        );
        assert_eq!(private_text("Wi-Fi", "pass"), "Wi-Fi\npass");
    }

    #[test]
    fn extracts_tags_and_keeps_text_as_written() {
        let p = parse_capture("Pricing\nпопробовать annual plan #product #pricing.");
        assert_eq!(p.title, "");
        assert_eq!(
            p.body,
            "Pricing\nпопробовать annual plan #product #pricing."
        );
        assert_eq!(p.tags, vec!["product", "pricing"]);
    }
}
