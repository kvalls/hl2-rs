//! On-demand asynchronous NPC late-precaching (`npc_create`, `npc_create_aimed`,
//! `ent_create`) when a map skipped preloading the developer spawn pool (for example,
//! main-menu background maps). Mirrors Source SDK `ai_concommands.cpp`: first spawn of
//! an unpooled class loads its model, rig, locomotion clips, materials and face data
//! on a worker thread, installs the hidden pool template, and caches it for instant
//! subsequent spawns.
use crate::{
    assets::{MaterialData, faces, load_material, names},
    effects::Effects,
    eyes::Eyes,
    gameplay::{Gameplay, PendingNpcSpawn},
    rendering::{
        FlexModels, ModelCache, SourceMaterial,
        runtime::{TemplateDrawAssets, TemplateDrawWorld, build_npc_template_draws},
    },
    simulation::Simulation,
    ui_host::Console,
};
use anyhow::Context;
#[cfg(test)]
use bevy::{
    app::{App, Update},
    camera::visibility::Visibility,
    ecs::{schedule::IntoScheduleConfigs, world::Mut},
    mesh::Mesh3d,
};
use bevy::{
    asset::Assets,
    ecs::{
        resource::Resource,
        system::{Commands, Res, ResMut, SystemParam},
    },
    image::Image,
    mesh::Mesh,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::Instant,
};

type Slot = Arc<Mutex<Option<Result<PreparedNpc, String>>>>;

/// Mounted VFS handle and in-flight worker jobs for late-precached NPC classes.
#[derive(Resource)]
pub struct RuntimeNpcs {
    vfs: Arc<source_assets::vpk::Vfs>,
    jobs: BTreeMap<String, Job>,
}

struct Job {
    started: Instant,
    waiters: Vec<PendingNpcSpawn>,
    result: Slot,
}

impl RuntimeNpcs {
    pub fn new(vfs: Arc<source_assets::vpk::Vfs>) -> Self {
        Self {
            vfs,
            jobs: BTreeMap::new(),
        }
    }
}

pub(crate) struct PreparedNpc {
    world: modkit_core::World,
    template: modkit_core::Entity,
    instance: Option<modkit_core::ModelInstance>,
    materials: BTreeMap<String, MaterialData>,
    gibs: BTreeMap<String, Vec<modkit_core::Surface>>,
    faces: faces::Faces,
    actor: Option<hl2_simulation::actors::PreparedActor>,
}

fn load_gibs_and_materials(
    mini_world: &mut modkit_core::World,
    vfs: &source_assets::vpk::Vfs,
    known: &BTreeSet<String>,
    (root_lod, picmip, hdr, allow_bc): (usize, i32, bool, bool),
) -> (
    BTreeMap<String, Vec<modkit_core::Surface>>,
    BTreeMap<String, MaterialData>,
) {
    let mut gibs = BTreeMap::new();
    for (path, skin) in crate::effects::gib_models(mini_world) {
        match crate::effects::read_effect_model_lod(vfs, &path, skin, root_lod) {
            Ok(surfaces) => {
                gibs.insert(hl2_simulation::gibs::render_key(&path, skin), surfaces);
            }
            Err(e) => mini_world.warnings.push(format!("{path}: {e:#}")),
        }
    }
    let material_names: BTreeSet<String> = names::rendered_material_names(mini_world)
        .into_iter()
        .chain(gibs.values().flatten().map(|s| s.material.clone()))
        .filter(|m| !known.contains(m))
        .collect();
    let (mut tex_cache, mut frame_cache, mut decoded, mut errors) =
        (BTreeMap::new(), BTreeMap::new(), 0usize, Vec::new());
    let mut materials = BTreeMap::new();
    for name in material_names {
        let material = load_material(
            vfs,
            &name,
            &mut tex_cache,
            &mut frame_cache,
            &mut decoded,
            &mut errors,
            (picmip, hdr, allow_bc),
        );
        materials.insert(name, material);
    }
    mini_world.warnings.extend(errors);
    (gibs, materials)
}

