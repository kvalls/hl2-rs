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
/// Studio header hull_min/hull_max (studiohdr_t offsets 104/116): the box
/// CreatePhysicsProp sweeps to place a prop.
pub fn read_hull(vfs: &Vfs, model: &str) -> Result<(Vec3, Vec3)> {
    let data = vfs
        .read(model)?
        .with_context(|| format!("missing model {model}"))?;
    eye_position(&data).with_context(|| format!("studio header: {model}"))?;
    let (mins, maxs) = (vec3(&data, 104)?, vec3(&data, 116)?);
    if !(mins.is_finite() && maxs.is_finite()) || mins.cmpgt(maxs).any() {
        bail!("invalid studio hull: {model}");
    }
    Ok((mins, maxs))
}
/// One mstudiobodyparts_t (studiohdr_t numbodyparts/bodypartindex at 232/236): its
/// name, model count and `base` (the body value stride of this group).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BodyPart {
    pub name: String,
    pub models: usize,
    pub base: i32,
}
pub fn body_parts(data: &[u8]) -> Result<Vec<BodyPart>> {
    let count = usize::try_from(i32le(data, 232)?)?;
    let index = usize::try_from(i32le(data, 236)?)?;
    if count > 64 {
        bail!("implausible body part count {count}");
    }
    (0..count)
        .map(|i| {
            let at = index + i * 16;
            let name_at = at
                .checked_add_signed(isize::try_from(i32le(data, at)?)?)
                .context("body part name offset")?;
            let name = data
                .get(name_at..)
                .context("body part name outside the file")?
                .split(|b| *b == 0)
                .next()
                .unwrap_or_default();
            Ok(BodyPart {
                name: String::from_utf8_lossy(name).into_owned(),
                models: usize::try_from(i32le(data, at + 4)?)?,
                base: i32le(data, at + 8)?,
            })
        })
        .collect()
}
/// One mstudiomodel_t's meshes as (material, vertex offset within the model) and its
/// first vertex (vertexindex / sizeof(mstudiovertex_t)). Read here because vmdl 0.2
/// steps submodels after the first with a wrong stride (a zombie's headcrab came out
/// empty); mstudiomodel_t is 148 bytes and mstudiomesh_t 116.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StudioSubmodel {
    pub vertex_offset: usize,
    pub meshes: Vec<(i32, usize)>,
}
pub fn studio_submodels(data: &[u8]) -> Result<Vec<Vec<StudioSubmodel>>> {
    let count = usize::try_from(i32le(data, 232)?)?;
    let index = usize::try_from(i32le(data, 236)?)?;
    if count > 64 {
        bail!("implausible body part count {count}");
    }
    let offset = |base: usize, relative: i32| {
        base.checked_add_signed(relative as isize)
            .context("studio offset out of range")
    };
    (0..count)
        .map(|i| {
            let part = index + i * 16;
            let models = usize::try_from(i32le(data, part + 4)?)?;
            if models > 256 {
                bail!("implausible submodel count {models}");
            }
            let first = offset(part, i32le(data, part + 12)?)?;
            (0..models)
                .map(|j| {
                    let model = first + j * 148;
                    let mesh_count = usize::try_from(i32le(data, model + 72)?)?;
                    if mesh_count > 4096 {
                        bail!("implausible mesh count {mesh_count}");
                    }
                    let mesh_first = offset(model, i32le(data, model + 76)?)?;
                    let vertex_index = usize::try_from(i32le(data, model + 84)?)?;
                    let meshes = (0..mesh_count)
                        .map(|k| {
                            let mesh = mesh_first + k * 116;
                            Ok((
                                i32le(data, mesh)?,
                                usize::try_from(i32le(data, mesh + 12)?)?,
                            ))
                        })
                        .collect::<Result<_>>()?;
                    Ok(StudioSubmodel {
                        vertex_offset: vertex_index / 48,
                        meshes,
                    })
                })
                .collect()
        })
        .collect()
}
/// An entity's starting body value: the `body` keyvalue (m_nBody), then the class's
/// Spawn bodygroups. Zombies wear their headcrab (CZombie/CFastZombie/CNPC_PoisonZombie
/// SetBodygroup(ZOMBIE_BODYGROUP_HEADCRAB = 1, !m_fIsHeadless)); the poison zombie also
/// enables its back crabs (ZOMBIE_BODYGROUP_NEST_BASE 2 + i) from `crabcount` (0 = all
/// three; one or two pick a mask, here by entity index instead of RandomInt).
pub fn entity_body(vfs: &Vfs, entity: &modkit_core::Entity, entity_id: usize, model: &str) -> i32 {
    let mut body = entity
        .get("body")
        .and_then(|b| b.trim().parse().ok())
        .unwrap_or(0);
    let class = entity.class();
    if !matches!(
        class,
        "npc_zombie"
            | "npc_zombie_torso"
            | "npc_fastzombie"
            | "npc_fastzombie_torso"
            | "npc_poisonzombie"
    ) {
        return body;
    }
    let Some(parts) = vfs
        .read(&model.replace('\\', "/"))
        .ok()
        .flatten()
        .and_then(|data| body_parts(&data).ok())
    else {
        return body;
    };
    body = set_bodygroup(&parts, body, 1, 1);
    if class == "npc_poisonzombie" {
        let crabs = entity
            .get("crabcount")
            .and_then(|c| c.trim().parse::<i32>().ok())
            .filter(|c| *c > 0)
            .unwrap_or(3)
            .min(3);
        let mask = match crabs {
            1 => [0x01, 0x02, 0x04][entity_id % 3],
            2 => [0x03, 0x05, 0x06][entity_id % 3],
            _ => 7,
        };
        for i in 0..3 {
            body = set_bodygroup(&parts, body, 2 + i, i32::from(mask & (1 << i) != 0));
        }
    }
    body
}
/// CBaseAnimating::SetBodygroup -> ::SetBodygroup: the body value with `group` set to
/// `value` (unknown groups or values leave `body` unchanged).
pub fn set_bodygroup(parts: &[BodyPart], body: i32, group: usize, value: i32) -> i32 {
    let Some(part) = parts.get(group) else {
        return body;
    };
    set_body_value((part.models, part.base), body, value)
}
/// ::SetBodygroup for one body part given as (submodel count, base).
pub fn set_body_value((models, base): (usize, i32), body: i32, value: i32) -> i32 {
    let models = models as i32;
    if value < 0 || value >= models || base <= 0 {
        return body;
    }
    let current = (body / base) % models;
    body - current * base + value * base
}
/// ::GetBodygroup for one body part given as (submodel count, base).
pub fn body_value((models, base): (usize, i32), body: i32) -> i32 {
    if models <= 1 || base <= 0 {
        0
    } else {
        (body / base).rem_euclid(models as i32)
    }
}
/// Classes whose AI calls SetBodygroup after spawning, and the groups it changes:
/// CNPC_PoisonZombie's head crab (ReleaseHeadcrab's fRemoveHead, group 1), nest crabs
/// (ZOMBIE_BODYGROUP_NEST_BASE 2..4) and the crab in its hand (ZOMBIE_BODYGROUP_THROW 5).
pub const RUNTIME_BODYGROUPS: &[(&str, &[usize])] = &[("npc_poisonzombie", &[1, 2, 3, 4, 5])];
/// Every body value reachable from `body` by setting `groups` (each to any submodel).
pub fn body_variants(parts: &[(usize, i32)], body: i32, groups: &[usize]) -> Vec<i32> {
    let mut bodies = vec![body];
    for &group in groups {
        let Some(&part) = parts.get(group) else {
            continue;
        };
        bodies = bodies
            .iter()
            .flat_map(|&b| (0..part.0 as i32).map(move |v| set_body_value(part, b, v)))
            .collect();
    }
    bodies.sort_unstable();
    bodies.dedup();
    bodies
}
/// Load the body variants of instances whose class changes bodygroups at runtime
/// (RUNTIME_BODYGROUPS) into `model_assets` with their body part table, at map load so the
/// frame systems never read models.
fn append_runtime_body_variants(world: &mut World, vfs: &Vfs, report: &mut ModelReport) {
    let mut wanted = Vec::new();
    for instance in &world.model_instances {
        let Some(class) = instance
            .entity
            .and_then(|id| world.entities.get(id))
            .map(|e| e.class())
        else {
            continue;
        };
        let Some((_, groups)) = RUNTIME_BODYGROUPS.iter().find(|(c, _)| *c == class) else {
            continue;
        };
        wanted.push((instance.clone(), *groups));
    }
    for (instance, groups) in wanted {
        let path = instance.model.to_lowercase();
        if !world.model_body_parts.contains_key(&path) {
            let parts = match vfs.read(&path).ok().flatten().map(|d| body_parts(&d)) {
                Some(Ok(parts)) => parts,
                _ => {
                    report
                        .errors
                        .push(format!("{path}: no body parts for runtime bodygroups"));
                    continue;
                }
            };
            world.model_body_parts.insert(
                path.clone(),
                parts.iter().map(|p| (p.models, p.base)).collect(),
            );
        }
        let parts = world.model_body_parts[&path].clone();
        for body in body_variants(&parts, instance.body, groups) {
            let variant = ModelInstance {
                body,
                ..instance.clone()
            };
            let key = variant.asset_key();
            if world.model_assets.contains_key(&key) {
                continue;
            }
            match read_model_body(vfs, &path, instance.skin, body) {
                Ok(surfaces) => {
                    world.model_assets.insert(key, surfaces);
                }
                Err(e) => report.errors.push(format!("{key}: {e:#}")),
            }
        }
    }
}
/// The submodel a body value selects in each body part (::GetBodygroup).
pub fn body_selection(parts: &[BodyPart], body: i32) -> Vec<usize> {
    parts
        .iter()
        .map(|p| {
            if p.models <= 1 || p.base <= 0 {
                0
            } else {
                ((body / p.base) % p.models as i32).max(0) as usize
            }
        })
        .collect()
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
            body: 0,
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
    read_model_body(vfs, path, skin, 0)
}
/// The model's meshes for a skin and a body value (each body part draws the submodel
/// the body selects; body 0 is every part's first submodel).
pub fn read_model_body(vfs: &Vfs, path: &str, skin: usize, body: i32) -> Result<Vec<Surface>> {
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
    let selection = if body == 0 {
        Vec::new()
    } else {
        body_selection(&body_parts(&mdl_bytes)?, body)
    };
    let submodels = studio_submodels(&mdl_bytes)?;
    let mut surfaces = Vec::new();
    for (body_index, (part, topology)) in submodels.iter().zip(&vtx.body_parts).enumerate() {
        let model_index = selection.get(body_index).copied().unwrap_or(0);
        let Some(model) = part.get(model_index) else {
            continue;
        };
        let lod = topology
            .models
            .get(model_index)
            .and_then(|m| m.lods.first())
            .context("missing model LOD")?;
        for (mesh_index, (&(mesh_material, mesh_vertex_offset), topology)) in
            model.meshes.iter().zip(&lod.meshes).enumerate()
        {
            let material = usize::try_from(mesh_material)?;
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
            let base = model
                .vertex_offset
                .checked_add(mesh_vertex_offset)
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
                    model: model_index,
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
    let prop_data = vfs
        .read("scripts/propdata.txt")
        .ok()
        .flatten()
        .and_then(|data| crate::keyvalues::parse(&String::from_utf8_lossy(&data)).ok())
        .unwrap_or_default();
    world.breakable_chunks = breakable_chunks(&prop_data);
    match crate::surfaceprops::read(vfs) {
        Ok(table) => world.surface_materials = table,
        Err(error) => physics_report
            .1
            .push(format!("surfaceproperties: {error:#}")),
    }
    // VBSP gives each world collision triangle its brush side's $surfaceprop
    // (ivp.cpp ConvertWorldBrushesToPhysCollide); resolve each side material once.
    let side_materials: std::collections::BTreeSet<String> = world
        .brush_side_materials
        .iter()
        .flatten()
        .filter(|m| !m.is_empty())
        .cloned()
        .collect();
    for material in side_materials {
        if let Ok(Some(prop)) = vfs.material_value(&material, "$surfaceprop") {
            world
                .material_surfaceprops
                .insert(material, prop.trim().to_lowercase());
        }
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
                    | "prop_ragdoll"
                    | "raggib"
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
                body: entity_body(vfs, entity, entity_id, model),
                scale: entity
                    .get("modelscale")
                    .and_then(|s| s.parse::<f32>().ok())
                    .filter(|s| s.is_finite() && *s > 0.)
                    .unwrap_or(1.),
                kind: entity.class().into(),
                // A prop_ragdoll collides through its ragdoll solids, not a model collider.
                solid: !entity.class().starts_with("item_")
                    && !entity.class().starts_with("weapon_")
                    && entity.class() != "prop_ragdoll"
                    // CRagGib::Spawn: FSOLID_NOT_SOLID, then a client ragdoll.
                    && entity.class() != "raggib"
                    && entity.get("solid").is_none_or(|s| s != "0"),
                solid_mode: None,
                entity: Some(entity_id),
            });
        }
    }
    let mut cache: BTreeMap<(String, usize, i32), Option<Vec<Surface>>> = BTreeMap::new();
    let mut report = ModelReport::default();
    let mut collision_attempted = BTreeSet::new();
    let mut ragdolls_read = BTreeSet::new();
    let mut collision_modes_reported = BTreeSet::new();
    let mut batches: BTreeMap<(String, Option<usize>, bool), Surface> = world
        .surfaces
        .drain(..)
        .map(|s| ((s.material.clone(), s.lightmap, s.background), s))
        .collect();
    for instance in &world.model_instances {
        let key = (instance.model.to_lowercase(), instance.skin, instance.body);
        let model = cache.entry(key.clone()).or_insert_with(|| {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                read_model_body(vfs, &key.0, key.1, key.2)
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
        // CAI_BaseNPC::SetupVPhysicsHull sets the shadow's mass from the studio header.
        if instance.entity.is_some_and(|id| {
            world
                .entities
                .get(id)
                .is_some_and(|e| e.class().starts_with("npc_"))
        }) && !world.model_mass.contains_key(&instance.asset_key())
        {
            if let Some(mass) = vfs
                .read(&instance.model)
                .ok()
                .flatten()
                .and_then(|mdl| studio_mass(&mdl).ok())
            {
                world.model_mass.insert(instance.asset_key(), mass);
            }
        }
        // Breakable props: prop_data health from the MDL keyvalues.
        if instance.kind.starts_with("prop_physics")
            && !world.model_prop_health.contains_key(&instance.asset_key())
        {
            if let Some(data) = vfs
                .read(&instance.model)
                .ok()
                .flatten()
                .and_then(|mdl| studio_keyvalues(&mdl).ok().flatten())
                .and_then(|text| resolve_prop_data(&prop_data, &text))
            {
                let health = data.health;
                world.model_prop_health.insert(instance.asset_key(), health);
                world.model_prop_data.insert(instance.asset_key(), data);
                // PropBreakableCreateAll's gib list (BuildGibList: the .phy break
                // blocks); `append_break_physics` preloads the gibs' physics.
                if health > 0. {
                    let phy = format!("{}.phy", instance.model.trim_end_matches(".mdl"));
                    let gibs = vfs
                        .read(&phy)
                        .ok()
                        .flatten()
                        .and_then(|data| crate::phy::read_keyvalues(&data).ok())
                        .map(|entries| crate::phy::break_models(&entries))
                        .unwrap_or_default();
                    if !gibs.is_empty() {
                        world.model_breaks.insert(instance.asset_key(), gibs);
                    }
                }
            }
        }
        // VPhysics props (CPhysicsProp::CreateVPhysics): authored solid parameters and
        // the authored convex pieces as stored (PhysModelCreate: entity space, any root).
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
                // Weapon-translated metrocop idles (weapon_pistol/smg1 acttables).
                "ACT_IDLE_PISTOL",
                "ACT_IDLE_SMG1",
                // Stunstick acttable results for the metrocop shove/baton chain.
                "ACT_MELEE_ATTACK_SWING",
                "ACT_IDLE_ANGRY_MELEE",
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
            // npc_metropolice AI activities (hl2-simulation ai::metropolice/police and the
            // weapon acttables): the cloud AI branch requested them at runtime only, so
            // their sequences were never loaded and every task using them failed.
            if instance.kind == "npc_metropolice" {
                wanted.extend(METROPOLICE_AI_ACTIVITIES.iter().map(|a| a.to_lowercase()));
            }
            if matches!(
                instance.kind.as_str(),
                "npc_citizen" | "npc_barney" | "npc_alyx" | "npc_combine_s"
            ) {
                wanted.extend(HUMANOID_AI_ACTIVITIES.iter().map(|a| a.to_lowercase()));
            }
            if instance.kind.starts_with("npc_") {
                wanted.extend(NPC_BASE_AI_ACTIVITIES.iter().map(|a| a.to_lowercase()));
            }
            if matches!(instance.kind.as_str(), "npc_zombie" | "npc_zombie_torso") {
                wanted.extend(ZOMBIE_AI_ACTIVITIES.iter().map(|a| a.to_lowercase()));
            }
            // CBlackHeadcrab::NPC_TranslateActivity's panic scurry.
            if matches!(
                instance.kind.as_str(),
                "npc_headcrab_black" | "npc_headcrab_poison"
            ) {
                wanted.insert("act_blackheadcrab_run_panic".into());
            }
            // SCHED_ZOMBIE_POISON_RANGE_ATTACK2's threat sequence.
            if instance.kind == "npc_poisonzombie" {
                wanted.insert("act_zombie_poison_threat".into());
            }
            // A raggib ragdolls from its sequence 0 (cycle 0).
            if instance.kind == "raggib" {
                if let Ok(Some(first)) = crate::animation::first_sequence_name(vfs, &instance.model)
                {
                    wanted.insert(first.to_lowercase());
                }
            }
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
        // NPC deaths become ragdolls (C_BaseAnimating::BecomeRagdollOnClient); map
        // prop_ragdolls are ragdolls from the start (CRagdollProp::Spawn). Another instance
        // of the model (a prop_physics mattress) may have loaded the rig first.
        if (instance.kind.starts_with("npc_")
            || instance.kind == "prop_ragdoll"
            || instance.kind == "raggib")
            && !ragdolls_read.contains(&instance.asset_key())
        {
            ragdolls_read.insert(instance.asset_key());
            if let Some(rig) = world.rigs.get(&instance.asset_key()) {
                match read_ragdoll(vfs, &instance.model, rig) {
                    Ok(Some(ragdoll)) => {
                        world
                            .model_ragdolls
                            .insert(instance.asset_key(), std::sync::Arc::new(ragdoll));
                    }
                    Ok(None) => {}
                    Err(e) => report
                        .animation_warnings
                        .push(format!("{} ragdoll: {e:#}", instance.model)),
                }
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
    append_runtime_body_variants(world, vfs, &mut report);
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
/// Preloads every listed gib model's .phy (skin-0 key) so breaking a prop reads nothing.
/// Gibs the PHY adapter cannot read (e.g. the cardboard_box00*_gib01 non-identity roots)
/// are left out without a load warning: they are only needed when a prop breaks, where
/// the spawn falls back to the render hull. Returns the models left out.
pub fn append_break_physics(world: &mut World, vfs: &Vfs) -> Vec<String> {
    let mut skipped = Vec::new();
    let models: std::collections::BTreeSet<String> = world
        .model_breaks
        .values()
        .flatten()
        .map(|gib| gib.model.clone())
        .chain(used_chunk_models(world))
        .collect();
    for model in models {
        if world.model_physics.contains_key(&format!("{model}#0")) {
            continue;
        }
        if append_model_physics(world, vfs, &model).is_err() {
            skipped.push(model);
        }
    }
    skipped
}
/// Chunk models of the propdata chunk sets that breakable props without .phy gibs use
/// (PropBreakableCreateAll's else-branch; ParsePropDataFile precaches every chunk model).
pub fn used_chunk_models(world: &World) -> Vec<String> {
    let mut models = Vec::new();
    for (key, data) in &world.model_prop_data {
        if data.health <= 0. || data.breakable_count <= 0 || world.model_breaks.contains_key(key) {
            continue;
        }
        let Some(name) = &data.breakable_model else {
            continue;
        };
        // GetRandomChunkModel: the first set whose name starts with the requested one.
        if let Some((_, set)) = world
            .breakable_chunks
            .iter()
            .find(|(set, _)| set.starts_with(name.as_str()))
        {
            models.extend(set.iter().cloned());
        }
    }
    models
}
/// Adds a non-map model's .phy solid (and supported pieces) to the world under its
/// skin-0 asset key, e.g. a thrown frag's w_grenade.phy. Returns the reason on failure.
/// Rigs of models that have no map instance (NPC weapon world models: their sequences'
/// events drive NPC firing, DESIGN 14w), with every activity's sequence, keyed like
/// instances (`model#0`). Returns the models that failed to load.
pub fn append_rigs(world: &mut World, vfs: &Vfs, models: &[String]) -> Vec<String> {
    let mut failed = Vec::new();
    for model in models {
        let key = format!("{}#0", model.to_lowercase());
        if world.rigs.contains_key(&key) {
            continue;
        }
        let loaded = crate::animation::load(vfs, model, &Default::default()).and_then(|first| {
            let wanted = first.activities.iter().map(|a| a.to_lowercase()).collect();
            crate::animation::load(vfs, model, &wanted)
        });
        match loaded {
            Ok(rig) => {
                world.rigs.insert(key, rig);
            }
            Err(e) => failed.push(format!("{model}: {e:#}")),
        }
    }
    failed
}
pub fn append_model_physics(world: &mut World, vfs: &Vfs, model: &str) -> Result<()> {
    let key = format!("{}#0", model.to_lowercase());
    let (solid, pieces) = read_physics(vfs, model)?.context("model has no .phy")?;
    world.model_physics.insert(key.clone(), solid);
    world.model_collision.insert(key, pieces?);
    Ok(())
}
/// studiohdr_t keyvalueindex/keyvaluesize (offsets 312/316 in MDL 44-49): the model's
/// $keyvalues text (prop_data, physgun_interactions, ...).
/// Activities the metrocop AI sets (class schedules, policing, flinches, melee) and
/// their stunstick/pistol/SMG1 acttable translations.
/// (DESIGN 14w adds the pistol, SMG1 and stunstick acttable results.)
pub const METROPOLICE_AI_ACTIVITIES: &[&str] = &[
    "ACT_PUSH_PLAYER",
    "ACT_MELEE_ATTACK1",
    "ACT_MELEE_ATTACK_SWING",
    "ACT_ACTIVATE_BATON",
    "ACT_DEACTIVATE_BATON",
    "ACT_POLICE_HARASS1",
    "ACT_POLICE_HARASS2",
    "ACT_IDLE_ANGRY",
    "ACT_IDLE_ANGRY_MELEE",
    "ACT_IDLE_ANGRY_PISTOL",
    "ACT_IDLE_ANGRY_SMG1",
    "ACT_WALK",
    "ACT_RUN",
    "ACT_WALK_PISTOL",
    "ACT_RUN_PISTOL",
    "ACT_WALK_RIFLE",
    "ACT_RUN_RIFLE",
    "ACT_WALK_ANGRY",
    "ACT_SMALL_FLINCH",
    "ACT_BIG_FLINCH",
    "ACT_FLINCH_PHYSICS",
    "ACT_COWER",
    "ACT_RANGE_ATTACK_PISTOL",
    "ACT_RANGE_ATTACK_SMG1",
    "ACT_GESTURE_RANGE_ATTACK_PISTOL",
    "ACT_GESTURE_RANGE_ATTACK_SMG1",
    "ACT_RELOAD_PISTOL",
    "ACT_RELOAD_SMG1",
    "ACT_DUCK_DODGE",
    "ACT_TO_COMBAT_SOUND",
    "ACT_COVER_PISTOL_LOW",
    "ACT_COVER_SMG1_LOW",
    "ACT_GESTURE_RELOAD_PISTOL",
    "ACT_GESTURE_RELOAD_SMG1",
    "ACT_IDLE_PISTOL",
    "ACT_IDLE_SMG1",
    "ACT_IDLE_SMG1_RELAXED",
    "ACT_IDLE_SMG1_STIMULATED",
    "ACT_RANGE_AIM_PISTOL_LOW",
    "ACT_RANGE_AIM_SMG1_LOW",
    "ACT_RANGE_ATTACK1",
    "ACT_RANGE_ATTACK_SMG1_LOW",
    "ACT_RELOAD",
    "ACT_RELOAD_PISTOL_LOW",
    "ACT_RELOAD_SMG1_LOW",
    "ACT_RUN_AIM_PISTOL",
    "ACT_RUN_AIM_RIFLE",
    "ACT_RUN_CROUCH_AIM_RIFLE",
    "ACT_RUN_CROUCH_RIFLE",
    "ACT_RUN_RIFLE_RELAXED",
    "ACT_RUN_RIFLE_STIMULATED",
    "ACT_WALK_AIM_PISTOL",
    "ACT_WALK_AIM_RIFLE",
    "ACT_WALK_CROUCH_AIM_RIFLE",
    "ACT_WALK_CROUCH_RIFLE",
    "ACT_WALK_RIFLE_RELAXED",
    "ACT_WALK_RIFLE_STIMULATED",
];
/// Activities every npc_* class may set through the shared AI (the base schedules and
/// tasks, flinches, natural attacks, deaths, jumps and leaps), loaded for every NPC
/// model so classes added later find their sequences; classes add their own lists.
pub const NPC_BASE_AI_ACTIVITIES: &[&str] = &[
    "ACT_IDLE",
    // CAI_ActBusyBehavior's standing busy hint (scripts/actbusy.txt ACT_BUSY_STAND).
    "ACT_BUSY_STAND",
    // CNPC_BaseZombie::NPC_TranslateActivity while burning.
    "ACT_WALK_ON_FIRE",
    "ACT_IDLE_ON_FIRE",
    "ACT_IDLE_ANGRY",
    "ACT_WALK",
    "ACT_RUN",
    "ACT_TURN_LEFT",
    "ACT_TURN_RIGHT",
    "ACT_MELEE_ATTACK1",
    "ACT_MELEE_ATTACK2",
    "ACT_RANGE_ATTACK1",
    "ACT_RANGE_ATTACK2",
    "ACT_RELOAD",
    "ACT_SMALL_FLINCH",
    "ACT_BIG_FLINCH",
    "ACT_FLINCH_PHYSICS",
    "ACT_COWER",
    "ACT_DIESIMPLE",
    "ACT_DIEBACKWARD",
    "ACT_DIEFORWARD",
    "ACT_DIEVIOLENT",
    // CAI_BaseNPC::SelectDeathPose (the client ragdoll's initial bone velocities).
    "ACT_DIE_FRONTSIDE",
    "ACT_DIE_BACKSIDE",
    "ACT_DIE_LEFTSIDE",
    "ACT_DIE_RIGHTSIDE",
    "ACT_JUMP",
    "ACT_GLIDE",
    "ACT_LAND",
    "ACT_HOP",
    "ACT_LEAP",
    "ACT_CLIMB_UP",
    "ACT_CLIMB_DOWN",
    "ACT_CLIMB_DISMOUNT",
];
/// CNPC_BaseZombie's class activities (DECLARE_ACTIVITY): GetSwatActivity's physics swats,
/// for the zombies that can swat (CanSwatPhysicsObjects: not the fast zombie).
pub const ZOMBIE_AI_ACTIVITIES: &[&str] = &[
    "ACT_ZOM_SWATLEFTMID",
    "ACT_ZOM_SWATRIGHTMID",
    "ACT_ZOM_SWATLEFTLOW",
    "ACT_ZOM_SWATRIGHTLOW",
    // SCHED_ZOMBIE_CHARGE_ENEMY's frustration and SCHED_ZOMBIE_BASH_DOOR (CZombie).
    "ACT_ZOMBIE_TANTRUM",
    "ACT_ZOMBIE_WALLPOUND",
];
/// Activities the humanoid AI classes (npc_citizen, npc_barney, npc_alyx, npc_combine_s;
/// hl2-simulation ai::humanoid and the base schedules) set, and every acttable result of
/// the weapons they carry (SMG1, AR2, shotgun, pistol, RPG, Alyx gun, crowbar; a test in
/// hl2-simulation keeps this in sync with its SDK acttables).
pub const HUMANOID_AI_ACTIVITIES: &[&str] = &[
    "ACT_IDLE_ANGRY",
    "ACT_WALK",
    "ACT_RUN",
    "ACT_WALK_AIM",
    "ACT_RUN_AIM",
    "ACT_RANGE_ATTACK1",
    "ACT_RANGE_ATTACK2",
    "ACT_COMBINE_THROW_GRENADE",
    "ACT_COMBINE_AR2_ALTFIRE",
    "ACT_RELOAD",
    "ACT_SMALL_FLINCH",
    "ACT_BIG_FLINCH",
    "ACT_FLINCH_PHYSICS",
    "ACT_COWER",
    "ACT_COVER_LOW",
    "ACT_RUN_PROTECTED",
    "ACT_TURN_LEFT",
    "ACT_TURN_RIGHT",
    "ACT_MELEE_ATTACK1",
    "ACT_COVER_LOW_RPG",
    "ACT_COVER_PISTOL_LOW",
    "ACT_COVER_SMG1_LOW",
    "ACT_CROUCHIDLE_STIMULATED",
    "ACT_GESTURE_RANGE_ATTACK_AR2",
    "ACT_GESTURE_RANGE_ATTACK_PISTOL",
    "ACT_GESTURE_RANGE_ATTACK_SHOTGUN",
    "ACT_GESTURE_RANGE_ATTACK_SMG1",
    "ACT_GESTURE_RELOAD_PISTOL",
    "ACT_GESTURE_RELOAD_SHOTGUN",
    "ACT_GESTURE_RELOAD_SMG1",
    "ACT_IDLE_AIM_RIFLE_STIMULATED",
    "ACT_IDLE_ANGRY_MELEE",
    "ACT_IDLE_ANGRY_PISTOL",
    "ACT_IDLE_ANGRY_RPG",
    "ACT_IDLE_ANGRY_SHOTGUN",
    "ACT_IDLE_ANGRY_SMG1",
    "ACT_IDLE_PISTOL",
    "ACT_IDLE_RPG",
    "ACT_IDLE_RPG_RELAXED",
    "ACT_IDLE_SHOTGUN_AGITATED",
    "ACT_IDLE_SHOTGUN_RELAXED",
    "ACT_IDLE_SHOTGUN_STIMULATED",
    "ACT_IDLE_SMG1",
    "ACT_IDLE_SMG1_RELAXED",
    "ACT_IDLE_SMG1_STIMULATED",
    "ACT_IDLE_STEALTH_PISTOL",
    "ACT_IDLE_STIMULATED",
    "ACT_MELEE_ATTACK_SWING",
    "ACT_RANGE_AIM_AR2_LOW",
    "ACT_RANGE_AIM_PISTOL_LOW",
    "ACT_RANGE_AIM_SMG1_LOW",
    "ACT_RANGE_ATTACK_AR2",
    "ACT_RANGE_ATTACK_PISTOL",
    "ACT_RANGE_ATTACK_PISTOL_LOW",
    "ACT_RANGE_ATTACK_RPG",
    "ACT_RANGE_ATTACK_SHOTGUN",
    "ACT_RANGE_ATTACK_SHOTGUN_LOW",
    "ACT_RANGE_ATTACK_SMG1",
    "ACT_RANGE_ATTACK_SMG1_LOW",
    "ACT_READINESS_PISTOL_AGITATED_TO_STIMULATED",
    "ACT_READINESS_PISTOL_RELAXED_TO_STIMULATED",
    "ACT_READINESS_PISTOL_RELAXED_TO_STIMULATED_WALK",
    "ACT_READINESS_PISTOL_STIMULATED_TO_RELAXED",
    "ACT_RELOAD_PISTOL",
    "ACT_RELOAD_PISTOL_LOW",
    "ACT_RELOAD_SHOTGUN",
    "ACT_RELOAD_SHOTGUN_LOW",
    "ACT_RELOAD_SMG1",
    "ACT_RELOAD_SMG1_LOW",
    "ACT_RUN_AIM_PISTOL",
    "ACT_RUN_AIM_RIFLE",
    "ACT_RUN_AIM_RIFLE_STIMULATED",
    "ACT_RUN_AIM_SHOTGUN",
    "ACT_RUN_AIM_STEALTH_PISTOL",
    "ACT_RUN_CROUCH_AIM_RIFLE",
    "ACT_RUN_CROUCH_RIFLE",
    "ACT_RUN_CROUCH_RPG",
    "ACT_RUN_PISTOL",
    "ACT_RUN_RIFLE",
    "ACT_RUN_RIFLE_RELAXED",
    "ACT_RUN_RIFLE_STIMULATED",
    "ACT_RUN_RPG",
    "ACT_RUN_STEALTH_PISTOL",
    "ACT_RUN_STIMULATED",
    "ACT_WALK_AIM_PISTOL",
    "ACT_WALK_AIM_RIFLE",
    "ACT_WALK_AIM_RIFLE_STIMULATED",
    "ACT_WALK_AIM_SHOTGUN",
    "ACT_WALK_AIM_STEALTH_PISTOL",
    "ACT_WALK_CROUCH_AIM_RIFLE",
    "ACT_WALK_CROUCH_RIFLE",
    "ACT_WALK_CROUCH_RPG",
    "ACT_WALK_PISTOL",
    "ACT_WALK_RIFLE",
    "ACT_WALK_RIFLE_RELAXED",
    "ACT_WALK_RIFLE_STIMULATED",
    "ACT_WALK_RPG",
    "ACT_WALK_STEALTH_PISTOL",
    "ACT_WALK_STIMULATED",
];
pub fn studio_keyvalues(mdl: &[u8]) -> Result<Option<String>> {
    if bytes(mdl, 0, 4)? != b"IDST" {
        bail!("model magic mismatch");
    }
    let (index, size) = (u32le(mdl, 312)? as usize, u32le(mdl, 316)? as usize);
    if index == 0 || size == 0 {
        return Ok(None);
    }
    let text = bytes(mdl, index, size)?;
    let end = text.iter().position(|b| *b == 0).unwrap_or(text.len());
    Ok(Some(String::from_utf8_lossy(&text[..end]).into_owned()))
}
/// CPropData::ParsePropFromKV health: the "base" entry of scripts/propdata.txt first
/// (recursively), then the block's own "health". None when the model has no prop_data.
pub fn prop_data_health(
    prop_data: &[crate::keyvalues::Entry],
    model_keyvalues: &str,
) -> Option<f32> {
    resolve_prop_data(prop_data, model_keyvalues).map(|data| data.health)
}
/// CPropData::ParsePropFromKV: the model's prop_data block over its propdata.txt "base"
/// chain (the base is parsed first, then the block's own keys override). None when the
/// model has no prop_data.
pub fn resolve_prop_data(
    prop_data: &[crate::keyvalues::Entry],
    model_keyvalues: &str,
) -> Option<modkit_core::PropData> {
    let entries = crate::keyvalues::parse(model_keyvalues).ok()?;
    // The studio keyvalue text wraps its blocks in "mdlkeyvalue { ... }".
    let block = entries
        .iter()
        .flat_map(|e| std::iter::once(e).chain(e.children()))
        .find(|e| e.key.eq_ignore_ascii_case("prop_data"))?;
    // The model's interaction sections are the prop_data block's siblings.
    let siblings: Vec<crate::keyvalues::Entry> = entries
        .iter()
        .find(|e| e.children().iter().any(|c| std::ptr::eq(c, block)))
        .map_or_else(|| entries.clone(), |parent| parent.children().to_vec());
    fn section<'a>(
        entries: &'a [crate::keyvalues::Entry],
        name: &str,
    ) -> Option<&'a crate::keyvalues::Entry> {
        entries.iter().find_map(|e| {
            if e.key.eq_ignore_ascii_case(name) && !e.children().is_empty() {
                Some(e)
            } else if e.key.eq_ignore_ascii_case("BreakableModels") {
                None
            } else {
                section(e.children(), name)
            }
        })
    }
    fn apply(
        prop_data: &[crate::keyvalues::Entry],
        block: &crate::keyvalues::Entry,
        interactions: &[crate::keyvalues::Entry],
        depth: usize,
        data: &mut modkit_core::PropData,
    ) {
        if let Some(base) = block
            .get("base")
            .and_then(|b| b.text())
            .filter(|_| depth < 8)
            .and_then(|base| section(prop_data, base))
        {
            // ParsePropFromBase: a base's interactions are inside its own section.
            apply(prop_data, base, base.children(), depth + 1, data);
        }
        // sPropdataInteractionSections: a key set to exactly this value sets the flag.
        let interaction = |section: &str, key: &str, value: &str| {
            interactions
                .iter()
                .find(|e| e.key.eq_ignore_ascii_case(section))
                .and_then(|e| e.get(key))
                .and_then(|e| e.text())
                .is_some_and(|v| v.trim().eq_ignore_ascii_case(value))
        };
        data.flammable |= interaction("fire_interactions", "flammable", "yes");
        data.explosive_resist |= interaction("fire_interactions", "explosive_resist", "yes");
        data.ignite_halfhealth |= interaction("fire_interactions", "ignite", "halfhealth");
        data.break_explode |= interaction("physgun_interactions", "onbreak", "explode_fire");
        let text = |key: &str| {
            block
                .children()
                .iter()
                .find(|e| e.key.eq_ignore_ascii_case(key))
                .and_then(|e| e.text())
                .map(str::trim)
        };
        let number = |key: &str| text(key).and_then(|v| v.parse::<f32>().ok());
        if let Some(v) = number("health") {
            data.health = v;
        }
        if let Some(v) = number("dmg.bullets") {
            data.dmg_bullets = v;
        }
        if let Some(v) = number("dmg.club") {
            data.dmg_club = v;
        }
        if let Some(v) = number("dmg.explosive") {
            data.dmg_explosive = v;
        }
        if let Some(v) = number("explosive_damage") {
            data.explosive_damage = Some(v);
        }
        if let Some(v) = number("explosive_radius") {
            data.explosive_radius = Some(v);
        }
        if let Some(v) = text("damage_table") {
            data.damage_table = (!v.is_empty()).then(|| v.to_string());
        }
        if let Some(v) = text("breakable_model") {
            data.breakable_model = (!v.is_empty()).then(|| v.to_string());
        }
        // GetInt: atoi of the text.
        if let Some(v) = number("breakable_count") {
            data.breakable_count = v as i32;
        }
        if let Some(v) = number("breakable_skin") {
            data.breakable_skin = v as i32;
        }
    }
    let mut data = modkit_core::PropData::default();
    apply(prop_data, block, &siblings, 0, &mut data);
    Some(data)
}
/// CPropData::ParsePropDataFile: the "BreakableModels" chunk sets in file order, model
/// names normalized like gib models.
pub fn breakable_chunks(prop_data: &[crate::keyvalues::Entry]) -> Vec<(String, Vec<String>)> {
    fn find(entries: &[crate::keyvalues::Entry]) -> Option<&crate::keyvalues::Entry> {
        entries.iter().find_map(|e| {
            if e.key.eq_ignore_ascii_case("BreakableModels") {
                Some(e)
            } else {
                find(e.children())
            }
        })
    }
    find(prop_data)
        .map(|section| {
            section
                .children()
                .iter()
                .map(|set| {
                    (
                        set.key.clone(),
                        set.children()
                            .iter()
                            .map(|model| crate::phy::fixup_model_name(&model.key))
                            .collect(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}
/// studiohdr_t::mass (offset 328 in MDL 44-49; owned police.mdl 90, combine_soldier
/// 100, wood_crate001a 30).
pub fn studio_mass(mdl: &[u8]) -> Result<f32> {
    if bytes(mdl, 0, 4)? != b"IDST" {
        bail!("model magic mismatch");
    }
    let mass = f32le(mdl, 328)?;
    if !mass.is_finite() || mass < 0. {
        bail!("invalid studio mass");
    }
    Ok(mass)
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
    let mut solid = crate::phy::first_solid(&entries).context("PHY has no solid block")?;
    let pieces = (|| {
        let mdl = vfs.read(model)?.context("PHY companion MDL missing")?;
        let checksum = crate::phy::studio_checksum(&mdl)?;
        let read = crate::phy::read(&phy, checksum)?;
        solid.compact_inertia = Some((read.mass_center, read.rotation_inertia));
        solid.drag_fractions = Some(read.drag_fractions);
        Ok(read.pieces)
    })();
    Ok(Some((solid, pieces)))
}
/// The model's ragdoll (`phy::ragdoll_model`) against its loaded rig's bones.
pub fn read_ragdoll(
    vfs: &Vfs,
    model: &str,
    rig: &modkit_core::animation::Rig,
) -> Result<Option<modkit_core::RagdollModel>> {
    let stem = model.trim_end_matches(".mdl");
    let Some(phy) = vfs.read(&format!("{stem}.phy"))? else {
        return Ok(None);
    };
    let mdl = vfs.read(model)?.context("PHY companion MDL missing")?;
    // studiohdr_t checksum (the PHY header repeats it); ragdolls have many root bones.
    let checksum = crate::u32le(&mdl, 8)?;
    let bones: Vec<_> = rig
        .bones
        .iter()
        .map(|b| (b.name.clone(), b.inverse_bind.inverse()))
        .collect();
    crate::phy::ragdoll_model(&phy, checksum, &bones)
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
    #[test]
    #[ignore = "reads the installed game (HL2_ROOT)"]
    fn weapon_solids_match_their_idle_pose_not_the_bind_mesh() {
        // Dropped NPC weapons: rotated root bones; the solid is used as stored.
        let game = crate::install::discover().unwrap();
        let vfs = Vfs::mount(&game).unwrap();
        let bounds = |points: &mut dyn Iterator<Item = Vec3>| {
            points.fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)))
        };
        for model in [
            "models/weapons/w_stunbaton.mdl",
            "models/weapons/w_pistol.mdl",
            "models/weapons/w_irifle.mdl",
        ] {
            let (_, pieces) = read_physics(&vfs, model).unwrap().unwrap();
            let pieces = pieces.unwrap();
            let solid = bounds(&mut pieces.iter().flat_map(|p| p.vertices.iter().copied()));
            let rig = crate::animation::load(&vfs, model, &["idle".into(), "idle1".into()].into())
                .unwrap();
            let clip = rig.clips.keys().next().unwrap().clone();
            let skin = rig.matrices(&clip, 0.);
            let surfaces = read_model(&vfs, model, 0).unwrap();
            let vertices = || surfaces.iter().flat_map(|s| s.vertices.iter());
            let idle = bounds(&mut vertices().map(|v| {
                let bone = v.skin.as_ref().map_or(0, |w| usize::from(w.bones[0]));
                skin[bone].transform_point3(v.position)
            }));
            let bind = bounds(&mut vertices().map(|v| v.position));
            let error = |(lo, hi): (Vec3, Vec3)| {
                (lo - solid.0)
                    .abs()
                    .max_element()
                    .max((hi - solid.1).abs().max_element())
            };
            assert!(error(idle) < 1., "{model}: idle {idle:?} solid {solid:?}");
            assert!(error(bind) > 5., "{model}: bind {bind:?} solid {solid:?}");
        }
    }
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
#[cfg(test)]
mod owned_phy_inertia {
    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn owned_frag_phy_reads_its_ivp_inertia() {
        let root = crate::install::discover().unwrap();
        let vfs = crate::vpk::Vfs::mount(&root).unwrap();
        let (solid, pieces) = super::read_physics(&vfs, "models/weapons/w_grenade.mdl")
            .unwrap()
            .unwrap();
        // One 12-sided prism (24 rim vertices + 2 cap tips), radius 1.808.
        assert_eq!(pieces.unwrap()[0].vertices.len(), 26);
        let (center, inertia) = solid.compact_inertia.unwrap();
        assert!(center.length() < 0.1, "{center}");
        // Long (Source z) axis 0.000711 m^2/kg, cross axes 0.00428.
        assert!((inertia.z - 0.000711).abs() < 1e-5, "{inertia}");
        assert!((inertia.x - 0.004283).abs() < 1e-5 && (inertia.y - 0.004283).abs() < 1e-5);
    }
    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn owned_crate_phy_lists_its_gibs() {
        let root = crate::install::discover().unwrap();
        let vfs = crate::vpk::Vfs::mount(&root).unwrap();
        let phy = vfs
            .read("models/props_junk/wood_crate001a.phy")
            .unwrap()
            .unwrap();
        let gibs = crate::phy::break_models(&crate::phy::read_keyvalues(&phy).unwrap());
        assert!(!gibs.is_empty());
        assert!(gibs
            .iter()
            .all(|g| g.model.starts_with("models/") && g.model.ends_with(".mdl")));
        for gib in &gibs {
            assert!(vfs.read(&gib.model).unwrap().is_some(), "{}", gib.model);
        }
    }
    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn owned_pallet_phy_stores_its_drag_fractions() {
        let root = crate::install::discover().unwrap();
        let vfs = crate::vpk::Vfs::mount(&root).unwrap();
        let (solid, _) = super::read_physics(&vfs, "models/props_junk/wood_pallet001a.mdl")
            .unwrap()
            .unwrap();
        // VPHY header words (private drag-axes research): the slatted pallet's gaps
        // show in the middle fraction, unlike a convex silhouette (~1, 1, 1).
        let f = solid.drag_fractions.unwrap();
        let expected = glam::Vec3::new(0.941_666_6, 0.402_616_3, 0.993_643_4);
        assert!((f - expected).abs().max_element() < 1e-6, "{f}");
        assert_eq!(solid.mass, 20.);
    }
}
#[cfg(test)]
mod owned_prop_data {
    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn owned_prop_data_health_matches_native_ent_text() {
        let vfs = crate::vpk::Vfs::mount(&crate::install::discover().unwrap()).unwrap();
        let prop_data = crate::keyvalues::parse(&String::from_utf8_lossy(
            &vfs.read("scripts/propdata.txt").unwrap().unwrap(),
        ))
        .unwrap();
        // Native ent_text (oracle, session 13): Cardboard.small 10, Wooden.Large 50.
        for (model, expected) in [
            ("models/props_junk/cardboard_box001a.mdl", Some(10.)),
            ("models/props_junk/wood_pallet001a.mdl", Some(50.)),
        ] {
            let mdl = vfs.read(model).unwrap().unwrap();
            let text = super::studio_keyvalues(&mdl).unwrap().unwrap();
            assert_eq!(
                super::prop_data_health(&prop_data, &text),
                expected,
                "{model}"
            );
        }
    }
    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn owned_prop_data_modifiers_tables_and_chunk_sets() {
        let vfs = crate::vpk::Vfs::mount(&crate::install::discover().unwrap()).unwrap();
        let prop_data = crate::keyvalues::parse(&String::from_utf8_lossy(
            &vfs.read("scripts/propdata.txt").unwrap().unwrap(),
        ))
        .unwrap();
        let data = |model: &str| {
            let mdl = vfs.read(model).unwrap().unwrap();
            let text = super::studio_keyvalues(&mdl).unwrap().unwrap();
            super::resolve_prop_data(&prop_data, &text).unwrap()
        };
        // Wooden.Large over Wooden.Base: health 50, bullets 0.75, club 2, explosive 1.5,
        // WoodChunks x 6 (used only without .phy gibs).
        let pallet = data("models/props_junk/wood_pallet001a.mdl");
        assert_eq!(
            (
                pallet.health,
                pallet.dmg_bullets,
                pallet.dmg_club,
                pallet.dmg_explosive
            ),
            (50., 0.75, 2., 1.5)
        );
        assert_eq!(pallet.breakable_model.as_deref(), Some("WoodChunks"));
        for (model, flags) in [
            (
                "models/props_c17/oildrum001_explosive.mdl",
                (true, true, true),
            ),
            ("models/props_junk/gascan001a.mdl", (true, false, false)),
            (
                "models/props_junk/wood_pallet001a.mdl",
                (false, false, false),
            ),
            ("models/props_c17/oildrum001.mdl", (false, false, false)),
        ] {
            // fire_interactions (flammable, explosive_resist, ignite halfhealth).
            let d = data(model);
            assert_eq!(
                (d.flammable, d.explosive_resist, d.ignite_halfhealth),
                flags,
                "{model}"
            );
        }
        // The gas can: a small blast with PropaneTank.Burst (onbreak explode_fire).
        let gas = data("models/props_junk/gascan001a.mdl");
        assert_eq!(
            (
                gas.explosive_damage,
                gas.explosive_radius,
                gas.break_explode
            ),
            (Some(25.), Some(80.), true)
        );
        assert!(!data("models/props_c17/oildrum001_explosive.mdl").break_explode);
        let propane = data("models/props_junk/propane_tank001a.mdl");
        assert_eq!(
            (
                propane.explosive_damage,
                propane.explosive_radius,
                propane.break_explode,
                propane.flammable
            ),
            (Some(1.), Some(50.), true, true)
        );
        assert_eq!((pallet.breakable_count, pallet.breakable_skin), (6, 0));
        assert_eq!(pallet.damage_table, None);
        // Cardboard.small: bullets 0.5, no chunk set.
        let box_ = data("models/props_junk/cardboard_box001a.mdl");
        assert_eq!((box_.health, box_.dmg_bullets), (10., 0.5));
        assert_eq!(box_.breakable_model, None);
        // glass.Small bottles use the glass impact table.
        let bottle = data("models/props_junk/garbage_glassbottle003a.mdl");
        assert_eq!(bottle.damage_table.as_deref(), Some("glass"));
        // The tic-tac-toe blocks override health 0 (unbreakable); furniture_couch01a
        // (Wooden.Huge, no .phy gibs, tr03) breaks into 10 WoodChunks.
        let block = data("models/props_c17/playgroundtick-tack-toe_block01a.mdl");
        assert_eq!((block.health, block.breakable_count), (0., 2));
        let couch = data("models/props_interiors/furniture_couch01a.mdl");
        assert_eq!((couch.health, couch.breakable_count), (130., 10));
        let chunks = super::breakable_chunks(&prop_data);
        let names: Vec<&str> = chunks.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            ["WoodChunks", "GlassChunks", "ConcreteChunks", "MetalChunks"]
        );
        assert_eq!(chunks[0].1[0], "models/gibs/wood_gib01e.mdl");
        assert_eq!(chunks[0].1.len(), 5);
    }
}

#[cfg(test)]
mod body_tests {
    use super::*;
    fn parts() -> Vec<BodyPart> {
        // models/zombie/poison.mdl's layout (studio, headcrab1..headcrab5).
        let mut parts = vec![BodyPart {
            name: "studio".into(),
            models: 1,
            base: 1,
        }];
        for i in 0..5 {
            parts.push(BodyPart {
                name: format!("headcrab{}", i + 1),
                models: 2,
                base: 1 << i,
            });
        }
        parts
    }
    #[test]
    fn bodygroups_set_and_select_submodels() {
        let p = parts();
        let body = set_bodygroup(&p, 0, 1, 1);
        assert_eq!(body, 1);
        let body = (2..5).fold(body, |b, g| set_bodygroup(&p, b, g, 1));
        assert_eq!(body, 15);
        assert_eq!(body_selection(&p, body), vec![0, 1, 1, 1, 1, 0]);
        assert_eq!(set_bodygroup(&p, body, 3, 0), 11);
        // Out-of-range groups and values leave the body alone.
        assert_eq!(set_bodygroup(&p, body, 9, 1), 15);
        assert_eq!(set_bodygroup(&p, body, 1, 2), 15);
    }
    #[test]
    fn poison_zombie_runtime_bodies_cover_the_nest_and_the_hand() {
        let p: Vec<_> = parts().iter().map(|p| (p.models, p.base)).collect();
        // Spawn: the head crab and all three nest crabs (15); the AI toggles 2..5.
        let bodies = body_variants(&p, 15, &[2, 3, 4, 5]);
        assert_eq!(bodies.len(), 16);
        assert!(bodies.iter().all(|b| b & 1 == 1));
        assert_eq!((bodies[0], bodies[15]), (1, 31));
        assert_eq!(body_value(p[5], set_body_value(p[5], 15, 1)), 1);
        assert_eq!(body_value(p[3], 15), 1);
        assert_eq!(body_value(p[3], 11), 0);
    }
    #[test]
    #[ignore = "requires owned HL2 installation"]
    fn owned_poison_zombie_runtime_variants_differ_by_their_crabs() {
        let vfs = crate::vpk::Vfs::mount(std::path::Path::new(
            &std::env::var("HL2_ROOT").expect("set HL2_ROOT"),
        ))
        .unwrap();
        let mut world = World::default();
        world.entities.push(modkit_core::Entity {
            properties: vec![
                ("classname".into(), "npc_poisonzombie".into()),
                ("model".into(), "models/zombie/poison.mdl".into()),
            ],
        });
        append_models(&mut world, &vfs);
        let parts = &world.model_body_parts["models/zombie/poison.mdl"];
        assert_eq!(parts.len(), 6);
        let triangles = |body: i32| {
            let key = if body == 0 {
                "models/zombie/poison.mdl#0".to_owned()
            } else {
                format!("models/zombie/poison.mdl#0@{body}")
            };
            world.model_assets[&key]
                .iter()
                .map(|s| s.indices.len() / 3)
                .sum::<usize>()
        };
        // Spawn body 15 (head crab + three nest crabs) and its 31 runtime variants.
        let bodies = body_variants(parts, 15, RUNTIME_BODYGROUPS[0].1);
        assert_eq!(bodies.len(), 32);
        for body in &bodies {
            assert!(triangles(*body) > 0, "{body}");
        }
        // Each nest crab and the crab in hand adds geometry.
        assert!(triangles(15) > triangles(13));
        assert!(triangles(13) > triangles(9));
        assert!(triangles(31) > triangles(15));
        // Headless (ReleaseHeadcrab removes the head crab).
        assert!(triangles(15) > triangles(14));
        println!(
            "triangles 1 {} 15 {} 31 {}",
            triangles(1),
            triangles(15),
            triangles(31)
        );
    }
    #[test]
    #[ignore = "requires owned HL2 installation"]
    fn owned_zombie_headcrab_bodygroup_adds_the_crab_mesh() {
        let vfs = crate::vpk::Vfs::mount(std::path::Path::new(
            &std::env::var("HL2_ROOT").expect("set HL2_ROOT"),
        ))
        .unwrap();
        let triangles = |path: &str, body| {
            read_model_body(&vfs, path, 0, body)
                .unwrap()
                .iter()
                .map(|s| s.indices.len() / 3)
                .sum::<usize>()
        };
        for path in [
            "models/zombie/classic.mdl",
            "models/zombie/classic_torso.mdl",
            "models/zombie/fast.mdl",
            "models/zombie/poison.mdl",
        ] {
            let data = vfs.read(path).unwrap().unwrap();
            let parts = body_parts(&data).unwrap();
            assert_eq!(parts[1].name, "headcrab1", "{path}");
            // vmdl agrees on the first submodel; later ones come from the raw records.
            let mut geometry = data.clone();
            geometry[180..184].copy_from_slice(&0i32.to_le_bytes());
            let mdl = vmdl::Mdl::read(&geometry).unwrap();
            let raw = studio_submodels(&data).unwrap();
            let first = &mdl.body_parts[0].models[0];
            assert_eq!(raw[0][0].vertex_offset, first.vertex_offset as usize);
            assert!(
                raw[1][0].meshes.is_empty() && raw[1][1].meshes.len() == 1,
                "{path}"
            );
            assert!(triangles(path, 1) > triangles(path, 0), "{path}");
        }
    }
}
