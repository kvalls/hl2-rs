//! A map's draw geometry, built on the loader worker (O49): batches, draw animation, which draws
//! survive skeleton attachment, root and coarser LOD meshes. Only material creation, joint
//! entities and draw spawning stay for the install frame (`mesh_build::resolve_draws`).
use super::{
    AnimatedMesh, Owner,
    lod::SharedLevels,
    mesh_build::{self, PreparedDraw, WeaponSkins},
};
use crate::{Status, assets::LoadedMap, gpu_skinning::Skeleton};
use bevy::{asset::Handle, ecs::entity::Entity};
use std::collections::BTreeMap;

/// The material one batch asks for, in batch order (a dropped draw still creates its material,
/// as before the split); `draw` is its index in `MapGeometry::draws` when the draw is kept.
pub(super) struct MaterialRequest {
    pub owner: Owner,
    pub name: String,
    pub lightmap: Option<usize>,
    pub model: bool,
    pub decal_order: usize,
    pub lit: bool,
    pub draw: Option<usize>,
}

/// Everything the install frame needs to spawn a map's draws except materials and joints.
pub(crate) struct MapGeometry {
    pub(super) draws: Vec<PreparedDraw>,
    pub(super) requests: Vec<MaterialRequest>,
    pub(super) weapon_skins: WeaponSkins,
    pub(super) shared_levels: SharedLevels,
    /// Draw options the geometry was built for; the install rebuilds it if they changed.
    pub(super) built_for: (bool, bool),
    meshes: usize,
    triangles: usize,
    partitioned_world_batches: usize,
}

impl MapGeometry {
    /// Builds the geometry with the given CPU skinning and world partition options.
    pub(crate) fn build(loaded: &LoadedMap, cpu_skinning: bool, world_partition: bool) -> Self {
        let local = Status::default();
        let (batches, weapon_skins) =
            mesh_build::collect_map_batches(loaded, &local, cpu_skinning, world_partition);
        let partitioned_world_batches = local
            .0
            .lock()
            .expect("status lock")
            .partitioned_world_batches;
        let mut geometry = Self::planned(batches, loaded, cpu_skinning);
        geometry.weapon_skins = weapon_skins;
        geometry.built_for = (cpu_skinning, world_partition);
        geometry.partitioned_world_batches = partitioned_world_batches;
        mesh_build::build_map_meshes(&mut geometry.draws, loaded, &mut geometry.shared_levels);
        geometry
    }

    /// The draws and material requests of `batches`, before any mesh is built (each kept draw
    /// still holds its batch).
    pub(super) fn planned(
        batches: Vec<mesh_build::BatchEntry>,
        loaded: &LoadedMap,
        cpu_skinning: bool,
    ) -> Self {
        let mut geometry = Self {
            draws: Vec::with_capacity(batches.len()),
            requests: Vec::with_capacity(batches.len()),
            weapon_skins: WeaponSkins::new(),
            shared_levels: SharedLevels::for_map(&loaded.flexes),
            built_for: (cpu_skinning, false),
            meshes: 0,
            triangles: 0,
            partitioned_world_batches: 0,
        };
        geometry.plan(batches, loaded, cpu_skinning);
        geometry
    }

    fn plan(&mut self, batches: Vec<mesh_build::BatchEntry>, loaded: &LoadedMap, cpu: bool) {
        let mut joints = BTreeMap::<Owner, usize>::new();
        for ((owner, name, lightmap, _), mut batch) in batches {
            let definition = loaded.materials.get(&name);
            let lit = batch.model
                && matches!(
                    owner,
                    Owner::Entity(_) | Owner::Weapon(_) | Owner::NpcWeapon(..)
                )
                && definition.is_none_or(|d| !d.unlit && !d.camera);
            let index_count = batch.indices.len();
            self.meshes += 1;
            self.triangles += index_count / 3;
            let mut animation = mesh_build::draw_animation(loaded, &owner, &mut batch);
            let kept = plan_skeleton(loaded, cpu, &owner, animation.as_mut(), &mut joints);
            self.requests.push(MaterialRequest {
                owner: owner.clone(),
                name: name.clone(),
                lightmap,
                model: batch.model,
                decal_order: batch.decal_layer.map_or(0, |l| l.order),
                lit,
                draw: kept.then_some(self.draws.len()),
            });
            if !kept {
                continue;
            }
            let private_mesh =
                definition.is_some_and(|d| d.iris.is_some() && d.iris_path.is_some());
            self.draws.push(PreparedDraw {
                owner,
                name,
                triangles: index_count / 3,
                index_count,
                material: Handle::default(),
                animation,
                lit,
                reflects: false,
                shared_lit_key: None,
                private_mesh,
                batch: Some(batch),
                root_key: None,
            });
        }
    }

