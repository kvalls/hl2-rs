//! F4 NPC spawn overlay drawing (DESIGN 14w step 4): a two-column panel on the HUD
//! canvas (classes | weapons and options) in the project's ProggyClean developer font
//! (source-ui/assets/fonts, MIT). Glyphs are rasterized once at setup; frame systems only
//! queue quads. The panel content comes from `crate::spawn::Overlay`.
use crate::spawn::{Panel, PanelRow, PanelTarget};
use source_ui::canvas::{Canvas, Color};

pub struct PanelFont(pub std::sync::Arc<source_ui::bitmap_text::BitmapFont>);
impl std::ops::Deref for PanelFont {
    type Target = source_ui::bitmap_text::BitmapFont;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl PanelFont {
    #[cfg(test)]
    pub fn load(px: f32) -> Option<Self> {
        source_ui::bitmap_text::BitmapFont::load(px).map(|font| Self(std::sync::Arc::new(font)))
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
        let tabs_top = top + pad + row;
        self.draw_tabs(canvas, panel, tabs_top, &mut hits);
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
        self.draw_rows(canvas, &panel.classes, [LEFT, header, column], &mut hits);
        self.draw_rows(
            canvas,
            &panel.options,
            [LEFT + column, header, options_width],
            &mut hits,
        );
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
    /// Tab bar: NPCs | Props | the filter box.
    fn draw_tabs(
        &self,
        canvas: &Canvas,
        panel: &Panel,
        tabs_top: f32,
        hits: &mut Vec<([f32; 4], PanelTarget)>,
    ) {
        let row = self.row();
        let mut x = LEFT + PAD;
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
    }
    /// One column of rows below `header`; `at` is the column's x, the header's y and its width.
    fn draw_rows(
        &self,
        canvas: &Canvas,
        rows: &[PanelRow],
        [x, header, w]: [f32; 3],
        hits: &mut Vec<([f32; 4], PanelTarget)>,
    ) {
        let row = self.row();
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
            self.text(canvas, &format!("{mark}{}", r.text), x + PAD, top, color);
            if let Some(target) = r.target {
                hits.push(([x, top - 2., w, row], target));
            }
        }
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
pub fn column_at(hits: &[([f32; 4], PanelTarget)], x: f32) -> Option<crate::spawn::Column> {
    use crate::spawn::PanelTarget::{Page, Row};
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
        assert!(font.advance > 0. && font.line > 0.);
        let canvas = source_ui::canvas::Canvas::default();
        font.text(
            &canvas,
            " ",
            0.,
            0.,
            source_ui::canvas::Color::new(1., 1., 1., 1.),
        );
        assert!(canvas.drain().is_empty(), "space has no ink");
        font.text(
            &canvas,
            "A",
            0.,
            0.,
            source_ui::canvas::Color::new(1., 1., 1., 1.),
        );
        assert_eq!(canvas.drain().len(), 1, "letters emit glyph geometry");
    }
    /// A top edge below the F1 and F5 overlays.
    const TOP: f32 = 118.;
    #[test]
    fn drawn_rows_are_clickable_where_they_are_drawn() {
        use crate::spawn::{Column, Overlay, OverlayKey, PanelTarget, Tab};
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
