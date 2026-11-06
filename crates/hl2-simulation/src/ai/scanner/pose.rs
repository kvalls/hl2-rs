//! The parts of a scanner that move on their own (DESIGN 14o session 47): its faceplate, tail,
//! dynamo wheel and alert flaps. In the owned combine_scanner.mdl each is an autoplay delta
//! sequence blended by one pose parameter, so setting the parameter is all it takes. The rules are
//! the SDK's UpdateHead, MoveExecute_Alive and OnStateChange, run once per think; the client
//! smooths networked pose parameters, which `between` stands for.

use crate::ai::host::angle_diff;
use crate::attention::approach;
use crate::view_control::angles_toward;
use glam::Vec3;
use modkit_core::animation::{simple_spline, Rig};
use serde::Serialize;

/// flex_horz and flex_vert run from minus this to this in the model.
const FACE_LIMIT: f32 = 20.;
/// How far the face turns in a think toward what it watches, and back with nothing to watch.
const FACE_STEP: f32 = 50.;
const FACE_RELAX: f32 = 10.;
/// The climb and fall that take the tail to its ends, and those ends.
const TAIL_SPEEDS: (f32, f32) = (-150., 250.);
const TAIL_ANGLES: (f32, f32) = (-25., 80.);
/// How far the dynamo turns in a think at full speed; it runs backward through its range.
const DYNAMO_STEP: f32 = 60.;

