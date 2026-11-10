//! The manhack's body as its physics motor turns it (DESIGN 14q session 51). In the SDK it is a
//! VPhysics object: MoveToTarget leans it into its thrust (m_vTargetBanking), TurnHeadToTarget
//! names the yaw it wants, CAI_BasePhysicsFlyingBot::Simulate accelerates the body toward both
//! every physics tick, ClampMotorForces weakens that with the blades' speed, and whatever hits the
//! body spins it. Here the same motor runs on angles the AI keeps, and a knock adds the spin a
//! push at a lever arm gives a box of the .phy's first solid.

use super::{Manhack, PHYSICS_MASS};
use crate::physics::angles;
use glam::{Quat, Vec3};
use serde::Serialize;

/// MoveToTarget's banking: degrees at full thrust along its forward or its right.
const BANK: f32 = 40.;
/// m_vCurrentBanking closes this share of the gap to its target each think.
const BANK_RATE: f32 = 0.5;
/// Simulate's yaw looks this many seconds of spin ahead.
const YAW_LEAD: f32 = 0.1;
/// m_flBladeSpeed at full power, under which it doubles and adds one a think, and what it gains a
/// second above that (SpinBlades).
const BLADE_FULL: f32 = 100.;
const BLADE_SLOW: f32 = 10.;
const BLADE_GAIN: f32 = 80.;
/// SpinBlades draws the blade under this and the blur from that.
const BLADE_SHOWN_UNDER: f32 = 40.;
const BLUR_SHOWN_FROM: f32 = 20.;
/// CheckCollisions and a hit leave the blades at this; a club's blow at that.
pub const BLADE_HIT: f32 = 20.;
pub const BLADE_CLUBBED: f32 = 10.;
/// manhack.phy's rotdamping, a second.
const ROT_DAMPING: f32 = 1.5;
/// The body's moments about its forward, left and up, in kg in2: the first solid's box (13 long,
/// 10.7 wide, 5.5 high) at its mass, times the .phy's inertia of 5.
const MOMENTS: Vec3 = Vec3::new(
    PHYSICS_MASS / 12. * (10.7 * 10.7 + 5.46 * 5.46) * 5.,
    PHYSICS_MASS / 12. * (12.98 * 12.98 + 5.46 * 5.46) * 5.,
    PHYSICS_MASS / 12. * (12.98 * 12.98 + 10.7 * 10.7) * 5.,
);
/// How far from its middle a blow lands at most: half its width.
pub const ARM: f32 = 5.;
/// LaunchObject spins what it throws by up to this many degrees a second about each axis.
const LAUNCH_SPIN: f32 = 600.;
/// The hold's shadow turns it this fast, in degrees a second.
const HELD_TURN_RATE: f32 = 3600.;

/// UTIL_AngleDiff: the shortest way from `from` to `to`, in degrees.
fn angle_diff(to: f32, from: f32) -> f32 {
    (to - from + 180.).rem_euclid(360.) - 180.
}

/// The body's angles and spin, its banking and its blades.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Attitude {
    /// Pitch (nose down), yaw and roll (right side down), in degrees.
    pub angles: Vec3,
    /// Its spin about its own forward, left and up, in degrees a second.
    pub spin: Vec3,
    /// m_vTargetBanking and m_vCurrentBanking: pitch and roll.
    pub target_banking: (f32, f32),
    pub banking: (f32, f32),
    /// m_fHeadYaw.
    pub head_yaw: f32,
    /// m_flBladeSpeed, and the blade and blur bodygroups last drawn for it.
    pub blade_speed: f32,
    shown: Option<(bool, bool)>,
    started: bool,
}

