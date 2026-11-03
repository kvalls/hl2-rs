//! Per-model face data read with the map: eyeball records for every model, and for NPC models
//! their facial flexes and gaze attachment.
use modkit_core::World;
use source_assets::{eyes, flexes, vpk::Vfs};
use std::{collections::BTreeMap, sync::Arc};

/// The map's face data by model asset key.
#[derive(Default)]
pub(super) struct Faces {
    pub eyes: BTreeMap<(String, String), eyes::Eyeball>,
    pub gaze: BTreeMap<String, eyes::Gaze>,
    pub flexes: BTreeMap<String, Arc<flexes::FaceModel>>,
}

/// Reads the face data of the world's model assets; failures go to `errors`.
pub(super) fn load(vfs: &Vfs, world: &World, errors: &mut Vec<String>) -> Faces {
    let mut out = Faces::default();
    for (key, surfaces) in &world.model_assets {
        let model = key.split('#').next().unwrap_or(key);
        let mut face = None;
        if world.model_instances.iter().any(|i| {
            i.asset_key() == *key
                && i.entity
                    .is_some_and(|id| world.entities[id].class().starts_with("npc_"))
        }) {
            match vfs
                .read(model)
                .and_then(|d| d.ok_or_else(|| anyhow::anyhow!("model missing")))
                .and_then(|d| flexes::read_flexes(&d))
            {
                Ok(model) if !model.meshes.is_empty() => {
                    face = Some(model);
                }
                Ok(_) => {}
                Err(e) => errors.push(format!("{model}: facial flexes: {e:#}")),
            }
            match eyes::load_gaze(vfs, model) {
                Ok(metadata) => {
                    out.gaze.insert(key.clone(), metadata);
                }
                Err(e) => errors.push(format!("{model}: gaze attachment metadata: {e:#}")),
            }
        }
        match eyes::load(vfs, model) {
            Ok(records) => {
                if let Some(flex) = face.take() {
                    out.flexes.insert(
                        key.clone(),
                        Arc::new(flexes::FaceModel {
                            flex,
                            eyes: records.clone(),
                        }),
                    );
                }
                for eye in records {
                    if let Some(surface) = surfaces.get(eye.surface) {
                        out.eyes
                            .insert((key.clone(), surface.material.clone()), eye);
                    } else {
                        errors.push(format!("{model}: eyeball mesh missing"));
                    }
                }
            }
            Err(e) => errors.push(format!("{model}: eyeball metadata: {e:#}")),
        }
    }
    out
}
