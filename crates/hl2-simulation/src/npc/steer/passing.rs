//! Steering before contact through the controller: a standing NPC and two NPCs meeting.

use super::super::tests::{key, locomotion, make_controller, pose, request, world};
use super::super::{normal_human, ActorPose, GoalKey, GoalState, MoveUpdate};
use crate::npc_probe::{ActorHull, Hull};
use crate::physics::Physics;
use glam::Vec3;

const HUMAN: Hull = Hull {
    mins: Vec3::new(-13., -13., 0.),
    maxs: Vec3::new(13., 13., 72.),
};

fn at(entity: usize, feet: Vec3) -> ActorHull {
    ActorHull {
        entity: Some(entity),
        feet,
        hull: HUMAN,
    }
}

fn feet_of(updates: &[MoveUpdate], entity: usize, feet: &mut Vec3) {
    if let Some(update) = updates.iter().rev().find(|u| u.key.actor == entity) {
        *feet = update.feet;
    }
}

#[test]
fn a_standing_npc_ahead_is_passed_without_stopping() {
    let physics = Physics::new(&world(vec![]));
    let mut controller = make_controller(None);
    let mut feet = Vec3::ZERO;
    let mut wanted = request(Vec3::new(240., 0., 0.));
    wanted.force_short = true;
    let blocker = [at(20, Vec3::new(120., 0., 0.))];
    controller.request(key(1), wanted, pose(feet), &physics, &blocker);
    let mut swerved_early = false;
    for _ in 0..400 {
        let updates = controller.tick(&physics, &[pose(feet)], &blocker, 0.02, false);
        feet_of(&updates, 7, &mut feet);
        let status = controller.status(key(1)).unwrap();
        assert!(
            !matches!(status, GoalState::Blocked(_)),
            "{status:?} at {feet}"
        );
        // Contact would start at x 94 (two 13-unit half widths short of 120).
        swerved_early |= feet.x < 60. && feet.y.abs() > 2.;
        if status.is_arrived() {
            break;
        }
    }
    assert!(controller.status(key(1)).unwrap().is_arrived(), "{feet}");
    assert!(swerved_early, "no swerve before contact");
}

#[test]
fn two_npcs_meeting_head_on_pass_each_other() {
    let physics = Physics::new(&world(vec![]));
    let mut controller = make_controller(None);
    let human = normal_human(1., 18., 1.).unwrap();
    controller
        .register_actor(8, 1., human, locomotion())
        .unwrap();
    controller.set_view_offset(8, Vec3::Z * 70.).unwrap();
    let other = GoalKey { actor: 8, ..key(1) };
    let mut feet = [Vec3::ZERO, Vec3::X * 300.];
    let pose_of = |entity: usize, feet: Vec3| ActorPose {
        entity,
        feet,
        yaw_degrees: if entity == 7 { 0. } else { 180. },
        scripted_by: None,
    };
    let hulls = |feet: [Vec3; 2]| [at(7, feet[0]), at(8, feet[1])];
    let mut wanted = request(feet[1]);
    wanted.force_short = true;
    controller.request(key(1), wanted, pose_of(7, feet[0]), &physics, &hulls(feet));
    let mut back = request(feet[0]);
    back.force_short = true;
    controller.request(other, back, pose_of(8, feet[1]), &physics, &hulls(feet));
    let mut closest = f32::MAX;
    for _ in 0..600 {
        let poses = [pose_of(7, feet[0]), pose_of(8, feet[1])];
        let updates = controller.tick(&physics, &poses, &hulls(feet), 0.02, false);
        feet_of(&updates, 7, &mut feet[0]);
        feet_of(&updates, 8, &mut feet[1]);
        closest = closest.min(feet[0].truncate().distance(feet[1].truncate()));
        let done = [key(1), other].map(|k| controller.status(k).unwrap().is_arrived());
        if done == [true, true] {
            break;
        }
    }
    for k in [key(1), other] {
        assert!(controller.status(k).unwrap().is_arrived(), "{feet:?}");
    }
    // Their hulls never overlapped: they went around each other, not through.
    assert!(closest >= 26., "{closest}");
}
