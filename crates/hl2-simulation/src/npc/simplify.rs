//! CAI_Navigator::SimplifyPath for ground routes (Source SDK `game/server/ai_navigator.cpp`
//! SimplifyPath, SimplifyPathForward, SimplifyPathInsertSimplification).
use super::{links, Actor, Goal};
use crate::npc_probe::{self, NpcCollisionWorld, Query};
use glam::Vec3;

/// ROUTE_SIMPLIFY_TIME_DELAY (in the PVS, AIStrongOpt off).
pub(super) const ROUTE_SIMPLIFY_DELAY: f32 = 0.5;
/// MIN_ANGLE_COS_SIMPLIFY: 40 degrees left or right of the current heading.
const MIN_ANGLE_COS_SIMPLIFY: f32 = 0.766;
/// CAI_Navigator::SimplifyPathForward with the full scan parameters (32 ft out, points
/// within 12 ft, 3 ft increments, four samples): head straight for the farthest
/// reachable point along the path ahead, else skip to the next waypoint when it is
/// reachable and nearer than the scan distance. Points must lie within 40 degrees of the
/// current heading unless `forced` (the first simplification of a path).
pub(super) fn simplify_path_forward<W: NpcCollisionWorld>(
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
    let next = goal.next;
    let points = &goal.route.feet[..links::ground_end(&goal.route, next)];
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
