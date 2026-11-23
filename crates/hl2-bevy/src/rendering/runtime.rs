//! Runtime model and draw spawning for newly spawned actors and weapons.
use super::{
    DrawTriangles, EyeMesh, ModelCache, ModelLit, NpcWeaponDraw, SourceEntity, SourceMaterial,
    build_model_draws, copy_asset, entity_transform, lod,
};
use bevy::{
    asset::{Assets, Handle},
    ecs::query::Has,
    prelude::{
        Commands, Entity, Mesh, Mesh3d, MeshMaterial3d, Name, Query, Res, ResMut, Transform,
        Visibility,
    },
};

type TemplateDraw<'a> = (
    &'a SourceEntity,
    &'a Mesh3d,
    &'a MeshMaterial3d<SourceMaterial>,
    Option<&'a Name>,
    Option<&'a bevy::camera::visibility::RenderLayers>,
    Option<&'a DrawTriangles>,
    Option<&'a NpcWeaponDraw>,
    Option<&'a EyeMesh>,
    Option<&'a ModelLit>,
    Option<&'a super::AnimatedMesh>,
    Has<crate::visibility::PvsDraw>,
    Option<&'a lod::ModelLevels>,
);

/// Killed runtime entities (NPCs, props, dropped weapons) never come back: their hidden
/// draws (model roots with their joints, meshes) are despawned once instead of staying
/// until the map changes. Map entities keep theirs (scripts may show them again).
pub fn despawn_killed_runtime_draws(
    mut commands: Commands,
    mut game: ResMut<crate::gameplay::Gameplay>,
    draws: Query<(Entity, &SourceEntity)>,
) {
    let game = &mut *game;
    let killed: std::collections::BTreeSet<usize> = game
        .runtime_entities
        .iter()
        .copied()
        .filter(|id| {
            game.scene.states.get(*id).is_some_and(|s| s.killed)
                && !game.scene.ragdoll_matrices.contains_key(id)
                && !game.despawned_draws.contains(id)
        })
        .collect();
    if killed.is_empty() {
        return;
    }
    for (entity, source) in &draws {
        if killed.contains(&source.0) {
            commands.entity(entity).try_despawn();
        }
    }
    game.despawned_draws.extend(killed);
}
/// CBaseAnimating::SetBodygroup at runtime (DESIGN 14o session 38): an entity whose Scene
/// body differs from the one it is drawn with gets its draws (and GPU skeleton) rebuilt
/// from the preloaded variant. Entities with eye or bone-merged weapon draws, which
/// build_model_draws does not make, are refused and reported.
pub fn swap_runtime_bodies(
    mut commands: Commands,
    mut game: ResMut<crate::gameplay::Gameplay>,
    draws: Query<(Entity, &SourceEntity, Has<EyeMesh>, Has<NpcWeaponDraw>)>,
    cache: Res<ModelCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SourceMaterial>>,
    mut inverse_binds: ResMut<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>,
) {
    let changed: Vec<(usize, i32)> = game
        .scene
        .bodies
        .iter()
        .filter(|(id, body)| game.drawn_bodies.get(id) != Some(body))
        .map(|(id, body)| (*id, *body))
        .collect();
    if changed.is_empty() {
        return;
    }
    let world = game.world.clone();
    for (id, body) in changed {
        game.drawn_bodies.insert(id, body);
        let Some(instance) = world.model_instances.iter().find(|i| i.entity == Some(id)) else {
            continue;
        };
        let key = modkit_core::ModelInstance {
            body,
            ..instance.clone()
        }
        .asset_key();
        if !world.model_assets.contains_key(&key) {
            game.body_swap_failures
                .push(format!("{id}: {key} not preloaded"));
            continue;
        }
        let own: Vec<_> = draws.iter().filter(|d| d.1.0 == id).collect();
        if own.iter().any(|d| d.2 || d.3) {
            game.body_swap_failures
                .push(format!("{id}: eye or weapon draws"));
            continue;
        }
        for draw in &own {
            commands.entity(draw.0).try_despawn();
        }
        let state = &game.scene.states[id];
        let transform = entity_transform(state.origin, state.rotation);
        build_model_draws(
            &mut commands,
            (&mut meshes, &mut materials, &mut inverse_binds),
            &cache,
            &world,
            id,
            transform,
            Some(body),
        );
        game.body_swaps += 1;
    }
}

