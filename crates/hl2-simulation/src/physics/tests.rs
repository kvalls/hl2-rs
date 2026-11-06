mod ragdolls;
use super::*;
#[test]
fn a_spawned_npc_hull_blocks_rays_and_follows_its_state() {
    let world = World::default();
    let mut physics = Physics::new(&world);
    physics.add_entity_hull(
        7,
        Vec3::new(-13., -13., 0.),
        Vec3::new(13., 13., 72.),
        Vec3::new(100., 0., 0.),
    );
    physics.refresh_entity_queries();
    let hit = physics.projectile_ray(Vec3::new(0., 0., 36.), Vec3::new(200., 0., 36.), &[]);
    assert_eq!(hit.map(|h| h.entity), Some(7));
    assert!(
        (hit.unwrap().position.x - 87.).abs() < 0.5,
        "front face at 100 - 13"
    );
    physics.set_entity(7, Vec3::new(100., 200., 0.), Quat::IDENTITY, true);
    physics.refresh_entity_queries();
    assert!(physics
        .projectile_ray(Vec3::new(0., 0., 36.), Vec3::new(200., 0., 36.), &[])
        .is_none());
}
use modkit_core::movement::{Input, Player, TICK};
fn box_surface(min: Vec3, max: Vec3) -> Surface {
    let vertices = (0..8)
        .map(|i| modkit_core::Vertex {
            normal: Default::default(),
            position: Vec3::new(
                if i & 1 == 0 { min.x } else { max.x },
                if i & 2 == 0 { min.y } else { max.y },
                if i & 4 == 0 { min.z } else { max.z },
            ),
            uv: glam::Vec2::ZERO,
            light_uv: glam::Vec2::ZERO,
            color: [255; 4],
            skin: Some(modkit_core::animation::Weights {
                bones: [0; 3],
                weights: [1., 0., 0.],
            }),
        })
        .collect();
    Surface {
        flex_source: None,
        background: false,
        material: "synthetic".into(),
        lightmap: None,
        vertices,
        indices: vec![
            0, 2, 3, 0, 3, 1, 4, 5, 7, 4, 7, 6, 0, 1, 5, 0, 5, 4, 2, 6, 7, 2, 7, 3, 0, 4, 6, 0, 6,
            2, 1, 3, 7, 1, 7, 5,
        ],
    }
}
fn door_world() -> World {
    use modkit_core::animation::{Bone, Clip, Pose, Rig};
    let instance = modkit_core::ModelInstance {
        background: false,
        model: "synthetic_door.mdl".into(),
        origin: Vec3::ZERO,
        angles: Vec3::ZERO,
        skin: 0,
        body: 0,
        scale: 1.,
        kind: "prop_door_rotating".into(),
        solid: true,
        solid_mode: None,
        entity: Some(0),
    };
    let idle = Pose {
        position: Vec3::new(-1., -1., 0.),
        rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
    };
    let key = instance.asset_key();
    World {
        entities: vec![modkit_core::Entity {
            properties: vec![("classname".into(), "prop_door_rotating".into())],
        }],
        model_assets: BTreeMap::from([(
            key.clone(),
            vec![box_surface(
                Vec3::new(0., -2., -54.),
                Vec3::new(48., 0., 54.),
            )],
        )]),
        rigs: BTreeMap::from([(
            key,
            Rig {
                view_bounds: Default::default(),
                pose_parameters: Vec::new(),
                autoplay: Vec::new(),
                attachments: Vec::new(),
                activities: Default::default(),
                hitboxes: Vec::new(),
                bone_surfaces: Vec::new(),
                bones: vec![Bone {
                    name: "door".into(),
                    parent: None,
                    bind: Pose {
                        position: Vec3::ZERO,
                        rotation: Quat::IDENTITY,
                    },
                    inverse_bind: Mat4::IDENTITY,
                }],
                clips: BTreeMap::from([(
                    "idle".into(),
                    Clip {
                        layer: Default::default(),
                        events: Vec::new(),
                        fps: 1.,
                        looping: true,
                        frames: vec![vec![idle.clone()], vec![idle]],
                    },
                )]),
                warnings: Vec::new(),
                sequences: Vec::new(),
            },
        )]),
        model_instances: vec![instance],
        ..Default::default()
    }
}
#[test]
fn rotating_door_collision_matches_idle_panel_before_and_after_entity_swing() {
    let world = door_world();
    let mut physics = Physics::new(&world);
    let start = Vec3::new(-40., 24., -54.);
    let end = Vec3::new(40., 24., -54.);
    let mins = Vec3::new(-16., -16., 0.);
    let maxs = Vec3::new(16., 16., 72.);
    let closed = physics.trace_hull(start, end, mins, maxs);
    assert!(closed.fraction > 0. && closed.fraction < 0.5);
    assert!(!closed.start_solid);
    assert_eq!(
        physics
            .impact_ray(start + Vec3::Z * 54., Vec3::X, 80.)
            .unwrap()
            .entity,
        0
    );
    // The historical mesh collider blocked the open-side panel instead.
    let mut raw = world.clone();
    raw.rigs.clear();
    let raw = Physics::new(&raw);
    assert_eq!(raw.trace_hull(start, end, mins, maxs).fraction, 1.);
    physics.set_entity(
        0,
        Vec3::ZERO,
        Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
        true,
    );
    physics.tick(TICK);
    assert_eq!(physics.trace_hull(start, end, mins, maxs).fraction, 1.);
    assert!(physics
        .impact_ray(start + Vec3::Z * 54., Vec3::X, 80.)
        .is_none());
    // Returning to closed must move queries back, including weapon rays.
    physics.set_entity(0, Vec3::ZERO, Quat::IDENTITY, true);
    physics.tick(TICK);
    assert!(
        (physics.trace_hull(start, end, mins, maxs).fraction - closed.fraction).abs() < 0.00001
    );
}
#[test]
fn rotating_door_pose_cache_does_not_change_other_instances_of_the_same_model() {
    let mut world = door_world();
    let mut static_prop = world.model_instances[0].clone();
    static_prop.kind = "static_prop".into();
    static_prop.entity = None;
    static_prop.origin = Vec3::Z * 200.;
    world.model_instances.push(static_prop);
    let physics = Physics::new(&world);
    // A regular prop with this asset keeps the raw model geometry.
    assert!(physics
        .impact_ray(Vec3::new(24., -40., 200.), Vec3::Y, 80.)
        .is_some());
    assert!(physics
        .impact_ray(Vec3::new(-40., 24., 200.), Vec3::X, 80.)
        .is_none());
    assert_eq!(physics.colliders.len(), 2);
}
#[test]
fn incremental_entity_queries_match_full_rebuild_across_moves_rotations_and_visibility() {
    let mut incremental = Physics::new(&door_world());
    let mut rebuilt = Physics::new(&door_world());
    for i in 0..80 {
        let origin = Vec3::new((i % 5) as f32 * 30., (i % 3) as f32 * -20., 0.);
        let rotation = angles(Vec3::new(0., (i % 4) as f32 * 45., 0.));
        let enabled = i % 7 != 0;
        incremental.set_entity(0, origin, rotation, enabled);
        // Repeating an identical entity pose must preserve the result without marking it again.
        incremental.set_entity(0, origin, rotation, enabled);
        rebuilt.set_entity(0, origin, rotation, enabled);
        incremental.refresh_entity_queries();
        rebuilt.refresh_queries();
        for y in [-80., -20., 24., 60., 100.] {
            let start = Vec3::new(-100., y, 54.);
            let end = Vec3::new(250., y, 54.);
            let a = incremental.projectile_sweep(start, end, ProjectileHull::Sphere(3.), &[]);
            let b = rebuilt.projectile_sweep(start, end, ProjectileHull::Sphere(3.), &[]);
            assert_eq!(a.is_some(), b.is_some(), "pose {i}, ray {y}");
            if let (Some(a), Some(b)) = (a, b) {
                assert_eq!(a.entity, b.entity);
                assert!((a.fraction - b.fraction).abs() < 1e-6);
                assert!(a.normal.distance(b.normal) < 1e-5);
            }
        }
    }
}
#[test]
fn projectile_queries_follow_scripted_door_poses_without_advancing_simulation() {
    let world = door_world();
    let mut physics = Physics::new(&world);
    let start = Vec3::new(60., 24., 54.);
    let end = Vec3::new(140., 24., 54.);
    assert!(physics
        .projectile_sweep(start, end, ProjectileHull::Sphere(10.), &[])
        .is_none());
    physics.set_entity(0, Vec3::X * 100., Quat::IDENTITY, true);
    physics.refresh_queries();
    let hit = physics
        .projectile_sweep(start, end, ProjectileHull::Sphere(10.), &[])
        .unwrap();
    assert_eq!(hit.entity, 0);
    assert!(hit.position.x > 88. && hit.position.x < 92.);
    assert!(physics.projectile_ray(start, end, &[]).is_some());
    assert!(physics.projectile_ray(start, end, &[0]).is_none());
    let closest = physics.entity_closest_point(0, start).unwrap();
    assert!((closest.x - 99.).abs() < 0.01);
    physics.set_entity(0, Vec3::X * 100., Quat::IDENTITY, false);
    physics.refresh_queries();
    assert!(physics
        .projectile_sweep(start, end, ProjectileHull::Box(Vec3::splat(3.)), &[])
        .is_none());
    assert!(physics.entity_closest_point(0, start).is_none());
}
#[test]
fn runtime_spawn_copies_a_template_npc_with_hull_and_trace_shape() {
    // Synthetic citizen template (an 8-unit column 60 tall, not game data) far below
    // the map, then a runtime copy at x 200 (DESIGN 14r part A).
    let instance = modkit_core::ModelInstance {
        background: false,
        model: "synthetic_citizen.mdl".into(),
        origin: crate::ai::spawn::POOL_ORIGIN,
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
    let mut world = World {
        entities: vec![modkit_core::Entity {
            properties: vec![
                ("classname".into(), "npc_citizen".into()),
                (crate::ai::spawn::POOL_KEY.into(), "npc_citizen".into()),
            ],
        }],
        model_assets: BTreeMap::from([(
            key,
            vec![box_surface(Vec3::new(-4., -4., 0.), Vec3::new(4., 4., 60.))],
        )]),
        model_instances: vec![instance],
        ..Default::default()
    };
    let mut physics = Physics::new(&world);
    let mut scene = crate::entities::Scene::new(&world);
    let origin = Vec3::new(200., 0., 0.);
    let (entity, instance) = crate::ai::spawn::instantiate(&world, 0, 1, origin, 90.);
    assert_eq!(entity.get(crate::ai::spawn::POOL_KEY), None);
    assert_eq!(entity.get("angles"), Some("0 90 0"));
    let id = scene.spawn_entity(&mut world, entity);
    assert_eq!(id, 1);
    assert!(scene.states[id].visible);
    let instance = instance.unwrap();
    assert_eq!((instance.entity, instance.origin), (Some(1), origin));
    world.model_instances.push(instance.clone());
    physics.add_spawned_instance(&world, &instance);
    physics.refresh_entity_queries();
    // The hull carries a fall at its top; a shot into the column names the new id.
    let fall = physics.trace_hull(
        Vec3::new(220., 0., 100.),
        Vec3::new(220., 0., 0.),
        Vec3::splat(-16.).with_z(0.),
        Vec3::splat(16.).with_z(72.),
    );
    assert!(
        (100. - fall.fraction * 100. - 72.).abs() < 0.1,
        "{}",
        fall.fraction
    );
    let hit = physics
        .impact_ray(Vec3::new(200., 0., 100.), -Vec3::Z, 200.)
        .unwrap();
    assert_eq!(hit.entity, 1);
    // The template keeps its own colliders far below.
    let template = physics
        .impact_ray(
            origin.with_x(0.) + crate::ai::spawn::POOL_ORIGIN + Vec3::Z * 100.,
            -Vec3::Z,
            200.,
        )
        .unwrap();
    assert_eq!(template.entity, 0);
}
#[test]
fn npc_hull_box_carries_the_player_while_traces_hit_the_model_shape() {
    // Synthetic citizen: an 8-unit-wide model column 60 tall (not game data).
    let instance = modkit_core::ModelInstance {
        background: false,
        model: "synthetic_citizen.mdl".into(),
        origin: Vec3::ZERO,
        angles: Vec3::new(0., 30., 0.),
        skin: 0,
        body: 0,
        scale: 1.,
        kind: "npc_citizen".into(),
        solid: true,
        solid_mode: Some(2),
        entity: Some(0),
    };
    let key = instance.asset_key();
    let world = World {
        entities: vec![modkit_core::Entity {
            properties: vec![("classname".into(), "npc_citizen".into())],
        }],
        model_assets: BTreeMap::from([(
            key,
            vec![box_surface(Vec3::new(-4., -4., 0.), Vec3::new(4., 4., 60.))],
        )]),
        model_instances: vec![instance],
        ..Default::default()
    };
    let mut physics = Physics::new(&world);
    let (mins, maxs) = (Vec3::new(-16., -16., 0.), Vec3::new(16., 16., 72.));
    // Standing over the shoulder (player center 20 units out, beyond the model but
    // over the 13-unit hull): the fall stops on the hull top at z 72.
    let fall = physics.trace_hull(Vec3::new(20., 0., 100.), Vec3::new(20., 0., 0.), mins, maxs);
    assert!(
        (100. - fall.fraction * 100. - 72.).abs() < 0.1,
        "{}",
        fall.fraction
    );
    // A shot just outside the model column misses (the hull is not a hitbox) ...
    assert!(physics
        .impact_ray(Vec3::new(8., 0., 100.), -Vec3::Z, 200.)
        .is_none());
    // ... and one into the column hits the NPC.
    let hit = physics
        .impact_ray(Vec3::new(0., 0., 100.), -Vec3::Z, 200.)
        .unwrap();
    assert_eq!(hit.entity, 0);
    assert!((hit.position.z - 60.).abs() < 0.1, "{}", hit.position);
    // The box follows the entity without rotating, and goes with it when disabled.
    physics.set_entity(0, Vec3::X * 100., Quat::from_rotation_z(0.7), true);
    physics.refresh_queries();
    let moved = physics.trace_hull(
        Vec3::new(120., 0., 100.),
        Vec3::new(120., 0., 0.),
        mins,
        maxs,
    );
    assert!(
        (100. - moved.fraction * 100. - 72.).abs() < 0.1,
        "{}",
        moved.fraction
    );
    // A thrown 5 kg box strikes the hull box: the contact event names the NPC
    // (impact damage input) and the box stops at the hull face.
    let body = physics.bodies.insert(
        RigidBodyBuilder::dynamic()
            .translation(vector![160. * SCALE, 0., 40. * SCALE])
            .linvel(vector![-600. * SCALE, 0., 0.])
            .gravity_scale(0.)
            .additional_mass(5.),
    );
    physics.colliders.insert_with_parent(
        ColliderBuilder::cuboid(4. * SCALE, 4. * SCALE, 4. * SCALE).density(0.),
        body,
        &mut physics.bodies,
    );
    let mut struck = false;
    for _ in 0..20 {
        physics.tick(TICK);
        struck |= physics
            .collisions
            .iter()
            .any(|c| c.entities.contains(&Some(0)));
    }
    assert!(struck);
    // The model-shaped hitboxes never form contact pairs with props (Rapier
    // enables a pair when either side's collision types allow it): a box placed
    // across the model column's face touches only the hull.
    let inside = physics.bodies.insert(
        RigidBodyBuilder::dynamic()
            .translation(vector![4. * SCALE, 0., 30. * SCALE])
            .gravity_scale(0.),
    );
    physics.colliders.insert_with_parent(
        ColliderBuilder::cuboid(2. * SCALE, 2. * SCALE, 2. * SCALE),
        inside,
        &mut physics.bodies,
    );
    physics.tick(TICK);
    assert!(physics.narrow.contact_pairs().all(|pair| {
        !physics.npc_hitboxes.contains(&pair.collider1)
            && !physics.npc_hitboxes.contains(&pair.collider2)
            || !pair.has_any_active_contact
    }));
    physics.remove_body(inside);
    let x = physics.bodies[body].translation().x / SCALE;
    assert!(x > 100. + 13., "box passed into the hull: x {x}");
    physics.remove_body(body);
    physics.set_entity(0, Vec3::X * 100., Quat::IDENTITY, false);
    physics.refresh_queries();
    let gone = physics.trace_hull(
        Vec3::new(120., 0., 100.),
        Vec3::new(120., 0., 0.),
        mins,
        maxs,
    );
    assert_eq!(gone.fraction, 1.);
}
fn native_prop_world() -> World {
    let instance = modkit_core::ModelInstance {
        background: false,
        model: "synthetic_separated_prop.mdl".into(),
        origin: Vec3::new(100., 200., 20.),
        angles: Vec3::new(0., 90., 0.),
        skin: 0,
        body: 0,
        scale: 1.,
        kind: "static_prop".into(),
        solid: true,
        solid_mode: Some(6),
        entity: None,
    };
    let piece = |min, max| {
        let surface = box_surface(min, max);
        modkit_core::ConvexPiece {
            vertices: surface.vertices.iter().map(|v| v.position).collect(),
            indices: surface.indices.as_chunks::<3>().0.to_vec(),
        }
    };
    let key = instance.asset_key();
    World {
        model_assets: BTreeMap::from([(
            key.clone(),
            vec![box_surface(
                Vec3::new(-20., -20., 0.),
                Vec3::new(20., 20., 40.),
            )],
        )]),
        model_collision: BTreeMap::from([(
            key,
            vec![
                piece(Vec3::new(-20., -20., 0.), Vec3::new(-5., 20., 40.)),
                piece(Vec3::new(5., -20., 0.), Vec3::new(20., 20., 40.)),
            ],
        )]),
        model_instances: vec![instance],
        ..Default::default()
    }
}
#[test]
fn native_convex_prop_pieces_preserve_rotated_gaps_and_weapon_rays() {
    let world = native_prop_world();
    let physics = Physics::new(&world);
    assert_eq!(physics.colliders.len(), 2);
    assert_eq!(physics.native_shape_count, 2);
    assert_eq!(physics.native_shape_fallbacks, 0);
    assert!(physics
        .colliders
        .iter()
        .all(|(_, c)| player_convex::supported(c)));
    let origin = world.model_instances[0].origin;
    let center = origin + Vec3::Z * 10.;
    let mins = Vec3::new(-1., -1., 0.);
    let maxs = Vec3::new(1., 1., 2.);
    // Rotating the local X gap by90degrees makes a world X corridor.
    let start = center - Vec3::X * 40.;
    let end = center + Vec3::X * 40.;
    let through_gap = physics.trace_hull(start, end, mins, maxs);
    assert_eq!(through_gap.fraction, 1.);
    assert!(!through_gap.start_solid);
    assert!(physics.impact_ray(start, Vec3::X, 80.).is_none());
    let across = physics.trace_hull(center - Vec3::Y * 40., center + Vec3::Y * 40., mins, maxs);
    assert!(across.fraction > 0. && across.fraction < 0.5);
    assert!(across.normal.distance(-Vec3::Y) < 0.00001);
    assert!(physics
        .impact_ray(start + Vec3::Y * 12., Vec3::X, 80.)
        .is_some());
    // Confirm this tests native decomposition rather than the broader
    // synthetic render mesh, which fills the gap.
    let mut fallback = world;
    fallback.model_collision.clear();
    assert!(
        Physics::new(&fallback)
            .trace_hull(start, end, mins, maxs)
            .fraction
            < 1.
    );
}
#[test]
fn invalid_native_piece_falls_back_as_a_whole_and_dynamic_props_keep_prior_policy() {
    let mut world = native_prop_world();
    let key = world.model_instances[0].asset_key();
    world.model_collision.get_mut(&key).unwrap()[1]
        .vertices
        .clear();
    let fallback = Physics::new(&world);
    assert_eq!(fallback.colliders.len(), 1);
    assert_eq!(fallback.native_shape_count, 0);
    assert_eq!(fallback.native_shape_fallbacks, 1);
    // Dynamic prop fallback remains one body and one convex render hull.
    world = native_prop_world();
    world.model_instances[0].kind = "prop_physics".into();
    let dynamic = Physics::new(&world);
    assert_eq!(dynamic.colliders.len(), 1);
    assert_eq!(dynamic.bodies.len(), 1);
    assert_eq!(dynamic.native_shape_count, 0);
    assert_eq!(dynamic.native_shape_fallbacks, 0);
}
/// The synthetic prop (body z 20..60 around (100, 200)) as a named prop_physics
/// plus one constraint entity (synthetic keyvalues, not game data).
fn constrained_prop(constraint: &[(&str, &str)]) -> (Physics, RigidBodyHandle) {
    let mut world = native_prop_world();
    world.model_instances[0].kind = "prop_physics".into();
    world.model_instances[0].entity = Some(0);
    world.entities.push(modkit_core::Entity {
        properties: vec![
            ("classname".into(), "prop_physics".into()),
            ("targetname".into(), "block".into()),
        ],
    });
    world.entities.push(modkit_core::Entity {
        properties: constraint
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    });
    let physics = Physics::new(&world);
    let body = physics.dynamic[&0];
    (physics, body)
}
#[test]
fn debris_props_let_the_player_through_but_stop_picks() {
    // The synthetic prop (body x 80..120, y 180..220, z 20..60) with and without
    // SF_PHYSPROP_DEBRIS; the player hull sweeps across it.
    for (flags, blocks) in [("4", false), ("0", true)] {
        let mut world = native_prop_world();
        world.model_instances[0].kind = "prop_physics".into();
        world.model_instances[0].entity = Some(0);
        world.entities.push(modkit_core::Entity {
            properties: vec![
                ("classname".into(), "prop_physics".into()),
                ("spawnflags".into(), flags.into()),
            ],
        });
        let mut physics = Physics::new(&world);
        physics.refresh_queries();
        let (mins, maxs) = (Vec3::new(-16., -16., 0.), Vec3::new(16., 16., 72.));
        let sweep = physics.trace_hull(
            Vec3::new(0., 200., 10.),
            Vec3::new(200., 200., 10.),
            mins,
            maxs,
        );
        assert_eq!(sweep.fraction < 1., blocks, "flags {flags}");
        // +USE / the gravity gun (COLLISION_GROUP_NONE traces) still find it.
        let pick = physics.pick_ray(Vec3::new(0., 200., 40.), Vec3::X, 200.);
        assert_eq!(pick.map(|h| h.entity), Some(0), "flags {flags}");
    }
}
#[test]
fn dropped_weapons_let_players_and_npcs_through_but_not_traces() {
    let mut world = native_prop_world();
    world.model_instances[0].kind = "prop_physics".into();
    world.model_instances[0].entity = Some(0);
    world.entities.push(modkit_core::Entity {
        properties: vec![("classname".into(), "weapon_stunstick".into())],
    });
    let mut physics = Physics::new(&world);
    physics.set_weapon_group(0);
    physics.refresh_queries();
    let (mins, maxs) = (Vec3::new(-16., -16., 0.), Vec3::new(16., 16., 72.));
    let sweep = physics.trace_hull(
        Vec3::new(0., 200., 10.),
        Vec3::new(200., 200., 10.),
        mins,
        maxs,
    );
    assert_eq!(sweep.fraction, 1., "the player hull hit the weapon");
    let body = &physics.bodies[physics.dynamic[&0]];
    let groups = physics.colliders[body.colliders()[0]].collision_groups();
    for blocked in [PLAYER_GROUPS, DEBRIS_GROUPS] {
        assert!(!groups.test(blocked));
    }
    assert!(!groups.test(InteractionGroups::new(HULL_GROUP, Group::ALL)));
    assert!(!groups.test(InteractionGroups::new(SHADOW_GROUP, Group::ALL)));
    assert!(groups.test(InteractionGroups::new(Group::GROUP_1, Group::ALL)));
    // Bullets and the gravity gun still find it.
    let shot = physics.ray(Vec3::new(0., 200., 40.), Vec3::X, 200.);
    assert!(shot.is_some());
    let pick = physics.pick_ray(Vec3::new(0., 200., 40.), Vec3::X, 200.);
    assert_eq!(pick.map(|h| h.entity), Some(0));
}
#[test]
fn hardest_impacts_come_first_and_a_freed_stuck_prop_leaves_the_wall() {
    let event = |speed: f32| PhysicsCollision {
        colliders: [ColliderHandle::invalid(); 2],
        entities: [Some(0), None],
        surfaceprops: [None, None],
        position: Vec3::ZERO,
        origins: [Vec3::ZERO; 2],
        normal: Vec3::X,
        speed,
        delta_time: f32::MAX,
        pre_velocities: [Vec3::ZERO; 2],
        post_velocities: [Vec3::ZERO; 2],
        masses: [1., 0.],
        angular: [(0., 0.); 2],
    };
    // The edge contact Rapier happened to list first (146 u/s along its normal)
    // goes after the head-on one (1475 u/s); equal speeds keep their order.
    let mut events = vec![event(146.), event(1475.), event(146.)];
    events[2].position = Vec3::Z;
    impact_order(&mut events);
    let order: Vec<_> = events.iter().map(|e| (e.speed, e.position.z)).collect();
    assert_eq!(order, [(1475., 0.), (146., 0.), (146., 1.)]);

    let mut world = native_prop_world();
    world.model_instances[0].kind = "prop_physics".into();
    world.model_instances[0].entity = Some(0);
    world.entities.push(modkit_core::Entity {
        properties: vec![("classname".into(), "prop_physics".into())],
    });
    let mut physics = Physics::new(&world);
    let groups = |physics: &Physics| {
        let body = &physics.bodies[physics.dynamic[&0]];
        physics.colliders[body.colliders()[0]].collision_groups()
    };
    physics.stick(0, Vec3::new(8., 0., 40.), Vec3::new(0., 0., 40.));
    assert_eq!(groups(&physics), DEBRIS_GROUPS);
    let body = &physics.bodies[physics.dynamic[&0]];
    assert!(body.is_fixed());
    assert!((body.translation().x / SCALE - 8.).abs() < 1e-3);
    // A punt or pickup frees it (EnableMotion) back at the position it hit the wall
    // from, as interactive debris (OnPhysGunPickup for world-stick props).
    physics.make_interactive_debris(0);
    let body = &physics.bodies[physics.dynamic[&0]];
    assert!(body.is_dynamic());
    assert!(body.translation().x.abs() < 1e-3);
    assert!(physics.is_interactive_debris(0));
    assert!(physics.stuck.is_empty());
}
#[test]
fn picked_up_debris_hits_npcs_until_it_sleeps() {
    let mut world = native_prop_world();
    world.model_instances[0].kind = "prop_physics".into();
    world.model_instances[0].entity = Some(0);
    world.entities.push(modkit_core::Entity {
        properties: vec![
            ("classname".into(), "prop_physics".into()),
            ("spawnflags".into(), "4".into()),
        ],
    });
    let mut physics = Physics::new(&world);
    let hull = InteractionGroups::new(
        HULL_GROUP,
        Group::ALL
            .difference(PROJECTILE_GROUP)
            .difference(SHADOW_GROUP)
            .difference(RAGDOLL_GROUP),
    );
    let groups = |physics: &Physics| {
        let body = &physics.bodies[physics.dynamic[&0]];
        physics.colliders[body.colliders()[0]].collision_groups()
    };
    assert!(!groups(&physics).test(hull));
    physics.make_interactive_debris(0);
    assert!(physics.is_interactive_debris(0));
    // NPC hulls collide; the player and other debris still do not.
    assert!(groups(&physics).test(hull));
    assert!(!groups(&physics).test(PLAYER_GROUPS));
    assert!(!groups(&physics).test(DEBRIS_GROUPS));
    physics.refresh_queries();
    let sweep = physics.trace_hull(
        Vec3::new(0., 200., 10.),
        Vec3::new(200., 200., 10.),
        Vec3::new(-16., -16., 0.),
        Vec3::new(16., 16., 72.),
    );
    assert_eq!(sweep.fraction, 1.);
    // At rest it is debris again.
    let handle = physics.dynamic[&0];
    physics.bodies[handle].sleep();
    physics.tick(TICK);
    assert!(!physics.is_interactive_debris(0));
    assert_eq!(groups(&physics), DEBRIS_GROUPS);
}
#[test]
fn hinged_prop_keeps_its_pivot_while_ropes_stay_off() {
    let (mut physics, body) = constrained_prop(&[
        ("classname", "phys_hinge"),
        ("attach1", "block"),
        ("origin", "100 200 60"),
        ("hingeaxis", "100 210 60"),
        ("hingefriction", "0.05"),
        ("spawnflags", "1"),
    ]);
    assert_eq!(
        (physics.constraints, physics.constraint_errors.len()),
        (1, 0)
    );
    let pivot = physics.bodies[body]
        .position()
        .inverse_transform_point(&Point::from(vector(Vec3::new(100., 200., 60.))));
    // A push off the hinge axis: it swings about it instead of falling.
    physics.bodies[body].set_linvel(vector![2., 0., 0.], true);
    for _ in 0..200 {
        physics.tick(TICK);
    }
    let held = physics.bodies[body].position() * pivot;
    let held = Vec3::new(held.x, held.y, held.z) / SCALE;
    assert!(held.distance(Vec3::new(100., 200., 60.)) < 0.5, "{held}");
    let state = physics.body_state(body).unwrap();
    assert!(state.position.z > 10., "fell: {}", state.position);
    // Ropes are disabled (rejected in session 15): reported, not joined.
    let (physics, _) = constrained_prop(&[
        ("classname", "phys_lengthconstraint"),
        ("attach2", "block"),
        ("origin", "100 200 200"),
        ("attachpoint", "100 200 60"),
    ]);
    assert_eq!(
        (physics.constraints, physics.constraint_errors.len()),
        (0, 1)
    );
    // Unknown attachments are reported, not joined.
    let (physics, _) = constrained_prop(&[
        ("classname", "phys_ballsocket"),
        ("attach1", "nothing"),
        ("origin", "0 0 0"),
    ]);
    assert_eq!(
        (physics.constraints, physics.constraint_errors.len()),
        (0, 1)
    );
}
#[test]
fn authored_vphysics_props_use_phy_mass_pieces_damping_and_surface_material() {
    // Synthetic parameters (not read from a game file).
    let mut world = native_prop_world();
    world.model_instances[0].kind = "prop_physics".into();
    world.model_instances[0].entity = Some(0);
    world.entities.push(modkit_core::Entity {
        properties: vec![("classname".into(), "prop_physics".into())],
    });
    let key = world.model_instances[0].asset_key();
    world.model_physics.insert(
        key,
        modkit_core::PhysicsSolid {
            mass: 35.,
            damping: 0.,
            rotdamping: 0.2,
            inertia: 2.,
            surfaceprop: "synthetic_wood".into(),
            ..Default::default()
        },
    );
    world.surface_materials.insert(
        "synthetic_wood".into(),
        modkit_core::SurfaceMaterial {
            friction: 0.6,
            elasticity: 0.3,
            ..Default::default()
        },
    );
    let physics = Physics::new(&world);
    assert_eq!(
        physics.native_shape_count, 2,
        "authored pieces, not the hull"
    );
    let body = &physics.bodies[physics.dynamic[&0]];
    assert!((body.mass() - 35.).abs() < 1e-3, "{}", body.mass());
    assert_eq!(body.linear_damping(), 0.);
    assert_eq!(body.angular_damping(), 0.2);
    assert_eq!(physics.prop_materials[&0], "synthetic_wood");
    for (_, collider) in physics.colliders.iter() {
        assert_eq!(collider.friction(), 0.6);
        assert_eq!(collider.restitution(), 0.3);
        assert_eq!(
            collider.friction_combine_rule(),
            CoefficientCombineRule::Multiply
        );
    }
    let inertia = body.mass_properties().local_mprops.principal_inertia();
    // massScale doubles the mass; inertiaScale 0.1 clamps to 0.5 x the authored 2.
    world.entities[0]
        .properties
        .push(("massScale".into(), "2".into()));
    world.entities[0]
        .properties
        .push(("inertiaScale".into(), "0.1".into()));
    let scaled = Physics::new(&world);
    let body = &scaled.bodies[scaled.dynamic[&0]];
    assert!((body.mass() - 70.).abs() < 1e-3);
    let scaled_inertia = body.mass_properties().local_mprops.principal_inertia();
    // 2x mass and a 0.5 / 2 inertia multiplier: half the tensor.
    assert!((scaled_inertia - inertia * 0.5).norm() < inertia.norm() * 1e-3);
}
#[test]
fn world_brush_sides_use_their_own_surfaceprop() {
    // Synthetic: one floor brush whose top side is "synthetic/ice" (friction 0.1)
    // or the default material; a 5 kg box slides across it from 300 u/s.
    let slide = |top: &str| {
        let mut world = World {
            brushes: vec![bounds_brush(
                Vec3::new(-4000., -4000., -16.),
                Vec3::new(4000., 4000., 0.),
            )],
            brush_side_materials: vec![["", "", "", "", top, ""].map(String::from).to_vec()],
            ..Default::default()
        };
        for (name, friction) in [("default", 0.8), ("synthetic_ice", 0.1)] {
            world.surface_materials.insert(
                name.into(),
                modkit_core::SurfaceMaterial {
                    friction,
                    elasticity: 0.1,
                    ..Default::default()
                },
            );
        }
        world
            .material_surfaceprops
            .insert("synthetic/ice".into(), "synthetic_ice".into());
        world.model_physics.insert(
            "synthetic#0".into(),
            modkit_core::PhysicsSolid {
                mass: 5.,
                damping: 0.,
                rotdamping: 0.,
                surfaceprop: "default".into(),
                ..Default::default()
            },
        );
        let mut physics = Physics::new(&world);
        physics.air_density = 0.;
        let hooked = physics.world_sides.sides.len();
        let body = physics.add_projectile_body(
            &world,
            "synthetic#0",
            Vec3::splat(4.),
            Vec3::new(0., 0., 4.05),
            Quat::IDENTITY,
            Vec3::new(300., 0., 0.),
            Vec3::ZERO,
        );
        for _ in 0..60 {
            physics.tick(0.015);
        }
        (hooked, physics.body_state(body).unwrap().position.x)
    };
    let (plain_hooks, plain) = slide("");
    let (ice_hooks, ice) = slide("synthetic/ice");
    assert_eq!((plain_hooks, ice_hooks), (0, 1));
    assert!(ice > plain + 20., "ice {ice} vs default {plain}");
}
#[test]
fn free_bodies_report_new_contacts_once_and_feel_air_drag() {
    // Synthetic floor and solid (not game data).
    let mut world = World {
        brushes: vec![bounds_brush(
            Vec3::new(-2000., -2000., -16.),
            Vec3::new(2000., 2000., 0.),
        )],
        ..Default::default()
    };
    world.model_physics.insert(
        "synthetic#0".into(),
        modkit_core::PhysicsSolid {
            mass: 5.,
            damping: 0.,
            rotdamping: 0.,
            surfaceprop: "synthetic_metal".into(),
            ..Default::default()
        },
    );
    let mut physics = Physics::new(&world);
    assert_eq!(physics.air_density, crate::drag::AIR_DENSITY);
    let body = physics.add_projectile_body(
        &world,
        "synthetic#0",
        Vec3::splat(4.),
        Vec3::new(0., 0., 1000.),
        Quat::IDENTITY,
        Vec3::new(1000., 0., 0.),
        Vec3::ZERO,
    );
    physics.tick(0.015);
    let v = physics.body_state(body).unwrap().velocity;
    // Retail drag on the 8-unit box (5 kg) moving along x: face 8 x 8 units, full
    // orthographic fraction, factor -0.5 x basis x |v| x density x dt (m/s).
    let face = (8. * 0.0254f32).powi(2);
    let speed = 1000. * SCALE;
    let factor = -0.5 * face / 5. * speed * crate::drag::AIR_DENSITY * 0.015;
    let expected = 1000. * (1. + factor);
    assert!((v.x - expected).abs() < 0.5, "{v} vs {expected}");
    // Drop onto the floor: one event at the fall speed, then resting is silent.
    physics.air_density = 0.;
    physics.set_body_velocity(body, Vec3::ZERO, Vec3::ZERO);
    if let Some(b) = physics.bodies.get_mut(body) {
        b.set_translation(vector(Vec3::new(0., 0., 40.)), true);
    }
    let mut events = vec![];
    for _ in 0..120 {
        physics.tick(0.015);
        events.extend(physics.collisions.iter().cloned());
    }
    assert!(!events.is_empty());
    let first = &events[0];
    let fall = (2. * 600. * 36f32).sqrt();
    assert!(
        (first.speed - fall).abs() < 25.,
        "{} vs {fall}",
        first.speed
    );
    assert_eq!(first.delta_time, f32::MAX);
    assert!(first.surfaceprops.contains(&Some("synthetic_metal".into())));
    assert!(first.entities.iter().all(Option::is_none));
    // Any later contact events are small settling contacts, not new impacts.
    assert!(events[1..].iter().all(|e| e.speed < 70.), "{events:?}");
    let rest = physics.body_state(body).unwrap().position;
    assert!((rest.z - 4.).abs() < 1., "{rest}");
}
#[test]
fn static_prop_bbox_and_unknown_modes_do_not_reuse_native_hulls() {
    for mode in [None, Some(2)] {
        let mut world = native_prop_world();
        world.model_instances[0].solid_mode = mode;
        let physics = Physics::new(&world);
        assert_eq!(physics.colliders.len(), 1);
        assert_eq!(physics.native_shape_count, 0);
        assert_eq!(physics.native_shape_fallbacks, 0);
        let center = world.model_instances[0].origin + Vec3::Z * 10.;
        let hit = physics.trace_hull(
            center - Vec3::X * 40.,
            center + Vec3::X * 40.,
            Vec3::new(-1., -1., 0.),
            Vec3::new(1., 1., 2.),
        );
        assert!(
            hit.fraction < 1.,
            "mode {mode:?} must use the complete render fallback"
        );
        // Mixed instances of one model must not share collision policy
        // merely because its native model pieces were already cached.
        world = native_prop_world();
        let mut other = world.model_instances[0].clone();
        other.solid_mode = mode;
        other.origin.z += 200.;
        world.model_instances.push(other);
        let mixed = Physics::new(&world);
        assert_eq!(mixed.colliders.len(), 3);
        assert_eq!(mixed.native_shape_count, 2);
        assert!(mixed
            .impact_ray(center - Vec3::X * 40., Vec3::X, 80.)
            .is_none());
        assert!(mixed
            .impact_ray(center + Vec3::Z * 200. - Vec3::X * 40., Vec3::X, 80.)
            .is_some());
    }
}
pub(super) fn bounds_brush(min: Vec3, max: Vec3) -> Brush {
    Brush {
        contents: 1,
        planes: [
            (Vec3::X, max.x),
            (-Vec3::X, -min.x),
            (Vec3::Y, max.y),
            (-Vec3::Y, -min.y),
            (Vec3::Z, max.z),
            (-Vec3::Z, -min.z),
        ]
        .into_iter()
        .map(|(normal, distance)| modkit_core::Plane { normal, distance })
        .collect(),
    }
}
#[test]
fn brush_wall_tangent_sweep_has_no_false_vertical_contact() {
    let physics = Physics::new(&World {
        brushes: vec![bounds_brush(
            Vec3::new(32., -1000., 0.),
            Vec3::new(100., 1000., 300.),
        )],
        ..Default::default()
    });
    let start = Vec3::new(15.96875, 0., 100.);
    let hit = physics.trace_hull(
        start,
        start + Vec3::new(0., 2., 1.),
        Vec3::new(-16., -16., 0.),
        Vec3::new(16., 16., 72.),
    );
    assert_eq!(hit.fraction, 1.);
    assert!(!hit.start_solid);
    assert!(!hit.all_solid);
}
#[test]
fn pressing_into_brush_wall_does_not_interrupt_jump_or_inject_sideways_motion() {
    let floor = bounds_brush(
        Vec3::new(-1000., -1000., -100.),
        Vec3::new(1000., 1000., 0.),
    );
    let open = Physics::new(&World {
        brushes: vec![floor.clone()],
        ..Default::default()
    });
    let walled = Physics::new(&World {
        brushes: vec![
            floor,
            bounds_brush(Vec3::new(32., -1000., 0.), Vec3::new(100., 1000., 300.)),
        ],
        ..Default::default()
    });
    let mut free_player = Player::new(Vec3::new(0., 0., 64.03125));
    let mut wall_player = Player::new(Vec3::new(15.96875, 0., 64.03125));
    free_player.step(Input::default(), &open, TICK);
    wall_player.step(Input::default(), &walled, TICK);
    for tick in 0..45 {
        let input = Input {
            forward: 1.,
            jump: tick == 0,
            ..Default::default()
        };
        free_player.step(input, &open, TICK);
        wall_player.step(input, &walled, TICK);
        assert!(
            (wall_player.feet.z - free_player.feet.z).abs() < 0.001,
            "tick {tick}: wall={wall_player:?}, open={free_player:?}"
        );
        assert!(wall_player.feet.y.abs() < 0.0001);
        assert!(wall_player.feet.x <= 15.969);
    }
}
#[test]
fn moving_overlap_reports_start_solid_and_allows_an_exit() {
    let mins = Vec3::new(-1., -1., 0.);
    let maxs = Vec3::new(1., 1., 2.);
    let mut rapier = Physics::default();
    rapier.colliders.insert(
        ColliderBuilder::cuboid(5. * SCALE, 8. * SCALE, 50. * SCALE).translation(vector![
            15. * SCALE,
            0.,
            50. * SCALE
        ]),
    );
    rapier.query.update(&rapier.colliders);
    for physics in [Physics::new(&clip_box(1)), rapier] {
        let start = Vec3::new(15., 0., 40.);
        let exit = physics.trace_hull(start, Vec3::new(-5., 0., 40.), mins, maxs);
        assert!(exit.start_solid);
        assert!(!exit.all_solid);
        assert_eq!(exit.fraction, 1.);
        let inside = physics.trace_hull(start, start + Vec3::Y, mins, maxs);
        assert!(inside.start_solid);
        assert!(inside.all_solid);
        assert_eq!(inside.fraction, 0.);
    }
}
fn convex_box(min: Vec3, max: Vec3, polyhedron: bool) -> ColliderBuilder {
    let half = (max - min) * 0.5;
    let shape = if polyhedron {
        let mut vertices = Vec::new();
        for x in [-half.x, half.x] {
            for y in [-half.y, half.y] {
                for z in [-half.z, half.z] {
                    vertices.push(Point::from(vector(Vec3::new(x, y, z))));
                }
            }
        }
        SharedShape::convex_hull(&vertices).unwrap()
    } else {
        SharedShape::cuboid(half.x * SCALE, half.y * SCALE, half.z * SCALE)
    };
    ColliderBuilder::new(shape).position(pose((min + max) * 0.5, Quat::IDENTITY))
}
#[test]
fn convex_prop_wall_preserves_jump_and_tangent_motion() {
    for polyhedron in [false, true] {
        let mut open = Physics::default();
        open.colliders.insert(convex_box(
            Vec3::new(-1000., -1000., -100.),
            Vec3::new(1000., 1000., 0.),
            polyhedron,
        ));
        open.query.update(&open.colliders);
        let mut walled = Physics::default();
        walled.colliders.insert(convex_box(
            Vec3::new(-1000., -1000., -100.),
            Vec3::new(1000., 1000., 0.),
            polyhedron,
        ));
        walled.colliders.insert(convex_box(
            Vec3::new(32., -1000., 0.),
            Vec3::new(100., 1000., 300.),
            polyhedron,
        ));
        walled.query.update(&walled.colliders);
        let start = Vec3::new(15.96875, 0., 100.);
        let tangent = walled.trace_hull(
            start,
            start + Vec3::new(0., 2., 1.),
            Vec3::new(-16., -16., 0.),
            Vec3::new(16., 16., 72.),
        );
        assert_eq!(tangent.fraction, 1.);
        assert!(!tangent.start_solid);
        let mut free_player = Player::new(Vec3::new(0., 0., 64.03125));
        let mut wall_player = Player::new(Vec3::new(15.96875, 0., 64.03125));
        free_player.step(Input::default(), &open, TICK);
        wall_player.step(Input::default(), &walled, TICK);
        for tick in 0..45 {
            let input = Input {
                forward: 1.,
                jump: tick == 0,
                ..Default::default()
            };
            free_player.step(input, &open, TICK);
            wall_player.step(input, &walled, TICK);
            assert!(
                (wall_player.feet.z - free_player.feet.z).abs() < 0.001,
                "polyhedron {polyhedron}, tick {tick}: {wall_player:?}"
            );
            assert!(wall_player.feet.y.abs() < 0.0001);
            assert!(wall_player.feet.x <= 15.969);
        }
    }
}
#[test]
fn rotated_convex_edge_bevel_allows_clearance_and_blocks_fast_crossing() {
    let rotation = Quat::from_rotation_z(35f32.to_radians())
        * Quat::from_rotation_y(40f32.to_radians())
        * Quat::from_rotation_x(25f32.to_radians());
    for polyhedron in [false, true] {
        let mut physics = Physics::default();
        let handle = physics.colliders.insert(
            convex_box(Vec3::new(-2., -4., -6.), Vec3::new(2., 4., 6.), polyhedron)
                .position(pose(Vec3::ZERO, rotation)),
        );
        physics.query.update(&physics.colliders);
        let start = Vec3::new(5.968779, 5.9419513, 6.3287473);
        let half = Vec3::new(1., 2., 3.);
        // This box is separated by an edge-cross-edge axis by .4119 units,
        // while all convex-face and player XYZ projections overlap.
        let collider = &physics.colliders[handle];
        assert!(!rapier3d::parry::query::intersection_test(
            collider.position(),
            collider.shape(),
            &pose(start, Quat::IDENTITY),
            &Cuboid::new(vector(half)),
        )
        .unwrap());
        assert!(!physics.trace_hull(start, start, -half, half).start_solid);
        let enter = physics.trace_hull(start, Vec3::ZERO, -half, half);
        assert!((enter.fraction - 0.05053343).abs() < 0.00001);
        assert!(enter.normal.distance(Vec3::new(0.94934547, 0.31423426, 0.)) < 0.0001);
        let crossing = physics.trace_hull(-Vec3::X * 200., Vec3::X * 200., -half, half);
        assert!(crossing.fraction > 0. && crossing.fraction < 0.5);
        assert!(!crossing.start_solid);
    }
}
#[test]
fn convex_cache_tracks_translation_rotation_shape_and_disabled_state() {
    let mut physics = Physics::default();
    let handle = physics
        .colliders
        .insert(convex_box(Vec3::splat(-2.), Vec3::splat(2.), false));
    let start = Vec3::X * 20.;
    let mins = -Vec3::ONE;
    let maxs = Vec3::ONE;
    physics.query.update(&physics.colliders);
    let initial = physics.trace_hull(start, Vec3::ZERO, mins, maxs);
    for i in 0..1000 {
        let t = Vec3::new((i as f32 * 0.17).sin() * 4., (i as f32 * 0.09).cos(), 0.);
        physics.colliders[handle].set_position(pose(t, Quat::IDENTITY));
        physics.query.update(&physics.colliders);
        physics.trace_hull(start + t, t, mins, maxs);
    }
    physics.colliders[handle].set_position(pose(Vec3::ZERO, Quat::IDENTITY));
    physics.query.update(&physics.colliders);
    assert_eq!(
        physics.trace_hull(start, Vec3::ZERO, mins, maxs).fraction,
        initial.fraction
    );
    physics.colliders[handle].set_shape(SharedShape::cuboid(4. * SCALE, 2. * SCALE, 2. * SCALE));
    physics.query.update(&physics.colliders);
    assert!(physics.trace_hull(start, Vec3::ZERO, mins, maxs).fraction < initial.fraction);
    physics.colliders[handle].set_position(pose(
        Vec3::ZERO,
        Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
    ));
    physics.query.update(&physics.colliders);
    assert!(
        (physics.trace_hull(start, Vec3::ZERO, mins, maxs).fraction - initial.fraction).abs()
            < 0.00001
    );
    physics.colliders[handle].set_enabled(false);
    physics.query.update(&physics.colliders);
    assert_eq!(
        physics.trace_hull(start, Vec3::ZERO, mins, maxs).fraction,
        1.
    );
}
fn clip_box(contents: u32) -> World {
    let mut world = World::default();
    world.brushes.push(Brush {
        contents,
        planes: [
            (Vec3::X, 20.),
            (-Vec3::X, -10.),
            (Vec3::Y, 8.),
            (-Vec3::Y, 8.),
            (Vec3::Z, 100.),
            (-Vec3::Z, 0.),
        ]
        .into_iter()
        .map(|(normal, distance)| modkit_core::Plane { normal, distance })
        .collect(),
    });
    world
}
#[test]
fn npc_only_clip_does_not_block_player_sweeps_or_overlap_queries() {
    // The installed station uses DETAIL|MONSTERCLIP boxes around benches.
    let physics = Physics::new(&clip_box(0x8020000));
    let mins = Vec3::new(-1., -1., 0.);
    let maxs = Vec3::new(1., 1., 2.);
    let sweep = physics.trace_hull(Vec3::new(-5., 0., 40.), Vec3::new(35., 0., 40.), mins, maxs);
    assert_eq!(sweep.fraction, 1.);
    assert!(!sweep.start_solid);
    let inside = Vec3::new(15., 0., 40.);
    assert!(!physics.trace_hull(inside, inside, mins, maxs).start_solid);
    // Shots and use traces (MASK_SHOT / MASK_SOLID) pass clip-only brushes; the
    // player's ground trace (MASK_PLAYERSOLID) still finds them.
    assert!(physics
        .impact_ray(Vec3::new(-5., 0., 40.), Vec3::X, 40.)
        .is_none());
    assert!(physics
        .ground_ray(Vec3::new(15., 0., 140.), -Vec3::Z, 60.)
        .is_some());
    assert_eq!(physics.colliders.len(), 1);
}
#[test]
fn physics_objects_pass_clip_only_brushes_but_rest_on_solid_ones() {
    // CCollisionEvent::ShouldCollide: world contents vs MASK_SOLID.
    for (contents, rests) in [(0x8020000, false), (0x10000, false), (1, true)] {
        let mut physics = Physics::new(&clip_box(contents));
        let body = physics
            .bodies
            .insert(RigidBodyBuilder::dynamic().translation(vector![
                15. * SCALE,
                0.,
                110. * SCALE
            ]));
        physics.colliders.insert_with_parent(
            ColliderBuilder::ball(4. * SCALE).density(100.),
            body,
            &mut physics.bodies,
        );
        for _ in 0..80 {
            physics.tick(0.015);
        }
        let z = physics.bodies[body].translation().z / SCALE;
        assert_eq!(z > 100., rests, "contents {contents:#x}: z {z}");
    }
}
#[test]
fn player_clip_solid_and_mixed_monster_solid_still_block_players() {
    let mins = Vec3::new(-1., -1., 0.);
    let maxs = Vec3::new(1., 1., 2.);
    for contents in [1, 0x10000, 1 | 0x20000] {
        let physics = Physics::new(&clip_box(contents));
        let sweep =
            physics.trace_hull(Vec3::new(-5., 0., 40.), Vec3::new(35., 0., 40.), mins, maxs);
        assert!(sweep.fraction < 1., "contents {contents:#x}");
        let inside = Vec3::new(15., 0., 40.);
        assert!(
            physics.trace_hull(inside, inside, mins, maxs).start_solid,
            "contents {contents:#x}"
        );
    }
}
#[test]
fn melee_box_sweep_reaches_a_nearby_offset_target_but_not_outside_its_width() {
    let mut physics = Physics::default();
    physics.colliders.insert(
        ColliderBuilder::cuboid(5. * SCALE, 5. * SCALE, 5. * SCALE)
            .translation(vector![40. * SCALE, 20. * SCALE, 0.])
            .user_data(1),
    );
    physics.query.update(&physics.colliders);
    assert!(physics.impact_ray(Vec3::ZERO, Vec3::X, 75.).is_none());
    assert_eq!(
        physics
            .impact_hull(Vec3::ZERO, Vec3::X, 75. - 1.732 * 16., Vec3::splat(16.))
            .unwrap()
            .entity,
        0
    );
    assert!(physics
        .impact_hull(
            Vec3::new(0., -20., 0.),
            Vec3::X,
            75. - 1.732 * 16.,
            Vec3::splat(16.)
        )
        .is_none());
}
#[test]
fn touching_floor_allows_standing_but_penetration_does_not() {
    let mut p = Physics::default();
    p.colliders
        .insert(ColliderBuilder::cuboid(10., 10., 1.).translation(vector![0., 0., -1.]));
    p.query.update(&p.colliders);
    let mins = Vec3::new(-16., -16., 0.);
    let maxs = Vec3::new(16., 16., 72.);
    assert!(!p.trace_hull(Vec3::ZERO, Vec3::ZERO, mins, maxs).start_solid);
    let below = -Vec3::Z;
    assert!(p.trace_hull(below, below, mins, maxs).start_solid);
}
#[test]
fn killed_dynamic_entity_no_longer_blocks_raycasts() {
    let mut p = Physics::default();
    let body = p.bodies.insert(RigidBodyBuilder::dynamic());
    let c =
        p.colliders
            .insert_with_parent(ColliderBuilder::ball(1.).user_data(1), body, &mut p.bodies);
    p.dynamic.insert(0, body);
    p.entity_colliders.insert(0, vec![c]);
    p.tick(0.015);
    assert!(p.ray(Vec3::new(-100., 0., 0.), Vec3::X, 200.).is_some());
    p.set_entity(0, Vec3::ZERO, Quat::IDENTITY, false);
    p.tick(0.015);
    assert!(p.ray(Vec3::new(-100., 0., 0.), Vec3::X, 200.).is_none());
}
pub(super) fn npc_config() -> crate::npc_probe::GroundConfig {
    crate::npc_probe::GroundConfig {
        hull: Hull {
            mins: Vec3::new(-13., -13., 0.),
            maxs: Vec3::new(13., 13., 72.),
        },
        step_height: 18.,
        step_down_multiplier: 1.,
    }
}
#[test]
fn npc_clip_masks_are_distinct_from_player_clip_and_preserve_player_queries() {
    for (contents, npc_blocked, player_blocked) in [
        (0x20000, true, false),
        (0x10000, false, true),
        (1, true, true),
    ] {
        let mut brush = bounds_brush(Vec3::new(25., -100., 0.), Vec3::new(35., 100., 100.));
        brush.contents = contents;
        let physics = Physics::new(&World {
            brushes: vec![brush],
            ..Default::default()
        });
        let hull = npc_config().hull;
        let npc = physics.npc_trace_hull(Vec3::ZERO, Vec3::X * 60., hull, Query::default());
        let player = physics.trace_hull(Vec3::ZERO, Vec3::X * 60., hull.mins, hull.maxs);
        assert_eq!(
            npc.trace.fraction < 1.,
            npc_blocked,
            "contents {contents:x}"
        );
        assert_eq!(
            player.fraction < 1.,
            player_blocked,
            "contents {contents:x}"
        );
    }
}
#[test]
fn npc_transient_hulls_exclude_self_but_block_on_player_and_other_actors() {
    use crate::npc_probe::NPC_BRUSH_ONLY_MASK;
    let physics = Physics::default();
    let hull = npc_config().hull;
    let actors = [
        ActorHull {
            entity: Some(7),
            feet: Vec3::ZERO,
            hull,
        },
        ActorHull {
            entity: Some(8),
            feet: Vec3::X * 60.,
            hull,
        },
    ];
    let excluded = [7];
    let query = Query {
        excluded_entities: &excluded,
        transients: &actors,
        ..Default::default()
    };
    let hit = physics.npc_trace_hull(Vec3::ZERO, Vec3::X * 100., hull, query);
    assert!(!hit.trace.start_solid);
    assert_eq!(hit.entity, Some(8));
    assert!(hit.transient && hit.trace.fraction < 1.);
    assert!(physics
        .npc_trace_hull(
            Vec3::ZERO,
            Vec3::X * 100.,
            hull,
            Query {
                contents_mask: NPC_BRUSH_ONLY_MASK,
                ..query
            }
        )
        .clear_path());
    let player = [ActorHull {
        entity: None,
        feet: Vec3::X * 60.,
        hull,
    }];
    let hit = physics.npc_trace_hull(
        Vec3::ZERO,
        Vec3::X * 100.,
        hull,
        Query {
            transients: &player,
            ..Default::default()
        },
    );
    assert!(hit.transient && hit.entity.is_none() && hit.trace.fraction < 1.);
    let inside = physics.npc_trace_hull(
        Vec3::ZERO,
        Vec3::ZERO,
        hull,
        Query {
            transients: &actors,
            ..Default::default()
        },
    );
    assert!(inside.trace.start_solid && inside.trace.all_solid);
    assert_eq!(inside.entity, Some(7));
}
#[test]
fn bullets_pass_grate_fences_that_other_traces_hit() {
    // A func_brush chain-link fence (d1_canals_01 *21: CONTENTS_GRATE | TRANSLUCENT).
    let mut fence = bounds_brush(Vec3::new(0., -100., 0.), Vec3::new(4., 100., 100.));
    fence.contents = 0x1000_0008;
    let world = World {
        entities: vec![modkit_core::Entity {
            properties: vec![
                ("classname".into(), "func_brush".into()),
                ("model".into(), "*1".into()),
            ],
        }],
        brush_models: vec![modkit_core::BrushModel {
            id: 1,
            brushes: vec![fence],
            ..Default::default()
        }],
        ..Default::default()
    };
    let physics = Physics::new(&world);
    let start = Vec3::new(-40., 0., 50.);
    assert_eq!(
        physics.impact_ray(start, Vec3::X, 80.).map(|h| h.entity),
        Some(0)
    );
    assert!(physics.bullet_ray(start, Vec3::X, 80., &[]).is_none());
    // A solid brush entity still stops bullets.
    let mut world = world;
    world.brush_models[0].brushes[0].contents = 1;
    let physics = Physics::new(&world);
    assert!(physics.bullet_ray(start, Vec3::X, 80., &[]).is_some());
}
#[test]
fn npc_brush_door_hits_have_entity_and_follow_pose_before_simulation_tick() {
    let world = World {
        entities: vec![modkit_core::Entity {
            properties: vec![
                ("classname".into(), "func_door".into()),
                ("model".into(), "*1".into()),
            ],
        }],
        brush_models: vec![modkit_core::BrushModel {
            id: 1,
            brushes: vec![bounds_brush(
                Vec3::new(0., -100., 0.),
                Vec3::new(4., 100., 100.),
            )],
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut physics = Physics::new(&world);
    let hull = npc_config().hull;
    let start = Vec3::new(-40., 0., 0.);
    let end = Vec3::new(40., 0., 0.);
    let hit = physics.npc_trace_hull(start, end, hull, Query::default());
    assert_eq!(hit.entity, Some(0));
    assert!(hit.trace.fraction < 1. && hit.trace.normal.x < -0.99);
    let excluded = [0];
    assert!(physics
        .npc_trace_hull(
            start,
            end,
            hull,
            Query {
                excluded_entities: &excluded,
                ..Default::default()
            }
        )
        .clear_path());
    physics.set_entity(0, Vec3::X * 200., Quat::IDENTITY, true);
    physics.refresh_queries();
    assert!(physics
        .npc_trace_hull(start, end, hull, Query::default())
        .clear_path());
    physics.set_entity(0, Vec3::ZERO, Quat::IDENTITY, true);
    physics.refresh_queries();
    assert!(
        (physics
            .npc_trace_hull(start, end, hull, Query::default())
            .trace
            .fraction
            - hit.trace.fraction)
            .abs()
            < 1e-5
    );
}
#[test]
fn npc_point_visibility_and_model_door_follow_rotation_and_disabled_state() {
    let mut physics = Physics::new(&door_world());
    let start = Vec3::new(-40., 24., 0.);
    let end = Vec3::new(40., 24., 0.);
    let point = Hull {
        mins: Vec3::ZERO,
        maxs: Vec3::ZERO,
    };
    let query = Query {
        contents_mask: crate::npc_probe::NPC_BRUSH_ONLY_MASK,
        ..Default::default()
    };
    let visible = physics.npc_trace_hull(start, end, point, query);
    assert!(visible.trace.fraction < 1.);
    assert_eq!(visible.entity, Some(0));
    let blocked = physics.npc_trace_hull(start, end, npc_config().hull, query);
    assert!(blocked.trace.fraction < visible.trace.fraction);
    physics.set_entity(
        0,
        Vec3::ZERO,
        Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
        true,
    );
    physics.refresh_queries();
    assert!(physics
        .npc_trace_hull(start, end, point, query)
        .clear_path());
    assert!(physics
        .npc_trace_hull(start, end, npc_config().hull, query)
        .clear_path());
    physics.set_entity(0, Vec3::ZERO, Quat::IDENTITY, false);
    physics.refresh_queries();
    assert!(physics
        .npc_trace_hull(start, end, npc_config().hull, query)
        .clear_path());
    physics.set_entity(0, Vec3::ZERO, Quat::IDENTITY, true);
    physics.refresh_queries();
    assert_eq!(
        physics.npc_trace_hull(start, end, point, query).entity,
        Some(0)
    );
}
#[test]
fn npc_stand_and_ground_move_validate_floor_and_do_not_teleport_to_target_z() {
    use crate::npc_probe::{fits, ground_move, stand};
    let physics = Physics::new(&npc_floor_world());
    let config = npc_config();
    assert!(stand(&physics, Vec3::ZERO, config, Query::default()));
    assert!(fits(&physics, Vec3::ZERO, config.hull, Query::default()));
    let movement = ground_move(
        &physics,
        Vec3::ZERO,
        Vec3::new(80., 0., 200.),
        config,
        Query::default(),
    );
    assert!(movement.completed, "{movement:?}");
    assert!((movement.end.x - 80.).abs() < 1e-4);
    assert!(movement.end.z < 0.2 && movement.end.z >= 0.);
    let empty = Physics::default();
    assert!(!stand(&empty, Vec3::ZERO, config, Query::default()));
    assert!(!ground_move(&empty, Vec3::ZERO, Vec3::X * 40., config, Query::default()).completed);
}
#[test]
fn npc_ground_probe_steps_onto_low_step_and_rejects_tall_step_and_wall() {
    use crate::npc_probe::ground_move;
    for (height, expected) in [(12., true), (25., false), (100., false)] {
        let mut world = npc_floor_world();
        world.brushes.push(bounds_brush(
            Vec3::new(20., -100., 0.),
            Vec3::new(150., 100., height),
        ));
        let physics = Physics::new(&world);
        let movement = ground_move(
            &physics,
            Vec3::ZERO,
            Vec3::X * 80.,
            npc_config(),
            Query::default(),
        );
        assert_eq!(
            movement.completed, expected,
            "height {height}: {movement:?}"
        );
        if expected {
            assert!((movement.end.z - height).abs() < 0.2);
        } else {
            assert!(movement.end.x < 7. && movement.end.z < 0.2);
        }
    }
}
#[test]
fn npc_ground_probe_blocks_gap_large_drop_and_low_ceiling() {
    use crate::npc_probe::ground_move;
    for floor in [None, Some(-30.)] {
        let mut world = World {
            brushes: vec![bounds_brush(
                Vec3::new(-100., -100., -20.),
                Vec3::new(20., 100., 0.),
            )],
            ..Default::default()
        };
        if let Some(z) = floor {
            world.brushes.push(bounds_brush(
                Vec3::new(20., -100., z - 10.),
                Vec3::new(200., 100., z),
            ));
        }
        let movement = ground_move(
            &Physics::new(&world),
            Vec3::ZERO,
            Vec3::X * 80.,
            npc_config(),
            Query::default(),
        );
        assert!(!movement.completed && movement.end.x < 40., "{movement:?}");
    }
    let mut world = npc_floor_world();
    world.brushes.push(bounds_brush(
        Vec3::new(20., -100., 0.),
        Vec3::new(150., 100., 12.),
    ));
    world.brushes.push(bounds_brush(
        Vec3::new(0., -100., 76.),
        Vec3::new(150., 100., 90.),
    ));
    let movement = ground_move(
        &Physics::new(&world),
        Vec3::ZERO,
        Vec3::X * 80.,
        npc_config(),
        Query::default(),
    );
    assert!(!movement.completed && movement.end.x < 7., "{movement:?}");
}
#[test]
fn npc_ground_probe_rejects_embedded_nan_and_excessive_travel() {
    use crate::npc_probe::{ground_move, BlockReason};
    let physics = Physics::new(&npc_floor_world());
    for (start, end, reason) in [
        (
            Vec3::new(0., 0., -10.),
            Vec3::X * 50.,
            BlockReason::StartSolid,
        ),
        (
            Vec3::ZERO,
            Vec3::new(f32::NAN, 0., 0.),
            BlockReason::InvalidInput,
        ),
        (Vec3::ZERO, Vec3::X * 10000., BlockReason::TravelBudget),
    ] {
        let movement = ground_move(&physics, start, end, npc_config(), Query::default());
        assert!(!movement.completed);
        assert_eq!(movement.reason, Some(reason));
        assert_eq!(movement.end, start);
    }
}
/// A free box resting on a floor plate (synthetic values, not game data).
fn shadow_scene(mass: f32, gravity: bool, z: f32) -> (Physics, RigidBodyHandle) {
    let mut physics = Physics::new(&World::default());
    physics.colliders.insert(
        ColliderBuilder::cuboid(2000. / 39.37, 2000. / 39.37, 8. / 39.37)
            .translation(vector![0., 0., -8. / 39.37])
            .friction(0.8),
    );
    let body = physics.bodies.insert(
        RigidBodyBuilder::dynamic()
            .translation(vector![40. / 39.37, 0., z / 39.37])
            .gravity_scale(if gravity { 1. } else { 0. })
            .additional_mass(mass),
    );
    physics.colliders.insert_with_parent(
        ColliderBuilder::cuboid(8. / 39.37, 8. / 39.37, 8. / 39.37)
            .density(0.)
            .friction(0.8),
        body,
        &mut physics.bodies,
    );
    physics.refresh_queries();
    (physics, body)
}
#[test]
fn stacked_props_keep_one_margin_like_a_prop_on_the_floor() {
    // Two 16-unit boxes with the prop skin dropped onto a floor plate (synthetic).
    let mut physics = Physics::new(&World::default());
    physics.colliders.insert(
        ColliderBuilder::cuboid(2000. * SCALE, 2000. * SCALE, 8. * SCALE).translation(vector![
            0.,
            0.,
            -8. * SCALE
        ]),
    );
    let boxes = [9., 26.].map(|z| {
        let body = physics
            .bodies
            .insert(RigidBodyBuilder::dynamic().translation(vector![0., 0., z * SCALE]));
        physics.colliders.insert_with_parent(
            ColliderBuilder::cuboid(8. * SCALE, 8. * SCALE, 8. * SCALE)
                .contact_skin(VPHYSICS_CONTACT_MARGIN * SCALE)
                .active_hooks(ActiveHooks::MODIFY_SOLVER_CONTACTS),
            body,
            &mut physics.bodies,
        );
        body
    });
    for _ in 0..200 {
        physics.tick(TICK);
    }
    let z = boxes.map(|b| physics.bodies[b].translation().z / SCALE);
    let floor_gap = z[0] - 8.;
    let stack_gap = z[1] - z[0] - 16.;
    assert!(
        (floor_gap - 0.5).abs() < 0.1,
        "floor gap {floor_gap} stack {stack_gap}"
    );
    assert!((stack_gap - 0.5).abs() < 0.1, "stack gap {stack_gap}");
}
#[test]
fn player_shadow_shoves_light_props_but_not_heavy_ones() {
    for (mass, moves) in [(20., true), (500., false)] {
        let (mut physics, body) = shadow_scene(mass, true, 8.);
        // The player stands against the box (hull edge x 16, box edge x 32) and
        // keeps walking into it; movement leaves the feet where they are.
        let feet = Vec3::new(15.9, 0., 0.);
        for _ in 0..40 {
            physics.update_player_shadow(
                feet,
                feet,
                false,
                true,
                0.,
                Vec3::new(190., 0., 0.),
                None,
                TICK,
            );
            physics.tick(TICK);
        }
        let x = physics.bodies[body].translation().x * 39.37;
        assert_eq!(x > 41., moves, "{mass} kg box at x {x}");
        // The shadow follows the player and never wanders into the world.
        let (shadow, _) = physics.player_shadow.unwrap();
        let s = *physics.bodies[shadow].translation() * 39.37;
        assert!((s.z - 36.).abs() < 1. && s.x < 20., "{s:?}");
    }
}
#[test]
fn player_shadow_does_not_push_the_held_object_or_sink_the_prop_it_stands_on() {
    let (mut physics, body) = shadow_scene(20., true, 8.);
    physics.dynamic.insert(7, body);
    let feet = Vec3::new(15.9, 0., 0.);
    for _ in 0..40 {
        physics.update_player_shadow(
            feet,
            feet,
            false,
            true,
            0.,
            Vec3::new(190., 0., 0.),
            Some(7),
            TICK,
        );
        physics.tick(TICK);
    }
    assert!(physics.bodies[body].translation().x * 39.37 < 40.5);
    // Landing on and standing on a light box on the floor: the shadow neither rams
    // it into the floor nor lifts it into the player's hull (owner report: stuck on
    // top of props). Feet fall onto the box top (z 16) and stay there.
    let (mut physics, body) = shadow_scene(5., true, 8.);
    for tick in 0..80 {
        let z = (40. - tick as f32 * 6.).max(16.);
        let feet = Vec3::new(40., 0., z);
        let old = Vec3::new(40., 0., (46. - tick as f32 * 6.).max(16.));
        physics.update_player_shadow(old, feet, false, z <= 16., 0., Vec3::ZERO, None, TICK);
        physics.tick(TICK);
    }
    let z = physics.bodies[body].translation().z * 39.37;
    assert!((z - 8.).abs() < 0.2, "box sank or rose to z {z}");
    let feet = Vec3::new(40., 0., 16.);
    assert!(
        physics.hull_contacts(feet, 72.).iter().all(|c| c.2 < 0.05),
        "{:?}",
        physics.hull_contacts(feet, 72.)
    );
}
pub(super) fn npc_floor_world() -> World {
    World {
        brushes: vec![bounds_brush(
            Vec3::new(-200., -200., -20.),
            Vec3::new(200., 200., 0.),
        )],
        ..Default::default()
    }
}

#[test]
fn a_barnacle_body_is_a_small_box_under_its_origin_not_its_tongue() {
    let (mins, maxs) = npc_hull("npc_barnacle").unwrap();
    assert_eq!((mins.z, maxs.z), (-40., 0.));
    assert_eq!((mins.x, maxs.x), (-16., 16.));
}
