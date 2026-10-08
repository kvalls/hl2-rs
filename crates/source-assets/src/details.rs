//! Detail props ('dprp' game lump v4, SDK gamebspfile.h DetailObjectLump_t) and their
//! supplemental lightstyle tables ('dplt' LDR / 'dplh' HDR, 5-byte records).
use crate::{bytes, f32le, u16le, u32le};
use anyhow::{bail, ensure, Context, Result};
use glam::{Vec2, Vec3};
use modkit_core::{DetailInstance, DetailProps, DetailSprite, DetailStyle};

const DPRP: u32 = 0x6470_7270;
const DPLT: u32 = 0x6470_6c74;
const DPLH: u32 = 0x6470_6c68;
const MAX_RECORDS: usize = 1 << 20;

/// Reads the detail lumps from the game-lump directory (lump 35). Offsets are absolute in
/// the original BSP bytes. A map without detail lumps yields an empty table.
pub fn read(bsp: &[u8], directory: &[u8]) -> Result<DetailProps> {
    let mut details = DetailProps::default();
    if directory.is_empty() {
        return Ok(details);
    }
    let count = u32le(directory, 0)? as usize;
    bytes(
        directory,
        4,
        count
            .checked_mul(16)
            .context("game lump directory overflow")?,
    )?;
    for n in 0..count {
        let header = &directory[4 + n * 16..4 + (n + 1) * 16];
        let id = u32le(header, 0)?;
        if !matches!(id, DPRP | DPLT | DPLH) {
            continue;
        }
        if u16le(header, 4)? & 1 != 0 {
            bail!("compressed detail game lump is unsupported");
        }
        let version = u16le(header, 6)?;
        let data = bytes(bsp, u32le(header, 8)? as usize, u32le(header, 12)? as usize)?;
        match id {
            DPRP => {
                ensure!(
                    version == 4,
                    "unsupported detail prop lump version {version}"
                );
                read_props(data, &mut details)?;
            }
            DPLT => details.styles_ldr = read_styles(data, version)?,
            _ => details.styles_hdr = read_styles(data, version)?,
        }
    }
    for (i, d) in details.instances.iter().enumerate() {
        let entries = if d.kind == 0 {
            details.models.len()
        } else {
            details.sprites.len()
        };
        ensure!(
            usize::from(d.index) < entries,
            "detail {i} references dictionary entry {} of {entries}",
            d.index
        );
        let end = d.style_first as usize + usize::from(d.style_count);
        ensure!(
            d.style_count == 0 || end <= details.styles_ldr.len().max(details.styles_hdr.len()),
            "detail {i} lightstyles {}..{end} out of range",
            d.style_first
        );
    }
    Ok(details)
}

fn count(data: &[u8], offset: usize, size: usize) -> Result<(usize, &[u8])> {
    let n = u32le(data, offset)?;
    let n = usize::try_from(n)
        .ok()
        .filter(|&n| n <= MAX_RECORDS)
        .context("detail count")?;
    let block = bytes(
        data,
        offset + 4,
        n.checked_mul(size).context("detail size overflow")?,
    )?;
    Ok((n, block))
}
fn finite(v: Vec3) -> Result<Vec3> {
    ensure!(v.is_finite(), "non-finite detail vector");
    Ok(v)
}
fn vec2(d: &[u8], o: usize) -> Result<Vec2> {
    let v = Vec2::new(f32le(d, o)?, f32le(d, o + 4)?);
    ensure!(v.is_finite(), "non-finite detail sprite coordinate");
    Ok(v)
}

