//! The node graph: usable nodes, routes (CAI_Pathfinder) and the door probes along them.
use super::links::{self, MoveCaps, Nav};
use super::{Actor, Goal, GoalBlockReason, NavRestrictions, PathStats, Route, STALE_RECHECK_TIME};
use crate::ai_hull::HullType;
use crate::npc_probe::{self, Hull, NpcCollisionWorld, Query};
use glam::Vec3;
use source_assets::navigation::{Graph, Node};
use std::collections::{BTreeMap, BTreeSet};

/// Usable node feet and each node's usable neighbors.
/// NODE_GROUND and NODE_CLIMB (ai_node.h).
const NODE_GROUND: u8 = 2;
const NODE_CLIMB: u8 = 4;
/// NODE_CLIMB_OFFSET and the climb node info bits (ai_node.h).
const NODE_CLIMB_OFFSET: f32 = 8.;
const CLIMB_OFF_FORWARD: u16 = 1 << 2;
const CLIMB_OFF_LEFT: u16 = 1 << 3;
const CLIMB_OFF_RIGHT: u16 = 1 << 4;
pub(super) type UsableGraph = (Vec<Option<Vec3>>, Vec<Vec<usize>>);
/// NearestNodeToPoint over the usable nodes.
pub(super) fn nearest_usable(positions: &[Option<Vec3>], point: Vec3) -> Option<usize> {
    positions
        .iter()
        .enumerate()
        .filter_map(|(n, p)| p.map(|p| (n, p.distance_squared(point))))
        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
        .map(|(n, _)| n)
}
/// CNodeList::ElementAtHead + RemoveAtHead: the open node with the least key.
pub(super) fn pop_nearest(open: &mut Vec<(usize, f32)>) -> Option<usize> {
    let best = (0..open.len()).min_by(|&a, &b| open[a].1.total_cmp(&open[b].1))?;
    Some(open.swap_remove(best).0)
}
/// CAI_Node::GetPosition(hull): the node origin raised by the hull's offset.
pub(super) fn node_feet(
    graph: &Graph,
    node: usize,
    restrictions: &NavRestrictions,
    hull: HullType,
) -> Option<Vec3> {
    route_feet(graph, node, restrictions, hull, false)
}
/// `node_feet`, with a ladder's climb nodes (their info holds the climb bits) for climbers.
pub(super) fn route_feet(
    graph: &Graph,
    node: usize,
    restrictions: &NavRestrictions,
    hull: HullType,
    climbs: bool,
) -> Option<Vec3> {
    let value = graph.nodes.get(node)?;
    let ground = value.node_type == NODE_GROUND && value.info == 0;
    let climb = climbs && value.node_type == NODE_CLIMB;
    if !(ground || climb) || restrictions.unusable_nodes.contains(&node) {
        return None;
    }
    let origin = if climb {
        climb_origin(value, hull)
    } else {
        value.origin
    };
    let feet = origin + Vec3::Z * *value.hull_z_offsets.get(hull.index())?;
    feet.is_finite().then_some(feet)
}
/// Where a hull stands at a climb node: the node marks the ladder, so the hull stands back
/// from it along the node's yaw by half its length and NODE_CLIMB_OFFSET, ahead of it for a
/// forward dismount and two shifts to the side for a left or right one.
fn climb_origin(node: &Node, hull: HullType) -> Vec3 {
    let bounds = hull.bounds();
    let shift = 0.5 * (bounds.maxs.x - bounds.mins.x) + NODE_CLIMB_OFFSET;
    let yaw = node.yaw.to_radians();
    let forward = Vec3::new(yaw.cos(), yaw.sin(), 0.);
    let side = forward.cross(Vec3::Z);
    node.origin
        + shift
            * if node.info & CLIMB_OFF_FORWARD != 0 {
                forward
            } else if node.info & CLIMB_OFF_LEFT != 0 {
                -2. * side - forward
            } else if node.info & CLIMB_OFF_RIGHT != 0 {
                2. * side - forward
            } else {
                -forward
            }
}
/// The link's accepted move types for the hull include bits_CAP_MOVE_GROUND.
pub(super) fn ground_link(link: &source_assets::navigation::Link, hull: HullType) -> bool {
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
pub(super) struct Destination {
    pub(super) feet: Vec3,
    pub(super) tolerance: f32,
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
            navs: Vec::new(),
        });
    }
    let graph = graph.ok_or(GoalBlockReason::MissingGraph)?;
    let view = actor
        .view_offset
        .ok_or(GoalBlockReason::MissingViewOffset)?;
    let starts = candidates(world, graph, restrictions, actor, start, view, query);
    let ends = candidates(world, graph, restrictions, actor, end, view, query);
    let climbs = actor.caps.climbs();
    let feet_of = |n| route_feet(graph, n, restrictions, actor.hull, climbs).unwrap();
    let start_node = starts
        .into_iter()
        .take(4)
        .find(|n| connector(world, actor, start, feet_of(*n), 0.1, query))
        .ok_or(GoalBlockReason::NoRoute)?;
    let end_node = ends
        .into_iter()
        .take(4)
        .find(|n| connector(world, actor, feet_of(*n), end, tolerance, query))
        .ok_or(GoalBlockReason::NoRoute)?;
    // CAI_Pathfinder::IsLinkUsable: graph data only (flags, accepted move types and
    // unusable nodes, in `search`); a stale-suggested link is unusable until it expires,
    // then IsLinkStillStale rechecks it once per plan (CheckStaleRoute: the local move).
    let Stale { links, now } = stale;
    let mut rechecked = false;
    let nodes = search(
        graph,
        restrictions,
        (actor.hull, &actor.caps),
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
            if !actor.caps.walks(&graph.links[link], actor.hull)
                || connector(world, actor, from, to, 0.1, query)
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
    let mut feet: Vec<_> = nodes.iter().map(|n| feet_of(*n)).collect();
    feet.push(end);
    // Doors along the route are probed as the actor reaches each segment (probe_doors).
    let navs = links::route_navs(graph, &nodes, &actor.caps, actor.hull);
    let mut door_known: Vec<_> = (0..feet.len())
        .map(|i| navs.get(i).is_some_and(|nav| *nav != Nav::Ground))
        .collect();
    door_known[0] = true;
    Ok(Route {
        nodes,
        doors: vec![None; feet.len()],
        feet,
        door_known,
        navs,
    })
}
/// The stale-suggested links and the movement clock, for one plan.
struct Stale<'a> {
    links: &'a mut BTreeMap<usize, f64>,
    now: f64,
}
/// The graph link of the route segment that ends at `feet[next]` (node to node).
pub(super) fn route_link(graph: &Graph, route: &Route, next: usize) -> Option<usize> {
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
pub(super) fn probe_doors<W: NpcCollisionWorld>(
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
pub(super) fn search(
    graph: &Graph,
    restrictions: &NavRestrictions,
    (hull, caps): (HullType, &MoveCaps),
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
    let climbs = caps.climbs();
    let positions: Vec<_> = (0..count)
        .map(|n| route_feet(graph, n, restrictions, hull, climbs))
        .collect();
    let mut adjacent = vec![Vec::new(); count];
    for (index, link) in graph.links.iter().enumerate() {
        if link.source >= count || link.destination >= count {
            return Err(GoalBlockReason::InvalidGraph);
        }
        if caps.link_nav(link, hull).is_some()
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
            let (from, to) = (positions[current].unwrap(), positions[neighbor].unwrap());
            let Some(scale) = caps.link_scale(&graph.links[link], hull, from, to) else {
                continue;
            };
            let Some(edge) = cost(link, from, to).map(|edge| edge * scale) else {
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

/// What a route search needs from the controller, with the counters it updates.
pub(super) struct Planner<'a> {
    pub(super) graph: Option<&'a Graph>,
    pub(super) restrictions: &'a NavRestrictions,
    pub(super) doors: &'a BTreeSet<usize>,
    pub(super) stale: &'a mut BTreeMap<usize, f64>,
    pub(super) now: f64,
    pub(super) stats: &'a mut PathStats,
}
impl Planner<'_> {
    /// One timed route plan for `entity` from `from`.
    pub(super) fn route<W: NpcCollisionWorld>(
        self,
        world: &W,
        actor: &Actor,
        entity: usize,
        from: Vec3,
        destination: Destination,
    ) -> Result<Route, GoalBlockReason> {
        let started = std::time::Instant::now();
        let mut probes = 0;
        let stale = Stale {
            links: self.stale,
            now: self.now,
        };
        let route = plan(
            world,
            self.graph,
            self.restrictions,
            self.doors,
            actor,
            from,
            destination,
            entity,
            stale,
            &mut probes,
        );
        self.stats.plans += 1;
        self.stats.failures += u64::from(route.is_err());
        self.stats.link_probes += probes;
        self.stats.micros += started.elapsed().as_micros() as u64;
        route
    }
}
