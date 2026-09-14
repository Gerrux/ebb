//! Fonts and colors. Uses the system's Segoe UI Variable and Segoe Fluent Icons
//! so nothing is bundled and text matches the rest of Windows 11.

use std::sync::{Arc, RwLock};

use egui::epaint::text::{FontTweak, VariationCoords};
use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId};

pub const SUCCESS: Color32 = Color32::from_rgb(52, 211, 153);

/// Light or dark: follow Windows, or pick one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeMode {
    System,
    Dark,
    Light,
}

impl ThemeMode {
    pub const ALL: [ThemeMode; 3] = [ThemeMode::System, ThemeMode::Dark, ThemeMode::Light];

    pub fn key(self) -> &'static str {
        match self {
            ThemeMode::System => "system",
            ThemeMode::Dark => "dark",
            ThemeMode::Light => "light",
        }
    }

    pub fn from_key(key: &str) -> ThemeMode {
        ThemeMode::ALL.into_iter().find(|m| m.key() == key).unwrap_or(ThemeMode::System)
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::System => "Как в Windows",
            ThemeMode::Dark => "Тёмная",
            ThemeMode::Light => "Светлая",
        }
    }
}

/// What a card's background is painted with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardBackground {
    /// The theme's glass.
    Theme,
    /// The color of Start and the taskbar.
    Taskbar,
    Custom([u8; 3]),
}

impl CardBackground {
    pub fn key(self) -> String {
        match self {
            CardBackground::Theme => "theme".into(),
            CardBackground::Taskbar => "taskbar".into(),
            CardBackground::Custom([r, g, b]) => format!("{r:02x}{g:02x}{b:02x}"),
        }
    }

    pub fn from_key(key: &str) -> Option<CardBackground> {
        match key {
            "theme" => Some(CardBackground::Theme),
            "taskbar" => Some(CardBackground::Taskbar),
            hex if hex.len() == 6 => {
                let n = u32::from_str_radix(hex, 16).ok()?;
                Some(CardBackground::Custom([(n >> 16) as u8, (n >> 8) as u8, n as u8]))
            }
            _ => None,
        }
    }
}

/// Colors for picking a card background by hand: Sticky Notes' and Keep's
/// paper colors, then dark ones for the dark theme.
pub const PAPER: [[u8; 3]; 12] = [
    [255, 242, 171],
    [203, 241, 196],
    [255, 204, 229],
    [231, 207, 255],
    [205, 233, 255],
    [255, 255, 255],
    [92, 43, 41],
    [97, 74, 25],
    [52, 89, 32],
    [22, 80, 75],
    [45, 85, 94],
    [66, 39, 94],
];

/// Colors in use; swapped by `apply` when the theme or the Windows accent changes.
#[derive(Clone, Copy)]
struct Palette {
    light: bool,
    text: Color32,
    dim: Color32,
    muted: Color32,
    glass: Color32,
    glass_hover: Color32,
    stroke: Color32,
    chip: Color32,
    card: Color32,
    card_hover: Color32,
    /// Text on cards: the theme's, or picked for contrast with a card color.
    card_text: Color32,
    card_dim: Color32,
    card_muted: Color32,
    card_light: bool,
}

const DARK: Palette = Palette {
    light: false,
    text: Color32::from_rgb(236, 239, 244),
    dim: Color32::from_rgb(178, 186, 198),
    muted: Color32::from_rgb(128, 136, 150),
    glass: Color32::from_rgba_unmultiplied_const(30, 33, 40, 215),
    glass_hover: Color32::from_rgba_unmultiplied_const(40, 44, 53, 228),
    stroke: Color32::from_rgba_unmultiplied_const(255, 255, 255, 26),
    chip: Color32::from_rgba_unmultiplied_const(255, 255, 255, 16),
    card: Color32::from_rgba_unmultiplied_const(30, 33, 40, 215),
    card_hover: Color32::from_rgba_unmultiplied_const(40, 44, 53, 228),
    card_text: Color32::from_rgb(236, 239, 244),
    card_dim: Color32::from_rgb(178, 186, 198),
    card_muted: Color32::from_rgb(128, 136, 150),
    card_light: false,
};

