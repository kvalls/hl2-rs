//! A ground move blocked by another NPC or the player on its way to the current waypoint
//! (Source SDK `game/server/ai_localnavigator.cpp` MoveCalcRaw; `ai_navigator.cpp`
//! OnObstructionPreSteer, OnFailedSteer, CalculateDeflection, PrependLocalAvoidance and
//! OnMoveBlocked; retail server.dll keeps `ai_no_steer` and the navigator's
//! `m_flTimeLastAvoidanceTriangulate`). In order: slide along an NPC's hit normal, prepend
//! a triangulated detour to the path (once per think), skip a reducible waypoint nearer
//! than a hull width; otherwise the move stays blocked and the schedule retries.
//!
//! Local steering tries the deflection and alternate headings before changing the route.

use super::graph::triangulate;
use super::{links, Actor, Goal, Route};
use crate::npc_probe::{self, GroundMove, NpcCollisionWorld, Query};
use glam::Vec3;

/// OnFailedSteer takes the deflection only while it stays this close to the wanted
/// direction (2D dot product).
const DEFLECTION_MIN_DOT: f32 = 0.7;

/// A substep from `start` along `direction` for `amount` units;
/// `validated`: `fits` and `stand` just passed at `start`.
pub(super) struct Step {
    pub(super) start: Vec3,
    pub(super) direction: Vec3,
    pub(super) amount: f32,
    pub(super) validated: bool,
}
impl Step {
    pub fn new(start: Vec3, direction: Vec3, amount: f32, validated: bool) -> Self {
        Self {
            start,
            direction,
            amount,
            validated,
        }
    }
}

/// A substep's outcome.
pub(super) enum Walk {
    /// The plain ground move, completed or blocked.
    Moved(GroundMove, Vec3),
    /// The route changed without advancing the actor.
    Rerouted,
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

/// Try a path change once local steering has exhausted its candidate headings.
pub(super) fn reroute<W: NpcCollisionWorld>(
    world: &W,
    actor: &Actor,
    goal: &mut Goal,
    query: Query<'_>,
    blocked: (&Step, &GroundMove),
    triangulated: &mut bool,
) -> bool {
    let (step, movement) = blocked;
    if movement.end.distance(goal.request.target_feet) <= goal.tolerance {
        return false;
    }
    let blocked = Blocked { step, movement };
    (!std::mem::replace(triangulated, true)
        && prepend_avoidance(world, actor, goal, &blocked, query))
        || skip_near_waypoint(actor, goal, step.start)
}

/// The direction along the hit plane, kept within DEFLECTION_MIN_DOT of `direction`.
pub(super) fn deflection(direction: Vec3, normal: Vec3) -> Option<Vec3> {
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
