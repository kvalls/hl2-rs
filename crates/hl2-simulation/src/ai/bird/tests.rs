use super::*;
use crate::ai::conditions::COND_HEAR_COMBAT;
use crate::ai::schedule::MoveStatus;

#[derive(Default)]
pub(crate) struct FakeMotor {
    activity: String,
    finished: bool,
    /// The legs of a route, when there is one.
    pub legs: Option<Vec<Vec3>>,
}
impl Motor for FakeMotor {
    fn set_activity(&mut self, activity: &str) {
        self.activity = activity.into();
    }
    fn activity_finished(&self) -> bool {
        self.finished
    }
    fn stop_moving(&mut self) -> bool {
        true
    }
    fn face(&mut self, _goal: Goal) -> bool {
        true
    }
    fn path_to(&mut self, _goal: Vec3, _run: bool) -> bool {
        true
    }
    fn movement(&self) -> MoveStatus {
        MoveStatus::Idle
    }
    fn fly_route(&mut self, _goal: Vec3) -> Option<Vec<Vec3>> {
        self.legs.clone()
    }
    fn fly_patrol(&mut self, _length: f32, _seed: &mut u64) -> Option<Vec<Vec3>> {
        self.legs.clone()
    }
}

fn crow() -> Bird {
    Bird::new(Kind::Crow, false, false, String::new(), 7)
}
fn npc(conditions: &[Cond]) -> AiNpc {
    let mut npc = AiNpc::new(1);
    npc.state = NpcState::Idle;
    npc.conditions = Conditions::of(conditions);
    npc
}
fn at(now: f64) -> Context {
    Context {
        now,
        ..Default::default()
    }
}
fn start(bird: &mut Bird, npc: &mut AiNpc, task: &str, ctx: &Context) -> Option<TaskStatus> {
    let mut motor = FakeMotor::default();
    bird.start_task(npc, task, &TaskArg::parse("0"), ctx, &mut motor)
}

#[test]
fn each_class_is_its_kind_and_others_are_not_birds() {
    assert_eq!(Kind::for_class("npc_crow"), Some(Kind::Crow));
    assert_eq!(Kind::for_class("npc_pigeon"), Some(Kind::Pigeon));
    assert_eq!(Kind::for_class("npc_seagull"), Some(Kind::Seagull));
    assert_eq!(Kind::for_class("npc_zombie"), None);
    assert_eq!(Kind::Pigeon.death_sound(), None);
    assert_eq!(Kind::Seagull.death_sound(), Some("NPC_Seagull.Pain"));
}

#[test]
fn every_cue_a_bird_plays_is_preloaded() {
    for kind in [Kind::Crow, Kind::Pigeon, Kind::Seagull] {
        let cues = [
            Some(kind.idle_sound()),
            kind.alert_sound(),
            kind.death_sound(),
        ];
        for cue in cues.into_iter().flatten() {
            assert!(SOUNDS.contains(&cue), "{cue}");
        }
    }
    assert!(SOUNDS.contains(&"NPC_Crow.Flap"));
}

#[test]
fn the_schedules_parse() {
    let mut schedules = Schedules(Default::default());
    add_schedules(&mut schedules);
    assert_eq!(schedules.0.len(), SCHEDULES.len());
}

#[test]
fn distance_decides_between_ignoring_walking_and_flying() {
    let mut bird = crow();
    bird.next_ground_move = 100.;
    let ctx = at(10.);
    let select = |bird: &mut Bird, distance: f32| {
        let mut npc = npc(&[]);
        bird.sense_enemy(&mut npc.conditions, distance);
        bird.select_schedule(&mut npc, &ctx)
    };
    assert_eq!(select(&mut bird, 1100.), "SCHED_IDLE_STAND");
    assert_eq!(select(&mut bird, 800.), "SCHED_CROW_WALK_AWAY");
    assert_eq!(select(&mut bird, 500.), "SCHED_CROW_FLY_AWAY");
}

#[test]
fn gunfire_scares_it_once_in_ten_seconds() {
    let mut bird = crow();
    bird.next_ground_move = 100.;
    let mut heard = npc(&[COND_HEAR_COMBAT]);
    let first = bird.select_schedule(&mut heard, &at(10.));
    assert_eq!(first, "SCHED_CROW_FLY_AWAY");
    let again = bird.select_schedule(&mut heard, &at(15.));
    assert_eq!(again, "SCHED_IDLE_STAND");
    let later = bird.select_schedule(&mut heard, &at(20.1));
    assert_eq!(later, "SCHED_CROW_FLY_AWAY");
}

