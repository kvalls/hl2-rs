//! Conservative human-ground MOVETO controller in Source units/Z-up.
//! Authored central-forward root motion drives travel. This is not the full AI
//! motor: pose weighting, acceleration/turn planning, ETA, triangulation, door
//! opening, give-way and dynamic/hint link semantics remain unsupported.
use crate::npc_probe::{self, ActorHull, GroundConfig, Hull, NpcCollisionWorld, Query};
use glam::Vec3;
use modkit_core::{animation::RootMotion, World};
use serde::Serialize;
use source_assets::{models::MotionSequence, navigation::Graph, vpk::Vfs};
use std::collections::{BTreeMap, BTreeSet};

/// GoalKey::scene reserved for AI navigation goals (step 14); scenes use entity ids.
pub const AI_SCENE: usize = usize::MAX - 1;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct GoalKey {
    pub scene: usize,
    pub event: usize,
    pub actor: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum MoveStyle {
    Walk,
    Run,
}
impl MoveStyle {
    pub fn parse(value: &str) -> Result<Self, GoalBlockReason> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "walk" => Ok(Self::Walk),
            "run" => Ok(Self::Run),
            _ => Err(GoalBlockReason::UnsupportedStyle(value.to_owned())),
        }
    }
    fn sequence(self) -> &'static str {
        match self {
            Self::Walk => "walk_all",
            Self::Run => "run_all",
        }
    }
    fn activity(self) -> &'static str {
        match self {
            Self::Walk => "ACT_WALK",
            Self::Run => "ACT_RUN",
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct MoveRequest {
    pub target_entity: usize,
    pub target_feet: Vec3,
    pub style: MoveStyle,
    pub event_distance: f32,
    pub force_short: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct ActorPose {
    pub entity: usize,
    pub feet: Vec3,
    pub yaw_degrees: f32,
    pub scripted_by: Option<usize>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum GoalBlockReason {
    InvalidInput,
    UnsupportedHull,
    UnsupportedStyle(String),
    MissingActor,
    ScriptOwned,
    ActorBusy,
    MissingLocomotion(String),
    UnsupportedLocomotion(String),
    MissingGraph,
    MissingViewOffset,
    UnsupportedGraphState,
    NoRoute,
    InvalidGraph,
    SearchBudget,
    Collision {
        blocker: Option<usize>,
        reason: String,
    },
    RootMotion(String),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum GoalState {
    Moving,
    Arrived,
    Blocked(GoalBlockReason),
    Canceled,
}
impl GoalState {
    pub fn is_arrived(&self) -> bool {
        matches!(self, Self::Arrived)
    }
    pub fn is_moving(&self) -> bool {
        matches!(self, Self::Moving)
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct MoveUpdate {
    pub key: GoalKey,
    pub feet: Vec3,
    pub yaw_degrees: f32,
    pub sequence: Option<String>,
    pub animation_time: f32,
    pub state: GoalState,
    pub contact_normal: Vec3,
    /// Door this NPC is blocked by and should open (AI navigator door handling).
    pub door: Option<usize>,
}
#[derive(Clone, Debug, Serialize)]
pub struct GoalSnapshot {
    pub key: GoalKey,
    pub target_entity: usize,
    pub target_feet: Vec3,
    pub feet: Vec3,
    pub yaw_degrees: f32,
    pub tolerance: f32,
    pub state: GoalState,
    pub route_nodes: Vec<usize>,
    pub route_feet: Vec<Vec3>,
    pub next_waypoint: usize,
    pub sequence: Option<String>,
    pub animation_time: f32,
    /// Unknown motor ETA must never satisfy a scene's readiness test.
    pub motor_eta: Option<f32>,
    pub contact_normal: Vec3,
}

#[derive(Clone, Debug, Default)]
pub struct NavRestrictions {
    pub unusable_nodes: BTreeSet<usize>,
    pub unusable_links: BTreeSet<usize>,
    pub dynamic_link_state_unknown: bool,
}
impl NavRestrictions {
    /// Exclude hinted nodes rather than pretending to implement hint locks.
    /// Dynamic controllers require runtime state not present in the AIN file.
    pub fn from_world(world: &World, graph: &Graph) -> Self {
        let mut result = Self::default();
        for entity in &world.entities {
            match entity.class() {
                "info_node_hint" => {
                    if let Some(id) = entity.get("nodeid").and_then(|s| s.parse::<i32>().ok()) {
                        for (index, hammer_id) in graph.hammer_ids.iter().enumerate() {
                            if *hammer_id == id {
                                result.unusable_nodes.insert(index);
                            }
                        }
                    }
                }
                "info_node_link" | "info_node_link_controller" => {
                    result.dynamic_link_state_unknown = true;
                }
                _ => {}
            }
        }
        result
    }
}

#[derive(Clone, Debug, Default)]
pub struct Locomotion {
    tracks: BTreeMap<MoveStyle, RootMotion>,
    failures: BTreeMap<MoveStyle, GoalBlockReason>,
}
impl Locomotion {
    /// Load once at map/actor setup. Simulation ticks never access the VFS.
    pub fn load(vfs: &Vfs, model: &str) -> anyhow::Result<Self> {
        let wanted = ["walk_all".to_owned(), "run_all".to_owned()].into();
        let sequences = source_assets::models::read_root_motion(vfs, model, &wanted)?;
        Ok(Self::from_sequences(&sequences))
    }
    pub fn from_sequences(sequences: &BTreeMap<String, MotionSequence>) -> Self {
        let mut result = Self::default();
        for style in [MoveStyle::Walk, MoveStyle::Run] {
            let checked = sequences
                .get(style.sequence())
                .ok_or_else(|| GoalBlockReason::MissingLocomotion(style.sequence().to_owned()))
                .and_then(|sequence| {
                    if sequence.activity_name != style.activity() || sequence.flags & 1 == 0 {
                        return Err(GoalBlockReason::UnsupportedLocomotion(
                            style.sequence().to_owned(),
                        ));
                    }
                    let track = sequence.blends.get(sequence.central_blend).ok_or_else(|| {
                        GoalBlockReason::UnsupportedLocomotion("central blend index".to_owned())
                    })?;
                    validate_forward_track(track)?;
                    Ok(track.clone())
                });
            match checked {
                Ok(track) => {
                    result.tracks.insert(style, track);
                }
                Err(reason) => {
                    result.failures.insert(style, reason);
                }
            }
        }
        result
    }
    pub fn required_clips(&self) -> BTreeSet<String> {
        self.tracks
            .keys()
            .map(|style| style.sequence().to_owned())
            .collect()
    }
    fn track(&self, style: MoveStyle) -> Result<&RootMotion, GoalBlockReason> {
        self.tracks.get(&style).ok_or_else(|| {
            self.failures
                .get(&style)
                .cloned()
                .unwrap_or_else(|| GoalBlockReason::MissingLocomotion(style.sequence().to_owned()))
        })
    }
}
fn validate_forward_track(track: &RootMotion) -> Result<(), GoalBlockReason> {
    let error = || {
        GoalBlockReason::UnsupportedLocomotion(
            "only complete forward central tracks are supported".to_owned(),
        )
    };
    track
        .validate()
        .map_err(|e| GoalBlockReason::RootMotion(e.to_string()))?;
    if track
        .duration()
        .map_err(|e| GoalBlockReason::RootMotion(e.to_string()))?
        <= 0.
        || track
            .records
            .last()
            .is_none_or(|r| r.end_frame != track.frame_count as i32 - 1)
    {
        return Err(error());
    }
    let mut previous = 0.;
    for record in &track.records {
        let amount = (record.v0 + record.v1) * 0.5;
        if record.direction.distance(Vec3::X) > 0.0001
            || record.end_yaw_degrees.abs() > 0.0001
            || record.v0 <= 0.
            || record.v1 <= 0.
            || (record.cumulative_position - Vec3::X * (previous + amount)).length() > 0.001
        {
            return Err(error());
        }
        previous = record.cumulative_position.x;
    }
    Ok(())
}
pub fn normal_human(
    scale: f32,
    step_height: f32,
    step_down_multiplier: f32,
) -> Result<GroundConfig, GoalBlockReason> {
    if !scale.is_finite()
        || scale <= 0.
        || !step_height.is_finite()
        || step_height <= 0.
        || !step_down_multiplier.is_finite()
        || step_down_multiplier < 1.
    {
        return Err(GoalBlockReason::InvalidInput);
    }
    Ok(GroundConfig {
        hull: Hull {
            mins: Vec3::new(-13., -13., 0.) * scale,
            maxs: Vec3::new(13., 13., 72.) * scale,
        },
        step_height,
        step_down_multiplier,
    })
}

#[derive(Clone)]
struct Actor {
    scale: f32,
    ground: GroundConfig,
    locomotion: Locomotion,
    view_offset: Option<Vec3>,
}
struct Goal {
    request: MoveRequest,
    tolerance: f32,
    state: GoalState,
    route: Route,
    next: usize,
    feet: Vec3,
    yaw: f32,
    animation_time: f32,
    retry_time: f32,
    contact_normal: Vec3,
}
#[derive(Clone, Debug, Default)]
struct Route {
    nodes: Vec<usize>,
    feet: Vec<Vec3>,
    /// Door blocking the segment that ends at `feet[i]`, if any.
    doors: Vec<Option<usize>>,
}
pub struct Controller {
    graph: Option<Graph>,
    restrictions: NavRestrictions,
    actors: BTreeMap<usize, Actor>,
    goals: BTreeMap<GoalKey, Goal>,
    pending: Vec<MoveUpdate>,
    /// Door entities NPC routes may pass through by opening them.
    doors: BTreeSet<usize>,
    /// Actors registered only for AI goals: scene requests for them stay blocked as
    /// before (MissingActor), so scripted behavior does not change.
    ai_only: BTreeSet<usize>,
}
impl Controller {
    /// Doors that block a graph link do not invalidate it; NPCs open them on contact.
    pub fn set_doors(&mut self, world: &World) {
        self.doors = world
            .entities
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                matches!(
                    e.class(),
                    "prop_door_rotating" | "func_door" | "func_door_rotating"
                )
            })
            .map(|(id, _)| id)
            .collect();
    }
    pub fn new(graph: Option<Graph>, restrictions: NavRestrictions) -> Self {
        Self {
            graph,
            restrictions,
            actors: BTreeMap::new(),
            goals: BTreeMap::new(),
            pending: Vec::new(),
            doors: BTreeSet::new(),
            ai_only: BTreeSet::new(),
        }
    }
    /// register_actor for an actor that only AI goals (GoalKey::scene == AI_SCENE) move.
    pub fn register_ai_actor(
        &mut self,
        entity: usize,
        model_scale: f32,
        ground: GroundConfig,
        locomotion: Locomotion,
    ) -> Result<(), GoalBlockReason> {
        self.register_actor(entity, model_scale, ground, locomotion)?;
        self.ai_only.insert(entity);
        Ok(())
    }
    pub fn has_actor(&self, entity: usize) -> bool {
        self.actors.contains_key(&entity)
    }
    /// Stop one goal (the AI's TASK_STOP_MOVING); the actor keeps its pose.
    pub fn cancel(&mut self, key: GoalKey) {
        if let Some(goal) = self.goals.get_mut(&key) {
            if goal.state != GoalState::Canceled {
                goal.state = GoalState::Canceled;
                self.pending.push(update(key, goal, &self.doors));
            }
        }
    }
    /// Drop a finished goal's record.
    pub fn forget(&mut self, key: GoalKey) {
        if self
            .goals
            .get(&key)
            .is_some_and(|g| matches!(g.state, GoalState::Arrived | GoalState::Canceled))
        {
            self.goals.remove(&key);
        }
    }
    pub fn register_actor(
        &mut self,
        entity: usize,
        model_scale: f32,
        ground: GroundConfig,
        locomotion: Locomotion,
    ) -> Result<(), GoalBlockReason> {
        let normal = normal_human(model_scale, ground.step_height, ground.step_down_multiplier)?;
        if !ground.hull.valid()
            || ground.hull.mins.distance(normal.hull.mins) > 0.001
            || ground.hull.maxs.distance(normal.hull.maxs) > 0.001
        {
            return Err(GoalBlockReason::UnsupportedHull);
        }
        self.actors.insert(
            entity,
            Actor {
                scale: model_scale,
                ground,
                locomotion,
                view_offset: None,
            },
        );
        Ok(())
    }
    pub fn set_view_offset(&mut self, entity: usize, offset: Vec3) -> Result<(), GoalBlockReason> {
        if !offset.is_finite() {
            return Err(GoalBlockReason::InvalidInput);
        }
        self.actors
            .get_mut(&entity)
            .ok_or(GoalBlockReason::MissingActor)?
            .view_offset = Some(offset);
        Ok(())
    }
    pub fn actor_hull(&self, entity: usize) -> Option<Hull> {
        self.actors.get(&entity).map(|actor| actor.ground.hull)
    }
    pub fn status(&self, key: GoalKey) -> Option<&GoalState> {
        self.goals.get(&key).map(|goal| &goal.state)
    }
    pub fn snapshots(&self) -> Vec<GoalSnapshot> {
        self.goals
            .iter()
            .map(|(key, goal)| GoalSnapshot {
                key: *key,
                target_entity: goal.request.target_entity,
                target_feet: goal.request.target_feet,
                feet: goal.feet,
                yaw_degrees: goal.yaw,
                tolerance: goal.tolerance,
                state: goal.state.clone(),
                route_nodes: goal.route.nodes.clone(),
                route_feet: goal.route.feet.clone(),
                next_waypoint: goal.next,
                sequence: goal
                    .state
                    .is_moving()
                    .then(|| goal.request.style.sequence().to_owned()),
                animation_time: goal.animation_time,
                motor_eta: None,
                contact_normal: goal.contact_normal,
            })
            .collect()
    }
    pub fn request<W: NpcCollisionWorld>(
        &mut self,
        key: GoalKey,
        request: MoveRequest,
        pose: ActorPose,
        world: &W,
        transients: &[ActorHull],
    ) -> GoalState {
        let mut goal = Goal {
            request,
            tolerance: 0.1,
            state: GoalState::Moving,
            route: Route::default(),
            next: 0,
            feet: pose.feet,
            yaw: pose.yaw_degrees,
            animation_time: 0.,
            retry_time: 0.,
            contact_normal: Vec3::ZERO,
        };
        let prepared = (|| {
            if pose.entity != key.actor
                || !pose.feet.is_finite()
                || !pose.yaw_degrees.is_finite()
                || !goal.request.target_feet.is_finite()
                || !goal.request.event_distance.is_finite()
                || goal.request.event_distance < 0.
            {
                return Err(GoalBlockReason::InvalidInput);
            }
            if pose.scripted_by.is_some() {
                return Err(GoalBlockReason::ScriptOwned);
            }
            if key.scene != AI_SCENE && self.ai_only.contains(&key.actor) {
                return Err(GoalBlockReason::MissingActor);
            }
            if self.goals.iter().any(|(other, g)| {
                other.actor == key.actor
                    && *other != key
                    && !matches!(g.state, GoalState::Arrived | GoalState::Canceled)
            }) {
                return Err(GoalBlockReason::ActorBusy);
            }
            let actor = self
                .actors
                .get(&key.actor)
                .ok_or(GoalBlockReason::MissingActor)?;
            actor.locomotion.track(goal.request.style)?;
            goal.tolerance = if goal.request.force_short {
                0.1
            } else {
                (actor.ground.hull.maxs.x - actor.ground.hull.mins.x) * 0.5
            }
            .max(goal.request.event_distance)
            .max(0.1);
            let excluded = [key.actor];
            let query = Query {
                excluded_entities: &excluded,
                transients,
                ..Default::default()
            };
            if !npc_probe::fits(world, pose.feet, actor.ground.hull, query)
                || !npc_probe::stand(world, pose.feet, actor.ground, query)
            {
                return Err(GoalBlockReason::Collision {
                    blocker: None,
                    reason: "actor cannot stand".to_owned(),
                });
            }
            if pose.feet.distance(goal.request.target_feet) <= goal.tolerance {
                goal.state = GoalState::Arrived;
            } else {
                goal.route = plan(
                    world,
                    self.graph.as_ref(),
                    &self.restrictions,
                    &self.doors,
                    actor,
                    pose.feet,
                    Destination {
                        feet: goal.request.target_feet,
                        tolerance: goal.tolerance,
                    },
                    key.actor,
                )?;
            }
            Ok(())
        })();
        if let Err(reason) = prepared {
            goal.state = GoalState::Blocked(reason);
        }
        // A rejected competing scene never owns the actor's pose or animation.
        if goal.state != GoalState::Blocked(GoalBlockReason::ActorBusy) {
            self.pending.push(update(key, &goal, &self.doors));
        }
        self.goals.insert(key, goal);
        self.status(key).cloned().unwrap()
    }
    pub fn cancel_scene(&mut self, scene: usize) {
        for (key, goal) in &mut self.goals {
            if key.scene == scene && goal.state != GoalState::Canceled {
                let owned = goal.state != GoalState::Blocked(GoalBlockReason::ActorBusy);
                goal.state = GoalState::Canceled;
                if owned {
                    self.pending.push(update(*key, goal, &self.doors));
                }
            }
        }
    }
    pub fn tick<W: NpcCollisionWorld>(
        &mut self,
        world: &W,
        poses: &[ActorPose],
        transients: &[ActorHull],
        dt: f32,
        ui_paused: bool,
    ) -> Vec<MoveUpdate> {
        if ui_paused {
            return Vec::new();
        }
        let mut updates = std::mem::take(&mut self.pending);
        for (key, goal) in &mut self.goals {
            if matches!(goal.state, GoalState::Arrived | GoalState::Canceled) {
                continue;
            }
            if matches!(goal.state, GoalState::Blocked(GoalBlockReason::ActorBusy)) {
                continue;
            }
            if matches!(
                goal.state,
                GoalState::Blocked(GoalBlockReason::InvalidInput)
            ) {
                updates.push(update(*key, goal, &self.doors));
                continue;
            }
            let outcome = (|| {
                if !dt.is_finite() || dt < 0. || dt > 1. {
                    return Err(GoalBlockReason::InvalidInput);
                }
                let pose = poses
                    .iter()
                    .find(|pose| pose.entity == key.actor)
                    .ok_or(GoalBlockReason::MissingActor)?;
                if !pose.feet.is_finite() || !pose.yaw_degrees.is_finite() {
                    return Err(GoalBlockReason::InvalidInput);
                }
                goal.feet = pose.feet;
                goal.yaw = pose.yaw_degrees;
                if pose.scripted_by.is_some() {
                    return Err(GoalBlockReason::ScriptOwned);
                }
                let actor = self
                    .actors
                    .get(&key.actor)
                    .ok_or(GoalBlockReason::MissingActor)?;
                let track = actor.locomotion.track(goal.request.style)?;
                if matches!(goal.state, GoalState::Blocked(_)) {
                    goal.retry_time += dt;
                    if goal.retry_time < 1.5 {
                        return Ok(());
                    }
                    goal.retry_time = 0.;
                    goal.route = plan(
                        world,
                        self.graph.as_ref(),
                        &self.restrictions,
                        &self.doors,
                        actor,
                        goal.feet,
                        Destination {
                            feet: goal.request.target_feet,
                            tolerance: goal.tolerance,
                        },
                        key.actor,
                    )?;
                    goal.next = 0;
                    goal.state = GoalState::Moving;
                }
                advance(world, actor, *key, goal, track, transients, dt)
            })();
            if let Err(reason) = outcome {
                goal.state = GoalState::Blocked(reason);
            }
            updates.push(update(*key, goal, &self.doors));
        }
        updates
    }
}
fn update(key: GoalKey, goal: &Goal, doors: &BTreeSet<usize>) -> MoveUpdate {
    // Navigators open a route door ahead of contact: request it while walking the door
    // segment or within 96 units of its start.
    let ahead = |index: usize| goal.route.doors.get(index).copied().flatten();
    let lookahead = ahead(goal.next).or_else(|| {
        goal.route
            .feet
            .get(goal.next)
            .filter(|start| start.distance(goal.feet) <= 96.)
            .and_then(|_| ahead(goal.next + 1))
    });
    let door = match &goal.state {
        GoalState::Blocked(GoalBlockReason::Collision {
            blocker: Some(blocker),
            ..
        }) if doors.contains(blocker) => Some(*blocker),
        GoalState::Moving => lookahead,
        _ => None,
    };
    MoveUpdate {
        door,
        key,
        feet: goal.feet,
        yaw_degrees: goal.yaw,
        sequence: goal
            .state
            .is_moving()
            .then(|| goal.request.style.sequence().to_owned()),
        animation_time: goal.animation_time,
        state: goal.state.clone(),
        contact_normal: goal.contact_normal,
    }
}
fn advance<W: NpcCollisionWorld>(
    world: &W,
    actor: &Actor,
    key: GoalKey,
    goal: &mut Goal,
    track: &RootMotion,
    transients: &[ActorHull],
    dt: f32,
) -> Result<(), GoalBlockReason> {
    let excluded = [key.actor];
    let query = Query {
        excluded_entities: &excluded,
        transients,
        ..Default::default()
    };
    if !npc_probe::fits(world, goal.feet, actor.ground.hull, query)
        || !npc_probe::stand(world, goal.feet, actor.ground, query)
    {
        return Err(GoalBlockReason::Collision {
            blocker: None,
            reason: "actor cannot stand".to_owned(),
        });
    }
    // SDK: the hull tolerance only decides whether to move; once moving, the navigator walks
    // until within SetArrivalDistance(event distance), measured horizontally.
    let arrival = goal.request.event_distance.max(0.1);
    if (goal.feet - goal.request.target_feet).truncate().length() <= arrival {
        goal.state = GoalState::Arrived;
        return Ok(());
    }
    let duration = track
        .duration()
        .map_err(|e| GoalBlockReason::RootMotion(e.to_string()))?;
    let mut remaining = dt;
    // Budget is ours, not an engine constant; failing never releases SECTION.
    for _ in 0..32 {
        if remaining <= 0. {
            return Ok(());
        }
        let target = *goal
            .route
            .feet
            .get(goal.next)
            .ok_or(GoalBlockReason::NoRoute)?;
        let delta = (target - goal.feet) * Vec3::new(1., 1., 0.);
        let distance = delta.length();
        if distance <= 0.001 {
            if goal.next + 1 >= goal.route.feet.len() {
                return Err(GoalBlockReason::NoRoute);
            }
            goal.next += 1;
            continue;
        }
        let from = goal.animation_time / duration;
        let full = track
            .movement(from, (goal.animation_time + remaining) / duration)
            .map_err(|e| GoalBlockReason::RootMotion(e.to_string()))?
            .position
            .x
            * actor.scale;
        if !full.is_finite() || full <= 0. {
            return Err(GoalBlockReason::RootMotion(
                "nonpositive forward interval".to_owned(),
            ));
        }
        let (amount, consumed) = if full <= distance {
            (full, remaining)
        } else {
            // Preserve an accelerated authored curve when a waypoint splits a tick.
            let (mut low, mut high) = (0., remaining);
            for _ in 0..24 {
                let mid = (low + high) * 0.5;
                let moved = track
                    .movement(from, (goal.animation_time + mid) / duration)
                    .map_err(|e| GoalBlockReason::RootMotion(e.to_string()))?
                    .position
                    .x
                    * actor.scale;
                if moved < distance {
                    low = mid;
                } else {
                    high = mid;
                }
            }
            (distance, high)
        };
        let direction = delta / distance;
        let movement = npc_probe::ground_move(
            world,
            goal.feet,
            goal.feet + direction * amount,
            actor.ground,
            query,
        );
        let actual = (movement.end - goal.feet).truncate().length();
        goal.feet = movement.end;
        goal.contact_normal = movement.normal;
        if actual > 0.0001 {
            goal.yaw = direction.y.atan2(direction.x).to_degrees();
            // Partial travel can precede a blocked substep. Only consumed travel
            // advances the animation; the obstruction immediately restores idle.
            goal.animation_time += consumed * (actual / amount).clamp(0., 1.);
        }
        if !movement.completed {
            // Blocked right beside the goal still counts as reaching it within tolerance.
            if goal.feet.distance(goal.request.target_feet) <= goal.tolerance {
                goal.state = GoalState::Arrived;
                return Ok(());
            }
            return Err(GoalBlockReason::Collision {
                blocker: movement.blocker,
                reason: format!("{:?}", movement.reason),
            });
        }
        remaining = (remaining - consumed).max(0.);
        if (goal.feet - goal.request.target_feet).truncate().length() <= arrival {
            goal.state = GoalState::Arrived;
            return Ok(());
        }
        if amount >= distance - 0.001 {
            if goal.next + 1 >= goal.route.feet.len() {
                return Err(GoalBlockReason::NoRoute);
            }
            goal.next += 1;
        }
    }
    Err(GoalBlockReason::SearchBudget)
}

fn node_feet(graph: &Graph, node: usize, restrictions: &NavRestrictions) -> Option<Vec3> {
    let value = graph.nodes.get(node)?;
    if value.node_type != 2 || value.info != 0 || restrictions.unusable_nodes.contains(&node) {
        return None;
    }
    let feet = value.origin + Vec3::Z * value.hull_z_offsets[0];
    feet.is_finite().then_some(feet)
}
fn connector<W: NpcCollisionWorld>(
    world: &W,
    actor: &Actor,
    start: Vec3,
    end: Vec3,
    tolerance: f32,
    query: Query<'_>,
) -> bool {
    if !npc_probe::fits(world, end, actor.ground.hull, query)
        || !npc_probe::stand(world, end, actor.ground, query)
    {
        return false;
    }
    let moved = npc_probe::ground_move(world, start, end, actor.ground, query);
    moved.completed && moved.end.distance(end) <= tolerance
}
fn candidates<W: NpcCollisionWorld>(
    world: &W,
    graph: &Graph,
    restrictions: &NavRestrictions,
    actor: &Actor,
    point: Vec3,
    view: Vec3,
    query: Query<'_>,
) -> Vec<usize> {
    let mut nearest: Vec<_> = (0..graph.nodes.len())
        .filter_map(|index| {
            let feet = node_feet(graph, index, restrictions)?;
            ((graph.nodes[index].origin - point)
                .abs()
                .cmple(Vec3::splat(720.))
                .all())
            .then_some((index, feet.distance_squared(point)))
        })
        .collect();
    nearest.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    nearest
        .into_iter()
        .take(10)
        .filter_map(|(index, _)| {
            let feet = node_feet(graph, index, restrictions)?;
            let visible = world
                .npc_trace_hull(
                    feet + view,
                    point + Vec3::Z,
                    Hull {
                        mins: Vec3::ZERO,
                        maxs: Vec3::ZERO,
                    },
                    query,
                )
                .clear_path();
            (visible
                && npc_probe::fits(world, feet, actor.ground.hull, query)
                && npc_probe::stand(world, feet, actor.ground, query))
            .then_some(index)
        })
        .collect()
}
struct Destination {
    feet: Vec3,
    tolerance: f32,
}
/// A graph link whose only obstruction is a door: NPCs open doors along routes.
fn door_link<W: NpcCollisionWorld>(
    world: &W,
    actor: &Actor,
    from: Vec3,
    to: Vec3,
    query: Query<'_>,
    doors: &BTreeSet<usize>,
) -> bool {
    if doors.is_empty()
        || !npc_probe::fits(world, to, actor.ground.hull, query)
        || !npc_probe::stand(world, to, actor.ground, query)
    {
        return false;
    }
    let moved = npc_probe::ground_move(world, from, to, actor.ground, query);
    !moved.completed && moved.blocker.is_some_and(|b| doors.contains(&b))
}
#[allow(clippy::too_many_arguments)]
fn plan<W: NpcCollisionWorld>(
    world: &W,
    graph: Option<&Graph>,
    restrictions: &NavRestrictions,
    doors: &BTreeSet<usize>,
    actor: &Actor,
    start: Vec3,
    destination: Destination,
    entity: usize,
) -> Result<Route, GoalBlockReason> {
    let Destination {
        feet: end,
        tolerance,
    } = destination;
    let excluded = [entity];
    let query = Query {
        contents_mask: npc_probe::NPC_BRUSH_ONLY_MASK,
        excluded_entities: &excluded,
        ..Default::default()
    };
    if connector(world, actor, start, end, tolerance, query) {
        return Ok(Route {
            nodes: Vec::new(),
            feet: vec![end],
            doors: vec![None],
        });
    }
    let graph = graph.ok_or(GoalBlockReason::MissingGraph)?;
    if restrictions.dynamic_link_state_unknown {
        return Err(GoalBlockReason::UnsupportedGraphState);
    }
    let view = actor
        .view_offset
        .ok_or(GoalBlockReason::MissingViewOffset)?;
    let starts = candidates(world, graph, restrictions, actor, start, view, query);
    let ends = candidates(world, graph, restrictions, actor, end, view, query);
    let start_node = starts
        .into_iter()
        .take(4)
        .find(|n| {
            connector(
                world,
                actor,
                start,
                node_feet(graph, *n, restrictions).unwrap(),
                0.1,
                query,
            )
        })
        .ok_or(GoalBlockReason::NoRoute)?;
    let end_node = ends
        .into_iter()
        .take(4)
        .find(|n| {
            connector(
                world,
                actor,
                node_feet(graph, *n, restrictions).unwrap(),
                end,
                tolerance,
                query,
            )
        })
        .ok_or(GoalBlockReason::NoRoute)?;
    let nodes = search(graph, restrictions, start_node, end_node, |_, from, to| {
        if connector(world, actor, from, to, 0.1, query) {
            Some(from.distance(to))
        } else {
            // Prefer open routes; a door link costs extra for the open/wait.
            door_link(world, actor, from, to, query, doors).then(|| from.distance(to) + 128.)
        }
    })?;
    let mut feet: Vec<_> = nodes
        .iter()
        .map(|n| node_feet(graph, *n, restrictions).unwrap())
        .collect();
    feet.push(end);
    let door_set = doors;
    let mut doors = vec![None];
    for pair in feet.windows(2) {
        let moved = npc_probe::ground_move(world, pair[0], pair[1], actor.ground, query);
        doors.push(
            moved
                .blocker
                .filter(|b| !moved.completed && door_set.contains(b)),
        );
    }
    Ok(Route { nodes, feet, doors })
}
fn search(
    graph: &Graph,
    restrictions: &NavRestrictions,
    start: usize,
    end: usize,
    mut cost: impl FnMut(usize, Vec3, Vec3) -> Option<f32>,
) -> Result<Vec<usize>, GoalBlockReason> {
    let count = graph.nodes.len();
    if count == 0
        || count > 1500
        || graph.links.len() > 1_000_000
        || node_feet(graph, start, restrictions).is_none()
        || node_feet(graph, end, restrictions).is_none()
    {
        return Err(GoalBlockReason::InvalidGraph);
    }
    let positions: Vec<_> = (0..count)
        .map(|n| node_feet(graph, n, restrictions))
        .collect();
    let mut adjacent = vec![Vec::new(); count];
    for (index, link) in graph.links.iter().enumerate() {
        if link.source >= count || link.destination >= count {
            return Err(GoalBlockReason::InvalidGraph);
        }
        if link.accepted_move_types[0] & 1 != 0
            && !restrictions.unusable_links.contains(&index)
            && positions[link.source].is_some()
            && positions[link.destination].is_some()
        {
            adjacent[link.source].push((link.destination, index));
            adjacent[link.destination].push((link.source, index));
        }
    }
    let goal = positions[end].unwrap();
    let mut g = vec![f32::MAX; count];
    let mut f = vec![f32::MAX; count];
    let mut parent = vec![None; count];
    let mut seen = vec![false; count];
    let mut open = vec![false; count];
    g[start] = 0.;
    f[start] = positions[start].unwrap().distance(goal) * 0.1;
    seen[start] = true;
    open[start] = true;
    for _ in 0..count.saturating_mul(count) {
        let mut selected = None;
        let mut lowest = f32::MAX;
        for index in 0..count {
            if open[index] && f[index] < lowest {
                selected = Some(index);
                lowest = f[index];
            }
        }
        let current = selected.ok_or(GoalBlockReason::NoRoute)?;
        open[current] = false;
        if current == end {
            let mut path = vec![end];
            while *path.last().unwrap() != start {
                if path.len() > count {
                    return Err(GoalBlockReason::InvalidGraph);
                }
                path.push(parent[*path.last().unwrap()].ok_or(GoalBlockReason::InvalidGraph)?);
            }
            path.reverse();
            return Ok(path);
        }
        for &(neighbor, link) in &adjacent[current] {
            let Some(edge) = cost(
                link,
                positions[current].unwrap(),
                positions[neighbor].unwrap(),
            ) else {
                continue;
            };
            if !edge.is_finite() || edge < 0. {
                return Err(GoalBlockReason::InvalidGraph);
            }
            if edge == f32::MAX {
                continue;
            }
            let next = g[current] + edge;
            if !next.is_finite() {
                return Err(GoalBlockReason::InvalidGraph);
            }
            if !seen[neighbor] || next < g[neighbor] {
                parent[neighbor] = Some(current);
                g[neighbor] = next;
                f[neighbor] = next + positions[neighbor].unwrap().distance(goal);
                if !f[neighbor].is_finite() {
                    return Err(GoalBlockReason::InvalidGraph);
                }
                seen[neighbor] = true;
                open[neighbor] = true;
            }
        }
    }
    Err(GoalBlockReason::SearchBudget)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::Physics;
    use modkit_core::{animation::MovementRecord, Brush, Entity, Plane};
    use source_assets::navigation::{Link, Node};

    fn cube(min: Vec3, max: Vec3) -> Brush {
        Brush {
            contents: 1,
            planes: vec![
                Plane {
                    normal: Vec3::X,
                    distance: max.x,
                },
                Plane {
                    normal: -Vec3::X,
                    distance: -min.x,
                },
                Plane {
                    normal: Vec3::Y,
                    distance: max.y,
                },
                Plane {
                    normal: -Vec3::Y,
                    distance: -min.y,
                },
                Plane {
                    normal: Vec3::Z,
                    distance: max.z,
                },
                Plane {
                    normal: -Vec3::Z,
                    distance: -min.z,
                },
            ],
        }
    }
    fn world(obstacles: Vec<Brush>) -> World {
        let mut brushes = vec![cube(
            Vec3::new(-1000., -1000., -50.),
            Vec3::new(1000., 1000., 0.),
        )];
        brushes.extend(obstacles);
        World {
            brushes,
            ..Default::default()
        }
    }
    fn track(v0: f32, v1: f32) -> RootMotion {
        RootMotion {
            fps: 10.,
            frame_count: 11,
            records: vec![MovementRecord {
                end_frame: 10,
                motion_flags: 1,
                v0,
                v1,
                end_yaw_degrees: 0.,
                direction: Vec3::X,
                cumulative_position: Vec3::X * (v0 + v1) * 0.5,
            }],
        }
    }
    fn locomotion() -> Locomotion {
        Locomotion {
            tracks: BTreeMap::from([
                (MoveStyle::Walk, track(80., 80.)),
                (MoveStyle::Run, track(200., 200.)),
            ]),
            ..Default::default()
        }
    }
    fn key(scene: usize) -> GoalKey {
        GoalKey {
            scene,
            event: 0,
            actor: 7,
        }
    }
    fn pose(feet: Vec3) -> ActorPose {
        ActorPose {
            entity: 7,
            feet,
            yaw_degrees: 0.,
            scripted_by: None,
        }
    }
    fn request(target: Vec3) -> MoveRequest {
        MoveRequest {
            target_entity: 9,
            target_feet: target,
            style: MoveStyle::Walk,
            event_distance: 0.,
            force_short: false,
        }
    }
    fn make_controller(graph: Option<Graph>) -> Controller {
        let mut controller = Controller::new(graph, NavRestrictions::default());
        controller
            .register_actor(7, 1., normal_human(1., 18., 1.).unwrap(), locomotion())
            .unwrap();
        controller.set_view_offset(7, Vec3::Z * 70.).unwrap();
        controller
    }
    fn graph(points: &[Vec3], edges: &[(usize, usize)]) -> Graph {
        Graph {
            version: 37,
            map_revision: 1,
            nodes: points
                .iter()
                .map(|point| Node {
                    origin: *point,
                    yaw: 0.,
                    hull_z_offsets: [0.; 10],
                    node_type: 2,
                    info: 0,
                    zone: 0,
                })
                .collect(),
            links: edges
                .iter()
                .map(|&(source, destination)| Link {
                    source,
                    destination,
                    accepted_move_types: [1; 10],
                })
                .collect(),
            hammer_ids: (0..points.len() as i32).collect(),
            trailing_bytes: vec![],
        }
    }
    fn tick(
        controller: &mut Controller,
        physics: &Physics,
        feet: &mut Vec3,
        dt: f32,
        paused: bool,
    ) -> Vec<MoveUpdate> {
        let updates = controller.tick(physics, &[pose(*feet)], &[], dt, paused);
        if let Some(update) = updates.last() {
            *feet = update.feet;
        }
        updates
    }
    #[test]
    fn arrival_preserves_supported_floor_and_never_snaps_authored_target_z() {
        let physics = Physics::new(&world(vec![]));
        let mut controller = make_controller(None);
        let mut feet = Vec3::ZERO;
        assert_eq!(
            controller.request(
                key(1),
                request(Vec3::new(100., 0., 8.)),
                pose(feet),
                &physics,
                &[]
            ),
            GoalState::Moving
        );
        for _ in 0..100 {
            tick(&mut controller, &physics, &mut feet, 0.02, false);
        }
        assert!(controller.status(key(1)).unwrap().is_arrived());
        assert!(feet.x >= 87. && feet.x <= 100.);
        assert!(
            feet.z < 0.2,
            "arrival must keep the supported floor: {feet:?}"
        );
        let snapshot = &controller.snapshots()[0];
        assert_eq!(snapshot.tolerance, 13.);
        assert!(snapshot.sequence.is_none() && snapshot.motor_eta.is_none());
        let mut impossible = make_controller(None);
        assert!(matches!(
            impossible.request(
                key(1),
                request(Vec3::new(100., 0., 50.)),
                pose(Vec3::ZERO),
                &physics,
                &[]
            ),
            GoalState::Blocked(_)
        ));
        assert!(!impossible.status(key(1)).unwrap().is_arrived());
    }
    #[test]
    fn ui_pause_freezes_pose_and_cycle_cancel_stops_without_arrival() {
        let physics = Physics::new(&world(vec![]));
        let mut controller = make_controller(None);
        let mut feet = Vec3::ZERO;
        controller.request(key(1), request(Vec3::X * 400.), pose(feet), &physics, &[]);
        tick(&mut controller, &physics, &mut feet, 0.25, false);
        let before = controller.snapshots()[0].clone();
        for _ in 0..10 {
            assert!(tick(&mut controller, &physics, &mut feet, 0.25, true).is_empty());
        }
        let after = &controller.snapshots()[0];
        assert_eq!(after.feet, before.feet);
        assert_eq!(after.animation_time, before.animation_time);
        controller.cancel_scene(1);
        let canceled = tick(&mut controller, &physics, &mut feet, 0.25, false);
        assert_eq!(canceled.last().unwrap().state, GoalState::Canceled);
        assert!(canceled.last().unwrap().sequence.is_none());
        assert_eq!(feet, before.feet);
        assert!(!controller.status(key(1)).unwrap().is_arrived());
    }
    #[test]
    fn player_obstruction_holds_idle_then_retries_actual_ground_travel() {
        let physics = Physics::new(&world(vec![]));
        let mut controller = make_controller(None);
        let mut feet = Vec3::ZERO;
        controller.request(key(1), request(Vec3::X * 200.), pose(feet), &physics, &[]);
        let blocker = [ActorHull {
            entity: None,
            feet: Vec3::X * 50.,
            hull: normal_human(1., 18., 1.).unwrap().hull,
        }];
        let updates = controller.tick(&physics, &[pose(feet)], &blocker, 0.5, false);
        let last = updates.last().unwrap();
        feet = last.feet;
        assert!(matches!(
            last.state,
            GoalState::Blocked(GoalBlockReason::Collision { .. })
        ));
        assert!(last.sequence.is_none());
        assert!(last.contact_normal.length() > 0.9);
        let blocked = controller.snapshots()[0].clone();
        tick(&mut controller, &physics, &mut feet, 0.5, false);
        assert_eq!(feet, blocked.feet);
        assert_eq!(
            controller.snapshots()[0].animation_time,
            blocked.animation_time
        );
        for _ in 0..40 {
            tick(&mut controller, &physics, &mut feet, 0.1, false);
        }
        assert!(controller.status(key(1)).unwrap().is_arrived());
        assert!(feet.x >= 187.);
    }
    #[test]
    fn scene_ownership_and_script_animation_cannot_be_stolen_by_retry() {
        let physics = Physics::new(&world(vec![]));
        let mut controller = make_controller(None);
        let mut feet = Vec3::ZERO;
        controller.request(key(1), request(Vec3::X * 400.), pose(feet), &physics, &[]);
        assert_eq!(
            controller.request(key(2), request(Vec3::Y * 400.), pose(feet), &physics, &[]),
            GoalState::Blocked(GoalBlockReason::ActorBusy)
        );
        for _ in 0..30 {
            let updates = tick(&mut controller, &physics, &mut feet, 0.1, false);
            assert!(updates.iter().all(|u| u.key != key(2)));
        }
        assert!(feet.x > 200., "a competing scene must not reset actor feet");
        controller.cancel_scene(2);
        assert!(tick(&mut controller, &physics, &mut feet, 0.1, false)
            .iter()
            .all(|u| u.key != key(2)));
        assert_eq!(controller.status(key(2)), Some(&GoalState::Canceled));
        let scripted = ActorPose {
            scripted_by: Some(3),
            ..pose(feet)
        };
        let updates = controller.tick(&physics, &[scripted], &[], 0.1, false);
        assert!(updates.iter().any(|u| u.key == key(1)
            && u.state == GoalState::Blocked(GoalBlockReason::ScriptOwned)
            && u.sequence.is_none()));
    }
    #[test]
    fn authored_acceleration_cycles_and_waypoint_splitting_preserve_distance() {
        let physics = Physics::new(&world(vec![]));
        let mut controller = make_controller(None);
        controller
            .actors
            .get_mut(&7)
            .unwrap()
            .locomotion
            .tracks
            .insert(MoveStyle::Walk, track(40., 120.));
        let mut feet = Vec3::ZERO;
        controller.request(key(1), request(Vec3::X * 900.), pose(feet), &physics, &[]);
        controller.goals.get_mut(&key(1)).unwrap().route.feet = vec![Vec3::X * 30., Vec3::X * 900.];
        tick(&mut controller, &physics, &mut feet, 0.5, false);
        assert!((feet.x - 30.).abs() < 0.001);
        tick(&mut controller, &physics, &mut feet, 0.5, false);
        assert!((feet.x - 80.).abs() < 0.001);
        tick(&mut controller, &physics, &mut feet, 0.5, false);
        assert!((feet.x - 110.).abs() < 0.001);
        assert!((controller.snapshots()[0].animation_time - 1.5).abs() < 0.0001);
    }
    #[test]
    fn graph_routes_around_wall_with_validated_connectors_and_excludes_air_hints() {
        let physical_world = world(vec![cube(
            Vec3::new(35., -45., 0.),
            Vec3::new(65., 45., 120.),
        )]);
        let physics = Physics::new(&physical_world);
        let points = [
            Vec3::new(0., 0., 0.),
            Vec3::new(0., 80., 0.),
            Vec3::new(100., 80., 0.),
            Vec3::new(100., 0., 0.),
        ];
        let mut controller = make_controller(Some(graph(&points, &[(0, 1), (1, 2), (2, 3)])));
        let mut feet = Vec3::ZERO;
        let mut wanted = request(points[3]);
        wanted.force_short = true;
        assert_eq!(
            controller.request(key(1), wanted, pose(feet), &physics, &[]),
            GoalState::Moving
        );
        let nodes = controller.snapshots()[0].route_nodes.clone();
        assert_eq!(nodes, vec![0, 1, 2, 3]);
        let mut highest_y = 0f32;
        for _ in 0..300 {
            tick(&mut controller, &physics, &mut feet, 0.02, false);
            highest_y = highest_y.max(feet.y);
            let hull = controller.actor_hull(7).unwrap();
            assert!(
                !physics
                    .npc_trace_hull(feet, feet, hull, Query::default())
                    .trace
                    .start_solid
            );
        }
        assert!(highest_y >= 79.);
        assert!(controller.status(key(1)).unwrap().is_arrived());
        assert!(feet.distance(points[3]) <= 0.1);
        let mut invalid = graph(&points, &[(0, 1), (1, 2), (2, 3)]);
        invalid.nodes[1].node_type = 3;
        invalid.nodes[1].hull_z_offsets[0] = -500.;
        assert!(node_feet(&invalid, 1, &NavRestrictions::default()).is_none());
        assert!(
            search(&invalid, &NavRestrictions::default(), 0, 3, |_, a, b| Some(
                a.distance(b)
            ))
            .is_err()
        );
        let map = World {
            entities: vec![Entity {
                properties: vec![
                    ("classname".into(), "info_node_hint".into()),
                    ("nodeid".into(), "1".into()),
                ],
            }],
            ..Default::default()
        };
        assert!(NavRestrictions::from_world(&map, &invalid)
            .unusable_nodes
            .contains(&1));
    }
    #[test]
    fn astar_equal_cost_uses_lower_id_and_reopens_previously_selected_nodes() {
        let diamond = graph(
            &[
                Vec3::ZERO,
                Vec3::new(1., 1., 0.),
                Vec3::new(1., -1., 0.),
                Vec3::X * 2.,
            ],
            &[(0, 2), (2, 3), (0, 1), (1, 3)],
        );
        assert_eq!(
            search(&diamond, &NavRestrictions::default(), 0, 3, |_, a, b| Some(
                a.distance(b)
            ))
            .unwrap(),
            vec![0, 1, 3]
        );
        let reopen = graph(
            &[
                Vec3::ZERO,
                Vec3::X * 100.,
                Vec3::Y * 100.,
                Vec3::X * 200.,
                Vec3::X * 300.,
            ],
            &[(0, 1), (0, 2), (2, 1), (1, 3), (3, 4)],
        );
        // Virtual costs expose the seen/open distinction. Production currently
        // supports base Euclidean costs, not NPC virtual MovementCost overrides.
        let costs = [10., 1., 1., 1., 1000.];
        assert_eq!(
            search(&reopen, &NavRestrictions::default(), 0, 4, |link, _, _| {
                Some(costs[link])
            })
            .unwrap(),
            vec![0, 2, 1, 3, 4]
        );
        assert_eq!(
            search(&reopen, &NavRestrictions::default(), 0, 4, |_, _, _| Some(
                f32::NAN
            )),
            Err(GoalBlockReason::InvalidGraph)
        );
    }
    #[test]
    fn missing_gait_and_nonforward_motion_are_explicit_without_guessed_speed() {
        let missing = Locomotion::from_sequences(&BTreeMap::new());
        assert!(matches!(
            missing.track(MoveStyle::Walk),
            Err(GoalBlockReason::MissingLocomotion(_))
        ));
        let mut sideways = track(80., 80.);
        sideways.records[0].direction = Vec3::Y;
        assert!(matches!(
            validate_forward_track(&sideways),
            Err(GoalBlockReason::UnsupportedLocomotion(_))
        ));
        assert!(MoveStyle::parse("crouch").is_err());
        assert_eq!(MoveStyle::parse("RUN").unwrap(), MoveStyle::Run);
        assert_eq!(
            normal_human(1., 18., 1.).unwrap().hull.maxs,
            Vec3::new(13., 13., 72.)
        );
    }
    #[test]
    #[ignore = "requires an installed owned HL2 copy; no native movement trajectory comparison"]
    fn owned_barney_locomotion_preloads_real_forward_tracks() {
        let game = source_assets::install::discover().unwrap();
        let vfs = Vfs::mount(&game).unwrap();
        let locomotion = Locomotion::load(&vfs, "models/barney.mdl").unwrap();
        assert_eq!(
            locomotion.required_clips(),
            BTreeSet::from(["walk_all".into(), "run_all".into()])
        );
        for (style, distance, duration) in [
            (MoveStyle::Walk, 80.00001, 1.),
            (MoveStyle::Run, 125.87412, 0.6),
        ] {
            let track = locomotion.track(style).unwrap();
            assert!((track.movement(0., 1.).unwrap().position.x - distance).abs() < 0.001);
            assert!((track.duration().unwrap() - duration).abs() < 0.00001);
            println!("{style:?}: central owned root distance={distance}, duration={duration}");
        }
    }
}
