use super::*;
use crate::ai::bird::tests::FakeMotor;

fn npc(state: NpcState, conditions: &[Cond]) -> AiNpc {
    let mut npc = AiNpc::new(1);
    npc.state = state;
    npc.conditions = Conditions::of(conditions);
    npc
}
fn hunting(distance: f32, now: f64) -> Context {
    Context {
        now,
        enemy: Some((0, Vec3::X * distance)),
        ..Default::default()
    }
}
fn start(scanner: &mut Scanner, task: &str, number: f32, motor: &mut FakeMotor) -> TaskStatus {
    let mut body = npc(NpcState::Idle, &[]);
    let argument = TaskArg::Number(number);
    let ctx = hunting(100., 1.);
    scanner
        .start_task(&mut body, task, &argument, &ctx, motor)
        .expect("a scanner task")
}

#[test]
fn alone_it_patrols() {
    let scanner = Scanner::default();
    let plan = scanner.plan(&npc(NpcState::Idle, &[]), &Context::default(), 1., 0.5);
    assert_eq!(plan, Plan::Patrol);
}

#[test]
fn a_script_owns_it_before_anything_else() {
    let scanner = Scanner::default();
    let scripted = npc(NpcState::Script, &[COND_LIGHT_DAMAGE]);
    let plan = scanner.plan(&scripted, &hunting(100., 1.), 1., 0.);
    assert_eq!(plan, Plan::Script);
}

#[test]
fn a_hit_flinches_or_sends_it_to_cover_by_state_and_health() {
    let s = Scanner::default();
    let ctx = Context::default();
    let hit = |state| npc(state, &[COND_LIGHT_DAMAGE]);
    assert_eq!(s.plan(&hit(NpcState::Idle), &ctx, 0.1, 0.9), Plan::Flinch);
    assert_eq!(
        s.plan(&hit(NpcState::Alert), &ctx, 0.5, 0.9),
        Plan::TakeCover
    );
    assert_eq!(s.plan(&hit(NpcState::Combat), &ctx, 1., 0.1), Plan::Flinch);
    assert_eq!(s.plan(&hit(NpcState::Combat), &ctx, 1., 0.9), Plan::Patrol);
}

#[test]
fn an_alert_scanner_in_good_health_has_no_flinch_to_play() {
    let s = Scanner::default();
    let hit = npc(NpcState::Alert, &[COND_LIGHT_DAMAGE]);
    assert_eq!(s.plan(&hit, &Context::default(), 0.9, 0.), Plan::Patrol);
}

#[test]
fn a_combat_flinch_is_four_in_eleven() {
    let s = Scanner::default();
    let hit = npc(NpcState::Combat, &[COND_LIGHT_DAMAGE]);
    let ctx = Context::default();
    assert_eq!(s.plan(&hit, &ctx, 1., 0.36), Plan::Flinch);
    assert_eq!(s.plan(&hit, &ctx, 1., 0.37), Plan::Patrol);
}

#[test]
fn it_stays_high_when_far_and_comes_down_when_near() {
    let s = Scanner::default();
    let combat = npc(NpcState::Combat, &[]);
    assert_eq!(
        s.plan(&combat, &hunting(500., 1.), 1., 0.5),
        Plan::SpotlightHover
    );
    assert_eq!(
        s.plan(&combat, &hunting(200., 1.), 1., 0.5),
        Plan::AttackHover
    );
}

#[test]
fn height_does_not_count_toward_the_attack_distance() {
    let s = Scanner::default();
    let combat = npc(NpcState::Combat, &[]);
    let above = Context {
        now: 1.,
        enemy: Some((0, Vec3::new(100., 0., -900.))),
        ..Default::default()
    };
    assert_eq!(s.plan(&combat, &above, 1., 0.5), Plan::AttackHover);
}

#[test]
fn between_attacks_it_only_hovers() {
    let s = Scanner {
        next_attack: 10.,
        ..Default::default()
    };
    let combat = npc(NpcState::Combat, &[]);
    assert_eq!(
        s.plan(&combat, &hunting(100., 5.), 1., 0.5),
        Plan::SpotlightHover
    );
}

#[test]
fn a_blocked_way_is_chased_before_anything_about_attacks() {
    let s = Scanner {
        next_attack: 10.,
        ..Default::default()
    };
    let blocked = npc(NpcState::Combat, &[COND_FLY_BLOCKED]);
    assert_eq!(s.plan(&blocked, &hunting(100., 5.), 1., 0.5), Plan::Chase);
}