/// The pose parameters of a scanner, in the model's own units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Pose {
    pub tail: f32,
    pub dynamo: f32,
    pub alert: f32,
    pub face_yaw: f32,
    pub face_pitch: f32,
}
impl Pose {
    /// The tail follows the climb and the dynamo the speed. `top_speed` is the limit it flies at;
    /// with none, in a dive, the dynamo runs flat out.
    pub fn fly(&mut self, velocity: Vec3, top_speed: Option<f32>) {
        let (slow, fast) = TAIL_SPEEDS;
        let (low, high) = TAIL_ANGLES;
        let share = (velocity.z.clamp(slow, fast) - slow) / (fast - slow);
        self.tail = low + (high - low) * simple_spline(share);
        let spin = top_speed.map_or(1., |top| velocity.length() / top);
        self.dynamo -= spin * DYNAMO_STEP;
        if self.dynamo < -180. {
            self.dynamo += 360.;
        }
    }
    /// The faceplate turns toward `look`, the way from its eyes to what it watches, unless that is
    /// behind it; `body` is the way the body faces.
    pub fn watch(&mut self, body: Vec3, look: Option<Vec3>) {
        let Some(look) = look.filter(|look| look.dot(body) >= 0.) else {
            self.face_yaw = approach(0., self.face_yaw, FACE_RELAX);
            self.face_pitch = approach(0., self.face_pitch, FACE_RELAX);
            return;
        };
        let (body_pitch, body_yaw) = angles_toward(body);
        let (look_pitch, look_yaw) = angles_toward(look);
        let turn = |current: f32, wanted: f32, facing: f32| {
            let target = current + angle_diff(wanted, facing + current);
            approach(target, current, FACE_STEP).clamp(-FACE_LIMIT, FACE_LIMIT)
        };
        self.face_yaw = turn(self.face_yaw, look_yaw, body_yaw);
        self.face_pitch = turn(self.face_pitch, look_pitch, body_pitch);
    }
    /// `share` of the way to `next`; the dynamo goes the short way round.
    pub fn between(&self, next: &Self, share: f32) -> Self {
        let mix = |a: f32, b: f32| a + (b - a) * share;
        Self {
            tail: mix(self.tail, next.tail),
            dynamo: self.dynamo + angle_diff(next.dynamo, self.dynamo) * share,
            alert: mix(self.alert, next.alert),
            face_yaw: mix(self.face_yaw, next.face_yaw),
            face_pitch: mix(self.face_pitch, next.face_pitch),
        }
    }
    /// Its values in a fixed order, for what notices that a pose changed.
    pub fn values(&self) -> [f32; 5] {
        [
            self.tail,
            self.dynamo,
            self.alert,
            self.face_yaw,
            self.face_pitch,
        ]
    }
    /// Writes its values over a model's normalized pose parameters.
    pub fn apply(&self, rig: &Rig, params: &mut [f32]) {
        let names = [
            "tail_control",
            "dynamo_wheel",
            "alert_control",
            "flex_horz",
            "flex_vert",
        ];
        for (name, value) in names.into_iter().zip(self.values()) {
            if let Some(i) = rig.pose_parameter(name) {
                params[i] = rig.pose_parameters[i].normalize(value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tail_follows_the_climb_between_its_ends() {
        let tail = |z: f32| {
            let mut pose = Pose::default();
            pose.fly(Vec3::Z * z, Some(250.));
            pose.tail
        };
        assert_eq!(tail(-150.), -25.);
        assert_eq!(tail(250.), 80.);
        assert_eq!(tail(-900.), -25.);
        assert_eq!(tail(900.), 80.);
        // Halfway between the speeds the spline is halfway too.
        assert!((tail(50.) - 27.5).abs() < 1e-3);
    }

    #[test]
    fn the_dynamo_spins_with_speed_and_wraps() {
        let mut pose = Pose::default();
        pose.fly(Vec3::ZERO, Some(250.));
        assert_eq!(pose.dynamo, 0.);
        pose.fly(Vec3::X * 125., Some(250.));
        assert_eq!(pose.dynamo, -30.);
        pose.dynamo = -170.;
        pose.fly(Vec3::X * 250., Some(250.));
        assert_eq!(pose.dynamo, 130.);
    }

    #[test]
    fn a_dive_runs_the_dynamo_flat_out_whatever_its_speed() {
        let mut pose = Pose::default();
        pose.fly(Vec3::ZERO, None);
        assert_eq!(pose.dynamo, -60.);
    }

    #[test]
    fn the_face_turns_to_what_it_watches_up_to_its_limit() {
        let mut pose = Pose::default();
        pose.watch(Vec3::X, Some(Vec3::new(1., 0.2, 0.)));
        assert!((pose.face_yaw - 11.31).abs() < 0.01, "{}", pose.face_yaw);
        assert_eq!(pose.face_pitch, 0.);
        pose.watch(Vec3::X, Some(Vec3::new(1., 1., 0.)));
        assert_eq!(pose.face_yaw, FACE_LIMIT);
    }

    #[test]
    fn looking_down_is_a_positive_pitch_as_a_view_angle_is() {
        let mut pose = Pose::default();
        pose.watch(Vec3::X, Some(Vec3::new(1., 0., -0.2)));
        assert!(
            (pose.face_pitch - 11.31).abs() < 0.01,
            "{}",
            pose.face_pitch
        );
    }

    #[test]
    fn with_nothing_to_watch_or_behind_it_the_face_eases_back() {
        let turned = Pose {
            face_yaw: 15.,
            face_pitch: -4.,
            ..Default::default()
        };
        let mut pose = turned;
        pose.watch(Vec3::X, None);
        assert_eq!((pose.face_yaw, pose.face_pitch), (5., 0.));
        let mut pose = turned;
        pose.watch(Vec3::X, Some(Vec3::NEG_X));
        assert_eq!((pose.face_yaw, pose.face_pitch), (5., 0.));
    }

    #[test]
    fn between_two_thinks_the_dynamo_takes_the_short_way() {
        let from = Pose {
            dynamo: -170.,
            tail: 0.,
            ..Default::default()
        };
        let to = Pose {
            dynamo: 170.,
            tail: 10.,
            ..Default::default()
        };
        let half = from.between(&to, 0.5);
        assert_eq!(half.tail, 5.);
        assert!((half.dynamo + 180.).abs() < 1e-3, "{}", half.dynamo);
    }
}
