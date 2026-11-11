//! Where the viewmodel sits each frame relative to the eye (Source SDK
//! baseviewmodel_shared.cpp CalcViewModelView and CalcViewModelLag; hl2
//! basehlcombatweapon_shared.cpp CalcViewmodelBob and AddViewmodelBob): HL2's bob from the
//! player's ground speed, the drift that trails a turn, and the pitch offset that moves the gun
//! forward, right and up as the view looks up. Source axes: x forward, y left, z up; angles are
//! (pitch down, yaw left, roll) in degrees.
use glam::{Quat, Vec3};

/// HL2_BOB_CYCLE_MAX (seconds of bob time a cycle) and HL2_BOB_UP (the rising share of it).
const BOB_CYCLE: f32 = 0.45;
const BOB_UP: f32 = 0.5;
/// The ground speed of a full bob (the speed clamp and RemapVal's top).
const BOB_SPEED: f32 = 320.;
/// g_fMaxViewModelLag and CalcViewModelLag's catch-up speed.
const MAX_LAG: f32 = 1.5;
const LAG_SPEED: f32 = 5.;

/// One frame's inputs: the eye's angles, the player's horizontal speed, the client clock and
/// the frame's length (0 while paused).
#[derive(Clone, Copy, Debug)]
pub struct SwayInput {
    pub angles: Vec3,
    pub speed: f32,
    pub time: f64,
    pub frame_time: f32,
}

/// The viewmodel's pose relative to the eye: an offset in eye axes and a rotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sway {
    pub offset: Vec3,
    pub rotation: Quat,
}

/// CalcViewmodelBob's statics (bobtime, lastbobtime) and the viewmodel's m_vecLastFacing.
#[derive(Clone, Copy, Debug, Default)]
pub struct ViewmodelSway {
    bob_time: f32,
    last_bob_time: f64,
    last_facing: Option<Vec3>,
}

/// AngleMatrix's rotation for (pitch, yaw, roll) in Source axes.
pub fn rotation(angles: Vec3) -> Quat {
    let [pitch, yaw, roll] = angles.to_array().map(f32::to_radians);
    Quat::from_rotation_z(yaw) * Quat::from_rotation_y(pitch) * Quat::from_rotation_x(roll)
}

/// One bob wave: the cycle's position as an angle, rising over BOB_UP of it.
fn bob_wave(bob_time: f32, cycle: f32, speed: f32) -> f32 {
    let mut phase = (bob_time - (bob_time / cycle).trunc() * cycle) / cycle;
    phase = if phase < BOB_UP {
        std::f32::consts::PI * phase / BOB_UP
    } else {
        std::f32::consts::PI + std::f32::consts::PI * (phase - BOB_UP) / (1. - BOB_UP)
    };
    let bob = speed * 0.005;
    (bob * 0.3 + bob * 0.7 * phase.sin()).clamp(-7., 4.)
}

impl ViewmodelSway {
    /// CalcViewmodelBob: (vertical, lateral). Nothing moves on a frame of length 0.
    fn bob(&mut self, input: SwayInput) -> (f32, f32) {
        let speed = input.speed.clamp(-BOB_SPEED, BOB_SPEED);
        if input.frame_time > 0. {
            self.bob_time += (input.time - self.last_bob_time) as f32 * (speed / BOB_SPEED);
            self.last_bob_time = input.time;
        }
        (
            bob_wave(self.bob_time, BOB_CYCLE, speed),
            bob_wave(self.bob_time, BOB_CYCLE * 2., speed),
        )
    }