/// Prepare one unpooled NPC class on a worker thread: entity template, model, rig,
/// ragdoll, locomotion clips, gibs, missing materials and face/eye metadata.
pub(crate) fn prepare_npc_template(
    vfs: &source_assets::vpk::Vfs,
    map_name: &str,
    label: &str,
    known: &BTreeSet<String>,
    config: &hl2_ui::config::Config,
    allow_bc: bool,
) -> anyhow::Result<PreparedNpc> {
    let template = hl2_simulation::ai::spawn::template_entity(label, 0)
        .with_context(|| format!("unknown NPC class {label}"))?;
    let mut mini_world = modkit_core::World {
        name: map_name.to_owned(),
        entities: vec![template],
        ..Default::default()
    };
    hl2_simulation::citizens::select_models(&mut mini_world, hl2_simulation::ai::spawn::POOL_KEY);
    let root_lod = config.video.root_lod();
    let report = source_assets::models::append_models_lod(&mut mini_world, vfs, root_lod);
    if mini_world.model_assets.is_empty() {
        anyhow::bail!("{label}: {}", report.errors.join("; "));
    }
    let _ = source_assets::models::append_break_physics(&mut mini_world, vfs);
    let actor = match hl2_simulation::actors::prepare_single_actor(&mut mini_world, vfs, 0) {
        Ok(actor) => actor,
        Err(e) => {
            mini_world.warnings.push(format!("NPC {label}: {e:#}"));
            None
        }
    };
    let (gibs, materials) = load_gibs_and_materials(
        &mut mini_world,
        vfs,
        known,
        (
            root_lod,
            config.video.picmip(),
            config.video.hdr.is_hdr(),
            allow_bc,
        ),
    );
    let mut face_errors = Vec::new();
    let faces = faces::load(vfs, &mini_world, &mut face_errors);
    mini_world.warnings.extend(face_errors);
    let template = mini_world.entities.remove(0);
    let instance = mini_world.model_instances.pop();
    Ok(PreparedNpc {
        world: mini_world,
        template,
        instance,
        materials,
        gibs,
        faces,
        actor,
    })
}

#[derive(SystemParam)]
pub struct HostState<'w> {
    game: ResMut<'w, Gameplay>,
    sim: ResMut<'w, Simulation>,
    console: ResMut<'w, Console>,
    options: Option<Res<'w, crate::Options>>,
    texture_caps: Option<Res<'w, crate::TextureCaps>>,
}

#[derive(SystemParam)]
pub struct RenderAssets<'w> {
    cache: ResMut<'w, ModelCache>,
    flex_models: ResMut<'w, FlexModels>,
    eyes: ResMut<'w, Eyes>,
    effects: Option<ResMut<'w, Effects>>,
    meshes: ResMut<'w, Assets<Mesh>>,
    materials: ResMut<'w, Assets<SourceMaterial>>,
    images: ResMut<'w, Assets<Image>>,
    inverse_binds: ResMut<'w, Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>,
}

fn enqueue_requests(npcs: &mut RuntimeNpcs, host: &mut HostState<'_>, render: &RenderAssets<'_>) {
    for pending in std::mem::take(&mut host.game.npc_requests) {
        let label = pending.request.pool_label().to_owned();
        if !hl2_simulation::ai::spawn::templates(&host.game.world, &label).is_empty() {
            let msg = host
                .game
                .finish_late_spawn(pending, &mut host.sim.physics, 0.);
            host.console.source.log(msg);
            continue;
        }
        if let Some(job) = npcs.jobs.get_mut(&label) {
            job.waiters.push(pending);
            continue;
        }
        let slot: Slot = Arc::default();
        let vfs = npcs.vfs.clone();
        let map_name = host.game.world.name.clone();
        let known: BTreeSet<String> = render.cache.definitions.keys().cloned().collect();
        let config = host.console.source.config.clone();
        let allow_bc = host.texture_caps.as_deref().is_some_and(|c| c.bc);
        let class_label = label.clone();
        let out = slot.clone();
        std::thread::spawn(move || {
            let result =
                prepare_npc_template(&vfs, &map_name, &class_label, &known, &config, allow_bc)
                    .map_err(|e| format!("{e:#}"));
            *out.lock().expect("npc precache slot") = Some(result);
        });
        npcs.jobs.insert(
            label,
            Job {
                started: Instant::now(),
                waiters: vec![pending],
                result: slot,
            },
        );
    }
}

