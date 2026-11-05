//! Routes through the air (DESIGN 14o session 46): the graph's links that accept flight for a hull.
//! A flight goes straight when nothing is in the way; otherwise it enters the network at a node it
//! can reach, follows the links and leaves by a node that can reach the goal. No way through means
//! no route, and the caller stays on the ground. Local detours around what the graph does not know
//! are not modeled here: a flight's own probe slides along those.
use super::graph::pop_nearest;
use super::Controller;
use crate::ai_hull::HullType;
use glam::Vec3;
use source_assets::navigation::Graph;

/// bits_CAP_MOVE_FLY in a link's types.
const MOVE_FLY: u8 = 4;
/// Nodes tried for the first and for the last leg, nearest first.
const LEG_TRIES: usize = 12;

/// The nodes a hull can fly between.
#[derive(Clone, Debug, Default)]
pub(super) struct AirGraph {
    positions: Vec<Vec3>,
    neighbors: Vec<Vec<usize>>,
}
impl AirGraph {
    fn build(graph: &Graph, hull: HullType) -> Self {
        let mut air = Self::default();
        let mut index = vec![None; graph.nodes.len()];
        let mut node = |air: &mut Self, n: usize| -> Option<usize> {
            let slot = index.get_mut(n)?;
            if slot.is_none() {
                let value = graph.nodes.get(n)?;
                let lift = value
                    .hull_z_offsets
                    .get(hull.index())
                    .copied()
                    .unwrap_or(0.);
                *slot = Some(air.positions.len());
                air.positions.push(value.origin + Vec3::Z * lift);
                air.neighbors.push(Vec::new());
            }
            *slot
        };
        for link in &graph.links {
            let types = link.accepted_move_types.get(hull.index()).copied();
            if types.unwrap_or(0) & MOVE_FLY == 0 {
                continue;
            }
            if let (Some(a), Some(b)) = (
                node(&mut air, link.source),
                node(&mut air, link.destination),
            ) {
                air.neighbors[a].push(b);
                air.neighbors[b].push(a);
            }
        }
        air
    }
    /// The nearest nodes with a clear leg to or from `point`: (node, length of the leg).
    fn legs(&self, point: Vec3, clear: impl Fn(Vec3) -> bool) -> Vec<(usize, f32)> {
        let mut nodes: Vec<(usize, f32)> = self
            .positions
            .iter()
            .enumerate()
            .map(|(n, p)| (n, p.distance(point)))
            .collect();
        nodes.sort_by(|a, b| a.1.total_cmp(&b.1));
        nodes.truncate(LEG_TRIES);
        nodes.retain(|(n, _)| clear(self.positions[*n]));
        nodes
    }
    /// The cheapest way from any start to any end, each with the cost of its leg.
    fn shortest(&self, starts: &[(usize, f32)], ends: &[(usize, f32)]) -> Option<Vec<Vec3>> {
        let mut cost = vec![f32::MAX; self.positions.len()];
        let mut came_from = vec![None; self.positions.len()];
        let mut open = Vec::new();
        for &(node, leg) in starts {
            cost[node] = leg;
            open.push((node, leg));
        }
        while let Some(node) = pop_nearest(&mut open) {
            for &next in &self.neighbors[node] {
                let step = cost[node] + self.positions[node].distance(self.positions[next]);
                if step < cost[next] {
                    cost[next] = step;
                    came_from[next] = Some(node);
                    open.push((next, step));
                }
            }
        }
        let (mut at, _) = ends
            .iter()
            .filter(|(n, _)| cost[*n] < f32::MAX)
            .map(|&(n, leg)| (n, cost[n] + leg))
            .min_by(|a, b| a.1.total_cmp(&b.1))?;
        let mut way = vec![self.positions[at]];
        while let Some(previous) = came_from[at] {
            way.push(self.positions[previous]);
            at = previous;
        }
        way.reverse();
        Some(way)
    }
}

