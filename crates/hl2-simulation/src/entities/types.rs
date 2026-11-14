//! Report and event types of the scene.

use serde::Serialize;

#[derive(Serialize)]
pub struct ChoreographyState {
    pub entity: usize,
    pub targetname: String,
    pub file: String,
    pub elapsed: Option<f64>,
    pub pause: bool,
    pub pause_reason: Option<&'static str>,
    pub completed_early: bool,
    pub active: bool,
}
/// PhysGunPickup_t reasons that fire prop outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysGunPickup {
    /// PICKED_UP_BY_PLAYER (+USE).
    Player,
    /// PICKED_UP_BY_CANNON.
    Cannon,
    /// PUNTED_BY_CANNON.
    Punt,
}
#[derive(Serialize)]
pub struct SceneAnimationState {
    pub entity: usize,
    pub targetname: String,
    pub animation: String,
    pub sample_time: f32,
    pub scene_owner: Option<usize>,
    pub scripted_by: Option<usize>,
    pub clip_loaded: bool,
    pub origin: [f32; 3],
    pub yaw_degrees: f32,
}
