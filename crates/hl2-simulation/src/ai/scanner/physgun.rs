//! The gravity gun's hand on a scanner (DESIGN 14q session 51; Source SDK hl2/npc_basescanner.cpp
//! OnPhysGunPickup, OnPhysGunDrop, HasPhysicsAttacker, VPhysicsCollision,
//! TakeDamageFromPhyscannon, MoveToTarget, SCHED_SCANNER_HELD_BY_PHYSCANNON). Retail's server
//! carries the same schedule and its two conditions by name; owned combine_scanner.phy gives the
//! body's mass and size.

use super::{Scanner, DIVE_BOMB};
use crate::ai::metropolice::CopRequest;
use crate::weapon_physcannon::{NpcPhysgun, PhysgunNpc, DROP_SPEED};
use glam::Vec3;
use serde::Serialize;

/// combine_scanner.phy's first solid, Scanner.Body: its mass and half its extent.
pub const PHYSICS_MASS: f32 = 8.701_758;
pub const PHYSICS_EXTENTS: Vec3 = Vec3::new(15., 14.5, 14.2);
/// A punt or a launch stalls the engine this long.
const STALL: f64 = 2.;
/// SCANNER_SMASH_TIME and SCANNER_SMASH_SPEED: thrown, a hit within this time costs its max health
/// at this speed, up to twice that.
const SMASH_TIME: f64 = 0.75;
const SMASH_SPEED: f32 = 250.;
/// How fast it turns to the yaw the hold keeps, in degrees a second.
pub const HELD_TURN_RATE: f32 = 3600.;

pub const HELD: &str = "SCHED_SCANNER_HELD_BY_PHYSCANNON";
/// Held, it idles: a hit or its release ends that.
pub const HELD_SCHEDULE: (&str, &str) = (
    HELD,
    "Tasks TASK_SET_ACTIVITY ACTIVITY:ACT_IDLE TASK_WAIT 5 \
     Interrupts COND_LIGHT_DAMAGE COND_HEAVY_DAMAGE",
);

/// What the gravity gun has done to it.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Gun {
    /// FVPHYSICS_PLAYER_HELD, and the yaw the hold keeps, in degrees.
    pub held: bool,
    pub held_yaw: Option<f32>,
    /// m_flEngineStallTime.
    pub stalled_until: f64,
    /// m_flLastPhysicsInfluenceTime, with the player as m_hPhysicsAttacker.
    pub influenced_at: Option<f64>,
    /// COND_SCANNER_RELEASED_FROM_PHYSCANNON, until its AI has seen it.
    pub released: bool,
}

/// How the gun's list names a scanner. A punt's DMG_PHYSGUN carries no damage, and a scanner
/// makes nothing of it.
pub fn listing(id: usize) -> PhysgunNpc {
    PhysgunNpc {
        id,
        mass: PHYSICS_MASS,
        extents: PHYSICS_EXTENTS,
        punt_damage: 0.,
    }
}

