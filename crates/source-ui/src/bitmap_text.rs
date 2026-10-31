//! Setup-rasterized monospace bitmap text for diagnostic panels.
use crate::canvas::{draw_texture, Canvas, Color, FilterMode, Texture2D};
struct Glyph {
    texture: Texture2D,
    x: f32,
    y: f32,
}
/// Printable ASCII of one monospace font size.
pub struct BitmapFont {
    glyphs: Vec<Option<Glyph>>,
    pub advance: f32,
    pub line: f32,
    ascent: f32,
}
impl BitmapFont {
    pub fn load(px: f32) -> Option<Self> {
        let bytes: &[u8] = crate::PROGGY_CLEAN;
        let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()).ok()?;
        let lines = font.horizontal_line_metrics(px)?;
        let glyphs = (32u8..127)
            .map(|c| {
                let (metrics, coverage) = font.rasterize(char::from(c), px);
                if metrics.width == 0 || metrics.height == 0 {
                    return None;
                }
                let rgba: Vec<u8> = coverage.iter().flat_map(|a| [255, 255, 255, *a]).collect();
                let texture =
                    Texture2D::from_rgba8(metrics.width as u16, metrics.height as u16, &rgba);
                texture.set_filter(FilterMode::Nearest);
                Some(Glyph {
                    texture,
                    x: metrics.xmin as f32,
                    y: -(metrics.ymin as f32 + metrics.height as f32),
                })
            })
            .collect();
        Some(Self {
            glyphs,
            advance: font.metrics('M', px).advance_width.round(),
            line: lines.new_line_size.ceil(),
            ascent: lines.ascent.round(),
        })
    }
    pub fn text(&self, canvas: &Canvas, text: &str, x: f32, top: f32, color: Color) {
        let baseline = top + self.ascent;
        for (i, c) in text.chars().enumerate() {
            let glyph = u8::try_from(c)
                .ok()
                .and_then(|c| c.checked_sub(32))
                .and_then(|i| self.glyphs.get(usize::from(i)))
                .and_then(Option::as_ref);
            if let Some(g) = glyph {
                draw_texture(
                    canvas,
                    &g.texture,
                    (x + i as f32 * self.advance + g.x).round(),
                    (baseline + g.y).round(),
                    color,
                );
            }
        }
    }
}
