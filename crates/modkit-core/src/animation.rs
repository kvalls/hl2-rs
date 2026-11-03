//! Engine-neutral skeletons, sampled clips and linear-blend skinning.
use glam::{Mat4, Quat, Vec3, Vec4};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Weights {
    pub bones: [u8; 3],
    pub weights: [f32; 3],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    pub position: Vec3,
    pub rotation: Quat,
}
/// Studio attachment (mstudioattachment_t): a named bone-local frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub bone: usize,
    pub local: Mat4,
}
/// Studio hitbox (mstudiobbox_t of hitbox set 0): a bone-local box and its hitgroup.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hitbox {
    pub bone: usize,
    pub group: i32,
    pub mins: Vec3,
    pub maxs: Vec3,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bone {
    pub name: String,
    pub parent: Option<usize>,
    pub bind: Pose,
    pub inverse_bind: Mat4,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Clip {
    #[serde(default)]
    pub events: Vec<ClipEvent>,
    pub fps: f32,
    pub looping: bool,
    /// Local poses as Source CalcPoseSingle returns them. Delta sequences keep raw
    /// offsets/rotations, not poses converted against the bind pose.
    pub frames: Vec<Vec<Pose>>,
    #[serde(default)]
    pub layer: ClipLayer,
}
/// Studio sequence flags used by pose composition (studio.h).
pub mod sequence_flags {
    pub const LOOPING: u32 = 0x0001;
    pub const DELTA: u32 = 0x0004;
    pub const AUTOPLAY: u32 = 0x0008;
    pub const POST: u32 = 0x0010;
    pub const ALLZEROS: u32 = 0x0020;
    pub const LOCAL: u32 = 0x0200;
    pub const WORLD: u32 = 0x4000;
}
/// Studio autolayer flags (studio.h STUDIO_AL_*).
pub mod autolayer_flags {
    pub const POST: u32 = 0x0010;
    pub const SPLINE: u32 = 0x0040;
    pub const XFADE: u32 = 0x0080;
    pub const NOBLEND: u32 = 0x0200;
    pub const LOCAL: u32 = 0x1000;
    pub const POSE: u32 = 0x4000;
}
/// Sequence-level composition data preserved from the MDL sequence descriptor.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ClipLayer {
    pub flags: u32,
    /// Per rig bone; empty means every bone has weight one. Unmapped bones are zero.
    pub bone_weights: Vec<f32>,
    pub autolayers: Vec<AutoLayer>,
    pub fade_in: f32,
    pub fade_out: f32,
    /// Sequence keyvalues `faceposer` block, used to retime scene gestures.
    #[serde(default)]
    pub faceposer: Option<Faceposer>,
    /// Pose-parameter blend grid; `Clip::frames` remains the central sample.
    #[serde(default)]
    pub blend: Option<BlendGrid>,
    /// The single sampled animation has no data (STUDIO_ALLZEROS animation).
    #[serde(default)]
    pub all_zeros: bool,
}
/// Model pose parameter (mstudioposeparamdesc_t); values are normalized 0..1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PoseParameter {
    pub name: String,
    pub start: f32,
    pub end: f32,
    pub looping: f32,
}
impl PoseParameter {
    /// Normalized value for an authored setting (Studio_SetPoseParameter).
    pub fn normalize(&self, value: f32) -> f32 {
        if self.end == self.start {
            return 0.;
        }
        let mut value = value;
        if self.looping != 0. {
            let wrap = (self.start + self.end) / 2. + self.looping / 2.;
            let shift = self.looping - wrap;
            value -= self.looping * ((value + shift) / self.looping).floor();
        }
        ((value - self.start) / (self.end - self.start)).clamp(0., 1.)
    }
    pub fn value(&self, normalized: f32) -> f32 {
        normalized * (self.end - self.start) + self.start
    }
}
/// One sequence blend axis: rig pose parameter and the sequence's authored range.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlendAxis {
    pub parameter: usize,
    pub start: f32,
    pub end: f32,
}
/// Every blend animation of a sequence: `anims[x + y * groups[0]]` as in seqdesc.anim(x, y).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BlendGrid {
    pub axes: [Option<BlendAxis>; 2],
    pub groups: [usize; 2],
    pub anims: Vec<Vec<Vec<Pose>>>,
    pub all_zeros: Vec<bool>,
}
impl BlendGrid {
    /// SDK Studio_LocalPoseParameter (posekeyindex == 0): local fraction and lower index.
    fn local(&self, axis: usize, params: &[f32], table: &[PoseParameter]) -> (f32, usize) {
        let Some(a) = &self.axes[axis] else {
            return (0., 0);
        };
        let Some(pose) = table.get(a.parameter) else {
            return (0., 0);
        };
        let value = params
            .get(a.parameter)
            .copied()
            .unwrap_or_else(|| pose.normalize(0.));
        let mut value = value;
        if pose.looping != 0. {
            let normalized_loop = pose.looping / (pose.end - pose.start);
            let wrap = 0.5 + normalized_loop / 2.;
            let shift = normalized_loop - wrap;
            value -= normalized_loop * ((value + shift) / normalized_loop).floor();
        }
        let local_start = (a.start - pose.start) / (pose.end - pose.start);
        let local_end = (a.end - pose.start) / (pose.end - pose.start);
        let mut setting = ((value - local_start) / (local_end - local_start)).clamp(0., 1.);
        if !setting.is_finite() {
            setting = 0.;
        }
        let groups = self.groups[axis].max(1);
        let mut index = 0;
        if groups > 2 {
            index = ((setting * (groups - 1) as f32) as usize).min(groups - 2);
            setting = setting * (groups - 1) as f32 - index as f32;
        }
        (setting, index)
    }
    fn anim(&self, x: usize, y: usize) -> Option<usize> {
        let i = x + y * self.groups[0].max(1);
        (i < self.anims.len()).then_some(i)
    }
}
/// SDK BlendBones: QuaternionBlend toward `layer` by `s` for bones with sequence weight.
fn blend_bones(base: &mut [Pose], layer: &[Pose], info: &ClipLayer, s: f32) {
    let s1 = 1. - s;
    for (i, (q1, q2)) in base.iter_mut().zip(layer).enumerate() {
        if info.bone_weights.get(i).is_some_and(|w| *w <= 0.) {
            continue;
        }
        let aligned = quaternion_align(q2.rotation, q1.rotation);
        q1.rotation =
            Quat::from_vec4(Vec4::from(q2.rotation) * (1. - s1) + Vec4::from(aligned) * s1)
                .normalize();
        q1.position = q1.position * s1 + q2.position * s;
    }
}
/// Faceposer metadata from MDL sequence keyvalues.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Faceposer {
    /// `type`: "gesture" or "posture"; postures fade while the actor moves.
    pub kind: String,
    /// Tag name and authored animation frame.
    pub tags: Vec<(String, i32)>,
    /// Tags made linear for retiming (`startloop`/`endloop`, default "loop"/"end").
    pub start_loop: String,
    pub end_loop: String,
}
impl Faceposer {
    pub fn is_gesture(&self) -> bool {
        self.kind.eq_ignore_ascii_case("gesture")
    }
}
/// Child sequence accumulated after its parent, ramped by the parent cycle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutoLayer {
    pub sequence: String,
    pub pose: i16,
    pub flags: u32,
    pub start: f32,
    pub peak: f32,
    pub tail: f32,
    pub end: f32,
}
impl AutoLayer {
    /// SDK AddSequenceLayers child (cycle, weight) for a parent cycle; `None` skips it.
    pub fn sample(&self, cycle: f32, weight: f32) -> Result<Option<(f32, f32)>, PoseError> {
        if self.flags & autolayer_flags::POSE != 0 {
            return Err(PoseError::Unsupported("pose-parameter autolayer"));
        }
        if self.start == self.end {
            return Ok(Some((cycle, weight)));
        }
        if cycle < self.start || cycle >= self.end {
            return Ok(None);
        }
        let mut s = 1.;
        if cycle < self.peak && self.start != self.peak {
            s = (cycle - self.start) / (self.peak - self.start);
        } else if cycle > self.tail && self.end != self.tail {
            s = (self.end - cycle) / (self.end - self.tail);
        }
        if self.flags & autolayer_flags::SPLINE != 0 {
            s = simple_spline(s);
        }
        let weight = if self.flags & autolayer_flags::XFADE != 0 && cycle > self.tail {
            (s * weight) / (1. - weight + s * weight)
        } else if self.flags & autolayer_flags::NOBLEND != 0 {
            s
        } else {
            weight * s
        };
        // An all-NaN ramp (retail female_shared.mdl layer_walk_p_aiming) passes every
        // SDK comparison above at full weight, and the SDK hands the NaN child cycle to
        // AccumulatePose; what retail then does is not established, so the child is
        // skipped here (open question, STATUS).
        let child_cycle = (cycle - self.start) / (self.end - self.start);
        if !child_cycle.is_finite() || !weight.is_finite() {
            return Ok(None);
        }
        Ok(Some((child_cycle, weight)))
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum PoseError {
    MissingSequence(String),
    Depth,
    BoneCount,
    Unsupported(&'static str),
}
impl std::fmt::Display for PoseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingSequence(name) => write!(f, "pose layer sequence not loaded: {name}"),
            Self::Depth => f.write_str("pose layer recursion budget exceeded"),
            Self::BoneCount => f.write_str("pose layer bone count differs from rig"),
            Self::Unsupported(what) => write!(f, "unsupported pose composition: {what}"),
        }
    }
}
impl std::error::Error for PoseError {}
pub fn simple_spline(s: f32) -> f32 {
    let s2 = s * s;
    3. * s2 - 2. * s2 * s
}
/// mathlib QuaternionAlign: choose the sign of `q` nearest `p`.
pub fn quaternion_align(p: Quat, q: Quat) -> Quat {
    let a = (Vec4::from(p) - Vec4::from(q)).length_squared();
    let b = (Vec4::from(p) + Vec4::from(q)).length_squared();
    if a > b {
        -q
    } else {
        q
    }
}
/// mathlib QuaternionMult: aligned Hamilton product `p * q`.
pub fn quaternion_mult(p: Quat, q: Quat) -> Quat {
    p * quaternion_align(p, q)
}
/// mathlib QuaternionScale: scale the rotation angle by `t`, keeping the sign of w.
pub fn quaternion_scale(p: Quat, t: f32) -> Quat {
    let sinom = Vec3::new(p.x, p.y, p.z).length().min(1.);
    let sinsom = (sinom.asin() * t).sin();
    let scale = sinsom / (sinom + f32::EPSILON);
    let w = (1. - sinsom * sinsom).max(0.).sqrt();
    Quat::from_xyzw(
        p.x * scale,
        p.y * scale,
        p.z * scale,
        if p.w < 0. { -w } else { w },
    )
}
/// bone_setup QuaternionSM: `(s * p) * q`, normalized.
pub fn quaternion_sm(s: f32, p: Quat, q: Quat) -> Quat {
    quaternion_mult(quaternion_scale(p, s), q).normalize()
}
/// bone_setup QuaternionMA: `p * (s * q)`, normalized.
pub fn quaternion_ma(p: Quat, s: f32, q: Quat) -> Quat {
    quaternion_mult(p, quaternion_scale(q, s)).normalize()
}
/// SDK SlerpBones for local-space sequences: weighted slerp, or additive delta/post.
pub fn slerp_bones(
    base: &mut [Pose],
    layer: &[Pose],
    info: &ClipLayer,
    weight: f32,
) -> Result<(), PoseError> {
    if weight <= 0. {
        return Ok(());
    }
    if info.flags & sequence_flags::WORLD != 0 {
        return Err(PoseError::Unsupported("world-space sequence"));
    }
    if base.len() != layer.len()
        || (!info.bone_weights.is_empty() && info.bone_weights.len() != base.len())
    {
        return Err(PoseError::BoneCount);
    }
    let s = weight.min(1.);
    for (i, (q1, q2)) in base.iter_mut().zip(layer).enumerate() {
        let s2 = s * info.bone_weights.get(i).copied().unwrap_or(1.);
        if s2 <= 0. {
            continue;
        }
        if info.flags & sequence_flags::DELTA != 0 {
            q1.rotation = if info.flags & sequence_flags::POST != 0 {
                quaternion_ma(q1.rotation, s2, q2.rotation)
            } else {
                quaternion_sm(s2, q2.rotation, q1.rotation)
            };
            q1.position += q2.position * s2;
        } else {
            let s1 = 1. - s2;
            q1.rotation = q2.rotation.slerp(q1.rotation, s1);
            q1.position = q1.position * s1 + q2.position * s2;
        }
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClipEvent {
    pub cycle: f32,
    pub id: i32,
    pub flags: u32,
    pub name: String,
    pub options: String,
}
/// Authored movement block. Speeds describe distance across the block's cycle fraction.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MovementRecord {
    pub end_frame: i32,
    pub motion_flags: u32,
    pub v0: f32,
    pub v1: f32,
    pub end_yaw_degrees: f32,
    pub direction: Vec3,
    pub cumulative_position: Vec3,
}
/// Movement of one animation blend; independent of skeletal pose sampling.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RootMotion {
    pub fps: f32,
    pub frame_count: u32,
    pub records: Vec<MovementRecord>,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RootMotionSample {
    pub position: Vec3,
    pub yaw_degrees: f32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootMotionError {
    InvalidData(&'static str),
    NoMovement,
    InvalidCycle,
    UncoveredCycle,
    NonFiniteSample,
}
impl std::fmt::Display for RootMotionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidData(reason) => write!(f, "invalid root motion: {reason}"),
            Self::NoMovement => f.write_str("animation has no authored movement records"),
            Self::InvalidCycle => {
                f.write_str("root-motion cycle is nonfinite or outside loop range")
            }
            Self::UncoveredCycle => {
                f.write_str("root-motion records do not cover the sampled frame")
            }
            Self::NonFiniteSample => f.write_str("root-motion sample overflows finite coordinates"),
        }
    }
}
impl std::error::Error for RootMotionError {}
impl RootMotion {
    pub fn validate(&self) -> Result<(), RootMotionError> {
        if !self.fps.is_finite() || self.fps <= 0. {
            return Err(RootMotionError::InvalidData("frame rate"));
        }
        if self.frame_count == 0 || self.frame_count > 4096 || self.records.len() > 4096 {
            return Err(RootMotionError::InvalidData("frame/record budget"));
        }
        let mut previous = 0;
        for record in &self.records {
            if record.end_frame <= previous || record.end_frame >= self.frame_count as i32 {
                return Err(RootMotionError::InvalidData("movement frame interval"));
            }
            if !record.v0.is_finite()
                || !record.v1.is_finite()
                || !record.end_yaw_degrees.is_finite()
                || !record.direction.is_finite()
                || !record.cumulative_position.is_finite()
            {
                return Err(RootMotionError::InvalidData("nonfinite movement record"));
            }
            previous = record.end_frame;
        }
        Ok(())
    }
    pub fn duration(&self) -> Result<f32, RootMotionError> {
        self.validate()?;
        let duration = (self.frame_count - 1) as f32 / self.fps;
        if !duration.is_finite() {
            return Err(RootMotionError::NonFiniteSample);
        }
        Ok(duration)
    }
    /// Cumulative local position/yaw, including complete cycles in either direction.
    /// The caller applies sequence loop/clamping policy; this method samples animation cycles.
    pub fn position(&self, cycle: f32) -> Result<RootMotionSample, RootMotionError> {
        self.validate()?;
        let Some(last) = self.records.last() else {
            return Err(RootMotionError::NoMovement);
        };
        if !cycle.is_finite() || cycle as f64 <= i32::MIN as f64 || cycle as f64 >= i32::MAX as f64
        {
            return Err(RootMotionError::InvalidCycle);
        }
        // Keep the endpoint convention: cycle1 samples its final frame; negative exact
        // integers sample the prior loop's final frame, rather than changing block selection.
        let loops = if cycle > 1. {
            cycle as i32
        } else if cycle < 0. {
            cycle as i32 - 1
        } else {
            0
        };
        let frame = (cycle - loops as f32) * (self.frame_count - 1) as f32;
        let mut previous_frame = 0.;
        let mut previous_position = Vec3::ZERO;
        let mut previous_yaw = 0.;
        for record in &self.records {
            if frame <= record.end_frame as f32 {
                let fraction =
                    (frame - previous_frame) / (record.end_frame as f32 - previous_frame);
                let distance = (record.v0 + 0.5 * (record.v1 - record.v0) * fraction) * fraction;
                let sample = RootMotionSample {
                    position: previous_position
                        + distance * record.direction
                        + loops as f32 * last.cumulative_position,
                    yaw_degrees: previous_yaw * (1. - fraction)
                        + record.end_yaw_degrees * fraction
                        + loops as f32 * last.end_yaw_degrees,
                };
                return finite_motion(sample);
            }
            previous_frame = record.end_frame as f32;
            previous_position = record.cumulative_position;
            previous_yaw = record.end_yaw_degrees;
        }
        Err(RootMotionError::UncoveredCycle)
    }
    /// Interval displacement expressed relative to the starting heading, with unwrapped yaw.
    pub fn movement(&self, from: f32, to: f32) -> Result<RootMotionSample, RootMotionError> {
        let start = self.position(from)?;
        let end = self.position(to)?;
        let delta = end.position - start.position;
        let (sin, cos) = (-start.yaw_degrees).to_radians().sin_cos();
        finite_motion(RootMotionSample {
            position: Vec3::new(
                delta.x * cos - delta.y * sin,
                delta.x * sin + delta.y * cos,
                delta.z,
            ),
            yaw_degrees: end.yaw_degrees - start.yaw_degrees,
        })
    }
}
fn finite_motion(sample: RootMotionSample) -> Result<RootMotionSample, RootMotionError> {
    if sample.position.is_finite() && sample.yaw_degrees.is_finite() {
        Ok(sample)
    } else {
        Err(RootMotionError::NonFiniteSample)
    }
}
impl Clip {
    pub fn duration(&self) -> f32 {
        self.frames.len().saturating_sub(1) as f32 / self.fps.max(1.)
    }
    /// CalcPoseSingle subset: blend-grid animations selected by pose parameters, each sampled
    /// at `cycle`. `None` when the selected animations have no data (PoseIsAllZeros).
    pub fn sample_posed(
        &self,
        cycle: f32,
        params: &[f32],
        table: &[PoseParameter],
    ) -> Option<Vec<Pose>> {
        let Some(grid) = &self.layer.blend else {
            return if self.layer.all_zeros {
                None
            } else {
                self.sample(cycle)
            };
        };
        let (s0, i0) = grid.local(0, params, table);
        let (s1, i1) = grid.local(1, params, table);
        let pick = |s: f32, i: usize| {
            if s > 0.999 {
                (i + 1, None)
            } else if s < 0.001 {
                (i, None)
            } else {
                (i, Some(s))
            }
        };
        let (x, sx) = pick(s0, i0);
        let (y, sy) = pick(s1, i1);
        let row = |y: usize| -> Option<(Vec<Pose>, bool)> {
            let a = grid.anim(x, y)?;
            let mut pose = self.sample_frames(&grid.anims[a], cycle)?;
            let mut zeros = grid.all_zeros.get(a).copied().unwrap_or(false);
            if let Some(s) = sx {
                let b = grid.anim(x + 1, y)?;
                blend_bones(
                    &mut pose,
                    &self.sample_frames(&grid.anims[b], cycle)?,
                    &self.layer,
                    s,
                );
                zeros &= grid.all_zeros.get(b).copied().unwrap_or(false);
            }
            Some((pose, zeros))
        };
        let (mut pose, mut zeros) = row(y)?;
        if let Some(s) = sy {
            let (next, next_zeros) = row(y + 1)?;
            blend_bones(&mut pose, &next, &self.layer, s);
            zeros &= next_zeros;
        }
        (!zeros).then_some(pose)
    }
    fn sample_frames(&self, frames: &[Vec<Pose>], cycle: f32) -> Option<Vec<Pose>> {
        let last = frames.len().checked_sub(1)?;
        if !cycle.is_finite() {
            return None;
        }
        let cycle = if self.looping {
            cycle - cycle.floor()
        } else {
            cycle.clamp(0., 1.)
        };
        let frame = cycle * last as f32;
        let a = (frame.floor() as usize).min(last);
        let b = (a + 1).min(last);
        Some(interpolate(&frames[a], &frames[b], frame - a as f32))
    }
    /// Interpolated local poses at a normalized cycle; looping clips wrap, others clamp.
    pub fn sample(&self, cycle: f32) -> Option<Vec<Pose>> {
        let last = self.frames.len().checked_sub(1)?;
        if !cycle.is_finite() {
            return None;
        }
        let cycle = if self.looping {
            cycle - cycle.floor()
        } else {
            cycle.clamp(0., 1.)
        };
        let frame = cycle * last as f32;
        let a = (frame.floor() as usize).min(last);
        let b = (a + 1).min(last);
        Some(interpolate(
            &self.frames[a],
            &self.frames[b],
            frame - a as f32,
        ))
    }
    /// Event intervals are open on the left, closed on the right, so frame updates never replay them.
    pub fn events_between(&self, previous: f32, current: f32) -> Vec<&ClipEvent> {
        if current < previous || !previous.is_finite() || !current.is_finite() {
            return Vec::new();
        }
        let duration = self.duration();
        let first = if self.looping && duration > 0. {
            (previous.max(0.) / duration).floor() as i32
        } else {
            0
        };
        let last = if self.looping && duration > 0. {
            (current.max(0.) / duration).floor() as i32
        } else {
            0
        };
        let mut result = Vec::new();
        for cycle in first..=last.min(first.saturating_add(64)) {
            for event in &self.events {
                let at = (cycle as f32 + event.cycle) * duration;
                if at > previous && at <= current {
                    result.push(event);
                }
            }
        }
        result
    }
}
/// Model-authored activity assignment. Ordering preserves the include traversal.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sequence {
    pub name: String,
    pub activity: String,
    pub weight: i32,
    #[serde(default)]
    pub order: u32,
    /// The sequence's bounding box (`mstudioseqdesc_t` bbmin, bbmax), model space.
    #[serde(default)]
    pub bounds: (Vec3, Vec3),
}
impl Sequence {
    /// A sequence without bounds.
    pub fn new(name: &str, activity: &str, weight: i32, order: u32) -> Self {
        Self {
            name: name.into(),
            activity: activity.into(),
            weight,
            order,
            bounds: Default::default(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Rig {
    /// The model's view box (`view_bbmin`/`view_bbmax`), or its hull when that is empty: with
    /// the current sequence's box, `C_BaseAnimating::GetRenderBounds`.
    #[serde(default)]
    pub view_bounds: (Vec3, Vec3),
    pub bones: Vec<Bone>,
    pub clips: BTreeMap<String, Clip>,
    #[serde(default)]
    pub sequences: Vec<Sequence>,
    pub warnings: Vec<String>,
    /// Model pose parameters merged by name across included models.
    #[serde(default)]
    pub pose_parameters: Vec<PoseParameter>,
    /// STUDIO_AUTOPLAY sequences, accumulated after layers every frame.
    #[serde(default)]
    pub autoplay: Vec<String>,
    /// Attachments of the root model, in model order.
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// Every sequence activity of the model and its included models (upper case),
    /// loaded or not: SelectWeightedSequence availability (Weapon_CanUse).
    #[serde(default)]
    pub activities: std::collections::BTreeSet<String>,
    /// Hitbox set 0 of the root model (bullet traces and hitgroups).
    #[serde(default)]
    pub hitboxes: Vec<Hitbox>,
}
impl Rig {
    pub fn attachment(&self, name: &str) -> Option<&Attachment> {
        self.attachments
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
    }
    /// Model-space attachment frame from skinning matrices (bone pose * inverse bind).
    pub fn attachment_matrix(&self, matrices: &[Mat4], attachment: &Attachment) -> Option<Mat4> {
        let skin = matrices.get(attachment.bone)?;
        let bone = self.bones.get(attachment.bone)?;
        Some(*skin * bone.inverse_bind.inverse() * attachment.local)
    }
    /// Merge preloaded cohorts without changing model traversal order or label ownership.
    pub fn merge_sequence_metadata(&mut self, sequences: impl IntoIterator<Item = Sequence>) {
        for sequence in sequences {
            if let Some(existing) = self.sequences.iter_mut().find(|s| s.name == sequence.name) {
                if sequence.order < existing.order {
                    *existing = sequence;
                }
            } else {
                self.sequences.push(sequence);
            }
        }
        self.sequences.sort_by_key(|s| s.order);
    }
    /// Label first, then model-authored activity. The caller supplies a bounded
    /// integer in 0..total_weight; native RNG/prediction stream parity is separate.
    /// Pass no current sequence for native LookupSequence semantics; activity selection
    /// can retain an appropriate negative-weight current sequence without drawing RNG.
    pub fn lookup_sequence(
        &self,
        name: &str,
        current: Option<&str>,
        mut draw: impl FnMut(u32) -> u32,
    ) -> Option<&str> {
        if let Some(sequence) = self
            .sequences
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
        {
            return Some(&sequence.name);
        }
        if let Some((label, _)) = self.clips.get_key_value(&name.to_lowercase()) {
            return Some(label);
        }
        if let Some(sequence) = self.sequences.iter().find(|s| {
            s.weight < 0
                && s.activity.eq_ignore_ascii_case(name)
                && current.is_some_and(|c| c.eq_ignore_ascii_case(&s.name))
        }) {
            return Some(&sequence.name);
        }
        let choices: Vec<_> = self
            .sequences
            .iter()
            .filter(|s| !s.activity.is_empty() && s.activity.eq_ignore_ascii_case(name))
            .collect();
        let total = choices
            .iter()
            .try_fold(0u32, |total, s| total.checked_add(s.weight.unsigned_abs()))?;
        // The native all-zero-weight table has no valid random interval. Reject it.
        if total == 0 || total > i32::MAX as u32 {
            return None;
        }
        let mut ticket = draw(total);
        if ticket >= total {
            return None;
        }
        for sequence in choices {
            // Native tuple construction gives a zero-weight entry one slot, while
            // its total interval is still the sum of absolute authored weights.
            let slots = sequence.weight.unsigned_abs().max(1);
            if ticket < slots {
                return Some(&sequence.name);
            }
            ticket -= slots;
        }
        None
    }
    pub fn bind_pose(&self) -> Vec<Pose> {
        self.bones.iter().map(|b| b.bind.clone()).collect()
    }
    /// SDK AccumulatePose subset: sample the sequence, SlerpBones it onto `pose`, then
    /// add non-local autolayers. IK, local-context and world-space layers are not applied.
    pub fn accumulate_pose(
        &self,
        pose: &mut [Pose],
        sequence: &str,
        cycle: f32,
        weight: f32,
    ) -> Result<(), PoseError> {
        let params = self.default_pose_values();
        self.accumulate(pose, sequence, cycle, weight.clamp(0., 1.), &params, 0)
    }
    /// AccumulatePose with explicit normalized pose-parameter values (rig order).
    pub fn accumulate_pose_with(
        &self,
        pose: &mut [Pose],
        sequence: &str,
        cycle: f32,
        weight: f32,
        params: &[f32],
    ) -> Result<(), PoseError> {
        self.accumulate(pose, sequence, cycle, weight.clamp(0., 1.), params, 0)
    }
    /// CBaseAnimating resets each pose parameter to the authored value 0.
    pub fn default_pose_values(&self) -> Vec<f32> {
        self.pose_parameters
            .iter()
            .map(|p| p.normalize(0.))
            .collect()
    }
    pub fn pose_parameter(&self, name: &str) -> Option<usize> {
        self.pose_parameters
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(name))
    }
    /// CalcAutoplaySequences: each autoplay sequence at real-time cycle, weight one.
    pub fn accumulate_autoplay(
        &self,
        pose: &mut [Pose],
        time: f32,
        params: &[f32],
    ) -> Result<(), PoseError> {
        for name in &self.autoplay {
            let Some(clip) = self.clips.get(name) else {
                continue;
            };
            let frames = clip.frames.len().saturating_sub(1);
            let cps = if frames > 0 {
                clip.fps / frames as f32
            } else {
                0.
            };
            let cycle = (time * cps).fract();
            self.accumulate(pose, name, cycle, 1., params, 0)?;
        }
        Ok(())
    }
    fn accumulate(
        &self,
        pose: &mut [Pose],
        sequence: &str,
        cycle: f32,
        weight: f32,
        params: &[f32],
        depth: usize,
    ) -> Result<(), PoseError> {
        if depth > 8 {
            return Err(PoseError::Depth);
        }
        let clip = self
            .clips
            .get(&sequence.to_lowercase())
            .ok_or_else(|| PoseError::MissingSequence(sequence.into()))?;
        if clip.layer.flags & sequence_flags::LOCAL != 0 {
            return Err(PoseError::Unsupported("local-context sequence"));
        }
        // CalcPoseSingle fails for all-zero animations; SlerpBones is then skipped.
        if let Some(layer) = clip.sample_posed(cycle, params, &self.pose_parameters) {
            slerp_bones(pose, &layer, &clip.layer, weight)?;
        }
        for auto in &clip.layer.autolayers {
            // AddSequenceLayers skips local layers; those belong to AddLocalLayers.
            if auto.flags & autolayer_flags::LOCAL != 0 {
                continue;
            }
            if let Some((child_cycle, child_weight)) = auto.sample(cycle, weight)? {
                self.accumulate(
                    pose,
                    &auto.sequence,
                    child_cycle,
                    child_weight.clamp(0., 1.),
                    params,
                    depth + 1,
                )?;
            }
        }
        Ok(())
    }
    /// Base-clip local pose at seconds. Delta sequences are composed onto the bind pose.
    pub fn local_pose(&self, name: &str, time: f32) -> Vec<Pose> {
        let Some(clip) = self.clips.get(&name.to_lowercase()) else {
            return self.bind_pose();
        };
        let Some(last) = clip.frames.len().checked_sub(1) else {
            return self.bind_pose();
        };
        let frames = last.max(1) as f32;
        let frame = if clip.looping {
            (time.max(0.) * clip.fps) % frames
        } else {
            (time.max(0.) * clip.fps).min(frames)
        };
        let a = (frame.floor() as usize).min(last);
        let b = (a + 1).min(last);
        let sampled = interpolate(&clip.frames[a], &clip.frames[b], frame.fract());
        if clip.layer.flags & sequence_flags::DELTA == 0 {
            return sampled;
        }
        let mut pose = self.bind_pose();
        if slerp_bones(&mut pose, &sampled, &clip.layer, 1.).is_err() {
            pose = self.bind_pose();
        }
        pose
    }
    /// Base-clip playback at seconds.
    pub fn matrices(&self, name: &str, time: f32) -> Vec<Mat4> {
        self.local_matrices(&self.local_pose(name, time))
    }
    /// Skinning matrices for already composed local poses; missing bones use bind.
    pub fn local_matrices(&self, poses: &[Pose]) -> Vec<Mat4> {
        let mut world = Vec::<Mat4>::with_capacity(self.bones.len());
        for (id, bone) in self.bones.iter().enumerate() {
            let pose = poses.get(id).unwrap_or(&bone.bind);
            let local = Mat4::from_rotation_translation(pose.rotation, pose.position);
            world.push(
                bone.parent
                    .and_then(|p| world.get(p))
                    .map_or(local, |parent| *parent * local),
            );
        }
        world
            .iter()
            .zip(&self.bones)
            .map(|(m, b)| *m * b.inverse_bind)
            .collect()
    }
}
fn interpolate(a: &[Pose], b: &[Pose], t: f32) -> Vec<Pose> {
    a.iter()
        .zip(b)
        .map(|(p, q)| Pose {
            position: p.position.lerp(q.position, t),
            rotation: p.rotation.slerp(q.rotation, t),
        })
        .collect()
}
pub fn skin(position: Vec3, weights: &Weights, matrices: &[Mat4]) -> Vec3 {
    let mut output = Vec3::ZERO;
    let mut total = 0.;
    for (&id, &weight) in weights.bones.iter().zip(&weights.weights) {
        if weight > 0. {
            if let Some(m) = matrices.get(id as usize) {
                output += m.transform_point3(position) * weight;
                total += weight;
            }
        }
    }
    if total > 0. {
        output / total
    } else {
        position
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sequence_lookup_preserves_labels_weight_intervals_and_negative_current() {
        let mut rig = Rig {
            sequences: vec![
                Sequence::new("idle_a", "ACT_IDLE", 2, 0),
                Sequence::new("idle_b", "ACT_IDLE", -1, 1),
                Sequence::new("act_idle", "ACT_OTHER", 1, 2),
            ],
            ..Default::default()
        };
        assert_eq!(
            rig.lookup_sequence("ACT_IDLE", None, |_| panic!("labels use no random draw")),
            Some("act_idle")
        );
        rig.sequences.pop();
        for (ticket, expected) in [(0, "idle_a"), (1, "idle_a"), (2, "idle_b")] {
            assert_eq!(
                rig.lookup_sequence("aCt_IdLe", None, |total| {
                    assert_eq!(total, 3);
                    ticket
                }),
                Some(expected)
            );
        }
        assert_eq!(
            rig.lookup_sequence("ACT_IDLE", Some("IDLE_B"), |_| panic!(
                "negative current is retained"
            )),
            Some("idle_b")
        );
        assert_eq!(
            rig.lookup_sequence("ACT_IDLE", Some("idle_a"), |_| 2),
            Some("idle_b")
        );
        assert!(rig
            .lookup_sequence("ACT_MISSING", None, |_| panic!())
            .is_none());
        assert!(rig.lookup_sequence("ACT_IDLE", None, |_| 3).is_none());
        rig.sequences[0].weight = 0;
        rig.sequences[1].weight = 0;
        assert!(rig
            .lookup_sequence("ACT_IDLE", None, |_| panic!("zero total is rejected"))
            .is_none());
        rig.sequences[1].weight = 2;
        assert_eq!(rig.lookup_sequence("ACT_IDLE", None, |_| 0), Some("idle_a"));
        assert_eq!(rig.lookup_sequence("ACT_IDLE", None, |_| 1), Some("idle_b"));
        rig.sequences[0].weight = i32::MIN;
        assert!(rig
            .lookup_sequence("ACT_IDLE", None, |_| panic!("overflow rejected"))
            .is_none());
        let first = Sequence::new("first", "ACT_NEW", 1, 3);
        let last = Sequence::new("last", "ACT_NEW", 1, 8);
        rig.merge_sequence_metadata([last.clone()]);
        rig.merge_sequence_metadata([last, first]);
        assert_eq!(
            rig.sequences
                .iter()
                .filter(|s| s.activity == "ACT_NEW")
                .count(),
            2
        );
        assert_eq!(rig.lookup_sequence("ACT_NEW", None, |_| 0), Some("first"));
        assert_eq!(rig.lookup_sequence("ACT_NEW", None, |_| 1), Some("last"));
    }
    fn turning_motion() -> RootMotion {
        RootMotion {
            fps: 20.,
            frame_count: 21,
            records: vec![
                MovementRecord {
                    end_frame: 10,
                    motion_flags: 0xc0,
                    v0: 4.,
                    v1: 8.,
                    end_yaw_degrees: 90.,
                    direction: Vec3::X,
                    cumulative_position: Vec3::X * 6.,
                },
                MovementRecord {
                    end_frame: 20,
                    motion_flags: 0xc0,
                    v0: 8.,
                    v1: 12.,
                    end_yaw_degrees: 180.,
                    direction: Vec3::Y,
                    cumulative_position: Vec3::new(6., 10., 0.),
                },
            ],
        }
    }
    #[test]
    fn root_motion_piecewise_acceleration_and_cumulative_blocks() {
        let motion = turning_motion();
        assert_eq!(motion.duration(), Ok(1.));
        assert_eq!(
            motion.position(0.25).unwrap(),
            RootMotionSample {
                position: Vec3::X * 2.5,
                yaw_degrees: 45.
            }
        );
        assert_eq!(
            motion.position(0.5).unwrap(),
            RootMotionSample {
                position: Vec3::X * 6.,
                yaw_degrees: 90.
            }
        );
        assert_eq!(
            motion.position(0.75).unwrap(),
            RootMotionSample {
                position: Vec3::new(6., 4.5, 0.),
                yaw_degrees: 135.
            }
        );
        assert_eq!(
            motion.position(1.).unwrap(),
            RootMotionSample {
                position: Vec3::new(6., 10., 0.),
                yaw_degrees: 180.
            }
        );
    }
    #[test]
    fn root_motion_turning_intervals_rotate_by_negative_initial_yaw() {
        let motion = turning_motion();
        let interval = motion.movement(0.5, 0.75).unwrap();
        assert!((interval.position - Vec3::X * 4.5).length() < 1e-5);
        assert_eq!(interval.yaw_degrees, 45.);
        let crossing = motion.movement(0.75, 1.25).unwrap();
        let expected = Vec3::new(3. / 2f32.sqrt(), -8. / 2f32.sqrt(), 0.);
        assert!((crossing.position - expected).length() < 1e-5);
        assert_eq!(crossing.yaw_degrees, 90.);
    }
    #[test]
    fn root_motion_accumulates_complete_positive_and_negative_loops() {
        let motion = turning_motion();
        assert_eq!(
            motion.position(2.).unwrap(),
            RootMotionSample {
                position: Vec3::new(12., 20., 0.),
                yaw_degrees: 360.
            }
        );
        assert_eq!(
            motion.position(-1.).unwrap(),
            RootMotionSample {
                position: Vec3::new(-6., -10., 0.),
                yaw_degrees: -180.
            }
        );
        assert_eq!(
            motion.position(-0.25).unwrap(),
            RootMotionSample {
                position: Vec3::new(0., -5.5, 0.),
                yaw_degrees: -45.
            }
        );
        assert_eq!(
            motion.position(1.25).unwrap(),
            RootMotionSample {
                position: Vec3::new(8.5, 10., 0.),
                yaw_degrees: 225.
            }
        );
    }
    #[test]
    fn root_motion_rejects_invalid_records_and_reports_missing_coverage() {
        let mut motion = turning_motion();
        for cycle in [f32::NAN, f32::INFINITY, i32::MAX as f32, i32::MIN as f32] {
            assert_eq!(motion.position(cycle), Err(RootMotionError::InvalidCycle));
        }
        assert_eq!(
            motion.movement(0., f32::NAN),
            Err(RootMotionError::InvalidCycle)
        );
        motion.records[1].end_frame = 10;
        assert!(motion.validate().is_err());
        motion.records[1].end_frame = 21;
        assert!(motion.validate().is_err());
        motion.records[1].end_frame = 20;
        motion.records[0].v0 = f32::NAN;
        assert!(motion.validate().is_err());
        motion.records[0].v0 = 4.;
        motion.records.truncate(1);
        assert_eq!(motion.position(0.75), Err(RootMotionError::UncoveredCycle));
        motion.records.clear();
        assert_eq!(motion.movement(0., 1.), Err(RootMotionError::NoMovement));
        motion.fps = 0.;
        assert!(motion.duration().is_err());
    }
    fn event_clip(looping: bool) -> Clip {
        Clip {
            layer: Default::default(),
            fps: 1.,
            looping,
            frames: vec![vec![], vec![]],
            events: vec![
                ClipEvent {
                    cycle: 0.,
                    id: 5004,
                    flags: 0,
                    name: String::new(),
                    options: "start".into(),
                },
                ClipEvent {
                    cycle: 0.5,
                    id: 5004,
                    flags: 0,
                    name: String::new(),
                    options: "middle".into(),
                },
            ],
        }
    }
    #[test]
    fn events_play_once_at_start_and_frame_boundaries() {
        let clip = event_clip(false);
        assert_eq!(clip.events_between(-f32::EPSILON, 0.)[0].options, "start");
        assert!(clip.events_between(0., 0.49).is_empty());
        assert_eq!(clip.events_between(0.49, 0.5)[0].options, "middle");
        assert!(clip.events_between(0.5, 2.).is_empty());
        assert!(clip.events_between(1., 0.5).is_empty());
        assert!(clip.events_between(0., f32::NAN).is_empty());
    }
    #[test]
    fn looping_events_cross_wrap_without_repeating_previous_boundary() {
        let clip = event_clip(true);
        let names = clip
            .events_between(0.75, 1.5)
            .into_iter()
            .map(|e| e.options.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["start", "middle"]);
        assert!(clip.events_between(1.5, 1.75).is_empty());
    }
    #[test]
    fn hierarchy_bind_and_interpolated_skin() {
        let bone = Bone {
            name: "root".into(),
            parent: None,
            bind: Pose {
                position: Vec3::X,
                rotation: Quat::IDENTITY,
            },
            inverse_bind: Mat4::from_translation(-Vec3::X),
        };
        let mut rig = Rig {
            bones: vec![bone],
            ..Default::default()
        };
        assert_eq!(rig.matrices("", 0.)[0], Mat4::IDENTITY);
        rig.clips.insert(
            "move".into(),
            Clip {
                layer: Default::default(),
                events: Vec::new(),
                fps: 1.,
                looping: false,
                frames: vec![
                    vec![Pose {
                        position: Vec3::X,
                        rotation: Quat::IDENTITY,
                    }],
                    vec![Pose {
                        position: Vec3::X * 3.,
                        rotation: Quat::IDENTITY,
                    }],
                ],
            },
        );
        let p = skin(
            Vec3::Y,
            &Weights {
                bones: [0; 3],
                weights: [1., 0., 0.],
            },
            &rig.matrices("move", 0.5),
        );
        assert!((p - Vec3::new(1., 1., 0.)).length() < 0.0001);
    }
    fn close(a: Quat, b: Quat) -> bool {
        a.dot(b).abs() > 0.99999
    }
    fn pose(position: Vec3, rotation: Quat) -> Pose {
        Pose { position, rotation }
    }
    #[test]
    fn quaternion_scale_follows_angle_and_keeps_w_sign() {
        let q = Quat::from_rotation_z(1.2);
        assert!(close(quaternion_scale(q, 1.), q));
        assert!(close(quaternion_scale(q, 0.), Quat::IDENTITY));
        assert!(close(quaternion_scale(q, 0.5), Quat::from_rotation_z(0.6)));
        assert!(quaternion_scale(-q, 0.5).w < 0.);
        // Alignment picks the nearer sign before the Hamilton product.
        let p = Quat::from_rotation_x(0.3);
        assert!(close(quaternion_mult(p, -q), p * q));
    }
    #[test]
    fn delta_layers_premultiply_and_post_layers_postmultiply() {
        let base = Quat::from_rotation_x(0.5);
        let delta = Quat::from_rotation_z(0.7);
        let mut info = ClipLayer {
            flags: sequence_flags::DELTA,
            ..Default::default()
        };
        let layer = [pose(Vec3::X * 2., delta)];
        let mut pre = [pose(Vec3::Y, base)];
        slerp_bones(&mut pre, &layer, &info, 1.).unwrap();
        assert!(close(pre[0].rotation, delta * base));
        assert_eq!(pre[0].position, Vec3::new(2., 1., 0.));
        info.flags |= sequence_flags::POST;
        let mut post = [pose(Vec3::Y, base)];
        slerp_bones(&mut post, &layer, &info, 0.5).unwrap();
        assert!(close(post[0].rotation, base * Quat::from_rotation_z(0.35)));
        assert_eq!(post[0].position, Vec3::new(1., 1., 0.));
        assert!(!close(pre[0].rotation, base * delta));
    }
    #[test]
    fn bone_weights_mask_and_scale_ordinary_blends() {
        let info = ClipLayer {
            bone_weights: vec![0., 0.5],
            ..Default::default()
        };
        let mut base = [
            pose(Vec3::ZERO, Quat::IDENTITY),
            pose(Vec3::ZERO, Quat::IDENTITY),
        ];
        let layer = [
            pose(Vec3::X * 4., Quat::from_rotation_y(1.)),
            pose(Vec3::X * 4., Quat::from_rotation_y(1.)),
        ];
        slerp_bones(&mut base, &layer, &info, 0.5).unwrap();
        assert_eq!(base[0].position, Vec3::ZERO);
        assert!(close(base[0].rotation, Quat::IDENTITY));
        assert!((base[1].position - Vec3::X).length() < 1e-6);
        assert!(close(base[1].rotation, Quat::from_rotation_y(0.25)));
        assert_eq!(
            slerp_bones(&mut base, &layer[..1], &info, 1.),
            Err(PoseError::BoneCount)
        );
        let world = ClipLayer {
            flags: sequence_flags::WORLD,
            ..Default::default()
        };
        assert!(slerp_bones(&mut base, &layer, &world, 1.).is_err());
    }
    fn ramp(flags: u32) -> AutoLayer {
        AutoLayer {
            sequence: "child".into(),
            pose: -1,
            flags,
            start: 0.2,
            peak: 0.4,
            tail: 0.6,
            end: 1.,
        }
    }
    #[test]
    fn autolayer_ramps_match_sdk_cases() {
        let linear = ramp(0);
        assert_eq!(linear.sample(0.1, 1.), Ok(None));
        assert_eq!(linear.sample(1., 1.), Ok(None));
        let (cycle, weight) = linear.sample(0.3, 0.8).unwrap().unwrap();
        assert!((cycle - 0.125).abs() < 1e-6 && (weight - 0.4).abs() < 1e-6);
        assert_eq!(linear.sample(0.5, 0.8).unwrap().unwrap().1, 0.8);
        let (_, tail) = linear.sample(0.8, 1.).unwrap().unwrap();
        assert!((tail - 0.5).abs() < 1e-6);
        let spline = ramp(autolayer_flags::SPLINE)
            .sample(0.25, 1.)
            .unwrap()
            .unwrap();
        assert!((spline.1 - simple_spline(0.25)).abs() < 1e-6);
        let noblend = ramp(autolayer_flags::NOBLEND)
            .sample(0.3, 0.2)
            .unwrap()
            .unwrap();
        assert!((noblend.1 - 0.5).abs() < 1e-6);
        let xfade = ramp(autolayer_flags::XFADE)
            .sample(0.8, 0.5)
            .unwrap()
            .unwrap();
        assert!((xfade.1 - 0.25 / 0.75).abs() < 1e-6);
        let mut constant = ramp(0);
        constant.end = constant.start;
        assert_eq!(constant.sample(0.9, 0.7), Ok(Some((0.9, 0.7))));
        assert!(ramp(autolayer_flags::POSE).sample(0.5, 1.).is_err());
        let nan = AutoLayer {
            start: f32::NAN,
            peak: f32::NAN,
            tail: f32::NAN,
            end: f32::NAN,
            ..ramp(0)
        };
        assert_eq!(nan.sample(0.5, 1.), Ok(None));
    }
    fn layered_rig() -> Rig {
        let bone = Bone {
            name: "root".into(),
            parent: None,
            bind: pose(Vec3::ZERO, Quat::from_rotation_x(0.2)),
            inverse_bind: Mat4::IDENTITY,
        };
        let parent = Clip {
            events: vec![],
            fps: 10.,
            looping: false,
            frames: vec![vec![pose(Vec3::ZERO, Quat::IDENTITY)]; 11],
            layer: ClipLayer {
                flags: sequence_flags::DELTA | sequence_flags::ALLZEROS,
                bone_weights: vec![0.],
                autolayers: vec![
                    ramp(0),
                    AutoLayer {
                        flags: autolayer_flags::LOCAL,
                        ..ramp(0)
                    },
                ],
                ..Default::default()
            },
        };
        let child = Clip {
            events: vec![],
            fps: 10.,
            looping: false,
            frames: vec![
                vec![pose(Vec3::ZERO, Quat::IDENTITY)],
                vec![pose(Vec3::Z * 8., Quat::from_rotation_z(0.8))],
            ],
            layer: ClipLayer {
                flags: sequence_flags::DELTA,
                ..Default::default()
            },
        };
        Rig {
            bones: vec![bone],
            clips: BTreeMap::from([("parent".into(), parent), ("child".into(), child)]),
            ..Default::default()
        }
    }
    #[test]
    fn accumulate_pose_adds_ramped_children_after_masked_parent() {
        let rig = layered_rig();
        let bind = rig.bind_pose();
        // Parent cycle 0.3: child weight 0.5, child cycle 0.125.
        let mut composed = rig.bind_pose();
        rig.accumulate_pose(&mut composed, "parent", 0.3, 1.)
            .unwrap();
        let child = rig.clips["child"].sample(0.125).unwrap();
        let mut expected = rig.bind_pose();
        slerp_bones(&mut expected, &child, &rig.clips["child"].layer, 0.5).unwrap();
        assert!(close(composed[0].rotation, expected[0].rotation));
        assert!((composed[0].position - Vec3::Z * 0.5).length() < 1e-5);
        assert!(!close(composed[0].rotation, bind[0].rotation));
        // Outside the ramp the zero-weight parent leaves the base unchanged.
        let mut outside = rig.bind_pose();
        rig.accumulate_pose(&mut outside, "parent", 0.1, 1.)
            .unwrap();
        assert!(close(outside[0].rotation, bind[0].rotation));
        assert_eq!(
            rig.accumulate_pose(&mut outside, "missing", 0.5, 1.),
            Err(PoseError::MissingSequence("missing".into()))
        );
    }
    #[test]
    fn accumulate_pose_rejects_cycles_local_context_and_missing_children() {
        let mut rig = layered_rig();
        let mut pose = rig.bind_pose();
        rig.clips.get_mut("child").unwrap().layer.autolayers = vec![AutoLayer {
            sequence: "child".into(),
            start: 0.,
            end: 0.,
            ..ramp(0)
        }];
        assert_eq!(
            rig.accumulate_pose(&mut pose, "child", 0.5, 1.),
            Err(PoseError::Depth)
        );
        rig.clips.get_mut("child").unwrap().layer.flags |= sequence_flags::LOCAL;
        assert!(matches!(
            rig.accumulate_pose(&mut pose, "child", 0.5, 1.),
            Err(PoseError::Unsupported(_))
        ));
        rig.clips.remove("child");
        assert_eq!(
            rig.accumulate_pose(&mut pose, "parent", 0.5, 1.),
            Err(PoseError::MissingSequence("child".into()))
        );
    }
    #[test]
    fn standalone_delta_playback_composes_onto_bind() {
        let rig = layered_rig();
        let bind = rig.bones[0].bind.rotation;
        let matrix = rig.matrices("child", 0.1);
        let (_, rotation, translation) = matrix[0].to_scale_rotation_translation();
        assert!(close(rotation, Quat::from_rotation_z(0.8) * bind));
        assert!((translation - Vec3::Z * 8.).length() < 1e-4);
        assert_eq!(
            rig.matrices("missing", 0.),
            rig.local_matrices(&rig.bind_pose())
        );
    }
}
#[cfg(test)]
mod blend_tests {
    use super::*;
    fn yaw_clip(all_zeros_middle: bool) -> (Clip, Vec<PoseParameter>) {
        let anim = |angle: f32| {
            vec![vec![Pose {
                position: Vec3::ZERO,
                rotation: Quat::from_rotation_z(angle),
            }]]
        };
        let clip = Clip {
            events: vec![],
            fps: 30.,
            looping: false,
            frames: anim(0.),
            layer: ClipLayer {
                flags: sequence_flags::DELTA | sequence_flags::POST,
                blend: Some(BlendGrid {
                    // Reversed authored range, like head_rot_z (66.7 .. -66.7).
                    axes: [
                        Some(BlendAxis {
                            parameter: 0,
                            start: 60.,
                            end: -60.,
                        }),
                        None,
                    ],
                    groups: [3, 1],
                    anims: vec![anim(1.), anim(0.), anim(-1.)],
                    all_zeros: vec![false, all_zeros_middle, false],
                }),
                ..Default::default()
            },
        };
        (
            clip,
            vec![PoseParameter {
                name: "head_yaw".into(),
                start: -60.,
                end: 60.,
                looping: 0.,
            }],
        )
    }
    fn angle(p: &[Pose]) -> f32 {
        let (axis, angle) = p[0].rotation.to_axis_angle();
        angle * axis.z.signum()
    }
    #[test]
    fn blend_grid_maps_reversed_ranges_and_interpolates() {
        let (clip, table) = yaw_clip(false);
        let at = |value: f32| {
            angle(
                &clip
                    .sample_posed(0., &[table[0].normalize(value)], &table)
                    .unwrap(),
            )
        };
        assert!(at(0.).abs() < 1e-5, "neutral uses the middle blend");
        // Blend 0 belongs to paramstart (+60), the last blend to paramend (-60).
        assert!((at(-60.) + 1.).abs() < 1e-4);
        assert!((at(60.) - 1.).abs() < 1e-4);
        assert!((at(-30.) + 0.5).abs() < 1e-3, "{}", at(-30.));
        assert_eq!(table[0].normalize(0.), 0.5);
    }
    #[test]
    fn all_zero_selection_skips_calc_pose_single() {
        let (clip, table) = yaw_clip(true);
        assert!(clip.sample_posed(0., &[0.5], &table).is_none());
        assert!(clip.sample_posed(0., &[0.25], &table).is_some());
        let single = Clip {
            layer: ClipLayer {
                all_zeros: true,
                ..Default::default()
            },
            ..yaw_clip(false).0
        };
        assert!(single.sample_posed(0., &[], &table).is_none());
    }
}
