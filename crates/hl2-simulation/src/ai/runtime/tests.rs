use super::*;
use modkit_core::Entity;

fn entity(pairs: &[(&str, &str)]) -> Entity {
    Entity {
        properties: pairs
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect(),
    }
}
/// A cop at the origin facing +X, a police goal with the player as its target and a
/// relay that counts the goal's first warning (synthetic map data).
fn world() -> World {
    World {
        entities: vec![
            entity(&[
                ("classname", "npc_metropolice"),
                ("targetname", "cop"),
                ("origin", "0 0 0"),
                ("additionalequipment", "weapon_stunstick"),
            ]),
            entity(&[
                ("classname", "ai_goal_police"),
                ("targetname", "post"),
                ("origin", "0 0 9"),
                ("angles", "0 0 0"),
                ("PoliceRadius", "100"),
                ("PoliceTarget", "!player"),
                ("OnFirstWarning", "counter,Add,1,0,-1"),
            ]),
            entity(&[
                ("classname", "env_global"),
                ("globalstate", "gordon_precriminal"),
                ("initialstate", "1"),
                ("spawnflags", "1"),
            ]),
        ],
        ..Default::default()
    }
}
struct Rig {
    world: World,
    scene: Scene,
    physics: Physics,
    controller: Controller,
    ai: AiRuntime,
}
fn rig() -> Rig {
    let world = world();
    let scene = Scene::new(&world);
    let physics = Physics::new(&world);
    let ai = AiRuntime::new(&world, None);
    Rig {
        world,
        scene,
        physics,
        controller: Controller::new(None, Default::default()),
        ai,
    }
}
fn run(r: &mut Rig, ticks: usize, player: PlayerState) {
    for _ in 0..ticks {
        r.scene.time += 0.015;
        r.ai.tick(
            &r.world,
            &mut r.scene,
            &mut r.controller,
            &r.physics,
            &[],
            player,
        );
    }
}
fn player_at(x: f32) -> PlayerState {
    PlayerState {
        feet: Vec3::new(x, 0., 0.),
        on_ground: true,
        ..Default::default()
    }
}

#[test]
fn ai_is_off_by_default_and_shoves_a_close_player_when_enabled() {
    let mut r = rig();
    run(&mut r, 20, player_at(30.));
    assert!(r.ai.actors[&0].npc.schedule.is_none());
    assert!(r.scene.sounds.is_empty());
    r.ai.enabled = true;
    run(&mut r, 20, player_at(30.));
    let cop = r.ai.actors[&0].class.cop().expect("cop");
    assert!(cop.precriminal);
    assert!(cop.warnings >= 1);
    assert!(r
        .scene
        .sounds
        .iter()
        .any(|s| s.name.starts_with("#METROPOLICE_BACK_UP") && s.origin.is_some()));
    assert!(r.ai.diagnostics.thinks >= 2);
}

#[test]
fn using_a_calm_cop_counts_as_bothering_him() {
    let mut r = rig();
    let far = player_at(500.);
    assert!(!r.ai.player_used(0, &r.world, &mut r.scene, &r.physics, far));
    r.ai.enabled = true;
    run(&mut r, 5, far);
    assert!(!r.ai.player_used(2, &r.world, &mut r.scene, &r.physics, far));
    for _ in 0..3 {
        assert!(r.ai.player_used(0, &r.world, &mut r.scene, &r.physics, far));
    }
    let cop = r.ai.actors[&0].class.cop().expect("cop");
    assert_eq!(
        cop.warnings,
        super::super::metropolice::METROPOLICE_MAX_WARNINGS
    );
    assert!(r
        .scene
        .sounds
        .iter()
        .any(|s| s.name.starts_with("#METROPOLICE_BACK_UP")));
}

#[test]
fn a_pooled_cop_is_hidden_until_placed_then_thinks() {
    let mut world = World {
        entities: vec![entity(&[
            ("classname", "env_global"),
            ("globalstate", "gordon_precriminal"),
            ("initialstate", "1"),
            ("spawnflags", "1"),
        ])],
        ..Default::default()
    };
    crate::ai::spawn::append_pool(&mut world);
    let id = crate::ai::spawn::free_slot(&world, "npc_metropolice", &Default::default()).unwrap();
    let mut r = Rig {
        scene: Scene::new(&world),
        physics: Physics::new(&world),
        ai: AiRuntime::new(&world, None),
        controller: Controller::new(None, Default::default()),
        world,
    };
    r.ai.enabled = true;
    assert!(!r.scene.states[id].visible);
    run(&mut r, 20, player_at(30.));
    assert_eq!(r.ai.diagnostics.thinks, 0, "hidden slots do not think");
    r.scene.states[id].origin = Vec3::ZERO;
    r.scene.states[id].visible = true;
    r.ai.set_equipment(id, "npc_metropolice", "weapon_stunstick");
    run(&mut r, 20, player_at(30.));
    let cop = r.ai.actors[&id].class.cop().expect("cop");
    assert!(cop.has_baton && cop.warnings >= 1);
}