#[test]
fn a_lost_enemy_or_the_switch_sends_it_back_on_patrol() {
    let s = Scanner::default();
    let lost = npc(NpcState::Combat, &[COND_LOST_ENEMY]);
    assert_eq!(s.plan(&lost, &hunting(100., 1.), 1., 0.5), Plan::Patrol);
    let told = Scanner {
        ignores_enemies: true,
        ..Default::default()
    };
    let combat = npc(NpcState::Combat, &[]);
    assert_eq!(
        told.plan(&combat, &hunting(100., 1.), 1., 0.5),
        Plan::Patrol
    );
}

#[test]
fn the_schedules_parse_and_standing_idle_is_patrolling() {
    let mut schedules = Schedules::default();
    add_schedules(&mut schedules);
    for (name, _) in SCHEDULES {
        assert!(schedules.0.contains_key(*name), "{name}");
    }
    let mut s = Scanner::default();
    let idle = npc(NpcState::Idle, &[]);
    let ctx = Context::default();
    assert_eq!(
        s.translate_schedule("SCHED_IDLE_STAND", &idle, &ctx),
        PATROL
    );
    assert_eq!(s.translate_schedule(CHASE, &idle, &ctx), CHASE);
}

#[test]
fn the_way_to_its_enemy_interrupts_the_schedules_that_listen() {
    let s = Scanner::default();
    let mut hovering = npc(NpcState::Combat, &[]);
    hovering.schedule = Some(ATTACK_HOVER.into());
    assert!(s.custom_interrupts(&hovering).has(COND_FLY_BLOCKED));
    hovering.schedule = Some(CHASE.into());
    assert!(s.custom_interrupts(&hovering).has(COND_FLY_CLEAR));
    hovering.schedule = Some(PATROL.into());
    assert!(!s.custom_interrupts(&hovering).has(COND_FLY_BLOCKED));
}

#[test]
fn a_route_is_flown_point_by_point_and_ends_within_its_tolerance() {
    let mut s = Scanner::new(false, 3);
    s.route = vec![Vec3::X * 100., Vec3::X * 200.];
    assert_eq!(s.next_leg(Vec3::ZERO), Some(Vec3::X * 100.));
    assert_eq!(s.next_leg(Vec3::X * 40.), Some(Vec3::X * 200.));
    assert_eq!(s.next_leg(Vec3::X * 160.), Some(Vec3::X * 200.));
    assert_eq!(s.next_leg(Vec3::X * 170.), None);
    assert!(s.route.is_empty());
}

#[test]
fn it_hovers_short_of_its_enemy_and_above_it() {
    let at = Vec3::new(1000., 0., 0.);
    let mut s = Scanner {
        enemy_at: Some(Vec3::ZERO),
        mode: Mode::Spot,
        ..Default::default()
    };
    assert_eq!(s.hover_point(at), Some(Vec3::new(160., 0., 64.)));
    s.mode = Mode::Attack;
    assert_eq!(s.hover_point(at), Some(Vec3::new(225., 0., 150.)));
    s.mode = Mode::Photo;
    assert_eq!(s.hover_point(at), Some(Vec3::new(96., 0., 32.)));
    s.mode = Mode::Follow;
    assert_eq!(s.hover_point(at), Some(Vec3::new(128., 0., 64.)));
    s.mode = Mode::Patrol;
    assert_eq!(s.hover_point(at), None);
}

#[test]
fn a_suit_brings_it_closer() {
    let at = Vec3::new(1000., 0., 0.);
    let mut s = Scanner {
        enemy_at: Some(Vec3::ZERO),
        enemy_in_suit: true,
        mode: Mode::Spot,
        ..Default::default()
    };
    assert_eq!(s.hover_point(at), Some(Vec3::new(80., 0., 32.)));
    s.mode = Mode::Attack;
    assert_eq!(s.hover_point(at), Some(Vec3::new(112.5, 0., 150.)));
}

#[test]
fn a_distance_override_replaces_every_range() {
    let at = Vec3::new(1000., 0., 0.);
    let mut s = Scanner {
        enemy_at: Some(Vec3::ZERO),
        distance_override: 16.,
        mode: Mode::Follow,
        ..Default::default()
    };
    assert_eq!(s.hover_point(at), Some(Vec3::new(16., 0., 64.)));
    s.mode = Mode::Attack;
    assert_eq!(s.hover_point(at), Some(Vec3::new(16., 0., 150.)));
}

#[test]
fn a_marked_direction_decides_the_side_it_hovers_on() {
    let s = Scanner {
        inspect_at: Some(Vec3::ZERO),
        inspect_facing: Some(Vec3::Y),
        mode: Mode::Follow,
        ..Default::default()
    };
    let point = s.hover_point(Vec3::new(1000., 0., 0.));
    assert_eq!(point, Some(Vec3::new(0., -128., 64.)));
}

