use super::super::tests::{cube, key, make_controller, pose, request, world};
use super::super::{graph::search, GoalState, NavRestrictions};
use super::*;
use crate::physics::Physics;
use modkit_core::{animation::MovementRecord, World};
use source_assets::navigation::Node;

const LIMITS: JumpLimits = JumpLimits::FAST_ZOMBIE;

/// `frames` at 30 fps at a constant speed (a record's v0 and v1 are units over the record).
fn rising(frames: u32, end: Vec3) -> RootMotion {
    RootMotion {
        fps: 30.,
        frame_count: frames,
        records: vec![MovementRecord {
            end_frame: (frames - 1) as _,
            motion_flags: 1,
            v0: end.length(),
            v1: end.length(),
            end_yaw_degrees: 0.,
            direction: end.normalize_or_zero(),
            cumulative_position: end,
        }],
    }
}
fn caps() -> MoveCaps {
    let clip = |name: &str, track| Clip {
        name: name.to_owned(),
        track,
    };
    MoveCaps {
        jump: Some((LIMITS, "jumpnavmove".into())),
        climb: Some(Arc::new(ClimbClips {
            up: clip("climbloop", rising(15, Vec3::Z * 45.)),
            dismount: Some(clip("climbdismount", rising(53, Vec3::new(53., 0., 106.)))),
        })),
    }
}
fn node(origin: Vec3, node_type: u8, info: u16) -> Node {
    Node {
        origin,
        yaw: 90.,
        hull_z_offsets: [0.; 10],
        node_type,
        info,
        zone: 0,
    }
}
fn link(source: usize, destination: usize, types: u8) -> Link {
    Link {
        source,
        destination,
        accepted_move_types: [types; 10],
    }
}
fn graph(nodes: Vec<Node>, links: Vec<Link>) -> Graph {
    Graph {
        version: 37,
        map_revision: 1,
        hammer_ids: (0..nodes.len() as i32).collect(),
        nodes,
        links,
        trailing_bytes: vec![],
    }
}
/// Two ledges joined only by a jump link, with ground nodes at both ends.
fn gap_graph() -> Graph {
    let at = |x: f32| node(Vec3::new(x, 0., 0.), 2, 0);
    graph(
        vec![at(-200.), at(-100.), at(100.), at(200.)],
        vec![link(0, 1, 1), link(1, 2, 2), link(2, 3, 1)],
    )
}
/// A ladder at x = 0 from the floor to z = 300 with a ledge behind its top: floor node 0, mount
/// node 1 and climb nodes 2, 3, the dismount node 4 and the ledge node 5.
fn ladder_graph() -> Graph {
    let climb = |z: f32, info| node(Vec3::new(0., 0., z), 4, info);
    graph(
        vec![
            node(Vec3::new(-100., 0., 0.), 2, 0),
            climb(0., 8),
            climb(0., 2),
            climb(300., 2),
            climb(300., 4),
            node(Vec3::new(80., 0., 300.), 2, 0),
        ],
        vec![
            link(0, 1, 1),
            link(1, 2, 8),
            link(2, 3, 8),
            link(3, 4, 8),
            link(4, 5, 1),
        ],
    )
}
/// A shelf at the ladder's top; the floor below stays open.
fn ladder_world() -> World {
    world(vec![cube(
        Vec3::new(-20., -100., 280.),
        Vec3::new(200., 100., 300.),
    )])
}
/// A wall across the gap, `height` high.
fn gap_world(height: f32) -> World {
    world(vec![cube(
        Vec3::new(-8., -1000., 0.),
        Vec3::new(8., 1000., height),
    )])
}

