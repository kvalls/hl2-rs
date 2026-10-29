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
            if let (Some(base), Some(path)) = (&material.base, &material.base_path) {
                self.textures.entry(path.clone()).or_insert_with(|| {
                    images.add(image_filtered(
                        base.width,
                        base.height,
                        base.rgba.clone(),
                        true,
                        filtering,
                    ))
                });
            }
            if let (Some(iris), Some(path)) = (&material.iris, &material.iris_path) {
                self.textures.entry(path.clone()).or_insert_with(|| {
                    images.add(image_filtered(
                        iris.width,
                        iris.height,
                        iris.rgba.clone(),
                        false,
                        filtering,
                    ))
                });
            }
            strip_pixels(&mut material);
            self.definitions.entry(name).or_insert(material);
        }
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