#[test]
fn what_it_inspects_comes_before_its_enemy_except_to_attack() {
    let at = Vec3::new(1000., 0., 0.);
    let mut s = Scanner {
        inspect_at: Some(Vec3::Z * 64.),
        enemy_at: Some(Vec3::ZERO),
        mode: Mode::Spot,
        ..Default::default()
    };
    assert_eq!(s.hover_point(at), Some(Vec3::new(160., 0., 128.)));
    s.mode = Mode::Attack;
    assert_eq!(s.hover_point(at), Some(Vec3::new(225., 0., 150.)));
}

#[test]
fn being_told_to_inspect_drops_the_flight_and_raises_the_condition_once() {
    let mut s = Scanner::new(false, 3);
    s.route = vec![Vec3::X];
    s.corner = Some(9);
    s.inspect_target(PLAYER, Mode::Photo, 10.);
    assert!(s.route.is_empty() && s.corner.is_none());
    assert_eq!((s.inspect, s.inspect_end), (Some(PLAYER), 15.));
    let mut seen = Conditions::default();
    s.sense_inspection(&mut seen, false, 10.);
    assert!(seen.has(COND_HAVE_TARGET));
    let mut again = Conditions::default();
    s.sense_inspection(&mut again, false, 10.1);
    assert!(!again.has(COND_HAVE_TARGET));
}

#[test]
fn a_photograph_not_yet_taken_keeps_the_inspection_going() {
    let mut s = Scanner::new(false, 3);
    s.inspect_target(PLAYER, Mode::Photo, 0.);
    let mut seen = Conditions::default();
    s.sense_inspection(&mut seen, false, 100.);
    assert_eq!(s.inspect_end, 105.);
    assert!(!seen.has(COND_INSPECT_DONE));
}

#[test]
fn an_inspection_ends_after_the_photograph_but_a_follow_does_not() {
    let mut s = Scanner::new(false, 3);
    s.inspect_target(PLAYER, Mode::Photo, 0.);
    s.photo_taken = true;
    let mut seen = Conditions::default();
    s.sense_inspection(&mut seen, false, 6.);
    assert!(seen.has(COND_INSPECT_DONE));
    assert_eq!(s.inspect, None);
    s.inspect_target(PLAYER, Mode::Follow, 0.);
    let mut seen = Conditions::default();
    s.sense_inspection(&mut seen, false, 600.);
    assert!(!seen.has(COND_INSPECT_DONE));
    assert_eq!(s.inspect, Some(PLAYER));
}

#[test]
fn with_an_enemy_no_inspection_runs_out() {
    let mut s = Scanner::new(false, 3);
    s.inspect_target(PLAYER, Mode::Spot, 0.);
    let mut seen = Conditions::default();
    s.sense_inspection(&mut seen, true, 600.);
    assert!(!seen.has(COND_INSPECT_DONE));
}

#[test]
fn a_photograph_needs_range_a_slow_body_and_its_aim() {
    let mut s = Scanner::new(false, 3);
    s.inspect_target(PLAYER, Mode::Photo, 0.);
    let target = Vec3::X * 100.;
    assert!(s.photo_placed(Vec3::ZERO, target, 0., 0.));
    assert!(
        !s.photo_placed(Vec3::ZERO, Vec3::X * 200., 0., 0.),
        "too far"
    );
    assert!(
        !s.photo_placed(Vec3::ZERO, Vec3::X * 64., 0., 0.),
        "too near"
    );
    assert!(
        !s.photo_placed(Vec3::ZERO, target, 10., 0.),
        "off to one side"
    );
    s.hover.velocity = Vec3::X * 64.;
    assert!(!s.photo_placed(Vec3::ZERO, target, 0., 0.), "too fast");
}

#[test]
fn what_it_was_told_decides_what_it_does_without_an_enemy() {
    let ctx = Context::default();
    let idle = npc(NpcState::Idle, &[]);
    let mut s = Scanner::new(false, 3);
    s.corner = Some(4);
    assert_eq!(s.plan(&idle, &ctx, 1., 0.5), Plan::Corners);
    s.inspect_target(PLAYER, Mode::Photo, 0.);
    assert_eq!(s.plan(&idle, &ctx, 1., 0.5), Plan::PhotographHover);
    let ready = npc(NpcState::Idle, &[COND_CAN_PHOTOGRAPH]);
    assert_eq!(s.plan(&ready, &ctx, 1., 0.5), Plan::Photograph);
    let blocked = npc(NpcState::Idle, &[COND_FLY_BLOCKED]);
    assert_eq!(s.plan(&blocked, &ctx, 1., 0.5), Plan::MoveToInspect);
    s.inspect_target(PLAYER, Mode::Follow, 0.);
    assert_eq!(s.plan(&idle, &ctx, 1., 0.5), Plan::FollowHover);
    s.inspect_target(PLAYER, Mode::Spot, 0.);
    assert_eq!(s.plan(&idle, &ctx, 1., 0.5), Plan::SpotlightHover);
}

