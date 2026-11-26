//! The lightmap atlas pages and their light styles. The first frame composes every page holding
//! faces lit by a style other than 0 (a named light's TurnOn/TurnOff, SDK lights.cpp; an
//! animated pattern) from their baked style samples. After that only the faces whose styles
//! changed value are composed, and the render world writes their rectangles into the uploaded
//! pages, so a style animating at 10 Hz costs its own faces, not whole 1024 x 1024 pages.
//! Nothing runs while the values stay the same.
use bevy::asset::{AssetId, RenderAssetUsages};
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::prelude::{App, Assets, Handle, Image, Local, Plugin, Res, ResMut, Resource};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    Extent3d, Origin3d, TexelCopyBufferLayout, TexelCopyTextureInfo, TextureAspect,
    TextureDimension, TextureFormat,
};
use bevy::render::renderer::RenderQueue;
use bevy::render::texture::GpuImage;
use bevy::render::{Render, RenderApp, RenderSystems};
use std::collections::BTreeSet;
use std::sync::Arc;

/// RGBA16F.
const TEXEL_BYTES: u32 = 8;

/// The map's atlas pages, the style generation last composed into them and the style values
/// they hold.
#[derive(Resource, Default)]
pub struct LightmapPages {
    handles: Vec<Handle<Image>>,
    applied: Option<u64>,
    values: Vec<f32>,
}

/// One face rectangle (with its border) to write into an uploaded page.
struct FaceWrite {
    page: AssetId<Image>,
    origin: (u32, u32),
    size: (u32, u32),
    bytes: Vec<u8>,
}

/// The latest face writes, extracted to the render world; `serial` tells new ones apart.
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(bevy::render::RenderApp)]
pub struct LightmapWrites {
    serial: u64,
    faces: Arc<Vec<FaceWrite>>,
}

fn bytes(rgba: &[u16]) -> Vec<u8> {
    rgba.iter().flat_map(|t| t.to_le_bytes()).collect()
}

fn image(lightmap: &modkit_core::Lightmap) -> Image {
    Image::new(
        Extent3d {
            width: lightmap.width.into(),
            height: lightmap.height.into(),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        bytes(&lightmap.rgba),
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
        values: Vec::new(),
    }
}

impl LightmapPages {
    pub(super) fn handles(&self) -> &[Handle<Image>] {
        &self.handles
    }
}

/// Recomposes the styled faces when the scene's light styles changed since the last frame:
/// whole pages the first time, then the changed faces' rectangles.
pub fn apply_lightstyles(
    game: Res<crate::gameplay::Gameplay>,
    pages: Option<ResMut<LightmapPages>>,
    mut images: ResMut<Assets<Image>>,
    writes: Option<ResMut<LightmapWrites>>,
    performance: Option<Res<crate::performance::Performance>>,
) {
    let Some(mut pages) = pages else {
        return;
    };
    let styles = &game.scene.lightstyles;
    let world = &game.world;
    if pages.applied == Some(styles.generation()) || pages.handles.is_empty() {
        return;
    }
    let _timing = crate::performance::scope(performance.as_deref(), "light styles");
    // Without the render-world writer (headless apps) every change recomposes whole pages.
    let partial = writes.filter(|_| pages.applied.is_some());
    pages.applied = Some(styles.generation());
    let values: Vec<f32> = (0..=u8::MAX).map(|s| styles.value(s)).collect();
    let changed =
        |style: u8| pages.values.get(usize::from(style)) != Some(&values[usize::from(style)]);
    if let Some(mut writes) = partial {
        let faces: Vec<FaceWrite> = (world.lightmap_styles.iter())
            .filter(|f| f.styles.iter().any(|(s, _)| changed(*s)))
            .filter_map(|f| {
                Some(FaceWrite {
                    page: pages.handles.get(f.page)?.id(),
                    origin: (f.x as u32 - 1, f.y as u32 - 1),
                    size: (f.width as u32 + 2, f.height as u32 + 2),
                    bytes: bytes(&source_assets::lighting::compose_face(f, |s| {
                        styles.value(s)
                    })),
                })
            })
            .collect();
        if !faces.is_empty() {
            writes.serial += 1;
            writes.faces = Arc::new(faces);
        }
    } else {
        let touched: BTreeSet<usize> = world.lightmap_styles.iter().map(|f| f.page).collect();
        for page in touched {
            let (Some(base), Some(handle)) = (world.lightmaps.get(page), pages.handles.get(page))
            else {
                continue;
            };
            let mut lightmap = base.clone();
            source_assets::lighting::compose_styles(
                &mut lightmap,
                page,
                &world.lightmap_styles,
                |s| styles.value(s),
            );
            let _ = images.insert(handle, image(&lightmap));
        }
    }
    pages.values = values;
}

/// Writes the newest face rectangles into the uploaded pages (render world, after the image
/// assets are prepared).
fn write_faces(
    writes: Res<LightmapWrites>,
    mut written: Local<u64>,
    images: Res<RenderAssets<GpuImage>>,
    queue: Res<RenderQueue>,
) {
    if writes.serial == *written {
        return;
    }
    *written = writes.serial;
    for face in writes.faces.iter() {
        let Some(page) = images.get(face.page) else {
            continue;
        };
        queue.write_texture(
            TexelCopyTextureInfo {
                texture: &page.texture,
                mip_level: 0,
                origin: Origin3d {
                    x: face.origin.0,
                    y: face.origin.1,
                    z: 0,
                },
                aspect: TextureAspect::All,
            },
            &face.bytes,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(face.size.0 * TEXEL_BYTES),
                rows_per_image: None,
            },
            Extent3d {
                width: face.size.0,
                height: face.size.1,
                depth_or_array_layers: 1,
            },
        );
    }
}

/// The face writes' resource, its extraction and the render-world writer.
pub struct LightmapWritesPlugin;
impl Plugin for LightmapWritesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LightmapWrites>()
            .add_plugins(ExtractResourcePlugin::<LightmapWrites>::default());
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.add_systems(Render, write_faces.in_set(RenderSystems::PrepareResources));
        }
    }
}
