//! prop_physics_create (SDK props.cpp CC_Prop_Physics_Create -> CreatePhysicsProp) for any
//! installed model (DESIGN 14r part B). A model the map already loaded spawns at once;
//! another loads on a worker thread through the map's model loader (late precache: the
//! native engine loads it synchronously and hitches; ours appears a few frames later)
//! and is inserted when ready.
use crate::{
    assets::PreparedModel,
    gameplay::Gameplay,
    rendering::{ModelCache, SourceMaterial},
    simulation::Simulation,
    ui_host::Console,
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
pub(crate) struct Request {
    pub model: String,
    pub placement: ModelPlacement,
    pub pickup: Option<modkit_core::Entity>,
}

#[derive(Clone, Copy)]
pub(crate) enum ModelPlacement {
    Aim(glam::Vec3, glam::Vec3),
    At(glam::Vec3),
}

impl Gameplay {
    /// Queue a model entity from a supplied world position using the normal asset worker.
    pub fn queue_model_at(
        &mut self,
        entity: modkit_core::Entity,
        origin: glam::Vec3,
    ) -> Result<(), String> {
        self.queue_model(entity, ModelPlacement::At(origin))
    }

    pub(crate) fn queue_model(
        &mut self,
        entity: modkit_core::Entity,
        placement: ModelPlacement,
    ) -> Result<(), String> {
        let class = entity.class();
        let model = if class == "prop_physics" {
            entity.get("model").map(model_path)
        } else if source_assets::models::physical_pickup(class) {
            source_assets::models::supply_model(class)
                .map(str::to_owned)
                .or_else(|| self.weapons.get(class).map(|w| w.world_model.clone()))
                .filter(|model| !model.is_empty())
                .or_else(|| self.ai.weapon_model(class))
        } else {
            None
        };
        let model = model.ok_or_else(|| format!("Can't create {class}: no supported model"))?;
        self.pickup_requests.push(Request {
            model,
            placement,
            pickup: Some(entity),
        });
        Ok(())
    }
}

struct Job {
    request: Request,
    started: std::time::Instant,
    result: Slot,
}
impl RuntimeProps {
    /// The map's mounted content.
    pub fn vfs(&self) -> &Arc<source_assets::vpk::Vfs> {
        &self.vfs
    }
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
/// A prop request's load: on a worker, unless a map prop_physics already brought the model in.
fn start_job(props: &RuntimeProps, game: &Gameplay, cache: &ModelCache, request: Request) -> Job {
    let slot: Slot = Arc::default();
    let key = format!("{}#0", request.model);
    if game.world.model_physics.contains_key(&key)
        || request
            .pickup
            .as_ref()
            .is_some_and(|e| e.class() != "prop_physics")
            && game.world.model_assets.contains_key(&key)
    {
        *slot.lock().expect("prop slot") = Some(Err(String::new()));
    } else {
        let vfs = props.vfs.clone();
        let known = cache.definitions.keys().cloned().collect();
        let name = request.model.clone();
        let class = request
            .pickup
            .as_ref()
            .map_or("prop_physics", |e| e.class())
            .to_owned();
        let out = slot.clone();
        std::thread::spawn(move || {
            let result = crate::assets::prepare_model(&vfs, &name, &class, &known)
                .map_err(|e| format!("{e:#}"));
            *out.lock().expect("prop slot") = Some(result);
        });
    }
    Job {
        request,
        started: std::time::Instant::now(),
        result: slot,
    }
}
fn queue_jobs(props: &mut RuntimeProps, game: &mut Gameplay, cache: &ModelCache) {
    let props_to_spawn =
        std::mem::take(&mut game.prop_requests)
            .into_iter()
            .map(|(model, eye, forward)| Request {
                model,
                placement: ModelPlacement::Aim(eye, forward),
                pickup: None,
            });
    for request in props_to_spawn.chain(std::mem::take(&mut game.pickup_requests)) {
        let job = start_job(props, game, cache, request);
        props.jobs.push(job);
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
    mut console: ResMut<Console>,
    mut effects: ResMut<crate::effects::Effects>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SourceMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut inverse_binds: ResMut<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>,
) {
    queue_jobs(&mut props, &mut game, &cache);
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
            Err(e) => Err(format!("Can't create {}: {e}", job.request.model)),
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
                let class = world.entities[id].class();
                let body = sim.physics.entity_mass(id);
                let name = world.entities[id].get("targetname");
                game.spawn_log.push(serde_json::json!({
                    "prop":job.request.model,"class":class,"name":name,"entity":id,"draws":draws,
                    "load_ms":load_ms,"origin":origin.to_array(),"mass":body
                }));
                format!(
                    "Created {class} {} at {:.0} {:.0} {:.0} (entity {id}, {load_ms:.0} ms)",
                    job.request.model, origin.x, origin.y, origin.z
                )
            }
            Err(e) => {
                game.spawn_log.push(
                    serde_json::json!({"prop":job.request.model,"error":e,"load_ms":load_ms}),
                );
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
    if let Some(entity) = job.request.pickup.clone() {
        if entity.class() == "prop_physics"
            && !game
                .world
                .model_physics
                .contains_key(&format!("{}#0", job.request.model))
        {
            return Err(format!(
                "Can't create {}: the model has no physics (.phy)",
                job.request.model
            ));
        }
        return create_model_entity(game, &mut sim.physics, &job.request, entity);
    }
    let key = format!("{}#0", job.request.model);
    if !game.world.model_physics.contains_key(&key) {
        return Err(format!(
            "Can't create {}: the model has no physics (.phy)",
            job.request.model
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
    let ModelPlacement::Aim(eye, forward) = job.request.placement else {
        return Err("Prop request has no aim placement".into());
    };
    let end = eye + forward * hl2_simulation::ai::spawn::MAX_TRACE_LENGTH;
    let trace = sim
        .physics
        .npc_trace_hull(eye, end, Hull { mins, maxs }, Query::default());
    if trace.trace.fraction >= 1. || trace.trace.all_solid {
        return Err(format!(
            "Can't create {}: no surface along the aim",
            job.request.model
        ));
    }
    let origin = eye + (end - eye) * trace.trace.fraction + trace.trace.normal;
    let id = game.world.entities.len();
    let entity = modkit_core::Entity {
        properties: [
            ("classname", "prop_physics".to_owned()),
            (
                "origin",
                format!("{:.10} {:.10} {:.10}", origin.x, origin.y, origin.z),
            ),
            ("angles", "0 0 0".to_owned()),
            ("model", job.request.model.clone()),
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
        model: job.request.model.clone(),
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
        .map_err(|e| format!("Can't create {}: {e}", job.request.model))
}

/// Install a loose model at its supplied origin or at the pickup aim ray's hit.
fn create_model_entity(
    game: &mut Gameplay,
    physics: &mut hl2_simulation::physics::Physics,
    request: &Request,
    mut entity: modkit_core::Entity,
) -> Result<usize, String> {
    let origin = match request.placement {
        ModelPlacement::At(origin) => origin,
        ModelPlacement::Aim(eye, forward) => {
            let hit = physics
                .impact_ray(eye, forward, hl2_simulation::ai::spawn::MAX_TRACE_LENGTH)
                .ok_or("No surface along the aim")?;
            hit.position + glam::Vec3::Z * 12.
        }
    };
    let angles = entity
        .get("angles")
        .and_then(modkit_core::parse_vec3)
        .unwrap_or_default();
    let instance = modkit_core::ModelInstance {
        background: false,
        model: request.model.clone(),
        origin,
        angles,
        skin: 0,
        body: 0,
        scale: 1.,
        kind: entity.class().into(),
        solid: true,
        solid_mode: None,
        entity: Some(game.world.entities.len()),
    };
    entity.properties.retain(|(key, _)| {
        !key.eq_ignore_ascii_case("origin") && !key.eq_ignore_ascii_case("model")
    });
    entity.properties.extend([
        (
            "origin".into(),
            format!("{} {} {}", origin.x, origin.y, origin.z),
        ),
        ("model".into(), request.model.clone()),
    ]);
    game.insert_entity(entity, Some(instance), physics)
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
