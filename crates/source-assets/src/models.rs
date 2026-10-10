//! Static-prop game lumps, model meshes, and installed sequence adapters.
use crate::{bytes, f32le, i16le, i32le, u16le, u32le, vec3, vpk::Vfs};
use anyhow::{bail, Context, Result};
use glam::{Mat3, Vec2, Vec3};
use modkit_core::{parse_vec3, ModelInstance, Surface, Vertex, World};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Studio header eye position used by an ordinary NPC's default view offset.
/// This is metadata only; activity-specific eye changes are separate behavior.
pub fn read_eye_position(vfs: &Vfs, model: &str) -> Result<Vec3> {
    let data = vfs
        .read(model)?
        .with_context(|| format!("missing model {model}"))?;
    eye_position(&data).with_context(|| format!("studio eye position: {model}"))
}
fn eye_position(data: &[u8]) -> Result<Vec3> {
    if data.len() > 64 * 1024 * 1024
        || bytes(data, 0, 4)? != b"IDST"
        || !(44..=49).contains(&i32le(data, 4)?)
        || usize::try_from(i32le(data, 76)?)? != data.len()
    {
        bail!("invalid studio header for eye position");
    }
    vec3(data, 80)
}

#[cfg(test)]
mod eye_position_tests {
    use super::*;
    #[test]
    fn studio_eye_rejects_truncated_mismatched_and_nonfinite_metadata() {
        let mut data = vec![0u8; 92];
        data[..4].copy_from_slice(b"IDST");
        data[4..8].copy_from_slice(&44i32.to_le_bytes());
        data[76..80].copy_from_slice(&92i32.to_le_bytes());
        data[88..92].copy_from_slice(&70f32.to_le_bytes());
        assert_eq!(eye_position(&data).unwrap(), Vec3::new(0., 0., 70.));
        assert!(eye_position(&data[..91]).is_err());
        data[88..92].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(eye_position(&data).is_err());
        data[88..92].copy_from_slice(&70f32.to_le_bytes());
        data[4..8].copy_from_slice(&43i32.to_le_bytes());
        assert!(eye_position(&data).is_err());
    }
}

