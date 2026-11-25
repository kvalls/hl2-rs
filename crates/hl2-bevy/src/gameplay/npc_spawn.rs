//! Runtime NPC template spawning and pending late-precache requests.
use super::Gameplay;
use glam::Vec3;
use hl2_simulation::{
    ai::spawn::{self, Placement, SpawnRequest},
    physics::Physics,
};
use modkit_core::World;
use std::{collections::BTreeMap, sync::Arc};

/// A validated spawn request waiting for its NPC class to finish loading off-thread.
#[derive(Clone, Debug)]
pub struct PendingNpcSpawn {
    pub request: SpawnRequest,
    pub placement: Placement,
}

/// The weapons each F4 class can be given: `Weapon_CanUse` on its pooled spawn's
/// loaded model (every activity of the model, loaded or not).
pub fn spawn_usable_weapons(
    world: &World,
    ai: &hl2_simulation::ai::runtime::AiRuntime,
) -> BTreeMap<String, Vec<String>> {
    spawn::SPAWN_CLASSES
        .iter()
        .filter_map(|class| {
            let id = spawn::free_slot(world, class, &Default::default())?;
            let class_name = spawn::spawn_class(class).0;
            let instance = world
                .model_instances
                .iter()
                .find(|i| i.entity == Some(id))?;
            let rig = world.rigs.get(&instance.asset_key())?;
            Some((
                (*class).to_owned(),
                ai.usable_weapons(class_name, &rig.activities),
            ))
        })
        .collect()
}

fn format_created(
    request: &SpawnRequest,
    p: &Placement,
    id: usize,
    load_ms: Option<f64>,
) -> String {
    let weapon = if request.equipment.is_empty() {
        "no weapon"
    } else {
        &request.equipment
    };
    match load_ms {
        Some(ms) => format!(
            "Created {} ({weapon}) at {:.0} {:.0} {:.0} yaw {:.0} (entity {id}, {ms:.0} ms)",
            request.classname, p.origin.x, p.origin.y, p.origin.z, p.yaw
        ),
        None => format!(
            "Created {} ({weapon}) at {:.0} {:.0} {:.0} yaw {:.0} (entity {id})",
            request.classname, p.origin.x, p.origin.y, p.origin.z, p.yaw
        ),
    }
}

impl Gameplay {
    /// An `npc_create` request (F4 Enter or the console): `CC_NPC_Create` placement from
    /// the eye along the aim, then a new entity copied from the class's hidden template
    /// (or queued for asynchronous late precaching when the class has no pool template yet).
    pub fn request_spawn(
        &mut self,
        request: SpawnRequest,
        physics: &mut Physics,
        eye: Vec3,
        forward: Vec3,
    ) -> String {
        let placement = spawn::place(
            &request,
            eye,
            forward,
            &hl2_simulation::ai::host::PhysicsPlacement { physics },
        );
        match placement {
            Err(e) => {
                self.spawn_log.push(serde_json::json!({
                    "request": request,
                    "placement": Option::<Placement>::None,
                    "entity": Option::<usize>::None,
                    "message": e,
                }));
                e
            }
            Ok(placement) => self.request_spawn_at(request, physics, placement),
        }
    }

    /// Instantiate at a supplied world pose through the usual template and late-precache paths.
    pub fn request_spawn_at(
        &mut self,
        request: SpawnRequest,
        physics: &mut Physics,
        p: Placement,
    ) -> String {
        let label = request.pool_label();
        if !spawn::templates(&self.world, label).is_empty() {
            self.complete_spawn(&request, &p, physics, None)
        } else if spawn::can_late_precache(label) {
            let msg = format!("Loading {}...", request.classname);
            self.npc_requests.push(PendingNpcSpawn {
                request,
                placement: p,
            });
            msg
        } else {
            let msg = format!(
                "Can't create {}: the class was not loaded on this map",
                request.classname
            );
            self.spawn_log.push(serde_json::json!({
                "request": request,
                "placement": Some(p),
                "entity": Option::<usize>::None,
                "message": msg,
            }));
            msg
        }
    }

