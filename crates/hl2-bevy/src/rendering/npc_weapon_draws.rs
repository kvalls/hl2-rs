//! Which weapon world models each NPC gets a draw for at map load (DESIGN 14w step 4).
use crate::assets::visible_entity;
use modkit_core::World;
use std::collections::BTreeSet;

/// (owner entity, model asset key, owner scale) for every NPC weapon draw: the active
/// weapon's world model, every weapon a pooled F4 spawn's class can use, and every weapon
/// the map's outputs give the NPC by name (GiveWeapon, created at runtime in Source with a
/// late precache). Each draw shows only while its weapon is the active one (NpcWeaponDraw).
pub(super) fn npc_weapon_draws(
    world: &World,
    ai: &hl2_simulation::ai::runtime::AiRuntime,
) -> Vec<(usize, String, f32)> {
    let usable_weapons = crate::gameplay::spawn_usable_weapons(world, ai);
    let given = hl2_simulation::npc_weapons::given_weapons(world);
    let mut out = Vec::new();
    for instance in &world.model_instances {
        let Some(id) = instance.entity.filter(|id| visible_entity(world, *id)) else {
            continue;
        };
        let mut models: BTreeSet<String> = ai.weapon_world_model(id).into_iter().collect();
        let entity = &world.entities[id];
        let mut classes: Vec<&String> = Vec::new();
        if let Some(label) = entity.get(hl2_simulation::ai::spawn::POOL_KEY) {
            classes.extend(usable_weapons.get(label).into_iter().flatten());
        }
        if let Some(name) = entity.get("targetname") {
            classes.extend(given.get(&name.to_ascii_lowercase()).into_iter().flatten());
        }
        models.extend(classes.into_iter().filter_map(|w| ai.weapon_model(w)));
        out.extend(
            models
                .into_iter()
                .map(|model| (id, format!("{model}#0"), instance.scale)),
        );
    }
    out
}
