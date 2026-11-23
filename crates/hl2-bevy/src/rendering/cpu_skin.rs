//! The CPU skinning path for meshes without GPU joints.
use super::{AnimatedMesh, source_to_bevy};
use bevy::prelude::{Mesh, Vec3};

/// The CPU skinning path: a mesh's positions and normals in this pose.
pub(super) fn skin_on_cpu(mesh: &mut Mesh, animation: &AnimatedMesh, matrices: &[glam::Mat4]) {
    let bind = &animation.vertices.bind;
    let deformed = animation.deformed.as_deref();
    let positions: Vec<_> = bind
        .iter()
        .enumerate()
        .map(|(i, (pos, weights))| {
            let base = deformed.and_then(|d| d.get(i).copied()).unwrap_or(*pos);
            let p = weights
                .as_ref()
                .map_or(base, |w| modkit_core::animation::skin(base, w, matrices));
            source_to_bevy(Vec3::from_array((p * animation.scale).to_array())).to_array()
        })
        .collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    if animation.vertices.normals.len() == bind.len() {
        let normals: Vec<_> = bind
            .iter()
            .zip(&animation.vertices.normals)
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
