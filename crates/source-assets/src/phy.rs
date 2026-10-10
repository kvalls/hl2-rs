//! Bounded VPHY/IVPS convex collision reader, separate from render meshes.
//! Format checked against owned files and SourceIO's MIT-licensed reader:
//! https://github.com/REDxEYE/SourceIO/blob/191e9cc7f39e7259f417dc5f904ad1fe59bb2464/library/models/phy/phy.py
//! This original Rust implementation supports one version-0x100/type-0 solid
//! with zero terminal bone assignment. Bone transforms/ragdolls are unsupported.
use crate::{bytes, f32le, i32le, keyvalues, u16le, u32le, vec3};
use anyhow::{bail, Context, Result};
use glam::Vec3;
use modkit_core::ConvexPiece;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
pub struct Phy {
    pub checksum: u32,
    pub pieces: Vec<ConvexPiece>,
    /// Preserves duplicate sections/keys; mass/material mapping is separate work.
    pub keyvalues: Vec<keyvalues::Entry>,
}

fn relative(data: &[u8], base: usize, field: usize) -> Result<usize> {
    let offset = usize::try_from(base as i64 + i32le(data, field)? as i64)
        .context("negative PHY relative pointer")?;
    if offset < 48 || offset >= data.len() {
        bail!("PHY relative pointer outside solid");
    }
    Ok(offset)
}

/// Reads only the proved static/single-solid format. Every range stays within
/// its solid; pointers cannot escape into the file header or KeyValues tail.
pub fn read(data: &[u8], mdl_checksum: u32) -> Result<Phy> {
    if data.len() > 64 * 1024 * 1024 {
        bail!("PHY file exceeds reader budget");
    }
    if u32le(data, 0)? != 16 || u32le(data, 4)? != 0 || u32le(data, 8)? != 1 {
        bail!("unsupported PHY header or solid count");
    }
    let checksum = u32le(data, 12)?;
    if checksum != mdl_checksum {
        bail!("PHY/MDL checksum mismatch");
    }
    let size = usize::try_from(u32le(data, 16)?)?;
    let end = 20usize.checked_add(size).context("PHY size overflow")?;
    bytes(data, 16, size.checked_add(4).context("PHY size overflow")?)?;
    if bytes(data, 20, 4)? != b"VPHY"
        || u16le(data, 24)? != 0x100
        || u16le(data, 26)? != 0
        || u32le(data, 44)? != 0
    {
        bail!("unsupported VPHY version/type/axis map");
    }
    vec3(data, 32)?;
    let compact = data.get(48..end).context("truncated PHY compact solid")?;
    if compact.len() != u32le(data, 28)? as usize
        || compact.len() < 48
        || u32le(compact, 28)? >> 8 != compact.len() as u32
        || bytes(compact, 44, 4)? != b"IVPS"
    {
        bail!("invalid IVPS compact header");
    }
    for at in (0..28).step_by(4) {
        f32le(compact, at)?;
    }
    let root = u32le(compact, 32)? as usize;
    if root < 48 {
        bail!("PHY tree overlaps compact header");
    }
    let mut pending = vec![(root, 0usize)];
    let mut visited = BTreeSet::new();
    let mut leaves = BTreeSet::new();
    let mut pieces = Vec::new();
    let mut triangle_budget = 0usize;
    while let Some((node, depth)) = pending.pop() {
        if depth > 128 || visited.len() >= 4096 || !visited.insert(node) {
            bail!("cyclic or excessive PHY tree");
        }
        bytes(compact, node, 28)?;
        vec3(compact, node + 8)?;
        let radius = f32le(compact, node + 20)?;
        if radius < 0. {
            bail!("negative PHY tree radius");
        }
        let right = i32le(compact, node)?;
        let convex = i32le(compact, node + 4)?;
        if convex != 0 {
            let leaf = relative(compact, node, node + 4)?;
            bytes(compact, leaf, 16)?;
            let flags = u32le(compact, leaf + 8)?;
            // Internal convexes describe acceleration-tree bounds, not pieces.
            // Their second field is a tree pointer, not a bone assignment.
            if flags & 3 == 0 && leaves.insert(leaf) {
                if i32le(compact, leaf + 4)? != 0 {
                    bail!("unsupported PHY terminal bone assignment");
                }
                let count = usize::from(u16le(compact, leaf + 12)?);
                triangle_budget = triangle_budget
                    .checked_add(count)
                    .context("PHY triangle overflow")?;
                if count < 4 || triangle_budget > 262144 || pieces.len() >= 1024 {
                    bail!("invalid or excessive PHY convex geometry");
                }
                let triangles = bytes(compact, leaf + 16, count * 16)?;
                let vertex_base = relative(compact, leaf, leaf)?;
                let mut ids = BTreeSet::new();
                let mut source_indices = Vec::with_capacity(count);
                for triangle in triangles.as_chunks::<16>().0 {
                    let indices = [
                        u16le(triangle, 4)?,
                        u16le(triangle, 8)?,
                        u16le(triangle, 12)?,
                    ];
                    if indices[0] == indices[1]
                        || indices[1] == indices[2]
                        || indices[0] == indices[2]
                    {
                        bail!("degenerate PHY triangle indices");
                    }
                    ids.extend(indices);
                    source_indices.push(indices);
                }
                if ids.len() < 4 {
                    bail!("PHY piece has fewer than four vertices");
                }
                let mut remap = BTreeMap::new();
                let mut vertices = Vec::with_capacity(ids.len());
                for id in ids {
                    let at = vertex_base
                        .checked_add(usize::from(id) * 16)
                        .context("PHY vertex overflow")?;
                    let point = vec3(compact, at)?;
                    f32le(compact, at + 12)?;
                    // Right-handed IVP metres -> Source Z-up inches. The minus
                    // sign is corroborated by MDL44 bounds and native bench height.
                    let point = Vec3::new(point.x, point.z, -point.y) / 0.0254;
                    if !point.is_finite() {
                        bail!("PHY coordinate conversion overflow");
                    }
                    remap.insert(id, vertices.len() as u32);
                    vertices.push(point);
                }
                pieces.push(ConvexPiece {
                    vertices,
                    indices: source_indices
                        .into_iter()
                        .map(|v| v.map(|id| remap[&id]))
                        .collect(),
                });
            }
        } else if right == 0 {
            bail!("PHY terminal node has no convex");
        }
        if right != 0 {
            pending.push((relative(compact, node, node)?, depth + 1));
            pending.push((
                node.checked_add(28).context("PHY node overflow")?,
                depth + 1,
            ));
        }
    }
    if pieces.is_empty() {
        bail!("PHY has no supported terminal convex pieces");
    }
    let tail = data.get(end..).context("PHY tail missing")?;
    let nul = tail
        .iter()
        .take(1024 * 1024)
        .position(|b| *b == 0)
        .context("PHY KeyValues terminator missing")?;
    if !tail[..nul].is_ascii() {
        bail!("non-ASCII PHY KeyValues");
    }
    let keyvalues = keyvalues::parse(std::str::from_utf8(&tail[..nul])?)?;
    Ok(Phy {
        checksum,
        pieces,
        keyvalues,
    })
}

