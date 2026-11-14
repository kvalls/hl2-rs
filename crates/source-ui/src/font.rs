//! Font faces drawn into a [`Canvas`]: character-cell sizing like the retail `CWin32Font`,
//! a glyph cache, optional blur and scanlines, and letter-edge outlines.
use crate::canvas::{
    draw_texture, draw_texture_ex, Canvas, Color, DrawTextureParams, FilterMode, MutexCell, Rect,
    Texture2D,
};
mod button;
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
    monochrome: MonochromeCache,
    cell_height: f32,
    ascent: f32,
    /// OS/2 usWinAscent per EM: the GDI cell's ascent.
    win_ascent: f32,
    freetype: Option<FreeTypeCell>,
}
type GlyphCache = Arc<MutexCell<HashMap<(char, u32, u16, u16), Glyph>>>;
type MonochromeCache = Arc<MutexCell<HashMap<(char, u16, u32), Option<Texture2D>>>>;
#[derive(Clone)]
struct Glyph {
    texture: Option<Texture2D>,
    x: f32,
    y: f32,
    advance: f32,
}
/// How a requested cell height (`tall`) maps to glyph size and where glyphs sit in the cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellBaseline {
    /// Retail CWin32Font: the baseline is tmAscent (OS/2 usWinAscent) below the cell top.
    Gdi,
    /// Capitals centered in the cell (OS/2 sCapHeight), used by the GameUI dialogs and menus.
    CapCentered,
    /// Retail CLinuxFont (FreeType), which `resource/FontInfo.kv` forces for GorDIN: see
    /// [`FreeTypeCell`]. Falls back to [`CellBaseline::Gdi`] when the font has no metrics.
    FreeType,
}

/// Retail CLinuxFont::CreateFromMemory character cells. The cell is the ascender rounded up to
/// whole pixels minus the scaled bounding-box bottom; the pixel size is searched (from `tall`
/// down, or up) for the largest one whose cell fits `tall`, and the baseline is that ascender.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FreeTypeCell {
    units_per_em: i64,
    ascender: i64,
    y_min: i64,
}
impl FreeTypeCell {
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let face = ttf_parser::Face::parse(bytes, 0).ok()?;
        let units_per_em = i64::from(face.units_per_em());
        // The ascender gives way to 'Ó' (U+00D3) when that glyph rises above it.
        let accent = face
            .glyph_index('\u{d3}')
            .and_then(|glyph| face.glyph_bounding_box(glyph))
            .map_or(i64::MIN, |rect| i64::from(rect.y_max));
        (units_per_em > 0).then(|| Self {
            units_per_em,
            ascender: i64::from(face.ascender()).max(accent),
            y_min: i64::from(face.global_bounding_box().y_min),
        })
    }
    /// Cell height and ascent in whole pixels at a FreeType pixel size (26.6 fixed point).
    fn cell(&self, pixels: i64) -> (i64, i64) {
        let mul_fix = |a: i64, b: i64| {
            let product = a * b;
            product.signum() * ((product.abs() + 0x8000) >> 16)
        };
        let scale = ((pixels << 22) + self.units_per_em / 2) / self.units_per_em;
        let ascent = (mul_fix(self.ascender, scale) + 63) & !63;
        let height = ascent - mul_fix(self.y_min, scale);
        ((height + 32) >> 6, ascent >> 6)
    }
    /// The pixel size (EM) and baseline (pixels below the cell top) for a requested `tall`.
    pub fn size_for(&self, tall: f32) -> (f32, f32) {
        let tall = tall.round().max(1.) as i64;
        let (mut pixels, mut step, mut first) = (tall, -1, true);
        loop {
            let (height, ascent) = self.cell(pixels);
            if height == tall || height < 7 || pixels < 2 {
                return (pixels as f32, ascent as f32);
            }
            if first {
                first = false;
                if height < tall {
                    step = 1;
                }
                pixels += step;
            } else if step < 1 {
                if height <= tall {
                    return (pixels as f32, ascent as f32);
                }
                pixels -= 1;
            } else if height <= tall {
                pixels += 1;
            } else {
                step = -1;
                pixels -= 1;
            }
        }
    }
    /// Baseline below the cell top for glyphs rasterized at `em` pixels.
    fn ascent_at(&self, em: f32) -> f32 {
        (self.ascender as f32 * em / self.units_per_em as f32).ceil()
    }
}

