//! Parallel pre-building of root and coarser LOD meshes during map installation.
use super::{
    AnimatedMesh, Batch, ModelLighting, Owner, SourceEntity, SourceMaterial, WeaponMesh,
    entity_modulation, face_split, lod,
    material::{self, UploadedTextures, alpha_mode, make_material},
    npc_weapon_draws,
};
use crate::{
    Status,
    assets::{LoadedMap, MaterialData, visible_entity},
};
use bevy::{
    asset::{Assets, Handle},
    math::{Mat4, Vec3, Vec4},
    prelude::{AlphaMode, Commands, Image, Transform, Visibility},
};
use modkit_core::Surface;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};

pub(super) type BatchEntry = ((Owner, String, Option<usize>, usize), Batch);
pub(super) type WeaponSkins = BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)>;

pub(super) struct PreparedDraw {
    pub owner: Owner,
    pub name: String,
    pub triangles: usize,
    pub index_count: usize,
    pub material: Handle<SourceMaterial>,
    pub animation: Option<AnimatedMesh>,
    pub lit: bool,
    pub reflects: bool,
    pub shared_lit_key: Option<(Option<usize>, bool)>,
    pub private_mesh: bool,
    pub batch: Option<Batch>,
    pub root_key: Option<lod::RootKey>,
}

pub(super) struct PrepareContext<'a> {
    pub loaded: &'a LoadedMap,
    pub uploaded: &'a UploadedTextures,
    pub lightmaps: &'a [Handle<Image>],
    pub white: &'a Handle<Image>,
    pub black_cube: &'a Handle<Image>,
    pub flashlight: &'a Handle<Image>,
    pub missing: &'a Handle<Image>,
    pub plain_materials: bool,
    pub cpu_skinning: bool,
}

pub(super) fn collect_map_batches(
    loaded: &LoadedMap,
    status: &Status,
    cpu_skinning: bool,
    world_partition: bool,
) -> (Vec<BatchEntry>, WeaponSkins) {
    let world = &loaded.world;
    let mut batches: BTreeMap<(Owner, String, Option<usize>, usize), Batch> = BTreeMap::new();
    let mut transparent_id = 0usize;
    let mut append_layer = |surface: &Surface,
                            transform: Mat4,
                            owner: Owner,
                            layer: Option<crate::assets::DecalLayer>| {
        let owner = if surface.background && owner == Owner::World {
            Owner::SkyWorld
        } else {
            owner
        };
        if surface.indices.is_empty() {
            return;
        }
        let definition = loaded.materials.get(&surface.material);
        let draw_id = if layer.is_some()
            || definition
                .is_some_and(|m| matches!(alpha_mode(m), AlphaMode::Blend | AlphaMode::Add))
        {
            transparent_id += 1;
            transparent_id
        } else {
            0
        };
        batches
            .entry((owner, surface.material.clone(), surface.lightmap, draw_id))
            .or_insert_with(|| Batch {
                decal_layer: layer,
                ..Default::default()
            })
            .append(surface, transform, definition);
    };
    let (ordinary, layered) = world
        .surfaces
        .split_at(world.surfaces.len() - loaded.decal_layers.len());
    for (surface, &layer) in layered.iter().zip(&loaded.decal_layers) {
        append_layer(surface, Mat4::IDENTITY, Owner::World, Some(layer));
    }
    let mut append =
        |surface: &Surface, transform, owner| append_layer(surface, transform, owner, None);
    for surface in ordinary {
        append(surface, Mat4::IDENTITY, Owner::World);
    }
    for surface in &world.overlays {
        append(surface, Mat4::IDENTITY, Owner::World);
    }
    collect_entity_batches(world, &mut append);
    for (id, key, scale) in npc_weapon_draws::npc_weapon_draws(world, &loaded.gameplay.ai) {
        if let Some(surfaces) = world.model_assets.get(&key) {
            let transform = Mat4::from_scale(Vec3::splat(scale));
            for surface in surfaces {
                append(surface, transform, Owner::NpcWeapon(id, key.clone()));
            }
        }
    }
    let weapon_skins = collect_weapon_batches(loaded, &mut append);
    let mut stats = status.0.lock().expect("status lock");
    stats.skipped_background_surfaces = 0;
    stats.world_partition = world_partition;
    let partitioned = partition_batches(batches, loaded, &mut stats, cpu_skinning, world_partition);
    (partitioned, weapon_skins)
}

