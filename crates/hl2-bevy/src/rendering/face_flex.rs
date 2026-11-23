//! Facial flexes on the bind positions of an actor's draws, before skinning (GPU and CPU paths).
mod morph;
use super::AnimatedMesh;
use crate::source_to_bevy;
use bevy::{
    asset::Assets,
    math::Vec3,
    mesh::{Mesh, Mesh3d},
};
pub(super) use morph::{MorphChannels, apply_morph_channels, build_morph_targets};
use source_assets::flexes::FaceModel;
use std::collections::HashMap;

/// Facial flex and eyeball data per model asset key (setup-loaded; no frame IO).
#[derive(bevy::prelude::Resource, Default)]
pub struct FlexModels(
    pub std::collections::BTreeMap<String, std::sync::Arc<source_assets::flexes::FaceModel>>,
);

/// One frame's face state of an actor and model, shared by all of the actor's surfaces.
pub(super) struct ActorFlex {
    weights: Vec<f32>,
    signature: u64,
    /// Deltas by vertex slot, built when the first surface whose deformation is out of date
    /// asks for them.
    deltas: Option<Vec<glam::Vec3>>,
}
pub(super) type FlexWeights = HashMap<(usize, *const FaceModel), ActorFlex>;

/// A draw's face state: the signature it was last deformed for, its `(vertex, flex_slot)` pairs
/// mapped once, and the last deformed bind positions (empty while at bind pose; reset on LOD switch).
#[derive(Clone, Default)]
pub(super) struct FlexState {
    signature: u64,
    mapped: bool,
    slots: Vec<(usize, usize)>,
    applied: Vec<glam::Vec3>,
}

/// What the draws of this frame deform from.
pub(super) struct FaceInputs<'a> {
    pub game: &'a crate::gameplay::Gameplay,
    pub models: Option<&'a FlexModels>,
    pub eyes: Option<&'a crate::eyes::Eyes>,
}

/// Descriptor weights quantized to 1/1024: deformation is rebuilt only when it changes.
fn flex_signature(weights: &[f32]) -> u64 {
    weights.iter().fold(0x9e37u64, |hash, w| {
        (hash ^ (w * 1024.).round() as i64 as u64).wrapping_mul(0x100000001b3)
    })
}

fn changed_vertices(
    state: &mut FlexState,
    bind: &[(glam::Vec3, Option<modkit_core::animation::Weights>)],
    deltas: &[glam::Vec3],
) -> Vec<(usize, glam::Vec3)> {
    let mut changed = Vec::new();
    if state.applied.is_empty() {
        for (idx, &(i, slot)) in state.slots.iter().enumerate() {
            let target = bind[i].0 + deltas[slot];
            if bind[i].0 != target {
                if state.applied.is_empty() {
                    state.applied = state.slots.iter().map(|&(v, _)| bind[v].0).collect();
                }
                state.applied[idx] = target;
                changed.push((i, target));
            }
        }
    } else {
        for (idx, &(i, slot)) in state.slots.iter().enumerate() {
            let target = bind[i].0 + deltas[slot];
            if state.applied[idx] != target {
                state.applied[idx] = target;
                changed.push((i, target));
            }
        }
    }
    changed
}

pub(super) fn apply_facial_flexes(
    animation: &mut AnimatedMesh,
    handle: &Mesh3d,
    meshes: &mut Assets<Mesh>,
    inputs: &FaceInputs,
    flex_weights: &mut FlexWeights,
    morph_weights: Option<bevy::prelude::Mut<'_, bevy::mesh::morph::MeshMorphWeights>>,
) {
    let FaceInputs { game, models, eyes } = *inputs;
    let (Some(id), Some(model)) = (
        animation.entity,
        models
            .and_then(|f| f.0.get(&animation.key))
            .filter(|_| !animation.vertices.flex.is_empty()),
    ) else {
        return;
    };
    let actor = flex_weights
        .entry((id, std::sync::Arc::as_ptr(model)))
        .or_insert_with(|| {
            let values = game.scene.actor_flex_values(id);
            let weights = model.descriptor_weights(&values, |eye| {
                eyes.and_then(|e| e.lid_bases.get(&(id, eye.surface)))
                    .copied()
            });
            ActorFlex {
                signature: flex_signature(&weights),
                weights,
                deltas: None,
            }
        });
    let signature = actor.signature;
    if signature == animation.flex_state.signature {
        return;
    }
    if let Some(channels) = &animation.morph {
        if let Some(mut morph_weights) = morph_weights
            && let bevy::mesh::morph::MeshMorphWeights::Value { weights } = &mut *morph_weights
        {
            channels.write_weights(&actor.weights, weights);
        }
        animation.flex_state.signature = signature;
        return;
    }
    let deltas = actor.deltas.get_or_insert_with(|| {
        let mut deltas = Vec::new();
        model
            .flex
            .vertex_deltas(&model.slots, &actor.weights, &mut deltas);
        deltas
    });
    if !animation.flex_state.mapped {
        // Vertices no flex moves keep their undeformed bind position: no slot.
        animation.flex_state.slots = animation
            .vertices
            .flex
            .iter()
            .enumerate()
            .filter_map(|(i, key)| {
                let slot = (*key).and_then(|key| model.slots.slot(key))?;
                Some((i, slot as usize))
            })
            .collect();
        animation.flex_state.mapped = true;
    }
    let changed = changed_vertices(&mut animation.flex_state, &animation.vertices.bind, deltas);
    animation.flex_state.signature = signature;
    if changed.is_empty() {
        return;
    }
    let scale = animation.scale;
    if animation.gpu.is_some() {
        let mut mesh = meshes.get_mut(&handle.0);
        let positions = mesh
            .as_deref_mut()
            .and_then(|m| m.attribute_mut(Mesh::ATTRIBUTE_POSITION));
        if let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) = positions {
            for &(i, target) in &changed {
                if let Some(p) = positions.get_mut(i) {
                    *p = source_to_bevy(Vec3::from_array((target * scale).to_array())).to_array();
                }
            }
        }
    } else {
        let deformed = animation
            .deformed
            .get_or_insert_with(|| animation.vertices.bind.iter().map(|(p, _)| *p).collect());
        for &(i, target) in &changed {
            deformed[i] = target;
        }
        // Force the CPU skinning path to rebuild from the deformed bind.
        animation.sampled = None;
    }
}
