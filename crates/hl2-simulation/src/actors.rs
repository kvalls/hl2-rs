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
/// Human-hull NPC classes registered for AI goals (step 14); scenes and scripted_sequences
/// move them too while they own the actor. SetHullType(HULL_HUMAN) in CNPC_MetroPolice,
/// CNPC_Combine and CNPC_PlayerCompanion (npc_citizen, npc_alyx) Spawn.
pub const AI_GROUND_HUMANS: [&str; 10] = [
    "npc_metropolice",
    "npc_combine_s",
    "npc_citizen",
    "npc_alyx",
    "npc_zombie",
    "npc_zombie_torso",
    "npc_poisonzombie",
    "npc_headcrab",
    "npc_headcrab_black",
    "npc_headcrab_poison",
];
/// AI movers' walk/run per weapon class ("" unarmed): Weapon_TranslateActivity of
/// ACT_WALK/ACT_RUN when the model has the result (else the base activity), played as
/// the first sequence with that activity. Variants missing a looping sequence are left
/// out (e.g. the unarmed soldier, whose model has only rifle walks and runs).
fn locomotion_variants(
    vfs: &Vfs,
    model: &str,
    rig: &modkit_core::animation::Rig,
) -> Result<std::collections::BTreeMap<String, crate::npc::Locomotion>> {
    use crate::npc::MoveStyle;
    let sequence = |activity: &str| {
        rig.sequences
            .iter()
            .find(|s| s.activity.eq_ignore_ascii_case(activity))
            .map(|s| s.name.clone())
    };
    let pick = |weapon: &str, base: &str| {
        let activity = crate::npc_weapons::translate(weapon, base)
            .filter(|a| sequence(a).is_some())
            .unwrap_or(base);
        sequence(activity).map(|name| (name, activity.to_owned()))
    };
    let mut choices = Vec::new();
    // Move and shoot's aiming walk/run (StartShootWhileMove needs both sequences).
    let mut aims = std::collections::BTreeMap::new();
    for weapon in std::iter::once("").chain(crate::npc_weapons::NPC_WEAPONS.iter().copied()) {
        // A model without ACT_RUN (the classic zombie, whose NPC_TranslateActivity
        // walks instead) runs with its walk; one without ACT_WALK (the headcrab, whose
        // NPC_TranslateActivity runs instead) walks with its run.
        let walk = pick(weapon, "ACT_WALK").or_else(|| pick(weapon, "ACT_RUN"));
        if let (Some(walk), Some(run)) = (walk.clone(), pick(weapon, "ACT_RUN").or(walk)) {
            choices.push((weapon.to_owned(), walk, run));
        }
        // CBlackHeadcrab's panic scurry (ACT_BLACKHEADCRAB_RUN_PANIC) for both styles.
        if weapon.is_empty() {
            if let Some(panic) = pick("", crate::ai::headcrab::RUN_PANIC) {
                choices.push((
                    crate::ai::headcrab::RUN_PANIC.to_owned(),
                    panic.clone(),
                    panic,
                ));
            }
        }
        if let (Some(walk), Some(run)) = (pick(weapon, "ACT_WALK_AIM"), pick(weapon, "ACT_RUN_AIM"))
        {
            aims.insert(weapon.to_owned(), (walk.0, run.0));
        }
    }
    let wanted = choices
        .iter()
        .flat_map(|(_, walk, run)| [walk.0.clone(), run.0.clone()])
        .collect();
    let sequences = source_assets::models::read_root_motion(vfs, model, &wanted)?;
    Ok(choices
        .into_iter()
        .map(|(weapon, walk, run)| {
            let mut locomotion = crate::npc::Locomotion::from_named(
                &sequences,
                [
                    (MoveStyle::Walk, &walk.0, &walk.1),
                    (MoveStyle::Run, &run.0, &run.1),
                ],
            );
            if let Some((walk, run)) = aims.get(&weapon) {
                locomotion.set_aim(walk.clone(), run.clone());
            }
            (weapon, locomotion)
        })
        .filter(|(_, locomotion)| locomotion.complete())
        .collect())
}
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
            let hull = crate::ai_hull::class_hull(class).unwrap_or(crate::ai_hull::HullType::Human);
            let ground = crate::npc::ground_for_hull(hull, instance.scale, 18., 1.)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            let wanted = if ai_only {
                let rig = world
                    .rigs
                    .get(&instance.asset_key())
                    .ok_or_else(|| anyhow::anyhow!("no rig for {}", instance.model))?;
                let variants = locomotion_variants(vfs, &instance.model, rig)?;
                let equipment = world.entities[actor]
                    .get("additionalequipment")
                    .unwrap_or("")
                    .to_lowercase();
                let motion = variants
                    .get(&equipment)
                    .or_else(|| variants.get(""))
                    .or_else(|| variants.values().next())
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("no walk/run locomotion"))?;
                let wanted = variants
                    .values()
                    .flat_map(|v| v.required_clips())
                    .collect::<std::collections::BTreeSet<_>>();
                controller
                    .register_ai_actor(actor, instance.scale, ground, motion)
                    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
                controller.set_locomotion_variants(actor, variants);
                // The zombies and crabs have no bits_CAP_DOORS_GROUP (CZombie pounds on
                // doors).
                if crate::ai::zombie::is_zombie_class(class)
                    || crate::ai::headcrab::is_headcrab_class(class)
                {
                    controller.set_opens_doors(actor, false);
                }
                controller.set_headcrab(actor, crate::ai::headcrab::is_headcrab_class(class));
                wanted
            } else {
                let motion = crate::npc::Locomotion::load(vfs, &instance.model)?;
                let wanted = motion.required_clips();
                controller
                    .register_actor(actor, instance.scale, ground, motion)
                    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
                wanted
            };
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
        if let Some(door) = update
            .door
            .filter(|_| controller.opens_doors(update.key.actor))
        {
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
        // weapon_frag (v_grenade.mdl)
        "drawbackhigh",
        "drawbacklow",
        "throw",
        "roll",
        "lob",
    ]
    .into_iter()
    .map(String::from)
    .collect::<std::collections::BTreeSet<String>>();
    // The remaining arsenal resolves its sequences from activities (SendWeaponAnim),
    // so those viewmodels load every sequence carrying one of these activities.
    let activities = [
        "act_vm_draw",
        "act_vm_idle",
        "act_vm_fidget",
        "act_vm_primaryattack",
        "act_vm_secondaryattack",
        "act_vm_reload",
        "act_vm_holster",
        "act_vm_haulback",
        "act_vm_throw",
        "act_vm_dryfire",
        "act_vm_idle_lowered",
        "act_vm_lowered_to_idle",
        "act_vm_idle_to_lowered",
        "act_crossbow_draw_unloaded",
    ];
    for w in weapons.values() {
        let mut wanted = wanted.clone();
        if matches!(
            w.class.as_str(),
            "weapon_crossbow" | "weapon_rpg" | "weapon_bugbait" | "weapon_physcannon"
        ) {
            wanted.extend(activities.iter().map(|a| (*a).to_owned()));
        }
        let key = format!("{}#0", w.viewmodel.to_lowercase());
        let base = source_assets::models::read_model(vfs, &w.viewmodel, 0)?;
        // Skin 1 (the crossbow's glowing bolt, CWeaponCrossbow::SetSkin): kept when any
        // surface material differs; the host draws only the differing surfaces.
        let alternate = source_assets::models::read_model(vfs, &w.viewmodel, 1)?;
        if alternate.len() == base.len()
            && alternate
                .iter()
                .zip(&base)
                .any(|(a, b)| a.material != b.material)
        {
            world
                .model_assets
                .insert(format!("{}#1", w.viewmodel.to_lowercase()), alternate);
        }
        world.model_assets.insert(key.clone(), base);
        world.rigs.insert(
            key,
            source_assets::animation::load(vfs, &w.viewmodel, &wanted)?,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn owned_ai_movers_walk_and_run_with_their_weapon_sequences() {
        use crate::npc::MoveStyle;
        let vfs = Vfs::mount(&source_assets::install::discover().unwrap()).unwrap();
        // Humanoid rigs load the HUMANOID_AI_ACTIVITIES (the walk/run translations too).
        let wanted = source_assets::models::HUMANOID_AI_ACTIVITIES
            .iter()
            .map(|a| a.to_lowercase())
            .collect();
        let rig = |model: &str| source_assets::animation::load(&vfs, model, &wanted).unwrap();
        let names = |model: &str, weapon: &str| {
            let variants = locomotion_variants(&vfs, model, &rig(model)).unwrap();
            let v = &variants[weapon];
            (
                v.sequence_name(MoveStyle::Walk),
                v.sequence_name(MoveStyle::Run),
            )
        };
        // combine_soldier: only rifle walks/runs (no unarmed variant).
        let soldier = "models/combine_soldier.mdl";
        assert_eq!(
            names(soldier, "weapon_smg1"),
            ("walk_all".into(), "runall".into())
        );
        assert_eq!(
            names(soldier, "weapon_ar2"),
            ("walk_all".into(), "runall".into())
        );
        assert!(!locomotion_variants(&vfs, soldier, &rig(soldier))
            .unwrap()
            .contains_key(""));
        // A male citizen: unarmed walk_all/run_all, armed ACT_WALK/RUN_RIFLE.
        let citizen = "models/humans/group01/male_07.mdl";
        assert_eq!(names(citizen, ""), ("walk_all".into(), "run_all".into()));
        assert_eq!(
            names(citizen, "weapon_smg1"),
            ("walkholdall1".into(), "run_holding_all".into())
        );
        // The classic zombie has no ACT_RUN: it walks for both (CZombie translates
        // ACT_RUN to ACT_WALK), on "walk" despite its 1e-4 sideways drift.
        let zombie = "models/zombie/classic.mdl";
        assert_eq!(names(zombie, ""), ("walk".into(), "walk".into()));
    }
}