fn collect_entity_batches(
    world: &modkit_core::World,
    append: &mut impl FnMut(&Surface, Mat4, Owner),
) {
    for (id, entity) in world.entities.iter().enumerate() {
        if !visible_entity(world, id) {
            continue;
        }
        let brush = entity
            .get("model")
            .and_then(|s| s.strip_prefix('*'))
            .and_then(|s| s.parse::<usize>().ok())
            .and_then(|id| world.brush_models.iter().find(|model| model.id == id));
        if let Some(brush) = brush {
            for surface in &brush.surfaces {
                append(surface, Mat4::IDENTITY, Owner::Entity(id));
            }
        }
    }
    for instance in &world.model_instances {
        if instance.entity.is_none_or(|id| !visible_entity(world, id)) {
            continue;
        }
        if let Some(surfaces) = world.model_assets.get(&instance.asset_key()) {
            let transform = Mat4::from_scale(Vec3::splat(instance.scale));
            for surface in surfaces {
                append(surface, transform, Owner::Entity(instance.entity.unwrap()));
            }
        }
    }
}

fn collect_weapon_batches(
    loaded: &LoadedMap,
    append: &mut impl FnMut(&Surface, Mat4, Owner),
) -> WeaponSkins {
    let mut weapon_skins: WeaponSkins = BTreeMap::new();
    for (name, weapon) in &loaded.gameplay.weapons {
        let model = weapon.viewmodel.to_lowercase();
        let Some(surfaces) = loaded.world.model_assets.get(&format!("{model}#0")) else {
            continue;
        };
        for surface in surfaces {
            append(surface, Mat4::IDENTITY, Owner::Weapon(name.clone()));
        }
        if let Some(alternate) = loaded.world.model_assets.get(&format!("{model}#1")) {
            for (a, b) in surfaces.iter().zip(alternate) {
                if a.material != b.material {
                    let sets = weapon_skins.entry(name.clone()).or_default();
                    sets.0.insert(a.material.clone());
                    sets.1.insert(b.material.clone());
                    append(b, Mat4::IDENTITY, Owner::Weapon(name.clone()));
                }
            }
        }
    }
    weapon_skins
}

fn partition_batches(
    batches: BTreeMap<(Owner, String, Option<usize>, usize), Batch>,
    loaded: &LoadedMap,
    stats: &mut crate::RunStatus,
    cpu_skinning: bool,
    world_partition: bool,
) -> Vec<BatchEntry> {
    batches
        .into_iter()
        .flat_map(|(key, batch)| {
            let partition = world_partition
                && key.0 == Owner::World
                && loaded.materials.get(&key.1).is_none_or(|m| {
                    matches!(alpha_mode(m), AlphaMode::Opaque | AlphaMode::Mask(_))
                });
            let chunks = match &key.0 {
                _ if partition => batch.spatial_chunks(),
                Owner::Entity(id) if cpu_skinning => {
                    face_split::split_entity_batch(batch, &loaded.world, &loaded.flexes, *id)
                }
                _ => vec![batch],
            };
            if chunks.len() > 1 {
                stats.partitioned_world_batches += 1;
            }
            chunks.into_iter().map(move |batch| (key.clone(), batch))
        })
        .collect()
}

/// The install-frame half of a map's draws: each batch's material in batch order (also for a
/// draw the geometry pass dropped, as before the split) and the joint entities of the kept
/// skinned draws, replacing the geometry pass's planned skeletons.
pub(super) fn resolve_draws(
    geometry: &mut super::MapGeometry,
    cx: &PrepareContext<'_>,
    commands: &mut Commands,
    (materials, slots): (
        &mut Assets<SourceMaterial>,
        &mut crate::material_slots::MaterialSlots,
    ),
    inverse_binds: &mut Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>,
    status: &Status,
) -> BTreeMap<Owner, crate::gpu_skinning::Skeleton> {
    let reflections = super::cubemaps::Envmaps {
        world: &cx.uploaded.envmaps,
        masks: &cx.uploaded.envmap_masks,
        plain: cx.plain_materials,
    };
    let mut material_handles = BTreeMap::new();
    let mut skeletons = BTreeMap::<Owner, crate::gpu_skinning::Skeleton>::new();
    let fallback = MaterialData::default();
    for request in &geometry.requests {
        let definition = cx.loaded.materials.get(&request.name).unwrap_or(&fallback);
        let reflects = request.lit && definition.model_envmap && !cx.plain_materials;
        let (material, shared_lit_key) = resolve_draw_material(
            cx,
            (&request.owner, &request.name, definition, request.lightmap),
            (request.lit, reflects, request.model, request.decal_order),
            (&reflections, materials, slots, &mut material_handles),
        );
        let Some(index) = request.draw else {
            continue;
        };
        let draw = &mut geometry.draws[index];
        draw.material = material;
        draw.reflects = reflects;
        draw.shared_lit_key = shared_lit_key;
        let planned = joint_count(draw.animation.as_ref());
        let kept = attach_draw_skeleton(
            commands,
            inverse_binds,
            cx,
            &draw.owner,
            draw.animation.as_mut(),
            &mut skeletons,
        );
        debug_assert!(kept, "the geometry pass keeps only attachable draws");
        debug_assert_eq!(
            joint_count(draw.animation.as_ref()),
            planned,
            "planned skeleton of {}",
            draw.name
        );
    }
    let mut stats = status.0.lock().expect("status lock");
    stats.skin_joints = skeletons.values().map(|s| s.joints.len()).sum();
    stats.materials = material_handles.len();
    skeletons
}

