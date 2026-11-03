//! Physics ragdoll tests.
use super::super::*;
use super::*;
/// Two 8-unit cubes: the child bone 16 units above its parent in the reference pose,
/// joined by a ragdoll constraint (+-30 degrees about every axis).
fn two_cube_ragdoll() -> modkit_core::RagdollModel {
    let cube = |half: f32| modkit_core::ConvexPiece {
        vertices: (0..8)
            .map(|i| {
                Vec3::new(
                    if i & 1 == 0 { -half } else { half },
                    if i & 2 == 0 { -half } else { half },
                    if i & 4 == 0 { -half } else { half },
                )
            })
            .collect(),
        indices: Vec::new(),
    };
    let body = |bone| modkit_core::RagdollBody {
        bone,
        pieces: vec![cube(4.)],
        mass: 5.,
        inertia: 1.,
        damping: 0.,
        rotdamping: 0.,
        surfaceprop: "default".into(),
        compact_inertia: None,
    };
    modkit_core::RagdollModel {
        bodies: vec![body(0), body(1)],
        joints: vec![modkit_core::RagdollJoint {
            parent: 0,
            child: 1,
            limits: [(-30., 30.); 3],
            friction: [0.; 3],
            child_in_parent: (Vec3::Z * 16., Quat::IDENTITY),
        }],
        self_collisions: false,
        collision_pairs: Vec::new(),
        animated_friction: None,
    }
}
#[test]
fn ragdoll_rays_impulses_and_wall_pins() {
    let world = npc_floor_world();
    let mut physics = Physics::new(&world);
    let model = two_cube_ragdoll();
    let poses = [
        (Vec3::new(0., 0., 4.), Quat::IDENTITY),
        (Vec3::new(0., 0., 20.), Quat::IDENTITY),
    ];
    let handles = physics.spawn_ragdoll(&world, &model, &poses, Vec3::ZERO, 1);
    // A crossbow pin on the ragdoll's spawn tick finds it before any physics step.
    assert!(physics
        .ragdoll_ray(Vec3::new(-100., 0., 20.), Vec3::new(100., 0., 20.))
        .is_some_and(|(body, ..)| body == handles[1]));
    physics.tick(0.015);
    // The client's ray against ragdolls finds the child cube's face.
    let (body, point, fraction) = physics
        .ragdoll_ray(Vec3::new(-100., 0., 20.), Vec3::new(100., 0., 20.))
        .unwrap();
    assert_eq!(body, handles[1]);
    assert!((point.x + 4.).abs() < 0.5, "{point}");
    assert!((fraction - 0.48).abs() < 0.01, "{fraction}");
    assert!(physics
        .ragdoll_ray(Vec3::new(-100., 0., 60.), Vec3::new(100., 0., 60.))
        .is_none());
    // A shot's 4000 kg in/s on the root moves the 10 kg ragdoll along +X.
    let before = physics.ragdoll_poses(&handles).unwrap()[0].0;
    physics.ragdoll_impulse_at(handles[0], Vec3::X * 4000., before);
    for _ in 0..10 {
        physics.tick(0.015);
    }
    let after = physics.ragdoll_poses(&handles).unwrap()[0].0;
    assert!(after.x - before.x > 5., "{before} -> {after}");
    // A bolt pin 40 units away pulls the struck point to the wall and holds it.
    let (center, _) = physics.ragdoll_body_center(handles[1]).unwrap();
    let wall = center + Vec3::new(0., 40., 20.);
    let anchor = physics.pin_ragdoll(handles[1], wall, center).unwrap();
    for _ in 0..300 {
        physics.tick(0.015);
    }
    let (pinned, _) = physics.ragdoll_body_center(handles[1]).unwrap();
    assert!(pinned.distance(wall) < 4., "{pinned} vs wall {wall}");
    physics.remove_ragdoll(&handles);
    physics.remove_body(anchor);
}
#[test]
fn a_landed_ragdoll_rests_and_wakes_on_an_impact() {
    // A landed ragdoll rests; PhysForceRagdollToSleep on a moving one: every solid sleeps and stays asleep
    // (FX_AffectRagdolls' simulating count drops); an ImpactTrace push wakes it.
    let model = two_cube_ragdoll();
    let world = npc_floor_world();
    let mut physics = Physics::new(&world);
    let poses = [
        (Vec3::new(0., 0., 60.), Quat::IDENTITY),
        (Vec3::new(0., 0., 76.), Quat::IDENTITY),
    ];
    let handles = physics.spawn_ragdoll(&world, &model, &poses, Vec3::ZERO, 1);
    let awake = |physics: &Physics| {
        handles
            .iter()
            .filter(|h| physics.ragdoll_body_center(**h).is_some_and(|(_, a)| a))
            .count()
    };
    // Landed, it rests on its own within a few seconds (RAGDOLL_SLEEP_*).
    let rested = (0..300).find(|_| {
        physics.tick(0.015);
        awake(&physics) == 0
    });
    assert!(rested.is_some_and(|tick| tick > 20), "{rested:?}");
    let (center, _) = physics.ragdoll_body_center(handles[0]).unwrap();
    physics.ragdoll_impulse_at(handles[0], Vec3::X * 4000., center);
    physics.tick(0.015);
    assert_eq!(awake(&physics), 2);
    for _ in 0..100 {
        physics.tick(0.015);
    }
    physics.ragdoll_sleep(&handles);
    for _ in 0..20 {
        physics.tick(0.015);
        assert_eq!(awake(&physics), 0);
    }
    let (center, _) = physics.ragdoll_body_center(handles[0]).unwrap();
    physics.ragdoll_impulse_at(handles[0], Vec3::X * 4000., center);
    physics.tick(0.015);
    assert_eq!(awake(&physics), 2);
    assert!(physics.ragdoll_body_center(handles[0]).unwrap().0.x > center.x);
}
#[test]
fn a_two_body_ragdoll_falls_jointed_and_stays_out_of_traces() {
    let model = two_cube_ragdoll();
    let world = npc_floor_world();
    let mut physics = Physics::new(&world);
    let poses = [
        (Vec3::new(0., 0., 60.), Quat::IDENTITY),
        (Vec3::new(0., 0., 76.), Quat::IDENTITY),
    ];
    let handles = physics.spawn_ragdoll(&world, &model, &poses, Vec3::ZERO, 1);
    assert_eq!(handles.len(), 2);
    // Server traces ignore client ragdolls.
    physics.tick(0.015);
    let (_, hit) = physics
        .ray(Vec3::new(0., 0., 120.), -Vec3::Z, 200.)
        .unwrap();
    assert!(hit.z.abs() < 1., "the ray stopped at {hit}, not the floor");
    // Push the top body sideways: it swings on the joint and both land on the floor.
    physics.ragdoll_nudge(&handles, 1, Vec3::X * 2000.);
    for _ in 0..400 {
        physics.tick(0.015);
    }
    let poses = physics.ragdoll_poses(&handles).unwrap();
    for (position, _) in &poses {
        assert!(position.z > 2. && position.z < 20., "{position}");
    }
    // The joint keeps the child bone 16 units from the parent's origin.
    let (p, r) = poses[0];
    let anchor = p + r * (Vec3::Z * 16.);
    assert!(anchor.distance(poses[1].0) < 1.5, "{anchor} {}", poses[1].0);
    // The relative rotation stays within the 30 degree limits (some solver slack).
    let relative = r.inverse() * poses[1].1;
    assert!(relative.angle_between(Quat::IDENTITY).to_degrees() < 60.);
    // Player movement passes through client ragdolls (owner playtest: the player's
    // convex SAT path once missed the ragdoll group).
    let center = poses[0].0;
    let trace = CollisionWorld::trace_hull(
        &physics,
        center - Vec3::X * 64. + Vec3::Z * 2.,
        center + Vec3::X * 64. + Vec3::Z * 2.,
        Vec3::new(-16., -16., 0.),
        Vec3::new(16., 16., 72.),
    );
    assert!(
        trace.fraction == 1. && !trace.start_solid,
        "the player hull hit the ragdoll: {}",
        trace.fraction
    );
    // So do NPC hull traces (session 37: a zombie torso landed on its own legs gib).
    let hit = crate::npc_probe::NpcCollisionWorld::npc_trace_hull(
        &physics,
        center + Vec3::Z * 40.,
        center + Vec3::Z * 0.5,
        crate::ai_hull::HullType::Tiny.bounds(),
        Default::default(),
    );
    assert!(
        !hit.trace.start_solid && hit.trace.fraction > 0.95,
        "the NPC hull hit the ragdoll: {} {:?}",
        hit.trace.fraction,
        hit.entity
    );
    physics.remove_ragdoll(&handles);
    assert!(physics.world_sides.ragdoll_bodies.is_empty());
}