const LIGHT: Palette = Palette {
    light: true,
    text: Color32::from_rgb(26, 28, 34),
    dim: Color32::from_rgb(72, 78, 90),
    muted: Color32::from_rgb(112, 118, 130),
    glass: Color32::from_rgba_unmultiplied_const(249, 250, 252, 225),
    glass_hover: Color32::from_rgba_unmultiplied_const(255, 255, 255, 240),
    stroke: Color32::from_rgba_unmultiplied_const(0, 0, 0, 22),
    chip: Color32::from_rgba_unmultiplied_const(0, 0, 0, 12),
    card: Color32::from_rgba_unmultiplied_const(249, 250, 252, 225),
    card_hover: Color32::from_rgba_unmultiplied_const(255, 255, 255, 240),
    card_text: Color32::from_rgb(26, 28, 34),
    card_dim: Color32::from_rgb(72, 78, 90),
    card_muted: Color32::from_rgb(112, 118, 130),
    card_light: true,
};

static PALETTE: RwLock<Palette> = RwLock::new(DARK);

fn palette() -> Palette {
    *PALETTE.read().unwrap()
}

pub fn is_light() -> bool {
    palette().light
}

pub fn text() -> Color32 {
    palette().text
}

pub fn dim() -> Color32 {
    palette().dim
}

pub fn muted() -> Color32 {
    palette().muted
}

pub fn glass_fill() -> Color32 {
    palette().glass
}

pub fn glass_fill_hover() -> Color32 {
    palette().glass_hover
}

pub fn glass_stroke() -> Color32 {
    palette().stroke
}

pub fn chip_fill() -> Color32 {
    palette().chip
}

/// A card's background color (its alpha comes from the card style).
pub fn card_fill(hovered: bool) -> Color32 {
    let p = palette();
    if hovered { p.card_hover } else { p.card }
}

pub fn card_text() -> Color32 {
    palette().card_text
}

pub fn card_dim() -> Color32 {
    palette().card_dim
}

pub fn card_muted() -> Color32 {
    palette().card_muted
}

/// A kind color made readable on the card background, which can be light in the
/// dark theme (or dark in the light one) when the user picked its color.
pub fn on_card(accent: Color32) -> Color32 {
    let p = palette();
    match (p.light, p.card_light) {
        (false, true) => adapt_dark(accent),
        (true, false) => {
            let u = |v: u8| (f32::from(v) / 0.68).min(255.0) as u8;
            Color32::from_rgb(u(accent.r()), u(accent.g()), u(accent.b()))
        }
        _ => accent,
    }
}

fn adapt_dark(c: Color32) -> Color32 {
    let d = |v: u8| (f32::from(v) * 0.68) as u8;
    Color32::from_rgb(d(c.r()), d(c.g()), d(c.b()))
}

/// A hover/selection wash over the current surface: white on dark, black on light.
pub fn wash(alpha: u8) -> Color32 {
    if is_light() { Color32::from_black_alpha((f32::from(alpha) * 0.7) as u8) } else { Color32::from_white_alpha(alpha) }
}

/// Background of the floating windows (bar, library) over their acrylic.
pub fn window_fill(alpha: u8) -> Color32 {
    if is_light() {
        Color32::from_rgba_unmultiplied(244, 245, 248, alpha.saturating_add(50))
    } else {
        Color32::from_rgba_unmultiplied(22, 24, 30, alpha)
    }
}

/// The layer's tint over the desktop.
pub fn scrim(alpha: u8) -> Color32 {
    // White over a dark desktop reads as grey; it takes about twice the alpha to look light.
    if is_light() { Color32::from_white_alpha(alpha.saturating_mul(2).max(150)) } else { Color32::from_black_alpha(alpha) }
}

/// Addresses in Link cards, readable on the card's background.
pub fn link() -> Color32 {
    if palette().card_light { Color32::from_rgb(0, 95, 184) } else { Color32::from_rgb(156, 195, 245) }
}

/// Selection and "on" highlights.
pub fn highlight(alpha: u8) -> Color32 {
    if is_light() { Color32::from_rgba_unmultiplied(0, 120, 212, alpha) } else { Color32::from_rgba_unmultiplied(96, 165, 250, alpha) }
}

/// A kind or tint color made readable on the current surface: the bright
/// accents disappear on white, so they're darkened in the light theme.
pub fn adapt(c: Color32) -> Color32 {
    if is_light() { adapt_dark(c) } else { c }
}