type MaterialKey = (String, Option<usize>, bool, [u32; 4], Option<Owner>, usize);

fn resolve_draw_material(
    cx: &PrepareContext<'_>,
    (owner, name, definition, lm): (&Owner, &str, &MaterialData, Option<usize>),
    (lit, reflects, is_model, decal_order): (bool, bool, bool, usize),
    (reflections, materials, slots, material_handles): (
        &super::cubemaps::Envmaps<'_>,
        &mut Assets<SourceMaterial>,
        &mut crate::material_slots::MaterialSlots,
        &mut BTreeMap<MaterialKey, Handle<SourceMaterial>>,
    ),
) -> (Handle<SourceMaterial>, Option<(Option<usize>, bool)>) {
    let modulation = owner
        .entity()
        .map_or(Vec4::ONE, |id| entity_modulation(&cx.loaded.world, id));
    let commentary = owner.entity().is_some_and(|id| {
        cx.loaded
            .world
            .entities
            .get(id)
            .is_some_and(|e| e.class() == "point_commentary_node")
    });
    let shared_lit_key = owner
        .entity()
        .filter(|_| lit && !reflects && !commentary && definition.two_texture.is_none())
        .map(|id| (Some(id), definition.half_lambert))
        .filter(|&key| slots.has_slot_for(key));
    let mat_key = (
        name.to_owned(),
        lm,
        lit,
        modulation.to_array().map(f32::to_bits),
        ((lit && shared_lit_key.is_none()) || definition.two_texture.is_some())
            .then(|| owner.clone()),
        decal_order,
    );
    let handle = material_handles
        .entry(mat_key)
        .or_insert_with(|| {
            let mut mat = build_draw_material(
                cx,
                name,
                definition,
                lm,
                modulation,
                (lit, reflects, is_model),
                reflections,
            );
            mat.decal_order = decal_order;
            materials.add(mat)
        })
        .clone();
    (handle, shared_lit_key)
}

fn build_draw_material(
    cx: &PrepareContext<'_>,
    name: &str,
    definition: &MaterialData,
    lm: Option<usize>,
    modulation: Vec4,
    (lit, reflects, is_model): (bool, bool, bool),
    reflections: &super::cubemaps::Envmaps<'_>,
) -> SourceMaterial {
    let mut material = make_material(
        definition,
        cx.uploaded.bases.get(name).unwrap_or(cx.missing).clone(),
        lm.and_then(|index| cx.lightmaps.get(index))
            .unwrap_or(cx.white)
            .clone(),
        (cx.uploaded.irises.get(name))
            .or_else(|| cx.uploaded.overlays.get(name))
            .or_else(|| cx.uploaded.seconds.get(name))
            .cloned(),
        (cx.white, cx.black_cube, cx.flashlight),
    );
    material.tint *= modulation;
    if cx.uploaded.seconds.contains_key(name)
        && !definition.camera
        && definition.two_texture.is_none()
    {
        material.parameters.w = material::BLEND_SECOND_BASE;
    }
    if let Some((base, second)) = cx.uploaded.two_textures.get(name) {
        material.base = base[0].clone();
        material.iris = second[0].clone();
        material.tint = Vec3::from_array(std::array::from_fn(|i| {
            crate::monitors::linear_modulation(definition.tint[i] * modulation[i])
        }))
        .extend(1.);
    }
    reflections.bind(&mut material, name, is_model, reflects);
    if lit {
        material.lighting = ModelLighting::fallback(definition.half_lambert);
    }
    material
}

