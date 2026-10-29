//! Color correction, host side. The simulation owns the entities and their weights
//! ([`hl2_simulation::color_correction`]); this module reads the lookup files of the map once
//! on a worker, builds one 32x32x32 volume per file and hands the weights of each frame to the
//! bloom composite ([`super::correction_pass`]). With `mat_colorcorrection` off, or on a map
//! without the entities (every base Half-Life 2 map), nothing is loaded and no component
//! exists.
use super::SourceBloom;
use crate::{gameplay::Gameplay, runtime_props::RuntimeProps, ui_host::Console};
use bevy::{
    asset::RenderAssetUsages,
    asset::{Assets, Handle},
    ecs::{
        component::Component,
        entity::Entity,
        query::With,
        resource::Resource,
        system::{Commands, Query, Res, ResMut},
    },
    image::Image,
    log::warn,
    render::{
        extract_component::ExtractComponent,
        render_resource::{Extent3d, TextureDimension, TextureFormat},
    },
};
use hl2_simulation::color_correction::Mix;
use std::sync::{Arc, Mutex};

/// A lookup file: 32 x 32 x 32 texels of RGB bytes (`CColorCorrectionSystem` refuses any other
/// size).
const SIZE: u32 = 32;
const FILE_BYTES: usize = (SIZE * SIZE * SIZE) as usize * 3;

/// The lookups and weights the bloom composite applies, on the camera that carries the bloom.
#[derive(Component, Clone, ExtractComponent)]
pub struct SourceColorCorrection {
    pub lookups: Arc<[Handle<Image>]>,
    pub mix: Mix,
}

type Files = Result<Vec<Vec<u8>>, String>;

enum State {
    Idle,
    Loading(Arc<Mutex<Option<Files>>>),
    Ready(Arc<[Handle<Image>]>),
    Failed,
}

#[derive(Resource)]
pub struct Lookups {
    /// The files `state` was started for.
    files: Vec<String>,
    state: State,
    /// Whether the camera carries a [`SourceColorCorrection`].
    applied: bool,
}
impl Default for Lookups {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            state: State::Idle,
            applied: false,
        }
    }
}

/// A lookup file as a volume texture (RGB padded to RGBA), `None` for a file of the wrong size.
pub(super) fn lookup_image(bytes: &[u8]) -> Option<Image> {
    if bytes.len() != FILE_BYTES {
        return None;
    }
    let data = bytes
        .as_chunks::<3>()
        .0
        .iter()
        .flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255])
        .collect();
    Some(Image::new(
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: SIZE,
        },
        TextureDimension::D3,
        data,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    ))
}

fn read_files(vfs: Arc<source_assets::vpk::Vfs>, files: Vec<String>) -> Files {
    files
        .iter()
        .map(|file| match vfs.read(file) {
            Ok(Some(bytes)) if bytes.len() == FILE_BYTES => Ok(bytes),
            Ok(Some(bytes)) => Err(format!(
                "color correction file {file} has bad size ({})",
                bytes.len()
            )),
            Ok(None) => Err(format!("color correction file {file} not found")),
            Err(error) => Err(format!("color correction file {file}: {error:#}")),
        })
        .collect()
}

impl Lookups {
    /// Takes the component off the camera.
    fn detach(&mut self, commands: &mut Commands, cameras: &Query<Entity, With<SourceBloom>>) {
        if self.applied {
            for camera in cameras {
                commands.entity(camera).remove::<SourceColorCorrection>();
            }
            self.applied = false;
        }
    }

    /// Drops the loaded volumes and the camera component.
    fn release(&mut self, commands: &mut Commands, cameras: &Query<Entity, With<SourceBloom>>) {
        self.detach(commands, cameras);
        if !matches!(self.state, State::Idle) {
            self.state = State::Idle;
            self.files.clear();
        }
    }

    /// Starts reading `files` unless they are what is loaded or loading.
    fn load(&mut self, files: &[String], vfs: Option<&Arc<source_assets::vpk::Vfs>>) {
        if self.files == files {
            return;
        }
        self.files = files.to_vec();
        let Some(vfs) = vfs else {
            self.state = State::Failed;
            return;
        };
        let slot: Arc<Mutex<Option<Files>>> = Arc::default();
        let (out, vfs, files) = (slot.clone(), vfs.clone(), files.to_vec());
        std::thread::spawn(move || {
            *out.lock().expect("color correction files") = Some(read_files(vfs, files));
        });
        self.state = State::Loading(slot);
    }

    /// Turns finished reads into volume textures.
    fn finish(&mut self, images: &mut Assets<Image>) {
        let State::Loading(slot) = &self.state else {
            return;
        };
        let Some(result) = slot.lock().expect("color correction files").take() else {
            return;
        };
        self.state = match result {
            Ok(files) => State::Ready(
                files
                    .iter()
                    .filter_map(|bytes| lookup_image(bytes))
                    .map(|image| images.add(image))
                    .collect(),
            ),
            Err(error) => {
                warn!("{error}");
                State::Failed
            }
        };
    }
}

/// Keeps the camera's [`SourceColorCorrection`] in step with the option and the simulation.
pub fn update(
    mut commands: Commands,
    (game, ui, props): (
        Res<Gameplay>,
        Option<Res<Console>>,
        Option<Res<RuntimeProps>>,
    ),
    mut lookups: ResMut<Lookups>,
    mut images: ResMut<Assets<Image>>,
    cameras: Query<Entity, With<SourceBloom>>,
    mut applied: Query<&mut SourceColorCorrection>,
) {
    let correction = &game.scene.color_correction;
    let enabled = ui.is_some_and(|ui| ui.source.config.video.advanced.color_correction);
    if !enabled || correction.is_empty() {
        lookups.release(&mut commands, &cameras);
        return;
    }
    lookups.load(
        correction.lookups(),
        props.as_deref().map(RuntimeProps::vfs),
    );
    lookups.finish(&mut images);
    let mix = correction.mix();
    let State::Ready(volumes) = &lookups.state else {
        // Still reading (or failed): the previous map's lookups must not stay on the camera.
        lookups.detach(&mut commands, &cameras);
        return;
    };
    let volumes = volumes.clone();
    // Without a contributing lookup the pass would return its input.
    if !mix.is_active() || volumes.len() != correction.lookups().len() {
        lookups.detach(&mut commands, &cameras);
        return;
    }
    for mut camera in &mut applied {
        if camera.mix != mix {
            camera.mix = mix;
        }
    }
    if !lookups.applied {
        for camera in &cameras {
            commands.entity(camera).insert(SourceColorCorrection {
                lookups: volumes.clone(),
                mix,
            });
        }
        lookups.applied = true;
    }
}

#[cfg(test)]
mod tests;