struct SkeletonParams<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    inverse_binds: &'a mut Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>,
    world: &'a modkit_core::World,
    rig: Option<&'a modkit_core::animation::Rig>,
    skeleton: &'a mut Option<crate::gpu_skinning::Skeleton>,
    scale: f32,
    id: usize,
}

fn spawn_skeleton_root(commands: &mut Commands, id: usize, visibility: Visibility) -> Entity {
    commands
        .spawn((
            crate::campaign::MapOwned,
            Transform::IDENTITY,
            visibility,
            SourceEntity(id),
        ))
        .id()
}

fn runtime_gpu_skeleton(
    sp: SkeletonParams,
    animation: Option<&super::AnimatedMesh>,
    weapon: Option<&NpcWeaponDraw>,
) -> Option<crate::gpu_skinning::Skeleton> {
    match (animation.and_then(|a| a.gpu.as_ref()), weapon) {
        (None, _) => None,
        (Some(_), None) => {
            let rig = sp.rig?;
            if sp.skeleton.is_none() {
                let root = spawn_skeleton_root(sp.commands, sp.id, Visibility::Inherited);
                *sp.skeleton = Some(crate::gpu_skinning::Skeleton::spawn(
                    sp.commands,
                    sp.inverse_binds,
                    rig,
                    sp.scale,
                    root,
                ));
            }
            sp.skeleton.clone()
        }
        (Some(_), Some(NpcWeaponDraw(key))) => {
            let (owner, owner_rig, weapon_rig) =
                (sp.skeleton.as_ref()?, sp.rig?, sp.world.rigs.get(key)?);
            Some(crate::gpu_skinning::Skeleton::merged(
                sp.inverse_binds,
                weapon_rig,
                owner_rig,
                owner,
                sp.scale,
            ))
        }
    }
}

struct RuntimeDrawParts<'a> {
    mesh: Handle<Mesh>,
    material: Handle<SourceMaterial>,
    name: Option<&'a Name>,
    layers: Option<&'a bevy::camera::visibility::RenderLayers>,
    triangles: Option<&'a DrawTriangles>,
    weapon: Option<&'a NpcWeaponDraw>,
    eye: Option<&'a EyeMesh>,
    lit: Option<&'a ModelLit>,
    tag_lit: bool,
    animation: Option<&'a super::AnimatedMesh>,
    pvs: bool,
}

fn spawn_runtime_instance_draw(
    commands: &mut Commands,
    slots: &mut crate::material_slots::MaterialSlots,
    (id, transform): (usize, Transform),
    gpu: Option<&crate::gpu_skinning::Skeleton>,
    parts: RuntimeDrawParts,
    levels: Option<super::lod::ModelLevels>,
) {
    let mut draw = commands.spawn((
        crate::campaign::MapOwned,
        Mesh3d(parts.mesh),
        MeshMaterial3d(parts.material),
        transform,
        SourceEntity(id),
    ));
    if let Some(name) = parts.name {
        draw.insert(name.clone());
    }
    if let Some(layers) = parts.layers {
        draw.insert(layers.clone());
    }
    if parts.pvs {
        draw.insert(crate::visibility::PvsDraw::default());
    }
    if let Some(levels) = levels {
        draw.insert(levels);
    }
    if let Some(triangles) = parts.triangles {
        draw.insert(DrawTriangles(triangles.0));
    }
    if let Some(weapon) = parts.weapon {
        draw.insert(NpcWeaponDraw(weapon.0.clone()));
    }
    if let Some(eye) = parts.eye {
        draw.insert(EyeMesh {
            key: eye.key.clone(),
            eye: eye.eye.clone(),
            scale: eye.scale,
        });
    }
    if let Some(lit) = parts.lit {
        let draw_entity = draw.id();
        if parts.tag_lit
            && let Some(tag) = slots.assign_draw_tag(draw_entity, (Some(id), lit.half_lambert))
        {
            draw.insert(bevy::mesh::MeshTag(tag));
        }
        draw.insert(
            ModelLit::new(Some(id), lit.key.clone(), lit.scale, lit.half_lambert)
                .reflecting(lit.reflects()),
        );
    }
    if let Some(animation) = parts.animation {
        if let Some(skeleton) = gpu {
            draw.insert((
                skeleton.component(),
                bevy::camera::visibility::DynamicSkinnedMeshBounds,
            ));
        }
        draw.insert(animation.copy_for(id, gpu.cloned()));
    }
}