fn read_props(data: &[u8], details: &mut DetailProps) -> Result<()> {
    let (models, block) = count(data, 0, 128)?;
    for name in block.as_chunks::<128>().0 {
        let end = name
            .iter()
            .position(|&b| b == 0)
            .context("unterminated detail model name")?;
        details
            .models
            .push(String::from_utf8_lossy(&name[..end]).replace('\\', "/"));
    }
    let mut offset = 4 + models * 128;
    let (_, block) = count(data, offset, 32)?;
    for s in block.as_chunks::<32>().0 {
        details.sprites.push(DetailSprite {
            ul: vec2(s, 0)?,
            lr: vec2(s, 8)?,
            tex_ul: vec2(s, 16)?,
            tex_lr: vec2(s, 24)?,
        });
    }
    offset += 4 + block.len();
    let (_, block) = count(data, offset, 52)?;
    for r in block.as_chunks::<52>().0 {
        let scale = f32le(r, 48)?;
        ensure!(scale.is_finite(), "non-finite detail scale");
        details.instances.push(DetailInstance {
            origin: finite(crate::vec3(r, 0)?)?,
            angles: finite(crate::vec3(r, 12)?)?,
            index: u16le(r, 24)?,
            leaf: u16le(r, 26)?,
            lighting: [r[28], r[29], r[30], r[31]],
            style_first: u32le(r, 32)?,
            style_count: r[36],
            sway: r[37],
            shape_angle: r[38],
            shape_size: r[39],
            orientation: r[40],
            kind: r[44],
            scale,
        });
    }
    offset += 4 + block.len();
    ensure!(
        offset == data.len(),
        "detail prop lump has {} trailing bytes",
        data.len().saturating_sub(offset)
    );
    Ok(())
}

