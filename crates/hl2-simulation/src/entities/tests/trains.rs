//! func_tracktrain on path_track, and point_teleport.
use super::*;

fn line_world() -> World {
    World {
        entities: vec![
            entity("path_track", "a", &[("origin", "0 0 0"), ("target", "b")]),
            entity(
                "path_track",
                "b",
                &[
                    ("origin", "1000 0 0"),
                    ("target", "c"),
                    ("OnPass", "marker,Kill,,0,-1"),
                ],
            ),
            entity("path_track", "c", &[("origin", "1000 1000 0")]),
            entity("info_target", "marker", &[]),
            entity(
                "func_tracktrain",
                "train",
                &[
                    ("origin", "500 500 0"),
                    ("target", "a"),
                    ("startspeed", "100"),
                    ("wheels", "50"),
                    ("spawnflags", "2"),
                    ("MoveSound", "Trainyard.train_move"),
                    ("StopSound", "Trainyard.train_stop"),
                ],
            ),
            entity(
                "point_teleport",
                "tp",
                &[
                    ("origin", "5 6 7"),
                    ("angles", "0 15 0"),
                    ("target", "!player"),
                ],
            ),
        ],
        ..Default::default()
    }
}

#[test]
fn a_train_starts_on_its_first_path_track_and_follows_the_path_at_its_speed() {
    let world = line_world();
    let mut scene = Scene::new(&world);
    scene.tick(&world, Vec3::ZERO, 0.015);
    // Find: the train goes to its first node and faces along the path.
    assert!(
        scene.states[4].origin.distance(Vec3::ZERO) < 0.01,
        "{}",
        scene.states[4].origin
    );
    assert!(scene.trains.pushes.is_empty());
    // SetSpeed is a fraction of startspeed: 100 units a second along +X.
    scene.send(4, "SetSpeed", "1");
    for _ in 0..100 {
        scene.tick(&world, Vec3::ZERO, 0.015);
    }
    let x = scene.states[4].origin.x;
    assert!((140. ..=160.).contains(&x), "{x}");
    let push = scene.trains.pushes[0];
    assert_eq!((push.entity, push.time), (4, scene.time));
    assert!(!scene.states[3].killed, "b is not passed yet");
    // Reaching b passes it (OnPass) and the train turns toward c (+Y).
    for _ in 0..700 {
        scene.tick(&world, Vec3::ZERO, 0.015);
    }
    assert!(scene.states[3].killed, "OnPass of b fired");
    let forward = scene.states[4].rotation * Vec3::X;
    assert!(forward.y > 0.9, "{forward}");
    assert!(scene.states[4].origin.y > 10., "{}", scene.states[4].origin);
    // The end of the path stops it at the last node.
    for _ in 0..1000 {
        scene.tick(&world, Vec3::ZERO, 0.015);
    }
    assert!(
        scene.states[4].origin.distance(Vec3::new(1000., 1000., 0.)) < 1.,
        "{}",
        scene.states[4].origin
    );
    assert_eq!(scene.trains.trains[&4].speed, 0.);
    // SoundUpdate / SoundStop: the move loop at pitch 60 + 140 x 100 / 1000, then the stop
    // sound and the loop destroyed at the dead end.
    use crate::sounds::PatchOp;
    let patches: Vec<_> = scene.sounds.iter().filter_map(|s| s.patch).collect();
    assert_eq!(
        patches.first(),
        Some(&PatchOp::Play {
            volume: 1.,
            pitch: 74.
        })
    );
    assert!(patches.contains(&PatchOp::Destroy));
    assert!(scene
        .sounds
        .iter()
        .any(|s| s.name == "Trainyard.train_stop"));
    // Stop and a rider: a point on the train moves and turns with it.
    let push = crate::trains::Push {
        entity: 4,
        time: 0.,
        from: (Vec3::ZERO, Quat::IDENTITY),
        to: (
            Vec3::new(10., 0., 0.),
            Quat::from_rotation_z(90f32.to_radians()),
        ),
    };
    let (point, yaw) = push.carry(Vec3::new(1., 0., 0.));
    assert!(point.distance(Vec3::new(10., 1., 0.)) < 1e-4, "{point}");
    assert!((yaw - 90.).abs() < 1e-3, "{yaw}");
}

#[test]
fn point_teleport_moves_the_player_to_its_origin_and_angles() {
    let world = line_world();
    let mut scene = Scene::new(&world);
    scene.send(5, "Teleport", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    let teleport = scene.trains.teleport.expect("player teleport");
    assert_eq!(teleport.feet, Vec3::new(5., 6., 7.));
    assert_eq!(teleport.angles, Vec3::new(0., 15., 0.));
}

#[test]
fn env_message_shows_its_titles_message_once() {
    // d1_trainstation_01's text_chapter (spawnflags 1: SF_MESSAGE_ONCE).
    let world = World {
        entities: vec![entity(
            "env_message",
            "text_chapter",
            &[("message", "CHAPTER1_TITLE"), ("spawnflags", "1")],
        )],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    scene.send(0, "ShowMessage", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    assert_eq!(scene.intro.messages.len(), 1);
    assert_eq!(
        scene.intro.messages[0].0,
        crate::script_intro::HudText::Title("CHAPTER1_TITLE".into())
    );
    assert!(scene.states[0].killed);
}

#[test]
fn game_text_replaces_the_message_on_its_channel() {
    // d1_trainstation_01's turret_hurt_1_text keys.
    let keys = [
        ("message", "turrets killed you"),
        ("x", "-1"),
        ("y", "-1"),
        ("effect", "0"),
        ("color", "100 100 100"),
        ("color2", "240 110 0"),
        ("fadein", "1.5"),
        ("fadeout", "0.5"),
        ("holdtime", "1.2"),
        ("fxtime", "0.25"),
        ("channel", "1"),
    ];
    let world = World {
        entities: vec![
            entity("game_text", "a", &keys),
            entity("game_text", "b", &keys),
        ],
        ..Default::default()
    };
    let mut scene = Scene::new(&world);
    scene.send(0, "Display", "");
    scene.send(1, "Display", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    assert_eq!(scene.intro.messages.len(), 1, "one per channel");
    let crate::script_intro::HudText::Text { message, channel } = &scene.intro.messages[0].0 else {
        panic!("game_text");
    };
    assert_eq!((*channel, message.text.as_str()), (1, "turrets killed you"));
    assert_eq!(
        (message.color, message.holdtime),
        ([100, 100, 100, 255], 1.2)
    );
}
