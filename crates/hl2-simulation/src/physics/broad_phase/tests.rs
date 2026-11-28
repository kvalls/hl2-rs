use super::*;
use rapier3d::prelude::{
    nalgebra, vector, BroadPhaseMultiSap, CCDSolver, ColliderBuilder, ImpulseJointSet,
    IntegrationParameters, IslandManager, Isometry, MultibodyJointSet, NarrowPhase,
    PhysicsPipeline, RigidBodyBuilder, RigidBodyHandle, RigidBodyType, Rotation, Vector,
};
use std::collections::HashSet;

type PairKey = ((u32, u32), (u32, u32));

struct Rng(u64);

impl Rng {
    fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 32) as u32
    }

    fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + self.next_f32() * (hi - lo)
    }

    fn vec(&mut self, span: f32) -> Vector<Real> {
        vector![
            self.range(-span, span),
            self.range(-span, span),
            self.range(-span, span)
        ]
    }

    fn pick(&mut self, len: usize) -> usize {
        (self.next_u32() as usize) % len
    }
}

fn pair_key(pair: ColliderPair) -> PairKey {
    let k1 = handle_key(pair.collider1);
    let k2 = handle_key(pair.collider2);
    if k1 <= k2 {
        (k1, k2)
    } else {
        (k2, k1)
    }
}

fn apply_split_events(active: &mut HashSet<PairKey>, events: &[BroadPhasePairEvent]) {
    for event in events {
        match *event {
            BroadPhasePairEvent::AddPair(pair) => {
                assert!(handle_key(pair.collider1) <= handle_key(pair.collider2));
                assert!(
                    active.insert(pair_key(pair)),
                    "duplicate AddPair emitted for {pair:?}"
                );
            }
            BroadPhasePairEvent::DeletePair(pair) => {
                assert!(handle_key(pair.collider1) <= handle_key(pair.collider2));
                assert!(
                    active.remove(&pair_key(pair)),
                    "DeletePair emitted for inactive pair {pair:?}"
                );
            }
        }
    }
}

fn apply_ref_events(
    active: &mut HashSet<PairKey>,
    events: &[BroadPhasePairEvent],
    removed: &[ColliderHandle],
) {
    for event in events {
        match *event {
            BroadPhasePairEvent::AddPair(pair) => {
                active.insert(pair_key(pair));
            }
            BroadPhasePairEvent::DeletePair(pair) => {
                active.remove(&pair_key(pair));
            }
        }
    }
    for &h in removed {
        let hk = handle_key(h);
        active.retain(|&(a, b)| a != hk && b != hk);
    }
}

fn assert_split_invariants(bp: &SplitBroadPhase, tracked: &HashSet<PairKey>) {
    assert!(bp.dirty_moving.is_empty());
    let mut reconstructed = HashSet::new();
    for (&m, partners) in &bp.active_pairs {
        assert!(bp.moving.contains_key(&m));
        assert!(!bp.static_aabbs.contains_key(&m));
        for w in partners.windows(2) {
            assert!(handle_key(w[0]) < handle_key(w[1]));
        }
        for &other in partners {
            if bp.static_aabbs.contains_key(&other) {
                assert!(bp
                    .static_partners
                    .get(&other)
                    .is_some_and(|v| v.contains(&m)));
            } else {
                assert!(bp.active_pairs.get(&other).is_some_and(|v| v.contains(&m)));
            }
            reconstructed.insert(pair_key(ColliderPair::new(m, other)));
        }
    }
    for (&s, partners) in &bp.static_partners {
        assert!(bp.static_aabbs.contains_key(&s));
        assert!(!partners.is_empty());
        for w in partners.windows(2) {
            assert!(handle_key(w[0]) < handle_key(w[1]));
        }
        for &m in partners {
            assert!(bp.active_pairs.get(&m).is_some_and(|v| v.contains(&s)));
        }
    }
    assert_eq!(&reconstructed, tracked);
}