impl Attitude {
    /// The body starts as it was placed, wanting the yaw it has.
    pub fn start(&mut self, yaw: f32) {
        if !std::mem::replace(&mut self.started, true) {
            self.angles.y = yaw;
            self.head_yaw = yaw;
        }
    }
    pub fn rotation(&self) -> Quat {
        angles(self.angles)
    }
    /// MoveToTarget: it leans into its thrust, by where that points from its own forward and right.
    pub fn bank_toward(&mut self, direction: Vec3) {
        let rotation = self.rotation();
        let (forward, right) = (rotation * Vec3::X, rotation * Vec3::NEG_Y);
        self.target_banking = (BANK * forward.dot(direction), BANK * right.dot(direction));
    }
    /// One think of MoveExecute_Alive and SpinBlades: unless the engine is stalled, the banking
    /// follows its target and the blades gain speed.
    pub fn think(&mut self, interval: f32, stalled: bool) {
        if stalled {
            return;
        }
        let ease = |now: f32, target: f32| BANK_RATE * now + (1. - BANK_RATE) * target;
        self.banking = (
            ease(self.banking.0, self.target_banking.0),
            ease(self.banking.1, self.target_banking.1),
        );
        self.blade_speed = if self.blade_speed < BLADE_SLOW {
            self.blade_speed * 2. + 1.
        } else {
            self.blade_speed + BLADE_GAIN * interval
        }
        .min(BLADE_FULL);
    }
    /// Something hit the blades or the body: they turn no faster than this for now.
    pub fn slow_blades(&mut self, speed: f32) {
        self.blade_speed = self.blade_speed.min(speed);
    }
    /// SpinBlades' bodygroups, when they change: the blade under 40, the blur from 20.
    pub fn blade_groups(&mut self) -> Option<(bool, bool)> {
        let now = (
            self.blade_speed < BLADE_SHOWN_UNDER,
            self.blade_speed >= BLUR_SHOWN_FROM,
        );
        (self.shown.replace(now) != Some(now)).then_some(now)
    }
    /// One physics tick of Simulate, with ClampMotorForces' share of the blades' speed: pitch and
    /// roll take the spin that would close their error in a second, yaw is pushed by its error
    /// past a tenth of a second of spin. The SDK writes these as accelerations over the tick, so a
    /// tick changes the spin by this much whatever its length.
    pub fn step(&mut self, dt: f32) {
        if dt <= 0. {
            return;
        }
        let power = self.blade_speed / BLADE_FULL;
        let roll = angle_diff(self.banking.1, self.angles.z) - self.spin.x;
        let pitch = angle_diff(self.banking.0, self.angles.x) - self.spin.y;
        let yaw = angle_diff(self.head_yaw, self.angles.y + self.spin.z * YAW_LEAD);
        self.spin += Vec3::new(roll, pitch, yaw) * power;
        self.spin *= (-ROT_DAMPING * dt).exp();
        let turned = self.angles + Vec3::new(self.spin.y, self.spin.z, self.spin.x) * dt;
        let wrap = |a: f32| angle_diff(a, 0.);
        self.angles = Vec3::new(wrap(turned.x), wrap(turned.y), wrap(turned.z));
    }
    /// Held, the grab's shadow keeps it level at the yaw it was taken at.
    pub fn hold(&mut self, yaw: f32, dt: f32) {
        let step = HELD_TURN_RATE * dt;
        let toward = |now: f32, to: f32| now + angle_diff(to, now).clamp(-step, step);
        self.angles = Vec3::new(
            toward(self.angles.x, 0.),
            toward(self.angles.y, yaw),
            toward(self.angles.z, 0.),
        );
        self.spin = Vec3::ZERO;
        self.head_yaw = yaw;
    }
    /// A push of `impulse` (kg in/s) landing `offset` from its middle, both in the world's axes:
    /// the spin it gives the body.
    pub fn knock(&mut self, offset: Vec3, impulse: Vec3) {
        let torque = self.rotation().inverse() * offset.cross(impulse);
        self.spin += torque / MOMENTS * 1f32.to_degrees();
    }
}

