//! Shared actor preparation and fixed-step scene locomotion orchestration.
use crate::{entities::Scene, physics::Physics};
use anyhow::Result;
use modkit_core::{
    movement::{Player, TICK},
    World,
};
use source_assets::vpk::Vfs;

pub fn prepare_choreography_animations(world: &mut World, vfs: &Vfs, scene: &Scene) {
    prepare_actor_animations(world, vfs, scene.required_animation_clips(world));
}
pub fn prepare_actor_animations(
    world: &mut World,
    vfs: &Vfs,
    clips: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
) {
    for (key, wanted) in clips {
        let Some(instance) = world
            .model_instances
            .iter()
            .find(|instance| instance.asset_key() == key)
        else {
            continue;
        };
        match source_assets::animation::load(vfs, &instance.model, &wanted) {
            Ok(rig) => {
                if let Some(current) = world.rigs.get_mut(&key) {
                    let same_skeleton = current.bones.len() == rig.bones.len()
                        && current
                            .bones
                            .iter()
                            .zip(&rig.bones)
                            .all(|(a, b)| a.name == b.name && a.parent == b.parent);
                    if same_skeleton {
                        current.clips.extend(rig.clips);
                        current.merge_sequence_metadata(rig.sequences);
                        current.warnings.extend(rig.warnings);
                    } else {
                        current
                            .warnings
                            .push("choreography clip skeleton differs from loaded model".into());
                    }
                } else {
                    world.rigs.insert(key, rig);
                }
            }
            Err(error) => world.warnings.push(format!(
                "choreography animations {}: {error:#}",
                instance.model
            )),
        }
    }
}
/// NPC classes registered with the shared human ground-movement controller.
pub const GROUND_HUMANS: [&str; 2] = ["npc_barney", "npc_kleiner"];
/// Human-hull NPC classes registered for AI goals only (step 14): scripted scene moves
/// for them stay unsupported as before. npc_metropolice: SetHullType(HULL_HUMAN).
pub const AI_GROUND_HUMANS: [&str; 1] = ["npc_metropolice"];
pub fn prepare_npcs(
    world: &mut World,
    vfs: &Vfs,
    map: &str,
    revision: u32,
) -> crate::npc::Controller {
    let graph = match vfs.read(&format!("maps/graphs/{map}.ain")) {
        Ok(Some(data)) => match source_assets::navigation::Graph::parse(&data, revision) {
            Ok(graph) => Some(graph),
            Err(error) => {
                world.warnings.push(format!("NPC graph: {error:#}"));
                None
            }
        },
        Ok(None) => None,
        Err(error) => {
            world.warnings.push(format!("NPC graph: {error:#}"));
            None
        }
    };
    let restrictions = graph
        .as_ref()
        .map(|g| crate::npc::NavRestrictions::from_world(world, g))
        .unwrap_or_default();
    let mut controller = crate::npc::Controller::new(graph, restrictions);
    controller.set_doors(world);
    let mut clips = std::collections::BTreeMap::new();
    // Normal human hull, step movement and ground/door capabilities are verified per factory:
    // Barney (owned executable) and Kleiner (retail CNPC_Kleiner::Spawn, matching SDK 2013).
    // Other NPC factories need their own hull/motor evidence before being registered here.
    for instance in &world.model_instances {
        let Some(actor) = instance.entity else {
            continue;
        };
        let class = world.entities[actor].class();
        let ai_only = AI_GROUND_HUMANS.contains(&class);
        if !GROUND_HUMANS.contains(&class) && !ai_only {
            continue;
        }
        let loaded = (|| -> anyhow::Result<_> {
            let motion = crate::npc::Locomotion::load(vfs, &instance.model)?;
            let wanted = motion.required_clips();
            let ground = crate::npc::normal_human(instance.scale, 18., 1.)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            if ai_only {
                controller.register_ai_actor(actor, instance.scale, ground, motion)
            } else {
                controller.register_actor(actor, instance.scale, ground, motion)
            }
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            // Verified ordinary Barney eye offset comes from his MDL, not the player's eye.
            if instance.scale == 1. {
                controller
                    .set_view_offset(
                        actor,
                        source_assets::models::read_eye_position(vfs, &instance.model)?,
                    )
                    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            }
            Ok(wanted)
        })();
        match loaded {
            Ok(wanted) => {
                clips.insert(instance.asset_key(), wanted);
            }
            Err(error) => world
                .warnings
                .push(format!("NPC {}: {error:#}", instance.model)),
        }
    }
    prepare_actor_animations(world, vfs, clips);
    controller
}

