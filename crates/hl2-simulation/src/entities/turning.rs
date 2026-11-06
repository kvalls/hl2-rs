//! Yaw arithmetic of scenes and scripted turns.

/// UTIL_AngleDiff: signed shortest difference `a - b` in degrees.
pub(super) fn angle_diff(a: f32, b: f32) -> f32 {
    let mut d = (a - b) % 360.;
    if d > 180. {
        d -= 360.;
    } else if d < -180. {
        d += 360.;
    }
    d
}
/// AI_ClampYaw: move `current` toward `target` by at most `speed` degrees.
pub(super) fn clamp_yaw(speed: f32, current: f32, target: f32) -> f32 {
    let step = angle_diff(target, current).clamp(-speed, speed);
    (current + step).rem_euclid(360.)
}