/// Authored sequence metadata and every blend's independent movement track.
/// The central index matches the current skeletal adapter; pose weighting is not evaluated.
#[derive(Clone, Debug, Serialize)]
pub struct MotionSequence {
    pub source_model: String,
    pub sequence_id: usize,
    pub activity_name: String,
    pub activity: i32,
    pub activity_weight: i32,
    pub flags: u32,
    pub blend_dimensions: [usize; 2],
    pub animation_indices: Vec<usize>,
    pub blends: Vec<modkit_core::animation::RootMotion>,
    pub central_blend: usize,
}
#[derive(Default)]
struct MotionBudget {
    bytes: usize,
    blends: usize,
    records: usize,
}
fn motion_count(data: &[u8], field: usize, limit: usize) -> Result<usize> {
    let count = usize::try_from(i32le(data, field)?)?;
    if count > limit {
        bail!("MDL movement table exceeds limit at {field}");
    }
    Ok(count)
}
fn motion_relative(data: &[u8], base: usize, field: usize) -> Result<usize> {
    Ok(usize::try_from(base as i64 + i32le(data, field)? as i64)?)
}
fn motion_string(data: &[u8], base: usize, field: usize) -> Result<String> {
    if i32le(data, field)? == 0 {
        return Ok(String::new());
    }
    let at = motion_relative(data, base, field)?;
    let tail = data.get(at..).context("MDL movement string outside file")?;
    let end = tail
        .iter()
        .take(4096)
        .position(|b| *b == 0)
        .context("unterminated MDL movement string")?;
    Ok(std::str::from_utf8(&tail[..end])?.into())
}
fn animation_motion(
    data: &[u8],
    at: usize,
    budget: &mut MotionBudget,
) -> Result<modkit_core::animation::RootMotion> {
    use modkit_core::animation::{MovementRecord, RootMotion};
    bytes(data, at, 100)?;
    let count = motion_count(data, at + 20, 4096)?;
    budget.records = budget
        .records
        .checked_add(count)
        .context("movement count overflow")?;
    if budget.records > 65536 {
        bail!("MDL movement record budget exceeded");
    }
    let mut records = Vec::with_capacity(count);
    if count > 0 {
        let start = motion_relative(data, at, at + 24)?;
        bytes(data, start, count * 44)?;
        for i in 0..count {
            let record = start + i * 44;
            records.push(MovementRecord {
                end_frame: i32le(data, record)?,
                motion_flags: u32le(data, record + 4)?,
                v0: f32le(data, record + 8)?,
                v1: f32le(data, record + 12)?,
                end_yaw_degrees: f32le(data, record + 16)?,
                direction: vec3(data, record + 20)?,
                cumulative_position: vec3(data, record + 32)?,
            });
        }
    }
    let motion = RootMotion {
        fps: f32le(data, at + 8)?,
        frame_count: u32::try_from(i32le(data, at + 16)?)?,
        records,
    };
    motion.validate()?;
    Ok(motion)
}
fn model_motion(
    data: &[u8],
    path: &str,
    wanted: &BTreeSet<String>,
    result: &mut BTreeMap<String, MotionSequence>,
    budget: &mut MotionBudget,
) -> Result<Vec<String>> {
    let version = u32le(data, 4)?;
    if bytes(data, 0, 4)? != b"IDST" || !(44..=49).contains(&version) {
        bail!("unsupported root-motion model container version");
    }
    let declared_length = usize::try_from(i32le(data, 76)?)?;
    if !(344..=64 * 1024 * 1024).contains(&declared_length) {
        bail!("invalid MDL movement file length");
    }
    let data = bytes(data, 0, declared_length)?;
    budget.bytes = budget
        .bytes
        .checked_add(data.len())
        .context("MDL byte count overflow")?;
    if budget.bytes > 64 * 1024 * 1024 {
        bail!("included MDL movement byte budget exceeded");
    }
    let animation_count = motion_count(data, 180, 4096)?;
    let animation_base = usize::try_from(i32le(data, 184)?)?;
    bytes(data, animation_base, animation_count * 100)?;
    let count = motion_count(data, 188, 4096)?;
    let base = usize::try_from(i32le(data, 192)?)?;
    bytes(data, base, count * 212)?;
    for id in 0..count {
        let at = base + id * 212;
        let name = motion_string(data, at, at + 4)?.to_lowercase();
        if !wanted.contains(&name) || result.contains_key(&name) {
            continue;
        }
        // Owned Barney geometry is v44 and includes v48 animation models. Only the
        // shared header/label/include traversal applies to other container versions.
        if version != 48 {
            bail!("movement decoding requires MDL version 48: {path}:{name}");
        }
        let count = motion_count(data, at + 56, 4096)?;
        let dimensions = [
            motion_count(data, at + 68, 4096)?,
            motion_count(data, at + 72, 4096)?,
        ];
        if count == 0 || dimensions.contains(&0) || dimensions[0] * dimensions[1] != count {
            bail!("invalid movement blend grid: {path}:{name}");
        }
        budget.blends = budget
            .blends
            .checked_add(count)
            .context("blend count overflow")?;
        if budget.blends > 4096 {
            bail!("MDL movement blend budget exceeded");
        }
        let table = motion_relative(data, at, at + 60)?;
        bytes(data, table, count * 2)?;
        let mut animation_indices = Vec::with_capacity(count);
        let mut blends = Vec::with_capacity(count);
        for blend in 0..count {
            let index = usize::try_from(i16le(data, table + blend * 2)?)?;
            if index >= animation_count {
                bail!("movement animation index outside table: {path}:{name}");
            }
            animation_indices.push(index);
            blends.push(
                animation_motion(data, animation_base + index * 100, budget)
                    .with_context(|| format!("{path}:{name} blend{blend}"))?,
            );
        }
        result.insert(
            name,
            MotionSequence {
                source_model: path.into(),
                sequence_id: id,
                activity_name: motion_string(data, at, at + 8)?,
                activity: i32le(data, at + 16)?,
                activity_weight: i32le(data, at + 20)?,
                flags: u32le(data, at + 12)?,
                blend_dimensions: dimensions,
                animation_indices,
                blends,
                central_blend: count / 2,
            },
        );
    }
    let count = motion_count(data, 336, 64)?;
    let base = usize::try_from(i32le(data, 340)?)?;
    bytes(data, base, count * 8)?;
    (0..count)
        .map(|id| {
            let at = base + id * 8;
            Ok(motion_string(data, at, at + 4)?
                .replace('\\', "/")
                .to_lowercase())
        })
        .collect()
}
/// Read installed v48 movement metadata without loading meshes/ANI.
/// Traversal supports the existing v44-49 model containers; movement decoding remains v48 only.
/// Missing sequence names are absent; empty tracks explicitly return NoMovement when sampled.
pub fn read_root_motion(
    vfs: &Vfs,
    model: &str,
    wanted: &BTreeSet<String>,
) -> Result<BTreeMap<String, MotionSequence>> {
    let wanted = wanted
        .iter()
        .map(|name| name.to_lowercase())
        .collect::<BTreeSet<_>>();
    if wanted.len() > 64 {
        bail!("root-motion request exceeds 64 sequences");
    }
    let mut result = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut pending = vec![(model.replace('\\', "/").to_lowercase(), 0)];
    let mut budget = MotionBudget::default();
    while let Some((path, depth)) = pending.pop() {
        if !seen.insert(path.clone()) {
            continue;
        }
        if depth > 8 || seen.len() > 64 {
            bail!("included-model root-motion traversal budget exceeded");
        }
        let data = vfs
            .read(&path)?
            .with_context(|| format!("root-motion model missing: {path}"))?;
        let includes = model_motion(&data, &path, &wanted, &mut result, &mut budget)
            .with_context(|| path.clone())?;
        // Preserve the skeletal reader's model-before-includes, first-match ordering.
        pending.extend(includes.into_iter().rev().map(|name| (name, depth + 1)));
    }
    Ok(result)
}