pub fn tick_npcs(
    controller: &mut crate::npc::Controller,
    scene: &mut Scene,
    world: &World,
    physics: &mut Physics,
    player: &Player,
    fly: bool,
) {
    let poses: Vec<_> = world
        .entities
        .iter()
        .enumerate()
        .filter_map(|(entity, e)| {
            let s = &scene.states[entity];
            if !e.class().starts_with("npc_") || s.killed || !s.visible {
                return None;
            }
            let forward = s.rotation * glam::Vec3::X;
            Some(crate::npc::ActorPose {
                entity,
                feet: s.origin,
                yaw_degrees: forward.y.atan2(forward.x).to_degrees(),
                scripted_by: s.scripted_by.or_else(|| scene.body_sequence_owner(entity)),
            })
        })
        .collect();
    let mut transients: Vec<_> = poses
        .iter()
        .filter_map(|p| {
            controller
                .actor_hull(p.entity)
                .map(|hull| crate::npc_probe::ActorHull {
                    entity: Some(p.entity),
                    feet: p.feet,
                    hull,
                })
        })
        .collect();
    if !fly {
        transients.push(crate::npc_probe::ActorHull {
            entity: None,
            feet: player.feet,
            hull: crate::npc_probe::Hull {
                mins: glam::Vec3::new(-16., -16., 0.),
                maxs: glam::Vec3::new(16., 16., if player.crouched { 36. } else { 72. }),
            },
        });
    }
    for command in scene.movement_commands.drain(..) {
        match command {
            crate::entities::SceneMoveCommand::CancelScene(owner) => controller.cancel_scene(owner),
            crate::entities::SceneMoveCommand::Start { key, request } => {
                if let Some(pose) = poses.iter().find(|p| p.entity == key.actor) {
                    controller.request(key, request, *pose, physics, &transients);
                }
            }
        }
    }
    for update in controller.tick(physics, &poses, &transients, TICK, false) {
        if let Some(door) = update.door {
            scene.npc_open_door(world, door, update.feet);
        }
        scene.apply_movement(
            update.key,
            update.feet,
            update.yaw_degrees,
            update
                .sequence
                .as_deref()
                .map(|s| (s, update.animation_time)),
            update.state.is_arrived(),
        );
    }
    for pose in poses {
        let s = &scene.states[pose.entity];
        physics.set_entity(pose.entity, s.origin, s.rotation, s.collides());
    }
    physics.refresh_entity_queries();
}

pub fn prepare_weapons(
    world: &mut World,
    vfs: &Vfs,
    weapons: &std::collections::BTreeMap<String, crate::gameplay::Weapon>,
) -> Result<()> {
    let wanted = [
        "idle01",
        "idle01empty",
        "idle",
        "fire",
        "fire1",
        "fire2",
        "fire3",
        "fire4",
        "fire01",
        "fire02",
        "fire03",
        "fire04",
        "altfire",
        "shake",
        "ir_fire2",
        "reload",
        "reloadempty",
        "reload1",
        "reload2",
        "reload3",
        "pump",
        "dryfire",
        "ir_idle",
        "ir_draw",
        "ir_fire",
        "ir_reload",
        "draw",
        "drawempty",
        "swing",
        "misscenter1",
        "hitcenter1",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    for w in weapons.values() {
        let key = format!("{}#0", w.viewmodel.to_lowercase());
        world.model_assets.insert(
            key.clone(),
            source_assets::models::read_model(vfs, &w.viewmodel, 0)?,
        );
        world.rigs.insert(
            key,
            source_assets::animation::load(vfs, &w.viewmodel, &wanted)?,
        );
    }
    Ok(())
}