#[test]
fn damage_and_the_input_send_it_up_and_a_flyer_stays_up() {
    let mut bird = crow();
    let hurt = bird.select_schedule(&mut npc(&[COND_LIGHT_DAMAGE]), &at(1.));
    assert_eq!(hurt, "SCHED_CROW_FLY_AWAY");
    bird.forced_fly = true;
    let told = bird.select_schedule(&mut npc(&[]), &at(1.));
    assert_eq!(told, "SCHED_CROW_FLY_AWAY");
    assert!(!bird.forced_fly);
    bird.flying = true;
    let up = bird.select_schedule(&mut npc(&[COND_LIGHT_DAMAGE]), &at(1.));
    assert_eq!(up, "SCHED_CROW_IDLE_FLY");
}

#[test]
fn a_falling_bird_takes_off_but_not_in_the_first_two_seconds() {
    let mut bird = crow();
    bird.next_ground_move = 100.;
    let falling = |now| Context {
        airborne: true,
        ..at(now)
    };
    assert_eq!(
        bird.select_schedule(&mut npc(&[]), &falling(1.)),
        "SCHED_IDLE_STAND"
    );
    assert_eq!(
        bird.select_schedule(&mut npc(&[]), &falling(3.)),
        "SCHED_CROW_FLY_AWAY"
    );
}

#[test]
fn it_strolls_when_the_time_comes_and_not_again_for_ten_seconds() {
    let mut bird = crow();
    let ctx = at(6.);
    assert_eq!(
        bird.select_schedule(&mut npc(&[]), &ctx),
        "SCHED_CROW_IDLE_WALK"
    );
    assert!(bird.next_ground_move >= 16.);
    assert_eq!(
        bird.select_schedule(&mut npc(&[]), &ctx),
        "SCHED_IDLE_STAND"
    );
}

#[test]
fn a_bird_takes_the_nearest_hint_of_its_group() {
    let hint = |x: f32, group: &str| FlyHint {
        position: Vec3::new(x, 0., 0.),
        group: group.into(),
    };
    let hints = [
        hint(500., "perch_1"),
        hint(100., "perch_2"),
        hint(20_000., ""),
    ];
    let nearest = |group| nearest_hint(&hints, Vec3::ZERO, group).map(|p| p.x);
    assert_eq!(nearest(""), Some(100.));
    assert_eq!(nearest("PERCH_1"), Some(500.));
    assert_eq!(nearest("perch_9"), None);
}

#[test]
fn a_hint_is_kept_across_flights() {
    let mut bird = crow();
    let mut n = npc(&[]);
    let mut ctx = at(1.);
    ctx.fly_hints = Some(std::sync::Arc::from(vec![FlyHint {
        position: Vec3::X * 100.,
        group: String::new(),
    }]));
    assert_eq!(
        start(&mut bird, &mut n, "TASK_FIND_HINTNODE", &ctx),
        Some(TaskStatus::Complete)
    );
    ctx.fly_hints = None;
    assert_eq!(
        start(&mut bird, &mut n, "TASK_FIND_HINTNODE", &ctx),
        Some(TaskStatus::Complete)
    );
    let mut motor = FakeMotor {
        legs: Some(vec![Vec3::Y, Vec3::X * 100.]),
        ..Default::default()
    };
    let task = "TASK_GET_PATH_TO_HINTNODE";
    let route = bird.start_task(&mut n, task, &TaskArg::parse("0"), &ctx, &mut motor);
    assert_eq!(route, Some(TaskStatus::Complete));
    assert_eq!(bird.waypoint.map(|w| w.position), Some(Vec3::Y));
    assert!(!bird.flying, "a hint is reached by taking off");
    assert!(!bird.last_leg());
    assert!(bird.next_leg());
    assert_eq!(bird.waypoint.map(|w| w.position), Some(Vec3::X * 100.));
    assert!(bird.last_leg() && !bird.next_leg());
}

#[test]
fn no_way_to_the_hint_keeps_it_on_the_ground_and_drops_the_hint() {
    let mut bird = crow();
    let mut n = npc(&[]);
    bird.hint = Some(Vec3::X);
    let task = "TASK_GET_PATH_TO_HINTNODE";
    let status = start(&mut bird, &mut n, task, &at(1.));
    assert_eq!(status, Some(TaskStatus::Failed));
    assert_eq!(bird.hint, None);
    assert!(bird.waypoint.is_none() && !bird.flying);
}

#[test]
fn no_hint_fails_the_task_and_a_named_corner_needs_none() {
    let mut bird = crow();
    let mut n = npc(&[]);
    let ctx = at(1.);
    assert_eq!(
        start(&mut bird, &mut n, "TASK_FIND_HINTNODE", &ctx),
        Some(TaskStatus::Failed)
    );
    let corner = Waypoint {
        position: Vec3::Z * 300.,
        corner: Some(7),
    };
    bird.goal_corner = Some(corner);
    assert_eq!(
        start(&mut bird, &mut n, "TASK_FIND_HINTNODE", &ctx),
        Some(TaskStatus::Complete)
    );
    start(&mut bird, &mut n, "TASK_GET_PATH_TO_HINTNODE", &ctx);
    assert!(bird.flying, "a named path flies at once");
    assert_eq!(bird.waypoint, Some(corner));
}

