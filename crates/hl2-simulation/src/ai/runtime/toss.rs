//! Grenade throw arcs: VecCheckThrow and VecCheckToss (h_ai.cpp).

use super::GRAVITY;
use crate::physics::Physics;
use glam::Vec3;

/// VecCheckThrow (h_ai.cpp): a throw at `speed` from `start` to `target` whose two
/// halves (start -> apex, target -> apex) are clear; None when blocked.
pub fn vec_check_throw(
    physics: &Physics,
    id: usize,
    start: Vec3,
    target: Vec3,
    speed: f32,
) -> Option<Vec3> {
    let delta = target - start;
    let time = delta.length() / speed;
    if time <= 0. {
        return None;
    }
    let mut velocity = delta / time;
    velocity.z += GRAVITY * time * 0.5;
    let mut apex = start + delta * 0.5;
    apex.z += 0.5 * GRAVITY * (time * 0.5) * (time * 0.5);
    (trace_clear(physics, start, apex, id) && trace_clear(physics, target, apex, id))
        .then_some(velocity)
}
/// VecCheckToss (h_ai.cpp) with no height ratio limit, gravity 1 and the randomized
/// target (`jitter`: four [0, 1) draws): the apex is the midpoint raised up to 300 units
/// (15 under a ceiling); None when either half is blocked or the target is 500 above.
pub fn vec_check_toss(
    physics: &Physics,
    id: usize,
    start: Vec3,
    target: Vec3,
    forward: Vec3,
    jitter: [f32; 4],
) -> Option<Vec3> {
    if target.z - start.z > 500. {
        return None;
    }
    let right = forward.cross(Vec3::Z).normalize_or_zero();
    let r = |u: f32, a: f32| -a + 2. * a * u;
    let target = target
        + right * (r(jitter[0], 8.) + r(jitter[1], 16.))
        + forward * (r(jitter[2], 8.) + r(jitter[3], 16.));
    let mid = start + (target - start) * 0.5;
    let mid = match physics.projectile_ray(mid, mid + Vec3::Z * 300., &[id]) {
        Some(hit) => hit.position - Vec3::Z * 15.,
        None => mid + Vec3::Z * 300.,
    };
    if mid.z < start.z || mid.z < target.z {
        return None;
    }
    let time1 = ((mid.z - start.z) / (0.5 * GRAVITY)).sqrt();
    let time2 = ((mid.z - target.z) / (0.5 * GRAVITY)).sqrt();
    if time1 < 0.1 {
        return None;
    }
    let mut velocity = (target - start) / (time1 + time2);
    velocity.z = GRAVITY * time1;
    let mut apex = start + velocity * time1;
    apex.z = mid.z;
    (trace_clear(physics, start, apex, id) && trace_clear(physics, target, apex, id))
        .then_some(velocity)
}
/// A throw trace's `fraction == 1`: Source ignores the solid a trace starts on or in
/// (startsolid) and only fails on a surface it enters. The flush target is the enemy's
/// last seen feet position, on the floor, where our ray reports a hit at its own start
/// (before this, every packaged flush toss failed).
fn trace_clear(physics: &Physics, from: Vec3, to: Vec3, id: usize) -> bool {
    match physics.projectile_ray(from, to, &[id]) {
        None => true,
        Some(hit) if hit.position.distance(from) < 0.5 => {
            let step = (to - from).normalize_or_zero();
            physics.projectile_ray(from + step, to, &[id]).is_none()
        }
        Some(_) => false,
    }
}
