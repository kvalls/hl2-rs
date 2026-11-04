//! The CPU skinning path for meshes without GPU joints.
use super::{AnimatedMesh, source_to_bevy};
use bevy::prelude::{Mesh, Vec3};

/// The CPU skinning path: a mesh's positions and normals in this pose.
pub(super) fn skin_on_cpu(mesh: &mut Mesh, animation: &AnimatedMesh, matrices: &[glam::Mat4]) {
    let positions: Vec<_> = animation
        .bind
        .iter()
        .map(|(bind, weights)| {
            let p = weights
                .as_ref()
                .map_or(*bind, |w| modkit_core::animation::skin(*bind, w, matrices));
            source_to_bevy(Vec3::from_array((p * animation.scale).to_array())).to_array()
        })
        .collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    if animation.normals.len() == animation.bind.len() {
        let normals: Vec<_> = animation
            .bind
            .iter()
            .zip(&animation.normals)
            .map(|((_, weights), normal)| {
                let n = weights.as_ref().map_or(*normal, |w| {
                    skin_normal(*normal, w, matrices).normalize_or_zero()
                });
                source_to_bevy(Vec3::from_array(n.to_array())).to_array()
            })
            .collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    }
}
/// Rotate a bind normal by the weighted bone matrices (CPU skinning path).
fn skin_normal(
    normal: glam::Vec3,
    weights: &modkit_core::animation::Weights,
    matrices: &[glam::Mat4],
) -> glam::Vec3 {
    let mut out = glam::Vec3::ZERO;
    let mut total = 0.;
    for (&bone, &weight) in weights.bones.iter().zip(&weights.weights) {
        if weight > 0.
            && let Some(m) = matrices.get(usize::from(bone))
        {
            out += m.transform_vector3(normal) * weight;
            total += weight;
        }
    }
    if total > 0. { out } else { normal }
}
