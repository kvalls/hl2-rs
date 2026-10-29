//! Conservative human-ground MOVETO controller in Source units/Z-up.
//! Authored central-forward root motion drives travel. This is not the full AI
//! motor: pose weighting, acceleration/turn planning, ETA, triangulation, door
//! opening, give-way and dynamic/hint link semantics remain unsupported.
use crate::ai_hull::HullType;
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
    /// CAI_DynamicLink: graph links each info_node_link / info_node_link_controller
    /// entity controls.
    pub dynamic: BTreeMap<usize, Vec<usize>>,
    /// Links whose dynamic link is LINK_OFF (bits_LINK_OFF).
    pub off_links: BTreeSet<usize>,
    /// info_node_link entities whose nodes or link are not in the graph (the SDK creates
    /// a missing link when hull bits are set; not modeled).
    pub missing_dynamic: usize,
}
/// MAX_NODE_LINK_DIST / MAX_AIR_NODE_LINK_DIST (ai_node.h).
const MAX_NODE_LINK_DIST: f32 = 60. * 12.;
const MAX_AIR_NODE_LINK_DIST: f32 = 120. * 12.;
impl NavRestrictions {
    /// Exclude hinted nodes rather than pretending to implement hint locks. Dynamic
    /// links (CAI_DynamicLink::InitDynamicLinks) take their initial state: an
    /// info_node_link controls the link between its StartNode/EndNode editor ids; an
    /// info_node_link_controller (GenerateLinksFromVolume) every link whose nodes lie
    /// within MAX_NODE_LINK_DIST (air: MAX_AIR_NODE_LINK_DIST) of its center and whose
    /// segment crosses its box grown by one unit.
    pub fn from_world(world: &World, graph: &Graph) -> Self {
        let mut result = Self::default();
        let node_of = |id: i32| graph.hammer_ids.iter().position(|h| *h == id);
        let link_of = |a: usize, b: usize| {
            graph.links.iter().position(|l| {
                (l.source == a && l.destination == b) || (l.source == b && l.destination == a)
            })
        };
        for (entity_id, entity) in world.entities.iter().enumerate() {
            let off = entity.get("initialstate").is_some_and(|v| v.trim() == "0");
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
                "info_node_link" => {
                    let id = |k: &str| entity.get(k).and_then(|v| v.trim().parse::<i32>().ok());
                    let link = id("StartNode")
                        .and_then(node_of)
                        .zip(id("EndNode").and_then(node_of))
                        .and_then(|(a, b)| link_of(a, b));
                    match link {
                        Some(link) => {
                            result.dynamic.insert(entity_id, vec![link]);
                            if off {
                                result.off_links.insert(link);
                            }
                        }
                        None => result.missing_dynamic += 1,
                    }
                }
                "info_node_link_controller" => {
                    let origin = entity.origin();
                    let vec = |k: &str| {
                        modkit_core::parse_vec3(entity.get(k).unwrap_or("0 0 0"))
                            .unwrap_or(Vec3::ZERO)
                    };
                    let (mins, maxs) = (origin + vec("mins"), origin + vec("maxs"));
                    let center = (mins + maxs) * 0.5;
                    let air = entity
                        .get("useairlinkradius")
                        .is_some_and(|v| v.trim() == "1");
                    let reach = if air {
                        MAX_AIR_NODE_LINK_DIST
                    } else {
                        MAX_NODE_LINK_DIST
                    } + 0.1;
                    let (lo, hi) = (mins - Vec3::ONE, maxs + Vec3::ONE);
                    let links: Vec<usize> = graph
                        .links
                        .iter()
                        .enumerate()
                        .filter(|(_, l)| {
                            let (Some(a), Some(b)) =
                                (graph.nodes.get(l.source), graph.nodes.get(l.destination))
                            else {
                                return false;
                            };
                            a.origin.distance(center) < reach
                                && b.origin.distance(center) < reach
                                && segment_crosses_box(a.origin, b.origin, lo, hi)
                        })
                        .map(|(i, _)| i)
                        .collect();
                    if off {
                        result.off_links.extend(links.iter().copied());
                    }
                    result.dynamic.insert(entity_id, links);
                }
                _ => {}
            }
        }
        result
    }
    /// TurnOn / TurnOff of a dynamic link entity; false when it controls nothing.
    pub fn set_dynamic(&mut self, entity: usize, on: bool) -> bool {
        let Some(links) = self.dynamic.get(&entity) else {
            return false;
        };
        for link in links {
            if on {
                self.off_links.remove(link);
            } else {
                self.off_links.insert(*link);
            }
        }
        true
    }
}
/// ROUTE_SIMPLIFY_TIME_DELAY (in the PVS, AIStrongOpt off).
const ROUTE_SIMPLIFY_DELAY: f32 = 0.5;
/// MIN_ANGLE_COS_SIMPLIFY: 40 degrees left or right of the current heading.
const MIN_ANGLE_COS_SIMPLIFY: f32 = 0.766;
/// CAI_Navigator::SimplifyPathForward with the full scan parameters (32 ft out, points
/// within 12 ft, 3 ft increments, four samples): head straight for the farthest
/// reachable point along the path ahead, else skip to the next waypoint when it is
/// reachable and nearer than the scan distance. Points must lie within 40 degrees of the
/// current heading unless `forced` (the first simplification of a path).
fn simplify_path_forward<W: NpcCollisionWorld>(
    world: &W,
    actor: &Actor,
    goal: &mut Goal,
    query: Query<'_>,
    forced: bool,
) {
    const SCAN_DIST: f32 = 32. * 12.;
    const RADIUS: f32 = 12. * 12.;
    const INCREMENT: f32 = 3. * 12.;
    const MAX_SAMPLES: usize = 4;
    let feet = goal.feet;
    let points = &goal.route.feet;
    let next = goal.next;
    // The goal waypoint is not reducible.
    if next + 1 >= points.len() {
        return;
    }
    let flat = |v: Vec3| v * Vec3::new(1., 1., 0.);
    let heading = flat(points[next] - feet).normalize_or_zero();
    let attempt = |p: Vec3| {
        forced || flat(p - feet).normalize_or_zero().dot(heading) > MIN_ANGLE_COS_SIMPLIFY
    };
    let reachable = |p: Vec3| {
        let moved = npc_probe::ground_move(world, feet, p, actor.ground, query);
        moved.completed && flat(moved.end - p).length() < 1.
    };
    // Walk out along the path: (point, waypoint index of the segment start, skip test).
    let mut samples = Vec::new();
    let mut remaining = SCAN_DIST - flat(points[next] - feet).length();
    let (mut at, mut wp) = (points[next], next);
    while remaining > 0. && wp + 1 < points.len() {
        let to_next = flat(points[wp + 1] - at).length();
        if to_next < INCREMENT * 1.1 {
            at = points[wp + 1];
            remaining -= to_next;
            wp += 1;
        } else {
            at += (points[wp + 1] - points[wp]).normalize_or_zero() * INCREMENT;
            remaining -= INCREMENT;
        }
        let skip = flat(at - feet).length() > RADIUS + 0.1;
        samples.push((at, wp, skip));
    }
    let mut tests = 0;
    for &(point, segment, skip) in samples.iter().rev() {
        if skip || tests >= MAX_SAMPLES || !attempt(point) {
            continue;
        }
        tests += 1;
        if reachable(point) {
            // SimplifyPathInsertSimplification: the point replaces the waypoints up to
            // its segment start.
            goal.route.feet[segment] = point;
            if let Some(door) = goal.route.doors.get_mut(segment) {
                *door = None;
            }
            goal.next = segment;
            return;
        }
    }
    let following = points[next + 1];
    if attempt(following) && flat(following - feet).length() < SCAN_DIST && reachable(following) {
        goal.next = next + 1;
        if let Some(door) = goal.route.doors.get_mut(next + 1) {
            *door = None;
        }
    }
}
/// IsBoxIntersectingRay for the segment a -> b.
fn segment_crosses_box(a: Vec3, b: Vec3, lo: Vec3, hi: Vec3) -> bool {
    let d = b - a;
    let (mut t0, mut t1) = (0f32, 1f32);
    for axis in 0..3 {
        if d[axis].abs() < 1e-6 {
            if a[axis] < lo[axis] || a[axis] > hi[axis] {
                return false;
            }
            continue;
        }
        let (mut x, mut y) = (
            (lo[axis] - a[axis]) / d[axis],
            (hi[axis] - a[axis]) / d[axis],
        );
        if x > y {
            std::mem::swap(&mut x, &mut y);
        }
        t0 = t0.max(x);
        t1 = t1.min(y);
        if t0 > t1 {
            return false;
        }
    }
    true
}

