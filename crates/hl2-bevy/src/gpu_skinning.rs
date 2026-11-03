//! Bevy skinning adapter. Source still samples poses; vertices stay immutable on the GPU.
use bevy::{
    mesh::{
        VertexAttributeValues,
        skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    },
    prelude::*,
};
use modkit_core::animation::{Rig, Weights};

#[derive(Component)]
pub struct Joint;
#[derive(Clone)]
pub struct Skeleton {
    pub joints: Vec<Entity>,
    pub inverse_bindposes: Handle<SkinnedMeshInverseBindposes>,
}
/// Include model scale in the basis change, rather than applying it twice to joints.
pub fn basis(scale: f32) -> Mat4 {
    Mat4::from_cols(
        (crate::source_to_bevy(Vec3::X) * scale).extend(0.),
        (crate::source_to_bevy(Vec3::Y) * scale).extend(0.),
        (crate::source_to_bevy(Vec3::Z) * scale).extend(0.),
        Vec4::W,
    )
}
pub fn convert(matrix: glam::Mat4, scale: f32) -> Mat4 {
    let basis = basis(scale);
    basis * Mat4::from_cols_array(&matrix.to_cols_array()) * basis.inverse()
}
/// Match the shared CPU skin function: ignore invalid/nonpositive influences,
/// normalize the remaining weights, and preserve a bind vertex with no influences.
pub fn influences(weights: Option<&Weights>, bones: usize) -> ([u16; 4], [f32; 4]) {
    let mut indices = [bones as u16; 4];
    let mut values = [0.; 4];
    if let Some(weights) = weights {
        for i in 0..3 {
            if usize::from(weights.bones[i]) < bones && weights.weights[i] > 0. {
                indices[i] = u16::from(weights.bones[i]);
                values[i] = weights.weights[i];
            }
        }
    }
    let total: f32 = values.iter().sum();
    if total > 0. {
        for weight in &mut values {
            *weight /= total;
        }
    } else {
        values[0] = 1.;
    }
    (indices, values)
}
impl Skeleton {
    pub fn spawn(
        commands: &mut Commands,
        assets: &mut Assets<SkinnedMeshInverseBindposes>,
        rig: &Rig,
        scale: f32,
        root: Entity,
    ) -> Self {
        let poses = rig.matrices("", 0.);
        let mut binds = Vec::with_capacity(rig.bones.len() + 1);
        let mut joints = Vec::with_capacity(rig.bones.len() + 1);
        for (pose, bone) in poses.iter().zip(&rig.bones) {
            binds.push(convert(bone.inverse_bind, scale));
            joints.push(
                commands
                    .spawn((
                        Joint,
                        ChildOf(root),
                        Transform::from_matrix(convert(*pose * bone.inverse_bind.inverse(), scale)),
                    ))
                    .id(),
            );
        }
        // Identity joint for vertices without valid weights.
        binds.push(Mat4::IDENTITY);
        joints.push(
            commands
                .spawn((Joint, ChildOf(root), Transform::IDENTITY))
                .id(),
        );
        Self {
            joints,
            inverse_bindposes: assets.add(SkinnedMeshInverseBindposes::from(binds)),
        }
    }
    /// CBoneMergeCache: a followed model's bones take the owner's same-named joints; an
    /// unmatched bone keeps its bind offset from its nearest matched ancestor (the
    /// ancestor's joint with that ancestor's inverse bind), and a chain without any match
    /// stays at the owner's origin in its bind pose (the owner's identity joint).
    pub fn merged(
        assets: &mut Assets<SkinnedMeshInverseBindposes>,
        rig: &Rig,
        owner_rig: &Rig,
        owner: &Skeleton,
        scale: f32,
    ) -> Self {
        let identity = *owner
            .joints
            .last()
            .expect("skeletons end with an identity joint");
        let mut joints = Vec::with_capacity(rig.bones.len() + 1);
        let mut binds = Vec::with_capacity(rig.bones.len() + 1);
        for (index, _) in rig.bones.iter().enumerate() {
            let mut bone = Some(index);
            let mut found = None;
            while let Some(b) = bone {
                if let Some(j) = owner_rig
                    .bones
                    .iter()
                    .position(|o| o.name.eq_ignore_ascii_case(&rig.bones[b].name))
                    .and_then(|j| owner.joints.get(j))
                {
                    found = Some((*j, b));
                    break;
                }
                bone = rig.bones[b].parent;
            }
            match found {
                Some((joint, b)) => {
                    joints.push(joint);
                    binds.push(convert(rig.bones[b].inverse_bind, scale));
                }
                None => {
                    joints.push(identity);
                    binds.push(Mat4::IDENTITY);
                }
            }
        }
        joints.push(identity);
        binds.push(Mat4::IDENTITY);
        Self {
            joints,
            inverse_bindposes: assets.add(SkinnedMeshInverseBindposes::from(binds)),
        }
    }
    pub fn component(&self) -> SkinnedMesh {
        SkinnedMesh {
            joints: self.joints.clone(),
            inverse_bindposes: self.inverse_bindposes.clone(),
        }
    }
    pub fn attributes(&self, mesh: &mut Mesh, vertices: &[(glam::Vec3, Option<Weights>)]) {
        let (indices, weights): (Vec<_>, Vec<_>) = vertices
            .iter()
            .map(|(_, weights)| influences(weights.as_ref(), self.joints.len() - 1))
            .unzip();
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_JOINT_INDEX,
            VertexAttributeValues::Uint16x4(indices),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights);
        mesh.generate_skinned_mesh_bounds()
            .expect("validated Source skin influences");
    }
}
/// Eye UVs are evaluated from the same pose while the GPU retains bind positions.
pub fn skin_vertex(
    mesh: &Mesh,
    vertex: usize,
    position: glam::Vec3,
    matrices: &[glam::Mat4],
) -> glam::Vec3 {
    let Some(VertexAttributeValues::Uint16x4(indices)) =
        mesh.attribute(Mesh::ATTRIBUTE_JOINT_INDEX)
    else {
        return position;
    };
    let Some(VertexAttributeValues::Float32x4(weights)) =
        mesh.attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT)
    else {
        return position;
    };
    indices[vertex]
        .iter()
        .zip(&weights[vertex])
        .map(|(&bone, &weight)| {
            matrices
                .get(usize::from(bone))
                .map_or(position, |m| m.transform_point3(position))
                * weight
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn animated_joint_bounds_enclose_vertices_without_mutating_bind_mesh() {
        use bevy::{
            asset::RenderAssetUsages,
            camera::visibility::{DynamicSkinnedMeshBounds, VisibilityPlugin},
            ecs::system::SystemState,
            mesh::{Indices, PrimitiveTopology},
        };
        use modkit_core::animation::{Bone, Clip, Pose};
        let bind = Pose {
            position: glam::Vec3::new(4., -3., 2.),
            rotation: glam::Quat::from_rotation_z(0.3),
        };
        let bind_matrix = glam::Mat4::from_rotation_translation(bind.rotation, bind.position);
        let rig = Rig {
            view_bounds: Default::default(),
            pose_parameters: Vec::new(),
            autoplay: Vec::new(),
            attachments: Vec::new(),
            activities: Default::default(),
            hitboxes: Vec::new(),
            bones: vec![Bone {
                name: "root".into(),
                parent: None,
                bind: bind.clone(),
                inverse_bind: bind_matrix.inverse(),
            }],
            clips: std::collections::BTreeMap::from([(
                "move".into(),
                Clip {
                    layer: Default::default(),
                    events: vec![],
                    fps: 1.,
                    looping: false,
                    frames: vec![
                        vec![bind],
                        vec![Pose {
                            position: glam::Vec3::new(-5., 8., 11.),
                            rotation: glam::Quat::from_rotation_y(0.8),
                        }],
                    ],
                },
            )]),
            warnings: vec![],
            sequences: vec![],
        };
        let scale = 2.5;
        let actor = crate::rendering::entity_transform(
            glam::Vec3::new(1000., -420., 38.),
            glam::Quat::from_rotation_z(1.1),
        );
        let vertices = vec![
            (
                glam::Vec3::new(1., 2., 3.),
                Some(Weights {
                    bones: [0; 3],
                    weights: [0.7, 0., 0.],
                }),
            ),
            (glam::Vec3::new(-7., 2., 8.), None),
            (
                glam::Vec3::new(4., -9., 5.),
                Some(Weights {
                    bones: [0; 3],
                    weights: [1., 0., 0.],
                }),
            ),
        ];
        let bind_positions: Vec<_> = vertices
            .iter()
            .map(|(p, _)| {
                crate::source_to_bevy(Vec3::from_array((*p * scale).to_array())).to_array()
            })
            .collect();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin, VisibilityPlugin))
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<SkinnedMeshInverseBindposes>>();
        let mut state = SystemState::<(Commands, ResMut<Assets<SkinnedMeshInverseBindposes>>)>::new(
            app.world_mut(),
        );
        let (mut commands, mut assets) = state.get_mut(app.world_mut()).unwrap();
        let root = commands.spawn(actor).id();
        let skeleton = Skeleton::spawn(&mut commands, &mut assets, &rig, scale, root);
        state.apply(app.world_mut());
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, bind_positions.clone());
        mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
        skeleton.attributes(&mut mesh, &vertices);
        let handle = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
        let entity = app
            .world_mut()
            .spawn((
                Mesh3d(handle.clone()),
                skeleton.component(),
                DynamicSkinnedMeshBounds,
                actor,
            ))
            .id();
        for time in [0., 0.37, 0.83, 1.] {
            let matrices = rig.matrices("move", time);
            *app.world_mut()
                .get_mut::<Transform>(skeleton.joints[0])
                .unwrap() = Transform::from_matrix(convert(
                matrices[0] * rig.bones[0].inverse_bind.inverse(),
                scale,
            ));
            app.update();
            let mesh = app.world().resource::<Assets<Mesh>>().get(&handle).unwrap();
            assert_eq!(
                mesh.attribute(Mesh::ATTRIBUTE_POSITION)
                    .unwrap()
                    .as_float3()
                    .unwrap(),
                &bind_positions
            );
            let aabb = app
                .world()
                .get::<bevy::camera::primitives::Aabb>(entity)
                .unwrap();
            let inverse = app
                .world()
                .resource::<Assets<SkinnedMeshInverseBindposes>>()
                .get(&skeleton.inverse_bindposes)
                .unwrap();
            for (vertex, (point, weights)) in vertices.iter().enumerate() {
                let expected = weights.as_ref().map_or(*point, |w| {
                    modkit_core::animation::skin(*point, w, &matrices)
                });
                let local = crate::source_to_bevy(Vec3::from_array((expected * scale).to_array()));
                assert!(
                    Vec3::from(aabb.center - aabb.half_extents)
                        .cmple(local + Vec3::splat(0.001))
                        .all()
                );
                assert!(
                    Vec3::from(aabb.center + aabb.half_extents)
                        .cmpge(local - Vec3::splat(0.001))
                        .all()
                );
                let (indices, weights) = influences(vertices[vertex].1.as_ref(), rig.bones.len());
                let gpu: Vec3 = indices
                    .iter()
                    .zip(weights)
                    .map(|(&joint, weight)| {
                        let transform = app
                            .world()
                            .get::<GlobalTransform>(skeleton.joints[usize::from(joint)])
                            .unwrap();
                        (transform.to_matrix() * inverse[usize::from(joint)])
                            .transform_point3(Vec3::from_array(bind_positions[vertex]))
                            * weight
                    })
                    .sum();
                assert!(gpu.distance(actor.transform_point(local)) < 0.001);
                assert!(skin_vertex(mesh, vertex, *point, &matrices).distance(expected) < 0.00002);
            }
        }
        app.world_mut().entity_mut(root).despawn();
        assert!(
            skeleton
                .joints
                .iter()
                .all(|id| app.world().get_entity(*id).is_err())
        );
    }
    #[test]
    fn normalized_influences_and_scaled_basis_match_cpu_skinning() {
        let matrices = [
            glam::Mat4::from_rotation_translation(
                glam::Quat::from_rotation_z(0.7),
                glam::Vec3::new(4., -8., 3.),
            ),
            glam::Mat4::from_rotation_translation(
                glam::Quat::from_rotation_x(-0.9),
                glam::Vec3::new(-6., 2., 11.),
            ),
        ];
        for weights in [
            Weights {
                bones: [0, 1, 255],
                weights: [0.2, 0.5, 0.3],
            },
            Weights {
                bones: [0, 1, 0],
                weights: [-1., 0., 0.],
            },
            Weights {
                bones: [1, 0, 1],
                weights: [2., 3., 4.],
            },
        ] {
            let (indices, values) = influences(Some(&weights), matrices.len());
            for scale in [0.25, 1., 2.5] {
                for point in [
                    glam::Vec3::ZERO,
                    glam::Vec3::new(4., 7., -3.),
                    glam::Vec3::new(-11., 2., 20.),
                ] {
                    let bind = crate::source_to_bevy(Vec3::from_array((point * scale).to_array()));
                    let gpu: Vec3 = indices
                        .iter()
                        .zip(values)
                        .map(|(&bone, weight)| {
                            matrices
                                .get(usize::from(bone))
                                .map_or(bind, |m| convert(*m, scale).transform_point3(bind))
                                * weight
                        })
                        .sum();
                    let expected = crate::source_to_bevy(Vec3::from_array(
                        (modkit_core::animation::skin(point, &weights, &matrices) * scale)
                            .to_array(),
                    ));
                    assert!(gpu.distance(expected) < 0.00002, "{gpu:?} != {expected:?}");
                }
            }
        }
    }
}