#[test]
fn debug_switches_ignore_the_player_and_stop_the_ai() {
    let mut r = rig();
    r.ai.enabled = true;
    r.ai.ignore_player = true;
    run(&mut r, 20, player_at(30.));
    let cop = r.ai.actors[&0].class.cop().expect("cop");
    assert_eq!(cop.warnings, 0, "an ignored player is not shoved");
    r.ai.ignore_player = false;
    // F4 allegiance: a foe spawn hates the player, a friend never makes him its enemy.
    r.ai.set_player_disposition(0, Some(Disposition::HT));
    run(&mut r, 20, player_at(30.));
    assert_eq!(r.ai.actors[&0].npc.enemy, Some(PLAYER), "forced hate");
    r.ai.enabled = false;
    run(&mut r, 1, player_at(30.));
    assert!(r.ai.actors[&0].npc.schedule.is_none());
    assert!(r.ai.actors[&0].npc.enemy.is_none());
    r.ai.enabled = true;
    r.ai.set_player_disposition(0, Some(Disposition::LI));
    run(&mut r, 20, player_at(30.));
    assert_ne!(r.ai.actors[&0].npc.enemy, Some(PLAYER), "a friend");
}

#[test]
fn weapon_equip_goes_to_any_weapon_user_and_never_to_natural_attackers() {
    let world = World {
        entities: vec![
            entity(&[
                ("classname", "npc_monk"),
                ("additionalequipment", "weapon_smg1"),
            ]),
            entity(&[
                ("classname", "npc_zombie"),
                ("additionalequipment", "weapon_smg1"),
            ]),
            entity(&[
                ("classname", "npc_citizen"),
                ("additionalequipment", "weapon_smg1"),
            ]),
        ],
        ..Default::default()
    };
    let mut ai = AiRuntime::new(&world, None);
    // Map additionalequipment: the monk holds it (his class has no AI layer here yet),
    // the zombie has no bits_CAP_USE_WEAPONS, the citizen's AI actor equips it.
    assert_eq!(ai.held_weapons[&0].class, "weapon_smg1");
    assert!(!ai.held_weapons.contains_key(&1));
    assert_eq!(ai.actors[&2].weapon.as_ref().unwrap().class, "weapon_smg1");
    assert!(ai.actors[&2].npc.shot_regulator.is_some());
    ai.set_equipment(1, "npc_zombie", "weapon_pistol");
    assert!(!ai.held_weapons.contains_key(&1));
    ai.set_equipment(0, "npc_monk", "");
    assert!(!ai.held_weapons.contains_key(&0), "unequipped");
    ai.set_equipment(0, "npc_monk", "weapon_ar2");
    assert_eq!(ai.held_weapons[&0].class, "weapon_ar2");
    ai.set_equipment(2, "npc_citizen", "weapon_ar2");
    assert_eq!(ai.actors[&2].weapon.as_ref().unwrap().class, "weapon_ar2");
    assert_eq!(
        ai.actors[&2].npc.shot_regulator.as_ref().unwrap().rule,
        npc_weapons::RegulatorRule::PlayerCompanion
    );
}

#[test]
fn a_runtime_spawned_cop_gets_an_actor_and_thinks() {
    let mut r = rig();
    r.ai.enabled = true;
    let cop = modkit_core::Entity {
        properties: vec![
            ("classname".into(), "npc_metropolice".into()),
            ("origin".into(), "0 60 0".into()),
            ("additionalequipment".into(), "weapon_stunstick".into()),
        ],
    };
    let id = r.scene.spawn_entity(&mut r.world, cop.clone());
    assert!(r.ai.add_actor(id, &cop));
    // Classes without an AI layer here (Kleiner: CAI_BaseActor, no combat).
    let kleiner = modkit_core::Entity {
        properties: vec![("classname".into(), "npc_kleiner".into())],
    };
    assert!(!r.ai.add_actor(id + 1, &kleiner));
    run(&mut r, 20, player_at(30.));
    let spawned = r.ai.actors[&id].class.cop().expect("cop");
    assert!(spawned.has_baton);
    assert!(r.ai.actors[&id].last_think > 0.);
}

#[test]
fn scripted_cops_are_left_to_their_script() {
    let mut r = rig();
    r.ai.enabled = true;
    r.scene.states[0].scripted_by = Some(2);
    run(&mut r, 20, player_at(30.));
    assert_eq!(r.ai.actors[&0].npc.state, NpcState::Script);
    assert!(r.ai.actors[&0].npc.schedule.is_none());
    assert!(r.scene.sounds.is_empty());
    assert!(r.ai.diagnostics.scripted_skips > 0);
    r.scene.states[0].scripted_by = None;
    run(&mut r, 10, player_at(30.));
    assert_ne!(r.ai.actors[&0].npc.state, NpcState::Script);
}

