use super::*;

/// Walks `path` from its first point as the motor plans it, 15 ms a tick: the seconds until
/// within `stop` of the end, and the speed each time a waypoint is passed.
fn walk(
    path: &[Vec3],
    ideal: f32,
    accel: Accel,
    arrival_distance: f32,
    stop: f32,
) -> (f32, Vec<f32>) {
    let dt = 0.015;
    let (mut feet, mut next, mut speed, mut heading) = (path[0], 1, 0., Vec2::ZERO);
    let (mut time, mut passed) = (0., Vec::new());
    let goal = path[path.len() - 1];
    while feet.truncate().distance(goal.truncate()) > stop {
        let plan = Plan {
            feet,
            heading,
            speed,
            waypoints: &path[next..],
            ideal,
            accel: accel.of(ideal),
            arrival_distance,
        };
        let (mut distance, new_speed) = step(&plan, dt);
        speed = new_speed;
        while distance > 0. && next < path.len() {
            let leg = path[next] - feet;
            let length = leg.truncate().length();
            heading = leg.truncate().normalize_or_zero();
            if distance < length {
                feet += leg / length * distance;
                break;
            }
            feet = path[next];
            distance -= length;
            next += 1;
            passed.push(speed);
        }
        time += dt;
        assert!(time < 60., "never arrived");
    }
    (time, passed)
}

#[test]
fn a_still_vortigaunt_takes_the_two_ramps_on_a_straight_walk() {
    // d1_trainstation_01: vort_sweepy's spawn to ss_VortSweep3, 233.5 units; Walk_all_HoldBroom
    // at 43.8 u/s, CBaseAnimating's acceleration 93.8 u/s/s. Full speed would take 5.33 s; the
    // ramps add speed/acceleration (0.467 s). Native (owned game, npc_task_text): 5.985 s from
    // TASK_SCRIPT_CUSTOM_MOVE_TO_TARGET to TASK_PLANT_ON_SCRIPT, counted at the AI's thinks.
    let path = [
        Vec3::new(-4300., -2940.6, -29.),
        Vec3::new(-4364., -2716., -32.),
    ];
    let (time, _) = walk(&path, 43.8, Accel::PlusFifty, 0., 0.1);
    let expected = 233.5 / 43.8 + 43.8 / 93.8;
    assert!((time - expected).abs() < 0.05, "{time} vs {expected}");
}

#[test]
fn a_right_angle_corner_is_taken_at_a_fifth_of_the_ideal_speed() {
    let path = [
        Vec3::ZERO,
        Vec3::new(300., 0., 0.),
        Vec3::new(300., 300., 0.),
    ];
    let (_, passed) = walk(&path, 50., Accel::PlusFifty, 0., 0.1);
    // The dot of the two legs is 0, plus 0.2: 10 u/s at the corner.
    assert!((passed[0] - 10.).abs() < 1.5, "{passed:?}");
}

#[test]
fn a_reversal_stops_at_the_turn() {
    let path = [Vec3::ZERO, Vec3::new(200., 0., 0.), Vec3::new(50., 30., 0.)];
    let (_, passed) = walk(&path, 50., Accel::PlusFifty, 0., 0.1);
    assert!(passed[0] < 2., "{passed:?}");
}

#[test]
fn an_arrival_distance_slows_to_a_walk_there() {
    // A scene's moveto ends 48 units short: the motor brakes to 1 u/s at that point.
    let path = [Vec3::ZERO, Vec3::new(400., 0., 0.)];
    let plan = |feet: Vec3, speed: f32| Plan {
        feet,
        heading: Vec2::X,
        speed,
        waypoints: &path[1..],
        ideal: 50.,
        accel: 100.,
        arrival_distance: 48.,
    };
    // At full speed 60 units before the goal (12 before the stop) it brakes at the full
    // 100 u/s/s: the speed it may still have there is sqrt(1 + 2 x 100 x 12) = 49.
    let (_, speed) = step(&plan(Vec3::new(340., 0., 0.), 50.), 0.015);
    assert!((speed - (49. - 100. * 0.015)).abs() < 0.05, "{speed}");
    // Far away it keeps its speed.
    let (distance, speed) = step(&plan(Vec3::new(100., 0., 0.), 50.), 0.015);
    assert!((speed - 50.).abs() < 0.01 && (distance - 0.75).abs() < 0.01);
}

#[test]
fn the_metrocop_accelerates_twice_its_speed_and_unblended_classes_have_no_motor() {
    assert_eq!(Accel::for_class("npc_vortigaunt"), Some(Accel::PlusFifty));
    assert_eq!(Accel::for_class("npc_fastzombie"), Some(Accel::PlusFifty));
    assert_eq!(Accel::for_class("npc_metropolice"), Some(Accel::Twice));
    assert_eq!(Accel::for_class("npc_strider"), Some(Accel::Same));
    assert_eq!(Accel::for_class("npc_headcrab"), None);
    assert_eq!(Accel::for_class("npc_crow"), None);
    assert_eq!(Accel::Twice.of(80.), 160.);
    let path = [Vec3::ZERO, Vec3::new(300., 0., 0.)];
    let (cop, _) = walk(&path, 80., Accel::Twice, 0., 0.1);
    assert!((cop - (300. / 80. + 0.5)).abs() < 0.05, "{cop}");
}
