//! What a think knows: the facts a class and the generic tasks read.
use super::super::bird::FlyHint;
use glam::Vec3;

/// Per-think facts the class and the generic tasks read.
#[derive(Clone, Debug, Default)]
pub struct Context {
    pub now: f64,
    pub origin: Vec3,
    pub enemy: Option<(usize, Vec3)>,
    pub target: Option<Vec3>,
    pub best_sound: Option<Vec3>,
    /// The local player's position (TASK_FACE_PLAYER), when there is one.
    pub player: Option<Vec3>,
    /// The NPC's yaw in degrees.
    pub yaw: f32,
    /// A uniform [0, 1) draw for this think (random waits, sentence choice).
    pub random: f32,
    /// The enemy's EyePosition (its feet plus view offset), with `enemy`.
    pub enemy_eye: Option<Vec3>,
    /// The enemy's last known position (enemy memory), with `enemy`.
    pub enemy_lkp: Option<Vec3>,
    /// The active weapon's (min, max) range (MIN/MAX of m_fMin/MaxRange1/2).
    pub weapon_range: Option<(f32, f32)>,
    /// The active weapon's GetMinBurst, GetMaxBurst and GetFireRate.
    pub weapon_burst: Option<(u32, u32, f32)>,
    /// The best sound's volume (CSound::Volume), with `best_sound`.
    pub best_sound_volume: f32,
    /// m_flDistTooFar (1024; SF_NPC_LONG_RANGE: 1e9).
    pub dist_too_far: f32,
    /// Whether the enemy has me in its view cone (FInViewCone), with `enemy`.
    pub enemy_sees_me: Option<bool>,
    /// Off the ground (falling: FL_ONGROUND clear).
    pub airborne: bool,
    /// bits_CAP_INNATE_RANGE_ATTACK1: InnateRange1MinRange / InnateRange1MaxRange.
    pub innate_range: Option<(f32, f32)>,
    /// Where a bird can fly; None for other classes.
    pub fly_hints: Option<std::sync::Arc<[FlyHint]>>,
}