fn read_styles(data: &[u8], version: u16) -> Result<Vec<DetailStyle>> {
    ensure!(
        version == 0,
        "unsupported detail lighting lump version {version}"
    );
    let (_, block) = count(data, 0, 5)?;
    ensure!(
        4 + block.len() == data.len(),
        "detail lighting lump size mismatch"
    );
    Ok(block
        .as_chunks::<5>()
        .0
        .iter()
        .map(|s| DetailStyle {
            lighting: [s[0], s[1], s[2], s[3]],
            style: s[4],
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lump(id: u32, version: u16, payload: &[u8], base: usize) -> (Vec<u8>, Vec<u8>) {
        let mut header = id.to_le_bytes().to_vec();
        header.extend(0u16.to_le_bytes());
        header.extend(version.to_le_bytes());
        header.extend((base as u32).to_le_bytes());
        header.extend((payload.len() as u32).to_le_bytes());
        (header, payload.to_vec())
    }
    fn props(sprites: u32, records: &[(u16, u8, u8)]) -> Vec<u8> {
        let mut p = 0u32.to_le_bytes().to_vec();
        p.extend(sprites.to_le_bytes());
        for i in 0..sprites {
            for v in [-9., 21., 9., 0., 0.25, 0.5, 0.5 + i as f32 * 0.1, 0.75f32] {
                p.extend(v.to_le_bytes());
            }
        }
        p.extend((records.len() as u32).to_le_bytes());
        for &(index, orientation, kind) in records {
            let mut r = vec![0u8; 52];
            r[0..4].copy_from_slice(&10f32.to_le_bytes());
            r[24..26].copy_from_slice(&index.to_le_bytes());
            r[26..28].copy_from_slice(&7u16.to_le_bytes());
            r[28..32].copy_from_slice(&[128, 64, 32, 0xff]);
            r[40] = orientation;
            r[44] = kind;
            r[48..52].copy_from_slice(&0.5f32.to_le_bytes());
            p.extend(r);
        }
        p
    }
    fn bsp(parts: &[(u32, u16, Vec<u8>)]) -> (Vec<u8>, Vec<u8>) {
        let mut file = vec![0u8; 64];
        let mut directory = (parts.len() as u32).to_le_bytes().to_vec();
        for (id, version, payload) in parts {
            let (header, data) = lump(*id, *version, payload, file.len());
            directory.extend(header);
            file.extend(data);
        }
        (file, directory)
    }

    #[test]
    fn reads_sprites_records_and_five_byte_styles() {
        let mut styles = 2u32.to_le_bytes().to_vec();
        styles.extend([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        let (file, dir) = bsp(&[
            (DPRP, 4, props(2, &[(1, 2, 1), (0, 2, 1)])),
            (DPLH, 0, styles),
            (DPLT, 0, 0u32.to_le_bytes().to_vec()),
        ]);
        let d = read(&file, &dir).unwrap();
        assert_eq!(d.sprites.len(), 2);
        assert_eq!(d.sprites[1].tex_lr, Vec2::new(0.6, 0.75));
        assert_eq!(d.instances.len(), 2);
        let i = &d.instances[0];
        assert_eq!((i.index, i.leaf, i.orientation, i.kind), (1, 7, 2, 1));
        assert_eq!(i.lighting, [128, 64, 32, 0xff]);
        assert_eq!(i.origin, Vec3::new(10., 0., 0.));
        assert_eq!(i.scale, 0.5);
        assert_eq!(d.styles_hdr.len(), 2);
        assert_eq!(d.styles_hdr[1].lighting, [6, 7, 8, 9]);
        assert_eq!(d.styles_hdr[1].style, 10);
        assert!(d.styles_ldr.is_empty());
    }

    #[test]
    fn rejects_malformed_detail_lumps() {
        // Dictionary index past the sprite table.
        let (file, dir) = bsp(&[(DPRP, 4, props(1, &[(1, 2, 1)]))]);
        assert!(read(&file, &dir).is_err());
        // Trailing bytes and truncation.
        let mut extra = props(1, &[(0, 2, 1)]);
        extra.push(0);
        let (file, dir) = bsp(&[(DPRP, 4, extra)]);
        assert!(read(&file, &dir).is_err());
        let mut short = props(1, &[(0, 2, 1)]);
        short.truncate(short.len() - 1);
        let (file, dir) = bsp(&[(DPRP, 4, short)]);
        assert!(read(&file, &dir).is_err());
        // Unsupported version, non-finite scale, oversized counts.
        let (file, dir) = bsp(&[(DPRP, 5, props(1, &[(0, 2, 1)]))]);
        assert!(read(&file, &dir).is_err());
        let mut nan = props(1, &[(0, 2, 1)]);
        let at = nan.len() - 4;
        nan[at..].copy_from_slice(&f32::NAN.to_le_bytes());
        let (file, dir) = bsp(&[(DPRP, 4, nan)]);
        assert!(read(&file, &dir).is_err());
        let (file, dir) = bsp(&[(DPLT, 0, u32::MAX.to_le_bytes().to_vec())]);
        assert!(read(&file, &dir).is_err());
        // Missing lumps are an empty table.
        assert!(read(&[], &[]).unwrap().instances.is_empty());
    }

    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn installed_trainstation_02_detail_sprites() {
        let root = crate::install::discover().unwrap();
        let vfs = crate::vpk::Vfs::mount(&root).unwrap();
        let data = vfs.read("maps/d1_trainstation_02.bsp").unwrap().unwrap();
        let bsp = crate::bsp::Bsp::parse(&data).unwrap();
        let d = &bsp.details;
        assert!(d.models.is_empty());
        assert_eq!(d.sprites.len(), 7);
        assert_eq!(d.instances.len(), 3622);
        assert!(d
            .instances
            .iter()
            .all(|i| i.kind == 1 && i.orientation == 2 && i.style_count == 0));
        let leaves: std::collections::BTreeSet<_> = d.instances.iter().map(|i| i.leaf).collect();
        assert_eq!(leaves.len(), 66);
        let first = &d.instances[0];
        assert_eq!(first.index, 6);
        assert!((first.origin - Vec3::new(-1_559.114_9, -909.122_7, 0.)).length() < 1e-3);
        assert!((first.scale - 0.903_986_3).abs() < 1e-6);
        assert!(d.styles_ldr.is_empty() && d.styles_hdr.is_empty());
    }
}
