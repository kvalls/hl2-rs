//! Source right-handed Z-up coordinates and Bevy right-handed Y-up ones.
use bevy::math::Vec3;

/// A Source position or direction in Bevy's axes.
pub fn source_to_bevy(p: Vec3) -> Vec3 {
    Vec3::new(p.x, p.z, -p.y)
}
/// A Bevy position or direction in Source's axes.
pub fn bevy_to_source(p: Vec3) -> Vec3 {
    Vec3::new(p.x, -p.z, p.y)
}
/// The Source direction of a yaw and pitch (radians, pitch positive up).
pub fn source_direction(yaw: f32, pitch: f32) -> Vec3 {
    Vec3::new(
        pitch.cos() * yaw.cos(),
        pitch.cos() * yaw.sin(),
        pitch.sin(),
    )
}
