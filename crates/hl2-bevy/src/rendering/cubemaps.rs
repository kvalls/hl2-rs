//! Reflections on models (DESIGN 14o session 48). A lit model has its own material, so each can
//! hold the cubemap sample near it; `present_lighting` binds one when a model first draws and again
//! when it has moved, so a model that stands still costs nothing.
use super::{SourceMaterial, material::cube_image_filtered};
use crate::assets::cubemaps::{Sample, nearest};
use bevy::prelude::{Assets, Handle, Image, Resource};
use glam::Vec3;
use std::collections::BTreeMap;

/// The map's cubemap samples, uploaded: where each was built and its texture.
#[derive(Resource, Default)]
pub struct ModelCubemaps(pub Vec<(Vec3, Handle<Image>)>);

impl ModelCubemaps {
    /// The sample a model at `at` reflects.
    pub fn pick(&self, at: Vec3) -> Option<(usize, Handle<Image>)> {
        let index = nearest(self.0.iter().map(|(origin, _)| *origin), at)?;
        Some((index, self.0[index].1.clone()))
    }
}

/// Uploads the map's samples, each once.
pub(super) fn upload(
    samples: &[Sample],
    images: &mut Assets<Image>,
    handles: &mut BTreeMap<String, Handle<Image>>,
    filtering: hl2_ui::config::Filtering,
) -> ModelCubemaps {
    let mut upload = |sample: &Sample| {
        let cube = &sample.cube;
        let handle = handles
            .entry(format!("{}#cube", sample.path))
            .or_insert_with(|| {
                images.add(cube_image_filtered(
                    cube.size,
                    cube.hdr,
                    cube.faces.concat(),
                    filtering,
                ))
            });
        (sample.origin, handle.clone())
    };
    ModelCubemaps(samples.iter().map(&mut upload).collect())
}

/// What a material's reflection comes from.
pub(super) struct Envmaps<'a> {
    pub world: &'a BTreeMap<String, Handle<Image>>,
    pub masks: &'a BTreeMap<String, Handle<Image>>,
    pub plain: bool,
}

impl Envmaps<'_> {
    /// World and brush surfaces carry their own envmap. A model's `env_cubemap` only takes its
    /// mask here: `present_lighting` binds the sample and switches the reflection on.
    pub fn bind(&self, material: &mut SourceMaterial, name: &str, model: bool, env_cubemap: bool) {
        if self.plain {
            return;
        }
        let own = self.world.get(name).filter(|_| !model);
        if let Some(envmap) = own {
            material.envmap = envmap.clone();
            material.envmap_tint.w = 1.;
        }
        let mask = self
            .masks
            .get(name)
            .filter(|_| own.is_some() || env_cubemap);
        if let Some(mask) = mask {
            material.envmap_mask = mask.clone();
        }
    }
}
