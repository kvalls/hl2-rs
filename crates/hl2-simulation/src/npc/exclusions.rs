//! What an actor's moves and probes pass through: the other headcrabs
//! (HL2COLLISION_GROUP_HEADCRAB, CHalfLife2::ShouldCollide) and the entities
//! CTraceFilterNav skips for it: func_brush entities that exclude its class (CFuncBrush
//! `excludednpc` / `invert_exclusion` through CAI_MoveProbe::ShouldBrushBeIgnored, which base
//! HL2 matches against the NPC's classname only; NPC_CheckBrushExclude does the same for its
//! collisions) and the targets of a logic_navigation set to Ignore (CLogicNavigation,
//! SetNavIgnore: IsNavIgnored). Both are read at map load: the func_brush SetExcluded/SetInvert
//! and logic_navigation TurnOn/TurnOff/Toggle inputs and targets spawned later are not modeled
//! (every installed logic_navigation starts on and none receives an input).
use super::{Actor, Controller};
use modkit_core::World;
use std::collections::BTreeSet;

impl Controller {
    /// HL2COLLISION_GROUP_HEADCRAB membership (the headcrab classes).
    pub fn set_headcrab(&mut self, entity: usize, on: bool) {
        if on {
            self.headcrabs.insert(entity);
        } else {
            self.headcrabs.remove(&entity);
        }
    }
    /// The entities this actor's probes pass (`nav_ignored_by`).
    pub fn set_nav_ignored(&mut self, entity: usize, world: &World, class: &str) {
        if let Some(actor) = self.actors.get_mut(&entity) {
            actor.nav_ignored = nav_ignored_by(world, class);
        }
    }
}

/// The entities a move of `entity` passes: itself (every crab for a crab) and its
/// nav-ignored entities.
pub(super) fn excluded_for(
    crabs: &BTreeSet<usize>,
    actor: Option<&Actor>,
    entity: usize,
) -> Vec<usize> {
    let mut excluded = if crabs.contains(&entity) {
        crabs.iter().copied().collect()
    } else {
        vec![entity]
    };
    if let Some(actor) = actor {
        excluded.extend_from_slice(&actor.nav_ignored);
    }
    excluded
}
/// The planner's probes: the actor itself and its nav-ignored entities.
pub(super) fn self_and_ignored(actor: &Actor, entity: usize) -> Vec<usize> {
    std::iter::once(entity)
        .chain(actor.nav_ignored.iter().copied())
        .collect()
}

/// Every entity CTraceFilterNav skips for an NPC of `class`: the func_brush entities whose
/// `excludednpc` is `class` (or, with `invert_exclusion`, is not; an empty name matches no
/// class) and the targets of a logic_navigation that starts on (SF_NAV_START_ON) with navprop
/// Ignore.
pub fn nav_ignored_by(world: &World, class: &str) -> Vec<usize> {
    let ignore_targets: Vec<&str> = world
        .entities
        .iter()
        .filter(|e| e.class() == "logic_navigation")
        .filter(|e| e.get("navprop").is_some_and(|v| v.trim() == "Ignore"))
        .filter(|e| {
            let flags: u32 = e
                .get("spawnflags")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            flags & 1 != 0
        })
        .filter_map(|e| e.get("target").map(str::trim).filter(|t| !t.is_empty()))
        .collect();
    world
        .entities
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            let name = e.get("targetname").map_or("", str::trim);
            let targeted = !name.is_empty()
                && ignore_targets
                    .iter()
                    .any(|t| crate::entities::glob(t, name));
            targeted || (e.class() == "func_brush" && brush_excludes(e, class))
        })
        .map(|(id, _)| id)
        .collect()
}
/// CAI_MoveProbe::ShouldBrushBeIgnored for one func_brush.
fn brush_excludes(brush: &modkit_core::Entity, class: &str) -> bool {
    let name = brush.get("excludednpc").map_or("", str::trim);
    let matches = !name.is_empty() && name.eq_ignore_ascii_case(class);
    let inverted = brush
        .get("invert_exclusion")
        .is_some_and(|v| v.trim() == "1");
    matches != inverted
}

#[cfg(test)]
mod tests {
    use super::nav_ignored_by;
    use modkit_core::{Entity, World};

    fn entity(pairs: &[(&str, &str)]) -> Entity {
        Entity {
            properties: pairs
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
        }
    }
    fn brush(excluded: &str, invert: &str) -> Entity {
        entity(&[
            ("classname", "func_brush"),
            ("excludednpc", excluded),
            ("invert_exclusion", invert),
        ])
    }

    #[test]
    fn func_brush_exclusions_and_logic_navigation_ignore() {
        // d1_trainstation_01's turnstile: its frame (excludednpc npc_citizen) and its arms
        // (func_physbox turnstyle_1, the target of a logic_navigation set to Ignore).
        let navigation = |flags| {
            entity(&[
                ("classname", "logic_navigation"),
                ("navprop", "Ignore"),
                ("spawnflags", flags),
                ("target", "turnstyle_*"),
            ])
        };
        let world = World {
            entities: vec![
                brush("npc_citizen", "0"),
                brush("npc_metropolice", "1"),
                brush("", "0"),
                brush("", "1"),
                entity(&[("classname", "func_physbox"), ("targetname", "turnstyle_1")]),
                navigation("1"),
            ],
            ..Default::default()
        };
        assert_eq!(nav_ignored_by(&world, "npc_citizen"), [0, 1, 3, 4]);
        assert_eq!(nav_ignored_by(&world, "npc_metropolice"), [3, 4]);
        // A logic_navigation that does not start on ignores nothing.
        let mut off = world.clone();
        off.entities[5] = navigation("0");
        assert_eq!(nav_ignored_by(&off, "npc_metropolice"), [3]);
    }
}
