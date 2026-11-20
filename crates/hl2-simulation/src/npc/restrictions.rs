//! Graph links and nodes no actor may use: unusable links, CAI_DynamicLink states.
use glam::Vec3;
use modkit_core::World;
use source_assets::navigation::Graph;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default)]
pub struct NavRestrictions {
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
    /// Hint nodes stay routable for every NPC (an NPC with `hintlimiting` is limited per
    /// actor: `hint_limits`). Dynamic
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
