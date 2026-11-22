//! Facial flexes on the bind positions of an actor's draws, before skinning (GPU and CPU paths).
use super::AnimatedMesh;
use crate::source_to_bevy;
use bevy::{
    asset::Assets,
    math::Vec3,
    mesh::{Mesh, Mesh3d},
};
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

/// A draw's face state: the signature its bind positions were last deformed for, and the
/// vertex slot ([`source_assets::flexes::VertexSlots`]) of each of its flexed vertices, mapped
/// once (empty until then; reset when the draw's vertices change, as on a LOD switch).
#[derive(Clone, Default)]
pub(super) struct FlexState {
    signature: u64,
    slots: Vec<Option<u32>>,
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

pub(super) fn apply_facial_flexes(
    animation: &mut AnimatedMesh,
    handle: &Mesh3d,
    meshes: &mut Assets<Mesh>,
    inputs: &FaceInputs,
    flex_weights: &mut FlexWeights,
) {
    let FaceInputs { game, models, eyes } = *inputs;
    let (Some(id), Some(model)) = (
        animation.entity,
        models
            .and_then(|f| f.0.get(&animation.key))
            .filter(|_| !animation.flex.is_empty()),
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
    let deltas = actor.deltas.get_or_insert_with(|| {
        let mut deltas = Vec::new();
        model
            .flex
            .vertex_deltas(&model.slots, &actor.weights, &mut deltas);
        deltas
    });
    if animation.flex_state.slots.len() != animation.flex.len() {
        // Vertices no flex moves keep their undeformed bind position: no slot.
        animation.flex_state.slots = (animation.flex.iter())
            .map(|key| key.and_then(|key| model.slots.slot(key)))
            .collect();
    }
    let scale = animation.scale;
    let mut changed = Vec::new();
    for (i, slot) in (animation.flex_state.slots.iter().enumerate())
        .filter_map(|(i, slot)| slot.map(|slot| (i, slot as usize)))
    {
        let target = animation.flex_base[i] + deltas[slot];
        if animation.bind[i].0 != target {
            animation.bind[i].0 = target;
            changed.push(i);
        }
    }
    animation.flex_state.signature = signature;
    if changed.is_empty() {
        return;
    }
    if animation.gpu.is_some() {
        let mut mesh = meshes.get_mut(&handle.0);
        let positions = mesh
            .as_deref_mut()
            .and_then(|m| m.attribute_mut(Mesh::ATTRIBUTE_POSITION));
        if let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) = positions {
            for &i in &changed {
                if let Some(p) = positions.get_mut(i) {
                    *p = source_to_bevy(Vec3::from_array((animation.bind[i].0 * scale).to_array()))
                        .to_array();
                }
            }
        }
    } else {
        // Force the CPU skinning path to rebuild from the deformed bind.
        animation.sampled = None;
    }
}