    /// CalcViewModelView without the shake: AddViewmodelBob, then CalcViewModelLag.
    pub fn place(&mut self, input: SwayInput) -> Sway {
        let eye = rotation(input.angles);
        let (vertical, lateral) = self.bob(input);
        // AddViewmodelBob: along forward and world up, then the angles, then along right.
        let mut origin = eye * Vec3::X * (vertical * 0.1) + Vec3::Z * (vertical * 0.1);
        let bobbed = input.angles + Vec3::new(-vertical * 0.4, -lateral * 0.3, vertical * 0.5);
        origin += eye * Vec3::NEG_Y * (lateral * 0.8);
        // CalcViewModelLag: the facing trails the bobbed forward; the gun lags behind it.
        let forward = rotation(bobbed) * Vec3::X;
        let last = self.last_facing.get_or_insert(forward);
        if input.frame_time > 0. {
            let difference = forward - *last;
            let mut speed = LAG_SPEED;
            let distance = difference.length();
            if distance > MAX_LAG {
                speed *= distance / MAX_LAG;
            }
            // Ours caps the step at the whole difference: a long frame (a hitch) would
            // overshoot the facing and set it swinging.
            let step = (speed * input.frame_time).min(1.);
            let caught = (*last + difference * step).normalize_or_zero();
            *last = if caught == Vec3::ZERO {
                forward
            } else {
                caught
            };
            origin -= difference * 5.;
        }
        let pitch = (input.angles.x + 180.).rem_euclid(360.) - 180.;
        origin += eye * Vec3::new(-pitch * 0.035, pitch * 0.03, -pitch * 0.02);
        Sway {
            offset: eye.inverse() * origin,
            rotation: eye.inverse() * rotation(bobbed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(angles: Vec3, speed: f32, time: f64) -> SwayInput {
        SwayInput {
            angles,
            speed,
            time,
            frame_time: 0.015,
        }
    }

    #[test]
    fn standing_level_it_stays_in_place() {
        let mut sway = ViewmodelSway::default();
        let placed = sway.place(input(Vec3::ZERO, 0., 1.));
        assert!(placed.offset.length() < 1e-5, "{:?}", placed.offset);
        assert!(placed.rotation.angle_between(Quat::IDENTITY) < 1e-5);
    }

    #[test]
    fn looking_up_moves_the_gun_forward_right_and_up() {
        let mut sway = ViewmodelSway::default();
        // 50 degrees up: forward 1.75, right 1.5, up 1.0 in eye axes.
        let placed = sway.place(input(Vec3::new(-50., 30., 0.), 0., 1.));
        let want = Vec3::new(1.75, -1.5, 1.0);
        assert!(
            (placed.offset - want).length() < 1e-4,
            "{:?}",
            placed.offset
        );
    }

    #[test]
    fn a_turn_leaves_the_gun_behind_and_it_catches_up() {
        let mut sway = ViewmodelSway::default();
        sway.place(input(Vec3::ZERO, 0., 1.));
        // Turning 10 degrees left at once: the gun trails to the right of the new facing.
        let first = sway.place(input(Vec3::new(0., 10., 0.), 0., 1.015));
        assert!(first.offset.y < -0.5, "{:?}", first.offset);
        let mut last = first;
        for frame in 2..200 {
            last = sway.place(input(Vec3::new(0., 10., 0.), 0., 1. + frame as f64 * 0.015));
        }
        assert!(last.offset.length() < 0.01, "{:?}", last.offset);
    }

    #[test]
    fn a_long_frame_catches_up_without_overshooting() {
        let mut sway = ViewmodelSway::default();
        sway.place(input(Vec3::ZERO, 0., 1.));
        let mut hitch = input(Vec3::new(0., 90., 0.), 0., 3.);
        hitch.frame_time = 2.;
        sway.place(hitch);
        let after = sway.place(input(Vec3::new(0., 90., 0.), 0., 3.015));
        assert!(after.offset.length() < 1e-4, "{:?}", after.offset);
    }

    #[test]
    fn running_bobs_within_the_sdk_clamps_and_standing_still_does_not() {
        let mut sway = ViewmodelSway::default();
        let mut highest = f32::MIN;
        let mut lowest = f32::MAX;
        for frame in 0..200 {
            let placed = sway.place(input(Vec3::ZERO, 320., frame as f64 * 0.015));
            highest = highest.max(placed.offset.z);
            lowest = lowest.min(placed.offset.z);
        }
        // Vertical bob 1.6 x (0.3 + 0.7 sin): z = 0.1 x bob, within -0.064 and 0.16; the lag
        // trails the bobbing pitch and takes some of it back (-0.03 to 0.13 measured).
        assert!(
            highest > 0.1 && highest < 0.161 && lowest < -0.01 && lowest > -0.065,
            "{lowest} {highest}"
        );
        let mut still = ViewmodelSway::default();
        let a = still.place(input(Vec3::ZERO, 0., 1.));
        let b = still.place(input(Vec3::ZERO, 0., 2.));
        assert_eq!(a, b);
    }
}
