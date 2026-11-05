//! The node searches of CAI_TacticalServices: cover from a threat, a line of fire to it.
use super::graph::{nearest_usable, pop_nearest};
use super::{Controller, COVER_NODE_LOCK};
use glam::Vec3;

impl Controller {
    /// CAI_TacticalServices::FindCoverNode: from the usable node nearest `near` (the
    /// NPC), visit nodes nearest-first by straight distance from `near` and return the
    /// feet of the first whose distance lies in [min, max) and that `is_cover` accepts.
    /// A neighbor joins the list only while it is nearer to `near` than 1.5 x its
    /// squared distance to the threat (keeps cover searches from running toward the
    /// threat). Node locks, hint cover activities and climb nodes are not modeled.
    pub fn find_cover_node(
        &mut self,
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
        let now = self.clock;
        self.node_locks.retain(|_, until| *until > now);
        while let Some(node) = pop_nearest(&mut open) {
            let feet = positions[node].expect("usable");
            let distance = feet.distance_squared(near);
            if distance >= min_sq && distance < max_sq && is_cover(feet) {
                // Don't let anyone else use this node for a while.
                self.node_locks.insert(node, now + COVER_NODE_LOCK);
                return Some(feet);
            }
            for &next in &adjacent[node] {
                if visited[next] {
                    continue;
                }
                let p = positions[next].expect("usable");
                let d = p.distance_squared(near);
                // Nodes that aren't ready to use yet are not expanded.
                if d < p.distance_squared(threat) * 1.5 && !self.node_locks.contains_key(&next) {
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
}