impl Scanner {
    /// What the gravity gun did to it this tick. True when it thinks at once, as the SDK's next
    /// think 0.01 s on.
    pub fn physgun(&mut self, event: NpcPhysgun, now: f64) -> bool {
        match event {
            NpcPhysgun::Punt(push) => {
                let speed = self.hover.velocity + push;
                self.hover.velocity = speed.clamp_length_max(crate::physics::MAX_VELOCITY);
                self.throw(now);
                true
            }
            NpcPhysgun::Push(speed) => {
                self.hover.velocity += speed;
                false
            }
            NpcPhysgun::Pickup => {
                self.gun.held = true;
                self.gun.released = false;
                self.gun.influenced_at = Some(now);
                false
            }
            NpcPhysgun::Hold { velocity, yaw } => {
                if self.gun.held {
                    self.hover.velocity = velocity;
                    self.gun.held_yaw = Some(yaw);
                }
                false
            }
            // DROPPED_BY_CANNON: DetachEntity keeps its speed under 1.5 x hl2_normspeed.
            NpcPhysgun::Drop { launch: None } => {
                self.let_go(now);
                self.hover.velocity = self.hover.velocity.clamp_length_max(DROP_SPEED);
                false
            }
            NpcPhysgun::Drop {
                launch: Some(speed),
            } => {
                self.let_go(now);
                self.hover.velocity = speed.clamp_length_max(crate::physics::MAX_VELOCITY);
                self.throw(now);
                true
            }
        }
    }
    /// OnPhysGunDrop: the player stays its physics attacker from now.
    fn let_go(&mut self, now: f64) {
        self.gun.released = self.gun.held;
        self.gun.held = false;
        self.gun.held_yaw = None;
        self.gun.influenced_at = Some(now);
    }
    /// PUNTED_BY_CANNON and LAUNCHED_BY_CANNON: the engine stalls and it screams as in a dive.
    fn throw(&mut self, now: f64) {
        self.gun.stalled_until = now + STALL;
        self.gun.influenced_at = Some(now);
        self.requests.push(CopRequest::Sound(DIVE_BOMB));
    }
    /// MoveToTarget does nothing while the engine is stalled, and the hold flies it while held.
    pub fn grounded(&self, now: f64) -> bool {
        self.gun.held || now < self.gun.stalled_until
    }
    /// HasPhysicsAttacker(SCANNER_SMASH_TIME) and not held: thrown by the player moments ago.
    pub fn thrown(&self, now: f64) -> bool {
        !self.gun.held
            && self
                .gun
                .influenced_at
                .is_some_and(|at| now - at <= SMASH_TIME)
    }
    /// TakeDamageFromPhyscannon for a hit at this speed: the share of its max health it costs.
    /// Held it takes no impact damage, and long after a throw none of this kind.
    pub fn smash(&self, speed: f32, now: f64) -> Option<f32> {
        self.thrown(now)
            .then(|| (speed / SMASH_SPEED).clamp(0., 2.))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scanner() -> Scanner {
        Scanner::default()
    }

    #[test]
    fn held_it_follows_the_hold_and_a_plain_drop_keeps_a_walkers_pace() {
        let mut s = scanner();
        assert!(!s.physgun(NpcPhysgun::Pickup, 1.));
        assert!(s.gun.held && s.grounded(1.));
        let hold = NpcPhysgun::Hold {
            velocity: Vec3::new(600., 0., 0.),
            yaw: 90.,
        };
        s.physgun(hold, 1.1);
        assert_eq!(s.hover.velocity, Vec3::new(600., 0., 0.));
        assert_eq!(s.gun.held_yaw, Some(90.));
        // Held, it takes no impact damage.
        assert_eq!(s.smash(600., 1.1), None);
        s.physgun(NpcPhysgun::Drop { launch: None }, 2.);
        assert!(!s.gun.held && s.gun.released && !s.grounded(2.));
        assert_eq!(s.hover.velocity.length(), DROP_SPEED);
        assert!(s.requests.is_empty());
    }

    #[test]
    fn a_launch_stalls_it_two_seconds_and_a_wall_soon_after_costs_twice_its_health() {
        let mut s = scanner();
        s.physgun(NpcPhysgun::Pickup, 1.);
        let launch = Some(Vec3::new(1500., 0., 0.));
        assert!(s.physgun(NpcPhysgun::Drop { launch }, 2.));
        assert_eq!(s.hover.velocity.x, 1500.);
        assert!(s.grounded(3.9) && !s.grounded(4.1));
        assert!(matches!(s.requests[..], [CopRequest::Sound(DIVE_BOMB)]));
        assert_eq!(s.smash(1500., 2.5), Some(2.));
        assert_eq!(s.smash(125., 2.75), Some(0.5));
        assert_eq!(s.smash(1500., 2.76), None);
    }

    #[test]
    fn a_punt_adds_its_push_and_stalls_and_a_pull_only_adds_speed() {
        let mut s = scanner();
        assert!(!s.physgun(NpcPhysgun::Push(Vec3::X * 40.), 1.));
        assert!(!s.grounded(1.) && !s.thrown(1.));
        assert!(s.physgun(NpcPhysgun::Punt(Vec3::X * 200.), 1.));
        assert_eq!(s.hover.velocity.x, 240.);
        assert!(s.grounded(2.9) && s.thrown(1.5));
    }
}
