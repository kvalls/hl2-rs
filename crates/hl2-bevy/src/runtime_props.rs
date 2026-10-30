//! prop_physics_create (SDK props.cpp CC_Prop_Physics_Create -> CreatePhysicsProp) for any
//! installed model (DESIGN 14r part B). A model the map already loaded spawns at once;
//! another loads on a worker thread through the map's model loader (late precache: the
//! native engine loads it synchronously and hitches; ours appears a few frames later)
//! and is inserted when ready.
use crate::{
    assets::PreparedModel,
    gameplay::Gameplay,
    movement::Simulation,
    rendering::{ModelCache, SourceMaterial},
};
use bevy::prelude::*;
use hl2_simulation::npc_probe::{Hull, NpcCollisionWorld, Query};
use std::sync::{Arc, Mutex};

type Slot = Arc<Mutex<Option<Result<PreparedModel, String>>>>;

/// The map's content for worker loads and the loads in flight (outcomes go to the
/// gameplay spawn log).
#[derive(Resource)]
pub struct RuntimeProps {
    vfs: Arc<source_assets::vpk::Vfs>,
    jobs: Vec<Job>,
}
struct Job {
    model: String,
    eye: glam::Vec3,
    forward: glam::Vec3,
    started: std::time::Instant,
    result: Slot,
}
impl RuntimeProps {
    pub fn new(vfs: Arc<source_assets::vpk::Vfs>) -> Self {
        Self {
            vfs,
            jobs: Vec::new(),
        }
    }
}
/// CC_Prop_Physics_Create's model name: "models/" + the argument, ".mdl" by default.
pub fn model_path(argument: &str) -> String {
    let name = argument.trim().replace('\\', "/").to_lowercase();
    let name = name.strip_prefix("models/").unwrap_or(&name);
    if name.ends_with(".mdl") {
        format!("models/{name}")
    } else {
        format!("models/{name}.mdl")
    }
}
/// Queued requests start loading; finished loads are merged, placed and drawn.
#[allow(clippy::too_many_arguments)]
pub fn poll(
    mut commands: Commands,
    mut props: ResMut<RuntimeProps>,
    mut game: ResMut<Gameplay>,
    mut sim: ResMut<Simulation>,
    mut cache: ResMut<ModelCache>,
    mut console: ResMut<crate::console::Console>,
    mut effects: ResMut<crate::effects::Effects>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SourceMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut inverse_binds: ResMut<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>,
) {
    for (model, eye, forward) in std::mem::take(&mut game.prop_requests) {
        let slot: Slot = Arc::default();
        if game.world.model_physics.contains_key(&format!("{model}#0")) {
            // Already loaded (a map prop_physics uses it): no worker.
            *slot.lock().expect("prop slot") = Some(Err(String::new()));
        } else {
            let vfs = props.vfs.clone();
            let known = cache.definitions.keys().cloned().collect();
            let name = model.clone();
            let out = slot.clone();
            std::thread::spawn(move || {
                let result = crate::assets::prepare_model(&vfs, &name, "prop_physics", &known)
                    .map_err(|e| format!("{e:#}"));
                *out.lock().expect("prop slot") = Some(result);
            });
        }
        props.jobs.push(Job {
            model,
            eye,
            forward,
            started: std::time::Instant::now(),
            result: slot,
        });
    }
    for id in std::mem::take(&mut game.model_draws) {
        let world = game.world.clone();
        let state = &game.scene.states[id];
        let transform = crate::rendering::entity_transform(state.origin, state.rotation);
        crate::rendering::build_model_draws(
            &mut commands,
            (&mut meshes, &mut materials, &mut inverse_binds),
            &cache,
            &world,
            id,
            transform,
            None,
        );
    }
    let mut pending = Vec::new();
    for job in std::mem::take(&mut props.jobs) {
        let finished = match job.result.lock() {
            Ok(mut slot) => Ok(slot.take()),
            Err(_) => Err(()),
        };
        let result = match finished {
            Ok(Some(result)) => result,
            Ok(None) => {
                pending.push(job);
                continue;
            }
            Err(_) => Err("model loader stopped".into()),
        };
        let load_ms = job.started.elapsed().as_secs_f64() * 1000.;
        let message = match result {
            Ok(prepared) => {
                cache.add_materials(&mut images, prepared.materials);
                effects.add_models(prepared.gibs, &cache, &mut meshes, &mut materials);
                game.merge_model(prepared.world);
                create(&mut game, &mut sim, &job, Some(prepared.hull))
            }
            // Already loaded.
            Err(e) if e.is_empty() => create(&mut game, &mut sim, &job, None),
            Err(e) => Err(format!("Can't create {}: {e}", job.model)),
        };
        let message = match message {
            Ok(id) => {
                let world = game.world.clone();
                let (origin, rotation) =
                    (game.scene.states[id].origin, game.scene.states[id].rotation);
                let transform = crate::rendering::entity_transform(origin, rotation);
                let draws = crate::rendering::build_model_draws(
                    &mut commands,
                    (&mut meshes, &mut materials, &mut inverse_binds),
                    &cache,
                    &world,
                    id,
                    transform,
                    None,
                );
                game.spawn_log.push(serde_json::json!({"prop":job.model,"entity":id,"draws":draws,"load_ms":load_ms,"origin":origin.to_array()}));
                format!(
                    "Created prop_physics {} at {:.0} {:.0} {:.0} (entity {id}, {load_ms:.0} ms)",
                    job.model, origin.x, origin.y, origin.z
                )
            }
            Err(e) => {
                game.spawn_log
                    .push(serde_json::json!({"prop":job.model,"error":e,"load_ms":load_ms}));
                e
            }
        };
        console.source.log(message);
    }
    props.jobs = pending;
}
/// CreatePhysicsProp: the model needs a .phy; its studio hull sweeps from the eye along
/// the aim (MASK_NPCSOLID); no hit (or all solid) creates nothing; the prop goes 1 unit
/// off the hit surface at angles 0 with the command's keyvalues.
fn create(
    game: &mut Gameplay,
    sim: &mut Simulation,
    job: &Job,
    hull: Option<(glam::Vec3, glam::Vec3)>,
) -> Result<usize, String> {
    let key = format!("{}#0", job.model);
    if !game.world.model_physics.contains_key(&key) {
        return Err(format!(
            "Can't create {}: the model has no physics (.phy)",
            job.model
        ));
    }
    let (mins, maxs) = match hull {
        Some(hull) => hull,
        None => game
            .world
            .model_assets
            .get(&key)
            .map(|surfaces| {
                surfaces.iter().flat_map(|s| &s.vertices).fold(
                    (glam::Vec3::splat(f32::MAX), glam::Vec3::splat(f32::MIN)),
                    |(lo, hi), v| (lo.min(v.position), hi.max(v.position)),
                )
            })
            .ok_or("model missing")?,
    };
    let end = job.eye + job.forward * hl2_simulation::ai::spawn::MAX_TRACE_LENGTH;
    let trace = sim
        .physics
        .npc_trace_hull(job.eye, end, Hull { mins, maxs }, Query::default());
    if trace.trace.fraction >= 1. || trace.trace.all_solid {
        return Err(format!(
            "Can't create {}: no surface along the aim",
            job.model
        ));
    }
    let origin = job.eye + (end - job.eye) * trace.trace.fraction + trace.trace.normal;
    let id = game.world.entities.len();
    let entity = modkit_core::Entity {
        properties: [
            ("classname", "prop_physics".to_owned()),
            (
                "origin",
                format!("{:.10} {:.10} {:.10}", origin.x, origin.y, origin.z),
            ),
            ("angles", "0 0 0".to_owned()),
            ("model", job.model.clone()),
            ("fademindist", "-1".to_owned()),
            ("fademaxdist", "0".to_owned()),
            ("fadescale", "1".to_owned()),
            ("inertiaScale", "1.0".to_owned()),
            ("physdamagescale", "0.1".to_owned()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect(),
    };
    let instance = modkit_core::ModelInstance {
        background: false,
        model: job.model.clone(),
        origin,
        angles: glam::Vec3::ZERO,
        skin: 0,
        body: 0,
        scale: 1.,
        kind: "prop_physics".into(),
        solid: true,
        solid_mode: None,
        entity: Some(id),
    };
    game.insert_entity(entity, Some(instance), &mut sim.physics)
        .map_err(|e| format!("Can't create {}: {e}", job.model))
}

#[cfg(test)]
mod tests {
    use super::model_path;
    #[test]
    fn prop_physics_create_names_models_like_the_sdk() {
        // "models/" + the argument, ".mdl" by default (Q_DefaultExtension).
        assert_eq!(
            model_path("props_c17/oildrum001"),
            "models/props_c17/oildrum001.mdl"
        );
        assert_eq!(
            model_path(r"Props_Junk\Watermelon01.mdl"),
            "models/props_junk/watermelon01.mdl"
        );
        assert_eq!(
            model_path("models/props_junk/popcan01a.mdl"),
            "models/props_junk/popcan01a.mdl"
        );
    }
}
