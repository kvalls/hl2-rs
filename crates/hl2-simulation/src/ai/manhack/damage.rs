//! OnTakeDamage_Alive's reactions (DESIGN 14p session 50). Retail's handler (server 103972e0)
//! has the SDK's constants. The push runs along the attack, away from the attacker: the manhack's
//! TraceAttack sets g_vecAttackDir to the attack's direction before the handler reads it.

use super::{Manhack, PHYSICS_MASS};
use crate::ai::metropolice::CopRequest;
use crate::entities::NpcDamage;
use crate::player_damage::{DMG_CLUB, DMG_PHYSGUN};
use glam::Vec3;

/// A club deals half as much again (tdInfo.ScaleDamage(1.5)).
pub const CLUB_SHARE: f32 = 1.5;
/// A club's push: VPhysicsTakeDamage's force of damage x 200 and m_vForceVelocity of damage x
/// 0.5, then 0.5 s of stalled engine.
const CLUB_FORCE: f32 = 200.;
const CLUB_PUSH: f32 = 0.5;
const CLUB_STALL: f64 = 0.5;
/// What the club may knock it into: a physics object of at most 200 kg whose middle lies in a box
/// of 128 units each way, 128 ahead along the swing, lined up within a dot of 0.9.
pub const CLUB_SEARCH: f32 = 128.;
pub const CLUB_MAX_MASS: f32 = 200.;
const CLUB_BEST_DOT: f32 = 0.9;
/// Any other hit: VPhysicsTakeDamage with the hit's own force (the x 20 goes only to the copy the
/// base class reads, retail as the SDK), m_vForceVelocity of damage x 20, 0.25 s stalled.
const HIT_PUSH: f32 = 20.;
const HIT_STALL: f64 = 0.25;
const BAT: &str = "NPC_Manhack.Bat";
/// CorpseGib: clubbed to death, the pieces fly at 150 u/s and spin up to 2000 degrees a second
/// about each axis; otherwise they keep its velocity and spin up to 500.
const CLUB_GIB_SPEED: f32 = 150.;
const CLUB_GIB_SPIN: f32 = 2000.;
const GIB_SPIN: f32 = 500.;

impl Manhack {
    /// A hit's reaction. A club bats it away (NPC_Manhack.Bat, the burst ends, the eye is
    /// stunned); anything else is shoved along the hit. Both stall the engine, replacing a longer
    /// stall as the SDK does. The punt's DMG_PHYSGUN is the gravity gun's own (`punted`); DMG_GENERIC
    /// carries no force. Not modeled: the club's turn toward a light physics object ahead.
    pub fn hurt(&mut self, report: &NpcDamage, (health, max): (f32, f32), now: f64) {
        // Alive at half health or less (integer halves), the smoke trail starts.
        let half = health > 0. && health <= (max / 2.).floor();
        self.smoking |= half && !self.no_damage_effects;
        if health <= 0. {
            self.club_kill =
                (report.kind & DMG_CLUB != 0).then_some(report.force.normalize_or_zero());
        }
        if report.kind & DMG_PHYSGUN != 0 {
            return;
        }
        let along = report.force.normalize_or_zero();
        if report.kind & DMG_CLUB != 0 {
            // The force reads the damage before the 1.5; the stage aims it (`bat`).
            self.batted = Some((along, report.damage / CLUB_SHARE));
            self.requests.push(CopRequest::Sound(BAT));
            self.stalled_until = now + CLUB_STALL;
            self.stop_burst(now);
            self.stun_eye();
            return;
        }
        let impulse = if report.kind == 0 {
            Vec3::ZERO
        } else {
            report.force / PHYSICS_MASS
        };
        self.hover.velocity += impulse + along * report.damage * HIT_PUSH;
        self.stalled_until = now + HIT_STALL;
    }
    /// CorpseGib's motion for the pieces, radians a second for the spin. Clubbed, they fly away from
    /// the attacker: the SDK negates g_vecAttackDir, which the base class has pointed at the
    /// inflictor by then; the swing's own direction stands in for that here.
    pub fn gib_motion(&mut self) -> (Vec3, Vec3) {
        let (velocity, spin) = match self.club_kill {
            Some(along) => (along * CLUB_GIB_SPEED, CLUB_GIB_SPIN),
            None => (self.hover.velocity, GIB_SPIN),
        };
        let spin = spin.to_radians();
        let axes = [self.dice(), self.dice(), self.dice()].map(|r| (r * 2. - 1.) * spin);
        (velocity, Vec3::from_array(axes))
    }
    /// The club's knock along `aim`: VPhysicsTakeDamage's damage x 200 over its mass and
    /// m_vForceVelocity's damage x 0.5.
    pub fn bat(&mut self, aim: Vec3, base: f32) {
        self.hover.velocity += aim * base * (CLUB_FORCE / PHYSICS_MASS + CLUB_PUSH);
    }
}