/// The joint count of a draw's skeleton, planned or spawned.
fn joint_count(animation: Option<&AnimatedMesh>) -> Option<usize> {
    animation.and_then(|a| a.gpu.as_ref().map(|s| s.joints.len()))
}

pub(super) fn draw_animation(
    loaded: &LoadedMap,
    owner: &Owner,
    batch: &mut Batch,
) -> Option<AnimatedMesh> {
    if !batch.skin.iter().any(|(_, w)| w.is_some()) {
        return None;
    }
    let world = &loaded.world;
    match owner {
        Owner::Entity(id) => world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(*id))
            .map(|instance| {
                let model = (instance.asset_key(), instance.scale);
                AnimatedMesh::from_batch(batch, (Some(*id), None), model, true)
            }),
        Owner::NpcWeapon(id, key) => {
            let scale = world
                .model_instances
                .iter()
                .find(|i| i.entity == Some(*id))
                .map_or(1., |i| i.scale);
            let owner = (Some(*id), Some(key.clone()));
            Some(AnimatedMesh::from_batch(
                batch,
                owner,
                (key.clone(), scale),
                false,
            ))
        }
        Owner::Weapon(name) => {
            let key = loaded.gameplay.weapons[name].viewmodel.to_lowercase();
            let owner = (None, Some(name.clone()));
            Some(AnimatedMesh::from_batch(
                batch,
                owner,
                (format!("{key}#0"), 1.),
                false,
            ))
        }
        Owner::World | Owner::SkyWorld => None,
    }
}

fn attach_draw_skeleton(
    commands: &mut Commands,
    inverse_binds: &mut Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>,
    cx: &PrepareContext<'_>,
    owner: &Owner,
    animation: Option<&mut AnimatedMesh>,
    skeletons: &mut BTreeMap<Owner, crate::gpu_skinning::Skeleton>,
) -> bool {
    let world = &cx.loaded.world;
    let Some(animation) = animation else {
        return true;
    };
    if let Owner::NpcWeapon(id, key) = owner {
        let npc = skeletons.get(&Owner::Entity(*id)).cloned();
        let npc_rig = world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(*id))
            .and_then(|i| world.rigs.get(&i.asset_key()));
        return match (cx.cpu_skinning, npc, npc_rig, world.rigs.get(key)) {
            (false, Some(npc), Some(npc_rig), Some(rig)) => {
                animation.gpu = Some(crate::gpu_skinning::Skeleton::merged(
                    inverse_binds,
                    rig,
                    npc_rig,
                    &npc,
                    animation.scale,
                ));
                true
            }
            _ => false,
        };
    }
    if !cx.cpu_skinning
        && let Some(rig) = world
            .rigs
            .get(&animation.key)
            .filter(|rig| !rig.bones.is_empty() && rig.bones.len() < 256)
    {
        let skeleton = skeletons.entry(owner.clone()).or_insert_with(|| {
            let mut root = commands.spawn((
                crate::campaign::MapOwned,
                Transform::IDENTITY,
                Visibility::Inherited,
            ));
            match owner {
                Owner::Entity(id) => {
                    root.insert(SourceEntity(*id));
                }
                Owner::Weapon(name) => {
                    root.insert(WeaponMesh(name.clone()));
                }
                _ => unreachable!("only owned model meshes animate"),
            }
            let root = root.id();
            crate::gpu_skinning::Skeleton::spawn(
                commands,
                inverse_binds,
                rig,
                animation.scale,
                root,
            )
        });
        animation.gpu = Some(skeleton.clone());
    }
    true
}

struct RootJob<'a> {
    batch: Batch,
    joint_count: Option<usize>,
    vertices: Option<Arc<lod::Vertices>>,
    scale: f32,
    face: Option<&'a source_assets::flexes::FaceModel>,
    has_animation: bool,
    private_mesh: bool,
}

struct LevelsJob<'a> {
    shared_key: Option<lod::LevelsKey>,
    private_id: usize,
    lods: &'a [modkit_core::ModelLod],
    material: String,
    definition: &'a MaterialData,
    first: &'a Surface,
    scale: f32,
    joint_count: Option<usize>,
    has_animation: bool,
    flexes: bool,
    morph: Option<Arc<super::face_flex::MorphChannels>>,
    face: Option<&'a source_assets::flexes::FaceModel>,
}

