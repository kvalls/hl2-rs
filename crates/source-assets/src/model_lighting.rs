//! BSP lumps for model lighting: leaf tree, leaf ambient samples and world lights.
//! LDR or HDR data follows the same choice as the lightmap atlas.
use crate::{bytes, f32le, i16le, i32le, index, records, u16le, vec3};
use anyhow::{bail, Context, Result};
use glam::Vec3;
use modkit_core::lighting::{
    AmbientSample, EmitType, LightLeaf, LightNode, LightingData, WorldLight,
};

/// ColorRGBExp32ToVector: channel * 2^exponent. Unlike lightmap texels
/// (TexLightToLinear, divided by 255), the engine scales cube colors back by 255.
fn rgbexp(d: &[u8]) -> Vec3 {
    let scale = 2f32.powi(i32::from(d[3] as i8));
    Vec3::new(f32::from(d[0]), f32::from(d[1]), f32::from(d[2])) * scale
}
fn cube(d: &[u8]) -> [Vec3; 6] {
    std::array::from_fn(|i| rgbexp(&d[i * 4..i * 4 + 4]))
}
fn short3(d: &[u8], o: usize) -> Result<Vec3> {
    Ok(Vec3::new(
        f32::from(i16le(d, o)?),
        f32::from(i16le(d, o + 2)?),
        f32::from(i16le(d, o + 4)?),
    ))
}

