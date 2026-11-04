//! Collision probes for a bounded ground NPC controller, in Source units/Z-up.
//!
//! These use the published move-probe structure, not the full Source motor:
//! no give-way, collision-pair overrides, vehicles, climbing or door opening.
use glam::Vec3;
use modkit_core::Trace;

/// Retail MoveLimit uses this mask during ordinary NPC execution.
pub const NPC_SOLID_MASK: u32 = 0x0202_400b;
/// Retail nearest-node/IGNORE_NPCS probes use this brush/prop mask.
pub const NPC_BRUSH_ONLY_MASK: u32 = 0x0002_400b;
pub const CONTENTS_MONSTER: u32 = 0x0200_0000;

#[derive(Clone, Copy, Debug)]
pub struct Hull {
    pub mins: Vec3,
    pub maxs: Vec3,
}
impl Hull {
    pub fn valid(self) -> bool {
        self.mins.is_finite()
            && self.maxs.is_finite()
            && (self.maxs - self.mins).is_finite()
            && (self.maxs + self.mins).is_finite()
            && self.mins.cmple(self.maxs).all()
    }
}

/// Explicit current actor bounds; the player has no rigid-body collider.
#[derive(Clone, Copy, Debug)]
pub struct ActorHull {
    /// None denotes a player proxy rather than an indexed map entity.
    pub entity: Option<usize>,
    pub feet: Vec3,
    pub hull: Hull,
}

