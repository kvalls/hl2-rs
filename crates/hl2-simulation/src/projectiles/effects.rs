//! The timed visual effects of projectiles and bullet impacts.
use glam::Vec3;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum EffectKind {
    GrenadeExplosion,
    BallImpact,
    BallExplosion,
    /// The crossbow bolt's glow sprite, turned on where it stuck and faded over 3 s.
    BoltGlow,
    /// g_pEffects->Sparks: where a bolt hit the world, and a scanner breaking up.
    Sparks,
    /// The bug bait's SporeExplosion (env_sporeexplosion): emits for 5 s
    /// (bugbait_distract_time) toward `normal`, its last particles live 2 s more.
    SporeExplosion,
    /// PerformCustomEffects by surface: FX_DebrisFlecks on concrete and tile, then wood.
    Flecks,
    WoodFlecks,
    /// FX_DustImpact on dirt and sand.
    Dust,
    /// FX_MetalSpark on metal and vents (`normal` is the shot's reflection).
    MetalSpark,
    /// One puff of a damaged scanner's SmokeTrail (`normal` is the body's velocity).
    TrailSmoke,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Effect {
    pub id: u64,
    pub magnitude: f32,
    pub kind: EffectKind,
    pub position: Vec3,
    pub normal: Vec3,
    pub at: f64,
    pub radius: f32,
}
impl Effect {
    pub fn duration(&self) -> f64 {
        match self.kind {
            EffectKind::BallImpact => 4.,
            EffectKind::GrenadeExplosion => 3.,
            EffectKind::BallExplosion => 8.,
            EffectKind::BoltGlow => crate::weapon_crossbow::STUCK_GLOW_TIME,
            EffectKind::Sparks | EffectKind::MetalSpark => 0.5,
            EffectKind::Flecks | EffectKind::WoodFlecks => 3.,
            EffectKind::Dust => 1.,
            EffectKind::TrailSmoke => 2.,
            EffectKind::SporeExplosion => f64::from(crate::weapon_bugbait::DISTRACT_TIME) + 2.,
        }
    }
}