/// The KeyValues text after every solid, for any solid count (ragdolls included),
/// without reading collision geometry.
pub fn read_keyvalues(data: &[u8]) -> Result<Vec<keyvalues::Entry>> {
    if data.len() > 64 * 1024 * 1024 || u32le(data, 0)? != 16 {
        bail!("unsupported PHY header");
    }
    let count = u32le(data, 8)?;
    if count == 0 || count > 1024 {
        bail!("unsupported PHY solid count");
    }
    let mut at = 16usize;
    for _ in 0..count {
        let size = usize::try_from(u32le(data, at)?)?;
        at = at
            .checked_add(4)
            .and_then(|a| a.checked_add(size))
            .context("PHY solid size overflow")?;
        if at > data.len() {
            bail!("truncated PHY solid");
        }
    }
    let tail = &data[at..];
    let nul = tail
        .iter()
        .take(1024 * 1024)
        .position(|b| *b == 0)
        .unwrap_or(tail.len().min(1024 * 1024));
    if !tail[..nul].is_ascii() {
        bail!("non-ASCII PHY KeyValues");
    }
    keyvalues::parse(std::str::from_utf8(&tail[..nul])?)
}

/// The "solid" blocks' VPhysics parameters, in file order. Keys are read in order,
/// so a repeated key keeps its last value; absent keys keep the SDK defaults
/// (g_PhysDefaultObjectParams). The key parser itself lives in vphysics.dll, so key
/// names follow the shipped .phy text: index, mass, inertia, damping, rotdamping,
/// surfaceprop, volume.
pub fn solids(entries: &[keyvalues::Entry]) -> Vec<modkit_core::PhysicsSolid> {
    entries
        .iter()
        .filter(|e| e.key.eq_ignore_ascii_case("solid"))
        .map(|block| {
            let mut solid = modkit_core::PhysicsSolid::default();
            for child in block.children() {
                let Some(value) = child.text() else {
                    continue;
                };
                let number = value.trim().parse::<f32>().ok().filter(|v| v.is_finite());
                match child.key.to_lowercase().as_str() {
                    "index" => solid.index = value.trim().parse().unwrap_or(solid.index),
                    "mass" => solid.mass = number.unwrap_or(solid.mass),
                    "inertia" => solid.inertia = number.unwrap_or(solid.inertia),
                    "damping" => solid.damping = number.unwrap_or(solid.damping),
                    "rotdamping" => solid.rotdamping = number.unwrap_or(solid.rotdamping),
                    "volume" => solid.volume = number.unwrap_or(solid.volume),
                    "surfaceprop" => solid.surfaceprop = value.trim().to_lowercase(),
                    _ => {}
                }
            }
            solid
        })
        .collect()
}

