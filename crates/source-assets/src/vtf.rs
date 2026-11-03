//! VTF 7.0-7.5 texture reader; RGBA/BGRA/RGB/BGR/BGRX and DXT1/3/5, plus cubemap faces
//! (including RGBA16161616F HDR cubemaps).
use crate::{bytes, u16le, u32le};
use anyhow::{bail, Context, Result};
#[derive(Clone, Debug)]
pub struct Image {
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
    pub format: u32,
    /// TEXTUREFLAGS_ONEBITALPHA | EIGHTBITALPHA: the material system's IsTranslucent(), which
    /// LightmappedGeneric uses to keep $selfillum and $basealphaenvmapmask.
    pub translucent: bool,
}
fn size(format: u32, w: usize, h: usize) -> Result<usize> {
    Ok(match format {
        0 | 1 | 12 | 16 => w * h * 4,
        2 | 3 => w * h * 3,
        4 => w * h * 2,
        13 | 20 => w.div_ceil(4) * h.div_ceil(4) * 8,
        14 | 15 => w.div_ceil(4) * h.div_ceil(4) * 16,
        24 | 25 => w * h * 8,
        _ => bail!("unsupported VTF image format {format}"),
    })
}
fn rgb565(v: u16) -> [u8; 4] {
    let r = ((v >> 11) & 31) as u32;
    let g = ((v >> 5) & 63) as u32;
    let b = (v & 31) as u32;
    [
        (r * 255 / 31) as u8,
        (g * 255 / 63) as u8,
        (b * 255 / 31) as u8,
        255,
    ]
}
fn color_block(block: &[u8], force_four: bool) -> Result<[[u8; 4]; 16]> {
    let c0 = u16le(block, 0)?;
    let c1 = u16le(block, 2)?;
    let mut colors = [rgb565(c0), rgb565(c1), [0; 4], [0; 4]];
    let a = colors[0];
    let b = colors[1];
    if c0 > c1 || force_four {
        for (i, (&x, &y)) in a[..3].iter().zip(&b[..3]).enumerate() {
            colors[2][i] = ((2 * x as u16 + y as u16) / 3) as u8;
            colors[3][i] = ((x as u16 + 2 * y as u16) / 3) as u8;
        }
        colors[2][3] = 255;
        colors[3][3] = 255;
    } else {
        for (i, (&x, &y)) in a[..3].iter().zip(&b[..3]).enumerate() {
            colors[2][i] = ((x as u16 + y as u16) / 2) as u8;
        }
        colors[2][3] = 255;
    }
    let bits = u32le(block, 4)?;
    let mut pixels = [[0; 4]; 16];
    for i in 0..16 {
        pixels[i] = colors[((bits >> (2 * i)) & 3) as usize];
    }
    Ok(pixels)
}
struct Layout {
    format: u32,
    flags: u32,
    width: usize,
    height: usize,
    /// Start of the chosen mip (frame 0, face 0).
    offset: usize,
    faces: usize,
    frames: usize,
    /// Authored mips stored below the chosen one.
    smaller_mips: usize,
}

pub const TEXTUREFLAGS_IGNORE_PICMIP: u32 = 0x0020_0000;