#[derive(Clone, Debug, Default)]
pub struct Locomotion {
    tracks: BTreeMap<MoveStyle, RootMotion>,
    failures: BTreeMap<MoveStyle, GoalBlockReason>,
    /// The sequence of each style when it is not the default walk_all/run_all (a
    /// weapon-translated ACT_WALK_RIFLE/ACT_RUN_RIFLE sequence, say).
    names: BTreeMap<MoveStyle, String>,
    /// The aiming walk/run (Weapon_TranslateActivity of ACT_WALK_AIM/ACT_RUN_AIM) that
    /// move and shoot plays over the same root-motion track.
    aim_names: BTreeMap<MoveStyle, String>,
}
impl Locomotion {
    /// Load once at map/actor setup. Simulation ticks never access the VFS.
    pub fn load(vfs: &Vfs, model: &str) -> anyhow::Result<Self> {
        let wanted = ["walk_all".to_owned(), "run_all".to_owned()].into();
        let sequences = source_assets::models::read_root_motion(vfs, model, &wanted)?;
        Ok(Self::from_sequences(&sequences))
    }
    pub fn from_sequences(sequences: &BTreeMap<String, MotionSequence>) -> Self {
        Self::from_named(
            sequences,
            [
                (
                    MoveStyle::Walk,
                    MoveStyle::Walk.sequence(),
                    MoveStyle::Walk.activity(),
                ),
                (
                    MoveStyle::Run,
                    MoveStyle::Run.sequence(),
                    MoveStyle::Run.activity(),
                ),
            ],
        )
    }
    /// Tracks from named looping sequences with their expected activities (the
    /// sequence SelectWeightedSequence picks for the translated ACT_WALK/ACT_RUN).
    pub fn from_named(
        sequences: &BTreeMap<String, MotionSequence>,
        styles: [(MoveStyle, &str, &str); 2],
    ) -> Self {
        let mut result = Self::default();
        for (style, name, activity) in styles {
            let key = name.to_lowercase();
            if key != style.sequence() {
                result.names.insert(style, name.to_owned());
            }
            let checked = sequences
                .get(&key)
                .ok_or_else(|| GoalBlockReason::MissingLocomotion(name.to_owned()))
                .and_then(|sequence| {
                    if !sequence.activity_name.eq_ignore_ascii_case(activity)
                        || sequence.flags & 1 == 0
                    {
                        return Err(GoalBlockReason::UnsupportedLocomotion(name.to_owned()));
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
            .map(|style| self.sequence_name(*style))
            .collect()
    }
    /// The sequence a style plays.
    /// Set the aiming sequences (both styles, or none).
    pub fn set_aim(&mut self, walk: String, run: String) {
        self.aim_names = [(MoveStyle::Walk, walk), (MoveStyle::Run, run)].into();
    }
    pub fn has_aim(&self) -> bool {
        self.aim_names.len() == 2
    }
    fn played_name(&self, style: MoveStyle, aiming: bool) -> String {
        match self.aim_names.get(&style).filter(|_| aiming) {
            Some(name) => name.clone(),
            None => self.sequence_name(style),
        }
    }
    pub fn sequence_name(&self, style: MoveStyle) -> String {
        self.names
            .get(&style)
            .cloned()
            .unwrap_or_else(|| style.sequence().to_owned())
    }
    /// Both styles have tracks.
    pub fn complete(&self) -> bool {
        self.failures.is_empty() && self.tracks.len() == 2
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
        // The classic zombie's walks drift sideways by ~1e-4 of their direction (0.01
        // units a cycle) and the torso's crawl by 0.2 degrees (0.13 of 37.4 units); the
        // motor moves along the path at the ground speed, so such drift is ignored and
        // the track moves along +X.
        let drift = record.cumulative_position - Vec3::X * (previous + amount);
        if record.direction.distance(Vec3::X) > 0.01
            || record.end_yaw_degrees.abs() > 0.0001
            || record.v0 <= 0.
            || record.v1 <= 0.
            || drift.x.abs() > 0.001
            || drift.truncate().length() > 0.05f32.max(0.005 * amount)
            || drift.z.abs() > 0.001
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
/// `normal_human` for another table hull (a class's SetHullType).
pub fn ground_for_hull(
    hull: HullType,
    scale: f32,
    step_height: f32,
    step_down_multiplier: f32,
) -> Result<GroundConfig, GoalBlockReason> {
    Ok(GroundConfig {
        hull: hull.scaled(scale),
        ..normal_human(scale, step_height, step_down_multiplier)?
    })
}

#[derive(Clone)]
struct Actor {
    scale: f32,
    ground: GroundConfig,
    /// The table hull of `ground.hull` (node offsets and link move types).
    hull: HullType,
    locomotion: Locomotion,
    /// Locomotion per active weapon class ("" unarmed), preloaded (`set_locomotion`).
    variants: BTreeMap<String, Locomotion>,
    view_offset: Option<Vec3>,
    /// Move and shoot: play the aiming walk/run.
    aiming: bool,
    /// bits_CAP_DOORS_GROUP: route doors are opened for this actor (zombies have none).
    opens_doors: bool,
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
    /// CAI_Navigator m_flNextSimplifyTime as seconds left (AI goals).
    simplify_in: f32,
    /// The path was simplified once (bFirstForPath forces the first attempt).
    simplified: bool,
}
#[derive(Clone, Debug, Default)]
struct Route {
    nodes: Vec<usize>,
    feet: Vec<Vec3>,
    /// Door blocking the segment that ends at `feet[i]`, if any.
    doors: Vec<Option<usize>>,
    /// `doors[i]` was probed (segments are probed when the actor reaches them; missing
    /// entries count as probed).
    door_known: Vec<bool>,
}
pub struct Controller {
    graph: Option<Graph>,
    restrictions: NavRestrictions,
    actors: BTreeMap<usize, Actor>,
    goals: BTreeMap<GoalKey, Goal>,
    pending: Vec<MoveUpdate>,
    /// Door entities NPC routes may pass through by opening them.
    doors: BTreeSet<usize>,
    stats: PathStats,
    /// Movement time (unpaused ticks), the clock of the stale-link expiries.
    clock: f64,
    /// Graph links an NPC failed to move along (bits_LINK_STALE_SUGGESTED) and when
    /// they may be rechecked (m_timeStaleExpires).
    stale: BTreeMap<usize, f64>,
}
/// CAI_Navigator::MarkCurWaypointFailedLink: a failed link stays unusable this long.
const STALE_LINK_TIME: f64 = 4.;
/// IsLinkStillStale: a recheck that still fails keeps the link stale this long.
const STALE_RECHECK_TIME: f64 = 1.;
/// Route planning cost counters (reported per sample; performance diagnostics).
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct PathStats {
    pub plans: u64,
    pub failures: u64,
    pub link_probes: u64,
    pub micros: u64,
}
impl Controller {
    pub fn path_stats(&self) -> PathStats {
        self.stats
    }
    /// The loaded node graph, when the map has one.
    pub fn graph(&self) -> Option<&Graph> {
        self.graph.as_ref()
    }
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
    /// info_node_link / info_node_link_controller TurnOn / TurnOff.
    pub fn set_dynamic_link(&mut self, entity: usize, on: bool) -> bool {
        self.restrictions.set_dynamic(entity, on)
    }
    pub fn new(graph: Option<Graph>, restrictions: NavRestrictions) -> Self {
        Self {
            graph,
            restrictions,
            actors: BTreeMap::new(),
            goals: BTreeMap::new(),
            pending: Vec::new(),
            doors: BTreeSet::new(),
            stats: PathStats::default(),
            clock: 0.,
            stale: BTreeMap::new(),
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
        // Scenes and scripted_sequences move AI actors too (the AI yields while they
        // own the actor); an active goal of the other kind answers ActorBusy.
        self.register_actor(entity, model_scale, ground, locomotion)
    }
    /// The locomotion variants of an AI actor by weapon class (Weapon_TranslateActivity
    /// of ACT_WALK/ACT_RUN on its model), preloaded at map load.
    pub fn set_locomotion_variants(
        &mut self,
        entity: usize,
        variants: BTreeMap<String, Locomotion>,
    ) {
        if let Some(actor) = self.actors.get_mut(&entity) {
            actor.variants = variants;
        }
    }
    /// Weapon_Equip: move with the new weapon's walk/run sequences (unchanged when the
    /// model has no variant for it). Moves already running keep their track.
    /// CAI_MoveAndShootOverlay::UpdateMoveShootActivity: the aiming walk/run on or off.
    pub fn set_move_aim(&mut self, entity: usize, aiming: bool) {
        if let Some(actor) = self.actors.get_mut(&entity) {
            actor.aiming = aiming && actor.locomotion.has_aim();
        }
    }
    /// The model has both aiming locomotions for the current weapon.
    pub fn has_move_aim(&self, entity: usize) -> bool {
        self.actors
            .get(&entity)
            .is_some_and(|a| a.locomotion.has_aim())
    }
    /// A runtime spawn of a template's model and class (DESIGN 14r part A): the same
    /// hull, locomotion variants and eye offset, with no goal. False without a
    /// registered template.
    pub fn copy_actor(&mut self, template: usize, entity: usize) -> bool {
        let Some(mut actor) = self.actors.get(&template).cloned() else {
            return false;
        };
        actor.aiming = false;
        self.actors.insert(entity, actor);
        true
    }
    pub fn set_locomotion(&mut self, entity: usize, weapon: &str) -> bool {
        let Some(actor) = self.actors.get_mut(&entity) else {
            return false;
        };
        match actor.variants.get(weapon) {
            Some(locomotion) => {
                actor.locomotion = locomotion.clone();
                true
            }
            None => false,
        }
    }
    /// Usable node feet and the usable ground links of each node (the graph search's
    /// filters); None without a graph or beyond the native node limit.
    fn usable_graph(&self) -> Option<UsableGraph> {
        let hull = HullType::Human;
        let graph = self.graph.as_ref()?;
        let restrictions = &self.restrictions;
        let count = graph.nodes.len();
        if count == 0 || count > 1500 {
            return None;
        }
        let positions: Vec<_> = (0..count)
            .map(|n| node_feet(graph, n, restrictions, hull))
            .collect();
        let mut adjacent = vec![Vec::new(); count];
        for (index, link) in graph.links.iter().enumerate() {
            if link.source < count
                && link.destination < count
                && ground_link(link, hull)
                && !restrictions.unusable_links.contains(&index)
                && !restrictions.off_links.contains(&index)
                && positions[link.source].is_some()
                && positions[link.destination].is_some()
            {
                adjacent[link.source].push(link.destination);
                adjacent[link.destination].push(link.source);
            }
        }
        Some((positions, adjacent))
    }
    /// CAI_TacticalServices::FindCoverNode: from the usable node nearest `near` (the
    /// NPC), visit nodes nearest-first by straight distance from `near` and return the
    /// feet of the first whose distance lies in [min, max) and that `is_cover` accepts.
    /// A neighbor joins the list only while it is nearer to `near` than 1.5 x its
    /// squared distance to the threat (keeps cover searches from running toward the
    /// threat). Node locks, hint cover activities and climb nodes are not modeled.
    pub fn find_cover_node(
        &self,
        near: Vec3,
        threat: Vec3,
        min: f32,
        max: f32,
        mut is_cover: impl FnMut(Vec3) -> bool,
    ) -> Option<Vec3> {
        let (positions, adjacent) = self.usable_graph()?;
        let count = positions.len();
        let start = nearest_usable(&positions, near)?;
        let max = if max <= 0. { 784. } else { max };
        let min = min.min(0.5 * max);
        let (min_sq, max_sq) = (min * min, max * max);
        let mut visited = vec![false; count];
        visited[start] = true;
        let mut open = vec![(start, 0f32)];
        while let Some(node) = pop_nearest(&mut open) {
            let feet = positions[node].expect("usable");
            let distance = feet.distance_squared(near);
            if distance >= min_sq && distance < max_sq && is_cover(feet) {
                return Some(feet);
            }
            for &next in &adjacent[node] {
                if visited[next] {
                    continue;
                }
                let p = positions[next].expect("usable");
                let d = p.distance_squared(near);
                if d < p.distance_squared(threat) * 1.5 {
                    open.push((next, d));
                }
                visited[next] = true;
            }
        }
        None
    }
    /// CAI_TacticalServices::FindLosNode (FLANKTYPE_NONE): from the usable node nearest
    /// `origin` (the NPC), visit nodes nearest-first by distance from `origin`, skipping
    /// the start node, and return the feet of the first whose distance to `threat` lies
    /// in (min, max) and that `can_shoot` accepts (TestShootPosition).
    pub fn find_los_node(
        &self,
        origin: Vec3,
        threat: Vec3,
        min: f32,
        max: f32,
        mut can_shoot: impl FnMut(Vec3) -> bool,
    ) -> Option<Vec3> {
        let (positions, adjacent) = self.usable_graph()?;
        let count = positions.len();
        let start = nearest_usable(&positions, origin)?;
        let mut visited = vec![false; count];
        visited[start] = true;
        let mut open = vec![(start, 0f32)];
        while let Some(node) = pop_nearest(&mut open) {
            let feet = positions[node].expect("usable");
            if node != start {
                let threat_distance = feet.distance(threat);
                if threat_distance < max && threat_distance > min && can_shoot(feet) {
                    return Some(feet);
                }
            }
            for &next in &adjacent[node] {
                if !visited[next] {
                    visited[next] = true;
                    open.push((
                        next,
                        positions[next].expect("usable").distance_squared(origin),
                    ));
                }
            }
        }
        None
    }
    /// Whether the controller opens route doors for this actor (bits_CAP_DOORS_GROUP).
    pub fn opens_doors(&self, entity: usize) -> bool {
        self.actors.get(&entity).is_none_or(|a| a.opens_doors)
    }
    pub fn set_opens_doors(&mut self, entity: usize, on: bool) {
        if let Some(actor) = self.actors.get_mut(&entity) {
            actor.opens_doors = on;
        }
    }
    pub fn has_actor(&self, entity: usize) -> bool {
        self.actors.contains_key(&entity)
    }
    /// Stop one goal (the AI's TASK_STOP_MOVING); the actor keeps its pose.
    pub fn cancel(&mut self, key: GoalKey) {
        if let Some(goal) = self.goals.get_mut(&key) {
            if goal.state != GoalState::Canceled {
                goal.state = GoalState::Canceled;
                self.pending.push(update(key, goal, &self.doors, None));
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
        // Validates the scale and step parameters.
        normal_human(model_scale, ground.step_height, ground.step_down_multiplier)?;
        let hull = ground
            .hull
            .valid()
            .then(|| HullType::from_bounds(ground.hull, model_scale))
            .flatten()
            .ok_or(GoalBlockReason::UnsupportedHull)?;
        self.actors.insert(
            entity,
            Actor {
                scale: model_scale,
                ground,
                hull,
                locomotion,
                variants: BTreeMap::new(),
                view_offset: None,
                aiming: false,
                opens_doors: true,
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
                sequence: goal.state.is_moving().then(|| {
                    self.actors.get(&key.actor).map_or_else(
                        || goal.request.style.sequence().to_owned(),
                        |a| a.locomotion.played_name(goal.request.style, a.aiming),
                    )
                }),
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
            simplify_in: 0.,
            simplified: false,
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
                let started = std::time::Instant::now();
                let mut probes = 0;
                let route = plan(
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
                    Stale {
                        links: &mut self.stale,
                        now: self.clock,
                    },
                    &mut probes,
                );
                self.stats.plans += 1;
                self.stats.failures += u64::from(route.is_err());
                self.stats.link_probes += probes;
                self.stats.micros += started.elapsed().as_micros() as u64;
                goal.route = route?;
                probe_doors(world, actor, key.actor, &mut goal, &self.doors);
            }
            Ok(())
        })();
        if let Err(reason) = prepared {
            goal.state = GoalState::Blocked(reason);
        }
        // A rejected competing scene never owns the actor's pose or animation.
        if goal.state != GoalState::Blocked(GoalBlockReason::ActorBusy) {
            let name = self
                .actors
                .get(&key.actor)
                .map(|a| a.locomotion.played_name(goal.request.style, a.aiming));
            self.pending.push(update(key, &goal, &self.doors, name));
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
                    self.pending.push(update(*key, goal, &self.doors, None));
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
        if dt.is_finite() && dt > 0. {
            self.clock += f64::from(dt);
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
                let name = self
                    .actors
                    .get(&key.actor)
                    .map(|a| a.locomotion.played_name(goal.request.style, a.aiming));
                updates.push(update(*key, goal, &self.doors, name));
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
                    let started = std::time::Instant::now();
                    let mut probes = 0;
                    let route = plan(
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
                        Stale {
                            links: &mut self.stale,
                            now: self.clock,
                        },
                        &mut probes,
                    );
                    self.stats.plans += 1;
                    self.stats.failures += u64::from(route.is_err());
                    self.stats.link_probes += probes;
                    self.stats.micros += started.elapsed().as_micros() as u64;
                    goal.route = route?;
                    goal.next = 0;
                    goal.state = GoalState::Moving;
                }
                let moved = advance(world, actor, *key, goal, track, transients, dt);
                probe_doors(world, actor, key.actor, goal, &self.doors);
                moved
            })();
            if let Err(reason) = outcome {
                // MarkCurWaypointFailedLink: a move blocked on a graph link (not by a
                // door, an NPC or the player, which hold and retry) marks the link stale.
                if let GoalBlockReason::Collision { blocker, .. } = &reason {
                    let actor_block = blocker.is_some_and(|b| {
                        self.doors.contains(&b) || transients.iter().any(|t| t.entity == Some(b))
                    });
                    let first_block = !matches!(goal.state, GoalState::Blocked(_));
                    if !actor_block && first_block {
                        if let Some(link) = self
                            .graph
                            .as_ref()
                            .and_then(|g| route_link(g, &goal.route, goal.next))
                        {
                            self.stale.insert(link, self.clock + STALE_LINK_TIME);
                        }
                    }
                }
                goal.state = GoalState::Blocked(reason);
            }
            let name = self
                .actors
                .get(&key.actor)
                .map(|a| a.locomotion.played_name(goal.request.style, a.aiming));
            updates.push(update(*key, goal, &self.doors, name));
        }
        updates
    }
}
fn update(key: GoalKey, goal: &Goal, doors: &BTreeSet<usize>, name: Option<String>) -> MoveUpdate {
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
            .then(|| name.unwrap_or_else(|| goal.request.style.sequence().to_owned())),
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
    // CAI_Navigator::SimplifyPath for AI goals (scene moves keep their routes).
    if key.scene == AI_SCENE {
        goal.simplify_in -= dt;
        if goal.simplify_in <= 0. {
            goal.simplify_in = ROUTE_SIMPLIFY_DELAY;
            let forced = !goal.simplified;
            goal.simplified = true;
            let plain = Query {
                excluded_entities: &excluded,
                ..Default::default()
            };
            simplify_path_forward(world, actor, goal, plain, forced);
        }
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

/// Usable node feet and each node's usable neighbors.
type UsableGraph = (Vec<Option<Vec3>>, Vec<Vec<usize>>);
/// NearestNodeToPoint over the usable nodes.
fn nearest_usable(positions: &[Option<Vec3>], point: Vec3) -> Option<usize> {
    positions
        .iter()
        .enumerate()
        .filter_map(|(n, p)| p.map(|p| (n, p.distance_squared(point))))
        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
        .map(|(n, _)| n)
}
/// CNodeList::ElementAtHead + RemoveAtHead: the open node with the least key.
fn pop_nearest(open: &mut Vec<(usize, f32)>) -> Option<usize> {
    let best = (0..open.len()).min_by(|&a, &b| open[a].1.total_cmp(&open[b].1))?;
    Some(open.swap_remove(best).0)
}
/// CAI_Node::GetPosition(hull): the node origin raised by the hull's offset.
fn node_feet(
    graph: &Graph,
    node: usize,
    restrictions: &NavRestrictions,
    hull: HullType,
) -> Option<Vec3> {
    let value = graph.nodes.get(node)?;
    if value.node_type != 2 || value.info != 0 || restrictions.unusable_nodes.contains(&node) {
        return None;
    }
    let feet = value.origin + Vec3::Z * *value.hull_z_offsets.get(hull.index())?;
    feet.is_finite().then_some(feet)
}
/// The link's accepted move types for the hull include bits_CAP_MOVE_GROUND.
fn ground_link(link: &source_assets::navigation::Link, hull: HullType) -> bool {
    link.accepted_move_types
        .get(hull.index())
        .is_some_and(|types| types & 1 != 0)
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
            let feet = node_feet(graph, index, restrictions, actor.hull)?;
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
            let feet = node_feet(graph, index, restrictions, actor.hull)?;
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
    stale: Stale<'_>,
    probes: &mut u64,
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
            door_known: vec![true],
        });
    }
    let graph = graph.ok_or(GoalBlockReason::MissingGraph)?;
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
                node_feet(graph, *n, restrictions, actor.hull).unwrap(),
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
                node_feet(graph, *n, restrictions, actor.hull).unwrap(),
                end,
                tolerance,
                query,
            )
        })
        .ok_or(GoalBlockReason::NoRoute)?;
    // CAI_Pathfinder::IsLinkUsable: graph data only (flags, accepted move types and
    // unusable nodes, in `search`); a stale-suggested link is unusable until it expires,
    // then IsLinkStillStale rechecks it once per plan (CheckStaleRoute: the local move).
    let Stale { links, now } = stale;
    let mut rechecked = false;
    let nodes = search(
        graph,
        restrictions,
        actor.hull,
        start_node,
        end_node,
        |link, from, to| {
            let Some(&expires) = links.get(&link) else {
                return Some(from.distance(to));
            };
            if now < expires || rechecked {
                return None;
            }
            rechecked = true;
            *probes += 1;
            if connector(world, actor, from, to, 0.1, query)
                || door_link(world, actor, from, to, query, doors)
            {
                links.remove(&link);
                Some(from.distance(to))
            } else {
                links.insert(link, now + STALE_RECHECK_TIME);
                None
            }
        },
    )?;
    let mut feet: Vec<_> = nodes
        .iter()
        .map(|n| node_feet(graph, *n, restrictions, actor.hull).unwrap())
        .collect();
    feet.push(end);
    // Doors along the route are probed as the actor reaches each segment (probe_doors).
    let mut door_known = vec![false; feet.len()];
    door_known[0] = true;
    Ok(Route {
        nodes,
        doors: vec![None; feet.len()],
        feet,
        door_known,
    })
}
/// The stale-suggested links and the movement clock, for one plan.
struct Stale<'a> {
    links: &'a mut BTreeMap<usize, f64>,
    now: f64,
}
/// The graph link of the route segment that ends at `feet[next]` (node to node).
fn route_link(graph: &Graph, route: &Route, next: usize) -> Option<usize> {
    let (&a, &b) = (
        route.nodes.get(next.checked_sub(1)?)?,
        route.nodes.get(next)?,
    );
    graph.links.iter().position(|l| {
        (l.source == a && l.destination == b) || (l.source == b && l.destination == a)
    })
}
/// The doors of the current and next route segments (the navigator opens a route door
/// ahead of contact): one local move per segment, when the actor reaches it.
fn probe_doors<W: NpcCollisionWorld>(
    world: &W,
    actor: &Actor,
    entity: usize,
    goal: &mut Goal,
    door_set: &BTreeSet<usize>,
) {
    if door_set.is_empty() {
        return;
    }
    let excluded = [entity];
    let query = Query {
        contents_mask: npc_probe::NPC_BRUSH_ONLY_MASK,
        excluded_entities: &excluded,
        ..Default::default()
    };
    for index in [goal.next, goal.next + 1] {
        if index == 0 || goal.route.door_known.get(index) != Some(&false) {
            continue;
        }
        goal.route.door_known[index] = true;
        let (from, to) = (goal.route.feet[index - 1], goal.route.feet[index]);
        let moved = npc_probe::ground_move(world, from, to, actor.ground, query);
        goal.route.doors[index] = moved
            .blocker
            .filter(|b| !moved.completed && door_set.contains(b));
    }
}
fn search(
    graph: &Graph,
    restrictions: &NavRestrictions,
    hull: HullType,
    start: usize,
    end: usize,
    mut cost: impl FnMut(usize, Vec3, Vec3) -> Option<f32>,
) -> Result<Vec<usize>, GoalBlockReason> {
    let count = graph.nodes.len();
    if count == 0
        || count > 1500
        || graph.links.len() > 1_000_000
        || node_feet(graph, start, restrictions, hull).is_none()
        || node_feet(graph, end, restrictions, hull).is_none()
    {
        return Err(GoalBlockReason::InvalidGraph);
    }
    let positions: Vec<_> = (0..count)
        .map(|n| node_feet(graph, n, restrictions, hull))
        .collect();
    let mut adjacent = vec![Vec::new(); count];
    for (index, link) in graph.links.iter().enumerate() {
        if link.source >= count || link.destination >= count {
            return Err(GoalBlockReason::InvalidGraph);
        }
        if ground_link(link, hull)
            && !restrictions.unusable_links.contains(&index)
            && !restrictions.off_links.contains(&index)
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
    #[test]
    fn path_simplification_cuts_corners_within_the_heading_cone() {
        let physics = Physics::new(&world(vec![]));
        let controller = make_controller(None);
        let actor = &controller.actors[&7];
        let excluded = [7];
        let query = Query {
            excluded_entities: &excluded,
            ..Default::default()
        };
        let route = Route {
            nodes: vec![],
            feet: vec![
                Vec3::new(150., 0., 0.),
                Vec3::new(150., 150., 0.),
                Vec3::new(150., 400., 0.),
            ],
            doors: vec![None; 3],
            ..Default::default()
        };
        let mut goal = Goal {
            request: request(Vec3::new(150., 400., 0.)),
            tolerance: 0.1,
            state: GoalState::Moving,
            route,
            next: 0,
            feet: Vec3::ZERO,
            yaw: 0.,
            animation_time: 0.,
            retry_time: 0.,
            contact_normal: Vec3::ZERO,
            simplify_in: 0.,
            simplified: false,
        };
        // Every scan point lies beyond 12 ft: the first (forced) pass skips to the next
        // reachable waypoint.
        simplify_path_forward(&physics, actor, &mut goal, query, true);
        assert_eq!(goal.next, 1);
        // Near the corner, heading +X: the nearest point up the next leg is inside the
        // 40-degree cone and replaces the corner; farther points are outside it.
        goal.next = 0;
        goal.feet = Vec3::new(100., 0., 0.);
        simplify_path_forward(&physics, actor, &mut goal, query, false);
        assert_eq!(goal.next, 0);
        assert!((goal.route.feet[0] - Vec3::new(150., 36., 0.)).length() < 0.01);
    }
    #[test]
    fn dynamic_links_take_their_initial_state_and_toggle() {
        // Nodes 0-1-2 in a row (editor ids 0..2); a link entity turns 1-2 off, a
        // controller box across 0-1 starts on.
        let g = graph(
            &[Vec3::ZERO, Vec3::new(200., 0., 0.), Vec3::new(400., 0., 0.)],
            &[(0, 1), (1, 2)],
        );
        let entity = |pairs: &[(&str, &str)]| modkit_core::Entity {
            properties: pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        };
        let world = World {
            entities: vec![
                entity(&[
                    ("classname", "info_node_link"),
                    ("StartNode", "2"),
                    ("EndNode", "1"),
                    ("initialstate", "0"),
                ]),
                entity(&[
                    ("classname", "info_node_link_controller"),
                    ("origin", "100 0 0"),
                    ("mins", "-8 -64 -64"),
                    ("maxs", "8 64 64"),
                    ("initialstate", "1"),
                ]),
                entity(&[
                    ("classname", "info_node_link"),
                    ("StartNode", "0"),
                    ("EndNode", "2"),
                ]),
            ],
            ..Default::default()
        };
        let mut r = NavRestrictions::from_world(&world, &g);
        assert_eq!(r.dynamic[&0], [1]);
        assert_eq!(r.dynamic[&1], [0]);
        assert_eq!(r.missing_dynamic, 1);
        assert_eq!(r.off_links, [1].into());
        assert!(r.set_dynamic(0, true) && r.off_links.is_empty());
        assert!(r.set_dynamic(1, false) && r.off_links == [0].into());
        assert!(!r.set_dynamic(2, true));
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
        assert!(node_feet(&invalid, 1, &NavRestrictions::default(), HullType::Human).is_none());
        assert!(search(
            &invalid,
            &NavRestrictions::default(),
            HullType::Human,
            0,
            3,
            |_, a, b| Some(a.distance(b))
        )
        .is_err());
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
    fn a_blocked_graph_link_turns_stale_and_the_retry_routes_around_it() {
        // The graph claims a direct link 0-3 through the wall: the search trusts link data
        // (CAI_Pathfinder::IsLinkUsable), the walk blocks, MarkCurWaypointFailedLink marks
        // the link stale and the retry goes around.
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
        let edges = [(0, 1), (1, 2), (2, 3), (0, 3)];
        let mut controller = make_controller(Some(graph(&points, &edges)));
        let mut feet = Vec3::ZERO;
        let mut wanted = request(points[3]);
        wanted.force_short = true;
        controller.request(key(1), wanted, pose(feet), &physics, &[]);
        assert_eq!(controller.snapshots()[0].route_nodes, vec![0, 3]);
        assert_eq!(controller.path_stats().link_probes, 0);
        let mut stale_seen = false;
        for _ in 0..800 {
            tick(&mut controller, &physics, &mut feet, 0.02, false);
            stale_seen |= controller.stale.contains_key(&3);
            if controller.status(key(1)).unwrap().is_arrived() {
                break;
            }
        }
        assert!(stale_seen);
        assert!(controller.status(key(1)).unwrap().is_arrived());
        assert_eq!(controller.snapshots()[0].route_nodes, vec![0, 1, 2, 3]);
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
            search(
                &diamond,
                &NavRestrictions::default(),
                HullType::Human,
                0,
                3,
                |_, a, b| Some(a.distance(b))
            )
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
            search(
                &reopen,
                &NavRestrictions::default(),
                HullType::Human,
                0,
                4,
                |link, _, _| { Some(costs[link]) }
            )
            .unwrap(),
            vec![0, 2, 1, 3, 4]
        );
        assert_eq!(
            search(
                &reopen,
                &NavRestrictions::default(),
                HullType::Human,
                0,
                4,
                |_, _, _| Some(f32::NAN)
            ),
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
    #[test]
    fn tactical_searches_pick_the_nearest_cover_and_line_of_fire_nodes() {
        // Synthetic chain of nodes along +X; node 0 under the NPC.
        let points: Vec<Vec3> = (0..6).map(|i| Vec3::new(i as f32 * 100., 0., 0.)).collect();
        let edges: Vec<(usize, usize)> = (0..5).map(|i| (i, i + 1)).collect();
        let controller = Controller::new(Some(graph(&points, &edges)), NavRestrictions::default());
        // Threat far behind (-X): every node is "cover" past x 250; the nearest wins.
        let threat = Vec3::new(-1000., 0., 0.);
        let cover = controller.find_cover_node(Vec3::ZERO, threat, 0., 1024., |p| p.x > 250.);
        assert_eq!(cover, Some(Vec3::new(300., 0., 0.)));
        // A threat ahead (+X 450): nodes nearer to it than 1/1.5 of their distance to us
        // are not expanded, so the search cannot run through the threat.
        let ahead = Vec3::new(450., 0., 0.);
        let cover = controller.find_cover_node(Vec3::ZERO, ahead, 0., 1024., |p| p.x > 450.);
        assert_eq!(cover, None);
        // Line of fire: skips the start node, needs (min, max) to the threat.
        let los = controller.find_los_node(Vec3::ZERO, ahead, 0., 300., |_| true);
        assert_eq!(los, Some(Vec3::new(200., 0., 0.)));
        let los = controller.find_los_node(Vec3::ZERO, ahead, 0., 1024., |p| p.x >= 500.);
        assert_eq!(los, Some(Vec3::new(500., 0., 0.)));
        assert!(Controller::new(None, NavRestrictions::default())
            .find_los_node(Vec3::ZERO, ahead, 0., 1024., |_| true)
            .is_none());
    }
}