pub fn static_props(bsp: &[u8], lump: &[u8]) -> Result<Vec<ModelInstance>> {
    if lump.is_empty() {
        return Ok(Vec::new());
    }
    let count = u32le(lump, 0)? as usize;
    bytes(
        lump,
        4,
        count.checked_mul(16).context("game lump overflow")?,
    )?;
    for n in 0..count {
        let record = &lump[4 + n * 16..4 + (n + 1) * 16];
        if u32le(record, 0)? != 0x73707270 {
            continue;
        }
        if u16le(record, 4)? & 1 != 0 {
            bail!("compressed static-prop lump is unsupported");
        }
        let version = u16le(record, 6)?;
        let data = bytes(bsp, u32le(record, 8)? as usize, u32le(record, 12)? as usize)?;
        return parse_static(data, version);
    }
    Ok(Vec::new())
}
fn parse_static(data: &[u8], version: u16) -> Result<Vec<ModelInstance>> {
    let stride = match version {
        4 => 56,
        5 => 60,
        6 => 64,
        _ => bail!("unsupported static-prop version {version}"),
    };
    let names = u32le(data, 0)? as usize;
    let dictionary = bytes(
        data,
        4,
        names.checked_mul(128).context("prop dictionary overflow")?,
    )?;
    let mut offset = 4 + dictionary.len();
    let leaves = u32le(data, offset)? as usize;
    offset += 4;
    offset += bytes(
        data,
        offset,
        leaves.checked_mul(2).context("prop leaf overflow")?,
    )?
    .len();
    let count = u32le(data, offset)? as usize;
    offset += 4;
    let records = bytes(
        data,
        offset,
        count.checked_mul(stride).context("prop count overflow")?,
    )?;
    let mut props = Vec::with_capacity(count);
    for record in records.chunks_exact(stride) {
        let name = bytes(dictionary, u16le(record, 24)? as usize * 128, 128)?;
        let name = std::str::from_utf8(&name[..name.iter().position(|&b| b == 0).unwrap_or(128)])?;
        props.push(ModelInstance {
            background: false,
            model: name.replace('\\', "/"),
            origin: vec3(record, 0)?,
            angles: vec3(record, 12)?,
            skin: usize::try_from(i32le(record, 32)?)?,
            scale: 1.,
            kind: "static_prop".into(),
            solid: record[30] != 0,
            solid_mode: Some(record[30]),
            entity: None,
        });
    }
    Ok(props)
}
pub fn read_model(vfs: &Vfs, path: &str, skin: usize) -> Result<Vec<Surface>> {
    let stem = path.trim_end_matches(".mdl");
    let mdl_bytes = vfs.read(path)?.context("MDL missing")?;
    let vvd_bytes = vfs.read(&format!("{stem}.vvd"))?.context("VVD missing")?;
    let vtx_bytes = vfs
        .read(&format!("{stem}.dx90.vtx"))?
        .context("DX90 VTX missing")?;
    if bytes(&mdl_bytes, 0, 4)? != b"IDST" || bytes(&vvd_bytes, 0, 4)? != b"IDSV" {
        bail!("model magic mismatch");
    }
    if !(44..=49).contains(&u32le(&mdl_bytes, 4)?)
        || u32le(&vvd_bytes, 4)? != 4
        || u32le(&vtx_bytes, 0)? != 7
    {
        bail!("unsupported MDL/VVD/VTX version");
    }
    let checksum = u32le(&mdl_bytes, 8)?;
    if checksum != u32le(&vvd_bytes, 8)? || checksum != u32le(&vtx_bytes, 16)? {
        bail!("model companion checksum mismatch");
    }
    // Geometry loading must not enter vmdl's unimplemented external animation-block path.
    let mut geometry_bytes = mdl_bytes.clone();
    bytes(&geometry_bytes, 180, 4)?;
    geometry_bytes[180..184].copy_from_slice(&0i32.to_le_bytes());
    let mdl = vmdl::Mdl::read(&geometry_bytes)?;
    let skin_reference_count = usize::try_from(i32le(&mdl_bytes, 220)?)?;
    let vvd = vmdl::Vvd::read(&vvd_bytes)?;
    let vtx = vmdl::Vtx::read(&vtx_bytes)?;
    let mut surfaces = Vec::new();
    for (body_index, (body, topology)) in mdl.body_parts.iter().zip(&vtx.body_parts).enumerate() {
        let Some(model) = body.models.first() else {
            continue;
        };
        let lod = topology
            .models
            .first()
            .and_then(|m| m.lods.first())
            .context("missing model LOD")?;
        for (mesh_index, (mesh, topology)) in model.meshes.iter().zip(&lod.meshes).enumerate() {
            let material = usize::try_from(mesh.material)?;
            let texture_index = if skin_reference_count > 0 {
                let index = skin
                    .checked_mul(skin_reference_count)
                    .and_then(|i| i.checked_add(material))
                    .context("skin overflow")?;
                *mdl.skin_table
                    .get(index)
                    .or_else(|| mdl.skin_table.get(material))
                    .context("skin material missing")? as usize
            } else {
                material
            };
            let texture = mdl
                .textures
                .get(texture_index)
                .context("model texture index outside table")?;
            let candidates = mdl
                .texture_paths
                .iter()
                .map(|p| format!("{}{}", p.replace('\\', "/"), texture.name))
                .chain(std::iter::once(texture.name.clone()));
            let mut material_name = texture.name.clone();
            if let Some(resolved) = vfs.resolve_material_name(&texture.name) {
                material_name = resolved;
            }
            for candidate in candidates {
                if vfs.read(&format!("materials/{candidate}.vmt"))?.is_some() {
                    material_name = candidate;
                    break;
                }
            }
            let mut surface = Surface {
                background: false,
                material: material_name,
                lightmap: None,
                vertices: Vec::new(),
                indices: Vec::new(),
                flex_source: None,
            };
            let mut vertex_ids = Vec::new();
            let base = usize::try_from(model.vertex_offset)?
                .checked_add(usize::try_from(mesh.vertex_offset)?)
                .context("model vertex overflow")?;
            for group in &topology.strip_groups {
                for strip in &group.strips {
                    let mut indices: Vec<_> = strip.indices().collect();
                    // vmdl 0.2 expands two extra triangles for a triangle strip.
                    if strip.flags.contains(vmdl::vtx::StripFlags::IS_TRI_STRIP) {
                        indices.truncate(indices.len().saturating_sub(6));
                    }
                    for triangle in indices.as_chunks::<3>().0 {
                        for &index in triangle {
                            let group_index = *group
                                .indices
                                .get(index)
                                .context("strip index out of range")?
                                as usize;
                            let vertex_id = group
                                .vertices
                                .get(group_index)
                                .context("VTX vertex missing")?
                                .original_mesh_vertex_id
                                as usize;
                            let vertex = vvd
                                .vertices
                                .get(base + vertex_id)
                                .context("VVD vertex missing")?;
                            surface.indices.push(surface.vertices.len() as u32);
                            vertex_ids.push(
                                u16::try_from(vertex_id).context("mesh vertex id exceeds u16")?,
                            );
                            surface.vertices.push(Vertex {
                                normal: Vec3::new(
                                    vertex.normal.x,
                                    vertex.normal.y,
                                    vertex.normal.z,
                                ),
                                position: Vec3::new(
                                    vertex.position.x,
                                    vertex.position.y,
                                    vertex.position.z,
                                ),
                                uv: Vec2::from_array(vertex.texture_coordinates),
                                color: [180, 180, 180, 255],
                                light_uv: Vec2::ZERO,
                                skin: Some({
                                    let mut bones = [0; 3];
                                    let mut weights = [0.; 3];
                                    for (i, w) in vertex.bone_weights.weights().enumerate() {
                                        bones[i] = w.bone_id;
                                        weights[i] = w.weight;
                                    }
                                    modkit_core::animation::Weights { bones, weights }
                                }),
                            });
                        }
                    }
                }
            }
            if !surface.indices.is_empty() {
                surface.flex_source = Some(modkit_core::FlexSource {
                    bodypart: body_index,
                    model: 0,
                    mesh: mesh_index,
                    vertex_ids,
                });
                surfaces.push(surface);
            }
        }
    }
    if surfaces.is_empty() {
        bail!("model has no default-bodygroup triangles");
    }
    Ok(surfaces)
}
pub fn rotation(angles: Vec3) -> Mat3 {
    Mat3::from_rotation_z(angles.y.to_radians())
        * Mat3::from_rotation_y(angles.x.to_radians())
        * Mat3::from_rotation_x(angles.z.to_radians())
}
#[derive(Default, Debug, Serialize)]
pub struct ModelReport {
    pub instances_loaded: usize,
    pub unique_models: usize,
    pub errors: Vec<String>,
    pub rigs_loaded: usize,
    pub clips_loaded: usize,
    pub animation_warnings: Vec<String>,
    pub collision_models_loaded: usize,
    pub collision_pieces_loaded: usize,
    pub collision_models_missing: usize,
    pub collision_warnings: Vec<String>,
    /// prop_physics models with authored .phy solid parameters.
    pub physics_solids_loaded: usize,
    pub surface_materials_loaded: usize,
}
pub fn append_models(world: &mut World, vfs: &Vfs) -> ModelReport {
    let mut physics_report = (0usize, Vec::new());
    match crate::surfaceprops::read(vfs) {
        Ok(table) => world.surface_materials = table,
        Err(error) => physics_report
            .1
            .push(format!("surfaceproperties: {error:#}")),
    }
    for (entity_id, entity) in world.entities.iter().enumerate() {
        if !entity.class().starts_with("npc_")
            && !entity.class().starts_with("weapon_")
            && !entity.class().starts_with("item_")
            && !matches!(
                entity.class(),
                "cycler_actor"
                    | "prop_physics"
                    | "prop_physics_multiplayer"
                    | "prop_dynamic"
                    | "prop_dynamic_override"
                    | "prop_door_rotating"
            )
        {
            continue;
        }
        let default_model = match entity.class() {
            "npc_barney" => Some("models/barney.mdl"),
            "npc_metropolice" => Some("models/police.mdl"),
            "npc_citizen" => Some("models/humans/group01/male_07.mdl"),
            // CNPCCombineCamera::Spawn sets this model (string beside the class name in retail server.dll).
            "npc_combine_camera" => Some("models/combine_camera/combine_camera.mdl"),
            _ => None,
        };
        let script_model = if entity.class().starts_with("weapon_") {
            vfs.read(&format!("scripts/{}.txt", entity.class()))
                .ok()
                .flatten()
                .and_then(|data| {
                    crate::keyvalues::value(&String::from_utf8_lossy(&data), "playermodel")
                        .ok()
                        .flatten()
                })
        } else {
            None
        };
        let item_model = match entity.class() {
            "item_healthkit" => Some("models/items/healthkit.mdl"),
            "item_healthvial" => Some("models/healthvial.mdl"),
            "item_battery" => Some("models/items/battery.mdl"),
            "item_suit" => Some("models/items/hevsuit.mdl"),
            "item_ammo_pistol" | "item_ammo_pistol_large" => Some("models/items/boxsrounds.mdl"),
            _ => None,
        };
        if let Some(model) = entity
            .get("model")
            .filter(|m| m.ends_with(".mdl"))
            .or(script_model.as_deref())
            .or(item_model)
            .or(default_model)
        {
            world.model_instances.push(ModelInstance {
                background: world.background_entities.contains(&entity_id),
                model: model.replace('\\', "/"),
                origin: entity.origin(),
                angles: parse_vec3(entity.get("angles").unwrap_or("0 0 0")).unwrap_or(Vec3::ZERO),
                skin: entity.get("skin").and_then(|s| s.parse().ok()).unwrap_or(0),
                scale: entity
                    .get("modelscale")
                    .and_then(|s| s.parse::<f32>().ok())
                    .filter(|s| s.is_finite() && *s > 0.)
                    .unwrap_or(1.),
                kind: entity.class().into(),
                solid: !entity.class().starts_with("item_")
                    && !entity.class().starts_with("weapon_")
                    && entity.get("solid").is_none_or(|s| s != "0"),
                solid_mode: None,
                entity: Some(entity_id),
            });
        }
    }
    let mut cache: BTreeMap<(String, usize), Option<Vec<Surface>>> = BTreeMap::new();
    let mut report = ModelReport::default();
    let mut collision_attempted = BTreeSet::new();
    let mut collision_modes_reported = BTreeSet::new();
    let mut batches: BTreeMap<(String, Option<usize>, bool), Surface> = world
        .surfaces
        .drain(..)
        .map(|s| ((s.material.clone(), s.lightmap, s.background), s))
        .collect();
    for instance in &world.model_instances {
        let key = (instance.model.to_lowercase(), instance.skin);
        let model = cache.entry(key.clone()).or_insert_with(|| {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                read_model(vfs, &key.0, key.1)
            })) {
                Ok(Ok(s)) => {
                    report.unique_models += 1;
                    Some(s)
                }
                result => {
                    report.errors.push(format!(
                        "{} skin {}: {}",
                        key.0,
                        key.1,
                        match result {
                            Ok(Err(e)) => format!("{e:#}"),
                            _ => "model reader panicked".into(),
                        }
                    ));
                    None
                }
            }
        });
        let Some(model) = model else {
            continue;
        };
        world
            .model_assets
            .entry(instance.asset_key())
            .or_insert_with(|| model.clone());
        // Only static props with a proved identity-root transform use PHY yet.
        // Animated doors, dynamic props and ragdolls retain explicit fallback.
        if instance.solid
            && instance.kind == "static_prop"
            && !instance.background
            && instance.solid_mode != Some(6)
            && collision_modes_reported.insert((instance.asset_key(), instance.solid_mode))
        {
            report.collision_warnings.push(format!(
                "{}: static-prop solid mode {:?} is unsupported; using render collision fallback",
                instance.model, instance.solid_mode
            ));
        }
        if instance.solid
            && instance.kind == "static_prop"
            && !instance.background
            && instance.solid_mode == Some(6)
            && collision_attempted.insert(instance.asset_key())
        {
            match read_collision(vfs, &instance.model) {
                Ok(Some(pieces)) => {
                    report.collision_models_loaded += 1;
                    report.collision_pieces_loaded += pieces.len();
                    world.model_collision.insert(instance.asset_key(), pieces);
                }
                Ok(None) => report.collision_models_missing += 1,
                Err(error) => report.collision_warnings.push(format!(
                    "{}: {error:#}; using render collision fallback",
                    instance.model
                )),
            }
        }
        // VPhysics props (CPhysicsProp::CreateVPhysics): authored solid parameters and,
        // when the identity-root adapter applies, the authored convex pieces.
        if instance.kind.starts_with("prop_physics")
            && !world.model_physics.contains_key(&instance.asset_key())
        {
            match read_physics(vfs, &instance.model) {
                Ok(Some((solid, pieces))) => {
                    physics_report.0 += 1;
                    world.model_physics.insert(instance.asset_key(), solid);
                    match pieces {
                        Ok(pieces) => {
                            report.collision_models_loaded += 1;
                            report.collision_pieces_loaded += pieces.len();
                            world.model_collision.insert(instance.asset_key(), pieces);
                        }
                        Err(error) => physics_report.1.push(format!(
                            "{}: {error:#}; dynamic prop uses its render hull",
                            instance.model
                        )),
                    }
                }
                Ok(None) => report.collision_models_missing += 1,
                Err(error) => physics_report
                    .1
                    .push(format!("{}: {error:#}", instance.model)),
            }
        }
        report.instances_loaded += 1;
        if instance.entity.is_some() && !world.rigs.contains_key(&instance.asset_key()) {
            let mut wanted = [
                "ACT_IDLE",
                "idle_subtle",
                "idle_baton",
                "walk_all",
                "run_all",
                "idle",
                "idle01",
                "fire",
                "reload",
                "draw",
                "swing",
                "attack",
                "death1",
                // npc_combine_camera deploy/retire activities (matched lowercase).
                "act_combine_camera_open_idle",
                "act_combine_camera_closed_idle",
                "act_combine_camera_close",
            ]
            .into_iter()
            .map(String::from)
            .collect::<BTreeSet<_>>();
            for e in &world.entities {
                for key in [
                    "DefaultAnim",
                    "m_iszPlay",
                    "m_iszIdle",
                    "m_iszPostIdle",
                    "animation",
                ] {
                    if let Some(name) = e.get(key) {
                        wanted.insert(name.to_lowercase());
                    }
                }
            }
            match crate::animation::load(vfs, &instance.model, &wanted) {
                Ok(rig) => {
                    report.rigs_loaded += 1;
                    report.clips_loaded += rig.clips.len();
                    report
                        .animation_warnings
                        .extend(rig.warnings.iter().cloned());
                    world.rigs.insert(instance.asset_key(), rig);
                }
                Err(e) => report
                    .animation_warnings
                    .push(format!("{}: {e:#}", instance.model)),
            }
        }
        let illumination = *world
            .illumination
            .entry(instance.asset_key())
            .or_insert_with(|| read_illumination(vfs, &instance.model).unwrap_or_default());
        if instance.entity.is_some() {
            continue;
        }
        let rotate = rotation(instance.angles);
        // Static props without baked vertex lighting use the light cache at their
        // illumination origin; evaluate it once per vertex here.
        let light = world.lighting.as_deref().map(|data| {
            let origin = instance.origin + rotate * (illumination.position * instance.scale);
            let mut state = data.state_at(origin);
            if illumination.ambient_boost() {
                data.boost(&mut state, origin);
            }
            state
        });
        for surface in model {
            let out = batches
                .entry((
                    surface.material.clone(),
                    surface.lightmap,
                    instance.background,
                ))
                .or_insert_with(|| Surface {
                    flex_source: None,
                    background: instance.background,
                    material: surface.material.clone(),
                    lightmap: surface.lightmap,
                    vertices: Vec::new(),
                    indices: Vec::new(),
                });
            let base = out.vertices.len() as u32;
            out.vertices.extend(surface.vertices.iter().map(|v| {
                let position = instance.origin + rotate * (v.position * instance.scale);
                let normal = (rotate * v.normal).normalize_or_zero();
                Vertex {
                    position,
                    normal,
                    color: light
                        .as_ref()
                        .map_or(v.color, |state| lit_color(state, position, normal)),
                    ..v.clone()
                }
            }));
            out.indices.extend(surface.indices.iter().map(|i| base + i));
        }
    }
    world.surfaces = batches.into_values().collect();
    report.physics_solids_loaded = physics_report.0;
    report.surface_materials_loaded = world.surface_materials.len();
    report.collision_warnings.extend(physics_report.1);
    report
}
/// Linear vertex lighting as a gamma-encoded byte color (the world shader
/// multiplies gamma-space texels, then linearizes).
pub fn lit_color(
    state: &modkit_core::lighting::LightState,
    position: Vec3,
    normal: Vec3,
) -> [u8; 4] {
    let light = if normal == Vec3::ZERO {
        state.ambient.iter().copied().sum::<Vec3>() / 6.
    } else {
        state.vertex_light(position, normal, false)
    };
    let byte = |c: f32| (c.max(0.).powf(1. / 2.2) * 255.).round().min(255.) as u8;
    [byte(light.x), byte(light.y), byte(light.z), 255]
}
/// studiohdr_t illumposition (offset 92) and flags (offset 152).
pub fn read_illumination(
    vfs: &Vfs,
    model: &str,
) -> Result<modkit_core::lighting::ModelIllumination> {
    let mdl = vfs.read(model)?.context("model missing")?;
    if mdl.get(..4) != Some(b"IDST") {
        bail!("not a studio model");
    }
    Ok(modkit_core::lighting::ModelIllumination {
        position: crate::vec3(&mdl, 92)?,
        flags: u32le(&mdl, 152)?,
    })
}
/// The first .phy solid's parameters, plus its convex pieces when the single-solid
/// identity-root reader supports the file (the error otherwise).
#[allow(clippy::type_complexity)]
pub fn read_physics(
    vfs: &Vfs,
    model: &str,
) -> Result<
    Option<(
        modkit_core::PhysicsSolid,
        Result<Vec<modkit_core::ConvexPiece>>,
    )>,
