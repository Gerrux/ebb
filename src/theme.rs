//! Fonts and colors. Uses the system's Segoe UI Variable and Segoe Fluent Icons
//! so nothing is bundled and text matches the rest of Windows 11.

use std::sync::Arc;

use egui::epaint::text::{FontTweak, VariationCoords};
use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId};

pub const TEXT: Color32 = Color32::from_rgb(236, 239, 244);
pub const TEXT_DIM: Color32 = Color32::from_rgb(178, 186, 198);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(128, 136, 150);

pub fn glass_fill() -> Color32 {
    Color32::from_rgba_unmultiplied(30, 33, 40, 215)
}

pub fn glass_fill_hover() -> Color32 {
    Color32::from_rgba_unmultiplied(40, 44, 53, 228)
}

pub fn glass_stroke() -> Color32 {
    Color32::from_rgba_unmultiplied(255, 255, 255, 26)
}

pub fn chip_fill() -> Color32 {
    Color32::from_rgba_unmultiplied(255, 255, 255, 16)
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
    let mut fonts = FontDefinitions::default();

    if let Some(segoe) = read_static(r"C:\Windows\Fonts\SegUIVar.ttf") {
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

    let mut semibold = vec!["segoe-semibold".to_owned()];
    semibold.extend(fonts.families[&FontFamily::Proportional].iter().cloned());
    fonts
        .families
        .insert(FontFamily::Name("semibold".into()), semibold);

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

    ctx.set_fonts(fonts);

    ctx.global_style_mut(|style| {
        style.visuals = egui::Visuals::dark();
        style.visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(96, 165, 250, 90);
        style.visuals.extreme_bg_color = Color32::TRANSPARENT;
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    });
}
