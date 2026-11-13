//! func_rotating (SDK bmodels.cpp CFuncRotating): turns about its yaw axis (or roll with
//! Z_AXIS, pitch with X_AXIS) at a target speed; with SF_BRUSH_ACCDCC it spins up and down by
//! its fan friction in 0.1 s steps, else a new speed applies at once. StopAtStartPos, the
//! running sound and HurtTouch are not modeled.
use glam::Vec3;

const START_ON: u32 = 1;
const BACKWARDS: u32 = 2;
const Z_AXIS: u32 = 4;
const X_AXIS: u32 = 8;
const ACCDCC: u32 = 16;
/// GetNextMoveInterval without StopAtStartPos.
const STEP: f64 = 0.1;

#[derive(Clone, Debug, PartialEq)]
pub struct Rotating {
    /// Degrees per second along `axis` (negative turns backward).
    pub speed: f32,
    target: f32,
    max: f32,
    friction: f32,
    accelerates: bool,
    reversed: bool,
    /// m_vecMoveAng: (pitch, yaw, roll) unit axis, negated by BACKWARDS.
    axis: Vec3,
    /// The local angles (pitch, yaw, roll).
    pub angles: Vec3,
    /// The next spin step, or SUB_CallUseToggle for START_ON.
    next: Option<f64>,
    starting: bool,
}

impl Rotating {
    /// CFuncRotating::Spawn: `maxspeed` (0 is 100), `fanfriction` / 100 (0 is 1), the axis
    /// flags; START_ON uses the entity 0.2 s after spawn.
    pub fn spawn(flags: u32, maxspeed: f32, fanfriction: f32, angles: Vec3, time: f64) -> Self {
        let mut axis = if flags & Z_AXIS != 0 {
            Vec3::Z
        } else if flags & X_AXIS != 0 {
            Vec3::X
        } else {
            Vec3::Y
        };
        if flags & BACKWARDS != 0 {
            axis = -axis;
        }
        let max = if maxspeed == 0. { 100. } else { maxspeed.abs() };
        let friction = if fanfriction == 0. {
            1.
        } else {
            fanfriction / 100.
        };
        let starting = flags & START_ON != 0;
        Self {
            speed: 0.,
            target: 0.,
            max,
            friction,
            accelerates: flags & ACCDCC != 0,
            reversed: false,
            axis,
            angles,
            next: starting.then_some(time + 0.2),
            starting,
        }
    }

    /// SetTargetSpeed: the speed at once without ACCDCC, else spin steps toward it.
    fn set_target(&mut self, speed: f32, time: f64) {
        let speed = speed.abs();
        self.target = if self.reversed { -speed } else { speed };
        if self.accelerates {
            self.next = Some(time + STEP);
        } else {
            self.speed = self.target.clamp(-self.max, self.max);
        }
    }

    /// RotatingUse (Toggle without the sign test): stop when turning, else full speed.
    fn use_toggle(&mut self, time: f64) {
        let speed = if self.speed != 0. { 0. } else { self.max };
        self.set_target(speed, time);
    }

    /// The inputs; false for an unknown one.
    pub fn input(&mut self, input: &str, value: f32, time: f64) -> bool {
        match input {
            "start" => self.set_target(self.max, time),
            "stop" => self.set_target(0., time),
            "toggle" => {
                let speed = if self.speed > 0. { 0. } else { self.max };
                self.set_target(speed, time);
            }
            "startforward" | "startbackward" => {
                self.reversed = input == "startbackward";
                self.set_target(self.max, time);
            }
            "reverse" => {
                self.reversed = !self.reversed;
                self.set_target(self.speed, time);
            }
            "setspeed" => {
                self.reversed = value < 0.;
                self.set_target(value.abs().clamp(0., 1.) * self.max, time);
            }
            _ => return false,
        }
        true
    }

    /// SpinUpMove / SpinDownMove / ReverseMove: one 0.1 s step toward the target.
    fn spin_step(&mut self) -> bool {
        let reversing = self.speed * self.target < 0.;
        let up = !reversing && self.speed.abs() < self.target.abs();
        let magnitude = if up {
            (self.speed.abs() + 0.2 * self.max * self.friction).min(self.target.abs())
        } else {
            let floor = if reversing { 0. } else { self.target.abs() };
            (self.speed.abs() - 0.1 * self.max * self.friction).max(floor)
        };
        let sign = if up {
            self.target.signum()
        } else {
            self.speed.signum()
        };
        self.speed = (magnitude * sign).clamp(-self.max, self.max);
        self.speed == self.target
    }

    /// One tick: a due step or start, then the turn by the angular velocity.
    pub fn advance(&mut self, time: f64, dt: f32) {
        if let Some(at) = self.next.filter(|&at| at <= time) {
            self.next = None;
            if std::mem::take(&mut self.starting) {
                self.use_toggle(time);
            } else if !self.spin_step() {
                self.next = Some(at + STEP);
            }
        }
        self.angles += self.axis * self.speed * dt;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hl1_room_rotator_starts_after_0_2_s_at_full_speed() {
        // d1_trainstation_01's hl1room_rotator: spawnflags 1, maxspeed 10, fanfriction 20.
        let mut r = Rotating::spawn(1, 10., 20., Vec3::ZERO, 0.);
        r.advance(0.195, 0.015);
        assert_eq!(r.speed, 0.);
        r.advance(0.21, 0.015);
        assert_eq!(r.speed, 10.);
        assert!((r.angles.y - 0.15).abs() < 1e-5);
        assert!(r.input("stop", 0., 1.));
        assert_eq!(r.speed, 0.);
        assert!(r.input("setspeed", -0.5, 1.));
        assert_eq!(r.speed, -5.);
    }

    #[test]
    fn accdcc_spins_up_and_down_by_the_fan_friction() {
        // ACCDCC | Z_AXIS, maxspeed 100, fanfriction 50: +10 per step up, -5 down.
        let mut r = Rotating::spawn(16 | 4, 100., 50., Vec3::ZERO, 0.);
        r.input("start", 0., 0.);
        let mut speeds = Vec::new();
        for tick in 1..=120 {
            r.advance(f64::from(tick) * 0.015, 0.015);
            speeds.push(r.speed);
        }
        assert_eq!(speeds[5], 0.);
        assert_eq!(speeds[6], 10.);
        assert_eq!(r.speed, 100.);
        assert!(r.angles.z > 0. && r.angles.y == 0.);
        r.input("stop", 0., 1.8);
        r.advance(1.905, 0.015);
        assert_eq!(r.speed, 95.);
    }
}
