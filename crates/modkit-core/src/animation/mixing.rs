//! Mixing local bone poses: frame interpolation, SDK BlendBones and SlerpBones, the quaternion
//! helpers they use, and the bone sets that limit them to the bones a caller needs.
use super::{sequence_flags, ClipLayer, Pose, PoseError};
use glam::{Quat, Vec3, Vec4};

/// Bones to compute (`None`: all). The others keep an unspecified pose; a set holds every
/// ancestor of its bones, so their model matrices are exact.
pub type Bones<'a> = Option<&'a [bool]>;
pub(super) fn wanted(bones: Bones, bone: usize) -> bool {
    bones.is_none_or(|b| b.get(bone).copied().unwrap_or(true))
}
pub(super) fn interpolate(a: &[Pose], b: &[Pose], t: f32, bones: Bones) -> Vec<Pose> {
    (a.iter().zip(b).enumerate())
        .map(|(i, (p, q))| match wanted(bones, i) {
            true => Pose {
                position: p.position.lerp(q.position, t),
                rotation: p.rotation.slerp(q.rotation, t),
            },
            false => p.clone(),
        })
        .collect()
}
/// SDK BlendBones: QuaternionBlend toward `layer` by `s` for bones with sequence weight.
pub(super) fn blend_bones(
    base: &mut [Pose],
    layer: &[Pose],
    info: &ClipLayer,
    s: f32,
    bones: Bones,
) {
    let s1 = 1. - s;
    for (i, (q1, q2)) in base.iter_mut().zip(layer).enumerate() {
        if !wanted(bones, i) || info.bone_weights.get(i).is_some_and(|w| *w <= 0.) {
            continue;
        }
        let aligned = quaternion_align(q2.rotation, q1.rotation);
        q1.rotation =
            Quat::from_vec4(Vec4::from(q2.rotation) * (1. - s1) + Vec4::from(aligned) * s1)
                .normalize();
        q1.position = q1.position * s1 + q2.position * s;
    }
}
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
    slerp_bones_of(base, layer, info, weight, None)
}
/// `slerp_bones` on `bones` only.
pub(super) fn slerp_bones_of(
    base: &mut [Pose],
    layer: &[Pose],
    info: &ClipLayer,
    weight: f32,
    bones: Bones,
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
        if s2 <= 0. || !wanted(bones, i) {
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
