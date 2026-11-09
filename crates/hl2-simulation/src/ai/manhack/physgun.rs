//! The gravity gun's hand on a manhack (DESIGN 14p session 50): OnPhysGunPickup, OnPhysGunDrop,
//! HasPhysicsAttacker and Slice's damage rules. Retail's server (Slice, 10398730) has the SDK's
//! branches and constants: 100 a second held, the 0.35 s window, x6 and x2, at least 1.

use super::{Manhack, LEAST_CUT, PLAYER_SHARE, SMASH_TIME, STALL, STUNNED};
use crate::ai::metropolice::CopRequest;
use crate::weapon_physcannon::NpcPhysgun;

/// A held manhack cuts this much a second, whatever it cuts.
const HELD_CUT: f32 = 100.;
/// Slice's share on an npc_combine_s it hates.
const SOLDIER_SHARE: f32 = 6.;
/// DetachEntity's limit on a plain drop: 1.5 x hl2_normspeed.
const DROP_SPEED: f32 = 285.;

/// What a slice cut.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Victim {
    Player,
    /// An NPC; an npc_combine_s it hates takes six times as much.
    Npc {
        hated_soldier: bool,
    },
    /// A prop or breakable, with the health it has left: it breaks at once.
    Breakable(f32),
}

impl Manhack {
    /// What the gravity gun did to it this tick. True when it thinks at once, as the SDK's next
    /// think 0.01 s on.
    pub fn physgun(&mut self, event: NpcPhysgun, now: f64) -> bool {
        match event {
            NpcPhysgun::Punt(push) => {
                self.punted(push, now);
                true
            }
            NpcPhysgun::Push(speed) => {
                self.hover.velocity += speed;
                false
            }
            // OnPhysGunPickup: the player is its physics attacker and it is held.
            NpcPhysgun::Pickup => {
                self.held = true;
                self.punted_at = Some(now);
                false
            }
            NpcPhysgun::Hold { velocity, yaw } => {
                if self.held {
                    self.hover.velocity = velocity;
                    self.held_yaw = Some(yaw);
                }
                false
            }
            // OnPhysGunDrop(DROPPED_BY_CANNON): no physics attacker; DetachEntity keeps its speed
            // under 1.5 x hl2_normspeed.
            NpcPhysgun::Drop { launch: None } => {
                self.let_go();
                self.punted_at = None;
                self.hover.velocity = self.hover.velocity.clamp_length_max(DROP_SPEED);
                false
            }
            // LAUNCHED_BY_CANNON: cleared, launched, stalled and stunned, the player its physics
            // attacker from now.
            NpcPhysgun::Drop {
                launch: Some(speed),
            } => {
                self.let_go();
                self.hover.velocity = speed.clamp_length_max(crate::physics::MAX_VELOCITY);
                self.stun(now);
                true
            }
        }
    }
    fn let_go(&mut self) {
        self.held = false;
        self.held_yaw = None;
    }
    /// The engine stalls for 2 s, the player is its physics attacker from now, and the eye is
    /// stunned.
    pub(super) fn stun(&mut self, now: f64) {
        self.stalled_until = now + STALL;
        self.punted_at = Some(now);
        self.stun_eye();
    }
    /// SetEyeState(MANHACK_EYE_STATE_STUNNED), which says so.
    pub(super) fn stun_eye(&mut self) {
        self.stunned = true;
        self.requests.push(CopRequest::Sound(STUNNED));
    }
    /// HasPhysicsAttacker(MANHACK_SMASH_TIME) and not held: thrown by the player moments ago.
    pub fn thrown(&self, now: f64) -> bool {
        !self.held && self.punted_at.is_some_and(|at| now - at <= SMASH_TIME)
    }
    /// HasPhysicsAttacker(MANHACK_SMASH_TIME): held, or thrown moments ago. Its cuts are the
    /// player's then.
    pub fn physics_attacker(&self, now: f64) -> bool {
        self.held || self.thrown(now)
    }
    /// Slice's damage for `interval` seconds of contact, at `per_second` (sk_manhack_melee_dmg);
    /// `guard_health` is sk_combine_guard_health.
    pub fn slice_damage(
        &self,
        (per_second, interval): (f32, f32),
        victim: Victim,
        guard_health: f32,
        now: f64,
    ) -> f32 {
        let mut damage = per_second * interval;
        if victim == Victim::Player {
            damage *= PLAYER_SHARE;
        }
        damage = match victim {
            _ if self.held => HELD_CUT * interval,
            Victim::Npc { .. } if self.thrown(now) => guard_health,
            Victim::Breakable(health) => health,
            Victim::Npc {
                hated_soldier: true,
            } => damage * SOLDIER_SHARE,
            _ => damage,
        };
        damage.max(LEAST_CUT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn manhack() -> Manhack {
        Manhack::new(false, 1)
    }
    const NPC: Victim = Victim::Npc {
        hated_soldier: false,
    };

    #[test]
    fn a_free_manhack_cuts_by_the_second_twice_on_the_player_and_at_least_one() {
        let m = manhack();
        assert_eq!(m.slice_damage((20., 0.1), NPC, 70., 0.), 2.);
        assert_eq!(m.slice_damage((20., 0.1), Victim::Player, 70., 0.), 4.);
        assert_eq!(m.slice_damage((5., 0.1), NPC, 70., 0.), 1.);
        let soldier = Victim::Npc {
            hated_soldier: true,
        };
        assert_eq!(m.slice_damage((20., 0.1), soldier, 70., 0.), 12.);
        assert_eq!(
            m.slice_damage((20., 0.1), Victim::Breakable(55.), 70., 0.),
            55.
        );
    }
    #[test]
    fn held_it_cuts_a_hundred_a_second_and_thrown_it_takes_a_guards_health_from_an_npc() {
        let mut m = manhack();
        assert!(!m.physgun(NpcPhysgun::Pickup, 1.));
        assert!(m.held && m.physics_attacker(5.) && !m.thrown(1.));
        assert_eq!(m.slice_damage((20., 0.1), NPC, 70., 1.), 10.);
        assert_eq!(
            m.slice_damage((20., 0.1), Victim::Breakable(55.), 70., 1.),
            10.
        );
        let launch = Some(Vec3::X * 1500.);
        assert!(m.physgun(NpcPhysgun::Drop { launch }, 2.));
        assert!(!m.held && m.thrown(2.2) && m.stalled(3.9) && m.stunned);
        assert_eq!(m.hover.velocity, Vec3::X * 1500.);
        assert_eq!(m.slice_damage((20., 0.1), NPC, 70., 2.2), 70.);
        // The window closes after 0.35 s, and a breakable takes its health either way.
        assert_eq!(m.slice_damage((20., 0.1), NPC, 70., 2.4), 2.);
        assert_eq!(
            m.slice_damage((20., 0.1), Victim::Breakable(9.), 70., 2.2),
            9.
        );
    }
    #[test]
    fn the_hold_steers_it_and_a_plain_drop_keeps_a_walkers_pace_and_forgets_the_player() {
        let mut m = manhack();
        let velocity = Vec3::new(600., 0., 0.);
        m.physgun(NpcPhysgun::Hold { velocity, yaw: 30. }, 0.5);
        assert_eq!(
            m.hover.velocity,
            Vec3::ZERO,
            "not held: the hold is ignored"
        );
        m.physgun(NpcPhysgun::Pickup, 1.);
        m.physgun(NpcPhysgun::Hold { velocity, yaw: 30. }, 1.1);
        assert_eq!((m.hover.velocity, m.held_yaw), (velocity, Some(30.)));
        assert!(!m.physgun(NpcPhysgun::Drop { launch: None }, 1.2));
        assert!(!m.held && m.held_yaw.is_none() && !m.physics_attacker(1.2));
        assert_eq!(m.hover.velocity, Vec3::X * DROP_SPEED);
        assert!(!m.stalled(1.2) && !m.stunned);
    }
    #[test]
    fn no_smash_while_held() {
        let mut m = manhack();
        m.physgun(NpcPhysgun::Pickup, 1.);
        assert_eq!(m.smash(1000., 1.1), None);
        m.physgun(
            NpcPhysgun::Drop {
                launch: Some(Vec3::X * 1500.),
            },
            2.,
        );
        assert_eq!(m.smash(1000., 2.1), Some(2.));
    }
}