fn install_prepared(
    commands: &mut Commands,
    host: &mut HostState<'_>,
    render: &mut RenderAssets<'_>,
    prepared: PreparedNpc,
    waiters: Vec<PendingNpcSpawn>,
    load_ms: f64,
) {
    let cpu_skinning = host.options.as_deref().is_some_and(|o| o.cpu_skinning);
    let filtering = host.console.source.config.video.texture_sampling().0;
    render
        .cache
        .add_materials_filtered(&mut render.images, prepared.materials, filtering);
    if let Some(effects) = render.effects.as_deref_mut() {
        effects.add_models(
            prepared.gibs,
            &render.cache,
            &mut render.meshes,
            &mut render.materials,
        );
    }
    for (k, v) in prepared.faces.flexes {
        render
            .cache
            .faces
            .entry(k.clone())
            .or_insert_with(|| v.clone());
        render.flex_models.0.entry(k).or_insert(v);
    }
    host.game.merge_model(prepared.world);
    let template_id = host
        .game
        .install_pool_template(prepared.template, prepared.instance);
    if let Some(actor) = prepared.actor {
        let game = &mut *host.game;
        if let Err(e) = hl2_simulation::actors::register_prepared_actor(
            &mut game.npcs,
            &game.world,
            template_id,
            actor,
        ) {
            Arc::make_mut(&mut game.world)
                .warnings
                .push(format!("NPC template {template_id}: {e:#}"));
        }
    }
    if let Some(instance) = host
        .game
        .world
        .model_instances
        .iter()
        .find(|i| i.entity == Some(template_id))
        && hl2_simulation::attention::is_flex_actor(host.game.world.entities[template_id].class())
        && let Some(gaze) = prepared.faces.gaze.get(&instance.asset_key())
    {
        render.eyes.insert_actor(
            template_id,
            instance.asset_key(),
            gaze.clone(),
            instance.scale,
        );
    }
    {
        let world = host.game.world.clone();
        build_npc_template_draws(
            commands,
            TemplateDrawAssets {
                meshes: &mut render.meshes,
                materials: &mut render.materials,
                inverse_binds: &mut render.inverse_binds,
            },
            TemplateDrawWorld {
                cache: &render.cache,
                world: &world,
                ai: &host.game.ai,
                eyes: &prepared.faces.eyes,
                cpu_skinning,
            },
            template_id,
        );
    }
    for pending in waiters {
        let msg = host
            .game
            .finish_late_spawn(pending, &mut host.sim.physics, load_ms);
        host.console.source.log(msg);
    }
}

