//! Font faces drawn into a [`Canvas`]: character-cell sizing like the retail `CWin32Font`,
//! a glyph cache, optional blur and scanlines, and letter-edge outlines.
use crate::canvas::{
    draw_texture, draw_texture_ex, Canvas, Color, DrawTextureParams, FilterMode, MutexCell, Rect,
    Texture2D,
};
use anyhow::{Context, Result};
use glam::{vec2, Vec2};
use std::{collections::HashMap, sync::Arc};

/// Outline style for rendering borders along font glyph contours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextOutline {
    pub color: Color,
    pub thickness: f32,
}

impl Default for TextOutline {
    fn default() -> Self {
        Self::NONE
    }
}

impl TextOutline {
    pub const NONE: Self = Self {
        color: Color::new(0., 0., 0., 0.),
        thickness: 0.,
    };

    pub const DEFAULT: Self = Self {
        color: Color::new(0.0, 0.0, 0.0, 0.45),
        thickness: 1.0,
    };

    pub const fn new(color: Color, thickness: f32) -> Self {
        Self { color, thickness }
    }

    pub fn is_visible(&self) -> bool {
        self.color.a > 0.0 && self.thickness > 0.0
    }
}

#[derive(Clone)]
pub struct FontFace {
    pub canvas: Canvas,
    font: Arc<fontdue::Font>,
    glyphs: GlyphCache,
    cell_height: f32,
    ascent: f32,
}
type GlyphCache = Arc<MutexCell<HashMap<(char, u32, u16, u16), Glyph>>>;
#[derive(Clone)]
struct Glyph {
    texture: Option<Texture2D>,
    x: f32,
    y: f32,
    advance: f32,
}
impl FontFace {
    pub fn load(bytes: &[u8], canvas: Canvas) -> Result<Self> {
        let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
            .map_err(anyhow::Error::msg)?;
        let face = ttf_parser::Face::parse(bytes, 0).context("read HUD font metrics")?;
        let units = f32::from(face.units_per_em()).max(1.);
        let (ascent, cell_height) = face.tables().os2.map_or_else(
            || {
                let metrics = font.horizontal_line_metrics(1.).unwrap();
                (metrics.ascent, metrics.ascent - metrics.descent)
            },
            |table| {
                let win_ascent = f32::from(table.windows_ascender()) / units;
                let descent = f32::from(table.windows_descender()).abs() / units;
                let cell_height = win_ascent + descent;
                let ascent = face
                    .capital_height()
                    .filter(|&cap| cap > 0)
                    .map_or(win_ascent, |cap| {
                        (cell_height + f32::from(cap) / units) * 0.5
                    });
                (ascent, cell_height)
            },
        );
        Ok(Self {
            canvas,
            font: Arc::new(font),
            glyphs: Arc::new(MutexCell::new(HashMap::new())),
            cell_height: cell_height.max(0.01),
            ascent,
        })
    }
    pub fn width(&self, value: &str, size: f32) -> f32 {
        // Retail CWin32Font uses a positive CreateFontA height: a character cell,
        // whereas fontdue's size is pixels per EM. Windows OS/2 metrics supply the
        // conversion; hhea descent differs significantly in the owned HalfLife2 font.
        let size = size.round().max(1.) / self.cell_height;
        value
            .chars()
            .map(|c| self.font.metrics(c, size).advance_width)
            .sum()
    }
    fn glyph(&self, c: char, size: u16, blur: u16, scanlines: u16) -> Glyph {
        self.raster(c, f32::from(size) / self.cell_height, blur, scanlines)
    }
    fn raster(&self, c: char, em_size: f32, blur: u16, scanlines: u16) -> Glyph {
        let key = (c, em_size.to_bits(), blur, scanlines);
        if let Some(glyph) = self.glyphs.borrow().get(&key) {
            return glyph.clone();
        }
        let (metrics, coverage) = self.font.rasterize(c, em_size);
        let margin = usize::from(blur);
        let width = metrics.width + margin * 2;
        let height = metrics.height + margin * 2;
        let ascent = self.ascent * em_size;
        let texture = if metrics.width == 0 || metrics.height == 0 {
            None
        } else {
            let mut bitmap = vec![0.; width * height];
            for y in 0..metrics.height {
                for x in 0..metrics.width {
                    bitmap[(y + margin) * width + x + margin] =
                        f32::from(coverage[y * metrics.width + x]) / 255.;
                }
            }
            if blur > 0 {
                bitmap = gaussian_blur(&bitmap, width, height, margin);
            }
            let rgba = bitmap
                .iter()
                .enumerate()
                .flat_map(|(i, coverage)| {
                    // The configured scanline pitch is retained. Exact GDI scanline and
                    // blur rasterization are not claimed equivalent to VGUI's native font engine.
                    let brightness = if scanlines > 1 && (i / width) % usize::from(scanlines) != 0 {
                        180
                    } else {
                        255
                    };
                    [
                        brightness,
                        brightness,
                        brightness,
                        (coverage.clamp(0., 1.) * 255.).round() as u8,
                    ]
                })
                .collect::<Vec<_>>();
            let texture = Texture2D::from_rgba8(width as u16, height as u16, &rgba);
            texture.set_filter(FilterMode::Linear);
            Some(texture)
        };
        let glyph = Glyph {
            texture,
            x: metrics.xmin as f32 - f32::from(blur),
            y: ascent - metrics.ymin as f32 - metrics.height as f32 - f32::from(blur),
            advance: metrics.advance_width,
        };
        self.glyphs.borrow_mut().insert(key, glyph.clone());
        glyph
    }
    /// Cached bitmap bounds relative to the character-cell origin, including font effects.
    /// Symbol HUDs can center their visible glyph rather than assuming text ascent symmetry.
    pub fn glyph_bounds(&self, character: char, size: f32, effects: (u16, u16)) -> Rect {
        let glyph = self.glyph(
            character,
            size.round().clamp(1., 1024.) as u16,
            effects.0.min(64),
            effects.1,
        );
        let dimensions = glyph.texture.as_ref().map_or(Vec2::ZERO, Texture2D::size);
        Rect::new(glyph.x, glyph.y, dimensions.x, dimensions.y)
    }
    pub fn draw(
        &self,
        value: &str,
        mut x: f32,
        y: f32,
        size: f32,
        effects: (u16, u16),
        color: Color,
    ) {
        let (blur, scanlines) = effects;
        let size = size.round().clamp(1., 1024.) as u16;
        for c in value.chars() {
            let glyph = self.glyph(c, size, blur.min(64), scanlines);
            if let Some(texture) = &glyph.texture {
                draw_texture(&self.canvas, texture, x + glyph.x, y + glyph.y, color);
            }
            x += glyph.advance;
        }
    }
    pub fn face(&self) -> Arc<fontdue::Font> {
        self.font.clone()
    }
    pub fn advance_em(&self, c: char, em: f32) -> f32 {
        self.font.metrics(c, em).advance_width
    }
    pub fn width_em(&self, value: &str, em: f32) -> f32 {
        value
            .chars()
            .map(|c| self.font.metrics(c, em).advance_width)
            .sum()
    }
    /// Width of `value` for a font whose character cell is `cell` pixels tall.
    pub fn width_cell(&self, value: &str, cell: f32) -> f32 {
        self.width_em(value, cell / self.cell_height)
    }
    /// Draws `value` with an outline/border along the letter edge path with its character cell's
    /// top-left corner at `top_left`. Glyphs are rasterized at a whole-pixel EM and snapped to
    /// whole pixels so 1:1 glyph bitmaps do not blur across subpixels.
    pub fn draw_cell_outlined(
        &self,
        value: &str,
        top_left: Vec2,
        cell: f32,
        color: Color,
        outline: TextOutline,
    ) {
        let raster = (cell / self.cell_height).round().clamp(1., 1024.);
        self.draw_baseline_outlined(
            value,
            (top_left + Vec2::new(0., self.ascent * raster)).round(),
            raster,
            1.0,
            color,
            outline,
        );
    }
    /// Draws `value` with its character cell's top-left corner at `top_left`. Glyphs are
    /// rasterized at a whole-pixel EM and scaled, so the glyph cache stays bounded.
    pub fn draw_cell(&self, value: &str, top_left: Vec2, cell: f32, color: Color) {
        self.draw_cell_outlined(value, top_left, cell, color, TextOutline::NONE);
    }
    /// Draws `value` with an outline/border along the letter edge path.
    pub fn draw_baseline_outlined(
        &self,
        value: &str,
        origin: Vec2,
        em: f32,
        scale: f32,
        color: Color,
        outline: TextOutline,
    ) {
        let mut pen = origin;
        let mut glyphs = Vec::with_capacity(value.len());
        let unit_scale = (scale - 1.0).abs() < 1e-3;
        for c in value.chars() {
            let glyph = self.raster(c, em, 0, 0);
            if let Some(texture) = &glyph.texture {
                let (pos, size) = if unit_scale {
                    (
                        vec2(
                            (pen.x + glyph.x).round(),
                            (pen.y + glyph.y - self.ascent * em).round(),
                        ),
                        texture.size(),
                    )
                } else {
                    (
                        vec2(
                            pen.x + glyph.x * scale,
                            pen.y + (glyph.y - self.ascent * em) * scale,
                        ),
                        texture.size() * scale,
                    )
                };
                glyphs.push((texture.clone(), pos, size));
            }
            pen.x += glyph.advance * scale;
        }

        if outline.is_visible() {
            const OFFSETS: [(f32, f32); 8] = [
                (-1.0, 0.0),
                (1.0, 0.0),
                (0.0, -1.0),
                (0.0, 1.0),
                (-1.0, -1.0),
                (1.0, -1.0),
                (-1.0, 1.0),
                (1.0, 1.0),
            ];
            for (dx, dy) in OFFSETS {
                let offset = vec2(dx * outline.thickness, dy * outline.thickness);
                for (texture, pos, size) in &glyphs {
                    draw_texture_ex(
                        &self.canvas,
                        texture,
                        pos.x + offset.x,
                        pos.y + offset.y,
                        outline.color,
                        DrawTextureParams {
                            dest_size: Some(*size),
                            ..Default::default()
                        },
                    );
                }
            }
        }

        if color.a > 0.0 {
            for (texture, pos, size) in &glyphs {
                draw_texture_ex(
                    &self.canvas,
                    texture,
                    pos.x,
                    pos.y,
                    color,
                    DrawTextureParams {
                        dest_size: Some(*size),
                        ..Default::default()
                    },
                );
            }
        }
    }
    pub fn draw_baseline(&self, value: &str, origin: Vec2, em: f32, scale: f32, color: Color) {
        self.draw_baseline_outlined(value, origin, em, scale, color, TextOutline::NONE);
    }
    /// Draws `value` vertically cropped between normalized fractions `fraction.x` and `fraction.y`
    /// (both in `[0.0, 1.0]`) of the glyph's bitmap height.
    pub fn draw_cropped(
        &self,
        value: &str,
        origin: Vec2,
        size: f32,
        effects: (u16, u16),
        color: Color,
        fraction: Vec2,
    ) {
        let (blur, scanlines) = effects;
        let size = size.round().clamp(1., 1024.) as u16;
        let mut x = origin.x;
        for c in value.chars() {
            let glyph = self.glyph(c, size, blur.min(64), scanlines);
            if let Some(texture) = &glyph.texture {
                let th = texture.height();
                let first = (fraction.x.clamp(0., 1.) * th).round();
                let last = (fraction.y.clamp(0., 1.) * th).round();
                if last > first {
                    draw_texture_ex(
                        &self.canvas,
                        texture,
                        x + glyph.x,
                        origin.y + glyph.y + first,
                        color,
                        DrawTextureParams {
                            source: Some(Rect::new(0., first, texture.width(), last - first)),
                            ..Default::default()
                        },
                    );
                }
            }
            x += glyph.advance;
        }
    }
}