fn expected_ref_pairs(
    ref_pairs: &HashSet<PairKey>,
    colliders: &ColliderSet,
    bodies: &RigidBodySet,
) -> HashSet<PairKey> {
    ref_pairs
        .iter()
        .copied()
        .filter(|&(k1, k2)| {
            let h1 = ColliderHandle::from_raw_parts(k1.0, k1.1);
            let h2 = ColliderHandle::from_raw_parts(k2.0, k2.1);
            let Some(c1) = colliders.get(h1).filter(|c| c.is_enabled()) else {
                return false;
            };
            let Some(c2) = colliders.get(h2).filter(|c| c.is_enabled()) else {
                return false;
            };
            !(is_fixed_only(c1, bodies) && is_fixed_only(c2, bodies))
        })
        .collect()
}

fn spawn_actor(
    rng: &mut Rng,
    bodies: &mut RigidBodySet,
    colliders: &mut ColliderSet,
    body_type: RigidBodyType,
    span: f32,
) -> (RigidBodyHandle, ColliderHandle) {
    let pos = Isometry::from_parts(
        rng.vec(span).into(),
        Rotation::from_euler_angles(rng.range(0.0, 3.1), rng.range(0.0, 3.1), 0.0),
    );
    let rb = RigidBodyBuilder::new(body_type).position(pos).build();
    let rb_h = bodies.insert(rb);
    let half = vector![
        rng.range(0.4, 2.2),
        rng.range(0.4, 2.2),
        rng.range(0.4, 2.2)
    ];
    let co = ColliderBuilder::cuboid(half.x, half.y, half.z).build();
    let co_h = colliders.insert_with_parent(co, rb_h, bodies);
    (rb_h, co_h)
}

#[test]
fn differential_parity_with_multi_sap_across_250_steps() {
    let mut rng = Rng(0x5EED_C0DE_1234_5678);
    let mut bodies = RigidBodySet::new();
    let mut colliders = ColliderSet::new();
    let mut islands = IslandManager::new();
    let mut reference = BroadPhaseMultiSap::new();
    let mut split = SplitBroadPhase::default();
    let mut ref_pairs = HashSet::new();
    let mut split_pairs = HashSet::new();
    let mut ref_events = Vec::new();
    let mut split_events = Vec::new();
    let mut actors = Vec::new();
    let mut modified = Vec::new();
    let mut removed = Vec::new();

    for i in 0..28 {
        let body_type = if i < 20 {
            RigidBodyType::Fixed
        } else {
            RigidBodyType::Dynamic
        };
        let (rb_h, co_h) = spawn_actor(&mut rng, &mut bodies, &mut colliders, body_type, 8.0);
        actors.push((rb_h, co_h, (16..20).contains(&i)));
        modified.push(co_h);
    }

    let dt = 1.0 / 60.0;
    let pred = 0.05;
    reference.update(
        dt,
        pred,
        &mut colliders,
        &bodies,
        &modified,
        &[],
        &mut ref_events,
    );
    split.update(
        dt,
        pred,
        &mut colliders,
        &bodies,
        &modified,
        &[],
        &mut split_events,
    );
    apply_ref_events(&mut ref_pairs, &ref_events, &[]);
    apply_split_events(&mut split_pairs, &split_events);
    assert_split_invariants(&split, &split_pairs);
    assert_eq!(
        split_pairs,
        expected_ref_pairs(&ref_pairs, &colliders, &bodies)
    );

    for step in 0..250 {
        modified.clear();
        removed.clear();
        ref_events.clear();
        split_events.clear();

        for _ in 0..5 {
            if actors.is_empty() {
                break;
            }
            let idx = rng.pick(actors.len());
            let (rb_h, co_h, is_door) = actors[idx];
            if modified.contains(&co_h) || removed.contains(&co_h) {
                continue;
            }
            let op = rng.next_u32() % 10;
            if op < 5 {
                if colliders.get(co_h).is_some_and(|c| c.is_enabled()) {
                    let delta = Isometry::from_parts(
                        rng.vec(2.0).into(),
                        Rotation::from_euler_angles(
                            rng.range(-0.4, 0.4),
                            rng.range(-0.4, 0.4),
                            0.0,
                        ),
                    );
                    let mut next = *bodies[rb_h].position() * delta;
                    next.translation.vector.x = next.translation.vector.x.clamp(-12.0, 12.0);
                    next.translation.vector.y = next.translation.vector.y.clamp(-12.0, 12.0);
                    next.translation.vector.z = next.translation.vector.z.clamp(-12.0, 12.0);
                    bodies.get_mut(rb_h).unwrap().set_position(next, true);
                    colliders.get_mut(co_h).unwrap().set_position(next);
                    modified.push(co_h);
                }
            } else if op == 5 && !is_door {
                let current = bodies[rb_h].body_type();
                let next_type = if current == RigidBodyType::Fixed {
                    RigidBodyType::Dynamic
                } else {
                    RigidBodyType::Fixed
                };
                bodies.get_mut(rb_h).unwrap().set_body_type(next_type, true);
                modified.push(co_h);
            } else if op == 6 {
                let co = colliders.get_mut(co_h).unwrap();
                let next_enabled = !co.is_enabled();
                co.set_enabled(next_enabled);
                modified.push(co_h);
                if !next_enabled {
                    removed.push(co_h);
                }
            } else if op == 7 && actors.len() > 14 && !is_door {
                colliders.remove(co_h, &mut islands, &mut bodies, true);
                removed.push(co_h);
                actors.swap_remove(idx);
            } else if is_door {
                let rot = Rotation::from_euler_angles(0.0, rng.range(-0.3, 0.3), 0.0);
                let mut next = *bodies[rb_h].position();
                next.rotation *= rot;
                next.translation.vector += rng.vec(0.4);
                bodies.get_mut(rb_h).unwrap().set_position(next, true);
                colliders.get_mut(co_h).unwrap().set_position(next);
                modified.push(co_h);
            }
        }

        if step % 9 == 0 && actors.len() < 36 {
            let body_type = if step % 18 == 0 {
                RigidBodyType::Fixed
            } else {
                RigidBodyType::Dynamic
            };
            let (rb_h, co_h) = spawn_actor(&mut rng, &mut bodies, &mut colliders, body_type, 8.0);
            actors.push((rb_h, co_h, false));
            modified.push(co_h);
        }

        reference.update(
            dt,
            pred,
            &mut colliders,
            &bodies,
            &modified,
            &removed,
            &mut ref_events,
        );
        split.update(
            dt,
            pred,
            &mut colliders,
            &bodies,
            &modified,
            &removed,
            &mut split_events,
        );
        apply_ref_events(&mut ref_pairs, &ref_events, &removed);
        apply_split_events(&mut split_pairs, &split_events);
        assert_split_invariants(&split, &split_pairs);
        assert_eq!(
            split_pairs,
            expected_ref_pairs(&ref_pairs, &colliders, &bodies),
            "parity mismatch at step {step}"
        );
    }
}

