//! Which materials a loaded map draws: the world's surfaces and overlays, its visible brush entities and
//! entity models.
use modkit_core::World;
use std::collections::BTreeSet;

pub(crate) fn visible_entity(world: &World, id: usize) -> bool {
    world.entities.get(id).is_some_and(|entity| {
        !entity.class().starts_with("trigger_") && !entity.class().starts_with("func_areaportal")
    })
}

pub(super) fn rendered_material_names(world: &World) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut append = |surfaces: &[modkit_core::Surface]| {
        names.extend(
            surfaces
                .iter()
                .filter(|surface| !surface.indices.is_empty())
                .map(|surface| surface.material.clone()),
        );
    };
    // Includes displacements and already-transformed static props. terrain is a
    // duplicate collision representation, and unused model_assets are not draws.
    append(&world.surfaces);
    append(&world.overlays);
    for (id, entity) in world.entities.iter().enumerate() {
        if !visible_entity(world, id) {
            continue;
        }
        if let Some(brush) = entity
            .get("model")
            .and_then(|name| name.strip_prefix('*'))
            .and_then(|id| id.parse::<usize>().ok())
            .and_then(|id| world.brush_models.iter().find(|model| model.id == id))
        {
            append(&brush.surfaces);
        }
    }
    for instance in &world.model_instances {
        if !instance.entity.is_some_and(|id| visible_entity(world, id)) {
            continue;
        }
        if let Some(surfaces) = world.model_assets.get(&instance.asset_key()) {
            append(surfaces);
        }
    }
    names
}

pub(super) fn map_material_names(
    world: &World,
    effects: &crate::effects::PreparedEffects,
    weapon_models: &[String],
    gameplay: &crate::gameplay::Gameplay,
) -> BTreeSet<String> {
    let mut names = rendered_material_names(world);
    names.insert(crate::shadows::BLOB_MATERIAL.to_owned());
    if gameplay.scene.commentary.enabled {
        names.insert(hl2_simulation::commentary::SPEECH_MAT_USED.to_owned());
    }
    names.extend(
        effects
            .models
            .values()
            .flatten()
            .map(|s| s.material.clone()),
    );
    for model in weapon_models {
        if let Some(surfaces) = world
            .model_assets
            .get(&format!("{}#0", model.to_lowercase()))
        {
            names.extend(surfaces.iter().map(|s| s.material.clone()));
        }
    }
    for weapon in gameplay.weapons.values() {
        for skin in 0..2 {
            if let Some(surfaces) = world
                .model_assets
                .get(&format!("{}#{skin}", weapon.viewmodel.to_lowercase()))
            {
                names.extend(surfaces.iter().map(|s| s.material.clone()));
            }
        }
    }
    names
}