fn gaussian_blur(bitmap: &[f32], width: usize, height: usize, radius: usize) -> Vec<f32> {
    let sigma = (radius as f32 * 0.5).max(0.5);
    let mut kernel = (-(radius as isize)..=radius as isize)
        .map(|x| (-(x * x) as f32 / (2. * sigma * sigma)).exp())
        .collect::<Vec<_>>();
    let sum: f32 = kernel.iter().sum();
    for weight in &mut kernel {
        *weight /= sum;
    }
    let mut horizontal = vec![0.; bitmap.len()];
    let mut result = vec![0.; bitmap.len()];
    for y in 0..height {
        for x in 0..width {
            for (i, weight) in kernel.iter().enumerate() {
                let sample = x as isize + i as isize - radius as isize;
                if sample >= 0 && (sample as usize) < width {
                    horizontal[y * width + x] += bitmap[y * width + sample as usize] * weight;
                }
            }
        }
    }
    for y in 0..height {
        for x in 0..width {
            for (i, weight) in kernel.iter().enumerate() {
                let sample = y as isize + i as isize - radius as isize;
                if sample >= 0 && (sample as usize) < height {
                    result[y * width + x] += horizontal[sample as usize * width + x] * weight;
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::WHITE;

    #[test]
    fn font_draw_baseline_and_cell_outlined_emits_glyph_edge_contours() {
        let canvas = Canvas::default();
        let font_bytes = include_bytes!("../assets/fonts/ProggyClean.ttf");
        let font = FontFace::load(font_bytes, canvas.clone()).expect("load font");

        // Transparent outline / zero thickness should emit only 1 foreground quad per glyph
        font.draw_baseline("OK", vec2(10., 20.), 16., 1.0, WHITE);
        let quads = canvas.drain();
        assert_eq!(quads.len(), 2, "2 foreground quads for 'OK'");
        assert!(quads
            .iter()
            .all(|q| q.color == WHITE && q.texture.is_some()));

        // Active outline should emit 8 offset outline quads plus 1 foreground quad per glyph
        let border_color = Color::new(0.0, 0.0, 0.0, 0.45);
        let outline = TextOutline::new(border_color, 1.0);
        font.draw_baseline_outlined("OK", vec2(10., 20.), 16., 1.0, WHITE, outline);
        let quads = canvas.drain();
        // 2 characters * 8 outline offsets = 16 outline quads, + 2 foreground quads = 18 total quads
        assert_eq!(quads.len(), 18);

        let outline_quads: Vec<_> = quads.iter().filter(|q| q.color == border_color).collect();
        let fg_quads: Vec<_> = quads.iter().filter(|q| q.color == WHITE).collect();
        assert_eq!(outline_quads.len(), 16);
        assert_eq!(fg_quads.len(), 2);

        // Verify draw_cell_outlined also emits the 8 outline quads + foreground
        font.draw_cell_outlined("A", vec2(50., 60.), 16., WHITE, outline);
        let quads = canvas.drain();
        assert_eq!(quads.len(), 9, "8 outline + 1 fg for 'A'");
        let outline_count = quads.iter().filter(|q| q.color == border_color).count();
        let fg_count = quads.iter().filter(|q| q.color == WHITE).count();
        assert_eq!(outline_count, 8);
        assert_eq!(fg_count, 1);
    }

    #[test]
    fn symbol_bounds_center_full_and_split_glyphs_despite_text_ascent() {
        let canvas = Canvas::default();
        let mut font = FontFace::load(
            include_bytes!("../assets/fonts/ProggyClean.ttf"),
            canvas.clone(),
        )
        .unwrap();
        // A symbol's ink need not be symmetric around its nominal text cell.
        font.ascent += 0.3;
        for size in [16., 28., 56.] {
            for effects in [(0, 0), (2, 0)] {
                let bounds = font.glyph_bounds('(', size, effects);
                let origin = vec2(100., 240. - bounds.center().y);
                font.draw("(", origin.x, origin.y, size, effects, WHITE);
                let full = canvas.drain();
                assert_eq!(full.len(), 1);
                assert_eq!(full[0].destination.center().y, 240.);
                for split in [0., 0.25, 0.5, 0.75, 1.] {
                    font.draw_cropped("(", origin, size, effects, WHITE, vec2(0., split));
                    font.draw_cropped("(", origin, size, effects, WHITE, vec2(split, 1.));
                    let pieces = canvas.drain();
                    assert_eq!(pieces[0].destination.y, full[0].destination.y);
                    let last = pieces.last().unwrap().destination;
                    assert_eq!(
                        last.y + last.h,
                        full[0].destination.y + full[0].destination.h
                    );
                    if pieces.len() == 2 {
                        assert_eq!(pieces[0].destination.y + pieces[0].destination.h, last.y);
                    }
                }
            }
        }
    }

    #[test]
    fn font_draw_cropped_uses_normalized_fractions_without_bottom_truncation() {
        let canvas = Canvas::default();
        let font_bytes = include_bytes!("../assets/fonts/ProggyClean.ttf");
        let font = FontFace::load(font_bytes, canvas.clone()).expect("load font");

        // Full vertical span (0.0 to 1.0) must render the full glyph height
        font.draw_cropped("A", vec2(10., 20.), 16., (0, 0), WHITE, vec2(0., 1.));
        let quads = canvas.drain();
        assert_eq!(quads.len(), 1);
        let quad = &quads[0];
        let th = quad.texture.as_ref().unwrap().height();
        assert_eq!(quad.source.y, 0.);
        assert_eq!(quad.source.h, th);
        assert_eq!(quad.destination.h, th);

        // Half vertical span (0.0 to 0.5) must render top half
        font.draw_cropped("A", vec2(10., 20.), 16., (0, 0), WHITE, vec2(0., 0.5));
        let quads = canvas.drain();
        assert_eq!(quads.len(), 1);
        assert_eq!(quads[0].source.y, 0.);
        assert_eq!(quads[0].source.h, (0.5 * th).round());
        assert_eq!(quads[0].destination.h, (0.5 * th).round());

        // Bottom half (0.5 to 1.0) must render bottom half through th
        font.draw_cropped("A", vec2(10., 20.), 16., (0, 0), WHITE, vec2(0.5, 1.));
        let quads = canvas.drain();
        assert_eq!(quads.len(), 1);
        assert_eq!(quads[0].source.y, (0.5 * th).round());
        assert_eq!(quads[0].source.h, th - (0.5 * th).round());
        assert_eq!(quads[0].destination.h, th - (0.5 * th).round());
    }
}
