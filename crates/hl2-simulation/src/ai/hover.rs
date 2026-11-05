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
    fn at_the_target_it_only_coasts() {
        let mut hover = Hover {
            velocity: Vec3::X * 100.,
        };
        hover.fly_scanner(Vec3::ZERO, THINK);
        assert!(hover.velocity.x < 100. && hover.velocity.x > 0.);
        assert_eq!(hover.velocity.y, 0.);
    }
}
