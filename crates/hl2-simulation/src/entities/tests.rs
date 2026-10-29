use super::*;
use glam::Mat4;
use source_assets::scenes::EventType;
use source_assets::vpk::Vfs;
#[test]
fn sequences_finish_at_the_last_visible_cycle_like_studio_frame_advance() {
    let clip = |frames: usize, fps: f32, looping: bool| modkit_core::animation::Clip {
        events: Vec::new(),
        fps,
        looping,
        frames: vec![Vec::new(); frames],
        layer: modkit_core::animation::ClipLayer {
            fade_out: 0.2,
            ..Default::default()
        },
    };
    // shootSMG1s (5 frames at 30 fps, 0.133 s): finished at the first advance.
    assert!((sequence_finish(&clip(5, 30., false)) - 0.001).abs() < 1e-6);
    // reload (66 frames at 30 fps, 2.167 s): 0.2 s before the end.
    assert!((sequence_finish(&clip(66, 30., false)) - (65. / 30. - 0.2)).abs() < 1e-5);
    // Looping sequences finish when they wrap.
    assert!((sequence_finish(&clip(66, 30., true)) - 65. / 30.).abs() < 1e-5);
}
#[test]
fn runtime_entities_append_after_map_entities() {
    // Synthetic: one map entity, then a spawned cop.
    let mut world = World {
        entities: vec![Entity {
            properties: vec![("classname".into(), "info_target".into())],
        }],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    let id = scene.spawn_entity(
        &mut world,
        Entity {
            properties: vec![
                ("classname".into(), "npc_metropolice".into()),
                ("origin".into(), "10 20 30".into()),
            ],
        },
    );
    assert_eq!(id, 1);
    assert_eq!(scene.states.len(), 2);
    assert_eq!(scene.states[id].origin, Vec3::new(10., 20., 30.));
    assert!(scene.states[id].visible);
    assert_eq!(scene.states[id].animation, "idle_baton");
    assert_eq!(scene.entity(&world, id).unwrap().class(), "npc_metropolice");
    assert_eq!(scene.entity(&world, 0).unwrap().class(), "info_target");
    assert!(scene.entity(&world, 2).is_none());
    // Runtime loops (I/O queue, heads, look targets, physics poses) tolerate the
    // spawned id: tick a while with the player nearby and an input sent to it.
    let mut physics = crate::physics::Physics::new(&world);
    scene.send(id, "Kill", "");
    for _ in 0..40 {
        scene.tick(&world, Vec3::new(40., 20., 30.), 0.015);
        for (index, state) in scene.states.iter().enumerate() {
            physics.set_entity(index, state.origin, state.rotation, state.collides());
        }
    }
    assert!(scene.states[id].killed);
}
fn entity(class: &str, name: &str, values: &[(&str, &str)]) -> Entity {
    Entity {
        properties: [("classname", class), ("targetname", name)]
            .into_iter()
            .chain(values.iter().copied())
            .map(|(k, v)| (k.into(), v.into()))
            .collect(),
    }
}
#[test]
fn random_timers_filters_and_cancel_pending_follow_the_sdk() {
    let world = World {
        entities: vec![
            entity(
                "logic_timer",
                "nag",
                &[
                    ("UseRandomTime", "1"),
                    ("LowerRandomBound", "10"),
                    ("UpperRandomBound", "20"),
                    ("OnTimer", "relay,Trigger,,0,-1"),
                ],
            ),
            entity(
                "filter_activator_name",
                "only_can",
                &[
                    ("filtername", "can"),
                    ("Negated", "Allow entities that match criteria"),
                ],
            ),
            entity(
                "filter_activator_name",
                "not_can",
                &[("filtername", "can"), ("Negated", "1")],
            ),
            entity("prop_physics", "can", &[]),
            entity(
                "logic_relay",
                "relay",
                &[("OnTrigger", "nag,Disable,,5,-1")],
            ),
        ],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    // CTimerEntity::ResetTimer: the first delay is RandomFloat(10, 20), not RefireTime.
    let first = scene.states[0].timer_at;
    assert!((10. ..=20.).contains(&first), "{first}");
    // PassesFilter: the name filter (Hammer's text Negated parses as 0) and its negation.
    assert!(scene.passes_filter(&world, "only_can", 3));
    assert!(!scene.passes_filter(&world, "only_can", usize::MAX));
    assert!(!scene.passes_filter(&world, "not_can", 3));
    assert!(scene.passes_filter(&world, "missing", 3));
    // CancelPending drops the relay's delayed outputs.
    scene.send(4, "Trigger", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    assert!(scene.queue.iter().any(|q| q.caller == 4));
    scene.send(4, "CancelPending", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    assert!(!scene.queue.iter().any(|q| q.caller == 4));
}
#[test]
fn ambient_inputs_follow_sdk_active_state() {
    use crate::sounds::AmbientControl::{Play, Stop};
    let world = World {
        entities: vec![
            // Looping, start silent, play everywhere: a barrier's touch hum.
            entity(
                "ambient_generic",
                "close",
                &[("message", "hum"), ("spawnflags", "17")],
            ),
            // Not looping: every PlaySound plays it again.
            entity(
                "ambient_generic",
                "once",
                &[("message", "beep"), ("spawnflags", "48")],
            ),
        ],
        ..World::default()
    };
    let mut scene = Scene::new(&world);
    let run = |scene: &mut Scene, id: usize, input: &str| {
        scene.send(id, input, "");
        scene.tick(&world, Vec3::ZERO, 0.015);
        std::mem::take(&mut scene.sounds)
            .into_iter()
            .filter_map(|r| r.ambient)
            .collect::<Vec<_>>()
    };
    assert_eq!(run(&mut scene, 0, "PlaySound"), [Play(0)]);
    assert_eq!(run(&mut scene, 0, "PlaySound"), []);
    assert_eq!(run(&mut scene, 0, "StopSound"), [Stop(0)]);
    assert_eq!(run(&mut scene, 0, "StopSound"), []);
    assert_eq!(run(&mut scene, 0, "ToggleSound"), [Play(0)]);
    assert_eq!(run(&mut scene, 0, "ToggleSound"), [Stop(0)]);
    assert_eq!(run(&mut scene, 1, "PlaySound"), [Play(1)]);
    assert_eq!(run(&mut scene, 1, "PlaySound"), [Play(1)]);
    assert_eq!(run(&mut scene, 1, "StopSound"), []);
}
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
#[test]
fn named_input_uses_deferred_glob_delivery_and_rejects_invalid_delay() {
    let world = World {
        entities: vec![
            entity("math_counter", "count_a", &[]),
            entity("math_counter", "count_b", &[]),
        ],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    assert!(!scene.send_named("count*", "Add", "10", f64::NAN));
    assert!(!scene.send_named("count*", "Add", "10", -1.));
    assert!(scene.send_named("count*", "Add", "2", 0.1));
    assert!(scene.send_named("COUNT_A", "Add", "3", 0.1));
    scene.tick(&world, Vec3::ZERO, 0.05);
    assert_eq!(scene.states[0].value, 0.);
    scene.tick(&world, Vec3::ZERO, 0.06);
    assert_eq!(scene.states[0].value, 5.);
    assert_eq!(scene.states[1].value, 2.);
}
#[test]
#[ignore = "requires an installed owned HL2 copy; isolated scene I/O, not campaign parity"]
fn owned_security03_blocks_movement_condition_and_delivers_real_gate_after_resume() {
    let game = source_assets::install::discover().unwrap();
    let data = std::fs::read(game.join("hl2/maps/d1_trainstation_01.bsp")).unwrap();
    let bsp = source_assets::bsp::Bsp::parse(&data).unwrap();
    let world = bsp.world("d1_trainstation_01").unwrap();
    let vfs = Vfs::mount(&game).unwrap();
    let mut scene = Scene::new(&world);
    // Isolate this one scene from the unrelated camera/train introductory chain.
    scene.queue.clear();
    assert_eq!(scene.load_choreography(&world, &vfs).unwrap(), 44);
    let id = world
        .entities
        .iter()
        .position(|e| e.get("targetname") == Some("security_03"))
        .unwrap();
    let door = world
        .entities
        .iter()
        .position(|e| e.get("targetname") == Some("storage_room_door"))
        .unwrap();
    assert!(scene.states[door].locked);
    // Barney is a point_template child; spawn him as the campaign does on arrival.
    assert!(scene.send_named("gordon_cop_template", "ForceSpawn", "", 0.));
    scene.tick(&world, Vec3::ZERO, 0.015);
    scene.queue.clear();
    assert!(scene.send_named("security_03", "Start", "", 0.));
    for _ in 0..600 {
        scene.tick(&world, Vec3::ZERO, 0.015);
    }
    let playback = scene.choreography[&id].playback.as_ref().unwrap();
    assert!((playback.elapsed - 6.88063383102417).abs() < 1e-6);
    assert!(scene.states[door].locked);
    scene.send_named("security_03", "Resume", "", 0.);
    for _ in 0..25 {
        scene.tick(&world, Vec3::ZERO, 0.015);
    }
    assert!(!scene.states[door].locked);
    assert_eq!(scene.states[door].target, 1.);
    let parsed = &scene.choreography[&id].data;
    assert!(parsed
        .events
        .iter()
        .any(|e| e.kind == EventType::FireTrigger
            && e.parameters[0] == "3"
            && (e.start - 7.013968).abs() < 1e-6));
    assert!(parsed
        .events
        .iter()
        .any(|e| e.kind == EventType::StopPoint && (e.start - 9.59397).abs() < 1e-6));
}
#[test]
#[ignore = "requires owned HL2 scene cache"]
fn owned_template_actors_get_scene_gesture_clips_before_force_spawn() {
    let game = source_assets::install::discover().unwrap();
    let data = std::fs::read(game.join("hl2/maps/d1_trainstation_01.bsp")).unwrap();
    let mut world = source_assets::bsp::Bsp::parse(&data)
        .unwrap()
        .world("d1_trainstation_01")
        .unwrap();
    let vfs = Vfs::mount(&game).unwrap();
    let entity = |world: &World, name: &str| {
        world
            .entities
            .iter()
            .position(|e| {
                e.get("targetname")
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
            .unwrap()
    };
    for (name, model) in [
        ("Barney", "models/barney.mdl"),
        ("kleiner", "models/kleiner.mdl"),
    ] {
        let id = entity(&world, name);
        world.model_instances.push(modkit_core::ModelInstance {
            background: false,
            model: model.into(),
            origin: world.entities[id].origin(),
            angles: Vec3::ZERO,
            skin: 0,
            body: 0,
            scale: 1.,
            kind: world.entities[id].class().into(),
            solid: true,
            solid_mode: None,
            entity: Some(id),
        });
    }
    let mut scene = Scene::new(&world);
    scene.queue.clear();
    scene.load_choreography(&world, &vfs).unwrap();
    // Both are point_template children: pending (killed) until ForceSpawn.
    assert!(scene.template_pending(entity(&world, "Barney")));
    assert!(scene.template_pending(entity(&world, "kleiner")));
    let clips = scene.required_animation_clips(&world);
    let barney = &clips["models/barney.mdl#0"];
    assert!(
        barney.contains("gesture08") && barney.contains("posture01"),
        "{barney:?}"
    );
    let kleiner = &clips["models/kleiner.mdl#0"];
    assert!(kleiner.contains("kposture01"), "{kleiner:?}");
}
#[test]
#[ignore = "requires owned HL2 scene cache; isolated LOOKAT execution, not native AI parity"]
fn owned_security02_refreshes_overlapping_look_targets_and_pause_cancel_lifetime() {
    use crate::attention::Target;
    let game = source_assets::install::discover().unwrap();
    let data = std::fs::read(game.join("hl2/maps/d1_trainstation_01.bsp")).unwrap();
    let world = source_assets::bsp::Bsp::parse(&data)
        .unwrap()
        .world("d1_trainstation_01")
        .unwrap();
    let vfs = Vfs::mount(&game).unwrap();
    let mut scene = Scene::new(&world);
    scene.queue.clear();
    scene.load_choreography(&world, &vfs).unwrap();
    let entity = |name: &str| {
        world
            .entities
            .iter()
            .position(|e| {
                e.get("targetname")
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
            .unwrap()
    };
    let id = entity("security_02");
    let barney = entity("Barney");
    // Barney is a point_template child; the campaign force-spawns him on arrival.
    assert!(scene.template_pending(barney));
    scene.send(entity("gordon_cop_template"), "ForceSpawn", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    assert!(!scene.template_pending(barney) && !scene.states[barney].killed);
    let monitor = entity("mark_secmonitor_look_1");
    scene.send(id, "Start", "");
    for _ in 0..50 {
        scene.tick(&world, Vec3::ZERO, 0.015);
    }
    let first = scene.look_targets.report()[&barney].last().unwrap();
    assert_eq!(first.target, Target::Entity(monitor));
    assert_eq!(first.scene, id);
    let elapsed = scene.choreography[&id].playback.as_ref().unwrap().elapsed;
    let e = &scene.choreography[&id].data.events[first.event];
    assert!(
        (first.importance
            - crate::attention::scene_importance(
                e.intensity(&scene.choreography[&id].data, elapsed as f32),
                elapsed as f32 - e.start
            ))
        .abs()
            < 1e-6
    );
    scene.send(id, "Pause", "");
    // Entity input delivery follows choreography in the retained fixed tick.
    scene.tick(&world, Vec3::ZERO, 0.015);
    let paused_elapsed = scene.choreography[&id].playback.as_ref().unwrap().elapsed;
    assert!((paused_elapsed - elapsed - 0.015).abs() < 1e-6);
    for _ in 0..30 {
        scene.tick(&world, Vec3::ZERO, 0.015);
    }
    assert_eq!(
        scene.choreography[&id].playback.as_ref().unwrap().elapsed,
        paused_elapsed
    );
    assert!(scene.look_targets.report()[&barney].last().unwrap().end > scene.time);
    scene.send(id, "Resume", "");
    for _ in 0..365 {
        scene.tick(&world, Vec3::ZERO, 0.015);
    }
    let targets = &scene.look_targets.report()[&barney];
    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0].target, Target::Entity(monitor));
    assert_eq!(targets[1].target, Target::Player);
    // Explicit cancellation stops refreshing but retains the last 0.1 sec interest.
    scene.send(id, "Cancel", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    assert!(!scene.look_targets.report()[&barney].is_empty());
    for _ in 0..7 {
        scene.tick(&world, Vec3::ZERO, 0.015);
    }
    assert!(!scene.look_targets.report().contains_key(&barney));
    scene.send(id, "Start", "");
    for _ in 0..50 {
        scene.tick(&world, Vec3::ZERO, 0.015);
    }
    scene.states[monitor].killed = true;
    scene.tick(&world, Vec3::ZERO, 0.015);
    assert!(!scene.look_targets.report().contains_key(&barney));
}
#[test]
#[ignore = "requires owned HL2 installation"]
fn owned_security_gestures_layer_barney_and_fade_after_cancel() {
    let game = source_assets::install::discover().unwrap();
    let data = std::fs::read(game.join("hl2/maps/d1_trainstation_01.bsp")).unwrap();
    let world = source_assets::bsp::Bsp::parse(&data)
        .unwrap()
        .world("d1_trainstation_01")
        .unwrap();
    let vfs = Vfs::mount(&game).unwrap();
    let mut scene = Scene::new(&world);
    scene.queue.clear();
    scene.load_choreography(&world, &vfs).unwrap();
    let entity = |name: &str| {
        world
            .entities
            .iter()
            .position(|e| {
                e.get("targetname")
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
            .unwrap()
    };
    let id = entity("security_02");
    let barney = entity("Barney");
    // Barney is a point_template child; the campaign force-spawns him on arrival.
    assert!(scene.template_pending(barney));
    scene.send(entity("gordon_cop_template"), "ForceSpawn", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    assert!(!scene.template_pending(barney) && !scene.states[barney].killed);
    // The parsed test world has no model instances, so request this scene's names directly.
    let wanted = scene.choreography[&id]
        .data
        .events
        .iter()
        .filter(|e| matches!(e.kind, EventType::Gesture | EventType::Sequence))
        .map(|e| e.parameters[0].to_lowercase())
        .filter(|n| !n.is_empty())
        .collect::<BTreeSet<_>>();
    let rig = source_assets::animation::load(&vfs, "models/barney.mdl", &wanted).unwrap();
    assert!(rig.warnings.is_empty(), "{:?}", rig.warnings);
    scene.send(id, "Start", "");
    let (mut seen, mut composed_ticks, mut moved) = (BTreeSet::new(), 0, false);
    let mut flexed = BTreeSet::new();
    for _ in 0..2000 {
        scene.tick(&world, Vec3::ZERO, 0.015);
        if let Some(values) = scene.flex_controllers.get(&barney) {
            assert!(values.values().all(|v| v.is_finite()));
            flexed.extend(
                values
                    .iter()
                    .filter(|(_, v)| v.abs() > 0.05)
                    .map(|(k, _)| k.clone()),
            );
        }
        let layers = scene.gestures.layers(barney);
        for layer in layers {
            assert!((0. ..=1.).contains(&layer.playback), "{layer:?}");
            assert!((0. ..=1.).contains(&layer.weight), "{layer:?}");
            seen.insert(layer.sequence.clone());
        }
        if layers.iter().any(|l| l.removal.is_none() && l.weight > 0.5) && composed_ticks < 40 {
            let base = scene.states[barney].animation.clone();
            let (matrices, errors) =
                scene
                    .gestures
                    .compose(&rig, barney, &base, 1., &rig.default_pose_values());
            assert!(errors.is_empty(), "{errors:?}");
            assert!(matrices.iter().all(|m| m.is_finite()));
            moved |= matrices
                .iter()
                .zip(rig.matrices(&base, 1.))
                .any(|(a, b)| !a.abs_diff_eq(b, 1e-3));
            composed_ticks += 1;
        }
    }
    assert!(
        !seen.is_empty(),
        "security_02 produced no Barney gesture layer"
    );
    // FlexAnimation tracks drive Barney's facial controllers during the scene.
    assert!(!flexed.is_empty(), "no flex controller moved");
    eprintln!("FLEXED {flexed:?}");
    assert!(
        composed_ticks > 0 && moved,
        "gesture layers did not move bones"
    );
    // Restart, let a layer start, then cancel: the 0.5 sec RemoveLayer fade drops it.
    scene.send(id, "Start", "");
    for _ in 0..2000 {
        scene.tick(&world, Vec3::ZERO, 0.015);
        if !scene.gestures.layers(barney).is_empty() {
            break;
        }
    }
    assert!(!scene.gestures.layers(barney).is_empty());
    scene.send(id, "Cancel", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    assert!(scene
        .gestures
        .layers(barney)
        .iter()
        .all(|l| l.removal.is_some()));
    for _ in 0..40 {
        scene.tick(&world, Vec3::ZERO, 0.015);
    }
    assert!(scene.gestures.layers(barney).is_empty());
}
#[test]
fn point_template_children_wait_for_force_spawn_and_spawn_once() {
    let world = World {
        entities: vec![
            entity(
                "point_template",
                "maker",
                &[
                    ("Template01", "actor"),
                    ("OnEntitySpawned", "relay,Trigger,,0,-1"),
                ],
            ),
            entity("npc_citizen", "actor", &[]),
            entity("logic_relay", "relay", &[]),
            entity(
                "point_template",
                "keeper",
                &[("Template01", "kept"), ("spawnflags", "1")],
            ),
            entity("npc_citizen", "kept", &[]),
        ],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    assert!(scene.template_pending(1) && scene.states[1].killed && !scene.states[1].visible);
    // "Don't remove template entities" keeps the authored entity alive.
    assert!(!scene.template_pending(4) && !scene.states[4].killed);
    // Dormant children ignore inputs, like entities that do not exist yet.
    scene.send(1, "Kill", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    let fired = scene.diagnostics.outputs_fired;
    scene.send(0, "ForceSpawn", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    assert!(!scene.template_pending(1) && !scene.states[1].killed && scene.states[1].visible);
    assert!(
        scene.diagnostics.outputs_fired > fired,
        "OnEntitySpawned fires"
    );
    scene.send(0, "ForceSpawn", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    assert!(scene
        .diagnostics
        .unsupported
        .contains_key("point_template.ForceSpawn:repeat"));
}
#[test]
fn trigger_hurt_follows_sdk_think_timing() {
    let cube = modkit_core::BrushModel {
        id: 1,
        brushes: vec![modkit_core::Brush {
            planes: [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z]
                .into_iter()
                .map(|normal| modkit_core::Plane {
                    normal,
                    distance: 8.,
                })
                .collect(),
            contents: 1,
        }],
        mins: Vec3::splat(-8.),
        maxs: Vec3::splat(8.),
        ..Default::default()
    };
    let hurt = |model: &str| World {
        entities: vec![entity(
            "trigger_hurt",
            "pain",
            &[
                ("model", "*1"),
                ("spawnflags", "1"),
                ("damage", "10"),
                ("damagecap", "30"),
                ("damagemodel", model),
                ("damagetype", "0"),
            ],
        )],
        brush_models: vec![cube.clone()],
        ..Default::default()
    };
    let world = hurt("0");
    let mut scene = Scene::new(&world);
    let mut dealt = Vec::new();
    // Inside for one second (67 ticks), then outside.
    for tick in 0..80 {
        let at = if tick < 67 {
            Vec3::ZERO
        } else {
            Vec3::X * 100.
        };
        scene.tick(&world, at, 0.015);
        for amount in scene.player_damage.drain(..).map(|d| d.amount) {
            dealt.push((tick, amount));
        }
    }
    // Immediate hit, then every 0.5 s (about 34 ticks): 5 per think, none on leaving
    // because the last think hurt the player.
    assert_eq!(dealt.len(), 2, "{dealt:?}");
    assert!(dealt.iter().all(|&(_, a)| a == 5.));
    assert!((33..=35).contains(&(dealt[1].0 - dealt[0].0)), "{dealt:?}");
    // Doubling: 5, 10, 15 (capped at 30 x 0.5).
    let world = hurt("1");
    let mut scene = Scene::new(&world);
    let mut amounts = Vec::new();
    for _ in 0..110 {
        scene.tick(&world, Vec3::ZERO, 0.015);
        amounts.extend(scene.player_damage.drain(..).map(|d| d.amount));
    }
    assert_eq!(amounts, [5., 10., 15., 15.]);
}
#[test]
fn trigger_hurt_hurts_npc_touchers_and_respects_filters() {
    let cube = modkit_core::BrushModel {
        id: 1,
        brushes: vec![modkit_core::Brush {
            planes: [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z]
                .into_iter()
                .map(|normal| modkit_core::Plane {
                    normal,
                    distance: 8.,
                })
                .collect(),
            contents: 1,
        }],
        mins: Vec3::splat(-8.),
        maxs: Vec3::splat(8.),
        ..Default::default()
    };
    // NPCs only (spawnflags 2), fixed damage 20 from a trigger at (5 0 0).
    let world = World {
        entities: vec![
            entity(
                "trigger_hurt",
                "pain",
                &[
                    ("model", "*1"),
                    ("spawnflags", "2"),
                    ("origin", "5 0 0"),
                    ("damage", "20"),
                    ("OnHurt", "relay,Trigger,,0,-1"),
                ],
            ),
            entity("npc_metropolice", "cop", &[("origin", "0 0 0")]),
        ],
        brush_models: vec![cube],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    let fired = scene.diagnostics.outputs_fired;
    scene.tick(&world, Vec3::ZERO, 0.015);
    assert!(scene.player_damage.is_empty());
    let hit = scene.entity_damage.pop().expect("NPC hurt");
    assert!(matches!(
        hit.target,
        crate::projectiles::DamageTarget::Entity(1)
    ));
    assert_eq!((hit.amount, hit.origin), (10., Vec3::new(5., 0., 0.)));
    assert!(scene.diagnostics.outputs_fired > fired);
    // A killed NPC stops the think: no hurt, no further damage.
    scene.states[1].killed = true;
    for _ in 0..40 {
        scene.tick(&world, Vec3::ZERO, 0.015);
    }
    assert!(scene.entity_damage.is_empty());
}
#[test]
fn no_touch_changelevel_still_accepts_explicit_input() {
    let world = World {
        entities: vec![entity(
            "trigger_changelevel",
            "exit",
            &[
                ("spawnflags", "2"),
                ("model", "*1"),
                ("map", "next"),
                ("landmark", "arrival"),
            ],
        )],
        brush_models: vec![modkit_core::BrushModel {
            id: 1,
            brushes: vec![modkit_core::Brush {
                planes: [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z]
                    .into_iter()
                    .map(|normal| modkit_core::Plane {
                        normal,
                        distance: 8.,
                    })
                    .collect(),
                contents: 1,
            }],
            mins: Vec3::splat(-8.),
            maxs: Vec3::splat(8.),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    scene.tick(&world, Vec3::ZERO, 0.015);
    assert!(scene.transition.is_none());
    scene.send(0, "ChangeLevel", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    assert_eq!(scene.transition, Some(("next".into(), "arrival".into())));
    let mut touch_world = world.clone();
    touch_world.entities[0]
        .properties
        .retain(|(key, _)| key != "spawnflags");
    let mut touch_scene = Scene::new(&touch_world);
    touch_scene.tick(&touch_world, Vec3::ZERO, 0.015);
    assert_eq!(
        touch_scene.transition,
        Some(("next".into(), "arrival".into()))
    );
}
#[test]
fn metrocops_idle_on_their_weapon_translated_idle_activity() {
    // Synthetic police rig: Idle_Baton (no activity), batonidle1/2 (ACT_IDLE) and
    // smg1idle1 (ACT_IDLE_SMG1); a stunstick cop, an SMG cop, one with DefaultAnim.
    use modkit_core::{
        animation::{Clip, Rig, Sequence},
        ModelInstance,
    };
    let clip = || Clip {
        layer: Default::default(),
        events: Vec::new(),
        fps: 30.,
        looping: true,
        frames: vec![vec![]; 10],
    };
    let seq = Sequence::new;
    let w = World {
        entities: vec![
            entity(
                "npc_metropolice",
                "a",
                &[("additionalequipment", "weapon_stunstick")],
            ),
            entity(
                "npc_metropolice",
                "b",
                &[("additionalequipment", "weapon_smg1")],
            ),
            entity("npc_metropolice", "c", &[("DefaultAnim", "Idle_Baton")]),
        ],
        model_instances: (0..3)
            .map(|id| ModelInstance {
                background: false,
                model: "police.mdl".into(),
                origin: Vec3::ZERO,
                angles: Vec3::ZERO,
                skin: 0,
                body: 0,
                scale: 1.,
                kind: "npc_metropolice".into(),
                solid: false,
                solid_mode: None,
                entity: Some(id),
            })
            .collect(),
        rigs: BTreeMap::from([(
            "police.mdl#0".into(),
            Rig {
                clips: ["idle_baton", "batonidle1", "batonidle2", "smg1idle1"]
                    .into_iter()
                    .map(|n| (n.to_string(), clip()))
                    .collect(),
                sequences: vec![
                    seq("idle_baton", "", 0, 0),
                    seq("batonidle1", "ACT_IDLE", 2, 1),
                    seq("batonidle2", "ACT_IDLE", 1, 2),
                    seq("smg1idle1", "ACT_IDLE_SMG1", 2, 3),
                ],
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let scene = Scene::new(&w);
    assert!(
        ["batonidle1", "batonidle2"].contains(&scene.states[0].animation.as_str()),
        "{}",
        scene.states[0].animation
    );
    assert_eq!(scene.states[1].animation, "smg1idle1");
    assert_eq!(scene.states[2].animation, "idle_baton");
}
#[test]
fn combine_cameras_deploy_retract_and_redeploy() {
    use modkit_core::{
        animation::{Clip, Rig, Sequence},
        ModelInstance,
    };
    let clip = |frames: usize, looping: bool| Clip {
        layer: Default::default(),
        events: Vec::new(),
        fps: 30.,
        looping,
        frames: vec![vec![]; frames],
    };
    let seq = |name: &str, activity: &str, order: u32| Sequence::new(name, activity, 1, order);
    let w = World {
        entities: vec![
            entity("npc_combine_camera", "cam", &[]),
            entity("npc_combine_camera", "sleeper", &[("spawnflags", "128")]),
        ],
        model_instances: (0..2)
            .map(|id| ModelInstance {
                background: false,
                model: "camera.mdl".into(),
                origin: Vec3::ZERO,
                angles: Vec3::ZERO,
                skin: 0,
                body: 0,
                scale: 1.,
                kind: "npc_combine_camera".into(),
                solid: false,
                solid_mode: None,
                entity: Some(id),
            })
            .collect(),
        rigs: BTreeMap::from([(
            "camera.mdl#0".into(),
            Rig {
                clips: BTreeMap::from([
                    ("idle".into(), clip(11, true)),
                    ("idlealert".into(), clip(11, true)),
                    ("ai_retract".into(), clip(48, false)),
                ]),
                sequences: vec![
                    seq("idle", "ACT_COMBINE_CAMERA_CLOSED_IDLE", 0),
                    seq("idlealert", "ACT_COMBINE_CAMERA_OPEN_IDLE", 1),
                    seq("ai_retract", "ACT_COMBINE_CAMERA_CLOSE", 2),
                ],
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let mut scene = Scene::new(&w);
    // Spawn: deployed unless StartInactive.
    assert_eq!(scene.states[0].animation, "idlealert");
    assert_eq!(scene.states[1].animation, "idle");
    assert!(!scene.states[1].enabled);
    // Toggle off plays the close transition, then rests in closed idle.
    scene.send(0, "Toggle", "");
    scene.tick(&w, Vec3::ZERO, 0.015);
    assert_eq!(scene.states[0].animation, "ai_retract");
    for _ in 0..120 {
        scene.tick(&w, Vec3::ZERO, 0.015);
    }
    assert_eq!(scene.states[0].animation, "idle");
    // Enable redeploys; enabling an enabled camera changes nothing.
    scene.send(0, "Enable", "");
    scene.send(1, "Enable", "");
    scene.tick(&w, Vec3::ZERO, 0.015);
    assert_eq!(
        (
            scene.states[0].animation.as_str(),
            scene.states[1].animation.as_str()
        ),
        ("idlealert", "idlealert")
    );
    assert!(
        scene.diagnostics.unsupported.is_empty(),
        "{:?}",
        scene.diagnostics.unsupported
    );
}
#[test]
fn head_and_body_flexes_offset_actor_pose_parameters() {
    use modkit_core::animation::{PoseParameter, Rig};
    let w = World {
        entities: vec![entity("npc_barney", "barney", &[])],
        ..Default::default()
    };
    let param = |name: &str, start: f32, end: f32| PoseParameter {
        name: name.into(),
        start,
        end,
        looping: 0.,
    };
    let rig = Rig {
        pose_parameters: vec![
            param("head_pitch", -30., 30.),
            param("head_yaw", -60., 60.),
            param("body_yaw", -30., 30.),
            param("gesture_height", -1., 1.),
        ],
        ..Default::default()
    };
    let mut scene = Scene::new(&w);
    let rest = scene.actor_pose_values(&rig, 0);
    let rest_signature = scene.pose_signature(0);
    // UpdateHeadControl adds head flexes to the look correction; UpdateBodyControl
    // drives body_yaw from body_rightleft. Values are in controller degrees.
    scene.flex_controllers.insert(
        0,
        BTreeMap::from([
            ("head_updown".into(), 15.),
            ("body_rightleft".into(), -15.),
            ("smile".into(), 1.),
        ]),
    );
    let p = scene.actor_pose_values(&rig, 0);
    assert!(
        (p[0] - 0.75).abs() < 1e-6 && (p[2] - 0.25).abs() < 1e-6,
        "{p:?}"
    );
    assert_eq!((p[1], p[3]), (rest[1], rest[3]));
    assert_ne!(scene.pose_signature(0), rest_signature);
    scene.heads.insert(
        0,
        crate::attention::Head {
            goal: Vec3::new(10., 30., 0.),
            ..Default::default()
        },
    );
    let p = scene.actor_pose_values(&rig, 0);
    assert!((p[0] - (25. + 30.) / 60.).abs() < 1e-6 && (p[1] - 0.75).abs() < 1e-6);
}
#[test]
fn props_parent_to_entities_and_follow_animated_attachments() {
    use modkit_core::{
        animation::{Attachment, Bone, Pose, Rig},
        ModelInstance,
    };
    let w = World {
        entities: vec![
            entity("npc_barney", "barney", &[("origin", "100 0 0")]),
            entity(
                "prop_dynamic",
                "helmet",
                &[("origin", "100 0 50"), ("parentname", "barney")],
            ),
            entity(
                "prop_dynamic",
                "plate",
                &[("origin", "0 0 0"), ("StartDisabled", "1")],
            ),
        ],
        model_instances: vec![ModelInstance {
            background: false,
            model: "barney.mdl".into(),
            origin: Vec3::new(100., 0., 0.),
            angles: Vec3::ZERO,
            skin: 0,
            body: 0,
            scale: 1.,
            kind: "npc_barney".into(),
            solid: false,
            solid_mode: None,
            entity: Some(0),
        }],
        rigs: BTreeMap::from([(
            "barney.mdl#0".into(),
            Rig {
                bones: vec![Bone {
                    name: "root".into(),
                    parent: None,
                    bind: Pose {
                        position: Vec3::ZERO,
                        rotation: Quat::IDENTITY,
                    },
                    inverse_bind: Mat4::IDENTITY,
                }],
                attachments: vec![Attachment {
                    name: "helmet_attachment".into(),
                    bone: 0,
                    local: Mat4::from_translation(Vec3::new(2., 0., 64.)),
                }],
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let mut scene = Scene::new(&w);
    // Map-spawn parentname keeps the authored world offset; StartDisabled hides props.
    assert_eq!(scene.states[1].parent.as_ref().map(|p| p.entity), Some(0));
    assert!(!scene.states[2].visible);
    scene.states[0].origin = Vec3::new(110., 0., 0.);
    scene.tick(&w, Vec3::ZERO, 0.015);
    assert!(scene.states[1]
        .origin
        .abs_diff_eq(Vec3::new(110., 0., 50.), 1e-4));
    // SetParent then SetParentAttachment snaps onto the attachment (logic_barney_init).
    scene.send(2, "SetParent", "barney");
    scene.send(2, "SetParentAttachment", "helmet_attachment");
    scene.tick(&w, Vec3::ZERO, 0.015);
    let parent = scene.states[2].parent.clone().unwrap();
    assert_eq!(parent.attachment.as_deref(), Some("helmet_attachment"));
    assert!(scene.states[2]
        .origin
        .abs_diff_eq(Vec3::new(112., 0., 64.), 1e-4));
    // The child follows the parent's rotation and position.
    scene.states[0].rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    scene.tick(&w, Vec3::ZERO, 0.015);
    assert!(scene.states[2]
        .origin
        .abs_diff_eq(Vec3::new(110., 2., 64.), 1e-4));
    assert!(scene.states[2]
        .rotation
        .abs_diff_eq(scene.states[0].rotation, 1e-5));
    // TurnOn/TurnOff toggle drawing; ClearParent keeps the last world pose.
    scene.send(2, "TurnOn", "");
    scene.send(2, "ClearParent", "");
    scene.tick(&w, Vec3::ZERO, 0.015);
    assert!(scene.states[2].visible && scene.states[2].parent.is_none());
    scene.states[0].origin = Vec3::ZERO;
    scene.tick(&w, Vec3::ZERO, 0.015);
    assert!(scene.states[2]
        .origin
        .abs_diff_eq(Vec3::new(110., 2., 64.), 1e-4));
    // An unknown attachment is reported and leaves the parent unchanged.
    scene.send(1, "SetParentAttachment", "missing");
    scene.tick(&w, Vec3::ZERO, 0.015);
    assert_eq!(scene.states[1].parent.as_ref().unwrap().attachment, None);
    assert!(scene
        .diagnostics
        .unsupported
        .keys()
        .any(|k| k.contains("missing-attachment")));
}
#[test]
fn setting_default_animation_preserves_playback_and_completion() {
    use modkit_core::{
        animation::{Clip, Rig},
        ModelInstance,
    };
    let w = World {
        entities: vec![
            entity(
                "prop_dynamic",
                "prop",
                &[
                    ("DefaultAnim", "closed"),
                    ("OnAnimationDone", "result,Add,1,0,-1"),
                ],
            ),
            entity("math_counter", "result", &[]),
        ],
        model_instances: vec![ModelInstance {
            background: false,
            model: "prop.mdl".into(),
            origin: Vec3::ZERO,
            angles: Vec3::ZERO,
            skin: 0,
            body: 0,
            scale: 1.,
            kind: "prop_dynamic".into(),
            solid: false,
            solid_mode: None,
            entity: Some(0),
        }],
        rigs: BTreeMap::from([(
            "prop.mdl#0".into(),
            Rig {
                clips: BTreeMap::from([(
                    "opening".into(),
                    Clip {
                        layer: Default::default(),
                        events: Vec::new(),
                        fps: 30.,
                        looping: false,
                        frames: vec![vec![]; 31],
                    },
                )]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let mut scene = Scene::new(&w);
    scene.send(0, "SetAnimation", "opening");
    scene.tick(&w, Vec3::ZERO, 0.015);
    let started = scene.states[0].animation_started;
    let due = scene.states[0].animation_done;
    scene.tick(&w, Vec3::ZERO, 0.2);
    // Native accepts a deferred name even when no such sequence is loaded.
    scene.send(0, "SetDefaultAnimation", "DeferredCase");
    scene.tick(&w, Vec3::ZERO, 0.015);
    assert_eq!(scene.states[0].default_animation, "DeferredCase");
    assert_eq!(scene.states[0].animation, "opening");
    assert_eq!(scene.states[0].animation_started, started);
    assert_eq!(scene.states[0].animation_done, due);
    assert!(scene.diagnostics.unsupported.is_empty());
    scene.send(0, "SetDefaultAnimation", "");
    scene.tick(&w, Vec3::ZERO, 0.015);
    assert!(scene.states[0].default_animation.is_empty());
    assert_eq!(scene.states[0].animation_done, due);
    scene.tick(&w, Vec3::ZERO, 0.9);
    assert_eq!(scene.states[1].value, 1.);
    scene.tick(&w, Vec3::ZERO, 0.9);
    assert_eq!(scene.states[1].value, 1.);
}
#[test]
fn shuffle_exhausts_cases_and_avoids_repeating_across_batches() {
    let w = World {
        entities: vec![
            entity(
                "logic_case",
                "choose",
                &[
                    ("OnCase01", "result,SetValue,1,0,-1"),
                    ("OnCase02", "result,SetValue,2,0,-1"),
                    ("OnCase03", "result,SetValue,3,0,-1"),
                ],
            ),
            entity("math_counter", "result", &[]),
        ],
        ..Default::default()
    };
    let mut s = Scene::new(&w);
    let mut chosen = Vec::new();
    for _ in 0..30 {
        s.send(0, "PickRandomShuffle", "");
        s.tick(&w, Vec3::ZERO, 0.015);
        chosen.push(s.states[1].value as i32);
    }
    for batch in chosen.as_chunks::<3>().0 {
        assert_eq!(
            batch.iter().copied().collect::<BTreeSet<_>>(),
            BTreeSet::from([1, 2, 3])
        );
    }
    for boundary in (3..chosen.len()).step_by(3) {
        assert_ne!(chosen[boundary - 1], chosen[boundary]);
    }
}
#[test]
fn scripted_post_idle_output_waits_for_its_clip() {
    use modkit_core::{
        animation::{Clip, Rig},
        ModelInstance,
    };
    let mut w = World {
        entities: vec![
            entity(
                "scripted_sequence",
                "script",
                &[
                    ("m_iszEntity", "actor"),
                    ("m_iszPlay", "play"),
                    ("m_iszPostIdle", "post"),
                    ("OnEndSequence", "result,SetValue,1,0,-1"),
                    ("OnPostIdleEndSequence", "result,SetValue,2,0,-1"),
                ],
            ),
            entity("npc_citizen", "actor", &[]),
            entity("math_counter", "result", &[]),
        ],
        model_instances: vec![ModelInstance {
            background: false,
            model: "actor.mdl".into(),
            origin: Vec3::ZERO,
            angles: Vec3::ZERO,
            skin: 0,
            body: 0,
            scale: 1.,
            kind: "npc_citizen".into(),
            solid: true,
            solid_mode: None,
            entity: Some(1),
        }],
        rigs: BTreeMap::from([(
            "actor.mdl#0".into(),
            Rig {
                clips: BTreeMap::from([
                    (
                        "play".into(),
                        Clip {
                            layer: Default::default(),
                            events: Vec::new(),
                            fps: 30.,
                            looping: false,
                            frames: vec![vec![]; 31],
                        },
                    ),
                    (
                        "post".into(),
                        Clip {
                            layer: Default::default(),
                            events: Vec::new(),
                            fps: 30.,
                            looping: false,
                            frames: vec![vec![]; 61],
                        },
                    ),
                ]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let mut s = Scene::new(&w);
    s.send(0, "BeginSequence", "");
    s.tick(&w, Vec3::ZERO, 0.015);
    s.tick(&w, Vec3::ZERO, 1.1);
    assert_eq!(s.states[2].value, 1.);
    s.tick(&w, Vec3::ZERO, 1.5);
    assert_eq!(s.states[2].value, 1.);
    s.tick(&w, Vec3::ZERO, 0.6);
    assert_eq!(s.states[2].value, 2.);
    // Both an omitted action and an explicitly empty one must enter post idle immediately.
    for omitted in [false, true] {
        w.entities[0].properties.retain(|(k, _)| k != "m_iszPlay");
        if !omitted {
            w.entities[0]
                .properties
                .push(("m_iszPlay".into(), String::new()));
        }
        let mut s = Scene::new(&w);
        s.send(0, "BeginSequence", "");
        s.tick(&w, Vec3::ZERO, 0.015);
        assert_eq!(s.states[1].animation, "post");
        assert_eq!(s.states[2].value, 1.);
        assert!(!s
            .diagnostics
            .unsupported
            .keys()
            .any(|k| k.contains("animation:")));
        s.tick(&w, Vec3::ZERO, 1.5);
        assert_eq!(s.states[2].value, 1.);
        s.tick(&w, Vec3::ZERO, 0.6);
        assert_eq!(s.states[2].value, 2.);
    }
}
#[test]
fn implicit_idle_and_scripted_activity_use_real_clip_and_deadlines() {
    use modkit_core::{
        animation::{Clip, Rig, Sequence},
        ModelInstance,
    };
    let mut world = World {
        entities: vec![
            entity(
                "scripted_sequence",
                "script",
                &[
                    ("m_iszEntity", "actor"),
                    ("m_iszPlay", "ACT_IDLE"),
                    ("m_iszPostIdle", "ACT_IDLE"),
                ],
            ),
            entity("npc_kleiner", "actor", &[]),
        ],
        model_instances: vec![ModelInstance {
            model: "actor.mdl".into(),
            origin: Vec3::ZERO,
            angles: Vec3::ZERO,
            skin: 0,
            body: 0,
            scale: 1.,
            kind: "npc_kleiner".into(),
            background: false,
            solid: false,
            solid_mode: None,
            entity: Some(1),
        }],
        rigs: BTreeMap::from([(
            "actor.mdl#0".into(),
            Rig {
                clips: BTreeMap::from([(
                    "model_idle".into(),
                    Clip {
                        layer: Default::default(),
                        events: vec![],
                        fps: 2.,
                        looping: true,
                        frames: vec![vec![]; 5],
                    },
                )]),
                sequences: vec![Sequence::new("model_idle", "ACT_IDLE", 1, 0)],
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    assert_eq!(scene.states[1].animation, "model_idle");
    scene.send(0, "BeginSequence", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    assert_eq!(scene.states[1].animation, "model_idle");
    let finish = scene.states[0].script_finish.unwrap();
    assert!((finish - scene.time - 2.).abs() < 0.0001);
    scene.tick(&world, Vec3::ZERO, 2.1);
    assert_eq!(scene.states[1].animation, "model_idle");
    assert!(scene.states[1].scripted_by.is_none());
    let post = scene.states[0].post_idle_done.unwrap().0;
    assert!((post - scene.time - 2.).abs() < 0.0001);
    // SCRIPT_POST_IDLE keeps the actor in NPC_STATE_SCRIPT until PostIdleDone.
    assert_eq!(scene.states[1].script_post_idle, Some(0));
    scene.tick(&world, Vec3::ZERO, 2.1);
    assert!(scene.states[1].script_post_idle.is_none());
    // SF_SCRIPT_LOOP_IN_POST_IDLE (256) holds it until the script is cancelled.
    let mut looping = world.clone();
    looping.entities[0]
        .properties
        .push(("spawnflags".into(), "256".into()));
    let mut scene = Scene::new(&looping);
    scene.send(0, "BeginSequence", "");
    for _ in 0..4 {
        scene.tick(&looping, Vec3::ZERO, 2.1);
    }
    assert_eq!(scene.states[1].script_post_idle, Some(0));
    scene.send(0, "CancelSequence", "");
    scene.tick(&looping, Vec3::ZERO, 0.015);
    assert!(scene.states[1].script_post_idle.is_none());
    world.entities[1]
        .properties
        .push(("DefaultAnim".into(), "authored_missing".into()));
    let mut scene = Scene::new(&world);
    assert_eq!(scene.states[1].animation, "authored_missing");
    // Explicit missing label does not silently fall back to an activity.
    assert!(!scene.animate(&world, 1, "authored_missing", false));
    assert_eq!(scene.states[1].animation, "authored_missing");
}
#[test]
fn duplicate_outputs_delays_and_fire_limits() {
    let w = World {
        entities: vec![
            entity(
                "logic_relay",
                "start",
                &[
                    ("OnTrigger", "count,Add,2,0.1,1"),
                    ("OnTrigger", "count,Add,3,0.1,-1"),
                ],
            ),
            entity("math_counter", "count", &[]),
        ],
        ..Default::default()
    };
    let mut s = Scene::new(&w);
    s.send(0, "Trigger", "");
    s.tick(&w, Vec3::ZERO, 0.015);
    assert_eq!(s.states[1].value, 0.);
    s.tick(&w, Vec3::ZERO, 0.1);
    assert_eq!(s.states[1].value, 5.);
    s.send(0, "Trigger", "");
    s.tick(&w, Vec3::ZERO, 0.015);
    s.tick(&w, Vec3::ZERO, 0.1);
    assert_eq!(s.states[1].value, 8.);
}
#[test]
fn rotating_door_lock_and_completion() {
    let w = World {
        entities: vec![entity(
            "prop_door_rotating",
            "door",
            &[("locked", "1"), ("speed", "90")],
        )],
        ..Default::default()
    };
    let mut s = Scene::new(&w);
    s.send(0, "Open", "");
    s.tick(&w, Vec3::ZERO, 0.015);
    assert_eq!(s.states[0].target, 0.);
    s.send(0, "Unlock", "");
    s.send(0, "Open", "");
    for _ in 0..70 {
        s.tick(&w, Vec3::ZERO, 0.015);
    }
    assert!((s.states[0].rotation * Vec3::X + Vec3::Y).length() < 0.001);
    assert_eq!(s.diagnostics.door_completions, 1);
}
#[test]
fn escaped_output_parameters_can_contain_commas() {
    let o = output("OnTrigger", "x\x1bSetValue\x1ba,b\x1b0\x1b-1").unwrap();
    assert_eq!(o.parameter, "a,b");
}
#[test]
fn paired_prop_doors_use_opener_side_and_return_to_authored_closed_pose() {
    let w = World {
        entities: vec![
            entity(
                "prop_door_rotating",
                "pair",
                &[
                    ("origin", "0 -47 0"),
                    ("angles", "0 0 0"),
                    ("spawnflags", "8192"),
                    ("speed", "90"),
                ],
            ),
            entity(
                "prop_door_rotating",
                "pair",
                &[
                    ("origin", "0 47 0"),
                    ("angles", "0 180 0"),
                    ("spawnflags", "8192"),
                    ("speed", "90"),
                ],
            ),
        ],
        ..Default::default()
    };
    let mut s = Scene::new(&w);
    for side in [-1., 1.] {
        s.use_entity_at(&w, 0, Vec3::X * side * 64.);
        for _ in 0..70 {
            s.tick(&w, Vec3::ZERO, 0.015);
        }
        for state in &s.states {
            let panel = state.origin + state.rotation * Vec3::Y * 47.;
            assert!(
                panel.x * side < -46.,
                "both panels must swing away from opener"
            );
        }
        s.use_entity_at(&w, 1, Vec3::X * side * 64.);
        for _ in 0..70 {
            s.tick(&w, Vec3::ZERO, 0.015);
        }
        for state in &s.states {
            assert_eq!(state.fraction, 0.);
            assert!(state.rotation.dot(state.base_rotation).abs() > 0.99999);
        }
    }
    assert_eq!(s.diagnostics.door_completions, 8);
}
#[test]
fn explicit_prop_door_direction_and_lock_override_opener_side() {
    for (direction, sign) in [(1, -1.), (2, 1.)] {
        let w = World {
            entities: vec![entity(
                "prop_door_rotating",
                "door",
                &[
                    ("opendir", if direction == 1 { "1" } else { "2" }),
                    ("locked", "1"),
                ],
            )],
            ..Default::default()
        };
        let mut s = Scene::new(&w);
        s.use_entity_at(&w, 0, Vec3::X * 64.);
        s.tick(&w, Vec3::ZERO, 0.015);
        assert_eq!(s.states[0].target, 0.);
        s.send(0, "Unlock", "");
        s.tick(&w, Vec3::ZERO, 0.015);
        s.use_entity_at(&w, 0, Vec3::X * 64.);
        s.tick(&w, Vec3::ZERO, 0.015);
        assert_eq!(s.states[0].target, 1.);
        assert_eq!(s.states[0].angle.signum(), sign);
    }
}
#[test]
fn open_away_input_resolves_current_named_player_and_caller_origins() {
    let w = World {
        entities: vec![
            entity("prop_door_rotating", "door", &[("wait", "-1")]),
            entity("info_target", "actor", &[("origin", "-64 0 0")]),
            entity(
                "logic_relay",
                "relay",
                &[
                    ("origin", "64 0 0"),
                    ("OnTrigger", "door,OpenAwayFrom,!caller,0,-1"),
                ],
            ),
        ],
        ..Default::default()
    };
    for (name, expected) in [
        ("actor", 1.),
        ("!player", 1.),
        ("!activator", 1.),
        ("missing", -1.),
    ] {
        let mut s = Scene::new(&w);
        // The input uses the current state, not the map's initial actor origin.
        s.states[1].origin = Vec3::X * 64.;
        s.send(0, "OpenAwayFrom", name);
        s.tick(&w, Vec3::X * 64., 0.015);
        assert_eq!(s.states[0].target, 1.);
        assert_eq!(s.states[0].angle.signum(), expected, "{name}");
        assert_eq!(
            s.diagnostics
                .unsupported
                .get("prop_door_rotating.openawayfrom"),
            None
        );
    }
    let mut s = Scene::new(&w);
    s.send(0, "OpenAwayFrom", "!player");
    s.send(0, "OpenAwayFrom", "missing");
    s.tick(&w, Vec3::X * 64., 0.015);
    assert_eq!(
        s.states[0].angle.signum(),
        1.,
        "second open input must not retarget the opening door"
    );
    let mut s = Scene::new(&w);
    s.send(2, "Trigger", "");
    s.tick(&w, -Vec3::X * 64., 0.015);
    assert_eq!(s.states[0].angle.signum(), 1., "caller relay, not player");
}
#[test]
fn open_away_input_obeys_lock_fixed_direction_and_keeps_opening_swing() {
    let w = World {
        entities: vec![entity(
            "prop_door_rotating",
            "door",
            &[("locked", "1"), ("opendir", "1"), ("wait", "-1")],
        )],
        ..Default::default()
    };
    let mut s = Scene::new(&w);
    s.send(0, "OpenAwayFrom", "!player");
    s.tick(&w, Vec3::X * 64., 0.015);
    assert_eq!(s.states[0].target, 0.);
    s.send(0, "Unlock", "");
    s.send(0, "OpenAwayFrom", "!player");
    s.tick(&w, Vec3::X * 64., 0.015);
    assert_eq!(s.states[0].target, 1.);
    assert_eq!(s.states[0].angle.signum(), -1.);
    s.send(0, "OpenAwayFrom", "!player");
    s.tick(&w, -Vec3::X * 64., 0.015);
    assert_eq!(s.states[0].angle.signum(), -1.);
}

#[test]
fn add_output_keyvalue_form_overrides_physdamagescale() {
    let world = World {
        entities: vec![modkit_core::Entity {
            properties: vec![
                ("classname".into(), "npc_metropolice".into()),
                ("physdamagescale".into(), "0".into()),
            ],
        }],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    scene.send(0, "AddOutput", "physdamagescale 1");
    scene.send(0, "AddOutput", "OnDeath relay:Trigger::0:-1");
    scene.tick(&world, Vec3::ZERO, 0.015);
    assert_eq!(
        scene
            .keyvalue_overrides
            .get(&(0, "physdamagescale".to_string()))
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(scene.keyvalue_overrides.len(), 1);
}
#[test]
fn a_ragdolled_actor_draws_its_physics_pose() {
    use modkit_core::animation::Rig;
    let w = World {
        entities: vec![entity("npc_metropolice", "cop", &[])],
        ..Default::default()
    };
    let rig = Rig::default();
    let mut scene = Scene::new(&w);
    let animated = scene.pose_signature(0);
    // ragdolls.rs publishes the physics skin; skinning and pose caching must use it.
    let pose = vec![Mat4::from_translation(Vec3::new(1., 2., 3.))];
    scene.ragdoll_matrices.insert(0, pose.clone());
    assert_eq!(scene.actor_matrices(&rig, 0), pose);
    let first = scene.pose_signature(0);
    assert_ne!(first, animated);
    scene.ragdoll_frame += 1;
    assert_ne!(scene.pose_signature(0), first);
}