fn mix(a: Color32, rgb: [u8; 3], t: f32) -> Color32 {
    let l = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    let [r, g, b, alpha] = a.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(l(r, rgb[0]), l(g, rgb[1]), l(b, rgb[2]), alpha)
}

/// Picks the palette for `mode` (Windows' own light/dark for System) and the
/// card background.
pub fn apply(ctx: &egui::Context, mode: ThemeMode, background: CardBackground, system: &crate::win::SystemColors) {
    let light = match mode {
        ThemeMode::System => system.light,
        ThemeMode::Dark => false,
        ThemeMode::Light => true,
    };
    let mut p = if light { LIGHT } else { DARK };
    let solid = match background {
        CardBackground::Theme => None,
        // The color Windows gives Start and the taskbar: the accent's when that's
        // switched on (and the dark theme, which it's made for), else its neutral grey.
        CardBackground::Taskbar => Some(match system.start {
            Some(rgb) if !light => rgb,
            Some(_) => system.palette[0],
            None if light => [238, 238, 238],
            None => [32, 32, 32],
        }),
        CardBackground::Custom(rgb) => Some(rgb),
    };
    if let Some(rgb) = solid {
        // Opacity comes from the card style; here only the color.
        let card = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
        let luma = 0.2126 * f32::from(rgb[0]) + 0.7152 * f32::from(rgb[1]) + 0.0722 * f32::from(rgb[2]);
        let card_light = luma > 140.0;
        p.card = card;
        p.card_hover = mix(card, if card_light { [0, 0, 0] } else { [255, 255, 255] }, 0.05);
        let text = if card_light { LIGHT } else { DARK };
        (p.card_text, p.card_dim, p.card_muted, p.card_light) = (text.text, text.dim, text.muted, card_light);
    }
    *PALETTE.write().unwrap() = p;
    crate::win::LIGHT_THEME.store(light, std::sync::atomic::Ordering::Relaxed);
    ctx.global_style_mut(|style| {
        style.visuals = if light { egui::Visuals::light() } else { egui::Visuals::dark() };
        style.visuals.selection.bg_fill = highlight(90);
        style.visuals.extreme_bg_color = Color32::TRANSPARENT;
    });
    ctx.request_repaint();
}

/// Typeface for card text. All ship with Windows; only the chosen one is loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardFont {
    System,
    Calibri,
    Georgia,
    SegoePrint,
    Consolas,
    Bahnschrift,
}

impl CardFont {
    pub const ALL: [CardFont; 6] =
        [CardFont::System, CardFont::Calibri, CardFont::Georgia, CardFont::SegoePrint, CardFont::Consolas, CardFont::Bahnschrift];

    pub fn key(self) -> &'static str {
        match self {
            CardFont::System => "system",
            CardFont::Calibri => "calibri",
            CardFont::Georgia => "georgia",
            CardFont::SegoePrint => "segoe-print",
            CardFont::Consolas => "consolas",
            CardFont::Bahnschrift => "bahnschrift",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CardFont::System => "Segoe UI",
            CardFont::Calibri => "Calibri",
            CardFont::Georgia => "Georgia",
            CardFont::SegoePrint => "Segoe Print (рукописный)",
            CardFont::Consolas => "Consolas",
            CardFont::Bahnschrift => "Bahnschrift",
        }
    }

    /// Regular and bold files in C:\Windows\Fonts; `None` for the UI font.
    fn files(self) -> Option<(&'static str, &'static str)> {
        match self {
            CardFont::System => None,
            CardFont::Calibri => Some(("calibri.ttf", "calibrib.ttf")),
            CardFont::Georgia => Some(("georgia.ttf", "georgiab.ttf")),
            CardFont::SegoePrint => Some(("segoepr.ttf", "segoeprb.ttf")),
            CardFont::Consolas => Some(("consola.ttf", "consolab.ttf")),
            CardFont::Bahnschrift => Some(("bahnschrift.ttf", "bahnschrift.ttf")),
        }
    }
}

static CARD_FONT: RwLock<CardFont> = RwLock::new(CardFont::System);

/// Card text in the chosen typeface.
pub fn card_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("card".into()))
}

pub fn card_bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("card-bold".into()))
}

/// Switches the card typeface; rebuilds the font set only when it changed.
pub fn set_card_font(ctx: &egui::Context, font: CardFont) {
    if std::mem::replace(&mut *CARD_FONT.write().unwrap(), font) != font {
        ctx.set_fonts(definitions(font));
    }
}

