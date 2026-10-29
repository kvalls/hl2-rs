//! F4 NPC spawn overlay drawing (DESIGN 14w step 4): a two-column panel on the HUD
//! canvas (classes | weapons and options) in the project's ProggyClean developer font
//! (source-ui/assets/fonts, MIT). Glyphs are rasterized once at setup; frame systems only
//! queue quads. The panel content comes from `hl2_simulation::ai::spawn::Overlay`.
use hl2_simulation::ai::spawn::{Panel, PanelRow, PanelTarget};
use source_ui::canvas::{Canvas, Color, FilterMode, Texture2D, draw_texture};

struct Glyph {
    texture: Texture2D,
    x: f32,
    y: f32,
}
/// Printable ASCII of one monospace font size.
pub struct PanelFont {
    glyphs: Vec<Option<Glyph>>,
    advance: f32,
    line: f32,
    ascent: f32,
}
impl PanelFont {
    pub fn load(px: f32) -> Option<Self> {
        let bytes: &[u8] = source_ui::PROGGY_CLEAN;
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
    /// Row height of the panel lists.
    fn row(&self) -> f32 {
        (self.line * 1.15).round()
    }
    /// List rows that fit in a canvas `height` below the title, tabs, headings, the two
    /// "more" notes and the message footer.
    pub fn rows_fit(&self, top: f32, height: f32) -> usize {
        let free = height - top - 2. * PAD - 6. * self.row();
        ((free / self.row()).floor() as usize).max(5)
    }
    /// Draw the overlay panel at the left of the screen with its top edge at `top`; returns each
    /// clickable row's rectangle (x, y, width, height in canvas pixels) and target.
    pub fn draw(
        &self,
        canvas: &Canvas,
        panel: &Panel,
        message: Option<&str>,
        top: f32,
    ) -> Vec<([f32; 4], PanelTarget)> {
        let pad = PAD;
        let row = self.row();
        let mut hits = Vec::new();
        let class_chars = panel
            .classes
            .iter()
            .map(|r| r.text.len() + 2)
            .max()
            .unwrap_or(0)
            .max(24);
        let option_chars = panel
            .options
            .iter()
            .map(|r| r.text.len() + 2)
            .max()
            .unwrap_or(0)
            .max(30);
        let column = class_chars as f32 * self.advance + 2. * pad;
        let options_width = option_chars as f32 * self.advance + 2. * pad;
        let title_width = panel.title.len() as f32 * self.advance + 2. * pad;
        let message_width = message.map_or(0., |m| (m.len() + 6) as f32 * self.advance + 2. * pad);
        let width = (column + options_width).max(title_width).max(message_width);
        let rows = panel.classes.len().max(panel.options.len()) + 1;
        let footer = usize::from(message.is_some());
        let height = (rows + 2 + footer) as f32 * row + 2. * pad;
        canvas.rectangle(
            LEFT,
            top,
            width,
            height,
            Color::new(0.025, 0.045, 0.07, 0.88),
        );
        let title = Color::from_rgba(240, 180, 90, 255);
        self.text(canvas, &panel.title, LEFT + pad, top + pad, title);
        // Tab bar: NPCs | Props | the filter box.
        let tabs_top = top + pad + row;
        let mut x = LEFT + pad;
        for tab in &panel.tabs {
            let w = (tab.text.len() + 2) as f32 * self.advance;
            let fill = if tab.cursor {
                Color::new(0.85, 0.55, 0.15, 0.45)
            } else if tab.selected {
                Color::new(0.3, 0.4, 0.55, 0.7)
            } else {
                Color::new(0.15, 0.18, 0.22, 0.7)
            };
            canvas.rectangle(x, tabs_top - 2., w, row, fill);
            let color = if tab.selected || tab.cursor {
                Color::from_rgba(255, 238, 160, 255)
            } else {
                Color::new(0.78, 0.78, 0.78, 1.)
            };
            self.text(canvas, &tab.text, x + self.advance, tabs_top, color);
            if let Some(target) = tab.target {
                hits.push(([x, tabs_top - 2., w, row], target));
            }
            x += w + self.advance;
        }
        let header = tabs_top + row;
        let classes_focused = panel.classes.iter().any(|r| r.cursor);
        let heading = |focused: bool| {
            if focused {
                Color::new(1., 1., 1., 1.)
            } else {
                Color::new(0.55, 0.55, 0.55, 1.)
            }
        };
        self.text(
            canvas,
            &panel.headings[0],
            LEFT + pad,
            header,
            heading(classes_focused),
        );
        self.text(
            canvas,
            &panel.headings[1],
            LEFT + column + pad,
            header,
            heading(!classes_focused),
        );
        let mut draw_rows = |rows: &[PanelRow], x: f32, w: f32| {
            for (i, r) in rows.iter().enumerate() {
                let top = header + (i + 1) as f32 * row;
                if r.cursor {
                    canvas.rectangle(x, top - 2., w, row, Color::new(0.85, 0.55, 0.15, 0.45));
                }
                let color = if r.selected || r.cursor {
                    Color::from_rgba(255, 238, 160, 255)
                } else {
                    Color::new(0.78, 0.78, 0.78, 1.)
                };
                let mark = if r.selected { "> " } else { "  " };
                self.text(canvas, &format!("{mark}{}", r.text), x + pad, top, color);
                if let Some(target) = r.target {
                    hits.push(([x, top - 2., w, row], target));
                }
            }
        };
        draw_rows(&panel.classes, LEFT, column);
        draw_rows(&panel.options, LEFT + column, options_width);
        if let Some(message) = message {
            let top = header + rows as f32 * row;
            self.text(
                canvas,
                &format!("last: {message}"),
                LEFT + pad,
                top,
                Color::new(0.78, 0.78, 0.78, 1.),
            );
        }
        hits
    }
}
const LEFT: f32 = 20.;
const PAD: f32 = 10.;
/// The drawn row under a pointer position (canvas pixels), from `PanelFont::draw`.
pub fn hit(hits: &[([f32; 4], PanelTarget)], x: f32, y: f32) -> Option<PanelTarget> {
    hits.iter()
        .find(|([left, top, w, h], _)| x >= *left && x < left + w && y >= *top && y < top + h)
        .map(|(_, target)| *target)
}
/// The column under a pointer position (for the wheel), from the drawn rows.
pub fn column_at(
    hits: &[([f32; 4], PanelTarget)],
    x: f32,
) -> Option<hl2_simulation::ai::spawn::Column> {
    use hl2_simulation::ai::spawn::PanelTarget::{Page, Row};
    hits.iter()
        .find_map(|([left, _, w, _], target)| match target {
            Row(column, _) | Page(column, _) if x >= *left && x < left + w => Some(*column),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_developer_font_rasterizes_printable_ascii_at_setup() {
        let font = super::PanelFont::load(18.).expect("ProggyClean");
        assert!(font.advance > 0. && font.line > font.ascent && font.ascent > 0.);
        // Space has no coverage; letters do.
        assert!(font.glyphs[0].is_none());
        assert!(font.glyphs[usize::from(b'A' - 32)].is_some());
    }
    /// A top edge below the F1 and F5 overlays.
    const TOP: f32 = 118.;
    #[test]
    fn drawn_rows_are_clickable_where_they_are_drawn() {
        use hl2_simulation::ai::spawn::{Column, Overlay, OverlayKey, PanelTarget, Tab};
        let font = super::PanelFont::load(18.).expect("ProggyClean");
        let canvas = source_ui::canvas::Canvas::default();
        let mut overlay = Overlay::default();
        overlay.key(OverlayKey::Toggle);
        overlay.rows_visible = font.rows_fit(TOP, 720.);
        let hits = font.draw(&canvas, &overlay.panel(true, false), Some("spawned"), TOP);
        // Every list row fits a 720-pixel window.
        assert!(hits.iter().all(|([_, top, _, h], _)| top + h <= 720.));
        let center = |target| {
            let ([x, y, w, h], _) = hits.iter().find(|(_, t)| *t == target).unwrap();
            (x + w / 2., y + h / 2.)
        };
        for target in [
            PanelTarget::Tab(Tab::Props),
            PanelTarget::Row(Column::Classes, 3),
            PanelTarget::Row(Column::Options, 0),
        ] {
            let (x, y) = center(target);
            assert_eq!(super::hit(&hits, x, y), Some(target));
        }
        let (x, _) = center(PanelTarget::Row(Column::Options, 0));
        assert_eq!(super::column_at(&hits, x), Some(Column::Options));
        assert_eq!(super::hit(&hits, 5., 5.), None);
    }
}