/// Retail CWin32Font::Create cells: `CreateFontA(+tall)` lets GDI pick the largest pixel size
/// whose VDMX row (yMax - yMin) fits `tall`, and that row's yMax is tmAscent (the baseline
/// below the cell top). Probed with GDI: HL2EP2 tall 72 -> 75 px, ascent 69; tall 48 -> 50, 46;
/// halflife2 tall 48 -> 41 px. Fonts without a VDMX row scale the OS/2 Windows metrics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GdiCell {
    /// (pixel size, ascent) for each requested height; 0 where VDMX has no fitting row.
    cells: [(u8, u8); 256],
    units_per_em: f32,
    win_ascent: f32,
    win_height: f32,
}
impl GdiCell {
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let face = ttf_parser::Face::parse(bytes, 0).ok()?;
        let os2 = face.tables().os2?;
        let win_ascent = f32::from(os2.windows_ascender());
        let win_height = win_ascent + f32::from(os2.windows_descender()).abs();
        let units_per_em = f32::from(face.units_per_em());
        if win_height <= 0. || units_per_em <= 0. {
            return None;
        }
        let rows = face
            .raw_face()
            .table(ttf_parser::Tag::from_bytes(b"VDMX"))
            .map(vdmx_rows)
            .unwrap_or_default();
        Some(Self::from_rows(&rows, units_per_em, win_ascent, win_height))
    }
    /// Cells from VDMX (pixel size, yMax, yMin) rows and the OS/2 Windows metrics.
    fn from_rows(
        rows: &[(u8, i16, i16)],
        units_per_em: f32,
        win_ascent: f32,
        win_height: f32,
    ) -> Self {
        let mut cells = [(0, 0); 256];
        for (tall, cell) in cells.iter_mut().enumerate() {
            // The tallest row that fits; of rows with that height GDI takes the smallest size
            // (rows ascend by pixel size): HL2EP2 76 and 77 px are both 74 tall, CreateFont(74)
            // gives 76 (probed advances).
            let mut best: Option<(i32, u8, i16)> = None;
            for &(pixels, y_max, y_min) in rows {
                let height = i32::from(y_max) - i32::from(y_min);
                if height <= tall as i32 && best.is_none_or(|(fit, ..)| height > fit) {
                    best = Some((height, pixels, y_max));
                }
            }
            if let Some((_, pixels, y_max)) = best {
                *cell = (pixels, y_max.clamp(0, 255) as u8);
            }
        }
        Self {
            cells,
            units_per_em,
            win_ascent,
            win_height,
        }
    }
    /// The pixel size (EM) and baseline (pixels below the cell top) for a requested `tall`.
    pub fn size_for(&self, tall: f32) -> (f32, f32) {
        let tall = tall.round().max(1.);
        if let Some(&(pixels, ascent)) = self.cells.get(tall as usize).filter(|cell| cell.0 > 0) {
            return (f32::from(pixels), f32::from(ascent));
        }
        let em = (tall * self.units_per_em / self.win_height).floor().max(1.);
        (em, (self.win_ascent * em / self.units_per_em).round())
    }
}

