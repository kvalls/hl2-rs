//! Letter-edge outlines that grow outward only: each glyph gets one cached "ring" texture, its
//! coverage dilated by the outline radius minus its own coverage, drawn once under the glyph.
use super::FontFace;
use crate::canvas::{FilterMode, Texture2D};

/// A glyph's outline texture and how far it reaches past the glyph bitmap on each side.
#[derive(Clone)]
pub(super) struct Ring {
    pub texture: Texture2D,
    pub radius: f32,
}

impl FontFace {
    /// The cached outline ring of `c` rasterized at `em` for an outline `thickness` pixels wide.
    pub(super) fn ring(&self, c: char, em: f32, thickness: f32) -> Option<Ring> {
        let radius = thickness.ceil().clamp(1., 8.) as u16;
        let key = (c, em.to_bits(), radius);
        if let Some(ring) = self.rings.borrow().get(&key) {
            return ring.clone();
        }
        let ring = self.raster(c, em, 0, 0).texture.map(|glyph| {
            let coverage: Vec<u8> = glyph.0.rgba.iter().skip(3).step_by(4).copied().collect();
            let (width, height) = (usize::from(glyph.0.width), usize::from(glyph.0.height));
            let alpha = ring_alpha(&coverage, width, height, usize::from(radius));
            let pad = usize::from(radius) * 2;
            let rgba: Vec<u8> = alpha.iter().flat_map(|&a| [255, 255, 255, a]).collect();
            let texture = Texture2D::from_rgba8((width + pad) as u16, (height + pad) as u16, &rgba);
            texture.set_filter(FilterMode::Linear);
            Ring {
                texture,
                radius: f32::from(radius),
            }
        });
        self.rings.borrow_mut().insert(key, ring.clone());
        ring
    }
}

/// Outline coverage around a `width` x `height` coverage bitmap, padded by `radius` texels on
/// every side: the coverage dilated over a disc of `radius` (radius 1 is the 3x3 square) minus
/// the glyph's own coverage. It is zero where the glyph is solid and never exceeds the part of
/// a texel the glyph leaves uncovered, so drawn beneath the glyph it only darkens the backdrop
/// outside the letter and the glyph keeps its own weight.
pub(super) fn ring_alpha(coverage: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    let (out_w, out_h) = (width + radius * 2, height + radius * 2);
    let r = radius as isize;
    // r*r + r keeps the corners at radius 1 and rounds the disc at larger radii.
    let offsets: Vec<(isize, isize)> = (-r..=r)
        .flat_map(|dy| (-r..=r).map(move |dx| (dx, dy)))
        .filter(|(dx, dy)| dx * dx + dy * dy <= r * r + r)
        .collect();
    let at = |x: isize, y: isize| -> u8 {
        let (x, y) = (x - r, y - r);
        if x < 0 || y < 0 || x >= width as isize || y >= height as isize {
            0
        } else {
            coverage[y as usize * width + x as usize]
        }
    };
    let mut ring = vec![0u8; out_w * out_h];
    for y in 0..out_h as isize {
        for x in 0..out_w as isize {
            let dilated = offsets
                .iter()
                .map(|&(dx, dy)| at(x + dx, y + dy))
                .max()
                .unwrap_or(0);
            ring[y as usize * out_w + x as usize] = dilated.saturating_sub(at(x, y));
        }
    }
    ring
}
