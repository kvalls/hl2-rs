//! The lightmap atlas pages and their light styles: a page holding faces lit by a switchable
//! style (a named light's TurnOn/TurnOff, SDK lights.cpp) is recomposed from the baked style
//! samples whenever the scene's style values change, then uploaded again. Nothing runs while
//! the values stay the same.
use bevy::asset::RenderAssetUsages;
use bevy::prelude::{Assets, Handle, Image, Res, ResMut, Resource};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::BTreeSet;

/// The map's atlas pages and the style generation last composed into them.
#[derive(Resource, Default)]
pub struct LightmapPages {
    handles: Vec<Handle<Image>>,
    applied: Option<u64>,
}

fn image(lightmap: &modkit_core::Lightmap) -> Image {
    Image::new(
        Extent3d {
            width: lightmap.width.into(),
            height: lightmap.height.into(),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        lightmap.rgba.iter().flat_map(|t| t.to_le_bytes()).collect(),
        TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Uploads the map's baked pages; styled faces are composed on the first frame.
pub(super) fn upload(world: &modkit_core::World, images: &mut Assets<Image>) -> LightmapPages {
    LightmapPages {
        handles: world
            .lightmaps
            .iter()
            .map(|lm| images.add(image(lm)))
            .collect(),
        applied: None,
    }
}

impl LightmapPages {
    pub(super) fn handles(&self) -> &[Handle<Image>] {
        &self.handles
    }
}

/// Recomposes the styled pages when the scene's light styles changed since the last upload.
pub fn apply_lightstyles(
    game: Res<crate::gameplay::Gameplay>,
    pages: Option<ResMut<LightmapPages>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(mut pages) = pages else {
        return;
    };
    let styles = &game.scene.lightstyles;
    let world = &game.world;
    if pages.applied == Some(styles.generation()) || pages.handles.is_empty() {
        return;
    }
    pages.applied = Some(styles.generation());
    let touched: BTreeSet<usize> = world.lightmap_styles.iter().map(|f| f.page).collect();
    for page in touched {
        let (Some(base), Some(handle)) = (world.lightmaps.get(page), pages.handles.get(page))
        else {
            continue;
        };
        let mut lightmap = base.clone();
        source_assets::lighting::compose_styles(&mut lightmap, page, &world.lightmap_styles, |s| {
            styles.value(s)
        });
        let _ = images.insert(handle, image(&lightmap));
    }
}