/// The 1:1 (or default) VDMX ratio group as (pixel size, yMax, yMin) rows.
fn vdmx_rows(data: &[u8]) -> Vec<(u8, i16, i16)> {
    let u16_at = |at: usize| {
        data.get(at..at + 2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
    };
    let Some(ratios) = u16_at(4) else {
        return Vec::new();
    };
    let ratios = usize::from(ratios);
    // RatioRange: charSet, xRatio, yStartRatio, yEndRatio; 0:0 matches any aspect.
    let group = (0..ratios).find(|&index| {
        data.get(6 + 4 * index..10 + 4 * index)
            .is_some_and(|r| (r[1] == 0 && r[2] == 0) || (r[1] == r[2] && r[1] == r[3]))
    });
    let Some(offset) = group.and_then(|index| u16_at(6 + 4 * ratios + 2 * index)) else {
        return Vec::new();
    };
    let base = usize::from(offset);
    let records = u16_at(base).map_or(0, usize::from);
    (0..records)
        .filter_map(|index| {
            let at = base + 4 + 6 * index;
            let pixels = u16_at(at)?;
            let y_max = u16_at(at + 2)? as i16;
            let y_min = u16_at(at + 4)? as i16;
            Some((u8::try_from(pixels).ok()?, y_max, y_min))
        })
        .collect()
}

impl FontFace {
    pub fn load(bytes: &[u8], canvas: Canvas) -> Result<Self> {
        Self::load_with(bytes, canvas, CellBaseline::CapCentered)
    }
    pub fn load_with(bytes: &[u8], canvas: Canvas, baseline: CellBaseline) -> Result<Self> {
        let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
            .map_err(anyhow::Error::msg)?;
        let face = ttf_parser::Face::parse(bytes, 0).context("read HUD font metrics")?;
        let units = f32::from(face.units_per_em()).max(1.);
        let (ascent, cell_height, win_ascent) = face.tables().os2.map_or_else(
            || {
                let metrics = font.horizontal_line_metrics(1.).unwrap();
                (
                    metrics.ascent,
                    metrics.ascent - metrics.descent,
                    metrics.ascent,
                )
            },
            |table| {
                let win_ascent = f32::from(table.windows_ascender()) / units;
                let descent = f32::from(table.windows_descender()).abs() / units;
                let cell_height = win_ascent + descent;
                let ascent = face
                    .capital_height()
                    .filter(|&cap| cap > 0 && baseline == CellBaseline::CapCentered)
                    .map_or(win_ascent, |cap| {
                        (cell_height + f32::from(cap) / units) * 0.5
                    });
                (ascent, cell_height, win_ascent)
            },
        );
        Ok(Self {
            canvas,
            font: Arc::new(font),
            glyphs: Arc::new(MutexCell::new(HashMap::new())),
            monochrome: Arc::new(MutexCell::new(HashMap::new())),
            cell_height: cell_height.max(0.01),
            ascent,
            win_ascent,
            freetype: (baseline == CellBaseline::FreeType)
                .then(|| FreeTypeCell::from_bytes(bytes))
                .flatten(),
        })
    }
    /// The EM a cell of `cell` pixels rasterizes at: retail CLinuxFont pixel sizes, else the
    /// cell over the OS/2 cell height (unrounded, as measurement and scaled draws use it).
    fn cell_em(&self, cell: f32) -> f32 {
        self.freetype
            .map_or(cell / self.cell_height, |ft| ft.size_for(cell).0)
    }
    /// Retail CWin32Font's height (GetFontTall, the GDI tmHeight) for a cell of `tall`
    /// pixels: GDI renders at a whole-pixel EM and the cell's ascent and descent round up,
    /// so it can come out a pixel taller than asked (native CHudHintKeyDisplay rows:
    /// Verdana bold at tall 21 is 22 high). FreeType cells keep `tall`.
    pub fn gdi_height(&self, tall: f32) -> f32 {
        let tall = tall.round().max(1.);
        if self.freetype.is_some() {
            return tall;
        }
        let em = (tall / self.cell_height).round();
        let up = |v: f32| (v - 1e-3).ceil();
        up(self.win_ascent * em) + up((self.cell_height - self.win_ascent) * em)
    }
    /// Baseline below the cell top for glyphs rasterized at `em` pixels.
    fn ascent_at(&self, em: f32) -> f32 {
        self.freetype
            .map_or(self.ascent * em, |ft| ft.ascent_at(em))
    }
    pub fn width(&self, value: &str, size: f32) -> f32 {
        // Retail CWin32Font uses a positive CreateFontA height: a character cell,
        // whereas fontdue's size is pixels per EM. Windows OS/2 metrics supply the
        // conversion; hhea descent differs significantly in the owned HalfLife2 font.
        let size = self.cell_em(size.round().max(1.));
        value
            .chars()
            .map(|c| self.font.metrics(c, size).advance_width)
            .sum()
    }
    fn glyph(&self, c: char, size: u16, blur: u16, scanlines: u16) -> Glyph {
        self.raster(c, self.cell_em(f32::from(size)), blur, scanlines)
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
        let ascent = self.ascent_at(em_size);
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
    /// A glyph with hard edges for symbol fonts drawn without antialiasing: every texel is fully
    /// on or off, point sampled, so it stays crisp at whole-pixel positions. `width_scale` moves
    /// texels toward the middle column, for a font engine whose hinting set narrower spacing.
    /// Cached per size and scale.
    pub fn monochrome_glyph(
        &self,
        character: char,
        size: f32,
        width_scale: f32,
    ) -> Option<Texture2D> {
        let size = size.round().clamp(1., 1024.) as u16;
        let key = (character, size, width_scale.to_bits());
        if let Some(texture) = self.monochrome.borrow().get(&key) {
            return texture.clone();
        }
        let texture = self.glyph(character, size, 0, 0).texture.map(|soft| {
            let rgba = hard_edges(&soft.0.rgba, usize::from(soft.0.width), width_scale);
            let texture = Texture2D::from_rgba8(soft.0.width, soft.0.height, &rgba);
            texture.set_filter(FilterMode::Nearest);
            texture
        });
        self.monochrome.borrow_mut().insert(key, texture.clone());
        texture
    }
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
        self.width_em(value, self.cell_em(cell))
    }
    /// The whole-pixel EM cell draws rasterize at.
    fn raster_em(&self, cell: f32) -> f32 {
        self.cell_em(cell).round().clamp(1., 1024.)
    }
    /// Pixel width of `value` as [`Self::draw_cell`] draws it in a cell of `cell` pixels. The
    /// whole-pixel EM makes this differ from a [`Self::width_cell`] measured at another scale.
    pub fn drawn_width_cell(&self, value: &str, cell: f32) -> f32 {
        self.width_em(value, self.raster_em(cell))
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
        let raster = self.raster_em(cell);
        self.draw_baseline_outlined(
            value,
            (top_left + Vec2::new(0., self.ascent_at(raster))).round(),
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
                            (pen.y + glyph.y - self.ascent_at(em)).round(),
                        ),
                        texture.size(),
                    )
                } else {
                    (
                        vec2(
                            pen.x + glyph.x * scale,
                            pen.y + (glyph.y - self.ascent_at(em)) * scale,
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

/// White texels, opaque where coverage reaches half and transparent elsewhere, with columns
/// scaled about the middle one.
fn hard_edges(rgba: &[u8], width: usize, width_scale: f32) -> Vec<u8> {
    let mut hard = [255, 255, 255, 0].repeat(rgba.len() / 4);
    let middle = (width / 2) as f32;
    for (i, texel) in rgba.as_chunks::<4>().0.iter().enumerate() {
        if texel[3] >= 128 {
            let (row, column) = (i / width, i % width);
            let moved = middle + ((column as f32 - middle) * width_scale).round();
            let column = (moved.max(0.) as usize).min(width - 1);
            hard[(row * width + column) * 4 + 3] = 255;
        }
    }
    hard
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
    fn freetype_cells_pick_the_largest_pixel_size_that_fits_like_retail_clinuxfont() {
        // GorDIN (installed hl2_complete): 1000 units per EM, hhea ascender 968, bbox yMin -232.
        // Retail draws the 1080p menu (MenuLarge tall 12 -> 27) at 22 px with a 22 px baseline,
        // where GDI cells (OS/2 win metrics, 1403 units) would give 19 px glyphs.
        let gordin = FreeTypeCell {
            units_per_em: 1000,
            ascender: 968,
            y_min: -232,
        };
        for (tall, size) in [
            (16., 13.),
            (17., 14.),
            (22., 18.),
            (24., 19.),
            (27., 22.),
            (36., 29.),
        ] {
            assert_eq!(gordin.size_for(tall), (size, size), "tall {tall}");
        }
        assert_eq!(gordin.cell(22), (27, 22));
        assert_eq!(gordin.cell(23), (28, 23));
    }

    #[test]
    fn gdi_cells_follow_the_vdmx_table_like_createfont() {
        // HL2EP2 (hl2/resource/hl2ep2.ttf): 1000 units per EM, usWin 909/40, VDMX 1:1 rows.
        // GDI probe: CreateFont(72) -> 75 px, tmAscent 69; 73 -> 75/69; 74 -> 76/70 (not 77, also
        // 74 tall); 48 -> 50/46.
        let mut table = vec![0, 0, 0, 1, 0, 1, 0, 1, 1, 1];
        let rows: [(u16, i16, i16); 6] = [
            (50, 46, -2),
            (51, 47, -3),
            (74, 68, -3),
            (75, 69, -3),
            (76, 70, -4),
            (77, 70, -4),
        ];
        let group = (table.len() + 2) as u16;
        table.extend(group.to_be_bytes());
        table.extend((rows.len() as u16).to_be_bytes());
        table.extend([50, 77]);
        for (pixels, y_max, y_min) in rows {
            table.extend(pixels.to_be_bytes());
            table.extend(y_max.to_be_bytes());
            table.extend(y_min.to_be_bytes());
        }
        let parsed = vdmx_rows(&table);
        assert_eq!(parsed.len(), 6);
        let cell = GdiCell::from_rows(&parsed, 1000., 909., 949.);
        for (tall, expected) in [
            (72., (75., 69.)),
            (73., (75., 69.)),
            (74., (76., 70.)),
            (48., (50., 46.)),
        ] {
            assert_eq!(cell.size_for(tall), expected, "tall {tall}");
        }
        // Outside the table the Windows metrics scale: 30 -> floor(31.6) = 31 px, ascent 28.
        assert_eq!(cell.size_for(30.), (31., 28.));
    }

    #[test]
    fn hard_edges_turn_coverage_fully_on_or_off() {
        let soft = [9, 9, 9, 127, 9, 9, 9, 128, 9, 9, 9, 0, 9, 9, 9, 255];
        assert_eq!(
            hard_edges(&soft, 4, 1.),
            [255, 255, 255, 0, 255, 255, 255, 255, 255, 255, 255, 0, 255, 255, 255, 255]
        );
    }

    #[test]
    fn hard_edges_scale_columns_about_the_middle() {
        let opaque = |columns: &[usize]| {
            let mut rgba = [255, 255, 255, 0].repeat(23);
            for column in columns {
                rgba[column * 4 + 3] = 255;
            }
            rgba
        };
        assert_eq!(
            hard_edges(&opaque(&[0, 11, 22]), 23, 10. / 11.),
            opaque(&[1, 11, 21])
        );
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
    fn gdi_height_rounds_the_em_and_the_cell_edges() {
        let font = FontFace::load_with(
            include_bytes!("../assets/fonts/ProggyClean.ttf"),
            Canvas::default(),
            CellBaseline::Gdi,
        )
        .unwrap();
        // ProggyClean: usWinAscent 1280 and usWinDescent 384 of 2048 units per EM.
        // A 13 px cell renders at EM 16: ascent 10 and descent 3 fit exactly.
        assert_eq!(font.gdi_height(13.), 13.);
        // A 16 px cell renders at EM 20: ascent 12.5 and descent 3.75 round up to 17.
        assert_eq!(font.gdi_height(16.), 17.);
        assert_eq!(font.gdi_height(21.), 22.);
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
