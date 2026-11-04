//! A ragdoll's start from the NPC that died: its solids' poses and the body the death force
//! strikes.
use crate::entities::Scene;
use glam::{Mat4, Quat, Vec3};
use modkit_core::RagdollModel;

/// Each solid's world pose from the entity frame and the bones' model space matrices.
pub(super) fn body_poses(
    model: &RagdollModel,
    entity: Mat4,
    matrices: &[Mat4],
) -> Vec<(Vec3, Quat)> {
    model
        .bodies
        .iter()
        .map(|b| {
            let (_, rotation, translation) =
                (entity * matrices[b.bone]).to_scale_rotation_translation();
            (translation, rotation.normalize())
        })
        .collect()
}
/// m_nForceBone = the trace's physicsbone: the solid that simulates the hit hitbox's bone (its
/// own, else its nearest ancestor's). Without a hitbox, the solid nearest the force point, else
/// the root. (The solid, the nearest solid.)
pub(super) fn force_body(
    poses: &[(Vec3, Quat)],
    position: Option<Vec3>,
    hit_bone: Option<usize>,
    parents: &[Option<usize>],
    solids: &[usize],
) -> (usize, usize) {
    let nearest = position.map_or(0, |position| {
        poses
            .iter()
            .enumerate()
            .min_by(|a, b| {
                a.1 .0
                    .distance_squared(position)
                    .total_cmp(&b.1 .0.distance_squared(position))
            })
            .map_or(0, |(i, _)| i)
    });
    let body = hit_bone.and_then(|bone| physics_bone(parents, solids, bone));
    (body.unwrap_or(nearest), nearest)
}
/// The drawing entity takes the corpse's frame; it is done like a corpse.
pub(super) fn corpse_takes_frame(scene: &mut Scene, source: usize, id: usize) {
    let (origin, rotation) = (scene.states[source].origin, scene.states[source].rotation);
    let state = &mut scene.states[id];
    state.origin = origin;
    state.rotation = rotation;
    state.killed = true;
    state.visible = false;
}
/// A studio bone's physicsbone: the index of the ragdoll solid that simulates it (the
/// bone's own solid, else its nearest ancestor's).
pub(super) fn physics_bone(
    parents: &[Option<usize>],
    solids: &[usize],
    bone: usize,
) -> Option<usize> {
    std::iter::successors(Some(bone), |b| parents.get(*b).copied().flatten())
        .find_map(|b| solids.iter().position(|&solid| solid == b))
}