#[test]
fn a_photograph_flashes_once_then_waits_or_ends() {
    let mut s = Scanner::new(false, 3);
    s.inspect_target(PLAYER, Mode::Photo, 0.);
    let mut motor = FakeMotor::default();
    let mut body = npc(NpcState::Idle, &[]);
    let task = "TASK_CSCANNER_PHOTOGRAPH";
    let arg = TaskArg::Number(0.);
    let at = |now| Context {
        now,
        ..Default::default()
    };
    let first = s.start_task(&mut body, task, &arg, &at(1.), &mut motor);
    assert_eq!(first, Some(Running));
    assert!(s.photo_taken);
    assert!(s
        .requests
        .contains(&CopRequest::Output("OnPhotographPlayer")));
    assert!(s.requests.contains(&CopRequest::Sound(TAKE_PHOTO)));
    let held = s.run_task(&mut body, task, &arg, &at(1.05), &mut motor);
    assert_eq!(held, Some(Running));
    s.inspect_end = 100.;
    let pause = s.run_task(&mut body, task, &arg, &at(1.2), &mut motor);
    assert_eq!(pause, Some(Running));
    assert!((6.2..=11.2).contains(&s.photo_wait), "{}", s.photo_wait);
    s.requests.clear();
    let wait = s.photo_wait;
    s.run_task(&mut body, task, &arg, &at(wait), &mut motor);
    assert_eq!(
        s.requests
            .iter()
            .filter(|r| matches!(r, CopRequest::Output(_)))
            .count(),
        1
    );
    s.inspect_end = 0.;
    let end = s.run_task(&mut body, task, &arg, &at(wait + 1.), &mut motor);
    assert_eq!(end, Some(Complete));
    assert_eq!(s.inspect, None);
}

#[test]
fn an_npc_in_a_photograph_fires_the_npc_output() {
    let mut s = Scanner::new(false, 3);
    s.inspect_target(7, Mode::Photo, 0.);
    let mut motor = FakeMotor::default();
    assert_eq!(
        start(&mut s, "TASK_CSCANNER_PHOTOGRAPH", 0., &mut motor),
        Running
    );
    assert!(s.requests.contains(&CopRequest::Output("OnPhotographNPC")));
}

#[test]
fn a_path_keeps_it_flying_until_the_last_corner_is_passed() {
    let mut s = Scanner::new(false, 3);
    s.corner = Some(4);
    let mut motor = FakeMotor::default();
    assert_eq!(
        start(&mut s, "TASK_WAIT_FOR_MOVEMENT", 0., &mut motor),
        Running
    );
    s.corner = None;
    assert_eq!(
        start(&mut s, "TASK_WAIT_FOR_MOVEMENT", 0., &mut motor),
        Complete
    );
}

#[test]
fn clearing_a_follow_goes_back_on_patrol() {
    let mut s = Scanner::new(false, 3);
    s.inspect_target(PLAYER, Mode::Follow, 0.);
    s.clear_follow();
    assert_eq!((s.inspect, s.mode), (None, Mode::Patrol));
}

#[test]
fn killed_high_above_a_distant_enemy_it_dives_at_it() {
    let mut s = Scanner::new(false, 3);
    s.enemy_at = Some(Vec3::ZERO);
    s.route = vec![Vec3::X];
    assert!(s.start_dive(Vec3::new(300., 0., 400.)));
    let dive = s.dive.expect("a dive");
    assert!((dive.direction - Vec3::new(-0.6, 0., -0.8)).length() < 0.001);
    assert!((20. ..=420.).contains(&dive.spin.abs()), "{}", dive.spin);
    assert!(s.route.is_empty());
}

#[test]
fn killed_low_near_or_with_no_enemy_it_does_not_dive() {
    let mut s = Scanner::new(false, 3);
    assert!(!s.start_dive(Vec3::Z * 1000.), "no enemy");
    s.enemy_at = Some(Vec3::ZERO);
    assert!(!s.start_dive(Vec3::new(1000., 0., 100.)), "too low");
    assert!(!s.start_dive(Vec3::new(100., 0., 200.)), "too near");
    assert_eq!(s.dive, None);
}