#[test]
fn jump_limits_are_the_fast_zombies() {
    let start = Vec3::ZERO;
    let end = |x: f32, z: f32| Vec3::new(x, 0., z);
    assert!(LIMITS.legal(start, end(100., 220.), end(100., 220.)));
    assert!(!LIMITS.legal(start, end(100., 221.), end(100., 221.)));
    assert!(LIMITS.legal(start, end(100., 0.), end(100., -384.)));
    assert!(!LIMITS.legal(start, end(100., 0.), end(100., -385.)));
    assert!(!LIMITS.legal(start, end(513., 0.), end(513., 0.)));
    // The apex may rise a quarter above the highest legal step.
    assert!(LIMITS.legal(start, end(50., 275.), end(100., 0.)));
    assert!(!LIMITS.legal(start, end(50., 276.), end(100., 0.)));
}

#[test]
fn links_cost_what_the_fast_zombie_prefers() {
    let caps = caps();
    let scale = |types, from: f32, to: f32| {
        let (a, b) = (Vec3::Z * from, Vec3::new(10., 0., to));
        caps.link_scale(&link(0, 1, types), HullType::Human, a, b)
    };
    assert_eq!(scale(1, 0., 0.), Some(1.));
    assert_eq!(scale(2, 100., 0.), Some(1.), "a jump down");
    assert_eq!(scale(2, 0., 100.), Some(3.), "a jump up");
    assert_eq!(scale(8, 0., 100.), Some(1.), "a climb up");
    assert_eq!(scale(8, 100., 0.), Some(8.), "a climb down");
    assert_eq!(scale(2, 0., 300.), None, "too high to jump");
    // A ground mover takes neither.
    let ground = MoveCaps::default();
    assert_eq!(ground.link_nav(&link(0, 1, 2), HullType::Human), None);
    assert_eq!(ground.link_nav(&link(0, 1, 8), HullType::Human), None);
}

#[test]
fn a_jump_arc_reaches_its_target_within_the_top_speed() {
    for to in [
        Vec3::new(300., 0., 100.),
        Vec3::new(0., 400., -300.),
        Vec3::new(100., 100., 0.),
    ] {
        let arc = jump_arc(Vec3::ZERO, to, 0.);
        // The top speed sizes the arc of a level jump; a step up can need more.
        if to.z == 0. {
            assert!(arc.velocity.truncate().length() <= MAX_JUMP_SPEED + 0.01);
        }
        assert!(arc.height >= to.z);
        let up = arc.velocity.z / JUMP_GRAVITY;
        let down = (2. * (arc.height - to.z) / JUMP_GRAVITY).sqrt();
        let landing = (arc.velocity * (up + down)).truncate();
        assert!((landing - to.truncate()).length() < 0.01, "{landing:?}");
        assert!((arc.apex.z - arc.height).abs() < 0.01);
    }
}

#[test]
fn the_jump_probe_clears_what_it_can_and_fails_on_what_it_cannot() {
    let hull = HullType::Human.bounds();
    let ends = (Vec3::new(-150., 0., 0.), Vec3::new(150., 0., 0.));
    let open = Physics::new(&world(vec![]));
    let query = Query::default();
    let low = jump_velocity(&open, hull, query, LIMITS, ends).unwrap();
    // A low wall in the way needs a higher arc than the open floor.
    let wall = |height| cube(Vec3::new(-8., -300., 0.), Vec3::new(8., 300., height));
    let hurdle = Physics::new(&world(vec![wall(80.)]));
    let high = jump_velocity(&hurdle, hull, query, LIMITS, ends).unwrap();
    assert!(high.z > low.z, "{high:?} {low:?}");
    // One above the apex limit stays blocked.
    let tall = Physics::new(&world(vec![wall(400.)]));
    assert!(jump_velocity(&tall, hull, query, LIMITS, ends).is_err());
    // Straight up is no jump.
    let same = (Vec3::ZERO, Vec3::Z * 100.);
    assert!(jump_velocity(&open, hull, query, LIMITS, same).is_err());
}

