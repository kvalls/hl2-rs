//! Hint-group navigation limiting (SDK ai_basenpc_movement.cpp CAI_BaseNPC::IsUnusableNode):
//! an NPC with `hintlimiting` and a non-empty `hintgroup` may only use graph nodes that are
//! hints of its group. The pathfinder skips other nodes when they leave the open set and
//! refuses one as the destination (ai_pathfinder.cpp), and the nearest-node choice passes them
//! over (ai_network.cpp), so routes never start, end or pass through them. The group is read at
//! spawn and changed by SetHintGroup (ai_changehintgroup); npc_antlion's own override is not
//! modeled.
use super::Controller;
use crate::entities::number;
use modkit_core::World;
use std::collections::BTreeSet;

impl Controller {
    /// The spawn limit of `entity` from its `hintgroup` and `hintlimiting` keyvalues.
    pub fn set_hint_limit(&mut self, entity: usize, world: &World) {
        let Some(e) = world.entities.get(entity) else {
            return;
        };
        let group = e.get("hintgroup").map_or("", str::trim);
        let limited = number(e, "hintlimiting", 0.) != 0.;
        self.limit_to_hint_group(entity, world, group, limited);
    }
    /// The nodes `entity` may use: all (`None`) unless it limits itself to a non-empty group.
    pub fn limit_to_hint_group(
        &mut self,
        entity: usize,
        world: &World,
        group: &str,
        limited: bool,
    ) {
        let limited = limited && !group.is_empty();
        let ids = self.graph.as_ref().map(|g| g.hammer_ids.as_slice());
        let nodes = limited.then(|| hint_nodes(world, ids, group));
        if let Some(actor) = self.actors.get_mut(&entity) {
            actor.hint_nodes = nodes;
        }
    }
}

/// The graph nodes whose hint (info_node_hint / info_node_air_hint, `nodeid` the node's editor
/// id) belongs to `group`.
fn hint_nodes(world: &World, hammer_ids: Option<&[i32]>, group: &str) -> BTreeSet<usize> {
    let Some(hammer_ids) = hammer_ids else {
        return BTreeSet::new();
    };
    (world.entities.iter())
        .filter(|e| matches!(e.class(), "info_node_hint" | "info_node_air_hint"))
        .filter(|e| e.get("hintgroup").map(str::trim) == Some(group))
        .filter_map(|e| e.get("nodeid")?.trim().parse::<i32>().ok())
        .filter_map(|id| hammer_ids.iter().position(|h| *h == id))
        .collect()
}

/// Whether `actor` may use graph node `node` (IsUnusableNode's negation).
pub(super) fn usable(actor: &super::Actor, node: usize) -> bool {
    actor
        .hint_nodes
        .as_ref()
        .is_none_or(|nodes| nodes.contains(&node))
}

#[cfg(test)]
mod tests {
    use super::*;
    use modkit_core::Entity;

    fn entity(pairs: &[(&str, &str)]) -> Entity {
        Entity {
            properties: (pairs.iter())
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        }
    }

    #[test]
    fn hint_limiting_keeps_only_the_hints_of_the_npcs_group() {
        let ids = [10, 11, 12, 13];
        let world = World {
            entities: vec![
                entity(&[
                    ("classname", "info_node_hint"),
                    ("nodeid", "11"),
                    ("hintgroup", "canal_a"),
                ]),
                entity(&[
                    ("classname", "info_node_air_hint"),
                    ("nodeid", "13"),
                    ("hintgroup", "canal_a"),
                ]),
                entity(&[
                    ("classname", "info_node_hint"),
                    ("nodeid", "12"),
                    ("hintgroup", "canal_b"),
                ]),
            ],
            ..World::default()
        };
        assert_eq!(
            hint_nodes(&world, Some(&ids), "canal_a"),
            BTreeSet::from([1, 3])
        );
        assert!(hint_nodes(&world, None, "canal_a").is_empty());
    }
}