#[test]
fn a_new_flight_mode_drops_the_route() {
    let mut s = Scanner::new(false, 3);
    s.route = vec![Vec3::X];
    let mut motor = FakeMotor::default();
    let status = start(&mut s, "TASK_CSCANNER_SET_FLY_SPOT", 0., &mut motor);
    assert_eq!(status, Complete);
    assert_eq!(s.mode, Mode::Spot);
    assert!(s.route.is_empty());
}

#[test]
fn a_patrol_with_no_way_fails_and_with_one_waits_for_the_flight() {
    let mut s = Scanner::new(false, 3);
    let mut motor = FakeMotor::default();
    let task = "TASK_GET_PATH_TO_RANDOM_NODE";
    assert_eq!(start(&mut s, task, 2000., &mut motor), Failed);
    motor.legs = Some(vec![Vec3::X * 500.]);
    assert_eq!(start(&mut s, task, 2000., &mut motor), Complete);
    assert_eq!(s.route.len(), 1);
    let wait = "TASK_WAIT_FOR_MOVEMENT";
    assert_eq!(start(&mut s, wait, 0., &mut motor), Running);
    s.route.clear();
    assert_eq!(start(&mut s, wait, 0., &mut motor), Complete);
}

#[test]
fn a_flinch_ends_at_once_and_stops_the_flight_first() {
    let mut s = Scanner::new(false, 3);
    s.route = vec![Vec3::X];
    let mut motor = FakeMotor::default();
    assert_eq!(start(&mut s, "TASK_STOP_MOVING", 0., &mut motor), Complete);
    assert!(s.route.is_empty());
    assert_eq!(start(&mut s, "TASK_SMALL_FLINCH", 0., &mut motor), Complete);
}

#[test]
fn it_leans_toward_a_target_to_one_side_and_more_with_speed() {
    let right = Vec3::NEG_Y;
    assert_eq!(lean(right, Vec3::X * 100., MAX_SPEED), 0.);
    assert_eq!(lean(right, Vec3::NEG_Y * 100., 0.), 0.);
    assert_eq!(lean(right, Vec3::NEG_Y * 100., MAX_SPEED), 120.);
    assert_eq!(lean(right, Vec3::Y * 100., MAX_SPEED), -120.);
    assert!(lean(right, Vec3::NEG_Y, 100.) < lean(right, Vec3::NEG_Y, 200.));
}

#[test]
fn the_engine_starts_once_and_follows_the_speed() {
    let mut s = Scanner::new(false, 3);
    s.engine_sound();
    assert!(s.engine_on);
    assert_eq!(s.requests.len(), 3);
    s.requests.clear();
    s.engine_sound();
    assert!(s.requests.is_empty(), "the same speed changes nothing");
    s.hover.velocity = Vec3::X * MAX_SPEED;
    s.engine_sound();
    let pitch = PatchOp::Pitch {
        pitch: 100.,
        seconds: 0.1,
    };
    let volume = PatchOp::Volume {
        volume: 1.,
        seconds: 0.1,
    };
    let cue = ENGINE;
    assert_eq!(
        s.requests,
        [
            CopRequest::Patch { cue, op: pitch },
            CopRequest::Patch { cue, op: volume }
        ]
    );
}

#[test]
fn a_gagged_scanner_never_chirps() {
    let mut s = Scanner::new(true, 3);
    assert!((0..1000).all(|_| s.idle_cue(false).is_none()));
}

#[test]
fn it_chirps_about_one_think_in_twenty_six_by_state() {
    let mut s = Scanner::new(false, 3);
    let cues: Vec<_> = (0..2600).filter_map(|_| s.idle_cue(true)).collect();
    assert!((60..150).contains(&cues.len()), "{}", cues.len());
    assert!(cues.iter().all(|cue| *cue == COMBAT));
    let idle = (0..2600).find_map(|_| s.idle_cue(false));
    assert_eq!(idle, Some(IDLE));
}

#[test]
fn every_cue_it_plays_is_preloaded() {
    for cue in [ENGINE, IDLE, COMBAT, PAIN, DIE, TAKE_PHOTO, DIVE_BOMB] {
        assert!(SOUNDS.contains(&cue), "{cue}");
    }
}

#[test]
fn scanners_placed_apart_bob_differently() {
    let (a, b) = (Scanner::new(false, 3), Scanner::new(false, 4));
    assert_ne!(a.bob, b.bob);
    assert!((2. ..=4.).contains(&a.bob.z.abs()));
}