#[test]
fn static_to_moving_door_promotion_is_seamless() {
    let mut bodies = RigidBodySet::new();
    let mut colliders = ColliderSet::new();
    let mut bp = SplitBroadPhase::default();

    let door_rb = bodies.insert(RigidBodyBuilder::fixed().translation(vector![0.0, 0.0, 0.0]));
    let door =
        colliders.insert_with_parent(ColliderBuilder::cuboid(1.0, 2.0, 0.2), door_rb, &mut bodies);
    let box_rb = bodies.insert(RigidBodyBuilder::dynamic().translation(vector![0.5, 0.0, 0.0]));
    let dyn_box =
        colliders.insert_with_parent(ColliderBuilder::cuboid(0.5, 0.5, 0.5), box_rb, &mut bodies);

    let mut events = Vec::new();
    bp.update(
        1.0 / 60.0,
        0.01,
        &mut colliders,
        &bodies,
        &[door, dyn_box],
        &[],
        &mut events,
    );
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], BroadPhasePairEvent::AddPair(_)));
    assert!(bp.static_aabbs.contains_key(&door));

    // Translate the door slightly while still overlapping the box: zero DeletePair/AddPair.
    events.clear();
    let next_pos = Isometry::translation(0.2, 0.0, 0.0);
    bodies
        .get_mut(door_rb)
        .unwrap()
        .set_position(next_pos, true);
    colliders.get_mut(door).unwrap().set_position(next_pos);
    bp.update(
        1.0 / 60.0,
        0.01,
        &mut colliders,
        &bodies,
        &[door],
        &[],
        &mut events,
    );
    assert!(
        events.is_empty(),
        "expected seamless promotion, got {} events",
        events.len()
    );
    assert!(bp.moving.contains_key(&door));
    assert!(!bp.static_aabbs.contains_key(&door));

    // Move the door away so it separates cleanly.
    events.clear();
    let far_pos = Isometry::translation(10.0, 0.0, 0.0);
    bodies.get_mut(door_rb).unwrap().set_position(far_pos, true);
    colliders.get_mut(door).unwrap().set_position(far_pos);
    bp.update(
        1.0 / 60.0,
        0.01,
        &mut colliders,
        &bodies,
        &[door],
        &[],
        &mut events,
    );
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], BroadPhasePairEvent::DeletePair(_)));
}