fn layout_picmip(data: &[u8], max_dimension: usize, picmip: i32) -> Result<Layout> {
    layout_level_picmip(data, max_dimension, picmip, 0)
}
/// The mip `below` levels under the largest mip that fits `max_dimension` and `picmip`.
fn layout_level_picmip(
    data: &[u8],
    max_dimension: usize,
    picmip: i32,
    below: usize,
) -> Result<Layout> {
    if bytes(data, 0, 4)? != b"VTF\0" {
        bail!("not a VTF texture");
    }
    let major = u32le(data, 4)?;
    let minor = u32le(data, 8)?;
    if major != 7 || minor > 5 {
        bail!("unsupported VTF {major}.{minor}");
    }
    let header = u32le(data, 12)? as usize;
    bytes(data, 0, header)?;
    let full_w = u16le(data, 16)? as usize;
    let full_h = u16le(data, 18)? as usize;
    if full_w == 0 || full_h == 0 || full_w > 16384 || full_h > 16384 {
        bail!("invalid VTF dimensions");
    }
    let flags = u32le(data, 20)?;
    let frames = u16le(data, 24)? as usize;
    if frames == 0 || frames > 4096 {
        bail!("invalid VTF frame count");
    }
    let format = u32le(data, 52)?;
    let mip_count = *bytes(data, 56, 1)?.first().unwrap() as usize;
    if mip_count == 0 || mip_count > 15 {
        bail!("invalid VTF mip count");
    }
    let depth = if minor >= 2 {
        u16le(data, 63)? as usize
    } else {
        1
    };
    if depth != 1 {
        bail!("volume VTF is unsupported");
    }
    let faces = if flags & 0x4000 != 0 {
        if minor < 5 && u16le(data, 26)? != 0xffff {
            7
        } else {
            6
        }
    } else {
        1
    };
    let mut image_offset = None;
    if minor >= 3 {
        let resources = u32le(data, 68)? as usize;
        if resources > 32 {
            bail!("invalid VTF resource count");
        }
        for i in 0..resources {
            let o = 80 + i * 8;
            let id = bytes(data, o, 4)?;
            if id[..3] == [0x30, 0, 0] {
                if id[3] & 2 != 0 {
                    bail!("inline image resource unsupported");
                }
                image_offset = Some(u32le(data, o + 4)? as usize);
            }
        }
    }
    let mut offset = if let Some(o) = image_offset {
        o
    } else {
        let lowformat = u32le(data, 57)?;
        let w = bytes(data, 61, 1)?[0] as usize;
        let h = bytes(data, 62, 1)?[0] as usize;
        header
            + if w == 0 || h == 0 {
                0
            } else {
                size(lowformat, w, h)?
            }
    };
    let mut mip = 0;
    while mip + 1 < mip_count && (full_w >> mip).max(full_h >> mip) > max_dimension.max(1) {
        mip += 1;
    }
    if flags & TEXTUREFLAGS_IGNORE_PICMIP == 0 && picmip > 0 {
        let skip = (picmip as usize).min(mip_count.saturating_sub(1).saturating_sub(mip));
        mip += skip;
    }
    mip = mip
        .checked_add(below)
        .filter(|&mip| mip < mip_count)
        .context("VTF mip outside its chain")?;
    for level in ((mip + 1)..mip_count).rev() {
        let w = (full_w >> level).max(1);
        let h = (full_h >> level).max(1);
        offset = offset
            .checked_add(
                size(format, w, h)?
                    .checked_mul(frames * faces)
                    .context("VTF mip size overflow")?,
            )
            .context("VTF offset overflow")?;
    }
    Ok(Layout {
        format,
        flags,
        width: (full_w >> mip).max(1),
        height: (full_h >> mip).max(1),
        offset,
        faces,
        frames,
        smaller_mips: mip_count - 1 - mip,
    })
}
pub fn decode(data: &[u8], max_dimension: usize) -> Result<Image> {
    decode_picmip(data, max_dimension, 0)
}
pub fn decode_picmip(data: &[u8], max_dimension: usize, picmip: i32) -> Result<Image> {
    let l = layout_picmip(data, max_dimension, picmip)?;
    decode_face(data, &l, 0)
}
/// All frames of a 2D texture at one mip, checked against the caller's decoded budget
/// before allocating. VTF storage orders each mip by frame, then cube face.
pub fn decode_frames(data: &[u8], max_dimension: usize, byte_budget: usize) -> Result<Vec<Image>> {
    decode_frames_picmip(data, max_dimension, 0, byte_budget)
}
pub fn decode_frames_picmip(
    data: &[u8],
    max_dimension: usize,
    picmip: i32,
    byte_budget: usize,
) -> Result<Vec<Image>> {
    let l = layout_picmip(data, max_dimension, picmip)?;
    if l.faces != 1 {
        bail!("animated 2D texture cannot be a cubemap");
    }
    let decoded = l
        .width
        .checked_mul(l.height)
        .and_then(|n| n.checked_mul(4))
        .and_then(|n| n.checked_mul(l.frames))
        .context("VTF animation size overflow")?;
    if decoded > byte_budget {
        bail!("VTF animation exceeds decoded byte budget");
    }
    // Validate the entire chosen mip before any frame allocations.
    bytes(
        data,
        l.offset,
        size(l.format, l.width, l.height)? * l.frames,
    )?;
    (0..l.frames)
        .map(|frame| decode_face(data, &l, frame))
        .collect()
}
/// Every frame's authored mip chain, from the largest mip that fits `max_dimension` and `picmip` down to
/// the smallest stored mip. Source samples these hand-made or tool-made levels with distance,
/// so they are uploaded as they are rather than regenerated.
pub fn decode_frame_mips(
    data: &[u8],
    max_dimension: usize,
    byte_budget: usize,
) -> Result<Vec<Vec<Image>>> {
    decode_frame_mips_picmip(data, max_dimension, 0, byte_budget)
}
pub fn decode_frame_mips_picmip(
    data: &[u8],
    max_dimension: usize,
    picmip: i32,
    byte_budget: usize,
) -> Result<Vec<Vec<Image>>> {
    let top = layout_picmip(data, max_dimension, picmip)?;
    if top.faces != 1 {
        bail!("animated 2D texture cannot be a cubemap");
    }
    let levels = (0..=top.smaller_mips)
        .map(|below| layout_level_picmip(data, max_dimension, picmip, below))
        .collect::<Result<Vec<_>>>()?;
    let mut decoded = 0usize;
    for l in &levels {
        decoded = l
            .width
            .checked_mul(l.height)
            .and_then(|n| n.checked_mul(4 * l.frames))
            .and_then(|n| n.checked_add(decoded))
            .context("VTF mip chain size overflow")?;
        // Validate every level before any frame allocations.
        bytes(
            data,
            l.offset,
            size(l.format, l.width, l.height)? * l.frames,
        )?;
    }
    if decoded > byte_budget {
        bail!("VTF mip chain exceeds decoded byte budget");
    }
    (0..top.frames)
        .map(|frame| levels.iter().map(|l| decode_face(data, l, frame)).collect())
        .collect()
}
/// Image data of a cubemap: six faces in Source order +X, -X, +Y, -Y, +Z, -Z (the same
/// layer order and orientation as a D3D/wgpu cube texture). The 7.0-7.4 spheremap face is
/// skipped.
#[derive(Debug)]
pub struct Cube {
    pub size: u16,
    /// RGBA16161616F: each face holds little-endian half floats (8 bytes per texel), linear.
    /// Otherwise RGBA8 texels as stored (sRGB-encoded for LDR envmaps).
    pub hdr: bool,
    pub faces: [Vec<u8>; 6],
}
pub fn decode_cube(data: &[u8], max_dimension: usize) -> Result<Cube> {
    decode_cube_picmip(data, max_dimension, 0)
}
pub fn decode_cube_picmip(data: &[u8], max_dimension: usize, picmip: i32) -> Result<Cube> {
    let l = layout_picmip(data, max_dimension, picmip)?;
    if l.faces < 6 || l.width != l.height {
        bail!("VTF is not a square cubemap");
    }
    let hdr = l.format == 24;
    let faces = (0..6)
        .map(|face| {
            if hdr {
                let n = size(24, l.width, l.height)?;
                Ok(bytes(data, l.offset + face * n, n)?.to_vec())
            } else {
                Ok(decode_face(data, &l, face)?.rgba)
            }
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Cube {
        size: u16::try_from(l.width)?,
        hdr,
        faces: faces.try_into().expect("six faces"),
    })
}
fn decode_face(data: &[u8], l: &Layout, face: usize) -> Result<Image> {
    let (format, w, h) = (l.format, l.width, l.height);
    let offset = l.offset + face * size(format, w, h)?;
    let image = bytes(data, offset, size(format, w, h)?)?;
    let mut rgba = vec![0; w * h * 4];
    match format {
        0 | 1 | 12 | 16 | 2 | 3 | 4 => {
            let stride = match format {
                2 | 3 => 3,
                4 => 2,
                _ => 4,
            };
            for i in 0..w * h {
                let p = &image[i * stride..(i + 1) * stride];
                let v = match format {
                    0 => [p[0], p[1], p[2], p[3]],
                    1 => [p[3], p[2], p[1], p[0]],
                    12 => [p[2], p[1], p[0], p[3]],
                    16 => [p[2], p[1], p[0], 255],
                    2 => [p[0], p[1], p[2], 255],
                    3 => [p[2], p[1], p[0], 255],
                    4 => rgb565(u16::from_le_bytes([p[0], p[1]])),
                    _ => unreachable!(),
                };
                rgba[i * 4..i * 4 + 4].copy_from_slice(&v);
            }
        }
        13 | 20 | 14 | 15 => {
            let blocksize = if format == 13 || format == 20 { 8 } else { 16 };
            for by in 0..h.div_ceil(4) {
                for bx in 0..w.div_ceil(4) {
                    let b = &image[(by * w.div_ceil(4) + bx) * blocksize..][..blocksize];
                    let mut pixels =
                        color_block(if blocksize == 8 { b } else { &b[8..] }, blocksize == 16)?;
                    if format == 14 {
                        for i in 0..16 {
                            pixels[i][3] = ((b[i / 2] >> (4 * (i % 2))) & 15) * 17;
                        }
                    }
                    if format == 15 {
                        let mut a = [0u8; 8];
                        a[0] = b[0];
                        a[1] = b[1];
                        if a[0] > a[1] {
                            for i in 2..8 {
                                a[i] =
                                    (((8 - i) * a[0] as usize + (i - 1) * a[1] as usize) / 7) as u8;
                            }
                        } else {
                            for i in 2..6 {
                                a[i] =
                                    (((6 - i) * a[0] as usize + (i - 1) * a[1] as usize) / 5) as u8;
                            }
                            a[6] = 0;
                            a[7] = 255;
                        }
                        let mut bits = 0u64;
                        for i in 0..6 {
                            bits |= (b[i + 2] as u64) << (8 * i);
                        }
                        for i in 0..16 {
                            pixels[i][3] = a[((bits >> (3 * i)) & 7) as usize];
                        }
                    }
                    for dy in 0..4 {
                        for dx in 0..4 {
                            let x = bx * 4 + dx;
                            let y = by * 4 + dy;
                            if x < w && y < h {
                                rgba[(y * w + x) * 4..(y * w + x) * 4 + 4]
                                    .copy_from_slice(&pixels[dy * 4 + dx]);
                            }
                        }
                    }
                }
            }
        }
        _ => bail!("unsupported VTF format {format}"),
    }
    Ok(Image {
        width: w as u16,
        height: h as u16,
        rgba,
        format,
        translucent: l.flags & (0x1000 | 0x2000) != 0,
    })
}
/// One frame of a sheet sequence: its duration and up to four images' texture
/// rectangles (u0, v0, u1, v1).
#[derive(Clone, Debug, PartialEq)]
pub struct SheetFrame {
    pub duration: f32,
    pub images: Vec<[f32; 4]>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct SheetSequence {
    pub number: i32,
    pub clamp: bool,
    pub total_time: f32,
    pub frames: Vec<SheetFrame>,
}
/// The VTF_RSRC_SHEET resource (particle sprite-card sheets). Layout read from the
/// installed sheets (no SDK source: the loader is in the closed particles library):
/// u32 size, then i32 version (1: four images per frame, 0: one), i32 sequence
/// count, and per sequence i32 number, i32 clamp, i32 frame count, f32 total time, then
/// per frame f32 duration and the images' (u0, v0, u1, v1). The whole resource must be
/// consumed. None when the texture has no sheet.
pub fn sheet(data: &[u8]) -> Result<Option<Vec<SheetSequence>>> {
    if bytes(data, 0, 4)? != b"VTF\0" || u32le(data, 8)? < 3 {
        return Ok(None);
    }
    let resources = u32le(data, 68)? as usize;
    if resources > 32 {
        bail!("invalid VTF resource count");
    }
    let Some(offset) = (0..resources).find_map(|i| {
        let o = 80 + i * 8;
        (bytes(data, o, 4).ok()? == [0x10, 0, 0, 0]).then(|| u32le(data, o + 4).ok())?
    }) else {
        return Ok(None);
    };
    let offset = offset as usize;
    let size = u32le(data, offset)? as usize;
    let end = offset + 4 + size;
    let mut o = offset + 4;
    let int = |o: &mut usize| -> Result<i32> {
        let v = crate::i32le(data, *o)?;
        *o += 4;
        Ok(v)
    };
    let version = int(&mut o)?;
    let images = if version == 0 { 1 } else { 4 };
    let count = int(&mut o)?;
    if !(0..=1024).contains(&count) {
        bail!("invalid sheet sequence count {count}");
    }
    let float = |o: &mut usize| -> Result<f32> {
        let v = crate::f32le(data, *o)?;
        *o += 4;
        Ok(v)
    };
    let mut sequences = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let number = int(&mut o)?;
        let clamp = int(&mut o)? != 0;
        let frames = int(&mut o)?;
        if !(0..=4096).contains(&frames) {
            bail!("invalid sheet frame count {frames}");
        }
        let total_time = float(&mut o)?;
        let mut list = Vec::with_capacity(frames as usize);
        for _ in 0..frames {
            let duration = float(&mut o)?;
            let mut rects = Vec::with_capacity(images);
            for _ in 0..images {
                rects.push([
                    float(&mut o)?,
                    float(&mut o)?,
                    float(&mut o)?,
                    float(&mut o)?,
                ]);
            }
            list.push(SheetFrame {
                duration,
                images: rects,
            });
        }
        sequences.push(SheetSequence {
            number,
            clamp,
            total_time,
            frames: list,
        });
    }
    if o != end {
        bail!(
            "sheet resource has {} unread bytes",
            end as isize - o as isize
        );
    }
    Ok(Some(sequences))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "reads the installed game (HL2_ROOT)"]
    fn installed_particle_sheets_parse_exactly() {
        let game = crate::install::discover().unwrap();
        let vfs = crate::vpk::Vfs::mount(&game).unwrap();
        for (path, sequences) in [
            (
                "materials/particle/fire_burning_character/fire_burning_character.vtf",
                6,
            ),
            ("materials/particle/vistasmokev1/vistasmokev1.vtf", 34),
        ] {
            let data = vfs.read(path).unwrap().unwrap();
            let sheet = sheet(&data).unwrap().unwrap();
            assert_eq!(sheet.len(), sequences, "{path}");
            assert!(sheet.iter().all(|s| !s.frames.is_empty()
                && s.frames
                    .iter()
                    .all(|f| f.images.len() == 4
                        && f.images[0].iter().all(|c| (0. ..=1.).contains(c)))));
        }
    }
    #[test]
    fn dxt1_red_block() {
        let b = [0, 248, 0, 0, 0, 0, 0, 0];
        let p = color_block(&b, false).unwrap();
        assert_eq!(p[0], [255, 0, 0, 255]);
    }
    #[test]
    fn dxt1_transparency() {
        let b = [0, 0, 0, 248, 255, 255, 255, 255];
        let p = color_block(&b, false).unwrap();
        assert_eq!(p[0][3], 0);
    }
    /// A 7.1 RGBA8888 header with the given size, frames and mips; no low-res image.
    fn header(w: u16, h: u16, frames: u16, mips: u8) -> Vec<u8> {
        let mut d = vec![0; 64];
        d[..4].copy_from_slice(b"VTF\0");
        d[4..8].copy_from_slice(&7u32.to_le_bytes());
        d[8..12].copy_from_slice(&1u32.to_le_bytes());
        d[12..16].copy_from_slice(&64u32.to_le_bytes());
        d[16..18].copy_from_slice(&w.to_le_bytes());
        d[18..20].copy_from_slice(&h.to_le_bytes());
        d[24..26].copy_from_slice(&frames.to_le_bytes());
        d[56] = mips;
        d
    }
    #[test]
    fn frame_mips_follow_storage_order() {
        // Smallest mip first; within a mip, frame by frame. Texel value = mip * 16 + frame.
        let mut d = header(4, 2, 2, 3);
        for (mip, (w, h)) in [(2, (1, 1)), (1, (2, 1)), (0, (4, 2))] {
            for frame in 0..2u8 {
                for _ in 0..w * h {
                    d.extend_from_slice(&[mip * 16 + frame, 0, 0, 255]);
                }
            }
        }
        let frames = decode_frame_mips(&d, 512, 1 << 20).unwrap();
        assert_eq!(frames.len(), 2);
        for (frame, chain) in frames.iter().enumerate() {
            let sizes: Vec<_> = chain.iter().map(|i| (i.width, i.height)).collect();
            assert_eq!(sizes, [(4, 2), (2, 1), (1, 1)]);
            for (mip, image) in chain.iter().enumerate() {
                assert!(image
                    .rgba
                    .chunks(4)
                    .all(|p| p[0] as usize == mip * 16 + frame));
            }
        }
        // The dimension cap drops the top mip; the budget covers the whole chain.
        let capped = decode_frame_mips(&d, 2, 1 << 20).unwrap();
        assert_eq!(capped[1].len(), 2);
        assert_eq!(capped[1][0].rgba[0], 16 + 1);
        assert!(decode_frame_mips(&d, 512, 4 * (8 + 2 + 1) * 2 - 1).is_err());
        // Truncated storage is rejected before allocation.
        assert!(decode_frame_mips(&d[..d.len() - 1], 512, 1 << 20).is_err());
    }
    #[test]
    fn picmip_skips_mips_unless_ignore_picmip_is_set() {
        // 4x2 texture, 3 mips (4x2, 2x1, 1x1).
        let mut d = header(4, 2, 1, 3);
        for (_mip, (w, h)) in [(2, (1, 1)), (1, (2, 1)), (0, (4, 2))] {
            for _ in 0..w * h {
                d.extend_from_slice(&[255, 0, 0, 255]);
            }
        }
        // picmip 0 -> full resolution (4, 2)
        let img0 = decode_picmip(&d, 512, 0).unwrap();
        assert_eq!((img0.width, img0.height), (4, 2));

        // picmip 1 -> skips 1 mip -> (2, 1)
        let img1 = decode_picmip(&d, 512, 1).unwrap();
        assert_eq!((img1.width, img1.height), (2, 1));

        // picmip 2 -> skips 2 mips -> (1, 1)
        let img2 = decode_picmip(&d, 512, 2).unwrap();
        assert_eq!((img2.width, img2.height), (1, 1));

        // Now set TEXTUREFLAGS_IGNORE_PICMIP (0x0020_0000) at flags offset 20..24
        let mut d_ignored = d.clone();
        d_ignored[20..24].copy_from_slice(&TEXTUREFLAGS_IGNORE_PICMIP.to_le_bytes());

        // picmip 2 on texture with IGNORE_PICMIP should remain at full (4, 2)
        let img_ignored = decode_picmip(&d_ignored, 512, 2).unwrap();
        assert_eq!((img_ignored.width, img_ignored.height), (4, 2));
    }
    #[test]
    fn truncated_vtf_is_error() {
        assert!(decode(b"VTF\0", 512).is_err());
    }
    #[test]
    fn rgba_fixture() {
        let mut d = vec![0; 68];
        d[..4].copy_from_slice(b"VTF\0");
        d[4..8].copy_from_slice(&7u32.to_le_bytes());
        d[8..12].copy_from_slice(&1u32.to_le_bytes());
        d[12..16].copy_from_slice(&64u32.to_le_bytes());
        d[16..18].copy_from_slice(&1u16.to_le_bytes());
        d[18..20].copy_from_slice(&1u16.to_le_bytes());
        d[24..26].copy_from_slice(&1u16.to_le_bytes());
        d[56] = 1;
        d[64..68].copy_from_slice(&[11, 22, 33, 255]);
        assert_eq!(decode(&d, 512).unwrap().rgba, vec![11, 22, 33, 255]);
    }
    fn cube_fixture(format: u32, texel: &[u8], faces: usize) -> Vec<u8> {
        let mut d = vec![0; 80];
        d[..4].copy_from_slice(b"VTF\0");
        d[4..8].copy_from_slice(&7u32.to_le_bytes());
        d[8..12].copy_from_slice(&4u32.to_le_bytes());
        d[12..16].copy_from_slice(&80u32.to_le_bytes());
        d[16..18].copy_from_slice(&1u16.to_le_bytes());
        d[18..20].copy_from_slice(&1u16.to_le_bytes());
        d[20..24].copy_from_slice(&0x4000u32.to_le_bytes());
        d[24..26].copy_from_slice(&1u16.to_le_bytes());
        // First frame 0 (not 0xffff): 7.4 cubemaps carry a seventh spheremap face.
        d[52..56].copy_from_slice(&format.to_le_bytes());
        d[56] = 1;
        d[63..65].copy_from_slice(&1u16.to_le_bytes());
        for face in 0..faces {
            d.extend(texel.iter().map(|b| b.wrapping_add(face as u8)));
        }
        d
    }
    #[test]
    fn cube_faces_follow_source_order_and_skip_spheremap() {
        let d = cube_fixture(0, &[10, 20, 30, 255], 7);
        let cube = decode_cube(&d, 512).unwrap();
        assert!(!cube.hdr);
        assert_eq!(cube.size, 1);
        for (face, rgba) in cube.faces.iter().enumerate() {
            assert_eq!(
                rgba,
                &[
                    10 + face as u8,
                    20 + face as u8,
                    30 + face as u8,
                    255u8.wrapping_add(face as u8)
                ]
            );
        }
        assert!(decode_cube(&d[..d.len() - 4], 512).is_ok());
        assert!(decode_cube(&d[..80 + 5 * 4 + 2], 512).is_err());
    }
    #[test]
    fn hdr_cube_keeps_half_floats() {
        // 1.0, 2.0, 0.5, 1.0 as IEEE half floats.
        let texel = [0x00, 0x3c, 0x00, 0x40, 0x00, 0x38, 0x00, 0x3c];
        let cube = decode_cube(&cube_fixture(24, &texel, 7), 512).unwrap();
        assert!(cube.hdr);
        assert_eq!(cube.faces[0], texel);
        assert_eq!(cube.faces[5][0], 5);
        assert!(decode(&cube_fixture(24, &texel, 7), 512).is_err());
        assert!(decode_cube(&rgba_only(), 512).is_err());
    }
    fn rgba_only() -> Vec<u8> {
        let mut d = cube_fixture(0, &[1, 2, 3, 4], 1);
        d[20..24].copy_from_slice(&0u32.to_le_bytes());
        d
    }
    #[test]
    fn animation_frames_follow_mip_frame_order_and_enforce_budget() {
        let mut data = rgba_only();
        data[24..26].copy_from_slice(&3u16.to_le_bytes());
        data.extend_from_slice(&[5, 6, 7, 8, 9, 10, 11, 12]);
        let frames = decode_frames(&data, 512, 12).unwrap();
        assert_eq!(
            frames.iter().map(|f| f.rgba.clone()).collect::<Vec<_>>(),
            vec![vec![1, 2, 3, 4], vec![5, 6, 7, 8], vec![9, 10, 11, 12]]
        );
        assert!(decode_frames(&data, 512, 11).is_err());
        assert!(decode_frames(&data[..data.len() - 1], 512, 12).is_err());
        assert!(decode_frames(&cube_fixture(0, &[1, 2, 3, 4], 7), 512, 100).is_err());
        // A smaller mip precedes all frames of the chosen large mip.
        data[16..18].copy_from_slice(&2u16.to_le_bytes());
        data[18..20].copy_from_slice(&2u16.to_le_bytes());
        data[56] = 2;
        data.extend_from_slice(&[21; 16]);
        data.extend_from_slice(&[22; 16]);
        data.extend_from_slice(&[23; 16]);
        let large = decode_frames(&data, 2, 48).unwrap();
        assert_eq!(
            large.iter().map(|f| f.rgba[0]).collect::<Vec<_>>(),
            vec![21, 22, 23]
        );
        assert_eq!(
            decode_frames(&data, 1, 12).unwrap()[2].rgba,
            vec![9, 10, 11, 12]
        );
    }
    #[test]
    #[ignore = "requires owned HL2 installation"]
    fn owned_combine_barrier_frames_fade_to_black() {
        let vfs = crate::vpk::Vfs::mount(&crate::install::discover().unwrap()).unwrap();
        for name in ["comshieldwall", "comshieldwall_close"] {
            let data = vfs
                .read(&format!("materials/effects/combineshield/{name}.vtf"))
                .unwrap()
                .unwrap();
            let frames = decode_frames(&data, 2048, 64 * 1024 * 1024).unwrap();
            assert_eq!(frames.len(), if name == "comshieldwall" { 31 } else { 1 });
            let energy = |image: &Image| {
                image
                    .rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| u64::from(c[0]) + u64::from(c[1]) + u64::from(c[2]))
                    .sum::<u64>()
            };
            assert!(energy(&frames[0]) > 0);
            if frames.len() == 31 {
                assert_eq!(energy(&frames[30]), 0);
            }
        }
    }
    #[test]
    #[ignore = "requires owned HL2 installation"]
    fn owned_trainstation_window_cubemaps_decode() {
        let mut vfs = crate::vpk::Vfs::mount(&crate::install::discover().unwrap()).unwrap();
        let bytes = vfs.read("maps/d1_trainstation_02.bsp").unwrap().unwrap();
        let bsp = crate::bsp::Bsp::parse(&bytes).unwrap();
        vfs.mount_pak(bsp.lump(40)).unwrap();
        let name = "materials/maps/d1_trainstation_02/c-3104_-2048_532";
        let ldr = decode_cube(&vfs.read(&format!("{name}.vtf")).unwrap().unwrap(), 512).unwrap();
        let hdr =
            decode_cube(&vfs.read(&format!("{name}.hdr.vtf")).unwrap().unwrap(), 512).unwrap();
        assert!(!ldr.hdr && hdr.hdr);
        assert_eq!((ldr.size, hdr.size), (32, 32));
        for face in 0..6 {
            assert_eq!(ldr.faces[face].len(), 32 * 32 * 4);
            assert_eq!(hdr.faces[face].len(), 32 * 32 * 8);
        }
        // Linear HDR texels are finite and within the integer-HDR range.
        // Alpha is unused (it holds arbitrary halves, including NaN, in owned files).
        let max = hdr
            .faces
            .iter()
            .flat_map(|f| f.as_chunks::<8>().0)
            .flat_map(|t| t[..6].as_chunks::<2>().0)
            .map(|h| half(u16::from_le_bytes([h[0], h[1]])))
            .fold(0f32, f32::max);
        assert!(max.is_finite() && max > 0.05 && max < 64., "{max}");
        eprintln!("hdr max {max}");
    }
    fn half(bits: u16) -> f32 {
        let sign = if bits & 0x8000 != 0 { -1. } else { 1. };
        let exponent = i32::from((bits >> 10) & 31);
        let mantissa = f32::from(bits & 1023);
        sign * if exponent == 0 {
            mantissa * 2f32.powi(-24)
        } else {
            (1. + mantissa / 1024.) * 2f32.powi(exponent - 15)
        }
    }
}
