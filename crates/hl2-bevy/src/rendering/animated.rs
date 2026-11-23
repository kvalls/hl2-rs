//! Animated mesh state and per-frame skeleton, morph weight, and CPU skinning updates.
use super::{
    Batch, FlexModels, SourceEntity, actor_poses, cpu_skin, face_flex, lod, placement,
    viewmodel_sample,
};
use bevy::{
    camera::primitives::{Aabb, MeshAabb},
    mesh::{Mesh, Mesh3d, morph::MeshMorphWeights},
    prelude::{Assets, Component, Entity, Query, Res, ResMut, Transform, With, Without},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Component)]
pub struct AnimatedMesh {
    pub(super) sampled: Option<(String, u32, u64)>,
    pub(super) gpu: Option<crate::gpu_skinning::Skeleton>,
    pub(super) entity: Option<usize>,
    pub(super) weapon: Option<String>,
    pub(super) key: String,
    pub(super) scale: f32,
    /// Shared immutable bind positions, normals and facial flex vertex references.
    pub(super) vertices: Arc<lod::Vertices>,
    /// Deformed bind positions for the CPU skinning path when facial flexes are active.
    pub(super) deformed: Option<Vec<glam::Vec3>>,
    /// Per-draw facial flex morph channels when using GPU skinning + Bevy morph targets.
    pub(super) morph: Option<Arc<face_flex::MorphChannels>>,
    pub(super) flex_state: face_flex::FlexState,
}

impl AnimatedMesh {
    pub(super) fn insert_into(self, draw: &mut bevy::ecs::system::EntityCommands) {
        if let Some(morph) = &self.morph {
            draw.insert(morph.weights_component());
        }
        draw.insert(self);
    }

    /// A batch's animation data (its skin taken; facial flexes only for an entity's own model).
    pub(super) fn from_batch(
        batch: &mut Batch,
        (entity, weapon): (Option<usize>, Option<String>),
        (key, scale): (String, f32),
        flexes: bool,
    ) -> Self {
        Self {
            sampled: None,
            gpu: None,
            entity,
            weapon,
            key,
            scale,
            vertices: Arc::new(lod::Vertices {
                bind: std::mem::take(&mut batch.skin),
                normals: batch.normals.clone(),
                flex: if flexes {
                    std::mem::take(&mut batch.flex)
                } else {
                    Vec::new()
                },
            }),
            deformed: None,
            morph: None,
            flex_state: Default::default(),
        }
    }

    /// A template draw's animation for another entity on its own skeleton.
    pub(super) fn copy_for(
        &self,
        entity: usize,
        gpu: Option<crate::gpu_skinning::Skeleton>,
    ) -> Self {
        let morph = gpu.is_some().then(|| self.morph.clone()).flatten();
        Self {
            sampled: None,
            gpu,
            entity: Some(entity),
            weapon: self.weapon.clone(),
            key: self.key.clone(),
            scale: self.scale,
            vertices: Arc::clone(&self.vertices),
            deformed: None,
            morph,
            flex_state: Default::default(),
        }
    }
}

/// Facial flex models, the eyes, shared actor skeletons, and mesh assets for animation.
type AnimationInputs<'w> = (
    Option<Res<'w, FlexModels>>,
    Option<Res<'w, crate::eyes::Eyes>>,
    ResMut<'w, actor_poses::ActorPoses>,
    ResMut<'w, Assets<Mesh>>,
);

type JointTransforms<'w, 's> = Query<
    'w,
    's,
    &'static mut Transform,
    (With<crate::gpu_skinning::Joint>, Without<SourceEntity>),
>;

