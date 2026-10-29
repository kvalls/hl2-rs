//! Host presentation and precaching for breakable prop gib models.
use super::{Effects, ModelPart};
use crate::{gameplay::Gameplay, rendering};
use bevy::{
    asset::Assets,
    prelude::{
        AlphaMode, Commands, Component, Mesh3d, MeshMaterial3d, Query, Res, ResMut, Transform,
        Visibility, With,
    },
};

/// Gib models are precached with their breakable props (PrecacheGibsForModel), in the
/// skins they can take: the parent's (BreakModelCreateSingle) or a chunk set's
/// breakable_skin; propdata chunk models only where a prop uses them.
pub fn gib_models(world: &modkit_core::World) -> std::collections::BTreeSet<(String, i32)> {
    let mut gibs = std::collections::BTreeSet::new();
    for instance in &world.model_instances {
        let key = instance.asset_key();
        if let Some(list) = world.model_breaks.get(&key) {
            for gib in list.iter().filter(|gib| !gib.ragdoll) {
                gibs.insert((gib.model.clone(), instance.skin as i32));
            }
        } else if let Some(data) = world.model_prop_data.get(&key) {
            let set = data.breakable_model.as_deref().and_then(|name| {
                world
                    .breakable_chunks
                    .iter()
                    .find(|(set, _)| set.starts_with(name))
            });
            if let Some((_, models)) = set.filter(|_| data.health > 0. && data.breakable_count > 0)
            {
                for model in models {
                    gibs.insert((model.clone(), data.breakable_skin));
                }
            }
        }
    }
    gibs
}

#[derive(Component)]
pub(crate) struct GibDraw;

/// Gibs use studio lighting (GetAmbientLightColor + direct light calculation) evaluated
/// at its center like other studio models, and alpha-blended while SUB_FadeOut runs.
pub fn present_gibs(
    mut commands: Commands,
    mut effects: ResMut<Effects>,
    game: Res<Gameplay>,
    mut materials: ResMut<Assets<rendering::SourceMaterial>>,
    mut draws: Query<&mut Transform, With<GibDraw>>,
) {
    let live: std::collections::BTreeSet<u64> = game.gibs.active.iter().map(|g| g.id).collect();
    let gone: Vec<u64> = effects
        .gibs
        .keys()
        .copied()
        .filter(|id| !live.contains(id))
        .collect();
    for id in gone {
        for (entity, material, _) in effects.gibs.remove(&id).unwrap_or_default() {
            if let Ok(mut cmd) = commands.get_entity(entity) {
                cmd.despawn();
            }
            materials.remove(&material);
        }
        effects.gib_state.remove(&id);
    }
    effects.gib_draws = 0;
    let lighting = game.world.lighting.as_deref();
    for gib in &game.gibs.active {
        let transform = rendering::entity_transform(gib.position, gib.rotation);
        if !effects.gibs.contains_key(&gib.id) {
            let parts: Vec<ModelPart> = effects
                .grenade_meshes
                .get(&gib.render_key())
                .cloned()
                .unwrap_or_default();
            let mut spawned = Vec::new();
            for (mesh, material) in parts {
                let Some(own) = materials.get(&material).cloned() else {
                    continue;
                };
                let alpha = own.alpha;
                let own = materials.add(own);
                let entity = commands
                    .spawn((
                        crate::campaign::MapOwned,
                        Mesh3d(mesh),
                        MeshMaterial3d(own.clone()),
                        transform,
                        Visibility::Visible,
                        GibDraw,
                    ))
                    .id();
                spawned.push((entity, own, alpha));
            }
            effects.gibs.insert(gib.id, spawned);
        }
        let center = gib.position;
        let refresh = effects
            .gib_state
            .get(&gib.id)
            .is_none_or(|(last, alpha)| last.distance_squared(center) >= 1. || *alpha != gib.alpha);
        effects.gib_draws += effects.gibs[&gib.id].len();
        let parts = &effects.gibs[&gib.id];
        for (entity, material, authored) in parts {
            if let Ok(mut current) = draws.get_mut(*entity) {
                *current = transform;
            }
            if !refresh {
                continue;
            }
            if let Some(mut m) = materials.get_mut(material) {
                if let Some(data) = lighting {
                    m.lighting =
                        rendering::ModelLighting::from_state(&data.state_at(center), false);
                }
                m.tint.w = f32::from(gib.alpha) / 255.;
                // kRenderTransTexture once the fade starts.
                m.alpha = if gib.alpha < 255 {
                    AlphaMode::Blend
                } else {
                    *authored
                };
            }
        }
        effects.gib_state.insert(gib.id, (center, gib.alpha));
    }
}
