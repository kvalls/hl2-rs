//! OnTakeDamage_Alive's reactions (DESIGN 14p session 50). Retail's handler (server 103972e0)
//! has the SDK's constants. The push runs along the attack, away from the attacker: the manhack's
//! TraceAttack sets g_vecAttackDir to the attack's direction before the handler reads it.

use super::{Manhack, PHYSICS_MASS};
use crate::ai::metropolice::CopRequest;
use crate::entities::NpcDamage;
use crate::player_damage::{DMG_CLUB, DMG_PHYSGUN};

/// A club deals half as much again (tdInfo.ScaleDamage(1.5)).
pub const CLUB_SHARE: f32 = 1.5;
/// A club's push: VPhysicsTakeDamage's force of damage x 200 and m_vForceVelocity of damage x
/// 0.5, then 0.5 s of stalled engine.
const CLUB_FORCE: f32 = 200.;
const CLUB_PUSH: f32 = 0.5;
const CLUB_STALL: f64 = 0.5;
/// Any other hit: VPhysicsTakeDamage with the hit's own force (the x 20 goes only to the copy the
/// base class reads, retail as the SDK), m_vForceVelocity of damage x 20, 0.25 s stalled.
const HIT_PUSH: f32 = 20.;
const HIT_STALL: f64 = 0.25;
const BAT: &str = "NPC_Manhack.Bat";

impl Manhack {
    /// A hit's reaction. A club bats it away (NPC_Manhack.Bat, the burst ends, the eye is
    /// stunned); anything else is shoved along the hit. Both stall the engine, replacing a longer
    /// stall as the SDK does. The punt's DMG_PHYSGUN is the gravity gun's own (`punted`); DMG_GENERIC
    /// carries no force. Not modeled: the club's turn toward a light physics object ahead.
    pub fn hurt(&mut self, report: &NpcDamage, (health, max): (f32, f32), now: f64) {
        // Alive at half health or less (integer halves), the smoke trail starts.
        let half = health > 0. && health <= (max / 2.).floor();
        self.smoking |= half && !self.no_damage_effects;
        if report.kind & DMG_PHYSGUN != 0 {
            return;
        }
        let along = report.force.normalize_or_zero();
        if report.kind & DMG_CLUB != 0 {
            // The force reads the damage before the 1.5.
            let base = report.damage / CLUB_SHARE;
            self.hover.velocity += along * base * (CLUB_FORCE / PHYSICS_MASS + CLUB_PUSH);
            self.requests.push(CopRequest::Sound(BAT));
            self.stalled_until = now + CLUB_STALL;
            self.stop_burst(now);
            self.stun_eye();
            return;
        }
        let impulse = if report.kind == 0 {
            glam::Vec3::ZERO
        } else {
            report.force / PHYSICS_MASS
        };
        self.hover.velocity += impulse + along * report.damage * HIT_PUSH;
        self.stalled_until = now + HIT_STALL;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player_damage::DMG_BULLET;
    use glam::Vec3;

    #[test]
    fn a_crowbar_bats_it_away_along_the_swing_and_stalls_it_half_a_second() {
        let mut m = Manhack::new(false, 5);
        m.start_burst(Vec3::X, 9.);
        m.requests.clear();
        // A crowbar's 10, scaled to 15 by the club rule.
        let report = NpcDamage::new(1, true, 15., Vec3::X * 3000., DMG_CLUB);
        m.hurt(&report, (10., 25.), 9.5);
        let speed = 10. * (CLUB_FORCE / PHYSICS_MASS + CLUB_PUSH);
        assert!(
            (m.hover.velocity.x - speed).abs() < 1e-3,
            "{}",
            m.hover.velocity.x
        );
        assert!(m.stalled(9.99) && !m.stalled(10.));
        assert!(
            m.stunned && !m.bursting(9.6) && m.smoking,
            "10 of 25 smokes"
        );
        assert_eq!(m.requests.len(), 2, "Bat and Stunned");
    }
    #[test]
    fn a_bullet_shoves_it_along_the_shot_for_a_quarter_second_and_a_punt_is_left_to_the_gun() {
        let mut m = Manhack::new(false, 5);
        let report = NpcDamage::new(1, true, 5., Vec3::Y * 77., DMG_BULLET);
        m.hurt(&report, (20., 25.), 3.);
        let speed = 77. / PHYSICS_MASS + 5. * HIT_PUSH;
        assert!((m.hover.velocity.y - speed).abs() < 1e-3);
        assert!(m.stalled(3.24) && !m.stalled(3.25) && !m.stunned && !m.smoking);
        let mut quiet = Manhack::new(false, 5);
        quiet.no_damage_effects = true;
        quiet.hurt(&report, (12., 25.), 3.);
        assert!(!quiet.smoking);
        let mut dying = Manhack::new(false, 5);
        dying.hurt(&report, (0., 25.), 3.);
        assert!(!dying.smoking, "a dead one makes no trail");
        let mut half = Manhack::new(false, 5);
        half.hurt(&report, (13., 25.), 3.);
        assert!(!half.smoking, "13 is over the integer half of 25");
        let mut punted = Manhack::new(false, 5);
        punted.hurt(
            &NpcDamage::new(1, true, 1., Vec3::Y, DMG_PHYSGUN),
            (24., 25.),
            3.,
        );
        assert_eq!(
            (punted.hover.velocity, punted.stalled(3.1)),
            (Vec3::ZERO, false)
        );
    }
}
