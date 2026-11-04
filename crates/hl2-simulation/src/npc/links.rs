//! Navigation beyond the ground: graph jump and climb links (DESIGN 14o session 44).
//! An actor with `MoveCaps` routes over them; a jump hands the actor to the host's step
//! fall until `Controller::landed`, and a climb moves it in the air on its sequences' root
//! speed. The behavior of CAI_Navigator's MoveJump and MoveClimb, written for this controller.
use super::{goal_tolerance, update, Actor, Controller, Goal, GoalBlockReason, GoalState};
use super::{MoveRequest, PathStats, Route};
use crate::ai_hull::HullType;
use crate::npc_probe::{self, Hull, NpcCollisionWorld, Query};
use glam::Vec3;
use modkit_core::animation::RootMotion;
use source_assets::models::MotionSequence;
use source_assets::navigation::{Graph, Link};
use std::collections::BTreeMap;
use std::sync::Arc;

/// bits_CAP_MOVE_GROUND, bits_CAP_MOVE_JUMP and bits_CAP_MOVE_CLIMB in a link's types.
const MOVE_GROUND: u8 = 1;
const MOVE_JUMP: u8 = 2;
const MOVE_CLIMB: u8 = 8;
/// sv_gravity x GetJumpGravity and CAI_BaseNPC::GetMaxJumpSpeed.
const JUMP_GRAVITY: f32 = 600.;
const MAX_JUMP_SPEED: f32 = 350.;
/// MoveClimbExecute's floor under the sequence speed (a mount or dismount that ended early).
const MIN_CLIMB_SPEED: f32 = 30.;
/// Up to this direction z a climb link is a step around the ladder, not a climb.
const LEVEL_CLIMB: f32 = 0.1;
/// The jump probe's bisection on the apex height: first step (and ceiling) and least step.
const JUMP_SEARCH_STEP: f32 = 1024.;
const JUMP_SEARCH_LEAST: f32 = 16.;
/// The activities a mover's capabilities need sequences for.
pub const ACTIVITIES: [&str; 3] = ["ACT_JUMP", "ACT_CLIMB_UP", "ACT_CLIMB_DISMOUNT"];

