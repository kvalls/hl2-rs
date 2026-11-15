//! Installed v48 MDL movement metadata and per-clip root-motion decoding.

use crate::{bytes, f32le, i16le, i32le, u32le, vec3, vpk::Vfs};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

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
pub(super) struct MotionBudget {
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
pub(crate) fn clip_root_motion(
    data: &[u8],
    at: usize,
) -> Result<Option<modkit_core::animation::RootMotion>> {
    if u32le(data, 4)? != 48 {
        return Ok(None);
    }
    let mut budget = MotionBudget::default();
    let motion = animation_motion(data, at, &mut budget)?;
    Ok((!motion.records.is_empty()).then_some(motion))
}
pub(super) fn animation_motion(
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
pub(super) fn model_motion(
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
        let animations = AnimationTable {
            count: animation_count,
            base: animation_base,
        };
        let seq = sequence_motion(data, path, &name, id, at, animations, budget)?;
        result.insert(name, seq);
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
struct AnimationTable {
    count: usize,
    base: usize,
}
fn sequence_motion(
    data: &[u8],
    path: &str,
    name: &str,
    sequence_id: usize,
    at: usize,
    animations: AnimationTable,
    budget: &mut MotionBudget,
) -> Result<MotionSequence> {
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
        if index >= animations.count {
            bail!("movement animation index outside table: {path}:{name}");
        }
        animation_indices.push(index);
        blends.push(
            animation_motion(data, animations.base + index * 100, budget)
                .with_context(|| format!("{path}:{name} blend{blend}"))?,
        );
    }
    Ok(MotionSequence {
        source_model: path.into(),
        sequence_id,
        activity_name: motion_string(data, at, at + 8)?,
        activity: i32le(data, at + 16)?,
        activity_weight: i32le(data, at + 20)?,
        flags: u32le(data, at + 12)?,
        blend_dimensions: dimensions,
        animation_indices,
        blends,
        central_blend: count / 2,
    })
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
