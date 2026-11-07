//! The GPU textures and material definitions of the loaded map, kept for models drawn after the
//! map load.

use super::{MaterialData, image_filtered};
use bevy::{
    asset::{Assets, Handle},
    ecs::resource::Resource,
    image::Image,
};
use std::collections::BTreeMap;

/// The loaded map's GPU textures and material definitions, kept for models drawn after
/// the map load (DESIGN 14r part B): texture handles by VTF path (shared with the map's
/// draws) and definitions without their decoded pixels.
#[derive(Resource, Default)]
pub struct ModelCache {
    pub textures: BTreeMap<String, Handle<Image>>,
    pub definitions: BTreeMap<String, MaterialData>,
    pub white: Handle<Image>,
    pub black_cube: Handle<Image>,
    pub flashlight: Handle<Image>,
    pub missing: Handle<Image>,
    /// Plain materials (a video option): no reflections, as for the map's own draws.
    pub plain: bool,
}
impl ModelCache {
    /// Upload a late-loaded model's materials: textures not on the GPU yet, then the
    /// definitions without pixels.
    pub fn add_materials(
        &mut self,
        images: &mut Assets<Image>,
        loaded: BTreeMap<String, MaterialData>,
    ) {
        self.add_materials_filtered(images, loaded, hl2_ui::config::Filtering::Trilinear);
    }

    pub fn add_materials_filtered(
        &mut self,
        images: &mut Assets<Image>,
        loaded: BTreeMap<String, MaterialData>,
        filtering: hl2_ui::config::Filtering,
    ) {
        for (name, mut material) in loaded {
            type Decoded = Option<std::sync::Arc<source_assets::vtf::Image>>;
            let mut upload = |image: &Decoded, path: &Option<String>, repeat: bool| {
                let (Some(image), Some(path)) = (image, path) else {
                    return;
                };
                self.textures.entry(path.clone()).or_insert_with(|| {
                    let rgba = image.rgba.clone();
                    images.add(image_filtered(
                        image.width,
                        image.height,
                        rgba,
                        repeat,
                        filtering,
                    ))
                });
            };
            upload(&material.base, &material.base_path, true);
            upload(&material.iris, &material.iris_path, false);
            // The mask uses the base UVs, so it repeats as the base does.
            upload(&material.envmap_mask, &material.envmap_mask_path, true);
            strip_pixels(&mut material);
            self.definitions.entry(name).or_insert(material);
        }
    }
}
impl ModelCache {
    /// A late model's light state. It reflects the sample near it when the map's own draws
    /// would: an `env_cubemap` material whose mask, when it names one, is on the GPU, since a
    /// reflection without its mask is far too strong.
    pub(super) fn model_lit(
        &self,
        id: usize,
        instance: &modkit_core::ModelInstance,
        definition: &MaterialData,
    ) -> super::lighting::ModelLit {
        let lit = super::lighting::ModelLit::new(
            Some(id),
            instance.asset_key(),
            instance.scale,
            definition.half_lambert,
        );
        lit.reflecting(self.reflects(definition))
    }
    /// Whether a late model's material reflects its sample.
    pub(super) fn reflects(&self, definition: &MaterialData) -> bool {
        let mask = definition.envmap_mask_path.as_ref();
        definition.model_envmap && !self.plain && mask.is_none_or(|p| self.textures.contains_key(p))
    }
}

/// A material definition without decoded images (their GPU copies stay in ModelCache).
pub(super) fn strip_pixels(material: &mut MaterialData) {
    material.base = None;
    material.iris = None;
    material.camera_overlay = None;
    material.envmap = None;
    material.envmap_mask = None;
    material.two_texture = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_late_model_reflects_with_its_mask_on_the_gpu_and_never_with_plain_materials() {
        let mut cache = ModelCache::default();
        let mut definition = MaterialData {
            model_envmap: true,
            ..Default::default()
        };
        assert!(cache.reflects(&definition), "no mask named: full strength");
        definition.envmap_mask_path = Some("materials/models/m_mask.vtf".into());
        assert!(!cache.reflects(&definition), "a mask not uploaded yet");
        let path = "materials/models/m_mask.vtf".to_owned();
        cache.textures.insert(path, Handle::default());
        assert!(cache.reflects(&definition));
        cache.plain = true;
        assert!(!cache.reflects(&definition));
        cache.plain = false;
        definition.model_envmap = false;
        assert!(!cache.reflects(&definition));
    }
}
