use crate::entities::{tests::turning_police_world, Scene};
use glam::Vec3;

/// The skeleton composed from scratch, with no memo.
fn fresh(
    scene: &mut Scene,
    rig: &modkit_core::animation::Rig,
    bones: Option<&[bool]>,
) -> Vec<glam::Mat4> {
    scene.pose_memo.clear();
    scene.actor_matrices_of(rig, 0, bones)
}

#[test]
fn memoized_partial_poses_equal_fresh_ones_and_follow_input_changes_within_a_tick() {
    let world = turning_police_world();
    let rig = &world.rigs["police.mdl#0"];
    let mut scene = Scene::new(&world);
    assert!(scene.ai_set_activity(&world, 0, "ACT_IDLE"));
    for _ in 0..40 {
        scene.tick(&world, Vec3::ZERO, 0.015);
        scene.update_sequence_transitions(&world);
    }
    let part: &[bool] = &[true];
    let entries = |scene: &Scene| scene.pose_memo.0.lock().unwrap().len();
    // Repeated partial requests in one tick share one entry; whole skeletons skip the memo.
    let first = scene.actor_matrices_of(rig, 0, Some(part));
    assert_eq!(scene.actor_matrices_of(rig, 0, Some(part)), first);
    assert_eq!(entries(&scene), 1);
    assert_eq!(scene.actor_matrices(rig, 0), first);
    assert_eq!(entries(&scene), 1);
    assert_eq!(first, fresh(&mut scene, rig, Some(part)));
    // The next tick moves the pose; a restart mid-tick (no tick between) changes the inputs:
    // composed again, never the stale skeleton.
    scene.tick(&world, Vec3::ZERO, 0.015);
    scene.update_sequence_transitions(&world);
    assert_eq!(entries(&scene), 0, "each tick starts empty");
    let moved = scene.actor_matrices_of(rig, 0, Some(part));
    assert_ne!(moved, first);
    assert!(scene.ai_set_activity(&world, 0, "ACT_IDLE"));
    let restarted = scene.actor_matrices_of(rig, 0, Some(part));
    assert_eq!(restarted, fresh(&mut scene, rig, Some(part)));
    // A head goal (a pose-signature input) set without a tick is seen too.
    scene.heads.entry(0).or_default().goal = Vec3::new(10., 20., 0.);
    let turned = scene.actor_matrices_of(rig, 0, Some(part));
    assert_eq!(turned, fresh(&mut scene, rig, Some(part)));
}