/// Pre-build all unique root and coarser LOD meshes (including skinning and facial morph targets)
/// in parallel across CPU cores before deterministic main-thread `Assets<Mesh>` insertion.
pub(super) fn build_map_meshes(
    prepared: &mut [PreparedDraw],
    loaded: &LoadedMap,
    shared_levels: &mut lod::SharedLevels,
) {
    let (roots_shared, roots_private) = build_roots_parallel(prepared, &loaded.flexes);
    let (levels_shared, levels_private) = build_levels_parallel(prepared, loaded, shared_levels);
    shared_levels.set_prebuilt(lod::PrebuiltMeshes {
        roots_shared,
        roots_private,
        levels_shared,
        levels_private,
    });
}

fn build_roots_parallel(
    prepared: &mut [PreparedDraw],
    faces: &BTreeMap<String, Arc<source_assets::flexes::FaceModel>>,
) -> (
    BTreeMap<lod::RootKey, (lod::BuiltRoot, usize)>,
    VecDeque<lod::BuiltRoot>,
) {
    let mut shared_indices: BTreeMap<lod::RootKey, usize> = BTreeMap::new();
    let mut shared_counts: BTreeMap<lod::RootKey, usize> = BTreeMap::new();
    let mut draw_job_idx = Vec::with_capacity(prepared.len());
    let mut jobs = Vec::new();
    for draw in prepared.iter_mut() {
        let key = lod::root_key(
            &draw.name,
            draw.index_count,
            draw.animation.as_ref(),
            draw.private_mesh,
        );
        draw.root_key = key.clone();
        let batch = draw.batch.take().expect("draw batch");
        if let Some(key) = key {
            *shared_counts.entry(key.clone()).or_insert(0) += 1;
            let idx = if let Some(&existing) = shared_indices.get(&key) {
                existing
            } else {
                let idx = jobs.len();
                shared_indices.insert(key.clone(), idx);
                jobs.push(make_root_job(Some(key), batch, draw, faces));
                idx
            };
            draw_job_idx.push(idx);
        } else {
            let idx = jobs.len();
            jobs.push(make_root_job(None, batch, draw, faces));
            draw_job_idx.push(idx);
        }
    }
    let results = parallel_map(jobs, |(key, job)| {
        let built = lod::build_root_mesh(lod::RootBuildInput {
            batch: job.batch,
            joint_count: job.joint_count,
            vertices: job.vertices.as_deref(),
            scale: job.scale,
            face: job.face,
            has_animation: job.has_animation,
            private_mesh: job.private_mesh,
        });
        (key, built)
    });
    for (draw, &job_idx) in prepared.iter_mut().zip(&draw_job_idx) {
        if let Some(anim) = &mut draw.animation {
            anim.morph = results[job_idx].1.morph.clone();
        }
    }
    let mut roots_shared = BTreeMap::new();
    let mut roots_private = VecDeque::new();
    for (key, built) in results {
        if let Some(key) = key {
            let count = shared_counts[&key];
            roots_shared.insert(key, (built, count));
        } else {
            roots_private.push_back(built);
        }
    }
    (roots_shared, roots_private)
}

fn make_root_job<'a>(
    shared_key: Option<lod::RootKey>,
    batch: Batch,
    draw: &PreparedDraw,
    faces: &'a BTreeMap<String, Arc<source_assets::flexes::FaceModel>>,
) -> (Option<lod::RootKey>, RootJob<'a>) {
    let (has_animation, joint_count, vertices, scale, face) = match &draw.animation {
        Some(anim) => (
            true,
            anim.gpu.as_ref().map(|s| s.joints.len()),
            Some(Arc::clone(&anim.vertices)),
            anim.scale,
            faces.get(&anim.key).map(Arc::as_ref),
        ),
        None => (false, None, None, 1., None),
    };
    (
        shared_key,
        RootJob {
            batch,
            joint_count,
            vertices,
            scale,
            face,
            has_animation,
            private_mesh: draw.private_mesh,
        },
    )
}