#[test]
fn only_the_crow_can_call_out_on_takeoff() {
    let takeoff_cues = |kind| {
        let mut bird = Bird::new(kind, false, false, String::new(), 7);
        let mut n = npc(&[]);
        let ctx = Context {
            random: 0.1,
            ..at(1.)
        };
        start(&mut bird, &mut n, "TASK_CROW_TAKEOFF", &ctx);
        assert_eq!(n.activity, "ACT_CROW_TAKEOFF");
        bird.requests.len()
    };
    assert_eq!(takeoff_cues(Kind::Crow), 2);
    assert_eq!(takeoff_cues(Kind::Pigeon), 1);
    assert_eq!(takeoff_cues(Kind::Seagull), 1);
}

#[test]
fn takeoff_ends_once_flying_and_fails_with_nowhere_to_go() {
    let mut bird = crow();
    let mut n = npc(&[]);
    let mut motor = FakeMotor::default();
    let arg = TaskArg::parse("0");
    let ctx = at(1.);
    let task = "TASK_CROW_TAKEOFF";
    let failed = bird.run_task(&mut n, task, &arg, &ctx, &mut motor);
    assert_eq!(failed, Some(TaskStatus::Failed));
    bird.waypoint = Some(Waypoint {
        position: Vec3::X,
        corner: None,
    });
    let waiting = bird.run_task(&mut n, task, &arg, &ctx, &mut motor);
    assert_eq!(waiting, Some(TaskStatus::Running));
    bird.flying = true;
    let done = bird.run_task(&mut n, task, &arg, &ctx, &mut motor);
    assert_eq!(done, Some(TaskStatus::Complete));
    assert_eq!(motor.activity, "ACT_FLY");
}

#[test]
fn a_flight_holds_its_schedule_until_it_ends_and_a_walk_goes_to_the_generic_task() {
    let mut bird = crow();
    let mut n = npc(&[]);
    let mut motor = FakeMotor::default();
    let arg = TaskArg::parse("0");
    let ctx = at(1.);
    let task = "TASK_WAIT_FOR_MOVEMENT";
    assert_eq!(bird.run_task(&mut n, task, &arg, &ctx, &mut motor), None);
    bird.flying = true;
    bird.waypoint = Some(Waypoint {
        position: Vec3::X,
        corner: None,
    });
    assert_eq!(
        bird.run_task(&mut n, task, &arg, &ctx, &mut motor),
        Some(TaskStatus::Running)
    );
    bird.waypoint = None;
    assert_eq!(
        bird.run_task(&mut n, task, &arg, &ctx, &mut motor),
        Some(TaskStatus::Complete)
    );
}

#[test]
fn a_failed_flight_drops_until_it_lands() {
    let mut bird = crow();
    let mut n = npc(&[]);
    let mut motor = FakeMotor::default();
    let arg = TaskArg::parse("0");
    let task = "TASK_CROW_FALL_TO_GROUND";
    bird.flying = true;
    start(&mut bird, &mut n, task, &at(1.));
    assert!(!bird.flying && bird.drop);
    let asked = bird.run_task(&mut n, task, &arg, &at(1.), &mut motor);
    assert_eq!(asked, Some(TaskStatus::Running));
    bird.drop = false;
    let falling = Context {
        airborne: true,
        ..at(1.1)
    };
    let down = bird.run_task(&mut n, task, &arg, &falling, &mut motor);
    assert_eq!(down, Some(TaskStatus::Running));
    let landed = bird.run_task(&mut n, task, &arg, &at(2.), &mut motor);
    assert_eq!(landed, Some(TaskStatus::Complete));
}

#[test]
fn the_evade_goal_lies_away_from_the_enemy_on_the_level() {
    let mut bird = crow();
    let mut n = npc(&[]);
    let ctx = Context {
        enemy: Some((0, Vec3::new(100., 0., 50.))),
        random: 0.5,
        ..at(1.)
    };
    start(&mut bird, &mut n, "TASK_CROW_PICK_EVADE_GOAL", &ctx);
    assert_eq!(n.save_position, Some(Vec3::new(-48., 0., 0.)));
    let alone = start(&mut bird, &mut n, "TASK_CROW_PICK_EVADE_GOAL", &at(1.));
    assert_eq!(alone, Some(TaskStatus::Failed));
}