#[derive(Clone, Copy, Debug)]
pub struct Query<'a> {
    pub contents_mask: u32,
    pub excluded_entities: &'a [usize],
    pub transients: &'a [ActorHull],
}
impl Default for Query<'_> {
    fn default() -> Self {
        Self {
            contents_mask: NPC_SOLID_MASK,
            excluded_entities: &[],
            transients: &[],
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct HullTrace {
    pub trace: Trace,
    /// None also covers world brushes and terrain.
    pub entity: Option<usize>,
    pub transient: bool,
}
impl HullTrace {
    pub fn clear() -> Self {
        Self {
            trace: Trace {
                fraction: 1.,
                normal: Vec3::ZERO,
                start_solid: false,
                all_solid: false,
            },
            entity: None,
            transient: false,
        }
    }
    pub fn invalid() -> Self {
        Self {
            trace: Trace {
                fraction: 0.,
                normal: Vec3::ZERO,
                start_solid: true,
                all_solid: true,
            },
            ..Self::clear()
        }
    }
    pub fn clear_path(self) -> bool {
        !self.trace.start_solid && !self.trace.all_solid && self.trace.fraction == 1.
    }
}

pub trait NpcCollisionWorld {
    fn npc_trace_hull(&self, start: Vec3, end: Vec3, hull: Hull, query: Query<'_>) -> HullTrace;
}

#[derive(Clone, Copy, Debug)]
pub struct GroundConfig {
    pub hull: Hull,
    /// Supplied by the actor, rather than assuming player or human step height.
    pub step_height: f32,
    pub step_down_multiplier: f32,
}
impl GroundConfig {
    fn valid(self) -> bool {
        self.hull.valid()
            && self.hull.maxs.x > self.hull.mins.x
            && self.hull.maxs.y > self.hull.mins.y
            && self.hull.maxs.z > self.hull.mins.z
            && self.step_height.is_finite()
            && self.step_height > 0.
            && self.step_down_multiplier.is_finite()
            && self.step_down_multiplier >= 1.
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockReason {
    InvalidInput,
    StartSolid,
    UnsupportedGround,
    Obstructed,
    NoGround,
    TravelBudget,
}
#[derive(Clone, Copy, Debug)]
pub struct GroundMove {
    pub end: Vec3,
    /// Horizontal travel completed. Callers must separately check goal Z/radius.
    pub completed: bool,
    pub blocker: Option<usize>,
    pub normal: Vec3,
    pub reason: Option<BlockReason>,
}

// Pinned ai_moveprobe.cpp: CheckStep/TestGroundMove, not invented motor tuning.
const MOVE_HEIGHT_EPSILON: f32 = 0.0625;
const LOCAL_STEP_SIZE: f32 = 16.;
// Our CPU budget, not a recovered engine constant. Reject rather than truncate.
const MAX_SUBSTEPS: usize = 256;

fn supported(hit: HullTrace) -> bool {
    !hit.trace.start_solid
        && !hit.trace.all_solid
        && hit.trace.fraction < 1.
        && hit.trace.normal.z > 0.
        // Standing on other actors requires Source's separate big-step rules.
        && !hit.transient
}

/// SDK CheckStandPosition's two diagonal contact pairs. Efficient-NPC center
/// sampling and CanStandOn class overrides are intentionally not reproduced.
pub fn stand<W: NpcCollisionWorld>(
    world: &W,
    feet: Vec3,
    config: GroundConfig,
    query: Query<'_>,
) -> bool {
    if !feet.is_finite() || !config.valid() {
        return false;
    }
    let up = feet + Vec3::Z * 0.1;
    let down = feet - Vec3::Z * config.step_height * config.step_down_multiplier;
    let mins = config.hull.mins;
    let maxs = config.hull.maxs;
    let contact_min = Vec3::new(
        mins.x * 0.75 + maxs.x * 0.25,
        mins.y * 0.75 + maxs.y * 0.25,
        mins.z,
    );
    let contact_max = Vec3::new(
        mins.x * 0.25 + maxs.x * 0.75,
        mins.y * 0.25 + maxs.y * 0.75,
        mins.z,
    );
    let center = Vec3::new(0., 0., mins.z);
    let pairs = [
        (contact_min, center, center, contact_max),
        (
            Vec3::new(contact_min.x, 0., mins.z),
            Vec3::new(0., contact_max.y, mins.z),
            Vec3::new(0., contact_min.y, mins.z),
            Vec3::new(contact_max.x, 0., mins.z),
        ),
    ];
    pairs.into_iter().any(|(a, b, c, d)| {
        supported(world.npc_trace_hull(up, down, Hull { mins: a, maxs: b }, query))
            && supported(world.npc_trace_hull(up, down, Hull { mins: c, maxs: d }, query))
    })
}

/// Retail CanFitAtPosition's 0.01 upward hull check.
pub fn fits<W: NpcCollisionWorld>(world: &W, feet: Vec3, hull: Hull, query: Query<'_>) -> bool {
    feet.is_finite()
        && hull.valid()
        && !world
            .npc_trace_hull(feet, feet + Vec3::Z * 0.01, hull, query)
            .trace
            .start_solid
}

/// Preserves an accelerated authored curve when a waypoint splits a tick: the time within
/// `remaining` whose forward motion reaches `distance`.
pub fn time_to_cover<E>(
    distance: f32,
    remaining: f32,
    forward: impl Fn(f32) -> Result<f32, E>,
) -> Result<f32, E> {
    let (mut low, mut high) = (0., remaining);
    for _ in 0..24 {
        let mid = (low + high) * 0.5;
        if forward(mid)? < distance {
            low = mid;
        } else {
            high = mid;
        }
    }
    Ok(high)
}

fn failure(end: Vec3, reason: BlockReason, hit: HullTrace) -> GroundMove {
    GroundMove {
        end,
        completed: false,
        blocker: hit.entity,
        normal: hit.trace.normal,
        reason: Some(reason),
    }
}

/// Validate directed 2.5D ground travel with hull up/forward/down sweeps and
/// floor checks. A blocked substep retains the last validated position: Source
/// can make partial motor moves/give way, which this conservative probe omits.
pub fn ground_move<W: NpcCollisionWorld>(
    world: &W,
    start: Vec3,
    desired_end: Vec3,
    config: GroundConfig,
    query: Query<'_>,
) -> GroundMove {
    if !start.is_finite() || !desired_end.is_finite() || !config.valid() {
        return failure(start, BlockReason::InvalidInput, HullTrace::invalid());
    }
    let initial = world.npc_trace_hull(start, start + Vec3::Z * 0.01, config.hull, query);
    if initial.trace.start_solid {
        return failure(start, BlockReason::StartSolid, initial);
    }
    // STEP_ON_INVALID_GROUND: a start hanging over an edge "toughs it out", or an NPC blown into
    // an invalid spot could never walk off it. Its next move, from valid ground, is strict again.
    let shaky = !stand(world, start, config, query);
    ground_steps(world, start, desired_end, config, query, shaky)
}

/// `ground_move` for a caller that just proved `start` passes `fits` and `stand` this tick:
/// those same traces would give the same answers.
pub fn ground_move_from_valid<W: NpcCollisionWorld>(
    world: &W,
    start: Vec3,
    desired_end: Vec3,
    config: GroundConfig,
    query: Query<'_>,
) -> GroundMove {
    ground_steps(world, start, desired_end, config, query, false)
}

/// GroundMoveLimit's steps; `shaky` skips the stand test of each one.
fn ground_steps<W: NpcCollisionWorld>(
    world: &W,
    start: Vec3,
    desired_end: Vec3,
    config: GroundConfig,
    query: Query<'_>,
    shaky: bool,
) -> GroundMove {
    if !desired_end.is_finite() {
        return failure(start, BlockReason::InvalidInput, HullTrace::invalid());
    }
    let delta = (desired_end - start) * Vec3::new(1., 1., 0.);
    let distance = delta.length();
    if !distance.is_finite() || distance > LOCAL_STEP_SIZE * MAX_SUBSTEPS as f32 {
        return failure(start, BlockReason::TravelBudget, HullTrace::clear());
    }
    let count = (distance / LOCAL_STEP_SIZE).ceil() as usize;
    let direction = delta.try_normalize().unwrap_or(Vec3::ZERO);
    let mut end = start;
    for i in 0..count {
        let amount = (distance - i as f32 * LOCAL_STEP_SIZE).min(LOCAL_STEP_SIZE);
        let from = end + Vec3::Z * MOVE_HEIGHT_EPSILON;
        let target = from + direction * amount;
        let forward = world.npc_trace_hull(from, target, config.hull, query);
        let mut horizontal_end = target;
        if !forward.clear_path() {
            // Unlike Source's escape-from-invalid-ground path, starting inside
            // geometry fails explicitly, so no controller can teleport out.
            if forward.trace.start_solid {
                return failure(end, BlockReason::StartSolid, forward);
            }
            let obstruction = from.lerp(target, forward.trace.fraction);
            let up_target = obstruction + Vec3::Z * config.step_height;
            let up = world.npc_trace_hull(obstruction, up_target, config.hull, query);
            if up.trace.start_solid || up.trace.all_solid {
                return failure(end, BlockReason::Obstructed, up);
            }
            let raised = obstruction.lerp(up_target, up.trace.fraction);
            let raised_target = Vec3::new(target.x, target.y, raised.z);
            let across = world.npc_trace_hull(raised, raised_target, config.hull, query);
            if !across.clear_path() {
                return failure(end, BlockReason::Obstructed, across);
            }
            // SDK minimum landing clearance is hull width/3. Check beyond a
            // short substep without moving the actor to that lookahead point.
            let minimum = (config.hull.maxs.y - config.hull.mins.y) / 3.;
            if (raised_target - raised).truncate().length() < minimum {
                let landing =
                    world.npc_trace_hull(raised, raised + direction * minimum, config.hull, query);
                if !landing.clear_path() {
                    return failure(end, BlockReason::Obstructed, landing);
                }
            }
            horizontal_end = raised_target;
        }
        let down_target = Vec3::new(
            horizontal_end.x,
            horizontal_end.y,
            end.z - config.step_height * config.step_down_multiplier - MOVE_HEIGHT_EPSILON,
        );
        let floor = world.npc_trace_hull(horizontal_end, down_target, config.hull, query);
        if !supported(floor) {
            return failure(end, BlockReason::NoGround, floor);
        }
        let landed = horizontal_end.lerp(down_target, floor.trace.fraction);
        if !shaky && !stand(world, landed, config, query) {
            return failure(end, BlockReason::UnsupportedGround, floor);
        }
        let candidate = landed + Vec3::Z * MOVE_HEIGHT_EPSILON;
        if !fits(world, candidate, config.hull, query) {
            return failure(end, BlockReason::Obstructed, floor);
        }
        end = candidate;
    }
    GroundMove {
        end,
        completed: true,
        blocker: None,
        normal: Vec3::ZERO,
        reason: None,
    }
}
