//! What a projectile's end does besides its blast: the grenades' view shake
//! (CBaseGrenade::Detonate, which npc_grenade_frag inherits, and CGrenadeAR2::Detonate both
//! call UTIL_ScreenShake(origin, 25, 150, 1 s, 750, SHAKE_START); env_explosion, explosive
//! props and the RPG missile do not) and a projectile leaving the world.
use super::{ProjectileKind, Projectiles};
use crate::entities::Scene;
use glam::Vec3;

/// The grenades' UTIL_ScreenShake: amplitude, frequency, duration (s) and radius
/// (GetShakeAmplitude 25, GetShakeRadius 750; the retail server holds the same 750).
pub const GRENADE_SHAKE: (f32, f32, f32, f32) = (25., 150., 1., 750.);

impl Projectiles {
    /// A projectile past the world's bounds is removed; a missile reports that it died.
    pub(super) fn left_world(&mut self, projectile: &super::Projectile) {
        self.diagnostics.out_of_world += 1;
        if projectile.kind == ProjectileKind::RpgMissile {
            if let Some(owner) = projectile.npc_owner() {
                self.npc_missiles_died.push(owner);
            } else {
                self.missile_events
                    .push(crate::weapon_rpg::MissileEvent::Removed);
            }
        }
    }
}

/// A grenade's detonation shake for the player standing at `player_feet`.
pub(super) fn grenade_shake(scene: &mut Scene, origin: Vec3, player_feet: Vec3) {
    let now = scene.time;
    scene.shakes.start(origin, GRENADE_SHAKE, player_feet, now);
}