#[test]
fn a_flight_turns_little_far_away_and_hardest_at_the_goal() {
    assert_eq!(turn_share(1000.), 0.25);
    assert_eq!(turn_share(256.), 0.);
    assert_eq!(turn_share(128.), 0.5);
    assert_eq!(turn_share(0.), 1.);
    let turned = heading(Vec3::X, Vec3::Y, 1000.);
    assert!(turned.y > 0. && turned.x > turned.y);
    assert!((turned.length() - 1.).abs() < 1e-5);
    assert_eq!(heading(Vec3::X, -Vec3::X, 128.), Vec3::X, "no way to turn");
}

#[test]
fn a_flight_slides_along_what_it_would_hit() {
    let glancing = Vec3::new(1., 1., 0.).normalize();
    assert!(deflect(glancing, -Vec3::X).distance(Vec3::Y) < 1e-5);
    assert_eq!(deflect(Vec3::X, -Vec3::X), Vec3::ZERO, "head on: it stops");
}

#[test]
fn it_banks_into_the_turn_up_to_the_limit() {
    // Heading along +x, right is -y: a goal to the right rolls one way, to the left the other.
    assert_eq!(bank(0., Vec3::new(100., -100., 0.)), 45.);
    assert_eq!(bank(0., Vec3::new(100., 100., 0.)), -45.);
    assert_eq!(bank(0., Vec3::X * 100.), 0.);
}

#[test]
fn the_first_steer_after_a_new_goal_keeps_the_old_direction() {
    let mut bird = crow();
    bird.waypoint = Some(Waypoint {
        position: Vec3::X * 1000.,
        corner: None,
    });
    bird.target = Vec3::Y;
    let first = bird.steer(Vec3::ZERO, Vec3::X, 0.1, 1.).unwrap();
    assert!(
        first.direction.y > 0.,
        "still turning toward the old target"
    );
    assert_eq!(bird.target, Vec3::Y);
    let second = bird.steer(Vec3::ZERO, Vec3::X, 0.1, 1.1).unwrap();
    assert_eq!(second.direction, Vec3::X);
    assert_eq!(bird.target, Vec3::X);
    assert!(!second.arrived);
    assert!(
        (bird.danger_sound_time - 6.1).abs() < 1e-9,
        "flying deafens it for a while"
    );
}

#[test]
fn a_steer_arrives_within_one_think_of_flight_and_needs_a_waypoint() {
    let mut bird = crow();
    assert_eq!(bird.steer(Vec3::ZERO, Vec3::X, 0.1, 1.), None);
    bird.waypoint = Some(Waypoint {
        position: Vec3::X * 21.,
        corner: None,
    });
    assert!(bird.steer(Vec3::ZERO, Vec3::X, 0.1, 1.).unwrap().arrived);
}

#[test]
fn it_soars_only_on_the_way_down_and_only_for_a_spell() {
    let mut bird = crow();
    bird.velocity = Vec3::Z * 10.;
    assert_eq!(bird.fly_activity(1., [0.1, 0.]), "ACT_FLY");
    assert_eq!(bird.soar_until, 4.);
    bird.velocity = -Vec3::Z * 10.;
    assert_eq!(
        bird.fly_activity(2., [0.1, 0.]),
        "ACT_FLY",
        "the spell holds"
    );
    assert_eq!(bird.fly_activity(5., [0.1, 0.]), "ACT_CROW_SOAR");
    assert_eq!(bird.soar_until, 6.);
    assert_eq!(bird.fly_activity(7., [0.1, 0.]), "ACT_FLY", "never twice");
}

#[test]
fn a_bird_is_stuck_only_when_still_and_stopped_at_a_check() {
    let mut bird = crow();
    let spot = Vec3::X;
    bird.start_flying(spot, 1.);
    assert!(
        !bird.stuck(spot, 3.9),
        "the first check waits three seconds"
    );
    bird.velocity = Vec3::X * 2.;
    assert!(!bird.stuck(spot, 4.), "still has speed");
    bird.velocity = Vec3::X * 0.5;
    assert!(!bird.stuck(spot, 4.5), "not time yet");
    assert!(bird.stuck(spot, 5.));
    assert!(!bird.stuck(Vec3::Y, 5.), "it moved");
}

#[test]
fn the_dice_differ_by_seed_and_stay_in_range() {
    let mut a = Bird::new(Kind::Crow, false, false, String::new(), 7);
    let mut b = Bird::new(Kind::Crow, false, false, String::new(), 8);
    let rolls = [a.roll(), a.roll(), a.roll()];
    assert!(rolls.iter().all(|r| (0. ..1.).contains(r)));
    assert_ne!(rolls[0], b.roll());
    assert!(a.next_ground_move < 5.);
}

#[test]
fn a_deaf_bird_hears_nothing() {
    assert_ne!(crow().sound_interests(), 0);
    let deaf = Bird::new(Kind::Crow, false, true, String::new(), 7);
    assert_eq!(deaf.sound_interests(), 0);
}