#[test]
fn only_a_mover_with_the_caps_routes_over_jump_and_climb_links() {
    let (gap, hull) = (gap_graph(), HullType::Human);
    let restrictions = NavRestrictions::default();
    let cost = |_: usize, a: Vec3, b: Vec3| Some(a.distance(b));
    let ground = MoveCaps::default();
    assert!(search(&gap, &restrictions, (hull, &ground), 0, 3, cost).is_err());
    let caps = caps();
    let nodes = search(&gap, &restrictions, (hull, &caps), 0, 3, cost).unwrap();
    assert_eq!(nodes, [0, 1, 2, 3]);
    let navs = route_navs(&gap, &nodes, &caps, hull);
    assert_eq!(navs[1], Nav::Ground);
    assert_eq!(navs[2], Nav::Jump);
    assert_eq!(navs[3], Nav::Ground);
    // Climb nodes count for climbers only.
    let ladder = ladder_graph();
    assert!(search(&ladder, &restrictions, (hull, &ground), 0, 5, cost).is_err());
    let nodes = search(&ladder, &restrictions, (hull, &caps), 0, 5, cost).unwrap();
    assert_eq!(nodes, [0, 1, 2, 3, 4, 5]);
    let navs = route_navs(&ladder, &nodes, &caps, hull);
    assert_eq!(navs[3], Nav::Climb { yaw: 90. });
    assert_eq!(navs[5], Nav::Ground);
}

#[test]
fn a_jump_hands_the_actor_over_and_the_route_resumes_at_the_landing() {
    let physics = Physics::new(&gap_world(40.));
    let mut controller = make_controller(Some(gap_graph()));
    controller.set_move_caps(7, caps());
    let mut feet = Vec3::new(-200., 0., 0.);
    let mut wanted = request(Vec3::new(200., 0., 0.));
    wanted.force_short = true;
    controller.request(key(1), wanted.clone(), pose(feet), &physics, &[]);
    let mut launch = None;
    for _ in 0..300 {
        let updates = controller.tick(&physics, &[pose(feet)], &[], 0.02, false);
        feet = updates.last().unwrap().feet;
        launch = controller.take_launches().pop();
        if launch.is_some() {
            break;
        }
    }
    let (entity, velocity) = launch.expect("the jump launches");
    assert_eq!(entity, 7);
    assert!(velocity.x > 0. && velocity.z > 0.);
    assert!(
        (feet.x + 100.).abs() < 1.,
        "launched from the ledge: {feet:?}"
    );
    assert_eq!(controller.jumpers(), [7]);
    // In the air: the pose belongs to the fall, the sequence to the jump, and a new request
    // waits for the landing.
    let air = Vec3::new(0., 0., 60.);
    let updates = controller.tick(&physics, &[pose(air)], &[], 0.02, false);
    assert_eq!(updates[0].feet, air);
    assert_eq!(updates[0].sequence.as_deref(), Some("jumpnavmove"));
    wanted.target_feet = Vec3::new(150., 0., 0.);
    let state = controller.request(key(1), wanted, pose(air), &physics, &[]);
    assert_eq!(state, GoalState::Moving);
    let land = Vec3::new(100., 0., 0.);
    controller.landed(7);
    assert!(controller.jumpers().is_empty());
    let updates = controller.tick(&physics, &[pose(land)], &[], 0.02, false);
    assert!(updates[0].state.is_moving(), "{:?}", updates[0].state);
    assert_eq!(controller.snapshots()[0].target_feet.x, 150.);
}

#[test]
fn a_blocked_jump_fails_the_link() {
    let physics = Physics::new(&gap_world(400.));
    let mut controller = make_controller(Some(gap_graph()));
    controller.set_move_caps(7, caps());
    let mut feet = Vec3::new(-110., 0., 0.);
    let mut wanted = request(Vec3::new(200., 0., 0.));
    wanted.force_short = true;
    controller.request(key(1), wanted, pose(feet), &physics, &[]);
    for _ in 0..100 {
        let updates = controller.tick(&physics, &[pose(feet)], &[], 0.02, false);
        feet = updates.last().unwrap().feet;
    }
    assert!(controller.take_launches().is_empty());
    assert!(controller.path_stats().jump_failures >= 1);
    assert!(matches!(
        controller.status(key(1)),
        Some(GoalState::Blocked(_))
    ));
}