pub fn icons(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("icons".into()))
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

/// Memory-mapped rather than read: saves ~3 MiB of private bytes versus a heap copy.
fn read_static(path: &str) -> Option<&'static [u8]> {
    crate::win::map_file_static(path)
}

pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(definitions(*CARD_FONT.read().unwrap()));
    ctx.add_plugin(crate::emoji::ColorEmoji::default());

    ctx.global_style_mut(|style| {
        style.visuals = egui::Visuals::dark();
        style.visuals.selection.bg_fill = highlight(90);
        style.visuals.extreme_bg_color = Color32::TRANSPARENT;
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    });
}

fn definitions(card: CardFont) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();

    let segoe = read_static(r"C:\Windows\Fonts\SegUIVar.ttf");
    if let Some(segoe) = segoe {
        fonts
            .font_data
            .insert("segoe".into(), Arc::new(FontData::from_static(segoe)));
        fonts.font_data.insert(
            "segoe-semibold".into(),
            Arc::new(FontData::from_static(segoe).tweak(FontTweak {
                coords: VariationCoords::new([("wght", 600.0)]),
                ..Default::default()
            })),
        );
        let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
        proportional.insert(0, "segoe".into());
    }

    // Emoji right after the UI font, ahead of egui's older Noto Emoji: its advances
    // and ZWJ ligatures are what `emoji` paints the color rendering over.
    let emoji = read_static(crate::emoji::FONT_PATH);
    if let Some(data) = emoji {
        fonts.font_data.insert("emoji".into(), Arc::new(FontData::from_static(data)));
        let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
        proportional.insert(segoe.is_some() as usize, "emoji".into());
    }
    let mut text_faces: Vec<&'static [u8]> = segoe.into_iter().collect();

    let mut semibold = vec!["segoe-semibold".to_owned()];
    semibold.extend(fonts.families[&FontFamily::Proportional].iter().cloned());
    fonts
        .families
        .insert(FontFamily::Name("semibold".into()), semibold);

    // Reference lines (commands, paths, hosts). Consolas ships with every Windows.
    let mut mono = Vec::new();
    if let Some(consolas) = read_static(r"C:\Windows\Fonts\consola.ttf") {
        fonts.font_data.insert("consolas".into(), Arc::new(FontData::from_static(consolas)));
        mono.push("consolas".to_owned());
        text_faces.push(consolas);
    }
    mono.extend(fonts.families[&FontFamily::Proportional].iter().cloned());
    fonts.families.insert(FontFamily::Monospace, mono);

    let mut icon_family = Vec::new();
    if let Some(icons) = read_static(r"C:\Windows\Fonts\SegoeIcons.ttf") {
        fonts
            .font_data
            .insert("fluent-icons".into(), Arc::new(FontData::from_static(icons)));
        icon_family.push("fluent-icons".to_owned());
    }
    icon_family.extend(fonts.families[&FontFamily::Proportional].iter().cloned());
    fonts
        .families
        .insert(FontFamily::Name("icons".into()), icon_family);

    // Card text: the chosen typeface over the UI font, which covers what it lacks.
    let proportional = fonts.families[&FontFamily::Proportional].clone();
    let semibold_list = fonts.families[&FontFamily::Name("semibold".into())].clone();
    let (mut regular, mut bold) = (Vec::new(), Vec::new());
    if let Some((r, b)) = card.files() {
        if let Some(data) = read_static(&format!(r"C:\Windows\Fonts\{r}")) {
            fonts.font_data.insert("card-regular".into(), Arc::new(FontData::from_static(data)));
            regular.push("card-regular".to_owned());
            text_faces.push(data);
        }
        if let Some(data) = read_static(&format!(r"C:\Windows\Fonts\{b}")) {
            fonts.font_data.insert("card-bold".into(), Arc::new(FontData::from_static(data)));
            bold.push("card-bold".to_owned());
            text_faces.push(data);
        }
    }
    crate::emoji::set_faces(emoji, text_faces);
    regular.extend(proportional);
    bold.extend(semibold_list);
    fonts.families.insert(FontFamily::Name("card".into()), regular);
    fonts.families.insert(FontFamily::Name("card-bold".into()), bold);
    fonts
}
