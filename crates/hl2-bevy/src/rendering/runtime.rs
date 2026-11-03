//! Runtime model and draw spawning for newly spawned actors and weapons.
use super::{
    DrawTriangles, EyeMesh, ModelLit, NpcWeaponDraw, SourceEntity, SourceMaterial, copy_asset,
    entity_transform, lod,
};
use bevy::{
    asset::{Assets, Handle},
    ecs::query::Has,
    prelude::{
        Commands, Entity, Mesh, Mesh3d, MeshMaterial3d, Name, Query, ResMut, Transform, Visibility,
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

struct SkeletonParams<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    inverse_binds: &'a mut Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>,
    world: &'a modkit_core::World,
    rig: Option<&'a modkit_core::animation::Rig>,
    skeleton: &'a mut Option<crate::gpu_skinning::Skeleton>,
    scale: f32,
    id: usize,
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
                let root = sp
                    .commands
                    .spawn((
                        crate::campaign::MapOwned,
                        Transform::IDENTITY,
                        Visibility::Inherited,
                        SourceEntity(sp.id),
                    ))
                    .id();
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
    animation: Option<&'a super::AnimatedMesh>,
    pvs: bool,
}

fn spawn_runtime_instance_draw(
    commands: &mut Commands,
    id: usize,
    transform: Transform,
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
        draw.insert(ModelLit {
            entity: Some(id),
            key: lit.key.clone(),
            scale: lit.scale,
            half_lambert: lit.half_lambert,
            last: None,
            dynamic: false,
        });
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

pub fn spawn_runtime_draws(
    mut commands: Commands,
    mut game: ResMut<crate::gameplay::Gameplay>,
    mut eyes: ResMut<crate::eyes::Eyes>,
    draws: Query<TemplateDraw>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SourceMaterial>>,
    mut inverse_binds: ResMut<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>,
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
                    .is_some_and(|a| a.gpu.is_none() || !a.flex.is_empty());
            let levels = draw.11.and_then(|l| l.copy_for(&mut meshes, private_mesh));
            let mesh = match (&levels, private_mesh) {
                (Some(levels), _) => Some(levels.root()),
                (None, true) => copy_asset(&mut meshes, &draw.1.0),
                (None, false) => Some(draw.1.0.clone()),
            };
            let material = match draw.8 {
                Some(_) => copy_asset(&mut materials, &draw.2.0),
                None => Some(draw.2.0.clone()),
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
                animation: draw.9,
                pvs: draw.10,
            };
            spawn_runtime_instance_draw(&mut commands, id, transform, gpu.as_ref(), parts, levels);
        }
    }
}
