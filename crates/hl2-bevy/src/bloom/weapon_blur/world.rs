//! Bounded-resolution world filter; the full-resolution weapon path is unchanged.
use super::{Pipelines, Textures, draw, pipeline};
use bevy::render::{
    render_resource::{
        BindGroup, BindGroupEntries, CachedRenderPipelineId, PipelineCache, TextureFormat,
        TextureView,
    },
    renderer::{RenderContext, RenderDevice},
    view::ViewTarget,
};
use std::collections::HashMap;

pub(super) type Cache = HashMap<(TextureFormat, u32), [CachedRenderPipelineId; 2]>;

pub(super) struct Prepared {
    pub pipelines: [CachedRenderPipelineId; 2],
    pub downsample: BindGroup,
    pub native: BindGroup,
}

pub(super) fn prepare(
    p: &Pipelines,
    cache: &PipelineCache,
    device: &RenderDevice,
    textures: &Textures,
    samples: u32,
    known: &mut Cache,
    (native, frame): (BindGroup, &TextureView),
) -> Option<Prepared> {
    let output = textures.output.as_ref()?;
    let format = output.texture.format();
    let pipelines = *known.entry((format, samples)).or_insert_with(|| {
        [
            pipeline(p, cache, "world_downsample", format, 1, true),
            pipeline(p, cache, "world_objects", format, samples, true),
        ]
    });
    let layout = cache.get_bind_group_layout(&p.image_layout);
    let downsample = device.create_bind_group(
        Some("world_blur_downsample"),
        &layout,
        &BindGroupEntries::sequential((frame, &p.sampler, frame)),
    );
    Some(Prepared {
        pipelines,
        downsample,
        native,
    })
}

pub(super) fn reconstruct(
    ctx: &mut RenderContext,
    cache: &PipelineCache,
    target: &ViewTarget,
    textures: &Textures,
    prepared: &super::Prepared,
    world: &Prepared,
    composite: &BindGroup,
) {
    if prepared._buffer.get().flags.x > 0. {
        draw(
            ctx,
            cache,
            prepared.pipelines[3],
            composite,
            &textures.output.as_ref().expect("world output").default_view,
            "world_motion_blur",
        );
    }
    let write = target.post_process_write();
    draw(
        ctx,
        cache,
        world.pipelines[1],
        &world.native,
        write.destination,
        "world_blur_composite",
    );
}
