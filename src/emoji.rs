//! Color emoji. epaint rasterizes glyphs into a coverage-only atlas, so emoji come
//! out as flat silhouettes in the text color. Text is still laid out with Segoe UI
//! Emoji (true advances, ZWJ ligatures, cursor positions); after each frame this
//! plugin collapses those glyphs and draws Direct2D's color rendering in their place.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use egui::epaint::{ClippedShape, TextShape};
use egui::{Color32, ColorImage, FullOutput, Mesh, Rect, Shape, TextureHandle, TextureOptions, pos2, vec2};

pub const FONT_PATH: &str = r"C:\Windows\Fonts\seguiemj.ttf";

struct Faces {
    emoji: Option<&'static [u8]>,
    /// Faces that come before the emoji font in some family; a char they cover is
    /// drawn by them, not by the emoji font.
    text: Vec<&'static [u8]>,
    cache: HashMap<char, bool>,
}

static FACES: LazyLock<RwLock<Faces>> =
    LazyLock::new(|| RwLock::new(Faces { emoji: None, text: Vec::new(), cache: HashMap::new() }));

/// Called whenever the font set is rebuilt.
pub fn set_faces(emoji: Option<&'static [u8]>, text: Vec<&'static [u8]>) {
    *FACES.write().unwrap() = Faces { emoji, text, cache: HashMap::new() };
}

fn has_char(data: &[u8], c: char) -> bool {
    use skrifa::MetadataProvider as _;
    skrifa::FontRef::new(data).is_ok_and(|font| font.charmap().map(c).is_some())
}

/// Whether `c` starts a glyph that the emoji font draws.
fn is_emoji(c: char) -> bool {
    if (c as u32) < 0xA9 {
        return false;
    }
    if let Some(&hit) = FACES.read().unwrap().cache.get(&c) {
        return hit;
    }
    let mut faces = FACES.write().unwrap();
    let hit = faces.emoji.is_some_and(|e| has_char(e, c)) && !faces.text.iter().any(|t| has_char(t, c));
    faces.cache.insert(c, hit);
    hit
}

/// Chars that attach to the preceding emoji: variation selectors, keycap, skin
/// tones, tag sequences, and ZWJ (which also pulls in the next char).
fn is_extender(c: char) -> bool {
    matches!(c, '\u{FE0E}' | '\u{FE0F}' | '\u{20E3}' | '\u{200D}' | '\u{1F3FB}'..='\u{1F3FF}' | '\u{E0020}'..='\u{E007F}')
}

fn is_regional(c: char) -> bool {
    matches!(c, '\u{1F1E6}'..='\u{1F1FF}')
}

/// Length in chars of the emoji sequence starting at `chars[0]`.
fn cluster_len(chars: &[char]) -> usize {
    if is_regional(chars[0]) {
        return if chars.get(1).copied().is_some_and(is_regional) { 2 } else { 1 };
    }
    let mut n = 1;
    while let Some(&c) = chars.get(n).filter(|&&c| is_extender(c)) {
        n += 1;
        if c == '\u{200D}' && n < chars.len() {
            n += 1;
        }
    }
    n
}

struct Sprite {
    texture: TextureHandle,
    /// Physical pixels from the texture's left edge to the pen position.
    left: f32,
    /// Physical pixels from the texture's top edge to the baseline.
    baseline: f32,
}

struct Entry {
    sprite: Option<Sprite>,
    last_used: u64,
}

#[derive(Default)]
pub struct ColorEmoji {
    sprites: HashMap<(String, u32), Entry>,
    frame: u64,
}

impl egui::Plugin for ColorEmoji {
    fn debug_name(&self) -> &'static str {
        "color_emoji"
    }

    fn output_hook(&mut self, ctx: &egui::Context, output: &mut FullOutput) {
        self.frame += 1;
        let ppp = output.pixels_per_point;
        let count = self.sprites.len();
        let mut changed = false;

        let shapes = std::mem::take(&mut output.shapes);
        let mut out = Vec::with_capacity(shapes.len());
        for clipped in shapes {
            self.expand(ctx, clipped.clip_rect, clipped.shape, ppp, &mut out);
        }
        output.shapes = out;
        changed |= self.sprites.len() != count;

        if self.sprites.len() > 256 {
            let frame = self.frame;
            self.sprites.retain(|_, e| e.last_used + 600 > frame);
            changed = true;
        }
        if changed {
            // Deltas from `load_texture` land after end_pass collected this frame's.
            let delta = ctx.tex_manager().write().take_delta();
            output.textures_delta.append(delta);
        }
    }
}