fn update_gpu_joints(
    skeleton: &crate::gpu_skinning::Skeleton,
    rig: &modkit_core::animation::Rig,
    matrices: &[glam::Mat4],
    scale: f32,
    joints: &mut JointTransforms,
    updated_joints: &mut BTreeSet<Entity>,
) {
    if !updated_joints.insert(skeleton.joints[0]) {
        return;
    }
    for ((joint, matrix), bone) in skeleton.joints.iter().zip(matrices.iter()).zip(&rig.bones) {
        if let Ok(mut transform) = joints.get_mut(*joint) {
            let pose = Transform::from_matrix(crate::gpu_skinning::convert(
                *matrix * bone.inverse_bind.inverse(),
                scale,
            ));
            if *transform != pose {
                *transform = pose;
            }
        }
    }
}

pub fn present_entities(
    game: Res<crate::gameplay::Gameplay>,
    sim: Res<crate::simulation::Simulation>,
    mut entities: placement::EntityPlacements,
    mut joints: JointTransforms,
    mut animations: Query<(
        &mut AnimatedMesh,
        &Mesh3d,
        Option<&mut Aabb>,
        Option<&mut MeshMorphWeights>,
    )>,
    performance: Option<Res<crate::performance::Performance>>,
    (flex_models, eyes, mut actor_poses, mut meshes): AnimationInputs,
) {
    let _timing = crate::performance::scope(performance.as_deref(), "animation");
    placement::place_entities(&mut entities, &game, &sim);
    let mut updated_joints = BTreeSet::new();
    let mut poses: BTreeMap<(String, String, u32, u64), Arc<Vec<glam::Mat4>>> = BTreeMap::new();
    let mut flex_weights = face_flex::FlexWeights::new();
    for (mut animation, handle, aabb, morph_weights) in &mut animations {
        if animation.entity.is_some() && animation.weapon.is_some() {
            continue;
        }
        let Some(rig) = game.world.rigs.get(&animation.key) else {
            continue;
        };
        // Facial flexes deform the bind positions before skinning (both GPU and CPU paths).
        face_flex::apply_facial_flexes(
            &mut animation,
            handle,
            &mut meshes,
            &face_flex::FaceInputs {
                game: &game,
                models: flex_models.as_deref(),
                eyes: eyes.as_deref(),
            },
            &mut flex_weights,
            morph_weights,
        );
        let stamp = match animation
            .entity
            .map(|id| (id, actor_poses::entity_stamp(&game, rig, id)))
        {
            Some((id, Some(stamp))) => Some((id, stamp)),
            Some((_, None)) => continue,
            None if animation.weapon.as_deref() != Some(game.inventory.active.as_str()) => continue,
            None => None,
        };
        let (clip, key, layer_key) = match &stamp {
            // Gesture layers make the pose actor-specific; the entity id keeps entries apart.
            Some((id, s)) => (
                s.clip.clone(),
                s.key,
                s.layers ^ (*id as u64).rotate_left(32),
            ),
            None => viewmodel_sample(&game, rig),
        };
        if animation
            .sampled
            .as_ref()
            .is_some_and(|(name, previous, layered)| {
                name == &clip && *previous == key && *layered == layer_key
            })
        {
            continue;
        }
        let sampled = (clip.clone(), key, layer_key);
        let matrices = poses
            .entry((animation.key.clone(), clip, key, layer_key))
            .or_insert_with(|| match &stamp {
                Some((id, s)) => actor_poses.pose(&game, (&animation.key, rig), *id, s),
                None => Arc::new(game.inventory.viewmodel_matrices(rig, game.scene.time)),
            });
        if let Some(skeleton) = &animation.gpu {
            update_gpu_joints(
                skeleton,
                rig,
                matrices,
                animation.scale,
                &mut joints,
                &mut updated_joints,
            );
            animation.sampled = Some(sampled);
            continue;
        }
        if let Some(mut mesh) = meshes.get_mut(&handle.0) {
            cpu_skin::skin_on_cpu(&mut mesh, &animation, matrices);
            if let (Some(mut aabb), Some(bounds)) = (aabb, mesh.compute_aabb()) {
                *aabb = bounds;
            }
            animation.sampled = Some(sampled);
        }
    }
}
