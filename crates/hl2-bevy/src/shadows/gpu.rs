//! Experimental render-world silhouette pass. Geometry crosses extraction once per dirty cell.
mod batch;
mod raster;
#[cfg(test)]
mod tests;

use super::MapShadows;
pub(super) use batch::Batch;
use bevy::{
    app::App,
    asset::AssetId,
    ecs::schedule::IntoScheduleConfigs,
    image::Image,
    prelude::{Res, ResMut, Resource},
    render::{
        ExtractSchedule, MainWorld, RenderApp,
        render_asset::RenderAssets,
        renderer::{RenderDevice, RenderGraph, RenderGraphSystems, RenderQueue},
        texture::GpuImage,
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

/// Present when silhouettes are drawn on the GPU: the default whenever a renderer exists.
/// `HL2RS_GPU_SILHOUETTES=0` keeps the CPU rasterizer (reference and fallback).
#[derive(Resource)]
pub(crate) struct GpuSilhouettes;

pub(super) fn install(app: &mut App) {
    if std::env::var("HL2RS_GPU_SILHOUETTES").is_ok_and(|v| v == "0") {
        return;
    }
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render
            .init_resource::<Pending>()
            .add_systems(ExtractSchedule, extract)
            .add_systems(RenderGraph, draw.in_set(RenderGraphSystems::Begin));
        app.insert_resource(GpuSilhouettes);
    }
}

#[derive(Resource, Default)]
struct Pending {
    atlas: Option<AssetId<Image>>,
    batch: Batch,
    submitted: Option<Arc<AtomicU64>>,
    raster: Option<raster::Raster>,
}

fn extract(mut world: ResMut<MainWorld>, mut pending: ResMut<Pending>) {
    let mut shadows = world.get_resource_mut::<MapShadows>();
    let atlas = shadows
        .as_ref()
        .and_then(|s| s.textures.atlas_image.as_ref())
        .map(|h| h.id());
    if atlas != pending.atlas {
        pending.batch.clear();
        pending.atlas = atlas;
        pending.submitted = shadows.as_ref().and_then(|s| s.textures.submitted.clone());
        if let Some(raster) = &mut pending.raster {
            raster.reset_materials();
        }
    }
    if let Some(shadows) = shadows.as_deref_mut()
        && let Some(batch) = shadows.textures.batch.as_mut()
        && !batch.cells.is_empty()
    {
        // If preparation lags, recompute the visible cells on the next update. Never
        // accumulate an unbounded stream of old poses while waiting for the image.
        if !pending.batch.cells.is_empty() {
            pending.batch.clear();
            for caster in &mut shadows.casters {
                caster.dirty = true;
            }
        }
        std::mem::swap(batch, &mut pending.batch);
    }
}

fn draw(
    mut pending: ResMut<Pending>,
    images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    if pending.batch.cells.is_empty() {
        return;
    }
    let Some(image) = pending.atlas.and_then(|id| images.get(id)) else {
        return;
    };
    let Pending {
        batch,
        raster,
        submitted,
        ..
    } = &mut *pending;
    raster
        .get_or_insert_with(|| raster::Raster::new(&device))
        .draw(&device, &queue, &image.texture, &image.texture_view, batch);
    if let Some(count) = submitted {
        count.fetch_add(batch.cells.len() as u64, Ordering::Relaxed);
    }
    batch.clear();
}
