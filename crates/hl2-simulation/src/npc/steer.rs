//! A ground move blocked by another NPC or the player on its way to the current waypoint
//! (Source SDK `game/server/ai_localnavigator.cpp` MoveCalcRaw; `ai_navigator.cpp`
//! OnObstructionPreSteer, OnFailedSteer, CalculateDeflection, PrependLocalAvoidance and
//! OnMoveBlocked; retail server.dll keeps `ai_no_steer` and the navigator's
//! `m_flTimeLastAvoidanceTriangulate`). In order: slide along an NPC's hit normal, prepend
//! a triangulated detour to the path (once per think), skip a reducible waypoint nearer
//! than a hull width; otherwise the move stays blocked and the schedule retries.
//!
//! Not modeled: the plane solver's steering (MoveCalcSteer) that runs before these, and the
//! same handling for world and entity obstructions, which keep the stale-link path.

use super::graph::triangulate;
use super::{links, Actor, Goal, Route};
use crate::npc_probe::{self, GroundMove, NpcCollisionWorld, Query};
use glam::Vec3;

/// OnFailedSteer takes the deflection only while it stays this close to the wanted
/// direction (2D dot product).
const DEFLECTION_MIN_DOT: f32 = 0.7;

/// What the motor does with a blocked substep.
enum Steer {
    /// A deflected move was made (feet, yaw and animation time updated).
    Moved,
    /// The path changed (a detour prepended or a near waypoint skipped): walk on.
    Rerouted,
    /// The move stays blocked.
    Blocked,
}

/// A substep from `start` along `direction` for `amount` units over `consumed` seconds of
/// the move's animation; `validated`: `fits` and `stand` just passed at `start`.
pub(super) struct Step {
    start: Vec3,
    direction: Vec3,
    amount: f32,
    consumed: f32,
    validated: bool,
}
impl Step {
    pub fn new(start: Vec3, direction: Vec3, amount: f32, consumed: f32, validated: bool) -> Self {
        Self {
            start,
            direction,
            amount,
            consumed,
            validated,
        }
    }
}

/// A substep's outcome.
pub(super) enum Walk {
    /// The plain ground move, completed or blocked.
    Moved(GroundMove),
    /// Steered around an actor: this many seconds of the move were used; walk on.
    Steered(f32),
}

/// The substep's ground move, steered when an actor blocks it. `triangulated` is this
/// think's PrependLocalAvoidance gate.
pub(super) fn walk<W: NpcCollisionWorld>(
    world: &W,
    actor: &Actor,
    goal: &mut Goal,
    step: Step,
    query: Query<'_>,
    triangulated: &mut bool,
) -> Walk {
    let end = step.start + step.direction * step.amount;
    let movement = if step.validated {
        npc_probe::ground_move_from_valid(world, step.start, end, actor.ground, query)
    } else {
        npc_probe::ground_move(world, step.start, end, actor.ground, query)
    };
    if movement.completed {
        return Walk::Moved(movement);
    }
    let blocked = Blocked {
        step: &step,
        movement: &movement,
    };
    match blocked.steer(world, actor, goal, query, triangulated) {
        Steer::Moved => Walk::Steered(step.consumed),
        Steer::Rerouted => Walk::Steered(0.),
        Steer::Blocked => Walk::Moved(movement),
    }
}

/// A substep and its blocked move.
struct Blocked<'a> {
    step: &'a Step,
    movement: &'a GroundMove,
}

/// The query for path simplification: CAI_Navigator::ShouldSimplifyTo ignores other NPCs
/// (AIMLF_IGNORE_TRANSIENTS) unless the scan passes a detour waypoint.
pub(super) fn simplify_query<'a>(goal: &Goal, query: Query<'a>) -> Query<'a> {
    if goal.next < goal.route.detour_end {
        return query;
    }
    Query {
        transients: &[],
        ..query
    }
}

impl Blocked<'_> {
    /// OnFailedSteer, PrependLocalAvoidance and OnMoveBlocked for a substep blocked by an
    /// actor.
    fn steer<W: NpcCollisionWorld>(
        &self,
        world: &W,
        actor: &Actor,
        goal: &mut Goal,
        query: Query<'_>,
        triangulated: &mut bool,
    ) -> Steer {
        let (step, movement) = (self.step, self.movement);
        // OnObstructionPreSteer: blocked within the goal tolerance arrives (the caller's check).
        let at_goal = movement.end.distance(goal.request.target_feet) <= goal.tolerance;
        if !movement.transient || at_goal {
            return Steer::Blocked;
        }
        // AIMR_BLOCKED_NPC only: the player has no entity here.
        if movement.blocker.is_some() {
            if let Some(moved) = deflect(world, actor, self, query) {
                goal.feet = moved.end;
                goal.contact_normal = Vec3::ZERO;
                goal.yaw = step.direction.y.atan2(step.direction.x).to_degrees();
                goal.animation_time += step.consumed;
                return Steer::Moved;
            }
        }
        if !std::mem::replace(triangulated, true)
            && prepend_avoidance(world, actor, goal, self, query)
        {
            return Steer::Rerouted;
        }
        if skip_near_waypoint(actor, goal, step.start) {
            return Steer::Rerouted;
        }
        Steer::Blocked
    }
}