#[test]
fn body_type_switch_fixed_dynamic_without_moving() {
    let mut bodies = RigidBodySet::new();
    let mut colliders = ColliderSet::new();
    let mut bp = SplitBroadPhase::default();

    let rb_a = bodies.insert(RigidBodyBuilder::fixed().translation(vector![0.0, 0.0, 0.0]));
    let col_a = colliders.insert_with_parent(ColliderBuilder::ball(1.0), rb_a, &mut bodies);
    let rb_b = bodies.insert(RigidBodyBuilder::fixed().translation(vector![0.5, 0.0, 0.0]));
    let col_b = colliders.insert_with_parent(ColliderBuilder::ball(1.0), rb_b, &mut bodies);

    let mut events = Vec::new();
    bp.update(
        1.0 / 60.0,
        0.01,
        &mut colliders,
        &bodies,
        &[col_a, col_b],
        &[],
        &mut events,
    );
    assert!(events.is_empty());

    // Switch A to Dynamic without moving position -> AddPair must fire.
    bodies
        .get_mut(rb_a)
        .unwrap()
        .set_body_type(RigidBodyType::Dynamic, true);
    bp.update(
        1.0 / 60.0,
        0.01,
        &mut colliders,
        &bodies,
        &[col_a],
        &[],
        &mut events,
    );
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], BroadPhasePairEvent::AddPair(_)));

    // Switch A back to Fixed without moving position -> DeletePair must fire.
    events.clear();
    bodies
        .get_mut(rb_a)
        .unwrap()
        .set_body_type(RigidBodyType::Fixed, true);
    bp.update(
        1.0 / 60.0,
        0.01,
        &mut colliders,
        &bodies,
        &[col_a],
        &[],
        &mut events,
    );
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], BroadPhasePairEvent::DeletePair(_)));
}

#[test]
fn fixed_fixed_sensor_overlap_emits_pair() {
    let mut bodies = RigidBodySet::new();
    let mut colliders = ColliderSet::new();
    let mut bp = SplitBroadPhase::default();

    let rb_sensor = bodies.insert(RigidBodyBuilder::fixed().translation(vector![0.0, 0.0, 0.0]));
    let sensor = colliders.insert_with_parent(
        ColliderBuilder::ball(1.0)
            .sensor(true)
            .active_collision_types(ActiveCollisionTypes::FIXED_FIXED),
        rb_sensor,
        &mut bodies,
    );
    let rb_wall = bodies.insert(RigidBodyBuilder::fixed().translation(vector![0.5, 0.0, 0.0]));
    let wall = colliders.insert_with_parent(ColliderBuilder::ball(1.0), rb_wall, &mut bodies);

    let mut events = Vec::new();
    bp.update(
        1.0 / 60.0,
        0.01,
        &mut colliders,
        &bodies,
        &[sensor, wall],
        &[],
        &mut events,
    );
    assert_eq!(events.len(), 1);
    match events[0] {
        BroadPhasePairEvent::AddPair(pair) => {
            assert_eq!(pair_key(pair), pair_key(ColliderPair::new(sensor, wall)));
        }
        BroadPhasePairEvent::DeletePair(_) => panic!("expected AddPair"),
    }
}