/// PhysModelParseSolid with solidIndex -1 (physics_shared.cpp): the first solid block.
pub fn first_solid(entries: &[keyvalues::Entry]) -> Option<modkit_core::PhysicsSolid> {
    solids(entries).into_iter().next()
}

/// Narrow adapter guard: only a version-44 MDL with one identity root has
/// a proved PHY-to-model transform. Do not silently apply this to ragdolls/doors.
pub fn identity_root_checksum(mdl: &[u8]) -> Result<u32> {
    if bytes(mdl, 0, 4)? != b"IDST" || u32le(mdl, 4)? != 44 || i32le(mdl, 156)? != 1 {
        bail!("PHY adapter requires MDL44 with one identity root");
    }
    let bone = usize::try_from(i32le(mdl, 160)?)?;
    bytes(mdl, bone, 216)?;
    if i32le(mdl, bone + 4)? != -1 || vec3(mdl, bone + 32)?.length_squared() > 1e-10 {
        bail!("unsupported PHY root parent/translation");
    }
    for axis in 0..4 {
        let expected = if axis == 3 { 1. } else { 0. };
        if (f32le(mdl, bone + 44 + axis * 4)? - expected).abs() > 1e-5 {
            bail!("unsupported PHY root rotation");
        }
    }
    for row in 0..3 {
        for col in 0..4 {
            let expected = if row == col { 1. } else { 0. };
            if (f32le(mdl, bone + 96 + (row * 4 + col) * 4)? - expected).abs() > 1e-5 {
                bail!("unsupported PHY pose-to-bone transform");
            }
        }
    }
    u32le(mdl, 8)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn put32(b: &mut [u8], at: usize, value: u32) {
        b[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn fixture() -> Vec<u8> {
        // Original synthetic tetrahedron: compact header, leaf, four triangles,
        // four float4 points and a terminal tree. No game bytes are embedded.
        let mut b = vec![0u8; 48 + 220];
        put32(&mut b, 0, 16);
        put32(&mut b, 8, 1);
        put32(&mut b, 12, 7);
        put32(&mut b, 16, 248);
        b[20..24].copy_from_slice(b"VPHY");
        b[24..26].copy_from_slice(&0x100u16.to_le_bytes());
        put32(&mut b, 28, 220);
        put32(&mut b, 76, 220 << 8);
        put32(&mut b, 80, 192);
        b[92..96].copy_from_slice(b"IVPS");
        put32(&mut b, 96, 80);
        b[108..110].copy_from_slice(&4u16.to_le_bytes());
        for (n, tri) in [[0u16, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]]
            .iter()
            .enumerate()
        {
            for (i, id) in tri.iter().enumerate() {
                let at = 116 + n * 16 + i * 4;
                b[at..at + 2].copy_from_slice(&id.to_le_bytes());
            }
        }
        for (n, point) in [
            [0f32, 0., 0.],
            [0.0254, 0., 0.],
            [0., -0.0254, 0.],
            [0., 0., 0.0254],
        ]
        .iter()
        .enumerate()
        {
            for (i, value) in point.iter().enumerate() {
                put32(&mut b, 176 + n * 16 + i * 4, value.to_bits());
            }
        }
        // node at240 -> leaf96, a negative relative pointer.
        put32(&mut b, 244, (-144i32) as u32);
        b.extend_from_slice(b"solid { \"index\" \"0\" \"mass\" \"1\" \"mass\" \"2\" }\0");
        b
    }
    #[test]
    fn solid_parameters_use_sdk_defaults_and_last_repeated_key() {
        let data = fixture();
        let entries = read_keyvalues(&data).unwrap();
        let solid = first_solid(&entries).unwrap();
        assert_eq!(solid.mass, 2.);
        assert_eq!(solid.damping, 0.1);
        assert_eq!(solid.rotdamping, 0.1);
        assert_eq!(solid.inertia, 1.);
        assert_eq!(solid.surfaceprop, "default");
        // Synthetic block with every key (not from a game file).
        let entries = keyvalues::parse(
            "solid { \"index\" \"0\" \"mass\" \"35.5\" \"surfaceprop\" \"Wood_Crate\" \"damping\" \"0\" \"rotdamping\" \"0.2\" \"inertia\" \"2\" \"volume\" \"300\" } editparams { \"totalmass\" \"35.5\" }",
        )
        .unwrap();
        let solid = first_solid(&entries).unwrap();
        assert_eq!(
            (
                solid.mass,
                solid.damping,
                solid.rotdamping,
                solid.inertia,
                solid.volume
            ),
            (35.5, 0., 0.2, 2., 300.)
        );
        assert_eq!(solid.surfaceprop, "wood_crate");
    }
    #[test]
    fn signed_pointers_handedness_and_duplicate_metadata() {
        let p = read(&fixture(), 7).unwrap();
        assert_eq!(p.checksum, 7);
        assert_eq!(p.pieces.len(), 1);
        assert_eq!(
            p.pieces[0].vertices,
            vec![Vec3::ZERO, Vec3::X, Vec3::Z, Vec3::Y]
        );
        assert_eq!(p.pieces[0].indices[0], [0, 2, 1]);
        assert_eq!(
            p.keyvalues[0]
                .children()
                .iter()
                .filter(|e| e.key == "mass")
                .count(),
            2
        );
    }
    #[test]
    fn truncation_checksum_tree_and_format_errors() {
        let original = fixture();
        for len in [0, 15, 47, 93, 175, 239, 267, original.len() - 1] {
            assert!(read(&original[..len], 7).is_err(), "len {len}");
        }
        assert!(read(&original, 8).is_err());
        for (at, value) in [
            (16, u32::MAX),
            (24, 0x101),
            (44, 1),
            (80, 0),
            (96, u32::MAX),
            (100, 1),
            (240, 0xffffffe4),
            (176, f32::NAN.to_bits()),
        ] {
            let mut b = original.clone();
            put32(&mut b, at, value);
            assert!(read(&b, 7).is_err(), "field {at}");
        }
    }
    #[test]
    fn child_hulls_are_not_collision_pieces() {
        let mut b = fixture();
        put32(&mut b, 104, 1);
        assert!(read(&b, 7)
            .unwrap_err()
            .to_string()
            .contains("no supported"));
    }
    #[test]
    fn cycle_rejection_does_not_follow_geometry_into_an_unbounded_tree() {
        let mut b = fixture();
        b.splice(268..268, [0u8; 56]);
        put32(&mut b, 16, 304);
        put32(&mut b, 28, 276);
        put32(&mut b, 76, 276 << 8);
        put32(&mut b, 240, 28);
        put32(&mut b, 268, (-28i32) as u32);
        put32(&mut b, 272, (-172i32) as u32);
        put32(&mut b, 300, (-200i32) as u32);
        assert!(read(&b, 7).unwrap_err().to_string().contains("cyclic"));
    }
    #[test]
    fn identity_root_guard_rejects_unproved_model_transforms() {
        let mut mdl = vec![0u8; 192 + 216];
        mdl[..4].copy_from_slice(b"IDST");
        put32(&mut mdl, 4, 44);
        put32(&mut mdl, 8, 7);
        put32(&mut mdl, 156, 1);
        put32(&mut mdl, 160, 192);
        put32(&mut mdl, 196, (-1i32) as u32);
        put32(&mut mdl, 248, 1f32.to_bits());
        for row in 0..3 {
            put32(&mut mdl, 192 + 96 + (row * 4 + row) * 4, 1f32.to_bits());
        }
        assert_eq!(identity_root_checksum(&mdl).unwrap(), 7);
        for (at, value) in [
            (4, 49),
            (156, 2),
            (196, 0),
            (224, 1f32.to_bits()),
            (236, 1f32.to_bits()),
            (288, 0),
        ] {
            let mut bad = mdl.clone();
            put32(&mut bad, at, value);
            assert!(identity_root_checksum(&bad).is_err(), "field {at}");
        }
        assert!(identity_root_checksum(&mdl[..300]).is_err());
    }
}
