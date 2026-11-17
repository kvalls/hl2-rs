//! An NPC's runtime hint group and `ai_changehintgroup` (SDK triggers.cpp CAI_ChangeHintGroup;
//! CAI_BaseNPC::SetHintGroup sets the group and its navigation limiting, and the base
//! OnChangeHintGroup does nothing). The group lives in the scene's keyvalue overrides, where
//! AddOutput's keyvalue form writes, so every reader of an NPC's `hintgroup` sees the change.
use super::{AiRuntime, Scene};
use crate::entities::{glob, number};
use crate::npc::Controller;
use glam::Vec3;
use modkit_core::{Entity, World};

/// An NPC's current hint group (GetHintGroup): what SetHintGroup or AddOutput last gave it,
/// else its `hintgroup` keyvalue.
pub(in crate::ai::runtime) fn group<'a>(world: &'a World, scene: &'a Scene, id: usize) -> &'a str {
    override_of(scene, id, "hintgroup")
        .or_else(|| world.entities.get(id)?.get("hintgroup"))
        .unwrap_or("")
        .trim()
}

fn override_of<'a>(scene: &'a Scene, id: usize, key: &str) -> Option<&'a str> {
    (scene.keyvalue_overrides)
        .range((id, String::new())..(id + 1, String::new()))
        .find(|((_, k), _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// FindQualifiedNPC's search: SearchType 0 by targetname (FindEntityByNameWithin, no distance
/// test with Radius 0), 1 by classname (FindEntityByClassnameWithin, the same radius rule),
/// 2 NPCs whose hint group is SearchName within the sphere (FindEntityInSphere, FLT_MAX for
/// Radius 0). Names match like FindEntityByName (a trailing `*`; `!activator`). Distances are
/// measured from the NPC's origin (the sphere test uses its collision box: not modeled).
struct Search {
    kind: i32,
    name: String,
    origin: Vec3,
    radius: f32,
}
impl Search {
    fn of(changer: &Entity, origin: Vec3) -> Self {
        Self {
            kind: number(changer, "SearchType", 0.) as i32,
            name: changer
                .get("SearchName")
                .unwrap_or("")
                .trim()
                .to_lowercase(),
            origin,
            radius: number(changer, "Radius", 0.),
        }
    }
    fn qualifies(&self, world: &World, scene: &Scene, id: usize, activator: usize) -> bool {
        let (Some(e), Some(state)) = (world.entities.get(id), scene.states.get(id)) else {
            return false;
        };
        if self.radius != 0. && state.origin.distance_squared(self.origin) >= self.radius.powi(2) {
            return false;
        }
        match self.kind {
            1 => glob(&self.name, &e.class().to_lowercase()),
            2 => group(world, scene, id).eq_ignore_ascii_case(&self.name),
            _ if self.name == "!activator" => id == activator,
            _ => (e.get("targetname")).is_some_and(|n| glob(&self.name, &n.to_lowercase())),
        }
    }
}

/// Fires an assault output with `npc` as its activator. CAI_AssaultBehavior::OnHitAssaultPoint
/// gives the NPC the point's `assaultgroup` and ClearAssaultPoint takes it away again (both
/// SetHintGroup without limiting; the SDK notes the clear also drops a group set before).
pub(in crate::ai::runtime) fn fire_assault_output(
    world: &World,
    scene: &mut Scene,
    (point, output): (usize, &'static str),
    npc: usize,
) {
    scene.fire(point, output, npc);
    let Some(e) = world.entities.get(point) else {
        return;
    };
    let group = e.get("assaultgroup").map_or("", str::trim);
    if e.class() != "assault_assaultpoint" || group.is_empty() {
        return;
    }
    let new_group = match output {
        "OnArrival" => group,
        "OnAssaultClear" => "",
        _ => return,
    };
    (scene.keyvalue_overrides).insert((npc, "hintgroup".into()), new_group.into());
    (scene.keyvalue_overrides).insert((npc, "hintlimiting".into()), "0".into());
}

impl AiRuntime {
    /// CAI_ChangeHintGroup::InputActivate: every qualified NPC takes NewHintGroup (empty
    /// clears it) and the changer's `hintlimiting` (SetHintGroup).
    pub(super) fn change_hint_group(
        &self,
        (world, scene, controller): (&World, &mut Scene, &mut Controller),
        changer: usize,
        activator: usize,
    ) {
        let (Some(e), Some(state)) = (world.entities.get(changer), scene.states.get(changer))
        else {
            return;
        };
        let search = Search::of(e, state.origin);
        let new_group = e.get("NewHintGroup").unwrap_or("").trim().to_owned();
        let limiting = number(e, "hintlimiting", 0.) != 0.;
        let targets: Vec<usize> = (self.actors.keys().copied())
            .filter(|&id| !scene.states.get(id).is_some_and(|s| s.killed))
            .filter(|&id| search.qualifies(world, scene, id, activator))
            .collect();
        for id in targets {
            let limit = if limiting { "1" } else { "0" };
            (scene.keyvalue_overrides).insert((id, "hintgroup".into()), new_group.clone());
            (scene.keyvalue_overrides).insert((id, "hintlimiting".into()), limit.into());
            controller.limit_to_hint_group(id, world, &new_group, limiting);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(pairs: &[(&str, &str)]) -> Entity {
        Entity {
            properties: (pairs.iter())
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        }
    }
    fn cop(name: &str, origin: &str, group: &str) -> Entity {
        entity(&[
            ("classname", "npc_metropolice"),
            ("targetname", name),
            ("origin", origin),
            ("hintgroup", group),
        ])
    }

    #[test]
    fn searches_follow_find_qualified_npc() {
        let world = World {
            entities: vec![
                cop("chaser_cop_2", "50 0 0", "old"),
                cop("chaser_cop_4", "3000 0 0", ""),
                entity(&[("classname", "npc_citizen"), ("origin", "100 0 0")]),
            ],
            ..Default::default()
        };
        let scene = Scene::new(&world);
        let search = |kind: &str, name: &str, radius: &str| {
            let changer = entity(&[
                ("SearchType", kind),
                ("SearchName", name),
                ("Radius", radius),
            ]);
            let s = Search::of(&changer, Vec3::ZERO);
            (0..3)
                .filter(|&id| s.qualifies(&world, &scene, id, 2))
                .collect::<Vec<_>>()
        };
        // Names with a trailing '*'; Radius 0 has no distance test, otherwise strictly inside.
        assert_eq!(search("0", "chaser_cop_*", "0"), [0, 1]);
        assert_eq!(search("0", "chaser_cop_*", "3000"), [0]);
        assert_eq!(search("0", "!activator", "0"), [2]);
        assert_eq!(search("1", "npc_metropolice", "0"), [0, 1]);
        // SearchType 2: the NPC's current group, within the sphere (all of it for 0).
        assert_eq!(search("2", "OLD", "0"), [0]);
        assert_eq!(search("2", "old", "1"), Vec::<usize>::new());
    }

    #[test]
    fn set_hint_group_overrides_the_keyvalue() {
        let world = World {
            entities: vec![
                cop("chaser_cop_4", "0 0 0", "start"),
                entity(&[
                    ("classname", "ai_changehintgroup"),
                    ("SearchType", "0"),
                    ("SearchName", "chaser_cop_4"),
                    ("NewHintGroup", "chaser_cop_4_hint"),
                    ("hintlimiting", "1"),
                ]),
            ],
            ..Default::default()
        };
        let mut scene = Scene::new(&world);
        let ai = AiRuntime::new(&world, None);
        assert!(ai.actors.contains_key(&0));
        let mut controller = Controller::new(None, Default::default());
        assert_eq!(group(&world, &scene, 0), "start");
        ai.change_hint_group((&world, &mut scene, &mut controller), 1, usize::MAX);
        assert_eq!(group(&world, &scene, 0), "chaser_cop_4_hint");
        assert_eq!(override_of(&scene, 0, "hintlimiting"), Some("1"));
        assert_eq!(override_of(&scene, 1, "hintgroup"), None);
    }

    #[test]
    fn an_assault_point_sets_its_group_on_arrival_and_clears_it() {
        let world = World {
            entities: vec![
                cop("cop", "0 0 0", "own"),
                entity(&[
                    ("classname", "assault_assaultpoint"),
                    ("assaultgroup", "ap_hints"),
                ]),
                entity(&[("classname", "assault_rallypoint")]),
            ],
            ..Default::default()
        };
        let mut scene = Scene::new(&world);
        fire_assault_output(&world, &mut scene, (2, "OnArrival"), 0);
        assert_eq!(group(&world, &scene, 0), "own");
        fire_assault_output(&world, &mut scene, (1, "OnArrival"), 0);
        assert_eq!(group(&world, &scene, 0), "ap_hints");
        assert_eq!(override_of(&scene, 0, "hintlimiting"), Some("0"));
        fire_assault_output(&world, &mut scene, (1, "OnAssaultClear"), 0);
        assert_eq!(group(&world, &scene, 0), "");
    }

    #[test]
    #[ignore = "requires an installed owned HL2 copy"]
    fn owned_chaser_cops_take_their_limited_group() {
        let game = source_assets::install::discover().unwrap();
        let data = std::fs::read(game.join("hl2/maps/d1_trainstation_06.bsp")).unwrap();
        let world = source_assets::bsp::Bsp::parse(&data)
            .unwrap()
            .world("d1_trainstation_06")
            .unwrap();
        let mut scene = Scene::new(&world);
        let ai = AiRuntime::new(&world, None);
        let mut controller = Controller::new(None, Default::default());
        let named = |name: &str| {
            (world.entities.iter().enumerate())
                .filter(|(_, e)| e.get("targetname") == Some(name))
                .map(|(id, _)| id)
                .collect::<Vec<_>>()
        };
        // Two changers share the name, one for chaser_cop_4 and one for chaser_cop_5.
        // The cops are point_template members: their template has spawned them by the time
        // their path_corner fires the changer.
        let cops = ["chaser_cop_4", "chaser_cop_5"].map(|cop| named(cop)[0]);
        assert!(cops.iter().all(|&id| scene.states[id].killed));
        for id in cops {
            scene.states[id].killed = false;
        }
        for changer in named("chaser_cop_4_changehint") {
            ai.change_hint_group((&world, &mut scene, &mut controller), changer, usize::MAX);
        }
        for id in cops {
            assert_eq!(group(&world, &scene, id), "chaser_cop_4_hint");
            assert_eq!(override_of(&scene, id, "hintlimiting"), Some("1"));
        }
        assert_eq!(group(&world, &scene, named("chaser_cop_1")[0]), "");
    }
}