type RuntimeAssets<'w> = (
    ResMut<'w, Assets<Mesh>>,
    ResMut<'w, Assets<SourceMaterial>>,
    ResMut<'w, Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>,
);

pub fn spawn_runtime_draws(
    mut commands: Commands,
    mut game: ResMut<crate::gameplay::Gameplay>,
    mut eyes: ResMut<crate::eyes::Eyes>,
    draws: Query<TemplateDraw>,
    (mut meshes, mut materials, mut inverse_binds): RuntimeAssets,
    mut slots: ResMut<crate::material_slots::MaterialSlots>,
) {
    if game.render_spawns.is_empty() {
        return;
    }
    let world = game.world.clone();
    for (template, id) in std::mem::take(&mut game.render_spawns) {
        eyes.copy_actor(template, id);
        let (Some(instance), Some(state)) = (
            world.model_instances.iter().find(|i| i.entity == Some(id)),
            game.scene.states.get(id),
        ) else {
            continue;
        };
        let rig = world
            .rigs
            .get(&instance.asset_key())
            .filter(|rig| !rig.bones.is_empty() && rig.bones.len() < 256);
        let transform = entity_transform(state.origin, state.rotation);
        let mut skeleton: Option<crate::gpu_skinning::Skeleton> = None;
        let (body, weapons): (Vec<_>, Vec<_>) = draws
            .iter()
            .filter(|d| d.0.0 == template)
            .partition(|d| d.6.is_none());
        for draw in body.into_iter().chain(weapons) {
            let sp = SkeletonParams {
                commands: &mut commands,
                inverse_binds: &mut inverse_binds,
                world: &world,
                rig,
                skeleton: &mut skeleton,
                scale: instance.scale,
                id,
            };
            let gpu = runtime_gpu_skeleton(sp, draw.9, draw.6);
            let private_mesh = draw.7.is_some()
                || draw
                    .9
                    .is_some_and(|a| a.gpu.is_none() || !a.vertices.flex.is_empty());
            let levels = draw.11.and_then(|l| l.copy_for(&mut meshes, private_mesh));
            let mesh = match (&levels, private_mesh) {
                (Some(levels), _) => Some(levels.root()),
                (None, true) => copy_asset(&mut meshes, &draw.1.0),
                (None, false) => Some(draw.1.0.clone()),
            };
            let tag_lit = draw.8.is_some_and(|lit| {
                !lit.reflects() && slots.has_slot_for((Some(id), lit.half_lambert))
            });
            let material = match draw.8 {
                Some(_) if !tag_lit => copy_asset(&mut materials, &draw.2.0),
                _ => Some(draw.2.0.clone()),
            };
            let (Some(mesh), Some(material)) = (mesh, material) else {
                continue;
            };
            let parts = RuntimeDrawParts {
                mesh,
                material,
                name: draw.3,
                layers: draw.4,
                triangles: draw.5,
                weapon: draw.6,
                eye: draw.7,
                lit: draw.8,
                tag_lit,
                animation: draw.9,
                pvs: draw.10,
            };
            spawn_runtime_instance_draw(
                &mut commands,
                &mut slots,
                (id, transform),
                gpu.as_ref(),
                parts,
                levels,
            );
        }
    }
}
/// The material of one runtime model draw: the definition's textures, the entity's render
/// color, and fallback model lighting until the first light sample.
pub(super) fn draw_material(
    cache: &super::ModelCache,
    definition: &super::MaterialData,
    lit: bool,
    modulation: bevy::math::Vec4,
) -> super::SourceMaterial {
    let base = definition
        .base_path
        .as_ref()
        .and_then(|p| cache.textures.get(p))
        .unwrap_or(&cache.missing)
        .clone();
    let iris = definition
        .iris_path
        .as_ref()
        .and_then(|p| cache.textures.get(p))
        .cloned();
    let mut material = super::make_material(
        definition,
        base,
        cache.white.clone(),
        iris,
        (&cache.white, &cache.black_cube, &cache.flashlight),
    );
    material.tint *= modulation;
    if lit {
        material.lighting = super::ModelLighting::fallback(definition.half_lambert);
    }
    let mask = definition.envmap_mask_path.as_ref();
    let mask = mask.and_then(|path| cache.textures.get(path));
    if let Some(mask) = mask.filter(|_| lit && cache.reflects(definition)) {
        material.envmap_mask = mask.clone();
    }
    material
}