fn build_levels_parallel(
    prepared: &[PreparedDraw],
    loaded: &LoadedMap,
    shared_levels: &lod::SharedLevels,
) -> (
    BTreeMap<lod::LevelsKey, lod::BuiltLevels>,
    lod::PrivateBuiltLevels,
) {
    let fallback = MaterialData::default();
    let jobs = collect_level_jobs(prepared, loaded, shared_levels, &fallback);
    let results = parallel_map(jobs, |job| {
        let levels = lod::Coarser {
            lods: job.lods,
            material: &job.material,
            definition: job.definition,
            first: job.first,
            scale: job.scale,
            joint_count: job.joint_count,
            has_animation: job.has_animation,
            flexes: job.flexes,
            morph: job.morph.as_deref(),
            face: job.face,
        }
        .build_meshes();
        (job.shared_key, job.private_id, job.material, levels)
    });
    let mut levels_shared = BTreeMap::new();
    let mut levels_private = VecDeque::new();
    for (shared_key, id, material, levels) in results {
        if let Some(key) = shared_key {
            levels_shared.insert(key, levels);
        } else {
            levels_private.push_back((id, material, levels));
        }
    }
    (levels_shared, levels_private)
}

fn collect_level_jobs<'a>(
    prepared: &[PreparedDraw],
    loaded: &'a LoadedMap,
    shared_levels: &lod::SharedLevels,
    fallback: &'a MaterialData,
) -> Vec<LevelsJob<'a>> {
    let mut entity_instance = BTreeMap::new();
    for inst in &loaded.world.model_instances {
        if let Some(id) = inst.entity {
            entity_instance.entry(id).or_insert(inst);
        }
    }
    let mut seen = BTreeSet::new();
    let mut jobs = Vec::new();
    for draw in prepared {
        let Some(id) = draw.owner.entity() else {
            continue;
        };
        if draw
            .animation
            .as_ref()
            .is_some_and(|a| a.gpu.is_none() || a.weapon.is_some())
        {
            continue;
        }
        let Some(&instance) = entity_instance.get(&id) else {
            continue;
        };
        let asset_key = instance.asset_key();
        if loaded
            .eyes
            .contains_key(&(asset_key.clone(), draw.name.clone()))
        {
            continue;
        }
        let Some(lods) = loaded.world.model_lods.get(&asset_key) else {
            continue;
        };
        let Some(first) = loaded.world.model_assets.get(&asset_key).and_then(|s| {
            s.iter()
                .find(|s| s.material == draw.name && !s.vertices.is_empty())
        }) else {
            continue;
        };
        let morph = draw.animation.as_ref().and_then(|a| a.morph.clone());
        let flexes = morph.is_none()
            && draw
                .animation
                .as_ref()
                .is_some_and(|a| shared_levels.deforms(a));
        let shared_key = (!flexes).then(|| {
            lod::levels_key(
                asset_key,
                draw.name.clone(),
                instance.scale,
                draw.animation.is_some(),
                morph.is_some(),
            )
        });
        if let Some(key) = &shared_key
            && !seen.insert(key.clone())
        {
            continue;
        }
        let face = morph.as_ref().and_then(|_| {
            draw.animation
                .as_ref()
                .and_then(|a| loaded.flexes.get(&a.key).map(Arc::as_ref))
        });
        jobs.push(LevelsJob {
            shared_key,
            private_id: id,
            lods,
            material: draw.name.clone(),
            definition: loaded.materials.get(&draw.name).unwrap_or(fallback),
            first,
            scale: instance.scale,
            joint_count: draw
                .animation
                .as_ref()
                .and_then(|a| a.gpu.as_ref().map(|s| s.joints.len())),
            has_animation: draw.animation.is_some(),
            flexes,
            morph,
            face,
        });
    }
    jobs
}

fn parallel_map<T: Send, R: Send, F: Fn(T) -> R + Sync>(items: Vec<T>, f: F) -> Vec<R> {
    let count = items.len();
    if count == 0 {
        return Vec::new();
    }
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .clamp(1, 8)
        .min(count);
    if threads <= 1 {
        return items.into_iter().map(f).collect();
    }
    let queue = std::sync::Mutex::new(items.into_iter().enumerate());
    let mut results: Vec<Option<R>> = std::iter::repeat_with(|| None).take(count).collect();
    let collected = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut local = Vec::new();
                    loop {
                        let next = queue.lock().expect("mesh build queue").next();
                        let Some((idx, item)) = next else { break };
                        local.push((idx, f(item)));
                    }
                    local
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("mesh build worker"))
            .collect::<Vec<_>>()
    });
    for (idx, res) in collected {
        results[idx] = Some(res);
    }
    results.into_iter().map(Option::unwrap).collect()
}

#[cfg(test)]
mod decal_tests;