/// Enqueue new NPC late-precache requests and install completed templates on the main thread.
pub fn poll(
    mut commands: Commands,
    mut npcs: ResMut<RuntimeNpcs>,
    mut host: HostState<'_>,
    mut render: RenderAssets<'_>,
) {
    if host.game.npc_requests.is_empty() && npcs.jobs.is_empty() {
        return;
    }
    enqueue_requests(&mut npcs, &mut host, &render);
    let mut pending_jobs = BTreeMap::new();
    for (label, job) in std::mem::take(&mut npcs.jobs) {
        let finished = match job.result.lock() {
            Ok(mut slot) => Ok(slot.take()),
            Err(_) => Err(()),
        };
        let result = match finished {
            Ok(Some(result)) => result,
            Ok(None) => {
                pending_jobs.insert(label, job);
                continue;
            }
            Err(_) => Err("npc precache worker stopped".into()),
        };
        let load_ms = job.started.elapsed().as_secs_f64() * 1000.;
        match result {
            Ok(prepared) => {
                install_prepared(
                    &mut commands,
                    &mut host,
                    &mut render,
                    prepared,
                    job.waiters,
                    load_ms,
                );
            }
            Err(e) => {
                for pending in job.waiters {
                    let message = format!("Can't create {}: {e}", pending.request.classname);
                    host.game.spawn_log.push(serde_json::json!({
                        "request": pending.request,
                        "placement": Some(pending.placement),
                        "entity": Option::<usize>::None,
                        "message": message,
                        "load_ms": load_ms,
                    }));
                    host.console.source.log(message);
                }
            }
        }
    }
    npcs.jobs = pending_jobs;
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
    use hl2_simulation::ai::spawn::SpawnRequest;
    use modkit_core::{Surface, Vertex, World};

    fn test_app(world: World) -> App {
        let mut app = App::new();
        app.insert_resource(Assets::<Mesh>::default());
        app.insert_resource(Assets::<SourceMaterial>::default());
        app.insert_resource(Assets::<Image>::default());
        app.insert_resource(Assets::<SkinnedMeshInverseBindposes>::default());
        app.insert_resource(crate::material_slots::MaterialSlots::default());
        app.insert_resource(ModelCache::default());
        app.insert_resource(FlexModels::default());
        app.insert_resource(Eyes::default());
        app.insert_resource(Gameplay::synthetic(world.clone()));
        app.insert_resource(Simulation::new(
            &world,
            glam::Vec3::new(0., 0., 64.),
            0.,
            0.,
            false,
            Vec::new(),
        ));
        app.insert_resource(Console::new(Default::default()));
        app.insert_resource(RuntimeNpcs::new(Arc::new(
            source_assets::vpk::Vfs::default(),
        )));
        app.add_systems(
            Update,
            (
                poll.before(crate::rendering::spawn_runtime_draws),
                crate::rendering::spawn_runtime_draws,
            ),
        );
        app
    }

    fn synthetic_prepared_npc(class: &str, model: &str) -> PreparedNpc {
        let mut mini_world = World::default();
        let triangle = Surface {
            background: false,
            material: "models/test/vort".into(),
            lightmap: None,
            flex_source: None,
            vertices: [
                glam::Vec3::new(0., 0., 0.),
                glam::Vec3::new(8., 0., 0.),
                glam::Vec3::new(0., 0., 16.),
            ]
            .into_iter()
            .map(|position| Vertex {
                position,
                uv: glam::Vec2::ZERO,
                color: [255; 4],
                light_uv: glam::Vec2::ZERO,
                skin: None,
                normal: glam::Vec3::X,
            })
            .collect(),
            indices: vec![0, 1, 2],
        };
        mini_world
            .model_assets
            .insert(format!("{model}#0"), vec![triangle]);
        let template = hl2_simulation::ai::spawn::template_entity(class, 0).unwrap();
        let instance = modkit_core::ModelInstance {
            background: false,
            model: model.into(),
            origin: hl2_simulation::ai::spawn::POOL_ORIGIN,
            angles: glam::Vec3::ZERO,
            skin: 0,
            body: 0,
            scale: 1.,
            kind: class.into(),
            solid: true,
            solid_mode: None,
            entity: Some(0),
        };
        PreparedNpc {
            world: mini_world,
            template,
            instance: Some(instance),
            materials: BTreeMap::from([("models/test/vort".to_owned(), MaterialData::default())]),
            gibs: BTreeMap::new(),
            faces: faces::Faces::default(),
            actor: None,
        }
    }

    fn request_spawn_in_app(app: &mut App, req: SpawnRequest) -> String {
        app.world_mut()
            .resource_scope(|world, mut game: Mut<Gameplay>| {
                let mut sim = world.resource_mut::<Simulation>();
                game.request_spawn(req, &mut sim.physics, glam::Vec3::Z * 64., glam::Vec3::X)
            })
    }

    #[test]
    fn completed_late_precache_installs_template_and_spawns_draws_in_one_update() {
        let mut app = test_app(World::default());
        let request = SpawnRequest {
            classname: "npc_vortigaunt".into(),
            name: Some("menu_vort".into()),
            aimed: true,
            ..Default::default()
        };
        let msg = request_spawn_in_app(&mut app, request);
        assert_eq!(msg, "Loading npc_vortigaunt...");
        assert_eq!(app.world().resource::<Gameplay>().npc_requests.len(), 1);
        let waiter = std::mem::take(&mut app.world_mut().resource_mut::<Gameplay>().npc_requests);
        let slot: Slot = Arc::new(Mutex::new(Some(Ok(synthetic_prepared_npc(
            "npc_vortigaunt",
            "models/vortigaunt.mdl",
        )))));
        app.world_mut().resource_mut::<RuntimeNpcs>().jobs.insert(
            "npc_vortigaunt".into(),
            Job {
                started: Instant::now(),
                waiters: waiter,
                result: slot,
            },
        );
        app.update();
        let game = app.world().resource::<Gameplay>();
        assert_eq!(game.world_copies, 0);
        assert_eq!(game.runtime_entities.len(), 1);
        let spawned_id = game.runtime_entities[0];
        let templates = hl2_simulation::ai::spawn::templates(&game.world, "npc_vortigaunt");
        assert_eq!(templates.len(), 1);
        let template_id = templates[0];
        assert!(!game.scene.states[template_id].visible);
        assert!(game.scene.states[spawned_id].visible);
        assert_eq!(
            game.world.entities[spawned_id].get("targetname"),
            Some("menu_vort")
        );
        assert!(game.render_spawns.is_empty());
        let console = app.world().resource::<Console>();
        assert!(
            console
                .source
                .shell
                .panel
                .output
                .iter()
                .any(|line| line.contains("Created npc_vortigaunt"))
        );
        let mut draw_query = app.world_mut().query::<(
            &crate::rendering::SourceEntity,
            &Mesh3d,
            Option<&Visibility>,
        )>();
        let draws: Vec<(usize, Option<Visibility>)> = draw_query
            .iter(app.world())
            .map(|(s, _, v)| (s.0, v.copied()))
            .collect();
        assert!(
            draws
                .iter()
                .any(|(id, v)| *id == template_id && *v == Some(Visibility::Hidden))
        );
        assert!(draws.iter().any(|(id, _)| *id == spawned_id));
    }

    #[test]
    fn duplicate_concurrent_requests_coalesce_and_reject_unknown_classes() {
        let mut app = test_app(World::default());
        let unknown = SpawnRequest {
            classname: "npc_not_real".into(),
            ..Default::default()
        };
        let unknown_msg = request_spawn_in_app(&mut app, unknown);
        assert!(unknown_msg.contains("the class was not loaded on this map"));
        assert!(app.world().resource::<Gameplay>().npc_requests.is_empty());

        let slot: Slot = Arc::default();
        app.world_mut().resource_mut::<RuntimeNpcs>().jobs.insert(
            "npc_vortigaunt".into(),
            Job {
                started: Instant::now(),
                waiters: Vec::new(),
                result: slot.clone(),
            },
        );
        for name in ["v1", "v2"] {
            let req = SpawnRequest {
                classname: "npc_vortigaunt".into(),
                name: Some(name.into()),
                ..Default::default()
            };
            let out = request_spawn_in_app(&mut app, req);
            assert_eq!(out, "Loading npc_vortigaunt...");
        }
        app.update();
        {
            let npcs = app.world().resource::<RuntimeNpcs>();
            assert_eq!(npcs.jobs.len(), 1);
            assert_eq!(npcs.jobs["npc_vortigaunt"].waiters.len(), 2);
            assert!(app.world().resource::<Gameplay>().npc_requests.is_empty());
        }
        *slot.lock().unwrap() = Some(Err("simulated decode error".into()));
        app.update();
        assert!(app.world().resource::<RuntimeNpcs>().jobs.is_empty());
        let game = app.world().resource::<Gameplay>();
        assert!(game.runtime_entities.is_empty());
        assert_eq!(game.spawn_log.len(), 3);
        let console = app.world().resource::<Console>();
        let errors = console
            .source
            .shell
            .panel
            .output
            .iter()
            .filter(|l| l.contains("simulated decode error"))
            .count();
        assert_eq!(errors, 2);
    }

    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn owned_prepare_npc_template_loads_vortigaunt_and_combine_without_errors() {
        let game_dir = source_assets::install::discover().unwrap();
        let vfs = source_assets::vpk::Vfs::mount_game(&game_dir, "hl2").unwrap();
        let config = hl2_ui::config::Config::default();
        let known = BTreeSet::new();
        for class in ["npc_vortigaunt", "npc_combine_s"] {
            let prepared =
                prepare_npc_template(&vfs, "background02", class, &known, &config, false).unwrap();
            assert!(!prepared.world.model_assets.is_empty(), "{class}: meshes");
            assert!(!prepared.world.rigs.is_empty(), "{class}: rig");
            assert!(!prepared.materials.is_empty(), "{class}: materials");
            assert!(prepared.actor.is_some(), "{class}: prepared actor");
        }
    }
}
