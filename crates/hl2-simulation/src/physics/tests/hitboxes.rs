//! NPC studio hitboxes: trace colliders outside the broad phase.
use super::super::*;
use super::*;

/// A synthetic citizen (hull box) whose one hitbox, group 3, hangs off its root bone at
/// x 40..48, z 56..64: beyond the hull, so only the hitbox reaches a body there.
fn hitbox_world() -> World {
    let instance = modkit_core::ModelInstance {
        background: false,
        model: "synthetic_hitbox_citizen.mdl".into(),
        origin: Vec3::ZERO,
        angles: Vec3::ZERO,
        skin: 0,
        body: 0,
        scale: 1.,
        kind: "npc_citizen".into(),
        solid: true,
        solid_mode: Some(2),
        entity: Some(0),
    };
    let key = instance.asset_key();
    let rig = modkit_core::animation::Rig {
        bones: vec![modkit_core::animation::Bone {
            name: "root".into(),
            parent: None,
            bind: modkit_core::animation::Pose {
                position: Vec3::ZERO,
                rotation: Quat::IDENTITY,
            },
            inverse_bind: Mat4::IDENTITY,
        }],
        hitboxes: vec![modkit_core::animation::Hitbox {
            bone: 0,
            group: 3,
            mins: Vec3::new(40., -4., 56.),
            maxs: Vec3::new(48., 4., 64.),
        }],
        ..Default::default()
    };
    World {
        entities: vec![modkit_core::Entity {
            properties: vec![("classname".into(), "npc_citizen".into())],
        }],
        model_assets: BTreeMap::from([(
            key.clone(),
            vec![box_surface(Vec3::new(-4., -4., 0.), Vec3::new(4., 4., 60.))],
        )]),
        rigs: BTreeMap::from([(key, rig)]),
        model_instances: vec![instance],
        ..Default::default()
    }
}

/// The height where a shot straight down at x 47 meets the NPC, with its hitgroup and bone.
fn shot_down(physics: &Physics) -> Option<(f32, i32, Option<usize>)> {
    let hit = physics.impact_ray(Vec3::new(47., 0., 100.), -Vec3::Z, 200.)?;
    assert_eq!(hit.entity, 0);
    Some((hit.position.z, hit.hitgroup, hit.bone))
}

#[test]
fn traces_hit_posed_hitboxes_that_never_touch_or_wake_a_body() {
    let mut physics = Physics::new(&hitbox_world());
    physics.refresh_queries();
    let pose = |physics: &mut Physics, lift: f32, enabled: bool| {
        let skin = [Mat4::from_translation(Vec3::Z * lift)];
        physics.set_npc_hitboxes(0, Mat4::IDENTITY, &skin, enabled);
        physics.refresh_entity_queries();
    };
    let near = |found: Option<(f32, i32, Option<usize>)>, top: f32| {
        found
            .is_some_and(|(z, group, bone)| (z - top).abs() < 0.01 && group == 3 && bone == Some(0))
    };
    assert!(near(shot_down(&physics), 64.));
    // A box resting inside the hitbox's reach (no gravity) falls asleep untouched.
    let body = physics.bodies.insert(
        RigidBodyBuilder::dynamic()
            .translation(vector![44. * SCALE, 0., 60. * SCALE])
            .gravity_scale(0.),
    );
    physics.colliders.insert_with_parent(
        ColliderBuilder::cuboid(2. * SCALE, 2. * SCALE, 2. * SCALE),
        body,
        &mut physics.bodies,
    );
    for _ in 0..200 {
        physics.tick(TICK);
    }
    assert!(physics.bodies[body].is_sleeping());
    // The animated hitbox moves through it every tick: traces follow the pose right after
    // the refresh and after each step, and the box sleeps on, in no contact pair.
    for tick in 0..20 {
        let lift = (tick % 2) as f32 * 2.;
        pose(&mut physics, lift, true);
        assert!(near(shot_down(&physics), 64. + lift));
        physics.tick(TICK);
        assert!(near(shot_down(&physics), 64. + lift));
    }
    assert!(physics.bodies[body].is_sleeping());
    let moved = physics.bodies[body].translation() - vector![44. * SCALE, 0., 60. * SCALE];
    assert!(moved.norm() < 1e-6, "{moved}");
    assert!(physics.narrow.contact_pairs().all(|pair| {
        !physics.is_trace_only(pair.collider1) && !physics.is_trace_only(pair.collider2)
    }));
    // A full rebuild keeps them; a killed NPC's hitboxes take no shot until enabled again,
    // at the pose of that tick.
    physics.refresh_queries();
    assert!(near(shot_down(&physics), 66.));
    pose(&mut physics, 0., false);
    physics.tick(TICK);
    assert_eq!(shot_down(&physics), None);
    pose(&mut physics, 4., true);
    assert!(near(shot_down(&physics), 68.));
    physics.tick(TICK);
    assert!(near(shot_down(&physics), 68.));
}

#[test]
fn runtime_npc_publishes_its_hull_and_reused_studio_hitbox_slot() {
    let mut world = hitbox_world();
    let mut physics = Physics::new(&world);
    physics.add_entity_hull(99, Vec3::splat(-4.), Vec3::splat(4.), Vec3::Z * 100.);
    let queued = physics.changed_entity_colliders.clone();
    let before: Vec<_> = physics.colliders.iter().map(|(handle, _)| handle).collect();
    let mut instance = world.model_instances[0].clone();
    instance.origin = Vec3::X * 100.;
    instance.entity = Some(1);
    world.entities.push(world.entities[0].clone());

    physics.add_spawned_instance(&world, &instance);
    let added: Vec<_> = physics
        .colliders
        .iter()
        .map(|(handle, _)| handle)
        .filter(|handle| !before.contains(handle))
        .collect();
    let hull = physics.npc_hulls[&1].0;
    let studio = physics.studio_hitboxes[&1].boxes[0].0;
    assert_eq!(
        added,
        [studio, hull],
        "the box reuses the discarded model slot"
    );
    assert_eq!(&physics.changed_entity_colliders[..queued.len()], queued);
    assert_eq!(&physics.changed_entity_colliders[queued.len()..], added);

    let eye = Vec3::new(147., 0., 100.);
    assert!(physics.impact_ray(eye, -Vec3::Z, 200.).is_none());
    physics.refresh_entity_queries();
    let hit = physics
        .impact_ray(eye, -Vec3::Z, 200.)
        .expect("published studio hitbox");
    assert_eq!((hit.entity, hit.hitgroup, hit.bone), (1, 3, Some(0)));
    let hull_hit = physics.trace_hull(
        Vec3::new(100., 0., 100.),
        Vec3::new(100., 0., 0.),
        Vec3::splat(-1.),
        Vec3::splat(1.),
    );
    assert!(hull_hit.fraction < 1., "the movement hull is published too");
}