/// Where a club knocks it from `origin` along `along`: toward the candidate (its middle and mass)
/// in the box ahead that lines up best, else along the swing.
pub fn club_aim(origin: Vec3, along: Vec3, candidates: impl Iterator<Item = (Vec3, f32)>) -> Vec3 {
    let ahead = origin + along * CLUB_SEARCH;
    let reach = Vec3::splat(CLUB_SEARCH);
    let inside = |c: Vec3| c.cmpge(ahead - reach).all() && c.cmple(ahead + reach).all();
    let mut best = (CLUB_BEST_DOT, along);
    for (middle, mass) in candidates {
        if mass > CLUB_MAX_MASS || !inside(middle) {
            continue;
        }
        let to = (middle - origin).normalize_or_zero();
        if along.dot(to) > best.0 {
            best = (along.dot(to), to);
        }
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player_damage::DMG_BULLET;

    #[test]
    fn a_crowbar_bats_it_away_along_the_swing_and_stalls_it_half_a_second() {
        let mut m = Manhack::new(false, 5);
        m.start_burst(Vec3::X, 9.);
        m.requests.clear();
        // A crowbar's 10, scaled to 15 by the club rule.
        let report = NpcDamage::new(1, true, 15., Vec3::X * 3000., DMG_CLUB);
        m.hurt(&report, (10., 25.), 9.5);
        assert_eq!(m.batted, Some((Vec3::X, 10.)));
        m.bat(Vec3::X, 10.);
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
    fn clubbed_to_death_its_pieces_fly_on_at_150_and_spin_harder() {
        let mut m = Manhack::new(false, 5);
        m.hover.velocity = Vec3::Y * 300.;
        let shot = NpcDamage::new(
            1,
            true,
            30.,
            Vec3::X * 77.,
            crate::player_damage::DMG_BULLET,
        );
        m.hurt(&shot, (-5., 25.), 1.);
        let (velocity, spin) = m.gib_motion();
        assert_eq!(
            velocity, m.hover.velocity,
            "shot dead: its own velocity, shove and all"
        );
        assert!(spin.abs().max_element() <= 500f32.to_radians());
        let mut c = Manhack::new(false, 5);
        let club = NpcDamage::new(1, true, 30., Vec3::X * 3000., DMG_CLUB);
        c.hurt(&club, (-5., 25.), 1.);
        let (velocity, spin) = c.gib_motion();
        assert_eq!(velocity, Vec3::X * 150.);
        assert!(spin.abs().max_element() <= 2000f32.to_radians());
        assert!(spin.abs().max_element() > 500f32.to_radians() * 0.5);
    }
    #[test]
    fn the_club_aims_it_at_the_best_lined_up_light_object_ahead() {
        let o = Vec3::ZERO;
        // Nothing near: along the swing.
        assert_eq!(club_aim(o, Vec3::X, std::iter::empty()), Vec3::X);
        let crate_ahead = (Vec3::new(100., 30., 0.), 20.);
        let heavy = (Vec3::new(100., 0., 0.), 500.);
        let wide = (Vec3::new(50., 50., 0.), 5.);
        let behind = (Vec3::new(-50., 0., 0.), 5.);
        let aim = club_aim(o, Vec3::X, [heavy, wide, behind, crate_ahead].into_iter());
        assert!((aim - Vec3::new(100., 30., 0.).normalize()).length() < 1e-5);
        // Past the box (more than 256 ahead) it is not seen.
        let far = (Vec3::new(300., 10., 0.), 5.);
        assert_eq!(club_aim(o, Vec3::X, [far].into_iter()), Vec3::X);
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