    /// The run status figures the geometry pass owns, written when the map is installed.
    pub(super) fn record(&self, status: &Status, world_partition: bool) {
        let mut stats = status.0.lock().expect("status lock");
        stats.skipped_background_surfaces = 0;
        stats.world_partition = world_partition;
        stats.partitioned_world_batches += self.partitioned_world_batches;
        stats.meshes += self.meshes;
        stats.triangles += self.triangles;
    }
}

/// A skeleton with the joint count `attach_draw_skeleton` will give the draw, so meshes can be
/// built before any joint entity exists; the install replaces it with the spawned one.
fn planned_skeleton(joints: usize) -> Skeleton {
    Skeleton {
        joints: vec![Entity::PLACEHOLDER; joints],
        inverse_bindposes: Handle::default(),
    }
}

/// `mesh_build::attach_draw_skeleton` without spawning: whether the draw is kept and, for a
/// GPU-skinned draw, its planned skeleton. A spawned skeleton has one joint per bone plus an
/// identity joint (`Skeleton::spawn`, `Skeleton::merged`), and an owner's draws share the
/// skeleton of its first.
fn plan_skeleton(
    loaded: &LoadedMap,
    cpu_skinning: bool,
    owner: &Owner,
    animation: Option<&mut AnimatedMesh>,
    joints: &mut BTreeMap<Owner, usize>,
) -> bool {
    let world = &loaded.world;
    let Some(animation) = animation else {
        return true;
    };
    if let Owner::NpcWeapon(id, key) = owner {
        let npc = joints.contains_key(&Owner::Entity(*id));
        let npc_rig = world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(*id))
            .and_then(|i| world.rigs.get(&i.asset_key()));
        return match (cpu_skinning, npc, npc_rig, world.rigs.get(key)) {
            (false, true, Some(_), Some(rig)) => {
                animation.gpu = Some(planned_skeleton(rig.bones.len() + 1));
                true
            }
            _ => false,
        };
    }
    if !cpu_skinning
        && let Some(rig) = world
            .rigs
            .get(&animation.key)
            .filter(|rig| !rig.bones.is_empty() && rig.bones.len() < 256)
    {
        let count = *joints.entry(owner.clone()).or_insert(rig.bones.len() + 1);
        animation.gpu = Some(planned_skeleton(count));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::MapGeometry;
    use crate::{
        Status,
        rendering::{material, mesh_build},
    };
    use bevy::{
        ecs::entity::Entity,
        prelude::{Assets, Handle},
    };

    #[test]
    #[ignore = "requires an owned Half-Life 2 installation"]
    fn owned_station_planned_skeletons_match_the_spawned_ones() {
        let root = source_assets::install::discover().unwrap();
        let mut map = crate::assets::load(&root, "hl2", "d1_trainstation_02").unwrap();
        let mut geometry = MapGeometry::build(&map, false, false);
        let planned: Vec<_> = geometry
            .draws
            .iter()
            .map(|d| {
                d.animation
                    .as_ref()
                    .and_then(|a| a.gpu.as_ref())
                    .map(|s| s.joints.len())
            })
            .collect();
        assert!(
            planned.iter().filter(|p| p.is_some()).count() > 10,
            "skinned draws"
        );
        let image = Handle::default();
        let uploaded = material::upload_map_textures(
            &mut map,
            &mut Assets::default(),
            &image,
            (
                hl2_ui::config::Config::default().video.advanced.filtering,
                false,
            ),
            &image,
        );
        let cx = mesh_build::PrepareContext {
            loaded: &map,
            uploaded: &uploaded,
            lightmaps: &[],
            white: &image,
            black_cube: &image,
            flashlight: &image,
            missing: &image,
            plain_materials: false,
            cpu_skinning: false,
        };
        let mut world = bevy::prelude::World::new();
        let skeletons = mesh_build::resolve_draws(
            &mut geometry,
            &cx,
            &mut world.commands(),
            (&mut Assets::default(), &mut Default::default()),
            &mut Assets::default(),
            &Status::default(),
        );
        assert!(!skeletons.is_empty());
        for (draw, planned) in geometry.draws.iter().zip(planned) {
            let spawned = draw.animation.as_ref().and_then(|a| a.gpu.as_ref());
            assert_eq!(spawned.map(|s| s.joints.len()), planned, "{}", draw.name);
            assert!(
                spawned.is_none_or(|s| !s.joints.contains(&Entity::PLACEHOLDER)),
                "{} kept a planned joint",
                draw.name
            );
        }
    }
}