    /// Install a late-precached hidden pool template entity (never added to AI or physics).
    pub(crate) fn install_pool_template(
        &mut self,
        entity: modkit_core::Entity,
        instance: Option<modkit_core::ModelInstance>,
    ) -> usize {
        if Arc::strong_count(&self.world) > 1 {
            self.world_copies += 1;
        }
        let world = Arc::make_mut(&mut self.world);
        let id = self.scene.spawn_entity(world, entity);
        if let Some(mut inst) = instance {
            inst.entity = Some(id);
            world.model_instances.push(inst);
        }
        id
    }

    /// Finish a queued spawn request after its template finished late precaching.
    pub(crate) fn finish_late_spawn(
        &mut self,
        pending: PendingNpcSpawn,
        physics: &mut Physics,
        load_ms: f64,
    ) -> String {
        self.complete_spawn(&pending.request, &pending.placement, physics, Some(load_ms))
    }

    fn complete_spawn(
        &mut self,
        request: &SpawnRequest,
        placement: &Placement,
        physics: &mut Physics,
        load_ms: Option<f64>,
    ) -> String {
        let mut entity = None;
        let message =
            match self.spawn_from_template(request, placement.origin, placement.yaw, physics) {
                Ok(id) => {
                    entity = Some(id);
                    format_created(request, placement, id, load_ms)
                }
                Err(e) => e,
            };
        let mut entry = serde_json::json!({
            "request": request,
            "placement": Some(placement),
            "entity": entity,
            "message": message,
        });
        if let Some(ms) = load_ms {
            entry["load_ms"] = serde_json::json!(ms);
        }
        self.spawn_log.push(entry);
        message
    }

    /// The F4/`npc_create` part: a class template, then the request's weapon, side and grenades.
    fn spawn_from_template(
        &mut self,
        request: &SpawnRequest,
        origin: Vec3,
        yaw: f32,
        physics: &mut Physics,
    ) -> Result<usize, String> {
        let label = request.pool_label().to_owned();
        let templates = spawn::templates(&self.world, &label);
        if templates.is_empty() {
            return Err(format!(
                "Can't create {}: the class was not loaded on this map",
                request.classname
            ));
        }
        let template = if label == "npc_citizen" {
            // CNPC_Citizen::SelectModel: the least-used head among living citizens.
            let roll = self.ai.uniform() * 0.5 + 0.5;
            let states = &self.scene.states;
            hl2_simulation::citizens::least_used_template(
                &self.world,
                &templates,
                spawn::POOL_KEY,
                |id| states.get(id).is_some_and(|s| !s.killed),
                roll,
            )
            .unwrap_or(templates[0])
        } else {
            let count = self.spawn_counts.entry(label).or_default();
            let template = templates[*count % templates.len()];
            *count += 1;
            template
        };
        let id = self
            .create_from_template(template, origin, yaw, &request.keyvalues(), physics)
            .map_err(|e| format!("Can't create {}: {e}", request.classname))?;
        self.ai
            .set_equipment(id, &request.classname, &request.equipment);
        // Its walk/run follow the new weapon (preloaded variants).
        self.npcs.set_locomotion(id, &request.equipment);
        self.ai
            .set_player_disposition(id, request.allegiance.disposition());
        if let Some(grenades) = request.grenades {
            self.ai.set_grenades(id, grenades);
        }
        // The new weapon's idle (Weapon_TranslateActivity of ACT_IDLE); the AI picks its
        // own activities from here when it runs this class.
        let idle = hl2_simulation::entities::weapon_idle_activity(&request.equipment);
        let world = self.world.clone();
        self.scene.ai_set_activity(&world, id, idle);
        Ok(id)
    }
}
