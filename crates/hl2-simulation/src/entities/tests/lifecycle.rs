use super::*;
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