pub(crate) struct TemplateDrawAssets<'a> {
    pub meshes: &'a mut Assets<Mesh>,
    pub materials: &'a mut Assets<SourceMaterial>,
    pub inverse_binds: &'a mut Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>,
}

pub(crate) struct TemplateDrawWorld<'a> {
    pub cache: &'a ModelCache,
    pub world: &'a modkit_core::World,
    pub ai: &'a hl2_simulation::ai::runtime::AiRuntime,
    pub eyes: &'a std::collections::BTreeMap<(String, String), source_assets::eyes::Eyeball>,
    pub cpu_skinning: bool,
}

/// Build the hidden template draws (body, eyes, LODs, and usable NPC weapons) for a
/// late-precached NPC pool template so [`spawn_runtime_draws`] can clone them onto spawns.
pub(crate) fn build_npc_template_draws(
    commands: &mut Commands,
    mut assets: TemplateDrawAssets<'_>,
    ctx: TemplateDrawWorld<'_>,
    template_id: usize,
) -> usize {
    let Some(instance) = ctx
        .world
        .model_instances
        .iter()
        .find(|i| i.entity == Some(template_id))
    else {
        return 0;
    };
    let (draws, skeleton) =
        build_template_body_draws(commands, &mut assets, &ctx, instance, template_id);
    let weapon_draws = build_template_weapon_draws(
        commands,
        &mut assets,
        &ctx,
        (instance, template_id),
        skeleton.as_ref(),
    );
    draws + weapon_draws
}

fn build_template_body_draws(
    commands: &mut Commands,
    assets: &mut TemplateDrawAssets<'_>,
    ctx: &TemplateDrawWorld<'_>,
    instance: &modkit_core::ModelInstance,
    id: usize,
) -> (usize, Option<crate::gpu_skinning::Skeleton>) {
    let asset_key = instance.asset_key();
    let Some(surfaces) = ctx.world.model_assets.get(&asset_key) else {
        return (0, None);
    };
    let batches =
        super::face_split::model_batches(surfaces, instance.scale, ctx.cache, (ctx.world, id));
    let rig = ctx
        .world
        .rigs
        .get(&asset_key)
        .filter(|r| !r.bones.is_empty() && r.bones.len() < 256);
    let modulation = super::entity_modulation(ctx.world, id);
    let mut shared_levels = lod::SharedLevels::for_map(&ctx.cache.faces);
    let mut skeleton: Option<crate::gpu_skinning::Skeleton> = None;
    let mut draws = 0;
    for (name, batch) in batches {
        spawn_template_body_batch(
            commands,
            assets,
            ctx,
            BodyBatchState {
                instance,
                id,
                rig,
                modulation,
                shared_levels: &mut shared_levels,
                skeleton: &mut skeleton,
            },
            (name, batch),
        );
        draws += 1;
    }
    (draws, skeleton)
}

struct BodyBatchState<'a> {
    instance: &'a modkit_core::ModelInstance,
    id: usize,
    rig: Option<&'a modkit_core::animation::Rig>,
    modulation: bevy::math::Vec4,
    shared_levels: &'a mut lod::SharedLevels,
    skeleton: &'a mut Option<crate::gpu_skinning::Skeleton>,
}