#[test]
fn policing_warns_a_noclip_player_like_native_notarget() {
    // The oracle runs noclip + notarget, and native still harasses at 80 units: the
    // policing conditions read the goal target, not the cop's senses.
    let mut r = rig();
    r.scene
        .ai_inputs
        .push((0, "setpolicegoal".into(), "post".into(), usize::MAX));
    r.ai.enabled = true;
    let player = PlayerState {
        noclip: true,
        ..player_at(150.)
    };
    run(&mut r, 40, player);
    let cop = r.ai.actors[&0].class.cop().expect("cop");
    let policing = cop.policing.as_ref().unwrap();
    assert!(policing.warnings >= 1, "{:?}", r.ai.actors[&0].npc.schedule);
    // One warning per 4-6 s: the warn schedule does not fail and re-warn at once.
    run(&mut r, 150, player);
    let cop = r.ai.actors[&0].class.cop().expect("cop");
    let policing = cop.policing.as_ref().unwrap();
    assert_eq!(policing.warnings, 1, "{:?}", r.ai.actors[&0].npc.schedule);
    assert!(!policing.hostile);
    // The idle stand is interrupted once the next harass time passes (4-6 s), not
    // only when its own wait ends.
    run(&mut r, 300, player);
    let cop = r.ai.actors[&0].class.cop().expect("cop");
    assert_eq!(cop.policing.as_ref().unwrap().warnings, 2);
}

