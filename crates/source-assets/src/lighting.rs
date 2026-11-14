//! Packs each face's primary RGBExp32 baked light samples into GPU atlas pages; faces with
//! switchable light styles keep every style's samples for [`compose_styles`].
use crate::{bytes, i16le, i32le, records};
use anyhow::{bail, Result};
use modkit_core::{Lightmap, StyledLightmap};
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
        border(&mut atlas.rgba, SIZE, (x, y, width, height));
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
/// Duplicates a face rectangle's edge texels into its one-texel border so filtering does not
/// leak into neighboring faces.
fn border(rgba: &mut [u16], size: usize, (x, y, width, height): (usize, usize, usize, usize)) {
    for dy in 0..height {
        for (source, dest) in [(x, x - 1), (x + width - 1, x + width)] {
            let row = (y + dy) * size;
            rgba.copy_within((row + source) * 4..(row + source) * 4 + 4, (row + dest) * 4);
        }
    }
    for dx in 0..width + 2 {
        for (source, dest) in [(y, y - 1), (y + height - 1, y + height)] {
            let from = (source * size + x - 1 + dx) * 4;
            rgba.copy_within(from..from + 4, (dest * size + x - 1 + dx) * 4);
        }
    }
}

/// Linear light of one ColorRGBExp32 sample, capped like the baked atlas.
fn decode(sample: &[u8; 4]) -> [f32; 3] {
    let scale = 2f32.powi(sample[3] as i8 as i32);
    [0, 1, 2].map(|i| sample[i] as f32 * scale / 255.)
}

/// Faces lit by a style other than 0 with every style's samples (`refs` from [`build`]).
pub fn styled_faces(
    lumps: &[Vec<u8>],
    hdr: bool,
    refs: &[Option<FaceLight>],
) -> Result<Vec<StyledLightmap>> {
    let hdr = choose_hdr(lumps, hdr);
    let data = &lumps[if hdr { 53 } else { 8 }];
    let texinfo: Vec<&[u8]> = records(&lumps[6], 72)?.collect();
    let mut faces = Vec::new();
    for (face, light) in records(&lumps[if hdr { 58 } else { 7 }], 56)?.zip(refs) {
        let (Some(light), styles) = (light, bytes(face, 16, 4)?) else {
            continue;
        };
        if styles.iter().all(|&s| s == 0 || s == 255) {
            continue;
        }
        // SURF_BUMPLIGHT faces store four lightmaps a style.
        let bumped = usize::try_from(i16le(face, 10)?)
            .ok()
            .and_then(|i| texinfo.get(i))
            .map(|t| i32le(t, 64))
            .transpose()?
            .is_some_and(|flags| flags & 0x800 != 0);
        let texels = light.width * light.height;
        let block = texels * 4 * if bumped { 4 } else { 1 };
        let offset = usize::try_from(i32le(face, 20)?)?;
        let mut layers = Vec::new();
        for (k, &style) in styles.iter().take_while(|&&s| s != 255).enumerate() {
            let samples = bytes(data, offset + k * block, texels * 4)?;
            layers.push((
                style,
                samples.as_chunks::<4>().0.iter().map(decode).collect(),
            ));
        }
        faces.push(StyledLightmap {
            page: light.page,
            x: light.x,
            y: light.y,
            width: light.width,
            height: light.height,
            styles: layers,
        });
    }
    Ok(faces)
}

/// Recomposes page `page`'s styled faces: each texel sums its styles' samples times the
/// style's value (`value`; the engine's lightstyle scale, 1 for 'm').
pub fn compose_styles(
    lightmap: &mut Lightmap,
    page: usize,
    faces: &[StyledLightmap],
    value: impl Fn(u8) -> f32,
) {
    let size = usize::from(lightmap.width);
    for face in faces.iter().filter(|f| f.page == page) {
        let values: Vec<f32> = face.styles.iter().map(|(s, _)| value(*s)).collect();
        for dy in 0..face.height {
            for dx in 0..face.width {
                let i = dy * face.width + dx;
                let mut rgb = [0f32; 3];
                for ((_, samples), v) in face.styles.iter().zip(&values) {
                    for (c, sum) in rgb.iter_mut().enumerate() {
                        *sum += samples[i][c] * v;
                    }
                }
                let target = ((face.y + dy) * size + face.x + dx) * 4;
                for (c, light) in rgb.iter().enumerate() {
                    lightmap.rgba[target + c] = half::f16::from_f32(light.min(MAX_LIGHT)).to_bits();
                }
            }
        }
        border(
            &mut lightmap.rgba,
            size,
            (face.x, face.y, face.width, face.height),
        );
    }
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
