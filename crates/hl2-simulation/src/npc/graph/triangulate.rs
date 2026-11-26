//! A local detour around an obstacle (SDK ai_pathfinder.cpp BuildComplexRoute's
//! triangulation: BuildTriangulationRoute, Triangulate, TestTriangulationRoute). When the
//! straight walk to a goal within MAX_LOCAL_NAV_DIST_GROUND is blocked, one apex beside the
//! obstacle is tried, start -> apex -> goal, before the node graph.

use super::{connector, Actor, Route};
use crate::npc_probe::{self, NpcCollisionWorld, Query};
use glam::Vec3;

/// MAX_LOCAL_NAV_DIST_GROUND without strong optimizations (50 feet): farther goals do not
/// try local routes.
pub(super) const MAX_LOCAL_DIST: f32 = 50. * 12.;
/// MAX_TRIAGULATION_DIST (32 feet). Over it the SDK clips the walk to `forward * 32 ft`
/// without the start offset and requires a move from there to the goal, which fails: longer
/// walks never triangulate.
const MAX_DIST: f32 = 32. * 12.;

/// BuildLocalRoute for a ground goal: the straight walk, else a triangulated detour.
pub(super) fn local_route<W: NpcCollisionWorld>(
    world: &W,
    actor: &Actor,
    (start, end): (Vec3, Vec3),
    tolerance: f32,
    query: Query<'_>,
) -> Option<Route> {
    if connector(world, actor, start, end, tolerance, query) {
        return Some(Route::direct(end));
    }
    let apex = apex(world, actor, (start, end), tolerance, query)?;
    // The apex waypoint is a detour (bits_WP_TO_DETOUR), not a graph node.
    Some(Route {
        nodes: Vec::new(),
        feet: vec![apex, end],
        doors: vec![None, None],
        door_known: vec![true, true],
        navs: Vec::new(),
        detour_end: 1,
    })
}

/// The detour apex for a blocked straight walk from `start` to `end` (goal tolerance
/// `tolerance`), or None. Two rounds of two apexes: half the hull's length to each side of
/// the point half a hull past the obstruction (the far side first), then a hull's length
/// farther out; an apex whose first leg is blocked is pulled back to the obstruction.
pub(in crate::npc) fn apex<W: NpcCollisionWorld>(
    world: &W,
    actor: &Actor,
    (start, end): (Vec3, Vec3),
    tolerance: f32,
    query: Query<'_>,
) -> Option<Vec3> {
    let full = end - start;
    let length = full.length();
    if !(1e-3..=MAX_DIST.min(MAX_LOCAL_DIST)).contains(&length) {
        return None;
    }
    let forward = full / length;
    let blocked = npc_probe::ground_move(world, start, end, actor.ground, query);
    let obstructed = (blocked.end - start).dot(forward).max(0.);
    let side = if 1. - forward.z.abs() > 1e-3 {
        forward.cross(Vec3::Z).normalize()
    } else {
        Vec3::X
    };
    let hull = actor.ground.hull;
    let half = 0.5 * (hull.maxs.x - hull.mins.x);
    let along = (obstructed + half).min(length);
    let center = start + forward * along;
    let mut apexes = [center - side * half, center + side * half];
    let mut distances = [along; 2];
    for _ in 0..2 {
        for j in [1, 0] {
            let first = npc_probe::ground_move(world, start, apexes[j], actor.ground, query);
            if first.completed && connector(world, actor, apexes[j], end, tolerance, query) {
                return Some(apexes[j]);
            }
            if !first.completed {
                let reached = (first.end - start).dot(forward);
                if distances[j] > reached {
                    apexes[j] += forward * (reached - distances[j]);
                    distances[j] = reached;
                }
            }
            let out = if j == 1 { side } else { -side };
            apexes[j] += out * (2. * half);
        }
    }
    None
}