#[test]
fn assault_goal_and_scripted_schedule_inputs_reach_the_cop() {
    let world = World {
        entities: vec![
            entity(&[
                ("classname", "npc_metropolice"),
                ("targetname", "cop1"),
                ("origin", "450 0 0"),
                ("additionalequipment", "weapon_pistol"),
            ]),
            entity(&[
                ("classname", "ai_goal_assault"),
                ("targetname", "cop1_assault"),
                ("actor", "cop1"),
                ("rallypoint", "cop1_rp"),
                ("AssaultCue", "1"),
            ]),
            entity(&[
                ("classname", "assault_rallypoint"),
                ("targetname", "cop1_rp"),
                ("origin", "632 0 32"),
                ("angles", "0 180 0"),
                ("assaultpoint", "cop1_ap"),
                ("priority", "1"),
            ]),
            entity(&[
                ("classname", "assault_assaultpoint"),
                ("targetname", "cop1_ap"),
                ("origin", "632 0 8"),
                ("assaulttimeout", "3.0"),
            ]),
            entity(&[
                ("classname", "aiscripted_schedule"),
                ("targetname", "cop1_sched"),
                ("m_iszEntity", "cop1"),
                ("goalent", "cop1_goal"),
                ("schedule", "2"),
            ]),
            entity(&[
                ("classname", "info_target"),
                ("targetname", "cop1_goal"),
                ("origin", "376 -60 0"),
            ]),
        ],
        ..Default::default()
    };
    let scene = Scene::new(&world);
    let physics = Physics::new(&world);
    let ai = AiRuntime::new(&world, None);
    let mut r = Rig {
        world,
        scene,
        physics,
        controller: Controller::new(None, Default::default()),
        ai,
    };
    let far = player_at(-2000.);
    // StartSchedule: a run to the goal entity, once (no SF_SCRIPT_REPEATABLE).
    r.scene
        .ai_inputs
        .push((4, "startschedule".into(), String::new(), usize::MAX));
    run(&mut r, 1, far);
    let actor = &r.ai.actors[&0];
    assert_eq!(
        actor.npc.schedule.as_deref(),
        Some("SCHED_AISCRIPTED_MOVE_TO_GOAL")
    );
    let cop = actor.class.cop().expect("cop");
    assert_eq!(cop.script_goal, Some((Vec3::new(376., -60., 0.), true)));
    r.ai.actors.get_mut(&0).unwrap().npc.schedule = None;
    r.scene
        .ai_inputs
        .push((4, "startschedule".into(), String::new(), usize::MAX));
    run(&mut r, 1, far);
    assert!(r.ai.actors[&0].npc.schedule.is_none());
    // Activate: the rally point is locked by the cop, the assault point resolved.
    r.scene
        .ai_inputs
        .push((1, "activate".into(), String::new(), usize::MAX));
    run(&mut r, 1, far);
    let cop = r.ai.actors[&0].class.cop().expect("cop");
    let assault = cop.assault.as_ref().expect("assault");
    assert!(assault.active());
    assert_eq!(assault.rally.entity, 2);
    assert_eq!(assault.rally.yaw, 180.);
    assert_eq!(assault.point.entity, 3);
    assert_eq!(r.ai.rally_locks.get(&2), Some(&0));
    // BeginAssault cues it; Deactivate disables it and frees the rally point.
    r.scene
        .ai_inputs
        .push((1, "beginassault".into(), String::new(), usize::MAX));
    r.scene
        .ai_inputs
        .push((1, "deactivate".into(), String::new(), usize::MAX));
    run(&mut r, 1, far);
    let cop = r.ai.actors[&0].class.cop().expect("cop");
    let assault = cop.assault.as_ref().unwrap();
    assert_eq!(assault.received, super::super::assault::Cue::EntityInput);
    assert!(!assault.active());
    assert!(r.ai.rally_locks.is_empty());
}
#[test]
fn friend_and_foe_cops_target_each_other_and_relationships_override_classes() {
    let world = World {
        entities: vec![
            entity(&[
                ("classname", "npc_metropolice"),
                ("targetname", "friend"),
                ("origin", "0 0 0"),
                ("angles", "0 0 0"),
                ("additionalequipment", "weapon_pistol"),
            ]),
            entity(&[
                ("classname", "npc_metropolice"),
                ("targetname", "foe"),
                ("origin", "300 0 0"),
                ("angles", "0 180 0"),
                ("additionalequipment", "weapon_pistol"),
            ]),
            entity(&[
                ("classname", "npc_citizen"),
                ("targetname", "bystander"),
                ("origin", "0 300 0"),
            ]),
            entity(&[
                ("classname", "ai_relationship"),
                ("subject", "npc_metropolice"),
                ("target", "bystander"),
                ("disposition", "1"),
                ("StartActive", "1"),
            ]),
        ],
        ..Default::default()
    };
    let scene = Scene::new(&world);
    let physics = Physics::new(&world);
    let ai = AiRuntime::new(&world, None);
    let mut r = Rig {
        world,
        scene,
        physics,
        controller: Controller::new(None, Default::default()),
        ai,
    };
    // Same class, no allegiances: the SDK table says neutral.
    assert_eq!(r.ai.npc_disposition(&r.world, 0, 1, true), Disposition::NU);
    r.ai.set_player_disposition(0, Some(Disposition::LI));
    r.ai.set_player_disposition(1, Some(Disposition::HT));
    assert_eq!(r.ai.npc_disposition(&r.world, 0, 1, true), Disposition::HT);
    assert_eq!(r.ai.npc_disposition(&r.world, 1, 0, true), Disposition::HT);
    r.ai.enabled = true;
    run(&mut r, 20, player_at(-3000.));
    assert_eq!(r.ai.actors[&0].npc.enemy, Some(1));
    assert_eq!(r.ai.actors[&1].npc.enemy, Some(0));
    // StartActive ai_relationship: cops hate the passive citizen (class rule NU).
    assert_eq!(r.ai.npc_disposition(&r.world, 1, 2, true), Disposition::HT);
    // The AI report (pair-keyed overrides included) serializes.
    assert!(serde_json::to_string(&r.ai).is_ok());
    r.scene
        .ai_inputs
        .push((3, "revertrelationship".into(), String::new(), usize::MAX));
    run(&mut r, 1, player_at(-3000.));
    assert_eq!(r.ai.npc_disposition(&r.world, 1, 2, true), Disposition::NU);
    // A killed enemy is dropped (COND_ENEMY_DEAD) for the next one.
    r.scene.states[1].killed = true;
    run(&mut r, 10, player_at(-3000.));
    assert_eq!(r.ai.actors[&0].npc.enemy, None);
}
#[test]
fn hurt_and_killed_cops_speak_their_pain_and_death_lines() {
    let mut r = rig();
    r.ai.enabled = true;
    let health = crate::gameplay::npc_spawn_health(&r.scene, &r.world.entities[0]);
    // A first light hit (> 0.8 health): PAIN_LIGHT; another within a second: none.
    r.scene.states[0].value = health * 0.85;
    r.scene.npc_damage.push((0, true, health * 0.15));
    r.scene.npc_damage.push((0, true, 1.));
    run(&mut r, 1, player_at(-3000.));
    let spoken = |r: &Rig| -> Vec<String> {
        r.scene
            .sounds
            .iter()
            .filter(|s| s.name.starts_with('#'))
            .map(|s| s.name.clone())
            .collect()
    };
    assert_eq!(spoken(&r), ["#METROPOLICE_PAIN_LIGHT"]);
    // Killed: DeathSound once.
    r.scene.sounds.clear();
    r.scene.states[0].killed = true;
    r.scene.npc_damage.push((0, true, 50.));
    r.scene.npc_damage.push((0, true, 50.));
    run(&mut r, 1, player_at(-3000.));
    assert_eq!(spoken(&r), ["#METROPOLICE_DIE"]);
    assert_eq!(
        sentence_parameters("#COMBINE_DIE"),
        Some(COMBINE_SENTENCE_PARAMETERS)
    );
}
#[test]
fn squads_share_enemies_gate_lines_and_mourn_their_dead() {
    // Three soldiers of squad "s" (the last two face away from the player) and a
    // squadless one far away.
    let soldier = |name: &str, origin: &str, yaw: &str, squad: &str| {
        let angles = format!("0 {yaw} 0");
        let mut pairs = vec![
            ("classname", "npc_combine_s"),
            ("targetname", name),
            ("origin", origin),
            ("angles", angles.as_str()),
        ];
        if !squad.is_empty() {
            pairs.push(("squadname", squad));
        }
        entity(&pairs)
    };
    let world = World {
        entities: vec![
            soldier("a", "0 0 0", "0", "s"),
            soldier("b", "0 200 0", "180", "s"),
            soldier("c", "0 -200 0", "180", "s"),
            soldier("lone", "0 3000 0", "180", ""),
        ],
        ..Default::default()
    };
    let mut r = Rig {
        scene: Scene::new(&world),
        physics: Physics::new(&world),
        controller: Controller::new(None, Default::default()),
        ai: AiRuntime::new(&world, None),
        world,
    };
    r.ai.enabled = true;
    let far = player_at(-5000.);
    run(&mut r, 1, far);
    let squad = &r.ai.squads["s"];
    assert_eq!(
        (squad.members.clone(), squad.leader()),
        (vec![0, 1, 2], Some(0))
    );
    assert!(r.ai.actors[&3].squad.is_none());
    // IN_SQUAD lines need a squad of two; NORMAL ones do not.
    let world = r.world.clone();
    assert!(!r.ai.say(3, "COMBINE_IDLE", &world, &mut r.scene));
    assert!(r.ai.say(3, "COMBINE_DANGER", &world, &mut r.scene));
    assert!(r.ai.say(0, "COMBINE_IDLE", &world, &mut r.scene));
    // The squad's sound gate: another member's normal line waits.
    assert!(!r.ai.say(1, "COMBINE_IDLE", &world, &mut r.scene));
    assert!(r.ai.say(1, "COMBINE_ALERT", &world, &mut r.scene));
    // Only the leader sees the player; SquadNewEnemy gives him to the others.
    run(&mut r, 12, player_at(300.));
    assert_eq!(r.ai.actors[&0].npc.enemy, Some(PLAYER));
    for member in [1, 2] {
        assert_eq!(
            r.ai.actors[&member].npc.enemy,
            Some(PLAYER),
            "member {member}"
        );
        assert!(r.ai.actors[&member]
            .npc
            .enemies
            .find(Some(PLAYER))
            .is_some());
    }
    assert!(r.ai.diagnostics.squad_events["new_enemy"] >= 2);
    assert!(r.ai.diagnostics.squad_events["memory"] >= 2);
    // A death: MAN_DOWN while two remain (one member speaks; the squad gate culls the
    // other's), LAST_OF_SQUAD for the last one. The earlier lines' waits are over.
    r.ai.squads.get_mut("s").expect("squad").sound_wait = 0.;
    r.ai.sound_wait.clear();
    r.scene.sounds.clear();
    r.scene.states[2].killed = true;
    run(&mut r, 1, far);
    assert_eq!(r.ai.squads["s"].members, [0, 1]);
    let spoken = |r: &Rig| -> Vec<String> {
        r.scene
            .sounds
            .iter()
            .filter(|s| {
                s.name.starts_with("#COMBINE_MAN_DOWN") || s.name.starts_with("#COMBINE_LAST")
            })
            .map(|s| s.name.clone())
            .collect()
    };
    assert_eq!(spoken(&r), ["#COMBINE_MAN_DOWN"]);
    r.scene.sounds.clear();
    r.scene.states[1].killed = true;
    run(&mut r, 1, far);
    assert_eq!(spoken(&r), ["#COMBINE_LAST_OF_SQUAD"]);
    assert_eq!(r.ai.squads["s"].leader(), None);
}
#[test]
fn squad_cops_ask_and_answer_idle_chatter_and_harass_a_near_player_once() {
    let cop = |name: &str, x: &str| {
        entity(&[
            ("classname", "npc_metropolice"),
            ("targetname", name),
            ("origin", x),
            ("squadname", "pair"),
        ])
    };
    let world = World {
        entities: vec![
            cop("a", "0 0 0"),
            cop("b", "0 300 0"),
            entity(&[
                ("classname", "env_global"),
                ("globalstate", "gordon_precriminal"),
                ("initialstate", "1"),
                ("spawnflags", "1"),
            ]),
        ],
        ..Default::default()
    };
    let mut r = Rig {
        scene: Scene::new(&world),
        physics: Physics::new(&world),
        controller: Controller::new(None, Default::default()),
        ai: AiRuntime::new(&world, None),
        world,
    };
    r.ai.enabled = true;
    run(&mut r, 1, player_at(-5000.));
    let world = r.world.clone();
    // A spoken question makes the asker wait and his squadmate answer it.
    r.ai.idle_lines.push(IdleLine {
        id: 0,
        group: "METROPOLICE_IDLE_QUEST",
        then: IdleThen::CopAsked(1),
    });
    r.ai.speak_idle_lines(&world, &mut r.scene);
    assert_eq!(r.ai.actors[&0].idle_chatter, COP_CHATTER_WAIT_FOR_RESPONSE);
    assert_eq!(r.ai.actors[&1].idle_chatter, COP_CHATTER_RESPONSE + 1);
    // The answer waits for the squad's sound gate, then comes on the next thinks.
    r.scene.sounds.clear();
    run(&mut r, 400, player_at(-5000.));
    assert!(r
        .scene
        .sounds
        .iter()
        .any(|s| s.name == "#METROPOLICE_IDLE_ANSWER"));
    assert_eq!(r.ai.actors[&0].idle_chatter, COP_CHATTER_ASK_QUESTION);
    assert_eq!(r.ai.actors[&1].idle_chatter, COP_CHATTER_ASK_QUESTION);
    // Harassment is remembered by the whole squad.
    r.ai.idle_lines.push(IdleLine {
        id: 1,
        group: "METROPOLICE_IDLE_HARASS_PLAYER",
        then: IdleThen::CopHarassed,
    });
    r.ai.squads.get_mut("pair").expect("squad").sound_wait = 0.;
    r.ai.sound_wait.clear();
    r.ai.speak_idle_lines(&world, &mut r.scene);
    assert!(r.ai.actors[&0].harassed && r.ai.actors[&1].harassed);
}
#[test]
fn an_npc_spawned_after_the_first_think_is_seen() {
    // A friend cop thinks alone (the sight candidate list is built), then a foe cop
    // is created at runtime in front of him (DESIGN 14r part A).
    let world = World {
        entities: vec![entity(&[
            ("classname", "npc_metropolice"),
            ("origin", "0 0 0"),
            ("angles", "0 0 0"),
            ("additionalequipment", "weapon_pistol"),
        ])],
        ..Default::default()
    };
    let scene = Scene::new(&world);
    let physics = Physics::new(&world);
    let ai = AiRuntime::new(&world, None);
    let mut r = Rig {
        world,
        scene,
        physics,
        controller: Controller::new(None, Default::default()),
        ai,
    };
    r.ai.enabled = true;
    r.ai.set_player_disposition(0, Some(Disposition::LI));
    run(&mut r, 10, player_at(-3000.));
    assert_eq!(r.ai.actors[&0].npc.enemy, None);
    let foe = entity(&[
        ("classname", "npc_metropolice"),
        ("origin", "300 0 0"),
        ("angles", "0 180 0"),
        ("additionalequipment", "weapon_pistol"),
    ]);
    let id = r.scene.spawn_entity(&mut r.world, foe.clone());
    assert!(r.ai.add_actor(id, &foe));
    r.ai.set_player_disposition(id, Some(Disposition::HT));
    run(&mut r, 20, player_at(-3000.));
    assert_eq!(r.ai.actors[&0].npc.enemy, Some(id));
    assert_eq!(r.ai.actors[&id].npc.enemy, Some(0));
}
#[test]
fn stunstick_swings_strike_only_hated_npcs_in_the_swept_box() {
    let cop = |name: &str, origin: &str, angles: &str| {
        entity(&[
            ("classname", "npc_metropolice"),
            ("targetname", name),
            ("origin", origin),
            ("angles", angles),
            ("additionalequipment", "weapon_stunstick"),
        ])
    };
    let world = World {
        entities: vec![
            cop("friend", "0 0 0", "0 0 0"),
            cop("foe", "40 0 0", "0 180 0"),
            cop("ally", "0 40 0", "0 0 0"),
        ],
        ..Default::default()
    };
    let scene = Scene::new(&world);
    let physics = Physics::new(&world);
    let ai = AiRuntime::new(&world, None);
    let mut r = Rig {
        world,
        scene,
        physics,
        controller: Controller::new(None, Default::default()),
        ai,
    };
    r.ai.set_player_disposition(0, Some(Disposition::LI));
    r.ai.set_player_disposition(1, Some(Disposition::HT));
    r.ai.set_player_disposition(2, Some(Disposition::LI));
    let swing = |end: Vec3| CopRequest::Swing {
        start: Vec3::new(0., 0., 60.),
        end,
        mins: Vec3::new(-16., -16., -40.),
        maxs: Vec3::splat(16.),
        damage: 40.,
    };
    let player = player_at(-3000.);
    // Toward the hated foe 40 units ahead: a hit with a combat sound.
    r.ai.actors
        .get_mut(&0)
        .unwrap()
        .class
        .requests_mut()
        .push(swing(Vec3::new(32., 0., 60.)));
    r.ai.apply_requests(0, &r.world, &mut r.scene, &r.physics, player);
    assert_eq!(r.ai.melee_hits.len(), 1);
    assert_eq!(r.ai.melee_hits[0].victim, 1);
    assert_eq!(r.ai.melee_hits[0].damage, 40.);
    // Toward the liked ally: CTraceFilterMelee passes it (no damage).
    r.ai.melee_hits.clear();
    r.ai.actors
        .get_mut(&0)
        .unwrap()
        .class
        .requests_mut()
        .push(swing(Vec3::new(0., 32., 60.)));
    r.ai.apply_requests(0, &r.world, &mut r.scene, &r.physics, player);
    assert!(r.ai.melee_hits.is_empty());
}
#[test]
fn combine_kick_strikes_the_player_enemy_or_a_hated_npc() {
    let world = World {
        entities: vec![
            entity(&[
                ("classname", "npc_combine_s"),
                ("origin", "0 0 0"),
                ("angles", "0 0 0"),
                ("additionalequipment", "weapon_smg1"),
            ]),
            entity(&[
                ("classname", "npc_citizen"),
                ("origin", "50 0 0"),
                ("angles", "0 180 0"),
            ]),
        ],
        ..Default::default()
    };
    let scene = Scene::new(&world);
    let physics = Physics::new(&world);
    let mut ai = AiRuntime::new(&world, None);
    ai.combine_kick_damage = 10.;
    let mut r = Rig {
        world,
        scene,
        physics,
        controller: Controller::new(None, Default::default()),
        ai,
    };
    let kick = CopRequest::Kick {
        start: Vec3::new(0., 0., 36.),
        forward: Vec3::X,
    };
    // The player as the enemy 40 units ahead: punch (-12, -7, 0), 100 forward + 50 up.
    r.ai.actors.get_mut(&0).unwrap().npc.enemy = Some(PLAYER);
    r.ai.actors
        .get_mut(&0)
        .unwrap()
        .class
        .requests_mut()
        .push(kick.clone());
    r.ai.apply_requests(0, &r.world, &mut r.scene, &r.physics, player_at(40.));
    assert_eq!(r.ai.player_hits.len(), 1);
    assert_eq!(r.ai.player_hits[0].damage, 10.);
    assert_eq!(r.ai.player_hits[0].punch, Vec3::new(-12., -7., 0.));
    assert_eq!(r.ai.player_hits[0].impulse, Vec3::new(100., 0., 50.));
    // A hated rebel in reach takes the kick through the melee hit path.
    r.ai.actors.get_mut(&0).unwrap().npc.enemy = Some(1);
    r.ai.set_player_disposition(0, Some(Disposition::HT));
    r.ai.set_player_disposition(1, Some(Disposition::LI));
    r.ai.actors
        .get_mut(&0)
        .unwrap()
        .class
        .requests_mut()
        .push(kick);
    r.ai.apply_requests(0, &r.world, &mut r.scene, &r.physics, player_at(-3000.));
    assert_eq!(r.ai.melee_hits.len(), 1);
    assert_eq!(
        (r.ai.melee_hits[0].victim, r.ai.melee_hits[0].damage),
        (1, 10.)
    );
}
#[test]
fn player_hits_give_damage_conditions_and_make_the_player_the_enemy() {
    let world = World {
        entities: vec![entity(&[
            ("classname", "npc_combine_s"),
            ("origin", "0 0 0"),
            ("angles", "0 0 0"),
            ("additionalequipment", "weapon_smg1"),
        ])],
        ..Default::default()
    };
    let scene = Scene::new(&world);
    let physics = Physics::new(&world);
    let ai = AiRuntime::new(&world, None);
    let mut r = Rig {
        world,
        scene,
        physics,
        controller: Controller::new(None, Default::default()),
        ai,
    };
    r.ai.enabled = true;
    // The player stands behind the soldier (outside his view cone).
    let behind = player_at(-300.);
    run(&mut r, 10, behind);
    assert_eq!(r.ai.actors[&0].npc.enemy, None);
    // A 357 round (more than 20): light and heavy damage, the player remembered.
    r.scene.npc_damage.push((0, true, 40.));
    run(&mut r, 1, behind);
    for _ in 0..10 {
        if r.ai.actors[&0].npc.enemy.is_some() {
            break;
        }
        run(&mut r, 1, behind);
    }
    assert_eq!(r.ai.actors[&0].npc.enemy, Some(PLAYER));
    assert!(r.scene.npc_damage.is_empty());
}
#[test]
fn grenade_throw_and_toss_velocities_land_on_the_target() {
    let world = World::default();
    let physics = Physics::new(&world);
    let start = Vec3::new(0., 0., 64.);
    let target = Vec3::new(400., 100., 0.);
    let land = |v: Vec3| {
        // Time to reach the target's horizontal distance, then the height there.
        let t = (target - start).truncate().length() / v.truncate().length();
        start + v * t - Vec3::Z * 0.5 * GRAVITY * t * t
    };
    let throw = vec_check_throw(&physics, 0, start, target, 650.).expect("throw");
    assert!((land(throw) - target).length() < 0.5);
    // Unjittered high toss (draws 0.5 = no offset): apex 300 above the midpoint.
    let toss = vec_check_toss(&physics, 0, start, target, Vec3::X, [0.5; 4]).expect("toss");
    assert!((land(toss) - target).length() < 0.5);
    assert!((toss.z - GRAVITY * ((300. - 32.) / (0.5 * GRAVITY)).sqrt()).abs() < 0.5);
    // Targets 500 units above the thrower are refused.
    assert!(vec_check_toss(
        &physics,
        0,
        start,
        target + Vec3::Z * 600.,
        Vec3::X,
        [0.5; 4]
    )
    .is_none());
}
#[test]
fn a_soldier_flushes_an_enemy_that_just_dropped_out_of_sight() {
    let world = World {
        entities: vec![entity(&[
            ("classname", "npc_combine_s"),
            ("origin", "0 0 0"),
            ("angles", "0 0 0"),
            ("additionalequipment", "weapon_smg1"),
            ("NumGrenades", "5"),
        ])],
        ..Default::default()
    };
    let scene = Scene::new(&world);
    let physics = Physics::new(&world);
    let ai = AiRuntime::new(&world, None);
    let mut r = Rig {
        world,
        scene,
        physics,
        controller: Controller::new(None, Default::default()),
        ai,
    };
    r.ai.enabled = true;
    assert!(matches!(&r.ai.actors[&0].class, AiClass::Humanoid(h) if h.grenades == 5));
    // The player was seen 1 s ago at (-300, 0) and now stands behind the soldier,
    // 100 units from there: out of sight, so the soldier tosses at the last seen spot.
    let behind = player_at(-400.);
    r.scene.time = 2.;
    {
        let actor = r.ai.actors.get_mut(&0).unwrap();
        actor.npc.state = NpcState::Combat;
        actor.npc.ideal_state = NpcState::Combat;
        actor.npc.enemy = Some(PLAYER);
        actor
            .npc
            .enemies
            .update(Some(PLAYER), Vec3::new(-300., 0., 0.), 0., true, 1.);
    }
    let mut schedules = Vec::new();
    for _ in 0..20 {
        run(&mut r, 1, behind);
        schedules.push(r.ai.actors[&0].npc.schedule.clone().unwrap_or_default());
    }
    assert!(
        schedules.iter().any(|s| s == "SCHED_COMBINE_RANGE_ATTACK2"),
        "{schedules:?}"
    );
    // COMBINE_AE_GREN_TOSS: a frag with the toss velocity, one grenade fewer.
    r.ai.actors
        .get_mut(&0)
        .unwrap()
        .class
        .requests_mut()
        .push(CopRequest::GrenadeToss);
    r.ai.apply_requests(0, &r.world, &mut r.scene, &r.physics, behind);
    assert_eq!(r.ai.grenade_tosses.len(), 1);
    assert!(r.ai.grenade_tosses[0].2.x < 0.);
    assert!(matches!(&r.ai.actors[&0].class, AiClass::Humanoid(h) if h.grenades == 4));
}
#[test]
fn set_police_goal_enables_policing_warnings_and_goal_outputs() {
    let mut r = rig();
    // logic_auto-style input before the AI is switched on still enables the goal.
    r.scene
        .ai_inputs
        .push((0, "setpolicegoal".into(), "post".into(), usize::MAX));
    run(&mut r, 1, player_at(500.));
    let cop = r.ai.actors[&0].class.cop().expect("cop");
    assert!(cop.policing.is_some());
    r.ai.enabled = true;
    // Inside 2 x PoliceRadius (200) but outside the radius and the shove range.
    run(&mut r, 40, player_at(150.));
    let cop = r.ai.actors[&0].class.cop().expect("cop");
    let policing = cop.policing.as_ref().unwrap();
    assert!(policing.warnings >= 1, "{:?}", r.ai.actors[&0].npc.schedule);
    assert!(r
        .scene
        .sounds
        .iter()
        .any(|s| s.name == "#METROPOLICE_MOVE_ALONG_A"));
    // An unknown goal name is counted, not panicking.
    r.scene
        .ai_inputs
        .push((0, "setpolicegoal".into(), "nowhere".into(), usize::MAX));
    run(&mut r, 1, player_at(150.));
    assert_eq!(r.ai.diagnostics.unknown_goals["nowhere"], 1);
}
#[test]
fn soldier_and_rebel_spawns_fight_each_other_with_their_class_schedules() {
    // Synthetic open ground: an F4-style foe soldier at the origin facing +X and a
    // friendly armed citizen 300 units ahead facing it (owner report: only cops
    // fought; the other classes stood idle with their weapons).
    let world = World {
        entities: vec![
            entity(&[
                ("classname", "npc_combine_s"),
                ("origin", "0 0 0"),
                ("angles", "0 0 0"),
                ("additionalequipment", "weapon_smg1"),
            ]),
            entity(&[
                ("classname", "npc_citizen"),
                ("origin", "300 0 0"),
                ("angles", "0 180 0"),
                ("additionalequipment", "weapon_smg1"),
            ]),
        ],
        ..Default::default()
    };
    let scene = Scene::new(&world);
    let physics = Physics::new(&world);
    let ai = AiRuntime::new(&world, None);
    let mut r = Rig {
        world,
        scene,
        physics,
        controller: Controller::new(None, Default::default()),
        ai,
    };
    assert!(r.ai.actors[&0].class.humanoid().is_some());
    assert!(r.ai.actors[&1].class.humanoid().is_some());
    r.ai.set_player_disposition(0, Some(Disposition::HT));
    r.ai.set_player_disposition(1, Some(Disposition::LI));
    r.ai.enabled = true;
    run(&mut r, 30, player_at(-3000.));
    assert_eq!(r.ai.actors[&0].npc.enemy, Some(1));
    assert_eq!(r.ai.actors[&1].npc.enemy, Some(0));
    assert_eq!(r.ai.actors[&0].npc.state, NpcState::Combat);
    let soldier = r.ai.actors[&0].npc.schedule.clone().unwrap_or_default();
    let rebel = r.ai.actors[&1].npc.schedule.clone().unwrap_or_default();
    assert!(soldier.starts_with("SCHED_COMBINE_"), "{soldier}");
    assert!(
        matches!(
            rebel.as_str(),
            "SCHED_RANGE_ATTACK1" | "SCHED_COMBAT_FACE" | "SCHED_WAKE_ANGRY"
        ),
        "{rebel}"
    );
    // Their ammo checks and the weapon's attack conditions run for every class.
    assert!(r.ai.actors[&1]
        .npc
        .unsupported_tasks
        .keys()
        .all(|k| !k.starts_with("schedule ")));
}
