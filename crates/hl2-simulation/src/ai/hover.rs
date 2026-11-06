//! Hovering flight (DESIGN 14o session 46, npc_cscanner step 1): speed fades away each think while
//! thrust pushes along the wanted direction, so a hovering NPC eases into a steady cruise and
//! drifts to a stop. Scanners and manhacks fly this way, unlike the birds' fixed airspeed. The
//! arithmetic only: the host carries the body and the class picks the thrust.
use glam::Vec3;
use serde::Serialize;

/// How hard a hovering NPC pushes: u/s² across and up, and the share of its speed left after a
/// second with no thrust.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thrust {
    pub across: f32,
    pub up: f32,
    pub decay: f32,
}

/// A scanner's thrust: stronger across with the target ahead, weaker when turning back to it.
const SCANNER_AHEAD: f32 = 250.;
const SCANNER_TURNING: f32 = 128.;
const SCANNER_UP: f32 = 400.;
const SCANNER_DECAY: f32 = 0.15;
/// The target counts as ahead past this cosine.
const AHEAD: f32 = 0.25;
/// With nothing to fly at, the share of its speed kept is this times the think's length.
const COAST: f32 = 9.5;
/// Running into something pushes it back along the surface's normal by this share of its speed.
const BOUNCE: f32 = 0.5;

/// A dead scanner's fall at its enemy.
const DIVE_THRUST: Thrust = Thrust {
    across: 1600.,
    up: 1600.,
    decay: 0.05,
};