pub fn read(lumps: &[Vec<u8>], versions: &[u32], hdr: bool) -> Result<LightingData> {
    read_with(lumps, versions, crate::lighting::choose_hdr(lumps, hdr))
}
/// Read the HDR (`hdr`) or LDR copy of the ambient samples and world lights.
pub fn read_with(lumps: &[Vec<u8>], versions: &[u32], hdr: bool) -> Result<LightingData> {
    let lump = |id: usize| lumps.get(id).map(Vec::as_slice).unwrap_or_default();
    let version = |id: usize| versions.get(id).copied().unwrap_or(0);
    let leaf_version = version(10);
    let leaf_size = match leaf_version {
        0 => 56,
        1 => 32,
        v => bail!("unsupported BSP leaf version {v}"),
    };
    let planes = records(lump(1), 20)?
        .map(|p| Ok((vec3(p, 0)?, f32le(p, 12)?)))
        .collect::<Result<Vec<_>>>()?;
    let nodes = records(lump(5), 32)?
        .map(|n| {
            let (normal, distance) = planes[index(i32le(n, 0)?, planes.len())?];
            Ok(LightNode {
                normal,
                distance,
                children: [i32le(n, 4)?, i32le(n, 8)?],
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let root = i32le(
        bytes(lump(14), 0, 48).context("BSP missing world model")?,
        36,
    )?;
    let raw_leaves = records(lump(10), leaf_size)?.collect::<Vec<_>>();
    let (index_lump, sample_lump) = if hdr { (51, 55) } else { (52, 56) };
    let mut ambient = Vec::new();
    let mut ranges = Vec::with_capacity(raw_leaves.len());
    if version(sample_lump) == 1 && !lump(index_lump).is_empty() {
        for record in records(lump(index_lump), 4)? {
            ranges.push((u16le(record, 0)?, u16le(record, 2)?));
        }
        if ranges.len() != raw_leaves.len() {
            bail!("leaf ambient index count differs from leaf count");
        }
        for sample in records(lump(sample_lump), 28)? {
            ambient.push(AmbientSample {
                cube: cube(sample),
                position: [sample[24], sample[25], sample[26]],
            });
        }
        for &(count, first) in &ranges {
            if count != 0 && usize::from(first) + usize::from(count) > ambient.len() {
                bail!("leaf ambient range outside sample lump");
            }
        }
    } else {
        // Older data: one centered cube per leaf, from the ambient lump or v0 leaves.
        let legacy = lump(sample_lump);
        for (i, leaf) in raw_leaves.iter().enumerate() {
            let cube = if legacy.len() >= (i + 1) * 24 {
                cube(&legacy[i * 24..])
            } else if leaf_version == 0 {
                cube(bytes(leaf, 30, 24)?)
            } else {
                [Vec3::splat(0.5); 6]
            };
            ambient.push(AmbientSample {
                cube,
                position: [0x80; 3],
            });
            ranges.push((1, u16::try_from(i).context("too many leaves for ambient")?));
        }
    }
    let mut leaves = raw_leaves
        .iter()
        .zip(&ranges)
        .map(|(leaf, &(count, first))| {
            Ok(LightLeaf {
                contents: i32le(leaf, 0)?,
                flags: u16le(leaf, 6)? >> 9,
                mins: short3(leaf, 8)?,
                maxs: short3(leaf, 14)?,
                ambient_count: count,
                ambient_first: first,
                sky_faces: Vec::new(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    add_sky_faces(lumps, hdr, &raw_leaves, &mut leaves)?;
    let light_lump = lump(if hdr { 54 } else { 15 });
    let stride = if light_lump.len() % 88 == 0 { 88 } else { 84 };
    let lights = records(light_lump, stride)?
        .map(|l| {
            let raw_kind = i32le(l, 40)?;
            Ok(WorldLight {
                origin: vec3(l, 0)?,
                intensity: vec3(l, 12)?,
                normal: vec3(l, 24)?,
                kind: EmitType::from_raw(raw_kind)
                    .with_context(|| format!("unknown world light type {raw_kind}"))?,
                style: i32le(l, 44)?,
                stopdot: f32le(l, 48)?,
                stopdot2: f32le(l, 52)?,
                exponent: f32le(l, 56)?,
                radius: f32le(l, 60)?,
                attenuation: [f32le(l, 64)?, f32le(l, 68)?, f32le(l, 72)?],
                flags: if stride == 88 { i32le(l, 76)? } else { 0 },
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(LightingData {
        root,
        nodes,
        leaves,
        ambient,
        lights,
        dynamic_minlight: 1.,
    })
}

/// Collect SURF_SKY/SURF_SKY2D face polygons for every leaf that lists them.
fn add_sky_faces(
    lumps: &[Vec<u8>],
    hdr: bool,
    raw_leaves: &[&[u8]],
    leaves: &mut [LightLeaf],
) -> Result<()> {
    let lump = |id: usize| lumps.get(id).map(Vec::as_slice).unwrap_or_default();
    let faces = records(lump(if hdr { 58 } else { 7 }), 56)?.collect::<Vec<_>>();
    let texinfo = records(lump(6), 72)?.collect::<Vec<_>>();
    let positions = records(lump(3), 12)?
        .map(|v| vec3(v, 0))
        .collect::<Result<Vec<_>>>()?;
    let edges = records(lump(12), 4)?.collect::<Vec<_>>();
    let surfedges = records(lump(13), 4)?.collect::<Vec<_>>();
    let leaffaces = records(lump(16), 2)?.collect::<Vec<_>>();
    let mut polygons = std::collections::BTreeMap::<usize, Vec<Vec3>>::new();
    for (leaf, raw) in leaves.iter_mut().zip(raw_leaves) {
        let first = usize::from(u16le(raw, 20)?);
        let count = usize::from(u16le(raw, 22)?);
        for entry in leaffaces.get(first..first + count).unwrap_or_default() {
            let face_id = usize::from(u16le(entry, 0)?);
            let Some(face) = faces.get(face_id) else {
                continue;
            };
            let info = i16le(face, 10)?;
            let Some(info) = usize::try_from(info).ok().and_then(|i| texinfo.get(i)) else {
                continue;
            };
            if i32le(info, 64)? & 0x6 == 0 {
                continue;
            }
            let polygon = match polygons.entry(face_id) {
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::btree_map::Entry::Vacant(entry) => {
                    let first_edge = usize::try_from(i32le(face, 4)?)?;
                    let edge_count = usize::try_from(i16le(face, 8)?)?;
                    let mut polygon = Vec::with_capacity(edge_count);
                    for surfedge in surfedges
                        .get(first_edge..first_edge + edge_count)
                        .context("sky face edges outside lump")?
                    {
                        let edge = i32le(surfedge, 0)?;
                        let record = edges
                            .get(edge.unsigned_abs() as usize)
                            .context("sky face edge outside lump")?;
                        let vertex = u16le(record, if edge < 0 { 2 } else { 0 })?;
                        polygon.push(
                            *positions
                                .get(usize::from(vertex))
                                .context("sky face vertex outside lump")?,
                        );
                    }
                    entry.insert(polygon)
                }
            };
            leaf.sky_faces.push(polygon.clone());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rgbexp_decodes_signed_exponent() {
        assert_eq!(rgbexp(&[255, 0, 128, 0]), Vec3::new(255., 0., 128.));
        assert_eq!(rgbexp(&[235, 255, 255, 0xF3]).x, 235. / 8192.);
    }
    #[test]
    #[ignore = "requires owned HL2 installation"]
    fn owned_security_map_has_leaf_ambient_and_world_lights() {
        let vfs = crate::vpk::Vfs::mount(&crate::install::discover().unwrap()).unwrap();
        for map in ["d1_trainstation_01", "d1_trainstation_02"] {
            let bytes = vfs.read(&format!("maps/{map}.bsp")).unwrap().unwrap();
            let bsp = crate::bsp::Bsp::parse(&bytes).unwrap();
            let data = bsp.model_lighting().unwrap();
            assert!(!data.lights.is_empty(), "{map}: world lights");
            assert!(
                data.ambient.len() >= data.leaves.len() / 4,
                "{map}: ambient samples"
            );
            // Barney's security_02 desk area must resolve to a lit leaf.
            let state = data.state_at(Vec3::new(-3346., -315., 40.));
            let ambient: f32 = state.ambient.iter().map(|c| c.length()).sum();
            println!(
                "{map}: {} lights, {} samples, desk ambient {ambient:.4}, {} local lights",
                data.lights.len(),
                data.ambient.len(),
                state.lights.len()
            );
        }
    }
}
