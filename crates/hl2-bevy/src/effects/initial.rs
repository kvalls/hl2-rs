//! Effects' state before anything is drawn.
use super::{EffectMaterial, Effects, ModelPart, ProjectileVisuals};
use bevy::prelude::Handle;
use std::collections::BTreeMap;

impl Effects {
    /// Nothing drawn yet: the map's sprite and decal materials and its grenade meshes.
    pub(super) fn new(
        materials: BTreeMap<String, Handle<EffectMaterial>>,
        grenade_meshes: BTreeMap<String, Vec<ModelPart>>,
        model_errors: Vec<String>,
    ) -> Self {
        Self {
            source: ProjectileVisuals::empty(),
            materials,
            quads: Vec::new(),
            marks: BTreeMap::new(),
            grenade_meshes,
            grenades: BTreeMap::new(),
            stuck: BTreeMap::new(),
            viewmodel_quads: Vec::new(),
            viewmodel_draws: 0,
            rng: 0x2545_f491_4f6c_dd1d,
            viewmodel_sway: Default::default(),
            viewmodel_camera: glam::Vec3::ZERO,
            load_sparks: f64::NEG_INFINITY,
            launch_sparks: f64::NEG_INFINITY,
            shot_serial: 0,
            player_flash: f64::NEG_INFINITY,
            player_muzzle_lights: 0,
            elight_time: 0.,
            model_errors,
            draws: 0,
            grenade_draws: 0,
            mark_draws: 0,
            pose_mismatches: 0,
            observed_decals: 0,
            observed_grenades: 0,
            gibs: BTreeMap::new(),
            gib_state: BTreeMap::new(),
            gib_draws: 0,
        }
    }
}
