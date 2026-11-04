//! Scene playback: speech, actors, sections and sequences.
use super::*;

fn choreography(events: &[(EventType, f32, &str)]) -> Arc<ChoreoScene> {
    let mut data = b"bvcd\x04".to_vec();
    data.extend(0u32.to_le_bytes());
    data.push(events.len() as u8);
    let mut strings = vec!["event".into(), "".into()];
    for (kind, start, parameter) in events {
        data.push(*kind as u8);
        data.extend(0i16.to_le_bytes());
        data.extend(start.to_le_bytes());
        data.extend((-1f32).to_le_bytes());
        data.extend((strings.len() as i16).to_le_bytes());
        strings.push((*parameter).into());
        data.extend(1i16.to_le_bytes());
        data.extend(1i16.to_le_bytes());
        data.extend([0, 8]);
        data.extend(0f32.to_le_bytes());
        data.extend([0, 0, 0, 0]);
        if *kind == EventType::Gesture {
            data.extend((-1f32).to_le_bytes());
        }
        data.extend([0, 0]);
        if *kind == EventType::Loop {
            data.push(255);
        }
        if *kind == EventType::Speak {
            data.push(0);
            data.extend(1i16.to_le_bytes());
            data.push(0);
        }
    }
    data.extend([0, 0, 0]);
    Arc::new(ChoreoScene::parse(&data, &strings).unwrap())
}
#[test]
fn scene_speech_captions_reach_the_player_through_a_microphone_speaker() {
    // ep1_background02: Kleiner speaks 6000 units away; a microphone beside him replays
    // the line at a speaker next to the player.
    let run = |microphone_disabled: &str| {
        let world = World {
            entities: vec![
                entity("logic_choreographed_scene", "scene", &[]),
                entity("npc_kleiner", "kleiner", &[("origin", "6000 0 0")]),
                entity(
                    "env_microphone",
                    "mic",
                    &[
                        ("origin", "6000 40 0"),
                        ("MaxRange", "100"),
                        ("SpeakerName", "speaker"),
                        ("StartDisabled", microphone_disabled),
                    ],
                ),
                entity("info_target", "speaker", &[("origin", "20 0 0")]),
            ],
            ..Default::default()
        };
        let mut data =
            Arc::unwrap_or_clone(choreography(&[(EventType::Speak, 0., "EP_01.KL_Line")]));
        data.actors.push(source_assets::scenes::Actor {
            name: "kleiner".into(),
            active: true,
            channels: Vec::new(),
        });
        data.events[0].actor = Some(0);
        data.events[0].end = Some(3.5);
        let mut scene = Scene::new(&world);
        scene.install_choreography(0, Arc::new(data));
        scene.send(0, "Start", "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        scene.tick(&world, Vec3::ZERO, 0.1);
        scene
    };
    let scene = run("0");
    assert_eq!(scene.sounds, vec!["EP_01.KL_Line"]);
    assert!(
        !scene.sounds[0].caption,
        "the scene captions the line itself"
    );
    assert_eq!(scene.captions.len(), 1);
    let caption = &scene.captions[0];
    assert_eq!(caption.token, "ep_01.kl_line");
    assert_eq!(caption.duration, 3.5);
    assert!(!caption.from_player);
    assert_eq!(caption.actor.as_ref().and_then(|a| a.entity), Some(1));
    let scene = run("1");
    assert_eq!(scene.sounds.len(), 1);
    assert!(scene.captions.is_empty(), "beyond scene_maxcaptionradius");
    assert_eq!(scene.diagnostics.attenuated_captions, 1);
}
#[test]
fn face_turns_standing_npc_toward_target_at_max_yaw_speed() {
    let world = World {
        entities: vec![
            entity("logic_choreographed_scene", "scene", &[]),
            entity("npc_citizen", "actor", &[]),
            entity("info_target", "mark", &[("origin", "0 100 0")]),
        ],
        ..Default::default()
    };
    let mut data = Arc::unwrap_or_clone(choreography(&[(EventType::Face, 0., "mark")]));
    data.actors.push(source_assets::scenes::Actor {
        name: "actor".into(),
        active: true,
        channels: Vec::new(),
    });
    data.events[0].actor = Some(0);
    data.events[0].end = Some(2.);
    let mut scene = Scene::new(&world);
    scene.install_choreography(0, Arc::new(data));
    let yaw = |scene: &Scene| {
        let f = scene.states[1].rotation * Vec3::X;
        f.y.atan2(f.x).to_degrees()
    };
    assert!(yaw(&scene).abs() < 1e-3);
    scene.send(0, "Start", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    scene.tick(&world, Vec3::ZERO, 0.1);
    // 450 degrees per second: 45 degrees in this 0.1 sec step toward the 90 degree goal.
    let partial = yaw(&scene);
    assert!(partial > 40. && partial < 90., "{partial}");
    for _ in 0..5 {
        scene.tick(&world, Vec3::ZERO, 0.1);
    }
    assert!((yaw(&scene) - 90.).abs() < 1e-3, "{}", yaw(&scene));
    assert!(!scene
        .diagnostics
        .unsupported
        .contains_key("logic_choreographed_scene.actor-event:Face"));
    assert_eq!(angle_diff(10., 350.), 20.);
    assert_eq!(clamp_yaw(5., 358., 10.), 3.);
}
#[test]
fn lookat_binds_target_alias_once_and_reports_missing_target_without_cycler_interest() {
    use crate::attention::Target;
    let mut world = World {
        entities: vec![
            entity("logic_choreographed_scene", "scene", &[("target1", "MARK")]),
            entity("npc_citizen", "actor", &[]),
            entity("cycler_actor", "cycler", &[]),
            entity("info_target", "mark", &[]),
        ],
        ..Default::default()
    };
    let mut data = Arc::unwrap_or_clone(choreography(&[
        (EventType::LookAt, 0., "!TaRgEt1"),
        (EventType::LookAt, 0., "!player"),
        (EventType::LookAt, 0.1, "absent"),
    ]));
    for name in ["actor", "cycler"] {
        data.actors.push(source_assets::scenes::Actor {
            name: name.into(),
            active: true,
            channels: Vec::new(),
        });
    }
    for (id, e) in data.events.iter_mut().enumerate() {
        e.actor = Some(if id == 1 { 1 } else { 0 });
        e.end = Some(1.);
    }
    let mut scene = Scene::new(&world);
    scene.install_choreography(0, Arc::new(data));
    scene.send(0, "Start", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    scene.tick(&world, Vec3::ZERO, 0.05);
    assert_eq!(scene.look_targets.report()[&1][0].target, Target::Entity(3));
    assert!(!scene.look_targets.report().contains_key(&2));
    // Renaming a surviving entity does not re-resolve an already-started event.
    world.entities[3].properties = vec![
        ("classname".into(), "info_target".into()),
        ("targetname".into(), "renamed".into()),
    ];
    scene.tick(&world, Vec3::ZERO, 0.1);
    assert_eq!(scene.look_targets.report()[&1][0].target, Target::Entity(3));
    assert_eq!(
        scene.diagnostics.unsupported["logic_choreographed_scene.LOOKAT:missing-target"],
        1
    );
}
#[test]
fn cycler_actor_named_and_target_aliases_deliver_real_actor_events() {
    let world = World {
        entities: vec![
            entity(
                "logic_choreographed_scene",
                "scene",
                &[("target1", "gman"), ("OnTrigger1", "gate,Add,1,0,-1")],
            ),
            // A same-name non-actor must not shadow the flex-capable actor.
            entity("logic_relay", "gman", &[]),
            entity("cycler_actor", "gman", &[]),
            entity("math_counter", "gate", &[]),
            entity("cycler_actor_extra", "unproved_class", &[]),
            entity("npc_citizen", "existing_npc", &[]),
            entity("prop_dynamic", "existing_prop", &[]),
        ],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    assert_eq!(scene.scene_actor(&world, 0, "GmAn", usize::MAX), Some(2));
    assert_eq!(
        scene.scene_actor(&world, 0, "!TaRgEt1", usize::MAX),
        Some(2)
    );
    assert_eq!(
        scene.scene_actor(&world, 0, "unproved_class", usize::MAX),
        None
    );
    assert_eq!(
        scene.scene_actor(&world, 0, "existing_npc", usize::MAX),
        Some(5)
    );
    assert_eq!(
        scene.scene_actor(&world, 0, "existing_prop", usize::MAX),
        Some(6)
    );
    let mut data = Arc::unwrap_or_clone(choreography(&[
        (EventType::Speak, 0., "OriginalTest.ActorLine"),
        (EventType::FireTrigger, 0.1, "1"),
        (EventType::StopPoint, 5., "noaction"),
    ]));
    for name in ["GmAn", "!TaRgEt1"] {
        data.actors.push(source_assets::scenes::Actor {
            name: name.into(),
            active: true,
            channels: Vec::new(),
        });
    }
    data.events[0].actor = Some(0);
    data.events[1].actor = Some(1);
    scene.install_choreography(0, Arc::new(data));
    scene.send(0, "Start", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    scene.tick(&world, Vec3::ZERO, 0.2);
    assert_eq!(scene.sounds, vec!["OriginalTest.ActorLine"]);
    assert_eq!(scene.states[3].value, 1.);
    assert_eq!(scene.diagnostics.scene_events_started, 2);
    assert!(!scene
        .diagnostics
        .unsupported
        .contains_key("logic_choreographed_scene.missing-actor"));
    assert!(scene
        .animation_states(&world)
        .iter()
        .any(|s| s.entity == 2 && s.targetname == "gman"));
    scene.states[2].killed = true;
    assert_eq!(scene.scene_actor(&world, 0, "!target1", usize::MAX), None);
    assert!(!scene.animation_states(&world).iter().any(|s| s.entity == 2));
}
#[test]
#[ignore = "requires an installed owned HL2 copy; actor I/O, not intro camera/face/AI parity"]
fn owned_gman_intro_resolves_cycler_actor_and_starts_authored_speech() {
    let game = source_assets::install::discover().unwrap();
    let data = std::fs::read(game.join("hl2/maps/d1_trainstation_01.bsp")).unwrap();
    let bsp = source_assets::bsp::Bsp::parse(&data).unwrap();
    let world = bsp.world("d1_trainstation_01").unwrap();
    let vfs = Vfs::mount(&game).unwrap();
    let mut scene = Scene::new(&world);
    scene.queue.clear();
    scene.load_choreography(&world, &vfs).unwrap();
    let id = world
        .entities
        .iter()
        .position(|e| e.get("targetname") == Some("scene2_lcs_intro"))
        .unwrap();
    let actor = world
        .entities
        .iter()
        .position(|e| e.get("targetname") == Some("gman"))
        .unwrap();
    assert_eq!(world.entities[actor].class(), "cycler_actor");
    assert_eq!(
        world.entities[actor].get("model"),
        Some("models/gman_high.mdl")
    );
    assert_eq!(
        scene.scene_actor(&world, id, "!target1", usize::MAX),
        Some(actor)
    );
    scene.send(id, "Start", "");
    for _ in 0..1201 {
        scene.tick(&world, Vec3::ZERO, 0.015);
    }
    assert_eq!(scene.diagnostics.scene_events_started, 17);
    assert!(!scene
        .diagnostics
        .unsupported
        .contains_key("logic_choreographed_scene.missing-actor"));
    assert!(scene.sounds.iter().any(|s| s == "Trainride.gman_riseshine"));
    assert!(scene.sounds.iter().any(|s| s == "Trainride.gman_02"));
}
#[test]
fn scene_stop_point_completes_once_without_cutting_off_tail() {
    let world = World {
        entities: vec![
            entity(
                "logic_choreographed_scene",
                "scene",
                &[
                    ("OnStart", "started,Add,1,0,-1"),
                    ("OnCompletion", "finished,Add,1,0,-1"),
                    ("OnTrigger1", "tail,Add,1,0,-1"),
                ],
            ),
            entity("math_counter", "started", &[]),
            entity("math_counter", "finished", &[]),
            entity("math_counter", "tail", &[]),
        ],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    scene.install_choreography(
        0,
        choreography(&[
            (EventType::StopPoint, 0.2, "noaction"),
            (EventType::FireTrigger, 0.5, "1"),
        ]),
    );
    scene.send(0, "Start", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    scene.send(0, "Start", "");
    scene.tick(&world, Vec3::ZERO, 0.25);
    assert_eq!(scene.states[1].value, 1.);
    assert_eq!(scene.states[2].value, 1.);
    assert_eq!(scene.states[3].value, 0.);
    assert!(scene.choreography[&0].playback.is_some());
    scene.tick(&world, Vec3::ZERO, 0.5);
    assert_eq!(scene.states[2].value, 1.);
    assert_eq!(scene.states[3].value, 1.);
    assert_eq!(scene.diagnostics.scene_completions, 1);
    assert!(scene.choreography[&0].playback.is_none());
}
#[test]
fn scene_input_pause_resume_and_cancel_do_not_invent_completion() {
    let world = World {
        entities: vec![
            entity(
                "logic_choreographed_scene",
                "scene",
                &[
                    ("OnCompletion", "count,Add,100,0,-1"),
                    ("OnCanceled", "count,Add,10,0,-1"),
                    ("OnTrigger1", "count,Add,1,0,-1"),
                ],
            ),
            entity("math_counter", "count", &[]),
        ],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    scene.install_choreography(
        0,
        choreography(&[
            (EventType::FireTrigger, 1., "1"),
            (EventType::StopPoint, 2., "noaction"),
        ]),
    );
    scene.send(0, "Start", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    scene.send(0, "Pause", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    let before = scene.choreography[&0].playback.as_ref().unwrap().elapsed;
    scene.tick(&world, Vec3::ZERO, 4.);
    assert_eq!(
        scene.choreography[&0].playback.as_ref().unwrap().elapsed,
        before
    );
    assert_eq!(scene.states[1].value, 0.);
    scene.send(0, "Resume", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    scene.tick(&world, Vec3::ZERO, 1.1);
    assert_eq!(scene.states[1].value, 1.);
    scene.send(0, "Cancel", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    scene.tick(&world, Vec3::ZERO, 4.);
    assert_eq!(scene.states[1].value, 11.);
    assert_eq!(scene.diagnostics.scene_completions, 0);
}
#[test]
fn scene_actor_conditions_hold_section_until_explicit_resume() {
    let world = World {
        entities: vec![
            entity(
                "logic_choreographed_scene",
                "scene",
                &[
                    ("target1", "actor"),
                    ("OnTrigger1", "door,Unlock,,0,-1"),
                    ("OnTrigger1", "door,Open,,0.1,-1"),
                ],
            ),
            entity("npc_citizen", "actor", &[]),
            entity("prop_door_rotating", "door", &[("locked", "1")]),
        ],
        ..Default::default()
    };
    let mut data = Arc::unwrap_or_clone(choreography(&[
        (EventType::MoveTo, 0., "mark"),
        (EventType::Section, 0.1, "noaction"),
        (EventType::FireTrigger, 0.2, "1"),
    ]));
    data.events[0].actor = Some(0);
    data.events[0].flags |= 1;
    data.actors.push(source_assets::scenes::Actor {
        name: "!target1".into(),
        active: true,
        channels: Vec::new(),
    });
    let mut scene = Scene::new(&world);
    scene.install_choreography(0, Arc::new(data));
    scene.send(0, "Start", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    scene.tick(&world, Vec3::ZERO, 2.);
    assert!(scene.states[2].locked);
    assert_eq!(scene.states[2].target, 0.);
    scene.tick(&world, Vec3::ZERO, 2.);
    assert!(scene.states[2].locked);
    assert!(scene
        .diagnostics
        .unsupported
        .contains_key("logic_choreographed_scene.SECTION:actor-resume-condition"));
    scene.send(0, "Resume", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    scene.tick(&world, Vec3::ZERO, 0.11);
    assert!(!scene.states[2].locked);
    assert_eq!(scene.states[2].target, 0.);
    scene.tick(&world, Vec3::ZERO, 0.11);
    assert_eq!(scene.states[2].target, 1.);
}
#[test]
fn movement_arrival_resolves_section_and_cancel_restores_body_animation() {
    let world = World {
        entities: vec![
            entity(
                "logic_choreographed_scene",
                "scene",
                &[("target1", "actor"), ("OnTrigger1", "count,Add,1,0,-1")],
            ),
            entity("npc_barney", "actor", &[("DefaultAnim", "idle")]),
            entity("info_target", "mark", &[("origin", "100 0 0")]),
            entity("math_counter", "count", &[]),
        ],
        ..Default::default()
    };
    let mut data = Arc::unwrap_or_clone(choreography(&[
        (EventType::MoveTo, 0., "mark"),
        (EventType::Section, 0.1, "noaction"),
        (EventType::FireTrigger, 0.2, "1"),
    ]));
    data.events[0].actor = Some(0);
    data.events[0].parameters[1] = "run".into();
    data.events[0].flags |= 1;
    data.actors.push(source_assets::scenes::Actor {
        name: "!target1".into(),
        active: true,
        channels: Vec::new(),
    });
    let mut scene = Scene::new(&world);
    scene.queue.clear();
    scene.install_choreography(0, Arc::new(data));
    scene.send(0, "Start", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    scene.tick(&world, Vec3::ZERO, 0.2);
    let key = crate::npc::GoalKey {
        scene: 0,
        event: 0,
        actor: 1,
    };
    assert!(scene.movement_commands.iter().any(|command| matches!(command, SceneMoveCommand::Start {key: k, request} if *k == key && request.target_entity == 2 && request.target_feet == Vec3::new(100.,0.,0.))));
    let frozen = scene.choreography[&0].playback.as_ref().unwrap().elapsed;
    scene.apply_movement(
        key,
        Vec3::new(20., 0., 0.),
        0.,
        Some(("run_all", 0.3)),
        false,
    );
    scene.tick(&world, Vec3::ZERO, 1.);
    assert_eq!(
        scene.choreography[&0].playback.as_ref().unwrap().elapsed,
        frozen
    );
    assert_eq!(scene.states[3].value, 0.);
    assert_eq!(scene.animation_time(1), 0.3);
    scene.apply_movement(
        key,
        Vec3::new(40., 0., 0.),
        0.,
        Some(("run_all", 0.4)),
        false,
    );
    assert_eq!(scene.animation_time(1), 0.4);
    scene.states[1].scripted_by = Some(99);
    scene.apply_movement(key, Vec3::new(100., 0., 0.), 0., None, true);
    assert!(!scene.movement_ready[&key]);
    assert_eq!(scene.states[1].origin.x, 40.);
    scene.states[1].scripted_by = None;
    scene.states[1].animation = "idle".into();
    scene.apply_movement(key, Vec3::new(100., 0., 0.), 0., None, true);
    scene.tick(&world, Vec3::ZERO, 0.2);
    assert_eq!(scene.states[3].value, 1.);
    scene.tick(&world, Vec3::ZERO, 1.);
    scene.send(0, "Start", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    scene.tick(&world, Vec3::ZERO, 0.015);
    scene.apply_movement(
        key,
        Vec3::new(20., 0., 0.),
        0.,
        Some(("run_all", 0.1)),
        false,
    );
    scene.send(0, "Cancel", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    assert_eq!(scene.states[1].animation, "idle");
    assert!(!scene.movement_ready.contains_key(&key));
    assert!(scene
        .movement_commands
        .iter()
        .any(|c| matches!(c, SceneMoveCommand::CancelScene(0))));
    scene.apply_movement(key, Vec3::new(100., 0., 0.), 0., None, true);
    assert_eq!(scene.states[1].origin.x, 20.);
}
#[test]
fn scene_sequence_uses_installed_clip_scene_clock_and_restores_baseline() {
    use modkit_core::{
        animation::{Clip, Rig},
        ModelInstance,
    };
    let world = World {
        entities: vec![
            entity(
                "logic_choreographed_scene",
                "scene",
                &[("target1", "actor")],
            ),
            entity("npc_citizen", "actor", &[("DefaultAnim", "idle")]),
        ],
        model_instances: vec![ModelInstance {
            background: false,
            model: "actor.mdl".into(),
            origin: Vec3::ZERO,
            angles: Vec3::ZERO,
            skin: 2,
            body: 0,
            scale: 1.,
            kind: "npc_citizen".into(),
            solid: false,
            solid_mode: None,
            entity: Some(1),
        }],
        rigs: BTreeMap::from([(
            "actor.mdl#2".into(),
            Rig {
                clips: BTreeMap::from([(
                    "performance".into(),
                    Clip {
                        layer: Default::default(),
                        fps: 30.,
                        looping: false,
                        frames: vec![vec![]; 31],
                        events: Vec::new(),
                    },
                )]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let mut data = Arc::unwrap_or_clone(choreography(&[
        (EventType::Sequence, 0., "Performance"),
        (EventType::Gesture, 0.25, "wave"),
        (EventType::StopPoint, 3., "noaction"),
    ]));
    data.actors.push(source_assets::scenes::Actor {
        name: "!target1".into(),
        active: true,
        channels: Vec::new(),
    });
    for event in &mut data.events[..2] {
        event.actor = Some(0);
        event.end = Some(2.);
    }
    let mut scene = Scene::new(&world);
    scene.install_choreography(0, Arc::new(data));
    assert_eq!(
        scene.required_animation_clips(&world),
        BTreeMap::from([(
            "actor.mdl#2".into(),
            BTreeSet::from(["performance".into(), "wave".into()])
        ),])
    );
    scene.send(0, "Start", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    scene.tick(&world, Vec3::ZERO, 0.4);
    // A gesture without layer support must not replace the actual body sequence.
    assert_eq!(scene.states[1].animation, "performance");
    assert!((scene.animation_time(1) - 0.4).abs() < 1e-6);
    scene.send(0, "Pause", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    let frozen = scene.animation_time(1);
    scene.tick(&world, Vec3::ZERO, 5.);
    assert_eq!(scene.states[1].animation, "performance");
    assert_eq!(scene.animation_time(1), frozen);
    let clocks = scene.choreography_states(&world);
    assert_eq!(clocks[0].pause_reason, Some("input"));
    assert!(clocks[0].active && clocks[0].pause);
    let samples = scene.animation_states(&world);
    assert_eq!(samples[0].scene_owner, Some(0));
    assert_eq!(samples[0].sample_time, frozen);
    assert!(samples[0].clip_loaded);
    scene.send(0, "Resume", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    scene.tick(&world, Vec3::ZERO, 0.5);
    assert!((scene.animation_time(1) - frozen - 0.5).abs() < 1e-6);
    scene.tick(&world, Vec3::ZERO, 1.1);
    assert_eq!(scene.states[1].animation, "idle");
    assert!(scene.choreography[&0].playback.is_some());
    // Cancel restores the baseline too, rather than leaving the NPC in a scene pose.
    scene.send(0, "Cancel", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    scene.send(0, "Start", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    scene.tick(&world, Vec3::ZERO, 0.25);
    assert_eq!(scene.states[1].animation, "performance");
    scene.send(0, "Cancel", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    assert_eq!(scene.states[1].animation, "idle");
    let clocks = scene.choreography_states(&world);
    assert!(!clocks[0].active);
    assert!(clocks[0].elapsed.is_none());
    assert!(scene.animation_states(&world)[0].scene_owner.is_none());
    let mut missing =
        Arc::unwrap_or_clone(choreography(&[(EventType::Sequence, 0., "missing_clip")]));
    missing.actors.push(source_assets::scenes::Actor {
        name: "actor".into(),
        active: true,
        channels: Vec::new(),
    });
    missing.events[0].actor = Some(0);
    missing.events[0].end = Some(1.);
    scene.install_choreography(0, Arc::new(missing));
    scene.send(0, "Start", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    scene.tick(&world, Vec3::ZERO, 0.1);
    assert_eq!(scene.states[1].animation, "idle");
    assert!(scene
        .diagnostics
        .unsupported
        .contains_key("npc_citizen.animation:missing_clip"));
}