fn template_material(
    assets: &mut TemplateDrawAssets<'_>,
    ctx: &TemplateDrawWorld<'_>,
    name: &str,
    batch: &super::Batch,
    modulation: bevy::math::Vec4,
) -> (Handle<SourceMaterial>, bool, usize) {
    let fallback = crate::assets::MaterialData::default();
    let definition = ctx.cache.definitions.get(name).unwrap_or(&fallback);
    let lit = batch.model && !definition.unlit && !definition.camera;
    let material = assets
        .materials
        .add(draw_material(ctx.cache, definition, lit, modulation));
    (material, lit, batch.indices.len() / 3)
}

fn spawn_template_body_batch(
    commands: &mut Commands,
    assets: &mut TemplateDrawAssets<'_>,
    ctx: &TemplateDrawWorld<'_>,
    state: BodyBatchState<'_>,
    (name, mut batch): (String, super::Batch),
) {
    let (instance, id) = (state.instance, state.id);
    let asset_key = instance.asset_key();
    let (material, lit, triangles) =
        template_material(assets, ctx, &name, &batch, state.modulation);
    let mut animation = batch.skin.iter().any(|(_, w)| w.is_some()).then(|| {
        super::AnimatedMesh::from_batch(
            &mut batch,
            (Some(id), None),
            (asset_key.clone(), instance.scale),
            true,
        )
    });
    if !ctx.cpu_skinning
        && let (Some(animation), Some(rig)) = (&mut animation, state.rig)
    {
        let skel = state.skeleton.get_or_insert_with(|| {
            let root = spawn_skeleton_root(commands, id, Visibility::Hidden);
            crate::gpu_skinning::Skeleton::spawn(
                commands,
                assets.inverse_binds,
                rig,
                instance.scale,
                root,
            )
        });
        animation.gpu = Some(skel.clone());
    }
    let eye = ctx.eyes.get(&(asset_key.clone(), name.clone()));
    let mut mesh = batch.mesh();
    if let Some(skel) = animation.as_ref().and_then(|a| a.gpu.as_ref()) {
        skel.attributes(&mut mesh, &animation.as_ref().unwrap().vertices.bind);
    }
    if animation.is_some() || eye.is_some() {
        mesh.asset_usage = bevy::asset::RenderAssetUsages::MAIN_WORLD
            | bevy::asset::RenderAssetUsages::RENDER_WORLD;
    }
    let handle = assets.meshes.add(mesh);
    let mut draw = commands.spawn((
        crate::campaign::MapOwned,
        Name::new(name.clone()),
        Mesh3d(handle.clone()),
        MeshMaterial3d(material),
        entity_transform(instance.origin, glam::Quat::IDENTITY),
        Visibility::Hidden,
        SourceEntity(id),
        crate::visibility::PvsDraw::default(),
        DrawTriangles(triangles),
    ));
    let fallback = crate::assets::MaterialData::default();
    let definition = ctx.cache.definitions.get(&name).unwrap_or(&fallback);
    if let Some(eye) = eye {
        draw.insert(EyeMesh {
            key: asset_key,
            eye: eye.clone(),
            scale: instance.scale,
        });
    } else if let Some(levels) = lod::levels_for_draw(
        ctx.world,
        id,
        &name,
        definition,
        (&handle, animation.as_ref()),
        (assets.meshes, state.shared_levels),
    ) {
        draw.insert(levels);
    }
    if lit {
        draw.insert(ctx.cache.model_lit(id, instance, definition));
    }
    if let Some(animation) = animation {
        if let Some(skel) = &animation.gpu {
            draw.insert((
                skel.component(),
                bevy::camera::visibility::DynamicSkinnedMeshBounds,
            ));
        }
        draw.insert(animation);
    }
}