impl Manhack {
    /// A lever arm for a blow that names no place: up to `ARM` off its middle, across `along`.
    pub fn lever(&mut self, along: Vec3) -> Vec3 {
        let roll = |m: &mut Self| (m.dice() - 0.5) * 2. * ARM;
        let arm = Vec3::new(roll(self), roll(self), roll(self));
        let along = along.normalize_or_zero();
        arm - along * arm.dot(along)
    }
    /// A push on its body somewhere off its middle: the blades slow and the body spins.
    pub fn jolt(&mut self, impulse: Vec3, blades: f32) {
        let offset = self.lever(impulse);
        self.attitude.knock(offset, impulse);
        self.attitude.slow_blades(blades);
    }
    /// LaunchObject's random spin on what it throws.
    pub fn launch_spin(&mut self) {
        let roll = |m: &mut Self| (m.dice() - 0.5) * 2. * LAUNCH_SPIN;
        let spin = Vec3::new(roll(self), roll(self), roll(self));
        self.attitude.spin += spin;
    }
    /// This think's banking: a burst keeps the lean it had, a flight at a target leans into it,
    /// and with nothing to fly at it levels out.
    pub fn bank(&mut self, origin: Vec3, now: f64) {
        if self.bursting(now) {
            return;
        }
        match self.target {
            Some(target) => self
                .attitude
                .bank_toward((target - origin).normalize_or_zero()),
            None => self.attitude.target_banking = (0., 0.),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TICK: f32 = 0.015;

    fn flying() -> Attitude {
        Attitude {
            blade_speed: BLADE_FULL,
            ..Default::default()
        }
    }

    fn run(a: &mut Attitude, seconds: f32) {
        for _ in 0..(seconds / TICK).round() as usize {
            a.step(TICK);
        }
    }

    #[test]
    fn it_leans_forty_degrees_into_a_thrust_ahead_and_takes_about_a_second_to_get_there() {
        let mut a = flying();
        a.bank_toward(Vec3::X);
        assert_eq!(a.target_banking, (BANK, 0.));
        for _ in 0..10 {
            a.think(0.1, false);
            run(&mut a, 0.1);
        }
        // The banking is there in a few thinks; the body closes its error at about 1/e a second.
        assert!((a.banking.0 - BANK).abs() < 0.1);
        assert!(a.angles.x > 15. && a.angles.x < 30., "{}", a.angles.x);
        assert_eq!(a.angles.z, 0.);
        // A thrust to its right rolls it right side down.
        let mut b = flying();
        b.bank_toward(Vec3::NEG_Y);
        assert_eq!(b.target_banking, (0., BANK));
    }

    #[test]
    fn its_yaw_swings_past_a_new_heading_and_settles_on_it() {
        let mut a = flying();
        a.head_yaw = 90.;
        let mut most = 0f32;
        for _ in 0..80 {
            a.step(TICK);
            most = most.max(a.angles.y);
        }
        assert!(most > 95. && most < 130., "overshoot {most}");
        assert!((a.angles.y - 90.).abs() < 3., "{}", a.angles.y);
    }

    #[test]
    fn a_knock_off_its_middle_spins_it_longer_with_slowed_blades() {
        // 300 units a second of its mass, three units to its left: about 420 degrees a second.
        let impulse = Vec3::X * 300. * PHYSICS_MASS;
        let mut fast = flying();
        fast.knock(Vec3::Y * 3., impulse);
        assert!((fast.spin.z.abs() - 437.).abs() < 5., "{}", fast.spin.z);
        assert!(
            fast.spin.z < 0.,
            "a push ahead on its left side turns it right"
        );
        let mut slow = fast;
        slow.slow_blades(BLADE_HIT);
        run(&mut fast, 0.3);
        run(&mut slow, 0.3);
        let turned = |a: &Attitude| a.angles.y.abs();
        assert!(
            turned(&slow) > 2. * turned(&fast),
            "{} {}",
            turned(&slow),
            turned(&fast)
        );
        // A knock in line with its middle spins nothing.
        let mut level = flying();
        level.knock(Vec3::X * 3., impulse);
        assert_eq!(level.spin, Vec3::ZERO);
    }

    #[test]
    fn stalled_blades_do_not_gain_speed_and_slow_ones_show_the_blade() {
        let mut a = Attitude::default();
        assert_eq!(a.blade_groups(), Some((true, false)));
        a.think(0.1, true);
        assert_eq!(a.blade_speed, 0.);
        // 1, 3, 7, 15, then eight a think.
        for _ in 0..4 {
            a.think(0.1, false);
        }
        assert_eq!(a.blade_speed, 15.);
        assert_eq!(a.blade_groups(), None);
        a.think(0.1, false);
        assert_eq!(a.blade_groups(), Some((true, true)));
        for _ in 0..20 {
            a.think(0.1, false);
        }
        assert_eq!(a.blade_speed, BLADE_FULL);
        assert_eq!(a.blade_groups(), Some((false, true)));
        a.slow_blades(BLADE_HIT);
        assert_eq!(a.blade_groups(), Some((true, true)));
    }

    #[test]
    fn held_it_is_level_at_the_holds_yaw_and_starts_where_it_was_placed() {
        let mut a = flying();
        a.start(170.);
        a.start(20.);
        assert_eq!((a.angles.y, a.head_yaw), (170., 170.));
        a.angles = Vec3::new(30., 170., -20.);
        a.spin = Vec3::splat(200.);
        for _ in 0..10 {
            a.hold(-170., TICK);
        }
        assert!(a.angles.x.abs() < 1e-3 && a.angles.z.abs() < 1e-3);
        assert!(angle_diff(a.angles.y, -170.).abs() < 1e-3);
        assert_eq!(a.spin, Vec3::ZERO);
    }
}