#[test]
fn a_climb_runs_on_the_sequences_and_dismounts_at_the_top() {
    let physics = Physics::new(&ladder_world());
    let mut controller = make_controller(Some(ladder_graph()));
    controller.set_move_caps(7, caps());
    let mut feet = Vec3::new(-100., 0., 0.);
    let mut wanted = request(Vec3::new(80., 0., 300.));
    wanted.force_short = true;
    let state = controller.request(key(1), wanted, pose(feet), &physics, &[]);
    assert_eq!(state, GoalState::Moving);
    let (mut sequences, mut peak_speed, mut last_z) = (Vec::new(), 0_f32, 0.);
    for _ in 0..800 {
        let updates = controller.tick(&physics, &[pose(feet)], &[], 0.02, false);
        let update = updates.last().unwrap();
        feet = update.feet;
        let name = update.sequence.clone().unwrap_or_default();
        if sequences.last() != Some(&name) {
            sequences.push(name.clone());
        }
        if name == "climbloop" {
            peak_speed = peak_speed.max((feet.z - last_z) / 0.02);
            assert_eq!(update.yaw_degrees, 90.);
            assert_eq!(controller.climbers(), [(7, false)]);
        }
        last_z = feet.z;
        if update.state.is_arrived() {
            break;
        }
    }
    assert!(controller.climbers().is_empty());
    assert_eq!(
        controller.status(key(1)),
        Some(&GoalState::Arrived),
        "{feet:?} {sequences:?}"
    );
    let order: Vec<_> = sequences
        .iter()
        .filter(|s| s.starts_with("climb"))
        .map(String::as_str)
        .collect();
    assert_eq!(order, ["climbloop", "climbdismount"]);
    // climbloop rises 45 units in 14 frames at 30 fps.
    assert!((peak_speed - 96.4).abs() < 2., "{peak_speed}");
    assert!(controller.path_stats().climbs == 1);
}

#[test]
fn a_stopped_climb_lets_go_and_a_request_waits_for_the_top() {
    let physics = Physics::new(&ladder_world());
    let mut controller = make_controller(Some(ladder_graph()));
    controller.set_move_caps(7, caps());
    let mut feet = Vec3::new(-100., 0., 0.);
    let mut wanted = request(Vec3::new(80., 0., 300.));
    wanted.force_short = true;
    controller.request(key(1), wanted.clone(), pose(feet), &physics, &[]);
    while feet.z < 100. {
        let updates = controller.tick(&physics, &[pose(feet)], &[], 0.02, false);
        feet = updates.last().unwrap().feet;
    }
    assert_eq!(controller.climbers().len(), 1);
    // A new goal now does not take it off the ladder.
    let state = controller.request(key(1), wanted, pose(feet), &physics, &[]);
    assert_eq!(state, GoalState::Moving);
    assert_eq!(controller.climbers().len(), 1);
    assert!(controller.take_drops().is_empty());
    controller.cancel(key(1));
    assert_eq!(controller.take_drops(), [7]);
    assert!(controller.climbers().is_empty());
}

#[test]
fn ground_routes_are_untouched() {
    let physics = Physics::new(&world(vec![]));
    let mut controller = make_controller(Some(gap_graph()));
    controller.set_move_caps(7, caps());
    let mut feet = Vec3::ZERO;
    controller.request(
        key(1),
        request(Vec3::new(30., 0., 0.)),
        pose(feet),
        &physics,
        &[],
    );
    for _ in 0..100 {
        let updates = controller.tick(&physics, &[pose(feet)], &[], 0.02, false);
        if let Some(update) = updates.last() {
            feet = update.feet;
        }
    }
    assert!(feet.x > 10.);
    assert_eq!(controller.path_stats().jumps, 0);
}
