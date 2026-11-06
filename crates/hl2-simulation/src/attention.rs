//! NPC look targets (SDK CAI_BaseActor / CAI_InterestTarget): timed explicit interests
//! refreshed by authored scene LOOKAT events and AI AddLookTarget, and the random look
//! queue MaintainLookTargets fills with timed glances (PickRandomLookTarget /
//! MakeRandomLookTarget). Synthetic queues, tactical/navigation looks, expressions and
//! PVS gating are unfinished.
use glam::Vec3;
use serde::Serialize;
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum Target {
    Player,
    Entity(usize),
    /// LOOKAT_POSITION (MakeRandomLookTarget's made-up point).
    Position(Vec3),
}
#[derive(Clone, Debug, Serialize)]
pub struct Interest {
    pub target: Target,
    pub importance: f32,
    pub start: f64,
    pub end: f64,
    pub scene: usize,
    pub event: usize,
    /// CAI_InterestTarget_t::m_flRamp: the ramp as a fraction of the duration.
    pub ramp: f32,
}
impl Interest {
    /// CAI_InterestTarget_t::Interest: zero outside [start, end]; with a ramp, eases in
    /// (1 - ExponentialDecay(0.2, ramp, t)) and out (smoothstep over the last ramp
    /// fraction); times the importance.
    pub fn interest(&self, now: f64) -> f32 {
        let span = (self.end - self.start).max(1e-6);
        let t = ((now - self.start) / span) as f32;
        if !(0. ..=1.).contains(&t) {
            return 0.;
        }
        let ramp = self.ramp;
        let t = if ramp != 0. && t < 1. - ramp {
            1. - (0.2f32.ln() / ramp * t).exp()
        } else if t > 1. - ramp {
            let t = (1. - t) / ramp;
            3. * t * t - 2. * t * t * t
        } else {
            1.
        };
        t * self.importance
    }
}
/// A possible random look target (a CEntitySphereQuery entry) for PickRandomLookTarget.
#[derive(Clone, Copy, Debug)]
pub struct Candidate {
    pub target: Target,
    /// EyePosition of the target.
    pub eye: Vec3,
    pub npc: bool,
    pub moving: bool,
}
/// The looking actor for MaintainLookTargets.
#[derive(Clone, Copy, Debug)]
pub struct Looker {
    pub eye: Vec3,
    /// BodyDirection3D (ValidHeadTarget).
    pub body_forward: Vec3,
    /// GetVectors forward/right/up (MakeRandomLookTarget).
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    /// NPC_STATE_SCRIPT (scripted_sequence ownership): no random picks.
    pub scripted: bool,
}
#[derive(Default)]
pub struct LookTargets {
    actors: BTreeMap<usize, Vec<Interest>>,
    /// m_randomLookQueue per actor.
    random: BTreeMap<usize, Vec<Interest>>,
    /// m_flNextRandomLookTime per actor.
    next_random: BTreeMap<usize, f64>,
    rng: u64,
    /// The next 0.1 s NPC think of `think`.
    next_think: f64,
    /// Actors in the player's PVS (COND_IN_PVS), refreshed by the host; None = unknown,
    /// which fails open (every actor updates its head and eyes).
    pvs: Option<std::collections::BTreeSet<usize>>,
    /// The scene time of the last PVS refresh.
    pub pvs_time: f64,
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
            ramp: 0.,
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
            ramp: 0.,
        });
    }
    pub fn cleanup(&mut self, now: f64, alive: impl Fn(usize) -> bool) {
        for queues in [&mut self.actors, &mut self.random] {
            queues.retain(|actor, queue| {
                queue.retain(|i| {
                    i.end >= now
                        && match i.target {
                            Target::Player | Target::Position(_) => true,
                            Target::Entity(id) => alive(id),
                        }
                });
                alive(*actor) && !queue.is_empty()
            });
        }
        self.next_random.retain(|actor, _| alive(*actor));
    }
    pub fn report(&self) -> &BTreeMap<usize, Vec<Interest>> {
        &self.actors
    }
    /// COND_IN_PVS for animated-face actors (host BSP PVS at the actor's eyes).
    pub fn set_pvs(&mut self, actors: std::collections::BTreeSet<usize>, now: f64) {
        self.pvs = Some(actors);
        self.pvs_time = now;
    }
    /// MaintainLookTargets returns before the head and eye updates when the player cannot
    /// see the actor; random picks still happen.
    pub fn in_pvs(&self, actor: usize) -> bool {
        self.pvs.as_ref().is_none_or(|set| set.contains(&actor))
    }
    pub fn random_report(&self) -> &BTreeMap<usize, Vec<Interest>> {
        &self.random
    }
    /// The active interests in MaintainLookTargets order: random, then explicit.
    pub fn active(&self, actor: usize) -> impl Iterator<Item = &Interest> {
        self.random
            .get(&actor)
            .into_iter()
            .flatten()
            .chain(self.actors.get(&actor).into_iter().flatten())
    }
    /// Actors with any interest (explicit or random).
    pub fn actors(&self) -> impl Iterator<Item = usize> + '_ {
        self.actors.keys().chain(self.random.keys()).copied()
    }
    fn next_u32(&mut self) -> u32 {
        if self.rng == 0 {
            self.rng = 0x9e37_79b9_7f4a_7c15;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 32) as u32
    }
    /// RandomInt(min, max), inclusive.
    fn random_int(&mut self, min: i32, max: i32) -> i32 {
        min + (self.next_u32() % ((max - min + 1).max(1) as u32)) as i32
    }
    fn random_float(&mut self, min: f32, max: f32) -> f32 {
        min + (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32 * (max - min)
    }
    /// The random-queue part of CAI_BaseActor::MaintainLookTargets for one NPC think:
    /// explicit interests cut random ones short (end = now + ramp + 0.2, with the SDK's
    /// ramp fraction) and push the next random pick 1 s away; otherwise, when due and
    /// not scripted, PickLookTarget(1.5, 2.5) and the next pick at duration - 0.4.
    /// `candidates` is only called when a pick is due; `visible` is FVisible.
    pub fn maintain(
        &mut self,
        actor: usize,
        now: f64,
        looker: Looker,
        candidates: impl FnOnce() -> Vec<Candidate>,
        visible: impl Fn(&Candidate) -> bool,
    ) {
        if self.actors.get(&actor).is_some_and(|q| !q.is_empty()) {
            for interest in self.random.get_mut(&actor).into_iter().flatten() {
                let ramp = f64::from(interest.ramp);
                if now < interest.end - ramp - 0.2 {
                    interest.end = now + ramp + 0.2;
                }
            }
            self.next_random.insert(actor, now + 1.);
            return;
        }
        if looker.scripted || now < self.next_random.get(&actor).copied().unwrap_or(0.) {
            return;
        }
        let candidates = candidates();
        let duration = self.pick_look_target(actor, now, &looker, &candidates, &visible);
        self.next_random
            .insert(actor, now + f64::from(duration) - 0.4);
    }
    /// PickLookTarget(1.5, 2.5) without an enemy (PickTacticalLookTarget never applies):
    /// PickRandomLookTarget, else MakeRandomLookTarget; queued like CAI_InterestTarget::Add.
    fn pick_look_target(
        &mut self,
        actor: usize,
        now: f64,
        looker: &Looker,
        candidates: &[Candidate],
        visible: &dyn Fn(&Candidate) -> bool,
    ) -> f32 {
        let (min_time, max_time) = (1.5, 2.5);
        let mut duration = self.random_float(min_time, max_time);
        let mut influence = self.random_float(0.3, 0.5);
        let mut ramp = self.random_float(0.2, 0.4);
        let target = match self.pick_random_look_target(actor, looker, candidates, visible) {
            Some((target, held)) => {
                duration = held.unwrap_or(duration);
                target
            }
            None => {
                // MakeRandomLookTarget: a point ahead, barely weighted, slowly ramped.
                let point = looker.eye
                    + looker.forward * 128.
                    + looker.right * self.random_float(-32., 32.)
                    + looker.up * self.random_float(-16., 16.);
                duration = self.random_float(min_time, max_time);
                influence = 0.01;
                ramp = self.random_float(0.8, 2.8);
                Target::Position(point)
            }
        };
        let queue = self.random.entry(actor).or_default();
        match target {
            Target::Position(_) => queue.retain(|i| i.target != target),
            _ => queue.retain(|i| !(i.target == target && i.ramp == 0.)),
        }
        queue.push(Interest {
            target,
            importance: influence,
            start: now,
            end: now + f64::from(duration),
            scene: crate::npc::AI_SCENE,
            event: 0,
            ramp: ramp / duration.max(1e-6),
        });
        duration
    }
    /// PickRandomLookTarget's 360-unit entity sphere (no navigation/combat branches): a
    /// moving player, or a standing one 1 time in 3, wins at once and is held 1-4 s;
    /// others get random importance 1-100, x10 for NPCs and x10 more when moving; all
    /// need FVisible and ValidHeadTarget; fewer considered than RandomInt(0, 5) is none.
    fn pick_random_look_target(
        &mut self,
        actor: usize,
        looker: &Looker,
        candidates: &[Candidate],
        visible: &dyn Fn(&Candidate) -> bool,
    ) -> Option<(Target, Option<f32>)> {
        let queued: Vec<Target> = self
            .random
            .get(&actor)
            .into_iter()
            .flatten()
            .map(|i| i.target)
            .collect();
        let valid = |c: &Candidate| valid_head_target(looker.eye, looker.body_forward, c.eye);
        let mut best = None;
        let mut highest = 0;
        let mut considered = 0;
        for c in candidates {
            if c.target == Target::Entity(actor) || queued.contains(&c.target) {
                continue;
            }
            considered += 1;
            if c.target == Target::Player
                && (c.moving || self.random_int(0, 2) == 0)
                && visible(c)
                && valid(c)
            {
                let held = self.random_float(1., 4.);
                best = Some((c.target, Some(held)));
                break;
            }
            let mut importance = self.random_int(1, 100);
            if c.npc {
                importance *= 10;
                if c.moving {
                    importance *= 10;
                }
            }
            if importance > highest && visible(c) && valid(c) {
                highest = importance;
                best = Some((c.target, None));
            }
        }
        if considered < self.random_int(0, 5) {
            return None;
        }
        best
    }
    /// Eyes choose the newest valid active interest (MaintainLookTargets scans random,
    /// explicit, synthetic backwards: explicit first), independent of head importance.
    /// Explicit scene targets are not subject to random-interest visibility/distance filters.
    pub fn eye_target(
        &self,
        actor: usize,
        eye: Vec3,
        forward: Vec3,
        position: impl Fn(Target) -> Option<Vec3>,
    ) -> Option<(&Interest, Vec3)> {
        let active: Vec<&Interest> = self.active(actor).collect();
        active.into_iter().rev().find_map(|interest| {
            if interest.target == Target::Entity(actor) {
                return Some((interest, eye + forward * 100.));
            }
            let target = match interest.target {
                Target::Position(p) => p,
                other => position(other)?,
            };
            let delta = target - eye;
            (delta.length() >= 1. && delta.normalize().dot(forward) > 0.259)
                .then_some((interest, target))
        })
    }
}
/// NPC classes with bits_CAP_ANIMATEDFACE (they run CAI_BaseActor::MaintainLookTargets).
/// Cyclers keep the host's own eye fallback.
pub fn has_animated_face(class: &str) -> bool {
    matches!(
        class,
        "npc_metropolice"
            | "npc_combine_s"
            | "npc_citizen"
            | "npc_barney"
            | "npc_alyx"
            | "npc_breen"
            | "npc_eli"
            | "npc_kleiner"
            | "npc_mossman"
            | "npc_monk"
            | "npc_dog"
            | "generic_actor"
            | "monster_generic"
    )
}
/// One NPC think (0.1 s) of the random look queue for every animated-face NPC, with or
/// without `--ai` (CAI_BaseNPC::PostMovement -> MaintainLookTargets). Candidates are the
/// drawn studio-model entities and the player within 360 units (CEntitySphereQuery; moving
/// brush entities are not offered yet); FVisible is a line of sight between eyes.
pub fn think(
    scene: &mut crate::entities::Scene,
    world: &modkit_core::World,
    physics: &crate::physics::Physics,
    player_moving: bool,
) {
    let now = scene.time;
    if now < scene.look_targets.next_think {
        return;
    }
    scene.look_targets.next_think = now + 0.1;
    let player_eye = scene
        .look_position(world, Target::Player)
        .unwrap_or(Vec3::ZERO);
    for actor in 0..world.entities.len() {
        let state = &scene.states[actor];
        if state.killed || !state.visible || !has_animated_face(world.entities[actor].class()) {
            continue;
        }
        let frames = scene.attachment_frames(world, actor, &["eyes"]);
        let state = &scene.states[actor];
        let eye = frames[0].map_or(state.origin + Vec3::Z * 64., |m| m.w_axis.truncate());
        let forward = state.rotation * Vec3::X;
        let flat = Vec3::new(forward.x, forward.y, 0.).normalize_or(Vec3::X);
        let looker = Looker {
            eye,
            body_forward: flat,
            forward,
            right: state.rotation * Vec3::NEG_Y,
            up: state.rotation * Vec3::Z,
            // GetState() != NPC_STATE_SCRIPT, including a scripted post idle.
            scripted: state.scripted_by.is_some() || state.script_post_idle.is_some(),
        };
        let origin = state.origin;
        let candidates = || {
            let mut list = Vec::new();
            if player_eye.distance(origin) <= 360. + 64. {
                list.push(Candidate {
                    target: Target::Player,
                    eye: player_eye,
                    npc: false,
                    moving: player_moving,
                });
            }
            for instance in &world.model_instances {
                let Some(id) = instance.entity else {
                    continue;
                };
                if id == actor {
                    continue;
                }
                let Some(other) = scene.states.get(id) else {
                    continue;
                };
                if other.killed || !other.visible || other.origin.distance(origin) > 360. {
                    continue;
                }
                let npc = world.entities[id].class().starts_with("npc_");
                let moving = if npc {
                    other.is_moving()
                } else {
                    physics
                        .entity_body(id)
                        .and_then(|b| physics.body_state(b))
                        .is_some_and(|b| b.velocity.length() > 1.)
                };
                let Some(eye) = scene.look_position(world, Target::Entity(id)) else {
                    continue;
                };
                list.push(Candidate {
                    target: Target::Entity(id),
                    eye,
                    npc,
                    moving,
                });
            }
            list
        };
        let candidates = candidates();
        let visible = |c: &Candidate| {
            let excluded = match c.target {
                Target::Entity(id) => vec![actor, id],
                _ => vec![actor],
            };
            physics.projectile_ray(eye, c.eye, &excluded).is_none()
        };
        scene
            .look_targets
            .maintain(actor, now, looker, || candidates, visible);
    }
}
/// CAI_BaseActor::ValidHeadTarget: at least 1 unit away, in front of the body and
/// within 0.7 of vertical (+-90 degrees sideways, about +-45 up/down).
pub fn valid_head_target(eye: Vec3, body_forward: Vec3, target: Vec3) -> bool {
    let delta = target - eye;
    let distance = delta.length();
    if distance < 1. {
        return false;
    }
    let dir = delta / distance;
    dir.dot(body_forward) > 0. && dir.z.abs() < 0.7
}
/// CAI_BaseActor::HeadTargetValidity: 1 within +-135 degrees of the forward attachment,
/// fading to 0 straight behind; scaled down inside 10 units (2D distance).
pub fn head_target_validity(eye: Vec3, forward: Vec3, target: Vec3) -> f32 {
    let delta = target - eye;
    let flat = delta.truncate().length();
    if flat <= 1. {
        return 0.;
    }
    let dot = delta.normalize_or_zero().dot(forward);
    let mut interest = (3.4142 + 3.4142 * dot).clamp(0., 1.);
    if flat < 10. {
        interest *= (flat - 1.) / (10. - 1.);
    }
    interest
}
pub fn scene_importance(intensity: f32, elapsed: f32) -> f32 {
    let t = (elapsed / 0.3).clamp(0., 1.);
    intensity.clamp(0., 3. * t * t - 2. * t * t * t)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn looker() -> Looker {
        Looker {
            eye: Vec3::Z * 64.,
            body_forward: Vec3::X,
            forward: Vec3::X,
            right: Vec3::NEG_Y,
            up: Vec3::Z,
            scripted: false,
        }
    }
    #[test]
    fn interest_ramps_in_and_out_like_cai_interest_target() {
        // 2 s interest, importance 0.5, ramp 0.4 s = fraction 0.2.
        let i = Interest {
            target: Target::Player,
            importance: 0.5,
            start: 10.,
            end: 12.,
            scene: 0,
            event: 0,
            ramp: 0.2,
        };
        assert_eq!(i.interest(9.9), 0.);
        assert_eq!(i.interest(12.1), 0.);
        // Ramp in: 1 - 0.2^(t / ramp); at t = ramp fraction it reaches 0.8 x importance.
        assert!(
            (i.interest(10.4) - 0.5 * 0.8).abs() < 1e-4,
            "{}",
            i.interest(10.4)
        );
        // Ramp out: smoothstep over the last 20%: halfway down at 11.8.
        assert!(
            (i.interest(11.8) - 0.25).abs() < 1e-4,
            "{}",
            i.interest(11.8)
        );
        assert_eq!(i.interest(12.), 0.);
    }
    #[test]
    fn random_glances_expire_and_explicit_interests_cut_them_short() {
        let mut targets = LookTargets::default();
        let player = Candidate {
            target: Target::Player,
            eye: Vec3::new(100., 0., 64.),
            npc: false,
            moving: true,
        };
        // A moving visible player in front always wins and is held 1-4 s.
        targets.maintain(1, 0., looker(), || vec![player], |_| true);
        let glance = targets.random_report()[&1][0].clone();
        assert_eq!(glance.target, Target::Player);
        assert!((1. ..=4.).contains(&(glance.end - glance.start)));
        // No new pick before duration - 0.4.
        targets.maintain(1, 0.5, looker(), || vec![player], |_| true);
        assert_eq!(targets.random_report()[&1].len(), 1);
        // A scene LOOKAT cuts the glance to ramp + 0.2 and delays the next pick 1 s.
        targets.refresh(1, Target::Entity(7), 1., 0.6, 3, 0);
        targets.maintain(1, 0.6, looker(), || vec![player], |_| true);
        let cut = targets.random_report()[&1][0].end;
        assert!(
            (cut - (0.6 + f64::from(glance.ramp) + 0.2)).abs() < 1e-6,
            "{cut}"
        );
        // Eyes prefer the explicit interest.
        let eye = targets.eye_target(1, Vec3::Z * 64., Vec3::X, |_| Some(Vec3::new(50., 0., 64.)));
        assert_eq!(eye.unwrap().0.target, Target::Entity(7));
        // Everything expires; nothing stays on the player.
        targets.cleanup(5., |_| true);
        assert!(targets.random_report().is_empty() && targets.report().is_empty());
    }
    #[test]
    fn without_visible_candidates_the_actor_looks_ahead_with_low_influence() {
        let mut targets = LookTargets::default();
        let behind = Candidate {
            target: Target::Entity(2),
            eye: Vec3::new(-100., 0., 64.),
            npc: true,
            moving: false,
        };
        // The NPC behind the body never passes ValidHeadTarget: a made-up point ahead.
        targets.maintain(1, 0., looker(), || vec![behind], |_| true);
        let i = &targets.random_report()[&1][0];
        let Target::Position(p) = i.target else {
            panic!("{:?}", i.target);
        };
        assert!((p.x - 128.).abs() < 1e-3 && p.y.abs() <= 32. && (p.z - 64.).abs() <= 16.);
        assert_eq!(i.importance, 0.01);
        // Scripted actors make no random picks.
        let mut scripted = LookTargets::default();
        let mut l = looker();
        l.scripted = true;
        scripted.maintain(1, 0., l, || vec![behind], |_| true);
        assert!(scripted.random_report().is_empty());
    }
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
pub(crate) fn approach(target: f32, value: f32, speed: f32) -> f32 {
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
