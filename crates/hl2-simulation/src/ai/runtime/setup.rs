//! What `AiRuntime::new` reads from the map and the game's scripts.
use super::super::actbusy::HintCandidate;
use super::super::police::PoliceGoal;
use super::super::schedule::Schedules;
use super::yaw_keyvalue;
use glam::Vec3;
use modkit_core::{Entity, World};
use source_assets::{keyvalues, vpk::Vfs};
use std::collections::BTreeMap;

/// The info_node_hint entities the actbusy search can use.
pub(super) fn busy_hints(world: &World) -> Vec<HintCandidate> {
    world
        .entities
        .iter()
        .enumerate()
        .filter(|(_, e)| e.class() == "info_node_hint")
        .map(|(id, e)| {
            let int = |k: &str| e.get(k).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
            HintCandidate {
                entity: id,
                origin: e.origin(),
                yaw: yaw_keyvalue(e),
                hint_type: int("hinttype"),
                group: e.get("Group").unwrap_or("").trim().to_owned(),
                activity: e.get("hintactivity").unwrap_or("").trim().to_owned(),
                disabled: int("StartHintDisabled") != 0,
                min_state: int("MinimumState"),
                max_state: int("MaximumState"),
            }
        })
        .collect()
}

/// Every class's schedules.
pub(super) fn class_schedules() -> Schedules {
    use crate::ai::{actbusy, bird, headcrab, humanoid, metropolice, police, scanner, zombie};
    let mut schedules = Schedules::default();
    police::add_schedules(&mut schedules);
    metropolice::add_schedules(&mut schedules);
    humanoid::add_schedules(&mut schedules);
    zombie::add_schedules(&mut schedules);
    headcrab::add_schedules(&mut schedules);
    bird::add_schedules(&mut schedules);
    scanner::add_schedules(&mut schedules);
    super::super::manhack::add_schedules(&mut schedules);
    actbusy::add_schedules(&mut schedules);
    schedules
}

/// Each ai_goal_actbusy's search range.
pub(super) fn actbusy_ranges(world: &World) -> BTreeMap<usize, f32> {
    world
        .entities
        .iter()
        .enumerate()
        .filter(|(_, e)| e.class() == "ai_goal_actbusy")
        .map(|(id, e)| {
            let range = e.get("busysearchrange").and_then(|v| v.trim().parse().ok());
            (id, range.unwrap_or(2048.))
        })
        .collect()
}

/// The NPC damage values of cfg/skill.cfg.
#[derive(Default)]
pub(super) struct SkillDamage {
    pub stunstick: Option<f32>,
    pub combine_kick: f32,
    pub combine_guard_kick: f32,
    pub zombie_slash: f32,
    pub zombie_poison_spit: f32,
    pub headcrab_melee: f32,
}

pub(super) fn skill_damage(vfs: &Vfs) -> SkillDamage {
    let text = vfs.read("cfg/skill.cfg").ok().flatten().unwrap_or_default();
    let Ok(tokens) = keyvalues::tokens(&String::from_utf8_lossy(&text)) else {
        return SkillDamage::default();
    };
    let value = |name: &str| {
        tokens
            .windows(2)
            .find(|p| p[0].eq_ignore_ascii_case(name))
            .and_then(|p| p[1].parse::<f32>().ok())
    };
    let stunstick = value("sk_npc_dmg_stunstick");
    if let Some(damage) = stunstick {
        super::super::metropolice::STUNSTICK_DAMAGE.with(|d| d.set(damage));
    }
    SkillDamage {
        stunstick,
        combine_kick: value("sk_combine_s_kick").unwrap_or(0.),
        combine_guard_kick: value("sk_combine_guard_kick").unwrap_or(0.),
        zombie_slash: value("sk_zombie_dmg_one_slash").unwrap_or(0.),
        zombie_poison_spit: value("sk_zombie_poison_dmg_spit").unwrap_or(0.),
        headcrab_melee: value("sk_headcrab_melee_dmg").unwrap_or(0.),
    }
}

/// The stunstick's hit and miss sounds from its weapon script.
pub(super) fn stunstick_sounds(vfs: &Vfs) -> (Option<String>, Option<String>) {
    let Ok(Some(script)) = vfs.read("scripts/weapon_stunstick.txt") else {
        return (None, None);
    };
    let Ok(entries) = keyvalues::parse(&String::from_utf8_lossy(&script)) else {
        return (None, None);
    };
    let sounds = entries.first().and_then(|r| r.get("SoundData"));
    let get = |key: &str| {
        sounds
            .and_then(|s| s.get(key))
            .and_then(|e| e.text())
            .map(str::to_owned)
    };
    (get("melee_hit"), get("melee_miss"))
}

/// An ai_goal_police entity's goal.
pub(super) fn police_goal(id: usize, e: &Entity) -> PoliceGoal {
    let number = |key: &str| e.get(key).and_then(|v| v.parse().ok());
    let angles = e.get("angles").and_then(modkit_core::parse_vec3);
    PoliceGoal {
        entity: id,
        origin: e.origin(),
        yaw: angles.unwrap_or(Vec3::ZERO).y,
        radius: number("PoliceRadius").unwrap_or(0.),
        target: None,
        spawnflags: e
            .get("spawnflags")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        override_knockout: false,
    }
}