impl ColorEmoji {
    fn expand(&mut self, ctx: &egui::Context, clip_rect: Rect, shape: Shape, ppp: f32, out: &mut Vec<ClippedShape>) {
        match shape {
            Shape::Vec(shapes) => {
                for shape in shapes {
                    self.expand(ctx, clip_rect, shape, ppp, out);
                }
            }
            Shape::Text(text) => self.text(ctx, clip_rect, text, ppp, out),
            shape => out.push(ClippedShape { clip_rect, shape }),
        }
    }

    fn text(&mut self, ctx: &egui::Context, clip_rect: Rect, mut text: TextShape, ppp: f32, out: &mut Vec<ClippedShape>) {
        let candidate = text.angle == 0.0
            && text.opacity_factor > 0.0
            && text.galley.rows.iter().any(|row| row.glyphs.iter().any(|g| is_emoji(g.chr)));
        if !candidate {
            out.push(ClippedShape { clip_rect, shape: Shape::Text(text) });
            return;
        }

        let galley = text.galley.clone();
        let job = &galley.job;
        let chars: Vec<(usize, char)> = job.text.char_indices().collect();
        let plain: Vec<char> = chars.iter().map(|&(_, c)| c).collect();
        // The tessellator snaps the galley origin the same way.
        let origin = (text.pos.to_vec2() * ppp).round() / ppp;

        let mut hidden = Vec::new();
        let mut meshes = Vec::new();
        let mut row_start = 0;
        for (row_index, row) in galley.rows.iter().enumerate() {
            let mut i = 0;
            while i < row.glyphs.len() {
                let glyph = &row.glyphs[i];
                let start = row_start + i;
                if !is_emoji(glyph.chr) || glyph.uv_rect.is_nothing() || plain.get(start) != Some(&glyph.chr) {
                    i += 1;
                    continue;
                }
                let len = cluster_len(&plain[start..]).min(row.glyphs.len() - i);
                let bytes = chars[start].0..chars.get(start + len).map_or(job.text.len(), |&(b, _)| b);
                let size = job
                    .sections
                    .iter()
                    .find(|s| s.byte_range.start.0 <= bytes.start && bytes.start < s.byte_range.end.0)
                    .map_or(14.0, |s| s.format.font_id.size);

                let mesh = &row.visuals.mesh;
                let mut color = mesh.vertices[glyph.first_vertex as usize].color;
                if color == Color32::PLACEHOLDER {
                    color = text.fallback_color;
                }
                if let Some(o) = text.override_text_color {
                    color = o;
                }
                let tint = Color32::from_white_alpha(color.a()).gamma_multiply(text.opacity_factor);

                if let Some(sprite) = self.sprite(ctx, &job.text[bytes], size * ppp) {
                    // egui centers a fallback face's line box on the primary font's;
                    // undo that so the emoji sits on the text baseline instead.
                    let centering = 0.5 * (glyph.font_height - glyph.font_face_height);
                    let baseline = glyph.pos.y - glyph.font_face_ascent - centering + glyph.font_ascent;
                    let pen = origin + row.pos.to_vec2() + vec2(glyph.pos.x, baseline);
                    let min = pos2((pen.x * ppp).round() - sprite.left, (pen.y * ppp).round() - sprite.baseline) / ppp;
                    let [w, h] = sprite.texture.size();
                    let rect = Rect::from_min_size(min, vec2(w as f32, h as f32) / ppp);
                    let mut quad = Mesh::with_texture(sprite.texture.id());
                    quad.add_rect_with_uv(rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
                    meshes.push(quad);
                    for g in &row.glyphs[i..i + len] {
                        if !g.uv_rect.is_nothing() {
                            hidden.push((row_index, g.first_vertex as usize));
                        }
                    }
                }
                i += len;
            }
            row_start += row.char_count_including_newline().0;
        }

        if !hidden.is_empty() {
            let galley = Arc::make_mut(&mut text.galley);
            for (row_index, first) in hidden {
                let vertices = &mut Arc::make_mut(&mut galley.rows[row_index].row).visuals.mesh.vertices;
                if let Some(quad) = vertices.get_mut(first..first + 4) {
                    let pos = quad[0].pos;
                    quad.iter_mut().for_each(|v| v.pos = pos);
                }
            }
        }
        out.push(ClippedShape { clip_rect, shape: Shape::Text(text) });
        out.extend(meshes.into_iter().map(|m| ClippedShape { clip_rect, shape: Shape::mesh(m) }));
    }

    fn sprite(&mut self, ctx: &egui::Context, cluster: &str, size_px: f32) -> Option<&Sprite> {
        let key = (cluster.to_owned(), (size_px * 4.0).round() as u32);
        let frame = self.frame;
        let entry = self.sprites.entry(key).or_insert_with(|| Entry {
            sprite: render(cluster, size_px).map(|(image, left, baseline)| Sprite {
                texture: ctx.load_texture("emoji", image, TextureOptions::LINEAR),
                left,
                baseline,
            }),
            last_used: frame,
        });
        entry.last_used = frame;
        entry.sprite.as_ref()
    }
}

/// Renders `cluster` in color at `size_px`; returns the image with the pen
/// position and baseline in it, both whole pixels.
fn render(cluster: &str, size_px: f32) -> Option<(ColorImage, f32, f32)> {
    use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT};
    use windows::Win32::Graphics::Direct2D::{
        D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
        D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE,
        D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1Factory,
    };
    use windows::Win32::Graphics::DirectWrite::{
        DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL,
        DWRITE_LINE_METRICS, DWRITE_TEXT_METRICS, DWriteCreateFactory, IDWriteFactory,
    };
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
    use windows::Win32::Graphics::Imaging::{
        CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory, WICBitmapCacheOnLoad,
        WICBitmapLockRead, WICRect,
    };
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx};
    use windows::core::w;

    if !(1.0..=512.0).contains(&size_px) {
        return None;
    }
    let wide: Vec<u16> = cluster.encode_utf16().collect();
    unsafe {
        // Already initialized on the UI thread by winit; harmless otherwise.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).ok()?;
        let format = dwrite
            .CreateTextFormat(
                w!("Segoe UI Emoji"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size_px,
                w!(""),
            )
            .ok()?;
        let layout = dwrite.CreateTextLayout(&wide, &format, 16384.0, 16384.0).ok()?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        layout.GetMetrics(&mut metrics).ok()?;
        let mut line = [DWRITE_LINE_METRICS::default()];
        let mut lines = 0;
        // Fails with "insufficient buffer" past one line, which a cluster never has.
        layout.GetLineMetrics(Some(&mut line), &mut lines).ok()?;

        // Room for ink that overhangs the advance box.
        let pad = (size_px * 0.25).ceil();
        let baseline = pad + line[0].baseline.ceil();
        let width = (metrics.widthIncludingTrailingWhitespace + 2.0 * pad).ceil() as u32;
        let height = (line[0].height.ceil() + 2.0 * pad) as u32;

        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let bitmap = wic.CreateBitmap(width, height, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad).ok()?;
        {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).ok()?;
            let properties = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let target = d2d.CreateWicBitmapRenderTarget(&bitmap, &properties).ok()?;
            // Glyphs without color layers fall back to this; the tint only carries alpha.
            let brush = target.CreateSolidColorBrush(&D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }, None).ok()?;
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            target.BeginDraw();
            target.Clear(Some(&D2D1_COLOR_F::default()));
            let origin = windows_numerics::Vector2 { X: pad, Y: baseline - line[0].baseline };
            target.DrawTextLayout(origin, &layout, &brush, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
            target.EndDraw(None, None).ok()?;
        }

        let rect = WICRect { X: 0, Y: 0, Width: width as i32, Height: height as i32 };
        let lock = bitmap.Lock(&rect, WICBitmapLockRead.0 as u32).ok()?;
        let stride = lock.GetStride().ok()? as usize;
        let mut len = 0;
        let mut data = std::ptr::null_mut();
        lock.GetDataPointer(&mut len, &mut data).ok()?;
        let src = std::slice::from_raw_parts(data, len as usize);

        let (w, h) = (width as usize, height as usize);
        let mut rgba = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for px in src[y * stride..y * stride + w * 4].chunks_exact(4) {
                rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
            }
        }
        if rgba.chunks_exact(4).all(|px| px[3] == 0) {
            return None;
        }
        Some((ColorImage::from_rgba_premultiplied([w, h], &rgba), pad, baseline))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn len(s: &str) -> usize {
        cluster_len(&s.chars().collect::<Vec<_>>())
    }

    #[test]
    fn clusters() {
        assert_eq!(len("😀a"), 1);
        assert_eq!(len("👍🏽!"), 2);
        assert_eq!(len("❤️x"), 2);
        assert_eq!(len("👨‍👩‍👧 hi"), 5);
        assert_eq!(len("🇷🇺🇺🇸"), 2);
        assert_eq!(len("🏴\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}"), 7);
    }
}