> {
    let stem = model.trim_end_matches(".mdl");
    let Some(phy) = vfs.read(&format!("{stem}.phy"))? else {
        return Ok(None);
    };
    let entries = crate::phy::read_keyvalues(&phy)?;
    let solid = crate::phy::first_solid(&entries).context("PHY has no solid block")?;
    let pieces = (|| {
        let mdl = vfs.read(model)?.context("PHY companion MDL missing")?;
        let checksum = crate::phy::identity_root_checksum(&mdl)?;
        Ok(crate::phy::read(&phy, checksum)?.pieces)
    })();
    Ok(Some((solid, pieces)))
}
pub fn read_collision(vfs: &Vfs, model: &str) -> Result<Option<Vec<modkit_core::ConvexPiece>>> {
    let stem = model.trim_end_matches(".mdl");
    let Some(phy) = vfs.read(&format!("{stem}.phy"))? else {
        return Ok(None);
    };
    let mdl = vfs.read(model)?.context("PHY companion MDL missing")?;
    let checksum = crate::phy::identity_root_checksum(&mdl)?;
    Ok(Some(crate::phy::read(&phy, checksum)?.pieces))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn movement_model() -> Vec<u8> {
        let mut data = vec![0; 1024];
        data[..4].copy_from_slice(b"IDST");
        data[4..8].copy_from_slice(&48i32.to_le_bytes());
        data[76..80].copy_from_slice(&1024i32.to_le_bytes());
        for (at, value) in [(180, 2i32), (184, 344), (188, 1), (192, 544)] {
            data[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        for (at, fps, frames) in [(344, 20f32, 21i32), (444, 10., 11)] {
            data[at + 8..at + 12].copy_from_slice(&fps.to_le_bytes());
            data[at + 16..at + 20].copy_from_slice(&frames.to_le_bytes());
        }
        data[364..368].copy_from_slice(&2i32.to_le_bytes());
        data[368..372].copy_from_slice(&456i32.to_le_bytes());
        for (at, value) in [
            (548, 356i32),
            (552, 372),
            (556, 1),
            (560, -1),
            (564, 7),
            (600, 2),
            (604, 212),
            (612, 2),
            (616, 1),
        ] {
            data[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        data[758..760].copy_from_slice(&1i16.to_le_bytes());
        data[900..909].copy_from_slice(b"move_all\0");
        data[916..925].copy_from_slice(b"ACT_WALK\0");
        for (at, end, v0, v1, yaw, direction, position) in [
            (
                800,
                10i32,
                4f32,
                8f32,
                90f32,
                [1f32, 0., 0.],
                [6f32, 0., 0.],
            ),
            (844, 20, 8., 12., 180., [0., 1., 0.], [6., 10., 0.]),
        ] {
            data[at..at + 4].copy_from_slice(&end.to_le_bytes());
            data[at + 4..at + 8].copy_from_slice(&0xc0u32.to_le_bytes());
            for (i, value) in [v0, v1, yaw]
                .into_iter()
                .chain(direction)
                .chain(position)
                .enumerate()
            {
                data[at + 8 + i * 4..at + 12 + i * 4].copy_from_slice(&value.to_le_bytes());
            }
        }
        data
    }
    #[test]
    fn mdl_movement_preserves_activity_blends_and_animation_relative_records() {
        let mut result = BTreeMap::new();
        let includes = model_motion(
            &movement_model(),
            "synthetic.mdl",
            &BTreeSet::from(["move_all".into()]),
            &mut result,
            &mut MotionBudget::default(),
        )
        .unwrap();
        assert!(includes.is_empty());
        let sequence = &result["move_all"];
        assert_eq!(sequence.source_model, "synthetic.mdl");
        assert_eq!(sequence.activity_name, "ACT_WALK");
        assert_eq!(sequence.activity, -1);
        assert_eq!(sequence.activity_weight, 7);
        assert_eq!(sequence.flags, 1);
        assert_eq!(sequence.blend_dimensions, [2, 1]);
        assert_eq!(sequence.animation_indices, [0, 1]);
        assert_eq!(sequence.central_blend, 1);
        let track = &sequence.blends[0];
        assert_eq!(track.records.len(), 2);
        assert_eq!(track.records[1].end_frame, 20);
        assert_eq!(track.records[1].motion_flags, 0xc0);
        assert_eq!((track.records[1].v0, track.records[1].v1), (8., 12.));
        assert_eq!(track.records[1].end_yaw_degrees, 180.);
        assert_eq!(track.records[1].direction, Vec3::Y);
        assert_eq!(track.records[1].cumulative_position, Vec3::new(6., 10., 0.));
        assert_eq!(
            track.position(0.75).unwrap().position,
            Vec3::new(6., 4.5, 0.)
        );
        assert_eq!(
            sequence.blends[1].position(0.),
            Err(modkit_core::animation::RootMotionError::NoMovement)
        );
    }
    #[test]
    fn mdl_movement_rejects_truncation_relative_ranges_and_invalid_blends() {
        let data = movement_model();
        for end in 0..888 {
            assert!(
                animation_motion(&data[..end], 344, &mut MotionBudget::default()).is_err(),
                "accepted truncated movement at{end}"
            );
        }
        let parse = |data: &[u8]| {
            model_motion(
                data,
                "synthetic.mdl",
                &BTreeSet::from(["move_all".into()]),
                &mut BTreeMap::new(),
                &mut MotionBudget::default(),
            )
        };
        for (field, value) in [
            (4, 49i32),
            (368, i32::MAX),
            (758, -1),
            (612, 3),
            (844, 10),
            (844, 21),
        ] {
            let mut bad = data.clone();
            if field == 758 {
                bad[field..field + 2].copy_from_slice(&(value as i16).to_le_bytes());
            } else {
                bad[field..field + 4].copy_from_slice(&value.to_le_bytes());
            }
            assert!(parse(&bad).is_err(), "accepted bad movement field{field}");
        }
        let mut bad = data.clone();
        bad[852..856].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(parse(&bad).is_err());
    }
    #[test]
    #[ignore = "requires an installed owned HL2 copy; central blend metadata, not weighted NPC motion"]
    fn owned_walk_run_root_motion_comes_from_male_shared_records() {
        let game = crate::install::discover().unwrap();
        let vfs = Vfs::mount(&game).unwrap();
        let sequences = read_root_motion(
            &vfs,
            "models/barney.mdl",
            &BTreeSet::from(["walk_all".into(), "run_all".into()]),
        )
        .unwrap();
        for (name, activity, distance, duration) in [
            ("walk_all", "ACT_WALK", 80.00001f32, 1f32),
            ("run_all", "ACT_RUN", 125.87412, 0.6),
        ] {
            let sequence = &sequences[name];
            assert_eq!(sequence.source_model, "models/humans/male_shared.mdl");
            assert_eq!(sequence.activity_name, activity);
            assert_eq!(sequence.blend_dimensions, [9, 1]);
            assert_eq!(sequence.blends.len(), 9);
            assert_eq!(sequence.central_blend, 4);
            let track = &sequence.blends[4];
            let movement = track.movement(0., 1.).unwrap();
            assert!((movement.position - Vec3::X * distance).length() < 0.001);
            assert_eq!(movement.yaw_degrees, 0.);
            assert!((track.duration().unwrap() - duration).abs() < 1e-6);
            println!(
                "{name}: distance={} duration={} authored_blends={}",
                movement.position.x,
                track.duration().unwrap(),
                sequence.blends.len()
            );
        }
    }
    #[test]
    #[ignore = "requires an installed owned HL2 copy; reads prop_physics .phy solids"]
    fn owned_prop_physics_masses_and_surface_materials_are_plausible() {
        let game = crate::install::discover().unwrap();
        let vfs = Vfs::mount(&game).unwrap();
        let table = crate::surfaceprops::read(&vfs).unwrap();
        for name in ["default", "metal", "wood_crate", "concrete", "grenade"] {
            let m = table.get(name).unwrap_or_else(|| panic!("{name}"));
            println!(
                "{name}: friction {} elasticity {} density {} hard {:?} soft {:?}",
                m.friction, m.elasticity, m.density, m.impact_hard, m.impact_soft
            );
            assert!(m.friction > 0. && m.density > 0.);
        }
        let mut checked = 0;
        for map in ["d1_trainstation_02", "d1_trainstation_05", "d1_canals_01"] {
            let data = std::fs::read(game.join(format!("hl2/maps/{map}.bsp"))).unwrap();
            let bsp = crate::bsp::Bsp::parse(&data).unwrap();
            let entities = crate::keyvalues::entities(bsp.lump(0)).unwrap();
            let props: Vec<_> = entities
                .into_iter()
                .filter(|e| e.class().starts_with("prop_physics"))
                .collect();
            let mut world = World {
                entities: props,
                ..Default::default()
            };
            append_models(&mut world, &vfs);
            for (key, solid) in &world.model_physics {
                println!(
                    "{map} {key}: mass {} surfaceprop {} damping {} rotdamping {} inertia {} pieces {}",
                    solid.mass,
                    solid.surfaceprop,
                    solid.damping,
                    solid.rotdamping,
                    solid.inertia,
                    world.model_collision.get(key).map_or(0, Vec::len)
                );
                assert!(solid.mass > 0. && solid.mass < 5000., "{key}");
                assert!(table.contains_key(&solid.surfaceprop), "{key}");
                checked += 1;
            }
        }
        assert!(checked >= 3, "{checked}");
    }
    #[test]
    #[ignore = "requires an installed owned HL2 copy; reads installed Gman mesh only"]
    fn owned_cycler_actor_uses_installed_model_mesh() {
        let game = crate::install::discover().unwrap();
        let data = std::fs::read(game.join("hl2/maps/d1_trainstation_01.bsp")).unwrap();
        let bsp = crate::bsp::Bsp::parse(&data).unwrap();
        let entities = crate::keyvalues::entities(bsp.lump(0)).unwrap();
        let actor = entities
            .into_iter()
            .find(|e| e.get("targetname") == Some("gman"))
            .unwrap();
        assert_eq!(actor.class(), "cycler_actor");
        let origin = actor.origin();
        let mut world = World {
            entities: vec![actor],
            ..Default::default()
        };
        let vfs = Vfs::mount(&game).unwrap();
        let report = append_models(&mut world, &vfs);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(report.unique_models, 1);
        assert_eq!(world.model_instances.len(), 1);
        let instance = &world.model_instances[0];
        assert_eq!(instance.entity, Some(0));
        assert_eq!(instance.model, "models/gman_high.mdl");
        assert_eq!(instance.origin, origin);
        assert_eq!(instance.kind, "cycler_actor");
        let surfaces = &world.model_assets[&instance.asset_key()];
        assert!(surfaces.iter().map(|s| s.vertices.len()).sum::<usize>() > 0);
    }
    #[test]
    fn source_yaw_rotates_x_toward_y() {
        assert!((rotation(Vec3::new(0., 90., 0.)) * Vec3::X - Vec3::Y).length() < 0.0001);
    }
    #[test]
    #[ignore = "requires an installed owned HL2 copy; normal Barney factory model only"]
    fn owned_barney_without_model_key_loads_native_default_and_eye() {
        let game = crate::install::discover().unwrap();
        let data = std::fs::read(game.join("hl2/maps/d1_trainstation_01.bsp")).unwrap();
        let bsp = crate::bsp::Bsp::parse(&data).unwrap();
        let actor = crate::keyvalues::entities(bsp.lump(0))
            .unwrap()
            .into_iter()
            .find(|e| e.get("targetname") == Some("barney"))
            .unwrap();
        assert_eq!(actor.class(), "npc_barney");
        assert_eq!(actor.get("model"), None);
        let mut world = World {
            entities: vec![actor],
            ..Default::default()
        };
        let vfs = Vfs::mount(&game).unwrap();
        let report = append_models(&mut world, &vfs);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(world.model_instances.len(), 1);
        assert_eq!(world.model_instances[0].model, "models/barney.mdl");
        assert_eq!(
            read_eye_position(&vfs, "models/barney.mdl").unwrap(),
            Vec3::new(0., 0., 70.)
        );
        assert!(world.rigs["models/barney.mdl#0"]
            .clips
            .contains_key("idle_subtle"));
    }
    #[test]
    fn truncated_static_dictionary_is_rejected() {
        assert!(parse_static(&[1, 0, 0, 0], 6).is_err());
    }
    #[test]
    fn static_v6_extracts_dictionary_origin_and_skin() {
        let mut d = vec![0u8; 4 + 128 + 4 + 4 + 64];
        d[0] = 1;
        d[4..4 + 15].copy_from_slice(b"models/test.mdl");
        d[136] = 1;
        d[140..144].copy_from_slice(&42f32.to_le_bytes());
        d[172] = 2;
        let props = parse_static(&d, 6).unwrap();
        assert_eq!(props[0].model, "models/test.mdl");
        assert_eq!(props[0].origin.x, 42.);
        assert_eq!(props[0].skin, 2);
    }
    #[test]
    fn static_prop_solid_modes_are_preserved_in_every_supported_version() {
        for (version, stride) in [(4, 56), (5, 60), (6, 64)] {
            let mut data = vec![0u8; 140 + 3 * stride];
            data[0] = 1;
            data[4..19].copy_from_slice(b"models/test.mdl");
            data[136] = 3;
            for (i, mode) in [0, 2, 6].into_iter().enumerate() {
                data[140 + i * stride + 30] = mode;
            }
            let props = parse_static(&data, version).unwrap();
            assert_eq!(
                props.iter().map(|p| p.solid_mode).collect::<Vec<_>>(),
                vec![Some(0), Some(2), Some(6)]
            );
            assert_eq!(
                props.iter().map(|p| p.solid).collect::<Vec<_>>(),
                vec![false, true, true]
            );
        }
    }
}
