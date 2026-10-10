//! Timed explicit NPC interests refreshed by authored scene LOOKAT events.
//! Random/synthetic queues, head poses, PVS gating and native AI scheduling are unfinished.
use glam::Vec3;
use serde::Serialize;
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Target {
    Player,
    Entity(usize),
}
#[derive(Clone, Debug, Serialize)]
pub struct Interest {
    pub target: Target,
    pub importance: f32,
    pub start: f64,
    pub end: f64,
    pub scene: usize,
    pub event: usize,
}
#[derive(Default)]
pub struct LookTargets {
    actors: BTreeMap<usize, Vec<Interest>>,
}
impl LookTargets {
    pub fn refresh(
        &mut self,
        actor: usize,
        target: Target,
        mut importance: f32,
        now: f64,
        scene: usize,
        event: usize,
    ) {
        let queue = self.actors.entry(actor).or_default();
        if let Some(index) = queue.iter().position(|i| i.target == target) {
            let previous = queue.remove(index);
            if previous.start == now {
                importance = importance.max(previous.importance);
            }
        }
        queue.push(Interest {
            target,
            importance,
            start: now,
            end: now + 0.1,
            scene,
            event,
        });
    }
    /// CAI_BaseActor::AddLookTarget(target, importance, duration) from the AI.
    pub fn add(&mut self, actor: usize, target: Target, importance: f32, now: f64, duration: f64) {
        let queue = self.actors.entry(actor).or_default();
        queue.retain(|i| i.target != target);
        queue.push(Interest {
            target,
            importance,
            start: now,
            end: now + duration,
            scene: crate::npc::AI_SCENE,
            event: 0,
        });
    }
    pub fn cleanup(&mut self, now: f64, alive: impl Fn(usize) -> bool) {
        self.actors.retain(|actor, queue| {
            queue.retain(|i| {
                i.end >= now
                    && match i.target {
                        Target::Player => true,
                        Target::Entity(id) => alive(id),
                    }
            });
            alive(*actor) && !queue.is_empty()
        });
    }
    pub fn report(&self) -> &BTreeMap<usize, Vec<Interest>> {
        &self.actors
    }
    /// Eyes choose newest valid explicit interest, independent of head importance.
    /// Explicit scene targets are not subject to random-interest visibility/distance filters.
    pub fn eye_target(
        &self,
        actor: usize,
        eye: Vec3,
        forward: Vec3,
        position: impl Fn(Target) -> Option<Vec3>,
    ) -> Option<(&Interest, Vec3)> {
        self.actors.get(&actor)?.iter().rev().find_map(|interest| {
            if interest.target == Target::Entity(actor) {
                return Some((interest, eye + forward * 100.));
            }
            let target = position(interest.target)?;
            let delta = target - eye;
            (delta.length() >= 1. && delta.normalize().dot(forward) > 0.259)
                .then_some((interest, target))
        })
    }
}
pub fn scene_importance(intensity: f32, elapsed: f32) -> f32 {
    let t = (elapsed / 0.3).clamp(0., 1.);
    intensity.clamp(0., 3. * t * t - 2. * t * t * t)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refresh_deduplicates_and_eyes_select_tail_even_at_zero_importance() {
        let mut targets = LookTargets::default();
        targets.refresh(1, Target::Player, 0.8, 1., 10, 0);
        targets.refresh(1, Target::Entity(2), 0., 1., 10, 1);
        assert_eq!(
            targets
                .eye_target(1, Vec3::ZERO, Vec3::X, |_| Some(Vec3::X * 4.))
                .unwrap()
                .0
                .target,
            Target::Entity(2)
        );
        targets.refresh(1, Target::Player, 0.2, 1., 11, 0);
        assert_eq!(targets.report()[&1].len(), 2);
        assert_eq!(targets.report()[&1][1].importance, 0.8);
        targets.refresh(1, Target::Player, 0.1, 1.015, 11, 0);
        assert_eq!(targets.report()[&1][1].importance, 0.1);
        targets.cleanup(1.12, |_| true);
        assert!(targets.report().is_empty());
    }
    #[test]
    fn self_looks_forward_and_dead_or_behind_targets_are_rejected() {
        let mut targets = LookTargets::default();
        targets.refresh(1, Target::Entity(1), 1., 0., 3, 0);
        assert_eq!(
            targets
                .eye_target(1, Vec3::ZERO, Vec3::X, |_| None)
                .unwrap()
                .1,
            Vec3::X * 100.
        );
        targets.refresh(1, Target::Entity(2), 1., 0., 3, 1);
        assert_eq!(
            targets
                .eye_target(1, Vec3::ZERO, Vec3::X, |_| Some(-Vec3::X))
                .unwrap()
                .0
                .target,
            Target::Entity(1)
        );
        targets.cleanup(0., |id| id != 2);
        assert_eq!(targets.report()[&1].len(), 1);
        targets.cleanup(0., |_| false);
        assert!(targets.report().is_empty());
        assert!((scene_importance(1., 0.15) - 0.5).abs() < 1e-6);
        assert_eq!(scene_importance(0.2, 0.15), 0.2);
    }
}
/// CAI_BaseActor head control state: head correction goal (pitch, yaw, roll degrees),
/// last head direction and influence.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Head {
    pub goal: Vec3,
    pub direction: Vec3,
    pub influence: f32,
}
/// SDK `UTIL_Approach(target, value, speed)`.
fn approach(target: f32, value: f32, speed: f32) -> f32 {
    let delta = target - value;
    if delta > speed {
        value + speed
    } else if delta < -speed {
        value - speed
    } else {
        target
    }
}
impl Head {
    /// One update of MaintainLookTargets/UpdateHeadControl, scaled from the 0.1 sec NPC think
    /// to `dt`. `targets` are (direction from the eye, interest); `frame` is the world rotation
    /// of the animated "forward" attachment. It is parented to the head bone, so it already
    /// carries the animation and the current head pose: like the SDK, the correction is
    /// measured in that tilted frame, not a level body frame.
    pub fn update(&mut self, targets: &[(Vec3, f32)], frame: glam::Mat3, dt: f32) {
        let thinks = (dt / 0.1).clamp(0., 2.);
        self.goal *= 0.8f32.powf(thinks);
        let forward = frame.x_axis.normalize_or(Vec3::X);
        let mut head = forward;
        let mut influence = 0.;
        for &(dir, interest) in targets {
            if interest <= 0. || !dir.is_finite() || dir.length_squared() < 1e-6 {
                continue;
            }
            let dir = dir.normalize();
            if influence == 0. {
                head = dir;
                influence = interest;
            } else {
                influence = influence * (1. - interest) + interest;
                let w = interest / influence;
                head = head * (1. - w) + dir * w;
            }
        }
        if influence > 0. {
            self.direction = head;
            self.influence = influence.min(1.);
        } else {
            let blend = 0.8f32.powf(thinks);
            self.direction = (self.direction * blend + head * (1. - blend)).normalize_or(forward);
            self.influence = (self.influence - 0.2 * thinks).max(0.);
        }
        // UpdateHeadControl with scene_clamplookat: target in the forward frame. Aligning the
        // frame's X to it (Studio_AlignIKMatrix) keeps left.z = 0, so MatrixAngles gives
        // pitch/yaw of the local direction and zero roll.
        let local = frame.transpose() * self.direction;
        let mut local = local.normalize_or(Vec3::X);
        local.z *= local.x.clamp(0.1, 1.);
        let local = local.normalize_or(Vec3::X);
        let influence = self.influence * (local.x * 2. + 2.).clamp(0., 1.);
        let yaw = local.y.atan2(local.x).to_degrees();
        let pitch = -local.z.clamp(-1., 1.).asin().to_degrees();
        let s0 = (1. - influence + 0.3 * influence).powf(thinks);
        let s1 = 1. - s0;
        self.goal.x = approach(self.goal.x * s0 + pitch * s1, self.goal.x, 10. * thinks);
        self.goal.y = approach(self.goal.y * s0 + yaw * s1, self.goal.y, 30. * thinks);
        self.goal.z = approach(self.goal.z * s0, self.goal.z, 10. * thinks);
    }
}
#[cfg(test)]
mod head_tests {
    use super::*;
    #[test]
    fn correction_is_measured_in_the_tilted_forward_frame() {
        // Head already pitched 20 degrees down (Source pitch down = toward -Z).
        let frame = glam::Mat3::from_rotation_y(20f32.to_radians());
        // A self-interest follows the forward attachment: no correction.
        let mut head = Head::default();
        head.update(&[(frame.x_axis, 1.)], frame, 0.1);
        assert!(head.goal.length() < 1e-3, "{:?}", head.goal);
        // A level target is 20 degrees up from the tilted head: pitch correction -20 at most,
        // limited to 10 degrees per think, instead of the zero a level frame would give.
        let mut level = Head::default();
        level.update(&[(Vec3::X, 1.)], frame, 0.1);
        assert!((level.goal.x + 10.).abs() < 1e-3, "{:?}", level.goal);
    }
    #[test]
    fn head_turns_toward_interest_at_think_rates_and_relaxes_without_one() {
        let mut head = Head::default();
        // Target 60 degrees to the left of a body facing +X.
        let dir = Vec3::new(0.5, 3f32.sqrt() / 2., 0.);
        head.update(&[(dir, 1.)], glam::Mat3::IDENTITY, 0.1);
        assert!(
            (head.goal.y - 30.).abs() < 1e-3,
            "30 degrees per think: {}",
            head.goal.y
        );
        for _ in 0..30 {
            head.update(&[(dir, 1.)], glam::Mat3::IDENTITY, 0.1);
        }
        assert!(head.goal.y > 45. && head.goal.y < 60., "{}", head.goal.y);
        // Body already facing the target: no correction needed.
        let mut aligned = Head::default();
        aligned.update(&[(Vec3::X, 1.)], glam::Mat3::IDENTITY, 0.1);
        assert!(aligned.goal.y.abs() < 1e-3);
        // Interest gone: influence and goal relax toward zero.
        for _ in 0..40 {
            head.update(&[], glam::Mat3::IDENTITY, 0.1);
        }
        assert!(head.influence == 0. && head.goal.y.abs() < 1.);
    }
}