/// A hovering NPC's motion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Hover {
    pub velocity: Vec3,
}
impl Hover {
    /// One think's fade and push along `direction` (a unit vector, or zero to coast).
    pub fn push(&mut self, direction: Vec3, thrust: Thrust, interval: f32) {
        let kept = thrust.decay.powf(interval);
        let across = thrust.across * interval;
        let up = thrust.up * interval;
        self.velocity = Vec3::new(
            kept * self.velocity.x + across * direction.x,
            kept * self.velocity.y + across * direction.y,
            kept * self.velocity.z + up * direction.z,
        );
    }
    /// A scanner's push toward `to_target`, never more in one think than the distance left.
    pub fn scanner_thrust(&self, to_target: Vec3, interval: f32) -> Thrust {
        let distance = to_target.length();
        let heading = self.velocity.normalize_or_zero();
        let ahead = to_target.normalize_or_zero().dot(heading) > AHEAD;
        let across = if ahead {
            SCANNER_AHEAD
        } else {
            SCANNER_TURNING
        };
        let reach = distance / interval;
        Thrust {
            across: across.min(reach),
            up: SCANNER_UP.min(reach),
            decay: SCANNER_DECAY,
        }
    }
    /// One think with nothing to fly at.
    pub fn coast(&mut self, interval: f32) {
        self.velocity *= (COAST * interval).min(1.);
    }
    /// Time-coherent noise, with `rates` a little different for every NPC.
    pub fn add_noise(&mut self, rates: Vec3, scale: f32, now: f32) {
        let wave = |rate: f32, curve: fn(f32) -> f32| {
            if rate == 0. {
                0.
            } else {
                curve(rate * now + rate)
            }
        };
        self.velocity += scale
            * Vec3::new(
                wave(rates.x, f32::sin),
                wave(rates.y, f32::cos),
                -wave(rates.z, f32::cos),
            );
    }
    /// One think of a dive along `direction`.
    pub fn fly_dive(&mut self, direction: Vec3, interval: f32) {
        self.push(direction, DIVE_THRUST, interval);
    }
    /// LimitSpeed.
    pub fn limit(&mut self, speed: f32) {
        self.velocity = self.velocity.clamp_length_max(speed);
    }
    /// Off a surface it is about to hit.
    pub fn bounce(&mut self, normal: Vec3) {
        self.velocity += normal * BOUNCE * self.velocity.length();
    }
    /// `scanner_thrust` applied: one think of flying at `to_target`.
    pub fn fly_scanner(&mut self, to_target: Vec3, interval: f32) {
        let thrust = self.scanner_thrust(to_target, interval);
        self.push(to_target.normalize_or_zero(), thrust, interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THINK: f32 = 0.1;

    #[test]
    fn with_no_thrust_speed_fades_to_the_decay_share_in_a_second() {
        let mut hover = Hover {
            velocity: Vec3::X * 100.,
        };
        let coast = Thrust {
            across: 0.,
            up: 0.,
            decay: 0.15,
        };
        for _ in 0..10 {
            hover.push(Vec3::ZERO, coast, THINK);
        }
        assert!(
            (hover.velocity.x - 15.).abs() < 0.01,
            "{}",
            hover.velocity.x
        );
    }

    #[test]
    fn a_steady_push_settles_at_a_cruising_speed() {
        let mut hover = Hover::default();
        let far = Vec3::X * 1e6;
        for _ in 0..200 {
            hover.fly_scanner(far, THINK);
        }
        let cruise = hover.velocity.x;
        // Each think keeps 0.15^0.1 of the speed and adds 25: 25 / (1 - 0.827).
        assert!((cruise - 144.6).abs() < 1., "{cruise}");
        hover.fly_scanner(far, THINK);
        assert!((hover.velocity.x - cruise).abs() < 0.01, "settled");
    }

    #[test]
    fn it_pushes_harder_when_the_target_is_ahead() {
        let moving = Hover {
            velocity: Vec3::X * 50.,
        };
        assert_eq!(moving.scanner_thrust(Vec3::X * 500., THINK).across, 250.);
        assert_eq!(moving.scanner_thrust(-Vec3::X * 500., THINK).across, 128.);
        assert_eq!(moving.scanner_thrust(Vec3::Y * 500., THINK).across, 128.);
    }

    #[test]
    fn a_still_hover_counts_as_turning() {
        let still = Hover::default();
        assert_eq!(still.scanner_thrust(Vec3::X * 500., THINK).across, 128.);
    }

    #[test]
    fn thrust_never_exceeds_what_the_distance_left_needs() {
        let hover = Hover {
            velocity: Vec3::X * 50.,
        };
        let close = hover.scanner_thrust(Vec3::X * 5., THINK);
        assert_eq!(close.across, 50.);
        assert_eq!(close.up, 50.);
    }

    #[test]
    fn climbing_is_stronger_than_crossing() {
        let mut climb = Hover::default();
        climb.fly_scanner(Vec3::Z * 1000., THINK);
        let mut cross = Hover::default();
        cross.fly_scanner(Vec3::X * 1000., THINK);
        assert!(climb.velocity.z > cross.velocity.x);
    }

    #[test]
    fn with_nothing_to_fly_at_it_slows_by_the_think() {
        let mut hover = Hover {
            velocity: Vec3::X * 100.,
        };
        hover.coast(THINK);
        assert!((hover.velocity.x - 95.).abs() < 0.001);
    }

    #[test]
    fn a_long_think_never_speeds_a_coast_up() {
        let mut hover = Hover {
            velocity: Vec3::X * 100.,
        };
        hover.coast(1.);
        assert_eq!(hover.velocity.x, 100.);
    }

    #[test]
    fn noise_stays_within_its_scale_on_each_axis() {
        let rates = Vec3::new(1.5, -1.5, 3.);
        for tick in 0..100 {
            let mut hover = Hover::default();
            hover.add_noise(rates, 3., tick as f32 * 0.1);
            assert!(hover.velocity.abs().max_element() <= 3.);
        }
    }

    #[test]
    fn a_rate_of_zero_adds_no_noise_on_that_axis() {
        let mut hover = Hover::default();
        hover.add_noise(Vec3::ZERO, 12., 5.);
        assert_eq!(hover.velocity, Vec3::ZERO);
    }

    #[test]
    fn speed_is_capped_along_its_direction() {
        let mut hover = Hover {
            velocity: Vec3::new(300., 400., 0.),
        };
        hover.limit(250.);
        assert!((hover.velocity - Vec3::new(150., 200., 0.)).length() < 0.01);
    }

    #[test]
    fn a_surface_pushes_it_back_by_half_its_speed() {
        let mut hover = Hover {
            velocity: Vec3::X * 100.,
        };
        hover.bounce(-Vec3::X);
        assert_eq!(hover.velocity.x, 50.);
    }

    #[test]
    fn a_dive_gathers_speed_far_past_a_cruise() {
        let mut hover = Hover::default();
        for _ in 0..30 {
            hover.fly_dive(-Vec3::Z, THINK);
        }
        assert!(hover.velocity.z < -400., "{}", hover.velocity.z);
    }

    #[test]
    fn at_the_target_it_only_coasts() {
        let mut hover = Hover {
            velocity: Vec3::X * 100.,
        };
        hover.fly_scanner(Vec3::ZERO, THINK);
        assert!(hover.velocity.x < 100. && hover.velocity.x > 0.);
        assert_eq!(hover.velocity.y, 0.);
    }
}
