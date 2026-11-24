//! Player physics shadow sleep and wake tests.

use super::super::{vector, Physics, SCALE};
use super::{npc_floor_world, shadow_scene};
use glam::Vec3;
use modkit_core::movement::TICK;

#[test]
fn stationary_player_shadow_sleeps_and_stops_reporting_changes() {
    let mut physics = Physics::new(&npc_floor_world());
    for _ in 0..200 {
        physics.update_player_shadow(
            Vec3::ZERO,
            Vec3::ZERO,
            false,
            true,
            0.,
            Vec3::ZERO,
            None,
            TICK,
        );
        physics.tick(TICK);
    }
    let (shadow, _) = physics.player_shadow.expect("player shadow created");
    assert!(
        physics.bodies[shadow].is_sleeping(),
        "stationary player shadow should fall asleep"
    );
    assert_eq!(
        physics.query.reported,
        (0, 0),
        "sleeping player shadow should not report collider changes"
    );
}

#[test]
fn sleeping_player_shadow_wakes_on_move_and_on_prop_impact() {
    let mut physics = Physics::new(&npc_floor_world());
    for _ in 0..200 {
        physics.update_player_shadow(
            Vec3::ZERO,
            Vec3::ZERO,
            false,
            true,
            0.,
            Vec3::ZERO,
            None,
            TICK,
        );
        physics.tick(TICK);
    }
    let (shadow, _) = physics.player_shadow.expect("player shadow created");
    assert!(physics.bodies[shadow].is_sleeping());

    // Zero-velocity update preserves the stationary position and keeps the body asleep.
    physics.update_player_shadow(
        Vec3::ZERO,
        Vec3::ZERO,
        false,
        true,
        0.,
        Vec3::ZERO,
        None,
        TICK,
    );
    assert!(physics.bodies[shadow].is_sleeping());
    let pos = *physics.bodies[shadow].translation() / SCALE;
    assert!(
        (Vec3::new(pos.x, pos.y, pos.z) - Vec3::new(0., 0., 36.)).length() < 1e-3,
        "unexpected shadow drift: {pos:?}"
    );

    // Moving the player wakes the shadow and drives it to the new position.
    let moved_feet = Vec3::new(3., 0., 0.);
    physics.update_player_shadow(
        Vec3::ZERO,
        moved_feet,
        false,
        true,
        0.,
        Vec3::new(200., 0., 0.),
        None,
        TICK,
    );
    assert!(!physics.bodies[shadow].is_sleeping());
    physics.tick(TICK);
    let after_move = physics.bodies[shadow].translation().x / SCALE;
    assert!(
        after_move > 1.,
        "player shadow did not follow moved feet: x={after_move}"
    );

    // A stationary sleeping shadow wakes and blocks an incoming prop on impact.
    let (mut physics, prop) = shadow_scene(10., true, 8.);
    for _ in 0..200 {
        physics.update_player_shadow(
            Vec3::ZERO,
            Vec3::ZERO,
            false,
            true,
            0.,
            Vec3::ZERO,
            None,
            TICK,
        );
        physics.tick(TICK);
    }
    let (shadow, _) = physics.player_shadow.expect("player shadow created");
    assert!(physics.bodies[shadow].is_sleeping());

    physics.bodies[prop].set_linvel(vector(Vec3::new(-400., 0., 0.)), true);
    let mut woke = false;
    for _ in 0..20 {
        physics.update_player_shadow(
            Vec3::ZERO,
            Vec3::ZERO,
            false,
            true,
            0.,
            Vec3::ZERO,
            None,
            TICK,
        );
        physics.tick(TICK);
        if !physics.bodies[shadow].is_sleeping() {
            woke = true;
        }
    }
    assert!(woke, "prop impact should wake the sleeping player shadow");
    let prop_x = physics.bodies[prop].translation().x / SCALE;
    assert!(
        prop_x > 10.,
        "woken shadow should block the prop instead of letting it tunnel: x={prop_x}"
    );
}
