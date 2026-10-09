//! Bounded Source MDL/ANI sequence reader. Blends, IK and flexes are separate work.
use crate::{bytes, f32le, i16le, i32le, u16le, u32le, vec3, vpk::Vfs};
use anyhow::{bail, Context, Result};
use glam::{Mat4, Quat, Vec3};
use modkit_core::animation::{
    sequence_flags, AutoLayer, BlendAxis, BlendGrid, Bone, Clip, ClipLayer, Faceposer, Pose,
    PoseParameter, Rig,
};
use std::collections::BTreeSet;
fn offset(data: &[u8], field: usize) -> Result<usize> {
    Ok(usize::try_from(i32le(data, field)?)?)
}
fn relative(data: &[u8], base: usize, field: usize) -> Result<usize> {
    Ok(usize::try_from(base as i64 + i32le(data, field)? as i64)?)
}
fn string(data: &[u8], at: usize) -> Result<String> {
    let tail = data.get(at..).context("string outside MDL")?;
    let end = tail
        .iter()
        .take(4096)
        .position(|b| *b == 0)
        .context("unterminated MDL string")?;
    Ok(std::str::from_utf8(&tail[..end])?.into())
}
struct SourceBone {
    bone: Bone,
    euler: Vec3,
    position_scale: Vec3,
    rotation_scale: Vec3,
}
fn bones(data: &[u8]) -> Result<Vec<SourceBone>> {
    let count = offset(data, 156)?;
    if count > 256 {
        bail!("MDL skeleton exceeds 256 bones");
    }
    let base = offset(data, 160)?;
    bytes(data, base, count * 216)?;
    (0..count)
        .map(|id| {
            let at = base + id * 216;
            let parent = i32le(data, at + 4)?;
            if parent >= id as i32 {
                bail!("unordered/cyclic skeleton");
            }
            let q = Quat::from_xyzw(
                f32le(data, at + 44)?,
                f32le(data, at + 48)?,
                f32le(data, at + 52)?,
                f32le(data, at + 56)?,
            )
            .normalize();
            let mut m = [0.; 16];
            for row in 0..3 {
                for col in 0..4 {
                    m[col * 4 + row] = f32le(data, at + 96 + (row * 4 + col) * 4)?;
                }
            }
            m[15] = 1.;
            Ok(SourceBone {
                bone: Bone {
                    name: string(data, relative(data, at, at)?)?,
                    parent: usize::try_from(parent).ok(),
                    bind: Pose {
                        position: vec3(data, at + 32)?,
                        rotation: q,
                    },
                    inverse_bind: Mat4::from_cols_array(&m),
                },
                euler: vec3(data, at + 60)?,
                position_scale: vec3(data, at + 72)?,
                rotation_scale: vec3(data, at + 84)?,
            })
        })
        .collect()
}
fn rle(data: &[u8], mut at: usize, mut frame: usize) -> Result<f32> {
    for _ in 0..4096 {
        let header = bytes(data, at, 2)?;
        let valid = header[0] as usize;
        let total = header[1] as usize;
        if total == 0 || valid == 0 || valid > total {
            bail!("invalid animation RLE span");
        }
        if frame < total {
            return Ok(i16le(data, at + 2 * (frame.min(valid - 1) + 1))? as f32);
        }
        frame -= total;
        at += 2 * (valid + 1);
    }
    bail!("animation RLE span budget exceeded")
}
fn values(data: &[u8], at: usize, frame: usize) -> Result<Vec3> {
    let mut out = [0.; 3];
    for (axis, value) in out.iter_mut().enumerate() {
        let off = i16le(data, at + axis * 2)?;
        if off > 0 {
            *value = rle(data, at + off as usize, frame)?;
        }
    }
    Ok(Vec3::from_array(out))
}
fn quaternion(data: &[u8], at: usize, wide: bool) -> Result<Quat> {
    let (x, y, z, negative) = if wide {
        let q = u64::from_le_bytes(bytes(data, at, 8)?.try_into()?);
        let value = |shift: u32| (((q >> shift) & 0x1fffffu64) as f32 - 1048576.) / 1048576.5;
        (value(0), value(21), value(42), q >> 63 != 0)
    } else {
        let z = u16le(data, at + 4)?;
        (
            (u16le(data, at)? as f32 - 32768.) / 32768.,
            (u16le(data, at + 2)? as f32 - 32768.) / 32768.,
            ((z & 0x7fff) as f32 - 16384.) / 16384.,
            z & 0x8000 != 0,
        )
    };
    let w = (1. - x * x - y * y - z * z).max(0.).sqrt() * if negative { -1. } else { 1. };
    Ok(Quat::from_xyzw(x, y, z, w).normalize())
}
fn euler(v: Vec3) -> Quat {
    Quat::from_rotation_z(v.z) * Quat::from_rotation_y(v.y) * Quat::from_rotation_x(v.x)
}
/// Pose of a bone without animation data: identity offsets for delta animations.
fn default_pose(bone: &Bone, delta: bool) -> Pose {
    if delta {
        Pose {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
        }
    } else {
        bone.bind.clone()
    }
}
/// Values follow SDK CalcBoneQuaternion/CalcBonePosition: delta records stay raw offsets.
fn frame(
    data: &[u8],
    mut at: usize,
    bones: &[SourceBone],
    frame: usize,
    delta_animation: bool,
) -> Result<Vec<Pose>> {
    let mut poses = bones
        .iter()
        .map(|b| default_pose(&b.bone, delta_animation))
        .collect::<Vec<_>>();
    for _ in 0..256 {
        let header = bytes(data, at, 4)?;
        let id = header[0] as usize;
        if id == 255 {
            break;
        }
        let bone = bones.get(id).context("animation bone outside skeleton")?;
        let flags = header[1];
        if flags & !0x3f != 0 {
            bail!("unsupported compressed animation flags {flags:#x}");
        }
        let delta = flags & 16 != 0;
        let raw_rotation_bytes = if flags & 2 != 0 {
            6
        } else if flags & 32 != 0 {
            8
        } else {
            0
        };
        let rotation = if raw_rotation_bytes > 0 {
            quaternion(data, at + 4, raw_rotation_bytes == 8)?
        } else if flags & 8 != 0 {
            let angles = values(data, at + 4, frame)? * bone.rotation_scale
                + if delta { Vec3::ZERO } else { bone.euler };
            euler(angles)
        } else if delta {
            Quat::IDENTITY
        } else {
            bone.bone.bind.rotation
        };
        let position = if flags & 1 != 0 {
            let at = at + 4 + raw_rotation_bytes;
            Vec3::new(
                half::f16::from_bits(u16le(data, at)?).to_f32(),
                half::f16::from_bits(u16le(data, at + 2)?).to_f32(),
                half::f16::from_bits(u16le(data, at + 4)?).to_f32(),
            )
        } else if flags & 4 != 0 {
            values(data, at + 4 + if flags & 8 != 0 { 6 } else { 0 }, frame)? * bone.position_scale
                + if delta {
                    Vec3::ZERO
                } else {
                    bone.bone.bind.position
                }
        } else if delta {
            Vec3::ZERO
        } else {
            bone.bone.bind.position
        };
        poses[id] = Pose { position, rotation };
        let next = i16le(data, at + 2)?;
        if next == 0 {
            return Ok(poses);
        }
        if next < 4 {
            bail!("invalid animation record chain");
        }
        at += next as usize;
    }
    Ok(poses)
}
fn clip(
    vfs: &Vfs,
    mdl: &[u8],
    ani: Option<&[u8]>,
    bones: &[SourceBone],
    animation: usize,
    looping: bool,
) -> Result<Clip> {
    let count = offset(mdl, 180)?;
    if animation >= count {
        bail!("sequence animation outside table");
    }
    let at = offset(mdl, 184)? + animation * 100;
    bytes(mdl, at, 100)?;
    let count = offset(mdl, at + 16)?;
    if count == 0 || count > 4096 {
        bail!("animation exceeds frame limit");
    }
    let fps = f32le(mdl, at + 8)?.clamp(1., 240.);
    let animation_flags = u32le(mdl, at + 12)?;
    let delta_animation = animation_flags & sequence_flags::DELTA != 0;
    let section_frames = offset(mdl, at + 84)?;
    let section_base = relative(mdl, at, at + 80)?;
    let mut frames = Vec::with_capacity(count);
    if animation_flags & sequence_flags::ALLZEROS != 0 {
        // No authored data: every frame is the base (or identity delta) pose.
        let pose = bones
            .iter()
            .map(|b| default_pose(&b.bone, delta_animation))
            .collect::<Vec<_>>();
        frames.resize(count, pose);
    }
    let authored = if frames.is_empty() { count } else { 0 };
    for f in 0..authored {
        let (mut block, mut index) = (i32le(mdl, at + 52)?, i32le(mdl, at + 56)?);
        let mut local_frame = f;
        if section_frames > 0 {
            let section = if count > section_frames && f == count - 1 {
                local_frame = 0;
                count / section_frames + 1
            } else {
                local_frame = f % section_frames;
                f / section_frames
            };
            block = i32le(mdl, section_base + section * 8)?;
            index = i32le(mdl, section_base + section * 8 + 4)?;
        }
        if block < 0 {
            bail!("unavailable animation block");
        }
        let (data, start) = if block == 0 {
            (mdl, usize::try_from(at as i64 + index as i64)?)
        } else {
            let block = block as usize;
            if block >= offset(mdl, 352)? {
                bail!("ANI block outside table");
            }
            let record = offset(mdl, 356)? + block * 8;
            let start = offset(mdl, record)?;
            let end = offset(mdl, record + 4)?;
            let data = bytes(
                ani.context("ANI companion missing")?,
                start,
                end.checked_sub(start).context("reversed ANI block")?,
            )?;
            (data, usize::try_from(index)?)
        };
        frames.push(frame(data, start, bones, local_frame, delta_animation)?);
    }
    let _ = vfs;
    Ok(Clip {
        events: Vec::new(),
        fps,
        looping,
        frames,
        layer: ClipLayer {
            all_zeros: animation_flags & sequence_flags::ALLZEROS != 0,
            ..ClipLayer::default()
        },
    })
}
pub fn load(vfs: &Vfs, path: &str, wanted: &BTreeSet<String>) -> Result<Rig> {
    let wanted = wanted
        .iter()
        .map(|s| s.to_lowercase())
        .collect::<BTreeSet<_>>();
    let data = vfs.read(path)?.context("MDL missing for animation")?;
    let base = bones(&data)?;
    let mut rig = Rig {
        bones: base.iter().map(|b| b.bone.clone()).collect(),
        ..Default::default()
    };
    let mut state = LoadState::default();
    load_sequences(vfs, path, &data, &wanted, &mut rig, &mut state, 0)?;
    match crate::eyes::read_attachments(&data) {
        Ok(attachments) => {
            rig.attachments = attachments
                .into_iter()
                .map(|(name, a)| modkit_core::animation::Attachment {
                    name,
                    bone: a.bone,
                    local: a.local,
                })
                .collect();
        }
        Err(e) => rig.warnings.push(format!("{path}: attachments: {e:#}")),
    }
    Ok(rig)
}
fn sequence_events(
    data: &[u8],
    sequence: usize,
) -> Result<(Vec<modkit_core::animation::ClipEvent>, usize)> {
    let count = offset(data, sequence + 24)?;
    if count > 4096 {
        bail!("sequence event limit");
    }
    if count == 0 {
        return Ok((Vec::new(), 0));
    }
    let base = relative(data, sequence, sequence + 28)?;
    bytes(data, base, count * 80)?;
    let mut events = Vec::new();
    let mut discarded = 0;
    for i in 0..count {
        let at = base + i * 80;
        let cycle = f32::from_le_bytes(bytes(data, at, 4)?.try_into()?);
        // Some shipped unused one-frame sequences contain NaN event cycles. Never schedule those.
        if !cycle.is_finite() || !(0. ..=1.).contains(&cycle) {
            discarded += 1;
            continue;
        }
        let options = bytes(data, at + 12, 64)?;
        let end = options.iter().position(|b| *b == 0).unwrap_or(64);
        let name = if i32le(data, at + 76)? != 0 {
            string(data, relative(data, at, at + 76)?)?
        } else {
            String::new()
        };
        events.push(modkit_core::animation::ClipEvent {
            cycle,
            id: i32le(data, at + 4)?,
            flags: u32le(data, at + 8)?,
            name,
            options: std::str::from_utf8(&options[..end])?.into(),
        });
    }
    Ok((events, discarded))
}
fn sequence_metadata(data: &[u8], at: usize) -> Result<modkit_core::animation::Sequence> {
    bytes(data, at, 212)?;
    let activity = if i32le(data, at + 8)? == 0 {
        String::new()
    } else {
        string(data, relative(data, at, at + 8)?)?
    };
    Ok(modkit_core::animation::Sequence {
        name: string(data, relative(data, at, at + 4)?)?.to_lowercase(),
        activity,
        weight: i32le(data, at + 20)?,
        order: 0,
    })
}
/// mstudioposeparamdesc_t table (header 300/304, stride 20).
fn pose_parameters(data: &[u8]) -> Result<Vec<PoseParameter>> {
    let count = offset(data, 300)?;
    if count > 64 {
        bail!("pose parameter table exceeds limit");
    }
    let base = offset(data, 304)?;
    bytes(data, base, count * 20)?;
    (0..count)
        .map(|i| {
            let at = base + i * 20;
            Ok(PoseParameter {
                name: string(data, relative(data, at, at)?)?,
                start: f32le(data, at + 8)?,
                end: f32le(data, at + 12)?,
                looping: f32le(data, at + 16)?,
            })
        })
        .collect()
}
/// All blend animations of a sequence with its pose-parameter axes (seqdesc 56..100).
fn blend_grid(
    vfs: &Vfs,
    data: &[u8],
    ani: Option<&[u8]>,
    source_bones: &[SourceBone],
    at: usize,
    param_map: &[usize],
    looping: bool,
) -> Result<BlendGrid> {
    let blends = offset(data, at + 56)?;
    let groups = [offset(data, at + 68)?.max(1), offset(data, at + 72)?.max(1)];
    if blends > 64 || groups[0] * groups[1] != blends {
        bail!("sequence blend grid does not match its groups");
    }
    let table = relative(data, at, at + 60)?;
    let mut axes = [None, None];
    for (k, axis) in axes.iter_mut().enumerate() {
        let index = i32le(data, at + 76 + k * 4)?;
        if index >= 0 {
            let parameter = *param_map
                .get(index as usize)
                .context("blend pose parameter outside model table")?;
            *axis = Some(BlendAxis {
                parameter,
                start: f32le(data, at + 84 + k * 4)?,
                end: f32le(data, at + 92 + k * 4)?,
            });
        }
    }
    let mut grid = BlendGrid {
        axes,
        groups,
        ..Default::default()
    };
    for b in 0..blends {
        let animation = usize::try_from(i16le(data, table + b * 2)?)?;
        let clip = clip(vfs, data, ani, source_bones, animation, looping)?;
        grid.all_zeros.push(clip.layer.all_zeros);
        grid.anims.push(clip.frames);
    }
    Ok(grid)
}
fn sequence_label(data: &[u8], sequences: usize, base: usize, id: usize) -> Result<String> {
    if id >= sequences {
        bail!("autolayer sequence outside table");
    }
    let at = base + id * 212;
    Ok(string(data, relative(data, at, at + 4)?)?.to_lowercase())
}
/// Raw autolayer; `child` is relative to the sequence's own model file (iRelativeSeq).
struct RawAutoLayer {
    child: usize,
    pose: i16,
    flags: u32,
    ramp: [f32; 4],
}
fn autolayer_records(data: &[u8], at: usize) -> Result<Vec<RawAutoLayer>> {
    let count = offset(data, at + 148)?;
    if count > 64 {
        bail!("autolayer count exceeds limit");
    }
    if count == 0 {
        return Ok(Vec::new());
    }
    let base = relative(data, at, at + 152)?;
    bytes(data, base, count * 24)?;
    (0..count)
        .map(|i| {
            let at = base + i * 24;
            Ok(RawAutoLayer {
                child: usize::try_from(i16le(data, at)?).context("negative autolayer sequence")?,
                pose: i16le(data, at + 2)?,
                flags: u32le(data, at + 4)?,
                ramp: [
                    f32le(data, at + 8)?,
                    f32le(data, at + 12)?,
                    f32le(data, at + 16)?,
                    f32le(data, at + 20)?,
                ],
            })
        })
        .collect()
}
/// `mdlkeyvalue { faceposer { ... } }` from the sequence keyvalue text. Text that does
/// not parse is treated as absent, like a failed KeyValues load.
fn sequence_faceposer(data: &[u8], at: usize) -> Result<Option<Faceposer>> {
    let size = offset(data, at + 176)?;
    if size == 0 {
        return Ok(None);
    }
    if size > 64 * 1024 {
        bail!("sequence keyvalues exceed limit");
    }
    let text = bytes(data, relative(data, at, at + 172)?, size)?;
    let text = &text[..text.iter().position(|b| *b == 0).unwrap_or(size)];
    let Ok(entries) = crate::keyvalues::parse(std::str::from_utf8(text)?) else {
        return Ok(None);
    };
    let Some(block) = entries
        .iter()
        .find(|e| e.key.eq_ignore_ascii_case("mdlkeyvalue"))
        .and_then(|e| e.get("faceposer"))
    else {
        return Ok(None);
    };
    let mut faceposer = Faceposer {
        kind: block
            .get("type")
            .and_then(|e| e.text())
            .unwrap_or("")
            .into(),
        start_loop: "loop".into(),
        end_loop: "end".into(),
        tags: Vec::new(),
    };
    for entry in block.children() {
        if entry.key.eq_ignore_ascii_case("startloop") {
            faceposer.start_loop = entry.text().unwrap_or("").into();
        } else if entry.key.eq_ignore_ascii_case("endloop") {
            faceposer.end_loop = entry.text().unwrap_or("").into();
        } else if entry.key.eq_ignore_ascii_case("tags") {
            for tag in entry.children() {
                faceposer
                    .tags
                    .push((tag.key.clone(), leading_int(tag.text().unwrap_or(""))));
            }
        }
    }
    Ok(Some(faceposer))
}
/// KeyValues GetInt: leading decimal integer, otherwise zero.
fn leading_int(text: &str) -> i32 {
    let text = text.trim_start();
    let end = text
        .char_indices()
        .take_while(|(i, c)| c.is_ascii_digit() || (*i == 0 && (*c == '-' || *c == '+')))
        .last()
        .map_or(0, |(i, c)| i + c.len_utf8());
    text[..end].parse().unwrap_or(0)
}
/// Sequence flags, fades, per-bone weights (remapped to rig bones) and named autolayers.
fn sequence_layer(
    data: &[u8],
    at: usize,
    source_bones: &[SourceBone],
    remap: &[Option<usize>],
    sequences: usize,
    base: usize,
) -> Result<ClipLayer> {
    let weights = relative(data, at, at + 156)?;
    bytes(data, weights, source_bones.len() * 4)?;
    let bone_weights = remap
        .iter()
        .map(|id| match id {
            Some(id) => f32le(data, weights + id * 4),
            None => Ok(0.),
        })
        .collect::<Result<Vec<_>>>()?;
    let autolayers = autolayer_records(data, at)?
        .into_iter()
        .map(|raw| {
            let [start, peak, tail, end] = raw.ramp;
            Ok(AutoLayer {
                sequence: sequence_label(data, sequences, base, raw.child)?,
                pose: raw.pose,
                flags: raw.flags,
                start,
                peak,
                tail,
                end,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ClipLayer {
        flags: u32le(data, at + 12)?,
        bone_weights,
        autolayers,
        fade_in: f32le(data, at + 104)?,
        fade_out: f32le(data, at + 108)?,
        faceposer: sequence_faceposer(data, at)?,
        ..ClipLayer::default()
    })
}
const MAX_CLIPS: usize = 1024;
/// About 112 MiB of sampled local poses per rig.
const MAX_POSES: usize = 4 << 20;
#[derive(Default)]
struct LoadState {
    poses: usize,
    seen: BTreeSet<String>,
    ordinal: u32,
    bytes: usize,
}
fn load_sequences(
    vfs: &Vfs,
    path: &str,
    data: &[u8],
    wanted: &BTreeSet<String>,
    rig: &mut Rig,
    state: &mut LoadState,
    depth: usize,
) -> Result<()> {
    if depth > 8 {
        bail!("included animation model depth budget exceeded");
    }
    if !state.seen.insert(path.to_lowercase()) {
        return Ok(());
    }
    state.bytes = state
        .bytes
        .checked_add(data.len())
        .context("animation byte budget overflow")?;
    if state.seen.len() > 64 || state.bytes > 64 * 1024 * 1024 {
        bail!("included animation model budget exceeded");
    }
    let source_bones = bones(data)?;
    let name = string(data, offset(data, 348)?)?.replace('\\', "/");
    let ani = if offset(data, 352)? > 0 {
        vfs.read(&name)?
    } else {
        None
    };
    let sequences = offset(data, 188)?;
    if sequences > 4096 {
        bail!("sequence table too large");
    }
    let base = offset(data, 192)?;
    let ordinal = state.ordinal;
    state.ordinal = state
        .ordinal
        .checked_add(sequences as u32)
        .context("sequence order overflow")?;
    bytes(data, base, sequences * 212)?;
    let mut selected = BTreeSet::new();
    for id in 0..sequences {
        let metadata = sequence_metadata(data, base + id * 212)?;
        let autoplay = u32le(data, base + id * 212 + 12)? & sequence_flags::AUTOPLAY != 0;
        if autoplay
            || wanted.contains(&metadata.name)
            || (!metadata.activity.is_empty() && wanted.contains(&metadata.activity.to_lowercase()))
        {
            selected.insert(id);
        }
    }
    // Autolayer children are dependencies of their parents; the parent alone has no pose.
    let mut pending = selected.iter().copied().collect::<Vec<_>>();
    while let Some(id) = pending.pop() {
        for RawAutoLayer { child, .. } in autolayer_records(data, base + id * 212)? {
            if child >= sequences {
                bail!("autolayer sequence outside table");
            }
            if selected.insert(child) {
                pending.push(child);
            }
        }
    }
    // Local pose parameter indices map to the rig table by name (shared virtual-model params).
    let param_map = pose_parameters(data)?
        .into_iter()
        .map(|param| {
            if let Some(i) = rig.pose_parameter(&param.name) {
                i
            } else {
                rig.pose_parameters.push(param);
                rig.pose_parameters.len() - 1
            }
        })
        .collect::<Vec<_>>();
    let remap = rig
        .bones
        .iter()
        .map(|b| {
            source_bones
                .iter()
                .position(|s| s.bone.name.eq_ignore_ascii_case(&b.name))
        })
        .collect::<Vec<_>>();
    for id in selected {
        let at = base + id * 212;
        let mut metadata = sequence_metadata(data, at)?;
        metadata.order = ordinal + id as u32;
        let name = metadata.name.clone();
        if rig.sequences.iter().any(|s| s.name == name) {
            continue;
        }
        if rig.sequences.len() >= 4096 {
            bail!("requested sequence metadata budget exceeded");
        }
        rig.sequences.push(metadata);
        if rig.clips.contains_key(&name) {
            continue;
        }
        // Gesture parents need their autolayer children, so the budget counts clips and
        // sampled poses rather than requested names.
        if rig.clips.len() >= MAX_CLIPS || state.poses >= MAX_POSES {
            rig.warnings
                .push(format!("sequence load budget reached; skipped {name}"));
            continue;
        }
        // Clip frames hold the central blend; blend grids keep every animation for pose params.
        let blends = offset(data, at + 56)?.max(1);
        let table = relative(data, at, at + 60)?;
        let animation = usize::try_from(i16le(data, table + (blends / 2) * 2)?)?;
        match clip(
            vfs,
            data,
            ani.as_deref(),
            &source_bones,
            animation,
            u32le(data, at + 12)? & 1 != 0,
        )
        .and_then(|clip| {
            sequence_layer(data, at, &source_bones, &remap, sequences, base).map(|layer| Clip {
                layer: ClipLayer {
                    all_zeros: clip.layer.all_zeros,
                    ..layer
                },
                ..clip
            })
        })
        .and_then(|mut clip| {
            if blends > 1 {
                clip.layer.blend = Some(blend_grid(
                    vfs,
                    data,
                    ani.as_deref(),
                    &source_bones,
                    at,
                    &param_map,
                    clip.looping,
                )?);
            }
            Ok(clip)
        }) {
            Ok(mut clip) => {
                state.poses += clip.frames.len() * rig.bones.len()
                    + clip.layer.blend.as_ref().map_or(0, |g| {
                        g.anims.iter().map(Vec::len).sum::<usize>() * rig.bones.len()
                    });
                let (events, discarded) = sequence_events(data, at)?;
                clip.events = events;
                if discarded > 0 {
                    rig.warnings.push(format!(
                        "{path}:{name}: discarded {discarded} invalid event cycles"
                    ));
                }
                let delta = clip.layer.flags & sequence_flags::DELTA != 0;
                let remap_frame = |frame: &Vec<Pose>| -> Vec<Pose> {
                    rig.bones
                        .iter()
                        .zip(&remap)
                        .map(|(b, id)| {
                            id.and_then(|id| frame.get(id))
                                .cloned()
                                .unwrap_or_else(|| default_pose(b, delta))
                        })
                        .collect()
                };
                for frame in &mut clip.frames {
                    *frame = remap_frame(frame);
                }
                if let Some(grid) = &mut clip.layer.blend {
                    for frames in &mut grid.anims {
                        for frame in frames.iter_mut() {
                            *frame = remap_frame(frame);
                        }
                    }
                }
                if clip.layer.flags & sequence_flags::AUTOPLAY != 0 && !rig.autoplay.contains(&name)
                {
                    rig.autoplay.push(name.clone());
                }
                rig.clips.insert(name, clip);
            }
            Err(e) => rig.warnings.push(format!("{path}:{name}: {e:#}")),
        }
    }
    let includes = offset(data, 336)?;
    if includes > 64 {
        bail!("included model table exceeds limit");
    }
    let base = offset(data, 340)?;
    for id in 0..includes {
        let at = base + id * 8;
        let include = string(data, relative(data, at, at + 4)?)?
            .replace('\\', "/")
            .to_lowercase();
        if let Some(bytes) = vfs.read(&include)? {
            if let Err(e) = load_sequences(vfs, &include, &bytes, wanted, rig, state, depth + 1) {
                rig.warnings.push(format!("{include}: {e:#}"));
            }
        } else {
            rig.warnings
                .push(format!("included model missing: {include}"));
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relative_activity_metadata_preserves_signed_weights_and_bounds() {
        let mut data = vec![0; 300];
        let at = 32;
        data[at + 4..at + 8].copy_from_slice(&212i32.to_le_bytes());
        data[at + 8..at + 12].copy_from_slice(&220i32.to_le_bytes());
        data[at + 20..at + 24].copy_from_slice(&(-3i32).to_le_bytes());
        data[244..250].copy_from_slice(b"IdleA\0");
        data[252..261].copy_from_slice(b"ACT_IDLE\0");
        let seq = sequence_metadata(&data, at).unwrap();
        assert_eq!(seq.name, "idlea");
        assert_eq!(seq.activity, "ACT_IDLE");
        assert_eq!(seq.weight, -3);
        assert!(sequence_metadata(&data[..260], at).is_err());
        data[at + 8..at + 12].fill(0);
        assert!(sequence_metadata(&data, at).unwrap().activity.is_empty());
        data[at + 8..at + 12].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(sequence_metadata(&data, at).is_err());
    }
    #[test]
    #[ignore = "requires owned HL2 installation"]
    fn owned_idle_activities_load_model_sequences_with_finite_poses() {
        let vfs = Vfs::mount(std::path::Path::new(
            &std::env::var("HL2_ROOT").expect("set HL2_ROOT"),
        ))
        .unwrap();
        for (model, expected) in [
            ("models/kleiner.mdl", vec![("idle_subtle", 1)]),
            (
                "models/police.mdl",
                vec![("batonidle1", 2), ("batonidle2", 1)],
            ),
        ] {
            let rig = load(&vfs, model, &BTreeSet::from(["ACT_IDLE".into()])).unwrap();
            assert!(rig.warnings.is_empty(), "{model}: {:?}", rig.warnings);
            assert_eq!(
                rig.sequences
                    .iter()
                    .filter(|s| !rig.autoplay.contains(&s.name))
                    .map(|s| (s.name.as_str(), s.weight))
                    .collect::<Vec<_>>(),
                expected
            );
            if model == "models/kleiner.mdl" {
                // Shared human head/body controls are autoplay pose-parameter blends.
                assert_eq!(
                    rig.autoplay,
                    [
                        "body_rot_z",
                        "spine_rot_z",
                        "neck_trans_x",
                        "head_rot_z",
                        "head_rot_y",
                        "head_rot_x"
                    ]
                );
                let head = rig
                    .pose_parameter("head_yaw")
                    .expect("head_yaw pose parameter");
                let grid = rig.clips["head_rot_z"].layer.blend.as_ref().unwrap();
                assert_eq!(grid.axes[0].as_ref().unwrap().parameter, head);
                assert_eq!((grid.groups, grid.anims.len()), ([3, 1], 3));
                // Default head_yaw 0 selects the neutral middle blend; turning it moves the head.
                let neutral = rig.local_matrices(&{
                    let mut pose = rig.bind_pose();
                    rig.accumulate_autoplay(&mut pose, 0., &rig.default_pose_values())
                        .unwrap();
                    pose
                });
                let mut turned_params = rig.default_pose_values();
                turned_params[head] = 1.;
                let turned = rig.local_matrices(&{
                    let mut pose = rig.bind_pose();
                    rig.accumulate_autoplay(&mut pose, 0., &turned_params)
                        .unwrap();
                    pose
                });
                assert!(neutral
                    .iter()
                    .zip(&turned)
                    .any(|(a, b)| !a.abs_diff_eq(*b, 1e-3)));
                assert!(turned.iter().all(|m| m.is_finite()));
            }
            for seq in &rig.sequences {
                assert!(rig.clips.contains_key(&seq.name));
                assert!(rig.matrices(&seq.name, 0.37).iter().all(|m| m.is_finite()));
            }
            assert!(rig.lookup_sequence("ACT_IDLE", None, |_| 0).is_some());
        }
    }
    fn event_data() -> Vec<u8> {
        let mut data = vec![0; 400];
        // The sequence begins at 32; its event table is relative to that sequence.
        data[56..60].copy_from_slice(&2i32.to_le_bytes());
        data[60..64].copy_from_slice(&96i32.to_le_bytes());
        data[128..132].copy_from_slice(&0.25f32.to_le_bytes());
        data[132..136].copy_from_slice(&5004i32.to_le_bytes());
        data[136..140].copy_from_slice(&1024u32.to_le_bytes());
        data[140..144].copy_from_slice(b"test");
        data[204..208].copy_from_slice(&192i32.to_le_bytes());
        data[320..336].copy_from_slice(b"AE_CL_PLAYSOUND\0");
        data[208..212].copy_from_slice(&f32::NAN.to_le_bytes());
        data
    }
    #[test]
    fn sequence_relative_events_read_options_and_skip_invalid_cycle() {
        let (events, invalid) = sequence_events(&event_data(), 32).unwrap();
        assert_eq!(invalid, 1);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].cycle, 0.25);
        assert_eq!(events[0].options, "test");
        assert_eq!(events[0].name, "AE_CL_PLAYSOUND");
    }
    #[test]
    fn truncated_or_outside_event_records_are_rejected() {
        let mut data = event_data();
        assert!(sequence_events(&data[..220], 32).is_err());
        data[204..208].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(sequence_events(&data, 32).is_err());
    }
    #[test]
    fn signed_rle_repeats_and_crosses_spans() {
        let bytes = [2, 4, 0xff, 0xff, 2, 0, 1, 2, 0xfd, 0xff];
        assert_eq!(rle(&bytes, 0, 0).unwrap(), -1.);
        assert_eq!(rle(&bytes, 0, 3).unwrap(), 2.);
        assert_eq!(rle(&bytes, 0, 5).unwrap(), -3.);
        assert!(rle(&[0, 0], 0, 0).is_err());
    }
    fn layer_data() -> Vec<u8> {
        // Two sequences at 0 and 212; sequence 0 has one autolayer and two bone weights.
        let mut data = vec![0; 760];
        for (seq, label) in [(0usize, 440usize), (212, 450)] {
            data[seq + 4..seq + 8].copy_from_slice(&((label - seq) as i32).to_le_bytes());
        }
        data[440..446].copy_from_slice(b"Parent");
        data[450..455].copy_from_slice(b"Child");
        data[12..16].copy_from_slice(&0x14u32.to_le_bytes());
        data[104..108].copy_from_slice(&0.2f32.to_le_bytes());
        data[108..112].copy_from_slice(&0.4f32.to_le_bytes());
        data[148..152].copy_from_slice(&1i32.to_le_bytes());
        data[152..156].copy_from_slice(&480i32.to_le_bytes());
        data[156..160].copy_from_slice(&520i32.to_le_bytes());
        data[480..482].copy_from_slice(&1i16.to_le_bytes());
        data[482..484].copy_from_slice(&(-1i16).to_le_bytes());
        data[484..488].copy_from_slice(&0x40u32.to_le_bytes());
        for (i, v) in [0.1f32, 0.3, 0.7, 0.9].iter().enumerate() {
            data[488 + i * 4..492 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        data[520..524].copy_from_slice(&0.5f32.to_le_bytes());
        data[524..528].copy_from_slice(&1f32.to_le_bytes());
        let kv = b"mdlkeyvalue { faceposer { \"type\" \"gesture\" \"endloop\" \"hold\" \"tags\" { \"apex\" \"12\" \"loop\" \"40x\" } } }\0";
        data[600..600 + kv.len()].copy_from_slice(kv);
        data[172..176].copy_from_slice(&600i32.to_le_bytes());
        data[176..180].copy_from_slice(&(kv.len() as i32).to_le_bytes());
        data
    }
    fn source_bone(name: &str) -> SourceBone {
        SourceBone {
            bone: Bone {
                name: name.into(),
                parent: None,
                bind: Pose {
                    position: Vec3::ZERO,
                    rotation: Quat::IDENTITY,
                },
                inverse_bind: Mat4::IDENTITY,
            },
            euler: Vec3::ZERO,
            position_scale: Vec3::ONE,
            rotation_scale: Vec3::ONE,
        }
    }
    #[test]
    fn sequence_layer_reads_flags_remapped_weights_and_named_autolayers() {
        let data = layer_data();
        let bones = [source_bone("a"), source_bone("b")];
        // Rig order b, missing, a: weights follow names; unmapped bones get zero.
        let remap = [Some(1), None, Some(0)];
        let layer = sequence_layer(&data, 0, &bones, &remap, 2, 0).unwrap();
        assert_eq!(layer.flags, sequence_flags::DELTA | sequence_flags::POST);
        assert_eq!(layer.bone_weights, [1., 0., 0.5]);
        assert_eq!(layer.fade_in, 0.2);
        assert_eq!(layer.fade_out, 0.4);
        let faceposer = layer.faceposer.as_ref().unwrap();
        assert!(faceposer.is_gesture());
        assert_eq!(
            (faceposer.start_loop.as_str(), faceposer.end_loop.as_str()),
            ("loop", "hold")
        );
        assert_eq!(
            faceposer.tags,
            [("apex".to_string(), 12), ("loop".to_string(), 40)]
        );
        assert_eq!(layer.autolayers.len(), 1);
        let auto = &layer.autolayers[0];
        assert_eq!(auto.sequence, "child");
        assert_eq!((auto.pose, auto.flags), (-1, 0x40));
        assert_eq!(
            [auto.start, auto.peak, auto.tail, auto.end],
            [0.1, 0.3, 0.7, 0.9]
        );
        // Out-of-table child, negative child, nonfinite ramp and weight are rejected.
        assert!(sequence_layer(&data, 0, &bones, &remap, 1, 0).is_err());
        let mut bad = data.clone();
        bad[480..482].copy_from_slice(&(-2i16).to_le_bytes());
        assert!(autolayer_records(&bad, 0).is_err());
        let mut bad = data.clone();
        bad[492..496].copy_from_slice(&f32::INFINITY.to_le_bytes());
        assert!(autolayer_records(&bad, 0).is_err());
        for field in [524, 108] {
            let mut bad = data.clone();
            bad[field..field + 4].copy_from_slice(&f32::NAN.to_le_bytes());
            assert!(sequence_layer(&bad, 0, &bones, &remap, 2, 0).is_err());
        }
    }
    #[test]
    #[ignore = "requires owned HL2 installation"]
    fn owned_barney_gesture_loads_masked_parent_and_delta_children() {
        let vfs = Vfs::mount(std::path::Path::new(
            &std::env::var("HL2_ROOT").expect("set HL2_ROOT"),
        ))
        .unwrap();
        let rig = load(
            &vfs,
            "models/barney.mdl",
            &BTreeSet::from(["g_pointRight".into()]),
        )
        .unwrap();
        let parent = &rig.clips["g_pointright"];
        let faceposer = parent.layer.faceposer.as_ref().unwrap();
        assert!(faceposer.is_gesture());
        for (tag, frame) in [("apex", 12), ("accent", 21), ("loop", 40), ("end", 48)] {
            assert!(
                faceposer.tags.contains(&(tag.to_string(), frame)),
                "{faceposer:?}"
            );
        }
        assert_eq!(parent.layer.autolayers.len(), 6);
        assert!(parent.layer.bone_weights.iter().all(|w| *w == 0.));
        assert_eq!(parent.layer.bone_weights.len(), rig.bones.len());
        let mut delta_children = 0;
        for auto in &parent.layer.autolayers {
            let child = rig.clips.get(&auto.sequence).unwrap_or_else(|| {
                panic!("child {} not loaded: {:?}", auto.sequence, rig.warnings)
            });
            delta_children += usize::from(child.layer.flags & sequence_flags::DELTA != 0);
            assert!(child
                .frames
                .iter()
                .flatten()
                .all(|p| p.position.is_finite() && p.rotation.is_finite()));
        }
        assert!(delta_children > 0);
        let bind = rig.bind_pose();
        let mut changed = 0;
        for cycle in [0.1, 0.3, 0.5, 0.7, 0.9] {
            let mut pose = rig.bind_pose();
            rig.accumulate_pose(&mut pose, "g_pointRight", cycle, 1.)
                .unwrap();
            assert!(rig.local_matrices(&pose).iter().all(|m| m.is_finite()));
            changed += pose
                .iter()
                .zip(&bind)
                .filter(|(a, b)| a.rotation.dot(b.rotation).abs() < 0.9999)
                .count();
        }
        assert!(changed > 0, "gesture layers left every bone at bind");
        let names = std::iter::once("g_pointright".to_string())
            .chain(parent.layer.autolayers.iter().map(|a| a.sequence.clone()))
            .collect::<Vec<_>>();
        assert!(
            !rig.warnings
                .iter()
                .any(|w| names.iter().any(|n| w.contains(n.as_str()))),
            "{:?}",
            rig.warnings
        );
    }
    #[test]
    fn compressed_identity_quaternion() {
        let q = quaternion(&[0, 128, 0, 128, 0, 64], 0, false).unwrap();
        assert!(q.dot(Quat::IDENTITY) > 0.99999);
    }
}

#[cfg(test)]
mod owned_weapon_tests {
    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn installed_grenade_viewmodel_has_sdk_throw_events() {
        let root = crate::install::discover().unwrap();
        let vfs = crate::vpk::Vfs::mount(&root).unwrap();
        let wanted = [
            "act_vm_pullback_high",
            "act_vm_pullback_low",
            "act_vm_throw",
            "act_vm_haulback",
            "act_vm_secondaryattack",
            "act_vm_draw",
            "act_vm_idle",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let rig = super::load(&vfs, "models/weapons/v_grenade.mdl", &wanted).unwrap();
        // (sequence, activity, frames, fps, event id, event cycle): SDK weapon_frag events.
        let expected = [
            ("drawbackhigh", "ACT_VM_PULLBACK_HIGH", 6, 20., 3900, 0.8),
            ("drawbacklow", "ACT_VM_PULLBACK_LOW", 6, 20., 3900, 0.8),
            ("throw", "ACT_VM_THROW", 12, 20., 3005, 1. / 11.),
            ("roll", "ACT_VM_SECONDARYATTACK", 15, 20., 3013, 1. / 7.),
            ("lob", "ACT_VM_HAULBACK", 15, 20., 3016, 1. / 7.),
        ];
        for (name, activity, frames, fps, id, cycle) in expected {
            let seq = rig.sequences.iter().find(|s| s.name == name).unwrap();
            assert_eq!(seq.activity, activity);
            let clip = &rig.clips[name];
            assert_eq!((clip.frames.len(), clip.fps), (frames, fps));
            assert!(clip
                .events
                .iter()
                .any(|e| e.id == id && (e.cycle - cycle).abs() < 1e-4));
        }
        assert_eq!(rig.clips["draw"].frames.len(), 39);
    }
}