impl Controller {
    /// The legs of a flight from `from` to `to`, the last being `to`. `clear` says whether a body
    /// can fly a straight leg. None: there is no way.
    pub fn fly_route(
        &mut self,
        hull: HullType,
        from: Vec3,
        to: Vec3,
        clear: impl Fn(Vec3, Vec3) -> bool,
    ) -> Option<Vec<Vec3>> {
        if clear(from, to) {
            return Some(vec![to]);
        }
        let key = hull.index();
        if !self.air.contains_key(&key) {
            let built = AirGraph::build(self.graph()?, hull);
            self.air.insert(key, built);
        }
        let air = self.air.get(&key)?;
        let starts = air.legs(from, |node| clear(from, node));
        let ends = air.legs(to, |node| clear(node, to));
        let mut way = air.shortest(&starts, &ends)?;
        way.push(to);
        Some(way)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::npc::NavRestrictions;
    use source_assets::navigation::{Link, Node};

    const WEST: Vec3 = Vec3::new(-100., 0., 0.);
    const EAST: Vec3 = Vec3::new(100., 0., 0.);

    fn node(x: f32, y: f32) -> Node {
        Node {
            origin: Vec3::new(x, y, 0.),
            yaw: 0.,
            hull_z_offsets: [0.; 10],
            node_type: 3,
            info: 0,
            zone: 0,
        }
    }
    fn link(source: usize, destination: usize, types: u8) -> Link {
        Link {
            source,
            destination,
            accepted_move_types: [types; 10],
        }
    }
    fn controller(nodes: Vec<Node>, links: Vec<Link>) -> Controller {
        let graph = Graph {
            version: 37,
            map_revision: 0,
            hammer_ids: vec![0; nodes.len()],
            nodes,
            links,
            trailing_bytes: Vec::new(),
        };
        Controller::new(Some(graph), NavRestrictions::default())
    }
    /// A wall along the y axis, up to y = 150, between west and east.
    fn past_the_wall(a: Vec3, b: Vec3) -> bool {
        let crosses = a.x * b.x < 0.;
        let at = a.y + (b.y - a.y) * (a.x / (a.x - b.x));
        !crosses || at > 150.
    }
    fn over_the_wall() -> Vec<Node> {
        vec![node(-60., 200.), node(60., 200.)]
    }

    #[test]
    fn a_clear_flight_goes_straight_with_no_graph_at_all() {
        let mut c = Controller::new(None, NavRestrictions::default());
        let way = c.fly_route(HullType::Tiny, WEST, EAST, |_, _| true);
        assert_eq!(way, Some(vec![EAST]));
    }

    #[test]
    fn a_wall_is_passed_by_the_nodes_over_it() {
        let mut c = controller(over_the_wall(), vec![link(0, 1, MOVE_FLY)]);
        let way = c
            .fly_route(HullType::Tiny, WEST, EAST, past_the_wall)
            .unwrap();
        let expected = [Vec3::new(-60., 200., 0.), Vec3::new(60., 200., 0.), EAST];
        assert_eq!(way, expected);
    }

    #[test]
    fn a_ground_link_is_no_way_through_the_air() {
        let mut c = controller(over_the_wall(), vec![link(0, 1, 1)]);
        assert_eq!(c.fly_route(HullType::Tiny, WEST, EAST, past_the_wall), None);
    }

    #[test]
    fn no_graph_and_a_wall_is_no_route() {
        let mut c = Controller::new(None, NavRestrictions::default());
        assert_eq!(c.fly_route(HullType::Tiny, WEST, EAST, past_the_wall), None);
    }

    #[test]
    fn nodes_with_no_clear_leg_are_not_entered() {
        let mut c = controller(over_the_wall(), vec![link(0, 1, MOVE_FLY)]);
        let sealed = |a: Vec3, b: Vec3| past_the_wall(a, b) && a != WEST;
        assert_eq!(c.fly_route(HullType::Tiny, WEST, EAST, sealed), None);
    }

    #[test]
    fn the_shorter_of_two_ways_is_taken() {
        let nodes = vec![
            node(-60., 200.),
            node(60., 200.),
            node(-60., 900.),
            node(60., 900.),
        ];
        let links = vec![link(2, 3, MOVE_FLY), link(0, 1, MOVE_FLY)];
        let mut c = controller(nodes, links);
        let way = c
            .fly_route(HullType::Tiny, WEST, EAST, past_the_wall)
            .unwrap();
        assert!(way.iter().all(|p| p.y <= 200.), "{way:?}");
    }

    #[test]
    fn a_route_needs_links_that_accept_the_hull() {
        let mut only_human = link(0, 1, 0);
        only_human.accepted_move_types[HullType::Human.index()] = MOVE_FLY;
        let mut c = controller(over_the_wall(), vec![only_human]);
        assert_eq!(c.fly_route(HullType::Tiny, WEST, EAST, past_the_wall), None);
        assert!(c
            .fly_route(HullType::Human, WEST, EAST, past_the_wall)
            .is_some());
    }

    #[test]
    #[ignore = "requires an installed owned HL2 copy; reads the graph of d1_town_01"]
    fn owned_town_graph_has_fly_links_for_the_tiny_hull() {
        let game = source_assets::install::discover().unwrap();
        let vfs = source_assets::vpk::Vfs::mount(&game).unwrap();
        let bytes = vfs.read("maps/graphs/d1_town_01.ain").unwrap().unwrap();
        let revision = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let graph = Graph::parse(&bytes, revision).unwrap();
        let air = AirGraph::build(&graph, HullType::Tiny);
        let air_nodes = graph.nodes.iter().filter(|n| n.node_type == 3).count();
        let links: usize = air.neighbors.iter().map(Vec::len).sum();
        println!(
            "{air_nodes} air nodes, {} flown, {links} link ends",
            air.positions.len()
        );
        assert!(air_nodes > 0 && links > 0);
    }
}