/// How a route waypoint is reached: the type of the link that leads to it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Nav {
    #[default]
    Ground,
    Jump,
    /// The node's yaw, faced while climbing.
    Climb {
        yaw: f32,
    },
}
/// CAI_BaseNPC::IsJumpLegal's limits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JumpLimits {
    pub rise: f32,
    pub drop: f32,
    pub distance: f32,
}
impl JumpLimits {
    /// CFastZombie::IsJumpLegal.
    pub const FAST_ZOMBIE: Self = Self {
        rise: 220.,
        drop: 384.,
        distance: 512.,
    };
    pub fn legal(self, start: Vec3, apex: Vec3, end: Vec3) -> bool {
        const SLACK: f32 = 0.1;
        end.z - start.z <= self.rise + SLACK
            && start.z - end.z <= self.drop + SLACK
            && apex.z - start.z <= self.rise * 1.25
            && start.distance(end) <= self.distance + SLACK
    }
}
/// A sequence by name with its root motion.
#[derive(Clone, Debug)]
pub struct Clip {
    pub name: String,
    pub track: RootMotion,
}
/// GetInstantaneousVelocity of a track at `time`.
fn track_speed(track: &RootMotion, time: f32) -> f32 {
    let duration = track.duration().unwrap_or(0.);
    if duration <= 0. {
        return 0.;
    }
    let span = (duration * 0.1).min(0.05);
    let start = time.rem_euclid(duration).min(duration - span);
    // Past cycle 1 the movement would wrap and add a whole cycle.
    let moved = track.movement(start / duration, ((start + span) / duration).min(1.));
    moved.map_or(0., |m| m.position.length() / span)
}
impl Clip {
    /// The height the whole sequence rises.
    fn height(&self) -> f32 {
        self.track.movement(0., 1.).map_or(0., |m| m.position.z)
    }
    /// The time at which the sequence has risen `height` (GetMovementFrame).
    fn time_at_height(&self, height: f32) -> f32 {
        let rise = |cycle: f32| self.track.movement(0., cycle).map_or(0., |m| m.position.z);
        let (mut low, mut high) = (0_f32, 1_f32);
        for _ in 0..16 {
            let middle = (low + high) * 0.5;
            if rise(middle) < height {
                low = middle;
            } else {
                high = middle;
            }
        }
        high * self.track.duration().unwrap_or(0.)
    }
}
/// What a class can do on links beyond the ground (bits_CAP_MOVE_JUMP, bits_CAP_MOVE_CLIMB),
/// with the sequences it does them on. Default: ground only.
#[derive(Clone, Debug, Default)]
pub struct MoveCaps {
    jump: Option<(JumpLimits, Arc<str>)>,
    climb: Option<Arc<ClimbClips>>,
}
#[derive(Debug)]
struct ClimbClips {
    up: Clip,
    dismount: Option<Clip>,
}
impl MoveCaps {
    /// The class's capabilities from its sequences (`ACTIVITIES`): only npc_fastzombie so far.
    pub fn for_class(class: &str, motion: &BTreeMap<String, MotionSequence>) -> Self {
        if class != "npc_fastzombie" {
            return Self::default();
        }
        let clip = |activity: &str| {
            let (name, sequence) = motion.iter().find(|(_, m)| m.activity_name == activity)?;
            let track = sequence.blends.get(sequence.central_blend)?.clone();
            let name = name.clone();
            Some(Clip { name, track })
        };
        Self {
            jump: clip("ACT_JUMP").map(|c| (JumpLimits::FAST_ZOMBIE, c.name.into())),
            climb: clip("ACT_CLIMB_UP").map(|up| {
                let dismount = clip("ACT_CLIMB_DISMOUNT");
                Arc::new(ClimbClips { up, dismount })
            }),
        }
    }
    pub(super) fn climbs(&self) -> bool {
        self.climb.is_some()
    }
    fn bits(&self) -> u8 {
        let jump = if self.jump.is_some() { MOVE_JUMP } else { 0 };
        let climb = if self.climb.is_some() { MOVE_CLIMB } else { 0 };
        MOVE_GROUND | jump | climb
    }
    /// The type of a link this mover may take (CAI_Pathfinder::IsLinkUsable's move test).
    pub(super) fn link_nav(&self, link: &Link, hull: HullType) -> Option<Nav> {
        let types = link.accepted_move_types.get(hull.index())? & self.bits();
        if types & MOVE_GROUND != 0 {
            Some(Nav::Ground)
        } else if types & MOVE_CLIMB != 0 {
            Some(Nav::Climb { yaw: 0. })
        } else if types & MOVE_JUMP != 0 {
            Some(Nav::Jump)
        } else {
            None
        }
    }
    /// The link is walked: a local move can test it.
    pub(super) fn walks(&self, link: &Link, hull: HullType) -> bool {
        self.link_nav(link, hull) == Some(Nav::Ground)
    }
    /// What a link's length is multiplied by in the route search: CAI_Navigator::MovementCost
    /// doubles jumps and climbs, CFastZombie::MovementCost prefers jumping down and climbing
    /// up. None: a link this mover cannot take between these nodes.
    pub(super) fn link_scale(
        &self,
        link: &Link,
        hull: HullType,
        from: Vec3,
        to: Vec3,
    ) -> Option<f32> {
        let rise = to.z - from.z;
        match self.link_nav(link, hull)? {
            Nav::Ground => Some(1.),
            Nav::Jump => {
                let (limits, _) = self.jump.as_ref()?;
                let scale = if rise < 0. { 0.5 } else { 1.5 };
                limits.legal(from, to, to).then_some(2. * scale)
            }
            Nav::Climb { .. } => Some(2. * if rise > 0. { 0.5 } else { 4. }),
        }
    }
}
/// The nav type of each route waypoint (`nodes[i]` is reached by the link from `nodes[i - 1]`,
/// and the leg off the graph at the end walks). Empty for ground movers.
pub(super) fn route_navs(
    graph: &Graph,
    nodes: &[usize],
    caps: &MoveCaps,
    hull: HullType,
) -> Vec<Nav> {
    if caps.bits() == MOVE_GROUND {
        return Vec::new();
    }
    let mut navs = vec![Nav::Ground; nodes.len() + 1];
    for (index, pair) in nodes.windows(2).enumerate() {
        let (a, b) = (pair[0], pair[1]);
        let link = graph.links.iter().find(|l| {
            (l.source == a && l.destination == b) || (l.source == b && l.destination == a)
        });
        navs[index + 1] = match link.and_then(|l| caps.link_nav(l, hull)) {
            Some(Nav::Climb { .. }) => Nav::Climb {
                yaw: graph.nodes[b].yaw,
            },
            Some(nav) => nav,
            None => Nav::Ground,
        };
    }
    navs
}
/// The end of the ground waypoints from `next`: SimplifyPath leaves jumps and climbs alone.
pub(super) fn ground_end(route: &Route, next: usize) -> usize {
    (next..route.feet.len())
        .find(|&i| route.navs.get(i).is_some_and(|nav| *nav != Nav::Ground))
        .unwrap_or(route.feet.len())
}
/// A jump arc: the launch velocity, its apex and the apex height above the start.
#[derive(Clone, Copy, Debug)]
struct JumpArc {
    velocity: Vec3,
    apex: Vec3,
    height: f32,
}
/// The arc through `from` and `to` with at least this apex height, raised as far as it must
/// be to stay up at the top horizontal speed and to clear the step.
fn jump_arc(from: Vec3, to: Vec3, least_height: f32) -> JumpArc {
    let step = to.z - from.z;
    let flat = (to - from).truncate();
    let distance = flat.length();
    let heading = (flat / distance.max(1e-3)).extend(0.);
    let half_flight = distance / MAX_JUMP_SPEED * 0.5;
    let height = least_height
        .max(0.5 * JUMP_GRAVITY * half_flight * half_flight)
        .max(step);
    let up = (2. * height / JUMP_GRAVITY).sqrt();
    let down = (2. * (height - step).abs() / JUMP_GRAVITY).sqrt();
    let speed = distance / (up + down).max(1e-3);
    JumpArc {
        velocity: heading * speed + Vec3::Z * (2. * JUMP_GRAVITY * height).sqrt(),
        apex: from + heading * speed * up + Vec3::Z * height,
        height,
    }
}
/// What stopped a swept arc.
enum Sweep {
    Clear,
    Ceiling(Option<usize>),
    Wall(Option<usize>),
}
/// The arc in ten hull steps (a rough sweep, as the move probe's).
fn sweep<W: NpcCollisionWorld>(
    world: &W,
    hull: Hull,
    query: Query<'_>,
    (from, to): (Vec3, Vec3),
    arc: &JumpArc,
) -> Sweep {
    let total = (to - from).truncate().length() / arc.velocity.truncate().length().max(1e-3);
    let step = total / 10.;
    let (mut at, mut velocity, mut time) = (from, arc.velocity, 0.);
    while time < total - 0.01 {
        let next = at + (velocity - Vec3::Z * (0.5 * JUMP_GRAVITY * step)) * step;
        let hit = world.npc_trace_hull(at, next, hull, query);
        if hit.trace.start_solid || hit.trace.fraction < 0.99 {
            return if hit.trace.normal.z < 0. {
                Sweep::Ceiling(hit.entity)
            } else {
                Sweep::Wall(hit.entity)
            };
        }
        velocity -= Vec3::Z * (JUMP_GRAVITY * step);
        at = next;
        time += step;
    }
    Sweep::Clear
}
/// The move probe's jump limit: the lowest legal arc from `from` to `to` that clears the way
/// (a bisection on the apex height). Err: illegal or blocked (by this entity, if any).
pub(super) fn jump_velocity<W: NpcCollisionWorld>(
    world: &W,
    hull: Hull,
    query: Query<'_>,
    limits: JumpLimits,
    (from, to): (Vec3, Vec3),
) -> Result<Vec3, Option<usize>> {
    let flat = (to - from).truncate().length();
    if !limits.legal(from, to, to) || flat <= 0. || !npc_probe::fits(world, to, hull, query) {
        return Err(None);
    }
    let baseline = jump_arc(from, to, 0.).height;
    let (mut height, mut step) = (baseline, JUMP_SEARCH_STEP);
    let (mut best, mut blocker) = (None, None);
    loop {
        let tried = jump_arc(from, to, height);
        height = tried.height;
        step *= 0.5;
        if !limits.legal(from, tried.apex, to) {
            height -= step;
        } else {
            match sweep(world, hull, query, (from, to), &tried) {
                Sweep::Clear => {
                    best = Some(tried.velocity);
                    height -= step;
                }
                Sweep::Ceiling(entity) => {
                    blocker = entity;
                    height -= step;
                }
                Sweep::Wall(entity) => {
                    blocker = entity;
                    height += step;
                }
            }
        }
        if height <= baseline || height > JUMP_SEARCH_STEP || step < JUMP_SEARCH_LEAST {
            return best.ok_or(blocker);
        }
    }
}
/// The animation of a climb (MoveClimbStart and MoveClimbExecute's activities).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ClimbPlay {
    /// The movement activity, on the level links around a ladder.
    Run,
    Up,
    /// ACT_CLIMB_DOWN (the fast zombie plays its climb up for it).
    Down,
    Dismount,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) enum Traversal {
    #[default]
    None,
    /// In the host's step fall until `Controller::landed`.
    Airborne,
    Climb(ClimbPlay),
}
/// A goal's jump or climb in progress.
#[derive(Clone, Debug, Default)]
pub(super) struct GoalLink {
    pub(super) traversal: Traversal,
    /// The jump velocity for the host (this tick).
    pub(super) launch: Option<Vec3>,
    /// A request that came during the traversal (deviation: Source reroutes on climb nodes
    /// only in mid-climb and fails otherwise; ours finishes the climb, then plans).
    pub(super) held: Option<MoveRequest>,
    /// Plan the request anew at the next tick.
    pub(super) replan: bool,
}
impl GoalLink {
    pub(super) fn traversing(&self) -> bool {
        self.traversal != Traversal::None
    }
    pub(super) fn label(&self) -> &'static str {
        match self.traversal {
            Traversal::None => "",
            Traversal::Airborne => "jump",
            Traversal::Climb(_) => "climb",
        }
    }
}
/// The goal's next waypoint is a jump or climb: `advance` stops short of it.
pub(super) fn at_link(goal: &Goal) -> bool {
    let nav = goal.route.navs.get(goal.next);
    nav.is_some_and(|nav| *nav != Nav::Ground)
}
/// The sequence a traversal plays, if it is not the movement's own.
pub(super) fn played(goal: &Goal, actor: &Actor) -> Option<String> {
    match goal.link.traversal {
        Traversal::None | Traversal::Climb(ClimbPlay::Run) => None,
        Traversal::Airborne => actor.caps.jump.as_ref().map(|(_, name)| name.to_string()),
        Traversal::Climb(ClimbPlay::Dismount) => {
            let clips = actor.caps.climb.as_ref()?;
            clips.dismount.as_ref().map(|c| c.name.clone())
        }
        Traversal::Climb(_) => actor.caps.climb.as_ref().map(|c| c.up.name.clone()),
    }
}
/// A running jump or climb, or one that starts at this waypoint. True: this tick's movement
/// is done here.
pub(super) fn traverse<W: NpcCollisionWorld>(
    world: &W,
    actor: &Actor,
    goal: &mut Goal,
    query: Query<'_>,
    dt: f32,
    stats: &mut PathStats,
) -> Result<bool, GoalBlockReason> {
    match goal.link.traversal {
        Traversal::Airborne => goal.animation_time += dt,
        Traversal::Climb(_) => climb_step(actor, goal, dt),
        Traversal::None => match goal.route.navs.get(goal.next) {
            Some(Nav::Jump) => start_jump(world, actor, goal, query, stats)?,
            Some(Nav::Climb { .. }) => {
                stats.climbs += 1;
                goal.animation_time = 0.;
                climb_step(actor, goal, dt);
            }
            _ => return Ok(false),
        },
    }
    Ok(true)
}
fn start_jump<W: NpcCollisionWorld>(
    world: &W,
    actor: &Actor,
    goal: &mut Goal,
    query: Query<'_>,
    stats: &mut PathStats,
) -> Result<(), GoalBlockReason> {
    let target = goal.route.feet.get(goal.next);
    let target = *target.ok_or(GoalBlockReason::NoRoute)?;
    let (limits, _) = actor.caps.jump.as_ref().ok_or(GoalBlockReason::NoRoute)?;
    let ends = (goal.feet, target);
    let launch = jump_velocity(world, actor.ground.hull, query, *limits, ends);
    let velocity = launch.map_err(|blocker| {
        stats.jump_failures += 1;
        GoalBlockReason::Collision {
            blocker,
            reason: "jump".to_owned(),
        }
    })?;
    stats.jumps += 1;
    goal.link.traversal = Traversal::Airborne;
    goal.link.launch = Some(velocity);
    goal.animation_time = 0.;
    goal.yaw = velocity.y.atan2(velocity.x).to_degrees();
    Ok(())
}
/// The climb sequence for this direction: the dismount takes over near the top.
fn climb_play(clips: &ClimbClips, goal: &mut Goal, direction: Vec3, distance: f32) -> ClimbPlay {
    let vertical = direction.z.abs() > LEVEL_CLIMB;
    let wanted = if direction.z > -0.01 {
        ClimbPlay::Up
    } else {
        ClimbPlay::Down
    };
    let mut play = match goal.link.traversal {
        Traversal::Climb(ClimbPlay::Dismount) => ClimbPlay::Dismount,
        Traversal::Climb(_) if vertical => wanted,
        Traversal::Climb(play) => play,
        _ if vertical => wanted,
        _ => ClimbPlay::Run,
    };
    let previous = goal.link.traversal;
    if play == ClimbPlay::Up {
        let left = (goal.next..goal.route.feet.len())
            .take_while(|&i| matches!(goal.route.navs.get(i), Some(Nav::Climb { .. })))
            .count();
        let dismount = clips.dismount.as_ref();
        if let Some(clip) = dismount.filter(|c| left <= 2 && distance < c.height().abs()) {
            play = ClimbPlay::Dismount;
            goal.animation_time = clip.time_at_height(clip.height() - distance);
        }
    }
    if previous != Traversal::Climb(play) && play != ClimbPlay::Dismount {
        goal.animation_time = 0.;
    }
    goal.link.traversal = Traversal::Climb(play);
    play
}
/// The climb sequence's speed now (MoveClimbExecute).
fn climb_speed(actor: &Actor, clips: &ClimbClips, goal: &Goal, play: ClimbPlay) -> f32 {
    let time = goal.animation_time;
    let speed = match play {
        ClimbPlay::Up | ClimbPlay::Down => track_speed(&clips.up.track, time),
        ClimbPlay::Dismount => clips
            .dismount
            .as_ref()
            .map_or(0., |c| track_speed(&c.track, time)),
        ClimbPlay::Run => actor
            .locomotion
            .track(goal.request.style)
            .map_or(0., |track| track_speed(track, time)),
    };
    if clips.dismount.is_some() {
        speed.max(MIN_CLIMB_SPEED)
    } else {
        100.
    }
}
/// One tick of a climb: a straight line to each climb waypoint, gravity off.
fn climb_step(actor: &Actor, goal: &mut Goal, dt: f32) {
    let Some(clips) = actor.caps.climb.as_deref() else {
        goal.link.traversal = Traversal::None;
        return;
    };
    let mut left = dt;
    // A ladder's mount and dismount nodes share a spot: several waypoints can pass in a tick.
    for _ in 0..8 {
        let Some(&destination) = goal.route.feet.get(goal.next) else {
            break;
        };
        let delta = destination - goal.feet;
        let distance = delta.length();
        let direction = delta.normalize_or_zero();
        let play = climb_play(clips, goal, direction, distance);
        let speed = climb_speed(actor, clips, goal, play);
        if let Some(Nav::Climb { yaw }) = goal.route.navs.get(goal.next) {
            goal.yaw = *yaw;
        }
        if distance >= speed * left {
            goal.feet += direction * (speed * left);
            goal.animation_time += left;
            return;
        }
        let spent = distance / speed;
        left -= spent;
        goal.animation_time += spent;
        goal.feet = destination;
        goal.next += 1;
        if !matches!(goal.route.navs.get(goal.next), Some(Nav::Climb { .. })) {
            break;
        }
    }
    finish(actor, goal);
}
/// A traversal is over: the next waypoint walks, unless that was the last one.
fn finish(actor: &Actor, goal: &mut Goal) {
    goal.link.traversal = Traversal::None;
    goal.animation_time = 0.;
    if let Some(request) = goal.link.held.take() {
        goal.tolerance = goal_tolerance(actor, &request);
        goal.request = request;
        goal.link.replan = true;
    } else if goal.next >= goal.route.feet.len() {
        goal.state = GoalState::Arrived;
    }
}
impl Controller {
    pub fn set_move_caps(&mut self, entity: usize, caps: MoveCaps) {
        if let Some(actor) = self.actors.get_mut(&entity) {
            actor.caps = caps;
        }
    }
    /// Jumps started since the last call: (entity, launch velocity) for the host's step fall.
    pub fn take_launches(&mut self) -> Vec<(usize, Vec3)> {
        std::mem::take(&mut self.launches)
    }
    /// Climbers whose climb was stopped: gravity takes them (MoveClimbStop).
    pub fn take_drops(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.drops)
    }
    /// The actors in a jump: the host calls `landed` when each is on the ground.
    pub fn jumpers(&self) -> Vec<usize> {
        self.goals
            .iter()
            .filter(|(_, g)| g.link.traversal == Traversal::Airborne)
            .map(|(key, _)| key.actor)
            .collect()
    }
    /// The actors climbing, and whether each is dismounting.
    pub fn climbers(&self) -> Vec<(usize, bool)> {
        self.goals
            .iter()
            .filter_map(|(key, g)| match g.link.traversal {
                Traversal::Climb(play) => Some((key.actor, play == ClimbPlay::Dismount)),
                _ => None,
            })
            .collect()
    }
    /// MoveJump's landing: the route goes on from the waypoint jumped to.
    pub fn landed(&mut self, entity: usize) {
        let Some(actor) = self.actors.get(&entity) else {
            return;
        };
        for (key, goal) in self.goals.iter_mut().filter(|(k, _)| k.actor == entity) {
            if goal.link.traversal != Traversal::Airborne {
                continue;
            }
            goal.link.traversal = Traversal::None;
            if !goal.state.is_moving() {
                continue;
            }
            goal.next += 1;
            finish(actor, goal);
            if goal.state.is_arrived() {
                self.pending.push(update(*key, goal, &self.doors, None));
            }
        }
    }
}

#[cfg(test)]
mod tests;
