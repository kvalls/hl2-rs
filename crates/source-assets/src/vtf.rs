//! VTF 7.0-7.5 texture reader; RGBA/BGRA/RGB/BGR/BGRX and DXT1/3/5, plus cubemap faces
//! (including RGBA16161616F HDR cubemaps).
use crate::{bytes, u16le, u32le};
use anyhow::{bail, Context, Result};
#[derive(Debug)]
pub struct Image {
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
    pub format: u32,
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
    width: usize,
    height: usize,
    /// Start of the chosen mip (frame 0, face 0).
    offset: usize,
    faces: usize,
}
fn layout(data: &[u8], max_dimension: usize) -> Result<Layout> {
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
        width: (full_w >> mip).max(1),
        height: (full_h >> mip).max(1),
        offset,
        faces,
    })
}
pub fn decode(data: &[u8], max_dimension: usize) -> Result<Image> {
    let l = layout(data, max_dimension)?;
    decode_face(data, &l, 0)
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
    let l = layout(data, max_dimension)?;
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
    })
}
#[cfg(test)]
mod tests {
    use super::*;
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
    #[ignore = "requires owned HL2 installation"]
    fn owned_trainstation_window_cubemaps_decode() {
        let mut vfs = crate::vpk::Vfs::mount(std::path::Path::new(
            &std::env::var("HL2_ROOT").expect("set HL2_ROOT"),
        ))
        .unwrap();
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