fn build_template_weapon_draws(
    commands: &mut Commands,
    assets: &mut TemplateDrawAssets<'_>,
    ctx: &TemplateDrawWorld<'_>,
    (instance, id): (&modkit_core::ModelInstance, usize),
    skeleton: Option<&crate::gpu_skinning::Skeleton>,
) -> usize {
    if ctx.cpu_skinning {
        return 0;
    }
    let asset_key = instance.asset_key();
    let (Some(skeleton), Some(owner_rig)) = (skeleton, ctx.world.rigs.get(&asset_key)) else {
        return 0;
    };
    let class = ctx.world.entities[id].class();
    let mut keys: std::collections::BTreeSet<String> = ctx
        .ai
        .usable_weapons(class, &owner_rig.activities)
        .into_iter()
        .filter_map(|w| ctx.ai.weapon_model(&w))
        .map(|m| format!("{m}#0"))
        .collect();
    if let Some(active) = ctx.ai.weapon_world_model(id) {
        keys.insert(format!("{active}#0"));
    }
    let modulation = super::entity_modulation(ctx.world, id);
    let scale_mat = bevy::math::Mat4::from_scale(bevy::math::Vec3::splat(instance.scale));
    let mut draws = 0;
    for weapon_key in keys {
        let (Some(surfaces), Some(weapon_rig)) = (
            ctx.world.model_assets.get(&weapon_key),
            ctx.world.rigs.get(&weapon_key),
        ) else {
            continue;
        };
        let mut batches = std::collections::BTreeMap::<String, super::Batch>::new();
        for s in surfaces.iter().filter(|s| !s.indices.is_empty()) {
            let def = ctx.cache.definitions.get(&s.material);
            batches
                .entry(s.material.clone())
                .or_default()
                .append(s, scale_mat, def);
        }
        for (name, batch) in batches {
            if spawn_template_weapon_batch(
                commands,
                assets,
                ctx,
                WeaponBatchState {
                    instance,
                    id,
                    weapon_key: &weapon_key,
                    owner_rig,
                    weapon_rig,
                    skeleton,
                    modulation,
                },
                (name, batch),
            ) {
                draws += 1;
            }
        }
    }
    draws
}

struct WeaponBatchState<'a> {
    instance: &'a modkit_core::ModelInstance,
    id: usize,
    weapon_key: &'a str,
    owner_rig: &'a modkit_core::animation::Rig,
    weapon_rig: &'a modkit_core::animation::Rig,
    skeleton: &'a crate::gpu_skinning::Skeleton,
    modulation: bevy::math::Vec4,
}

fn spawn_template_weapon_batch(
    commands: &mut Commands,
    assets: &mut TemplateDrawAssets<'_>,
    ctx: &TemplateDrawWorld<'_>,
    state: WeaponBatchState<'_>,
    (name, mut batch): (String, super::Batch),
) -> bool {
    if !batch.skin.iter().any(|(_, w)| w.is_some()) {
        return false;
    }
    let (material, lit, triangles) =
        template_material(assets, ctx, &name, &batch, state.modulation);
    let weapon_key = state.weapon_key.to_owned();
    let mut animation = super::AnimatedMesh::from_batch(
        &mut batch,
        (Some(state.id), Some(weapon_key.clone())),
        (weapon_key.clone(), state.instance.scale),
        false,
    );
    let merged = crate::gpu_skinning::Skeleton::merged(
        assets.inverse_binds,
        state.weapon_rig,
        state.owner_rig,
        state.skeleton,
        state.instance.scale,
    );
    let mut mesh = batch.mesh();
    merged.attributes(&mut mesh, &animation.vertices.bind);
    mesh.asset_usage =
        bevy::asset::RenderAssetUsages::MAIN_WORLD | bevy::asset::RenderAssetUsages::RENDER_WORLD;
    animation.gpu = Some(merged.clone());
    let mut draw = commands.spawn((
        crate::campaign::MapOwned,
        Name::new(name.clone()),
        Mesh3d(assets.meshes.add(mesh)),
        MeshMaterial3d(material),
        entity_transform(state.instance.origin, glam::Quat::IDENTITY),
        Visibility::Hidden,
        SourceEntity(state.id),
        NpcWeaponDraw(weapon_key),
        crate::visibility::PvsDraw::default(),
        DrawTriangles(triangles),
        merged.component(),
        bevy::camera::visibility::DynamicSkinnedMeshBounds,
        animation,
    ));
    if lit {
        let fallback = crate::assets::MaterialData::default();
        let definition = ctx.cache.definitions.get(&name).unwrap_or(&fallback);
        draw.insert(ctx.cache.model_lit(state.id, state.instance, definition));
    }
    true
}
