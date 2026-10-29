//! Packs each face's primary RGBExp32 baked light samples into GPU atlas pages.
use crate::{bytes, i32le, records};
use anyhow::{bail, Result};
use modkit_core::Lightmap;
const SIZE: usize = 1024;
const ONE: u16 = 0x3c00;
/// 65535 / 4096: the integer HDR lightmap range.
pub const MAX_LIGHT: f32 = 65535. / 4096.;
/// Native HL2 runs the HDR path: use the HDR lighting copies (lightmaps 53 with faces 58,
/// ambient 51/55, world lights 54) when the map has them, otherwise the LDR copies.
pub fn use_hdr(lumps: &[Vec<u8>]) -> bool {
    let lump = |id: usize| lumps.get(id).is_none_or(Vec::is_empty);
    lump(7) || (!lump(53) && !lump(58))
}
/// The lighting copies to read: the HDR ones when `hdr` is wanted and the map has them; an LDR
/// frame (`mat_hdr_level` below 2) reads the LDR ones, unless the map has none.
pub fn choose_hdr(lumps: &[Vec<u8>], hdr: bool) -> bool {
    use_hdr(lumps) && (hdr || lumps.get(7).is_none_or(Vec::is_empty))
}
#[derive(Clone, Copy)]
pub struct FaceLight {
    pub page: usize,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}
pub fn build(lumps: &[Vec<u8>], hdr: bool) -> Result<(Vec<Lightmap>, Vec<Option<FaceLight>>)> {
    let hdr = choose_hdr(lumps, hdr);
    let data = &lumps[if hdr { 53 } else { 8 }];
    let faces = records(&lumps[if hdr { 58 } else { 7 }], 56)?;
    let mut pages = Vec::<Lightmap>::new();
    let mut refs = Vec::new();
    let (mut x, mut y, mut row) = (1usize, 1usize, 0usize);
    for face in faces {
        let offset = i32le(face, 20)?;
        if offset < 0 || data.is_empty() {
            refs.push(None);
            continue;
        }
        let width = usize::try_from(i32le(face, 36)?.max(0))? + 1;
        let height = usize::try_from(i32le(face, 40)?.max(0))? + 1;
        if width + 2 > SIZE || height + 2 > SIZE {
            refs.push(None);
            continue;
        }
        let samples = bytes(data, offset as usize, width * height * 4)?;
        if x + width + 1 > SIZE {
            x = 1;
            y += row + 2;
            row = 0;
        }
        if pages.is_empty() || y + height + 1 > SIZE {
            if pages.len() >= 64 {
                bail!("lightmap atlas exceeds 64 pages");
            }
            pages.push(Lightmap {
                width: SIZE as u16,
                height: SIZE as u16,
                rgba: vec![ONE; SIZE * SIZE * 4],
            });
            x = 1;
            y = 1;
            row = 0;
        }
        let page = pages.len() - 1;
        let atlas = &mut pages[page];
        for dy in 0..height {
            for dx in 0..width {
                let s = &samples[(dy * width + dx) * 4..][..4];
                let scale = 2f32.powi(s[3] as i8 as i32);
                let target = ((y + dy) * SIZE + x + dx) * 4;
                // Linear ColorRGBExp32. Integer HDR lightmaps (retail materialsystem 10062ae0:
                // min(65535, L * 4096) in 16 bits) cap baked light at 16.
                for (i, &channel) in s.iter().take(3).enumerate() {
                    atlas.rgba[target + i] =
                        half::f16::from_f32((channel as f32 * scale / 255.).min(MAX_LIGHT))
                            .to_bits();
                }
                atlas.rgba[target + 3] = ONE;
            }
        }
        // Duplicate edge texels to prevent filtering from leaking into neighboring faces.
        for dy in 0..height {
            for (source, dest) in [(x, x - 1), (x + width - 1, x + width)] {
                let color: [u16; 4] =
                    atlas.rgba[((y + dy) * SIZE + source) * 4..][..4].try_into()?;
                atlas.rgba[((y + dy) * SIZE + dest) * 4..][..4].copy_from_slice(&color);
            }
        }
        for dx in 0..width + 2 {
            for (source, dest) in [(y, y - 1), (y + height - 1, y + height)] {
                let color: [u16; 4] =
                    atlas.rgba[(source * SIZE + x - 1 + dx) * 4..][..4].try_into()?;
                atlas.rgba[(dest * SIZE + x - 1 + dx) * 4..][..4].copy_from_slice(&color);
            }
        }
        refs.push(Some(FaceLight {
            page,
            x,
            y,
            width,
            height,
        }));
        x += width + 2;
        row = row.max(height);
    }
    Ok((pages, refs))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_ldr_frame_reads_the_ldr_copies_when_the_map_has_them() {
        let mut lumps = vec![vec![]; 64];
        assert!(
            choose_hdr(&lumps, false),
            "no LDR faces: HDR is all there is"
        );
        lumps[7] = vec![0; 56];
        lumps[8] = vec![0; 4];
        lumps[53] = vec![0; 4];
        lumps[58] = vec![0; 56];
        assert!(choose_hdr(&lumps, true));
        assert!(!choose_hdr(&lumps, false));
    }

    #[test]
    fn rgbexp_and_border() {
        let mut lumps = vec![vec![]; 64];
        let mut face = vec![0u8; 56];
        face[36..40].copy_from_slice(&1i32.to_le_bytes());
        lumps[7] = face;
        // Linear 2.0 (255 * 2^1) and 1.0 are both kept; the border copies the edge texel.
        lumps[8] = vec![255, 0, 0, 1, 0, 255, 0, 0];
        let (pages, refs) = build(&lumps, true).unwrap();
        let r = refs[0].unwrap();
        let p = &pages[r.page];
        let texel = |x: usize| -> [f32; 4] {
            std::array::from_fn(|i| half::f16::from_bits(p.rgba[(r.y * SIZE + x) * 4 + i]).to_f32())
        };
        assert_eq!(texel(r.x), [2., 0., 0., 1.]);
        assert_eq!(texel(r.x - 1), [2., 0., 0., 1.]);
        assert_eq!(texel(r.x + 1), [0., 1., 0., 1.]);
    }
}
