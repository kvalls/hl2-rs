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

pub(super) type Cache = HashMap<(TextureFormat, u32), [CachedRenderPipelineId; 4]>;

pub(super) struct Prepared {
    pub pipelines: [CachedRenderPipelineId; 4],
    pub gather: BindGroup,
    pub downsample: BindGroup,
    pub environment: BindGroup,
    pub native: BindGroup,
}

pub(super) fn prepare(
    p: &Pipelines,
    cache: &PipelineCache,
    device: &RenderDevice,
    textures: &Textures,
    samples: u32,
    known: &mut Cache,
    (native, gather, frame): (BindGroup, BindGroup, &TextureView),
) -> Option<Prepared> {
    let output = textures.output.as_ref()?;
    let format = output.texture.format();
    let pipelines = *known.entry((format, samples)).or_insert_with(|| {
        [
            pipeline(p, cache, "world_downsample", format, 1, true),
            pipeline(p, cache, "world_objects", format, samples, true),
            pipeline(
                p,
                cache,
                "world_environment",
                super::prepare::ENVIRONMENT_FORMAT,
                1,
                true,
            ),
            pipeline(p, cache, "world_object_gather", format, samples, true),
        ]
    });
    let layout = cache.get_bind_group_layout(&p.image_layout);
    let downsample = device.create_bind_group(
        Some("world_blur_downsample"),
        &layout,
        &BindGroupEntries::sequential((frame, &p.sampler, frame)),
    );
    let image = &textures.background.default_view;
    let environment = device.create_bind_group(
        Some("world_blur_environment"),
        &layout,
        &BindGroupEntries::sequential((image, &p.sampler, image)),
    );
    Some(Prepared {
        pipelines,
        downsample,
        environment,
        gather,
        native,
    })
}

/// The quarter-size frame the camera gather reads, and its wide local luminance.
pub(super) fn prepare_images(
    ctx: &mut RenderContext,
    cache: &PipelineCache,
    textures: &Textures,
    world: &Prepared,
) {
    draw(
        ctx,
        cache,
        world.pipelines[0],
        &world.downsample,
        &textures.background.default_view,
        "world_blur_downsample",
    );
    draw(
        ctx,
        cache,
        world.pipelines[2],
        &world.environment,
        &textures
            .environment
            .as_ref()
            .expect("world environment")
            .default_view,
        "world_blur_environment",
    );
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
    draw(
        ctx,
        cache,
        world.pipelines[3],
        &world.gather,
        &textures
            .object_gathered
            .as_ref()
            .expect("world object gather")
            .default_view,
        "world_object_gather",
    );
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