#[test]
fn late_static_floor_pairs_with_previously_stepped_dynamic_body() {
    let mut bodies = RigidBodySet::new();
    let mut colliders = ColliderSet::new();
    let mut bp = SplitBroadPhase::default();
    let mut events = Vec::new();

    // Step empty world once, then step dynamic body before static floor exists.
    bp.update(
        1.0 / 60.0,
        0.01,
        &mut colliders,
        &bodies,
        &[],
        &[],
        &mut events,
    );
    assert!(!bp.static_initialized);

    let dyn_rb = bodies.insert(RigidBodyBuilder::dynamic().translation(vector![0.0, 0.5, 0.0]));
    let dyn_col =
        colliders.insert_with_parent(ColliderBuilder::cuboid(0.5, 0.5, 0.5), dyn_rb, &mut bodies);
    bp.update(
        1.0 / 60.0,
        0.01,
        &mut colliders,
        &bodies,
        &[dyn_col],
        &[],
        &mut events,
    );
    assert!(events.is_empty());
    assert!(!bp.static_initialized);

    // Late static floor inserted overlapping dyn_col; only floor is in modified_colliders.
    let floor_rb = bodies.insert(RigidBodyBuilder::fixed().translation(vector![0.0, 0.0, 0.0]));
    let floor_col = colliders.insert_with_parent(
        ColliderBuilder::cuboid(10.0, 0.25, 10.0),
        floor_rb,
        &mut bodies,
    );
    bp.update(
        1.0 / 60.0,
        0.01,
        &mut colliders,
        &bodies,
        &[floor_col],
        &[],
        &mut events,
    );
    assert!(bp.static_initialized);
    assert_eq!(events.len(), 1);
    match events[0] {
        BroadPhasePairEvent::AddPair(pair) => {
            assert_eq!(
                pair_key(pair),
                pair_key(ColliderPair::new(dyn_col, floor_col))
            );
        }
        BroadPhasePairEvent::DeletePair(_) => panic!("expected AddPair"),
    }
}

#[test]
fn non_finite_aabb_is_ignored_and_cleans_up_pairs() {
    let mut bodies = RigidBodySet::new();
    let mut colliders = ColliderSet::new();
    let mut bp = SplitBroadPhase::default();
    let mut events = Vec::new();

    let bad_rb =
        bodies.insert(RigidBodyBuilder::dynamic().translation(vector![Real::NAN, 0.0, 0.0]));
    let bad_col = colliders.insert_with_parent(ColliderBuilder::ball(0.5), bad_rb, &mut bodies);
    let bad_static_rb =
        bodies.insert(RigidBodyBuilder::fixed().translation(vector![Real::NEG_INFINITY, 0.0, 0.0]));
    let bad_static_col =
        colliders.insert_with_parent(ColliderBuilder::ball(0.5), bad_static_rb, &mut bodies);
    let dyn_rb = bodies.insert(RigidBodyBuilder::dynamic().translation(vector![0.0, 0.0, 0.0]));
    let dyn_col = colliders.insert_with_parent(ColliderBuilder::ball(1.0), dyn_rb, &mut bodies);
    let floor_rb = bodies.insert(RigidBodyBuilder::fixed().translation(vector![0.0, -0.5, 0.0]));
    let floor_col = colliders.insert_with_parent(
        ColliderBuilder::cuboid(2.0, 0.5, 2.0),
        floor_rb,
        &mut bodies,
    );

    bp.update(
        1.0 / 60.0,
        0.01,
        &mut colliders,
        &bodies,
        &[bad_col, bad_static_col, dyn_col, floor_col],
        &[],
        &mut events,
    );
    assert!(!bp.moving.contains_key(&bad_col));
    assert!(!bp.static_aabbs.contains_key(&bad_static_col));
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], BroadPhasePairEvent::AddPair(_)));

    // Now make dyn_col non-finite and verify its active pair is cleaned up.
    events.clear();
    let nan_pos = Isometry::translation(Real::INFINITY, 0.0, 0.0);
    bodies.get_mut(dyn_rb).unwrap().set_position(nan_pos, true);
    colliders.get_mut(dyn_col).unwrap().set_position(nan_pos);
    bp.update(
        1.0 / 60.0,
        0.01,
        &mut colliders,
        &bodies,
        &[dyn_col],
        &[],
        &mut events,
    );
    assert!(!bp.moving.contains_key(&dyn_col));
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], BroadPhasePairEvent::DeletePair(_)));
    assert_split_invariants(&bp, &HashSet::new());
}