/// CalculateDeflection: the wanted direction projected onto the plane of the hit normal,
/// taken when it stays within DEFLECTION_MIN_DOT and the move along it is clear.
fn deflect<W: NpcCollisionWorld>(
    world: &W,
    actor: &Actor,
    blocked: &Blocked,
    query: Query<'_>,
) -> Option<GroundMove> {
    let step = blocked.step;
    let deflection = deflection(step.direction, blocked.movement.normal)?;
    let end = step.start + deflection * step.amount;
    let moved = npc_probe::ground_move(world, step.start, end, actor.ground, query);
    moved.completed.then_some(moved)
}

/// The direction along the hit plane, kept within DEFLECTION_MIN_DOT of `direction`.
fn deflection(direction: Vec3, normal: Vec3) -> Option<Vec3> {
    let along = normal.cross(direction.cross(normal)).normalize_or_zero();
    (direction.truncate().dot(along.truncate()) > DEFLECTION_MIN_DOT).then_some(along)
}

/// PrependLocalAvoidance: a triangulated detour to the current waypoint, first backing a
/// hull's width away from it when the obstruction is nearer than half a hull. This
/// interval's move toward the detour's first point must be clear.
fn prepend_avoidance<W: NpcCollisionWorld>(
    world: &W,
    actor: &Actor,
    goal: &mut Goal,
    blocked: &Blocked,
    query: Query<'_>,
) -> bool {
    let step = blocked.step;
    let Some(&waypoint) = goal.route.feet.get(goal.next) else {
        return false;
    };
    let hull = actor.ground.hull;
    let width = hull.maxs.x - hull.mins.x;
    let clear = (blocked.movement.end - step.start).truncate().length();
    let mut points = Vec::new();
    let mut from = step.start;
    if clear < width * 0.5 {
        let away = (step.start - waypoint)
            .truncate()
            .normalize_or_zero()
            .extend(0.);
        let back = npc_probe::ground_move(world, from, from + away * width, actor.ground, query);
        if back.completed {
            from = back.end;
            points.push(from);
        }
    }
    let Some(apex) = triangulate::apex(world, actor, (from, waypoint), goal.tolerance, query)
    else {
        return false;
    };
    points.push(apex);
    let toward = points[0] - step.start;
    let test = step.start + toward.clamp_length_max(step.amount);
    if !npc_probe::ground_move(world, step.start, test, actor.ground, query).completed {
        return false;
    }
    insert_detour(&mut goal.route, goal.next, &points);
    true
}

/// The detour's waypoints go in before `next`; the waypoint it leads to counts as a detour
/// too (BuildTriangulationRoute's end copy carries bits_WP_TO_DETOUR).
fn insert_detour(route: &mut Route, next: usize, points: &[Vec3]) {
    for (i, &point) in points.iter().enumerate() {
        let at = next + i;
        route.feet.insert(at, point);
        if at < route.doors.len() {
            route.doors.insert(at, None);
        }
        if at < route.door_known.len() {
            route.door_known.insert(at, true);
        }
        if at < route.navs.len() {
            route.navs.insert(at, links::Nav::Ground);
        }
        // A detour point has no node: repeating the previous node keeps route_link's
        // segments (no link into the detour, the original link out of it).
        if at <= route.nodes.len() {
            if let Some(&node) = route.nodes.get(at.saturating_sub(1)) {
                route.nodes.insert(at, node);
            }
        }
    }
    route.detour_end = next + points.len() + 1;
}

/// OnMoveBlocked: a reducible waypoint (not the goal, walked on to the next) nearer than a
/// hull width is skipped.
fn skip_near_waypoint(actor: &Actor, goal: &mut Goal, feet: Vec3) -> bool {
    let next = goal.next;
    if next + 1 >= links::ground_end(&goal.route, next) {
        return false;
    }
    let hull = actor.ground.hull;
    let width = hull.maxs.x - hull.mins.x;
    let waypoint = goal.route.feet[next];
    if (waypoint - feet).truncate().length() >= width {
        return false;
    }
    goal.next += 1;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_deflection_slides_along_the_hit_plane_only_at_glancing_angles() {
        // A hull face hit head-on (normal -X) gives nothing to slide along; heading 53
        // degrees off the normal (dot 0.8 > 0.7) slides along +Y, 37 degrees (0.6) does not.
        let face = Vec3::NEG_X;
        assert!(deflection(Vec3::X, face).is_none());
        let along = deflection(Vec3::new(0.6, 0.8, 0.), face).unwrap();
        assert!(along.abs_diff_eq(Vec3::Y, 1e-5), "{along}");
        assert!(deflection(Vec3::new(0.8, 0.6, 0.), face).is_none());
    }
}
