//! Button fitting against the same whole-pixel raster metrics used by cell text.
use super::FontFace;

impl FontFace {
    /// Width sufficient for equal visible side padding without moving or scaling the text.
    pub fn button_label_width(&self, label: &str, cell: f32) -> f32 {
        let em = self.cell_em(cell).round().clamp(1., 1024.);
        let mut pen = 0.;
        let mut left = f32::INFINITY;
        let mut right = 0f32;
        for character in label.chars() {
            let metrics = self.font.metrics(character, em);
            if metrics.width > 0 && metrics.height > 0 {
                let x = (pen + metrics.xmin as f32).round();
                left = left.min(x);
                right = right.max(x + metrics.width as f32);
            }
            pen += metrics.advance_width;
        }
        if !left.is_finite() {
            return pen;
        }
        pen.max(right + left.max(0.))
    }
}
