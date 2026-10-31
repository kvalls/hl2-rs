//! Render integration through a small frame snapshot and camera markers.
pub mod diagnostics;
mod viewmodel_blur;
mod weapon_blur;
mod world_blur;
use bevy::prelude::{
    App, Component, IntoScheduleConfigs, Plugin, PostUpdate, Resource, SystemSet, TransformSystems,
};
pub use weapon_blur::memory::{Report as BlurMemory, install as install_memory_probe};

#[derive(Component)]
pub struct WorldCamera;
#[derive(Component)]
pub struct WeaponCamera;

/// Updated by the host after presentation; no simulation or UI implementation leaks here.
#[derive(Resource, Default)]
pub struct BlurFrame {
    pub map: usize,
    pub time: f64,
    pub weapon: String,
    pub viewmodel: bool,
    pub world: bool,
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrepareBlurSet;
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReconstructionSet;

pub struct Hl2AdvancedOptionsPlugin;
impl Plugin for Hl2AdvancedOptionsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BlurFrame>()
            .add_plugins(weapon_blur::WeaponBlurPlugin)
            .add_systems(
                PostUpdate,
                (
                    (viewmodel_blur::update, world_blur::update),
                    weapon_blur::depth::update,
                )
                    .chain()
                    .in_set(PrepareBlurSet)
                    .before(TransformSystems::Propagate),
            );
    }
}

pub mod host;