fn step_stacked_scene(broad_phase: &mut dyn BroadPhase) -> Vec<(Vector<Real>, Real)> {
    let mut pipeline = PhysicsPipeline::new();
    let mut islands = IslandManager::new();
    let mut narrow_phase = NarrowPhase::new();
    let mut bodies = RigidBodySet::new();
    let mut colliders = ColliderSet::new();
    let mut impulse_joints = ImpulseJointSet::new();
    let mut multibody_joints = MultibodyJointSet::new();
    let mut ccd_solver = CCDSolver::new();

    let floor_rb = bodies.insert(RigidBodyBuilder::fixed().translation(vector![0.0, -0.5, 0.0]));
    colliders.insert_with_parent(
        ColliderBuilder::cuboid(20.0, 0.5, 20.0),
        floor_rb,
        &mut bodies,
    );
    let wall_rb = bodies.insert(RigidBodyBuilder::fixed().translation(vector![4.0, 2.0, 0.0]));
    colliders.insert_with_parent(
        ColliderBuilder::cuboid(0.5, 2.5, 10.0),
        wall_rb,
        &mut bodies,
    );

    let mut box_bodies = Vec::new();
    for i in 0..4 {
        let rb = bodies.insert(RigidBodyBuilder::dynamic().translation(vector![
            0.0,
            0.5 + i as Real,
            0.0
        ]));
        colliders.insert_with_parent(ColliderBuilder::cuboid(0.5, 0.5, 0.5), rb, &mut bodies);
        box_bodies.push(rb);
    }

    let mover_rb = bodies
        .insert(RigidBodyBuilder::kinematic_position_based().translation(vector![-3.0, 1.0, 0.0]));
    colliders.insert_with_parent(
        ColliderBuilder::cuboid(0.6, 0.6, 0.6),
        mover_rb,
        &mut bodies,
    );

    let ball_rb = bodies.insert(
        RigidBodyBuilder::dynamic()
            .translation(vector![0.0, 0.5, -4.0])
            .linvel(vector![0.0, 0.0, 6.0]),
    );
    colliders.insert_with_parent(ColliderBuilder::ball(0.5), ball_rb, &mut bodies);
    box_bodies.push(ball_rb);

    let gravity = vector![0.0, -9.81, 0.0];
    let params = IntegrationParameters {
        dt: 1.0 / 60.0,
        ..Default::default()
    };

    for step in 0..120 {
        let x = -3.0 + (step as Real) * 0.04;
        bodies
            .get_mut(mover_rb)
            .unwrap()
            .set_next_kinematic_position(Isometry::translation(x.min(1.5), 1.0, 0.0));
        pipeline.step(
            &gravity,
            &params,
            &mut islands,
            broad_phase,
            &mut narrow_phase,
            &mut bodies,
            &mut colliders,
            &mut impulse_joints,
            &mut multibody_joints,
            &mut ccd_solver,
            None,
            &(),
            &(),
        );
    }

    box_bodies
        .into_iter()
        .map(|h| {
            let rb = &bodies[h];
            (*rb.translation(), rb.linvel().norm())
        })
        .collect()
}

#[test]
fn full_physics_pipeline_step_parity_stacked_boxes_and_moving_obstacles() {
    let mut reference = BroadPhaseMultiSap::new();
    let mut split = SplitBroadPhase::default();

    let ref_state = step_stacked_scene(&mut reference);
    let split_state = step_stacked_scene(&mut split);

    assert_eq!(ref_state.len(), split_state.len());
    for ((ref_pos, ref_speed), (split_pos, split_speed)) in ref_state.iter().zip(split_state.iter())
    {
        assert!(split_pos.iter().all(|c| c.is_finite()));
        assert!(split_pos.y > 0.0, "body fell through floor: {split_pos:?}");
        assert!(
            (ref_pos - split_pos).norm() < 0.15,
            "position divergence: ref={ref_pos:?}, split={split_pos:?}"
        );
        assert!(
            (ref_speed - split_speed).abs() < 0.25,
            "speed divergence: ref={ref_speed}, split={split_speed}"
        );
    }
}
