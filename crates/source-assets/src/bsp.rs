//! BSP 19/20 world brush faces, displacement terrain, entity KeyValues, and solid brush planes.
use crate::{bytes, f32le, i16le, i32le, index, keyvalues, records, u16le, u32le, vec3};
use anyhow::{bail, Context, Result};
use glam::{Vec2, Vec3};
use modkit_core::{Brush, Plane, Surface, Vertex, World};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
};

pub struct Bsp {
    pub version: u32,
    pub revision: u32,
    lumps: Vec<Vec<u8>>,
    pub lump_versions: Vec<u32>,
    pub static_props: Vec<modkit_core::ModelInstance>,
    lightmaps: Vec<modkit_core::Lightmap>,
    face_lighting: Vec<Option<crate::lighting::FaceLight>>,
}

/// Independent sky eligibility bits in the camera's containing BSP leaf.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct SkyVisibility {
    pub sky_3d: bool,
    pub sky_2d: bool,
}
impl SkyVisibility {
    /// A 3D sky also needs the distant 2D background behind its miniature geometry.
    pub fn background_visible(self) -> bool {
        self.sky_3d || self.sky_2d
    }
}
impl Bsp {
    pub fn parse(data: &[u8]) -> Result<Self> {
        if bytes(data, 0, 4)? != b"VBSP" {
            bail!("not a Valve BSP");
        }
        bytes(data, 0, 1036)?;
        let version = u32le(data, 4)?;
        if version != 19 && version != 20 {
            bail!("unsupported BSP version {version}; reader supports HL2 BSP 19/20");
        }
        let mut lumps = Vec::new();
        let mut lump_versions = Vec::new();
        for i in 0..64 {
            let o = 8 + i * 16;
            let offset = u32le(data, o)? as usize;
            let len = u32le(data, o + 4)? as usize;
            lump_versions.push(u32le(data, o + 8)?);
            let raw = bytes(data, offset, len)?;
            let decoded = if raw.starts_with(b"LZMA") {
                let expected = u32le(raw, 4)? as usize;
                let compressed = u32le(raw, 8)? as usize;
                if expected > 128 * 1024 * 1024 {
                    bail!("BSP lump {i} exceeds 128 MiB limit");
                }
                let mut stream = bytes(raw, 12, 5)?.to_vec();
                stream.extend_from_slice(&(expected as u64).to_le_bytes());
                stream.extend_from_slice(bytes(raw, 17, compressed)?);
                let mut output = Vec::new();
                lzma_rs::lzma_decompress_with_options(
                    &mut Cursor::new(stream),
                    &mut output,
                    &lzma_rs::decompress::Options {
                        memlimit: Some(128 * 1024 * 1024),
                        ..Default::default()
                    },
                )
                .with_context(|| format!("LZMA lump {i}"))?;
                if output.len() != expected {
                    bail!("BSP lump {i} decompressed size mismatch");
                }
                output
            } else {
                raw.to_vec()
            };
            lumps.push(decoded);
        }
        let (lightmaps, face_lighting) = crate::lighting::build(&lumps)?;
        Ok(Self {
            lightmaps,
            face_lighting,
            static_props: crate::models::static_props(data, &lumps[35])?,
            version,
            revision: u32le(data, 1032)?,
            lumps,
            lump_versions,
        })
    }
    pub fn lump(&self, id: usize) -> &[u8] {
        &self.lumps[id]
    }
    /// Leaf ambient samples, world lights and the leaf tree for model lighting.
    pub fn model_lighting(&self) -> Result<modkit_core::lighting::LightingData> {
        crate::model_lighting::read(&self.lumps, &self.lump_versions)
    }
    pub fn model_lighting_with(&self, hdr: bool) -> Result<modkit_core::lighting::LightingData> {
        crate::model_lighting::read_with(&self.lumps, &self.lump_versions, hdr)
    }
    pub fn visibility_index(&self) -> Result<crate::visibility::VisibilityIndex> {
        crate::visibility::VisibilityIndex::new(&self.lumps, self.lump_versions[10])
    }
    pub fn bounds_in_pvs(&self, eye: Vec3, mins: Vec3, maxs: Vec3) -> Result<bool> {
        crate::visibility::bounds_in_pvs(&self.lumps, self.lump_versions[10], eye, mins, maxs)
    }
    /// Query separate LEAF_FLAGS_SKY (3D) and LEAF_FLAGS_SKY2D eligibility.
    /// Missing/malformed node/leaf tables or a non-finite point return an error.
    pub fn sky_visibility(&self, point: Vec3) -> Result<SkyVisibility> {
        let leaf = crate::visibility::leaf(&self.lumps, self.lump_versions[10], point)?;
        // dleaf_t / dleaf_version_0_t pack area:9 and flags:7 into the word at +6.
        let flags = u16le(leaf, 6)? >> 9;
        Ok(SkyVisibility {
            sky_3d: flags & 0x01 != 0,
            sky_2d: flags & 0x04 != 0,
        })
    }
    /// Whether the current leaf permits miniature 3D sky scenery.
    /// A leaf with only 2D sky must not enable background models.
    pub fn sky_3d_visible(&self, point: Vec3) -> Result<bool> {
        Ok(self.sky_visibility(point)?.sky_3d)
    }
    pub fn world(&self, name: &str) -> Result<World> {
        let mut world = self.model_world(name, 0)?;
        let clusters = self.background_clusters()?;
        if let Some(e) = world.entities.iter().find(|e| e.class() == "sky_camera") {
            world.background_camera = Some(modkit_core::BackgroundCamera {
                origin: e.origin(),
                scale: e
                    .get("scale")
                    .and_then(|s| s.parse::<f32>().ok())
                    .filter(|s| s.is_finite() && *s > 0.)
                    .unwrap_or(16.),
            });
        }
        for instance in &mut world.model_instances {
            instance.background = clusters.contains(&crate::visibility::cluster(
                &self.lumps,
                self.lump_versions[10],
                instance.origin,
            )?);
        }
        for (id, e) in world.entities.iter().enumerate() {
            if clusters.contains(&crate::visibility::cluster(
                &self.lumps,
                self.lump_versions[10],
                e.origin(),
            )?) {
                world.background_entities.push(id);
            }
        }
        for (id, record) in records(self.lump(14), 48)?.enumerate().skip(1) {
            let model = self.model_world(name, id)?;
            world.brush_models.push(modkit_core::BrushModel {
                id,
                surfaces: model.surfaces,
                brushes: model.brushes,
                mins: vec3(record, 0)?,
                maxs: vec3(record, 12)?,
            });
        }
        match self.model_lighting() {
            Ok(lighting) => world.lighting = Some(std::sync::Arc::new(lighting)),
            Err(e) => world.warnings.push(format!("model lighting data: {e:#}")),
        }
        Ok(world)
    }
    fn background_clusters(&self) -> Result<BTreeSet<i16>> {
        let entities = keyvalues::entities(self.lump(0))?;
        let Some(e) = entities.iter().find(|e| e.class() == "sky_camera") else {
            return Ok(BTreeSet::new());
        };
        crate::visibility::pvs(
            self.lump(4),
            crate::visibility::cluster(&self.lumps, self.lump_versions[10], e.origin())?,
        )
    }
    fn model_world(&self, name: &str, model_index: usize) -> Result<World> {
        let mut world = World {
            name: name.into(),
            lightmaps: if model_index == 0 {
                self.lightmaps.clone()
            } else {
                Vec::new()
            },
            entities: if model_index == 0 {
                keyvalues::entities(self.lump(0))?
            } else {
                Vec::new()
            },
            model_instances: if model_index == 0 {
                self.static_props.clone()
            } else {
                Vec::new()
            },
            ..Default::default()
        };
        let positions = records(self.lump(3), 12)?
            .map(|v| vec3(v, 0))
            .collect::<Result<Vec<_>>>()?;
        let edges = records(self.lump(12), 4)?
            .map(|v| Ok([u16le(v, 0)? as usize, u16le(v, 2)? as usize]))
            .collect::<Result<Vec<_>>>()?;
        let surfedges = records(self.lump(13), 4)?
            .map(|v| i32le(v, 0))
            .collect::<Result<Vec<_>>>()?;
        let texinfo = records(self.lump(6), 72)?.collect::<Vec<_>>();
        let plane_normals = records(self.lump(1), 20)?
            .map(|p| vec3(p, 0))
            .collect::<Result<Vec<_>>>()?;
        let texdata = records(self.lump(2), 32)?.collect::<Vec<_>>();
        let strings = self.lump(43);
        let table = self.lump(44);
        let models = records(self.lump(14), 48)?.collect::<Vec<_>>();
        let model = models.get(model_index).context("BSP missing model")?;
        let first = i32le(model, 40)?;
        let count = i32le(model, 44)?;
        let use_hdr = crate::lighting::use_hdr(&self.lumps);
        let faces = records(self.lump(if use_hdr { 58 } else { 7 }), 56)?.collect::<Vec<_>>();
        let lighting = self.lump(if use_hdr { 53 } else { 8 });
        let disps = records(self.lump(26), 176)?.collect::<Vec<_>>();
        let dispverts = records(self.lump(33), 20)?.collect::<Vec<_>>();
        let background_faces = if model_index == 0 {
            crate::visibility::faces(
                &self.lumps,
                self.lump_versions[10],
                &self.background_clusters()?,
            )?
        } else {
            BTreeSet::new()
        };
        let mut batches: BTreeMap<(String, Option<usize>, bool), Surface> = BTreeMap::new();
        let mut displacement_count = 0;
        if first < 0
            || count < 0
            || (first as usize)
                .checked_add(count as usize)
                .is_none_or(|end| end > faces.len())
        {
            bail!("world face range outside faces");
        }
        for (local_face, f) in faces[first as usize..(first + count) as usize]
            .iter()
            .enumerate()
        {
            let face_light = self.face_lighting[first as usize + local_face];
            let ti = i16le(f, 10)?;
            if ti < 0 {
                continue;
            }
            let info = texinfo[index(ti as i32, texinfo.len())?];
            let flags = u32le(info, 64)?;
            // Sky and nodraw faces are not world surfaces. No Source shaders are executed.
            if flags & (0x2 | 0x4 | 0x80) != 0 {
                continue;
            }
            let td = texdata[index(i32le(info, 68)?, texdata.len())?];
            let name_id = i32le(td, 12)?;
            let offset = u32le(table, index(name_id, table.len() / 4)? * 4)? as usize;
            let tail = strings
                .get(offset..)
                .context("texture string outside lump")?;
            let end = tail
                .iter()
                .position(|b| *b == 0)
                .context("unterminated texture string")?;
            let material = std::str::from_utf8(&tail[..end])?.to_lowercase();
            // Compile tools are invisible, except toolsblack (an UnlitGeneric black texture),
            // e.g. the frame around trainstation_02's Combine slate.
            if material.starts_with("tools/") && !material.starts_with("tools/toolsblack") {
                continue;
            }
            let width = i32le(td, 16)?.max(1) as f32;
            let height = i32le(td, 20)?.max(1) as f32;
            let firstedge = i32le(f, 4)?;
            let numedges = i16le(f, 8)?;
            if firstedge < 0
                || numedges < 0
                || firstedge as usize + numedges as usize > surfedges.len()
            {
                bail!("face edge range outside surfedges");
            }
            let mut polygon = Vec::new();
            for &edge in &surfedges[firstedge as usize..firstedge as usize + numedges as usize] {
                let edge_id = edge.checked_abs().context("edge index overflow")?;
                let e = edges[index(edge_id, edges.len())?];
                let vertex = e[usize::from(edge < 0)];
                polygon.push(
                    *positions
                        .get(vertex)
                        .context("edge vertex outside vertices")?,
                );
            }
            if polygon.len() < 3 {
                continue;
            }
            let s = vec3(info, 0)?;
            let so = f32le(info, 12)?;
            let t = vec3(info, 16)?;
            let to = f32le(info, 28)?;
            let uv = |p: Vec3| Vec2::new((s.dot(p) + so) / width, (t.dot(p) + to) / height);
            let normal = (polygon[1] - polygon[0])
                .cross(polygon[2] - polygon[0])
                .normalize_or_zero();
            // Shading normal (envmap reflections): the face plane, oriented toward the side the
            // clockwise Source winding faces. In owned maps that is the stored plane for both
            // `side` values (d1_trainstation_02: 6597 of 6601 brush faces), so the winding decides
            // and `side` only breaks ties for degenerate windings. Displacements reuse their
            // base face's normal (an approximation).
            let plane = *plane_normals
                .get(usize::from(u16le(f, 0)?))
                .context("face plane outside planes")?;
            // Summed fan area: robust where the first three vertices are nearly collinear.
            let area = (1..polygon.len() - 1).fold(Vec3::ZERO, |sum, i| {
                sum + (polygon[i] - polygon[0]).cross(polygon[i + 1] - polygon[0])
            });
            let winding = plane.dot(area);
            let face_normal = if winding > 0. || (winding == 0. && bytes(f, 2, 1)?[0] != 0) {
                -plane
            } else {
                plane
            };
            let color = |p: Vec3| -> Result<[u8; 4]> {
                let lightofs = i32le(f, 20)?;
                if lightofs < 0 || lighting.is_empty() {
                    let v = (170. + normal.z.abs() * 55.) as u8;
                    return Ok([v, v, v, 255]);
                }
                let x = (vec3(info, 32)?.dot(p) + f32le(info, 44)? - i32le(f, 28)? as f32)
                    .round()
                    .clamp(0., i32le(f, 36)?.max(0) as f32) as usize;
                let y = (vec3(info, 48)?.dot(p) + f32le(info, 60)? - i32le(f, 32)? as f32)
                    .round()
                    .clamp(0., i32le(f, 40)?.max(0) as f32) as usize;
                let stride = usize::try_from(i32le(f, 36)?.max(0))? + 1;
                let sample = bytes(lighting, lightofs as usize + 4 * (y * stride + x), 4)?;
                let scale = 2f32.powi(sample[3] as i8 as i32);
                let mut rgb = [0u8; 4];
                for i in 0..3 {
                    rgb[i] = ((sample[i] as f32 * scale / 255.).max(0.04).powf(1. / 2.2) * 255.)
                        .min(255.) as u8;
                }
                rgb[3] = 255;
                Ok(rgb)
            };
            let light_uv = |p: Vec3| -> Result<Vec2> {
                if let Some(l) = face_light {
                    let x = (vec3(info, 32)?.dot(p) + f32le(info, 44)? - i32le(f, 28)? as f32)
                        .clamp(0., (l.width - 1) as f32);
                    let y = (vec3(info, 48)?.dot(p) + f32le(info, 60)? - i32le(f, 32)? as f32)
                        .clamp(0., (l.height - 1) as f32);
                    Ok(Vec2::new(
                        (l.x as f32 + x + 0.5) / 1024.,
                        (l.y as f32 + y + 0.5) / 1024.,
                    ))
                } else {
                    Ok(Vec2::ZERO)
                }
            };
            let batch = batches
                .entry((
                    material.clone(),
                    face_light.map(|l| l.page),
                    background_faces.contains(&(first as usize + local_face)),
                ))
                .or_insert_with(|| Surface {
                    flex_source: None,
                    background: background_faces.contains(&(first as usize + local_face)),
                    material,
                    vertices: Vec::new(),
                    indices: Vec::new(),
                    lightmap: face_light.map(|l| l.page),
                });
            let disp = i16le(f, 12)?;
            if disp >= 0 {
                if polygon.len() != 4 {
                    bail!("displacement base is not a quad");
                }
                let d = disps[index(disp as i32, disps.len())?];
                let power = i32le(d, 20)?;
                if !(2..=4).contains(&power) {
                    bail!("invalid displacement power {power}");
                }
                let start = vec3(d, 0)?;
                let corner = (0..4)
                    .min_by(|&a, &b| {
                        polygon[a]
                            .distance_squared(start)
                            .total_cmp(&polygon[b].distance_squared(start))
                    })
                    .unwrap();
                polygon.rotate_left(corner);
                let cells = 1usize << power;
                let side = cells + 1;
                let base = batch.vertices.len() as u32;
                let dvstart = i32le(d, 12)?;
                for y in 0..side {
                    for x in 0..side {
                        let fy = y as f32 / cells as f32;
                        let fx = x as f32 / cells as f32;
                        let p = polygon[0]
                            .lerp(polygon[1], fy)
                            .lerp(polygon[3].lerp(polygon[2], fy), fx);
                        let dv =
                            dispverts[index(dvstart + (y * side + x) as i32, dispverts.len())?];
                        let pos = p + vec3(dv, 0)? * f32le(dv, 12)?;
                        batch.vertices.push(Vertex {
                            normal: face_normal,
                            position: pos,
                            uv: uv(p),
                            color: if face_light.is_some() {
                                [255; 4]
                            } else {
                                color(p)?
                            },
                            skin: None,
                            light_uv: light_uv(p)?,
                        });
                    }
                }
                for y in 0..cells {
                    for x in 0..cells {
                        let a = base + (y * side + x) as u32;
                        let b = a + side as u32;
                        batch
                            .indices
                            .extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
                    }
                }
                let count = cells * cells * 6;
                world.terrain.push(Surface {
                    flex_source: None,
                    background: batch.background,
                    lightmap: batch.lightmap,
                    material: batch.material.clone(),
                    vertices: batch.vertices[base as usize..].to_vec(),
                    indices: batch.indices[batch.indices.len() - count..]
                        .iter()
                        .map(|i| i - base)
                        .collect(),
                });
                displacement_count += 1;
            } else {
                let base = batch.vertices.len() as u32;
                for p in polygon.iter().copied() {
                    batch.vertices.push(Vertex {
                        normal: face_normal,
                        position: p,
                        uv: uv(p),
                        color: if face_light.is_some() {
                            [255; 4]
                        } else {
                            color(p)?
                        },
                        skin: None,
                        light_uv: light_uv(p)?,
                    });
                }
                for i in 1..polygon.len() - 1 {
                    batch
                        .indices
                        .extend_from_slice(&[base, base + i as u32, base + i as u32 + 1]);
                }
            }
        }
        world.surfaces = batches.into_values().collect();
        let planes = records(self.lump(1), 20)?
            .map(|p| {
                Ok(Plane {
                    normal: vec3(p, 0)?,
                    distance: f32le(p, 12)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let sides = records(self.lump(19), 8)?.collect::<Vec<_>>();
        let brushes = records(self.lump(18), 12)?.collect::<Vec<_>>();
        let nodes = records(self.lump(5), 32)?.collect::<Vec<_>>();
        let leaf_size = if self.lump_versions[10] == 0 { 56 } else { 32 };
        let leaves = records(self.lump(10), leaf_size)?.collect::<Vec<_>>();
        let leafbrushes = records(self.lump(17), 2)?
            .map(|r| u16le(r, 0))
            .collect::<Result<Vec<_>>>()?;
        let mut stack = vec![i32le(model, 36)?];
        let mut visited = BTreeSet::new();
        let mut world_brushes = BTreeSet::new();
        while let Some(node) = stack.pop() {
            if !visited.insert(node) {
                continue;
            }
            if node >= 0 {
                let n = nodes[index(node, nodes.len())?];
                stack.push(i32le(n, 4)?);
                stack.push(i32le(n, 8)?);
            } else {
                let leaf = leaves[index(
                    node.checked_neg().context("leaf index overflow")? - 1,
                    leaves.len(),
                )?];
                let start = u16le(leaf, 24)? as usize;
                let count = u16le(leaf, 26)? as usize;
                for &brush in leafbrushes
                    .get(start..start + count)
                    .context("leaf brush range")?
                {
                    world_brushes.insert(brush as usize);
                }
            }
        }
        for id in world_brushes {
            let b = brushes.get(id).context("brush index")?;
            let contents = u32le(b, 8)?;
            if model_index == 0 && contents & (1 | 0x10000 | 0x20000) == 0 {
                continue;
            }
            let start = usize::try_from(i32le(b, 0)?)?;
            let count = usize::try_from(i32le(b, 4)?)?;
            let mut ps = Vec::new();
            for side in sides
                .get(start..start + count)
                .context("brush side range")?
            {
                ps.push(
                    *planes
                        .get(u16le(side, 0)? as usize)
                        .context("brush plane index")?,
                );
            }
            world.brushes.push(Brush {
                planes: ps,
                contents,
            });
        }
        if model_index == 0 {
            world.warnings.push(format!("{displacement_count} displacement surfaces have triangle collision; neighbor seams still need validation"));
            world.warnings.push(
                "Source campaign fidelity remains incomplete; see README and entity diagnostics"
                    .into(),
            );
        }
        Ok(world)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bad_headers_are_rejected() {
        assert!(Bsp::parse(b"VBSP").is_err());
        let mut b = vec![0; 1036];
        b[..4].copy_from_slice(b"VBSP");
        b[4..8].copy_from_slice(&21u32.to_le_bytes());
        assert!(Bsp::parse(&b).is_err());
    }
    #[test]
    fn lump_bounds_are_checked() {
        let mut b = vec![0; 1036];
        b[..4].copy_from_slice(b"VBSP");
        b[4..8].copy_from_slice(&20u32.to_le_bytes());
        b[8..12].copy_from_slice(&1035u32.to_le_bytes());
        b[12..16].copy_from_slice(&20u32.to_le_bytes());
        assert!(Bsp::parse(&b).is_err());
    }
    fn sky_test_bsp(leaf_version: u32) -> Bsp {
        let mut lumps = vec![Vec::new(); 64];
        lumps[1] = vec![0; 20];
        lumps[1][..4].copy_from_slice(&1f32.to_le_bytes());
        lumps[5] = vec![0; 32];
        lumps[5][4..8].copy_from_slice(&(-1i32).to_le_bytes());
        lumps[5][8..12].copy_from_slice(&(-2i32).to_le_bytes());
        let size = if leaf_version == 0 { 56 } else { 32 };
        lumps[10] = vec![0; 2 * size];
        // Distinguish packed area bits from 3D/2D sky flags on opposite sides of x=0.
        lumps[10][6..8].copy_from_slice(&(511u16 | (1 << 9)).to_le_bytes());
        lumps[10][size + 6..size + 8].copy_from_slice(&(4u16 << 9).to_le_bytes());
        lumps[14] = vec![0; 48];
        let mut lump_versions = vec![0; 64];
        lump_versions[10] = leaf_version;
        Bsp {
            version: 20,
            revision: 0,
            lumps,
            lump_versions,
            static_props: Vec::new(),
            lightmaps: Vec::new(),
            face_lighting: Vec::new(),
        }
    }
    #[test]
    fn sky_visibility_tracks_current_leaf_and_separates_2d_sky() {
        for version in [0, 1] {
            let bsp = sky_test_bsp(version);
            assert!(bsp.sky_3d_visible(Vec3::X).unwrap());
            assert!(bsp.sky_3d_visible(Vec3::ZERO).unwrap());
            assert!(!bsp.sky_3d_visible(-Vec3::X).unwrap());
            assert_eq!(
                bsp.sky_visibility(Vec3::X).unwrap(),
                SkyVisibility {
                    sky_3d: true,
                    sky_2d: false
                }
            );
            assert_eq!(
                bsp.sky_visibility(-Vec3::X).unwrap(),
                SkyVisibility {
                    sky_3d: false,
                    sky_2d: true
                }
            );
        }
    }
    #[test]
    fn sky_visibility_separates_neither_2d_3d_and_both_flags() {
        for version in [0, 1] {
            let mut bsp = sky_test_bsp(version);
            for (flags, sky_3d, sky_2d) in [
                (0, false, false),
                (1, true, false),
                (4, false, true),
                (5, true, true),
            ] {
                // All area bits and unrelated sky-rad bits must not alter eligibility.
                let packed = 511u16 | ((flags | 2) << 9);
                bsp.lumps[10][6..8].copy_from_slice(&packed.to_le_bytes());
                let visibility = bsp.sky_visibility(Vec3::X).unwrap();
                assert_eq!(visibility, SkyVisibility { sky_3d, sky_2d });
                assert_eq!(visibility.background_visible(), sky_3d || sky_2d);
                assert_eq!(bsp.sky_3d_visible(Vec3::X).unwrap(), sky_3d);
            }
        }
    }
    #[test]
    fn sky_visibility_rejects_missing_leaves_cycles_and_invalid_points() {
        let mut bsp = sky_test_bsp(1);
        assert!(bsp.sky_3d_visible(Vec3::splat(f32::NAN)).is_err());
        assert!(bsp.sky_3d_visible(Vec3::splat(f32::INFINITY)).is_err());
        assert!(bsp.sky_visibility(Vec3::splat(f32::NAN)).is_err());
        assert!(bsp.sky_visibility(Vec3::splat(f32::INFINITY)).is_err());
        bsp.lumps[5][4..8].copy_from_slice(&0i32.to_le_bytes());
        assert!(bsp.sky_3d_visible(Vec3::X).is_err());
        bsp.lumps[5][4..8].copy_from_slice(&(-1i32).to_le_bytes());
        bsp.lumps[10].clear();
        assert!(bsp.sky_3d_visible(Vec3::X).is_err());
    }
    #[test]
    #[ignore = "requires owned HL2 installation"]
    fn owned_trainstation_brush_normals_face_the_rendered_side() {
        let vfs = crate::vpk::Vfs::mount(std::path::Path::new(
            &std::env::var("HL2_ROOT").expect("set HL2_ROOT"),
        ))
        .unwrap();
        let bytes = vfs.read("maps/d1_trainstation_02.bsp").unwrap().unwrap();
        let world = Bsp::parse(&bytes)
            .unwrap()
            .world("d1_trainstation_02")
            .unwrap();
        let (mut front, mut total) = (0, 0);
        for s in &world.surfaces {
            for t in s.indices.as_chunks::<3>().0 {
                let [a, b, c] = t.map(|i| s.vertices[i as usize].position);
                let n = s.vertices[t[0] as usize].normal;
                assert!((n.length() - 1.).abs() < 1e-3);
                let cross = (b - a).cross(c - a);
                // Brush triangles lie in their plane (displacement cells do not).
                let d = cross.normalize_or_zero().dot(n);
                if cross.length() > 1e-2 && d.abs() > 0.99 {
                    total += 1;
                    // Clockwise Source windings: the cross product points behind the face.
                    front += usize::from(d < 0.);
                }
            }
        }
        assert!(front * 100 > total * 99, "{front}/{total}");
    }
}
