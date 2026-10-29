//! Cached textures and both resolved colour sources; no steady-frame allocations.
use super::{Data, FrameGroups, Pipelines, Prepared, Textures, WeaponBlur, pipeline};
use bevy::{
    core_pipeline::prepass::{PreviousViewData, ViewPrepassTextures},
    ecs::{
        entity::Entity,
        query::{Or, With, Without},
        system::{Commands, Local, Query, Res},
    },
    math::{Mat4, Vec4},
    prelude::Msaa,
    render::{
        camera::ExtractedCamera,
        render_resource::{
            BindGroup, BindGroupEntries, BindGroupLayout, BindingResource, CachedRenderPipelineId,
            Extent3d, PipelineCache, TextureDescriptor, TextureDimension, TextureFormat,
            TextureUsages, TextureView, TextureViewId, UniformBuffer,
        },
        renderer::{RenderDevice, RenderQueue},
        texture::CachedTexture,
        view::{ExtractedView, ViewDepthTexture, ViewTarget},
    },
};
use std::collections::HashMap;
type Stale = (
    Or<(With<Textures>, With<Prepared>, With<Key>)>,
    Without<WeaponBlur>,
);
type TextureViewData<'w> = (
    Entity,
    &'w ExtractedCamera,
    &'w ExtractedView,
    &'w WeaponBlur,
    Option<&'w Textures>,
);
pub(super) fn textures(
    mut commands: Commands,
    device: Res<RenderDevice>,
    views: Query<TextureViewData>,
    stale: Query<Entity, Stale>,
) {
    for entity in &stale {
        commands
            .entity(entity)
            .remove::<(Textures, Prepared, Key)>();
    }
    for (entity, camera, view, blur, current) in &views {
        let Some(size) = camera.physical_viewport_size else {
            continue;
        };
        if current.is_some_and(|t| {
            t.world == blur.world && t.size == size && t.format == view.target_format
        }) {
            continue;
        }
        let texture = |width, height, format, usage| {
            let texture = device.create_texture(&TextureDescriptor {
                label: Some("weapon_motion_texture"),
                size: Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            });
            let default_view = texture.create_view(&Default::default());
            CachedTexture {
                texture,
                default_view,
            }
        };
        let copied = TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING;
        let filtered = TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING;
        let background = if blur.world {
            texture(
                size.x.div_ceil(4),
                size.y.div_ceil(4),
                view.target_format,
                filtered,
            )
        } else {
            texture(size.x, size.y, view.target_format, copied)
        };
        let reduced = [4, 32, 32].map(|factor| {
            texture(
                size.x.div_ceil(factor),
                size.y.div_ceil(factor),
                TextureFormat::Rgba16Float,
                TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            )
        });
        commands.entity(entity).insert(Textures {
            background,
            size,
            format: view.target_format,
            reduced,
            world: blur.world,
            output: blur.world.then(|| {
                texture(
                    size.x.div_ceil(4),
                    size.y.div_ceil(4),
                    view.target_format,
                    filtered,
                )
            }),
        });
    }
}

#[derive(bevy::ecs::component::Component)]
pub(super) struct Key {
    sources: [TextureViewId; 2],
    textures: [TextureViewId; 7],
    samples: u32,
    world: bool,
}
fn same_pair<T: PartialEq>(a: &[T; 2], b: &[T; 2]) -> bool {
    a == b || (a[0] == b[1] && a[1] == b[0])
}
impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        same_pair(&self.sources, &other.sources)
            && self.textures == other.textures
            && self.samples == other.samples
            && self.world == other.world
    }
}
type View<'w> = (
    Entity,
    &'w WeaponBlur,
    &'w Textures,
    &'w Msaa,
    &'w ExtractedView,
    &'w ViewTarget,
    &'w ViewPrepassTextures,
    &'w ViewDepthTexture,
    Option<&'w PreviousViewData>,
    Option<&'w Key>,
    Option<&'w mut Prepared>,
);
pub(super) fn groups(
    mut commands: Commands,
    (p, cache): (Res<Pipelines>, Res<PipelineCache>),
    (device, queue): (Res<RenderDevice>, Res<RenderQueue>),
    mut known: Local<HashMap<(TextureFormat, u32, bool), [CachedRenderPipelineId; 4]>>,
    mut image_pipelines: Local<super::world::Cache>,
    mut views: Query<View>,
) {
    for (
        entity,
        blur,
        textures,
        msaa,
        view,
        target,
        prepass,
        main_depth,
        previous,
        old,
        prepared,
    ) in &mut views
    {
        let Some(vectors) = prepass.motion_vectors_view() else {
            continue;
        };
        let depth = main_depth.view();
        let sources = [target.main_texture_view(), target.main_texture_other_view()];
        let key = texture_key(
            textures,
            vectors,
            depth,
            sources.map(TextureView::id),
            msaa.samples(),
        );
        let data = view_data(blur, textures, view, previous);
        if old == Some(&key)
            && let Some(mut prepared) = prepared
        {
            if prepared._buffer.get().params != data.params
                || prepared._buffer.get().reprojection != data.reprojection
                || prepared._buffer.get().flags != data.flags
            {
                prepared._buffer.set(data);
                prepared._buffer.write_buffer(&device, &queue);
            }
            continue;
        }
        let pipelines = *known
            .entry((view.target_format, msaa.samples(), blur.world))
            .or_insert_with(|| {
                pipeline_set(&p, &cache, view.target_format, msaa.samples(), blur.world)
            });
        let mut buffer = UniformBuffer::from(data);
        buffer.write_buffer(&device, &queue);
        let binding = buffer.binding().expect("written buffer");
        let raw = cache.get_bind_group_layout(&p.raw_layouts[usize::from(msaa.samples() > 1)]);
        let reduce = cache.get_bind_group_layout(&p.reduce_layout);
        let groups = [
            device.create_bind_group(
                Some("weapon_reduce_raw"),
                &raw,
                &BindGroupEntries::sequential((vectors, depth, binding.clone())),
            ),
            device.create_bind_group(
                Some("weapon_reduce_tiles"),
                &reduce,
                &BindGroupEntries::sequential((&textures.reduced[0].default_view, binding.clone())),
            ),
            device.create_bind_group(
                Some("weapon_dilate"),
                &reduce,
                &BindGroupEntries::sequential((&textures.reduced[1].default_view, binding.clone())),
            ),
        ];
        let frames = sources.map(|frame| {
            frame_groups(
                (&device, &p, &cache),
                textures,
                (frame, vectors, depth, binding.clone()),
                msaa.samples(),
                &mut image_pipelines,
            )
        });
        commands.entity(entity).insert((
            key,
            Prepared {
                pipelines,
                groups,
                frames,
                _buffer: buffer,
            },
        ));
    }
}
type Inputs<'a> = (
    &'a TextureView,
    &'a TextureView,
    &'a TextureView,
    BindingResource<'a>,
);
fn frame_groups(
    (device, p, cache): (&RenderDevice, &Pipelines, &PipelineCache),
    textures: &Textures,
    inputs: Inputs<'_>,
    samples: u32,
    known: &mut super::world::Cache,
) -> FrameGroups {
    let (frame, vectors, depth, binding) = inputs;
    let layout = cache.get_bind_group_layout(&p.composite_layouts[usize::from(samples > 1)]);
    let composite = composite_group(
        device,
        p,
        &layout,
        textures,
        (frame, vectors, depth, binding.clone()),
        false,
    );
    let world = if textures.world {
        let native = composite_group(
            device,
            p,
            &layout,
            textures,
            (frame, vectors, depth, binding),
            true,
        );
        super::world::prepare(p, cache, device, textures, samples, known, (native, frame))
    } else {
        None
    };
    FrameGroups {
        source: frame.id(),
        composite,
        world,
    }
}
fn composite_group(
    device: &RenderDevice,
    p: &Pipelines,
    layout: &BindGroupLayout,
    textures: &Textures,
    (frame, vectors, depth, uniform): Inputs<'_>,
    native: bool,
) -> BindGroup {
    let filtered = if native {
        &textures.output.as_ref().expect("world output").default_view
    } else {
        frame
    };
    device.create_bind_group(
        Some("motion_composite"),
        layout,
        &BindGroupEntries::sequential((
            frame,
            &p.sampler,
            uniform,
            &textures.background.default_view,
            vectors,
            depth,
            &textures.reduced[2].default_view,
            filtered,
        )),
    )
}
fn texture_key(
    textures: &Textures,
    vectors: &TextureView,
    depth: &TextureView,
    sources: [TextureViewId; 2],
    samples: u32,
) -> Key {
    Key {
        sources,
        textures: [
            textures.background.default_view.id(),
            vectors.id(),
            depth.id(),
            textures.reduced[0].default_view.id(),
            textures.reduced[1].default_view.id(),
            textures.reduced[2].default_view.id(),
            textures
                .output
                .as_ref()
                .unwrap_or(&textures.background)
                .default_view
                .id(),
        ],
        samples,
        world: textures.world,
    }
}
fn view_data(
    blur: &WeaponBlur,
    textures: &Textures,
    view: &ExtractedView,
    previous: Option<&PreviousViewData>,
) -> Data {
    let mut reprojection = Mat4::IDENTITY;
    if blur.world {
        let current = view
            .clip_from_world
            .unwrap_or_else(|| view.clip_from_view * view.world_from_view.to_matrix().inverse());
        let prior = previous.map_or(current, |previous| previous.clip_from_world);
        if blur.max > 0. {
            reprojection = prior * current.inverse();
            if let Some(previous) = previous {
                // An infinite perspective inverse maps clip Z to the camera origin.
                // Remove cancellation noise from camera-only rotations at large map coordinates.
                let origin = previous.world_from_clip.z_axis;
                if origin.w.abs() > 1e-6
                    && (origin.truncate() / origin.w)
                        .distance_squared(view.world_from_view.translation())
                        < 0.0001
                {
                    reprojection.z_axis.x = 0.;
                    reprojection.z_axis.y = 0.;
                    reprojection.z_axis.w = 0.;
                }
            }
        }
    }
    Data {
        reprojection,
        flags: Vec4::new(f32::from(blur.camera_moving), 0., 0., 0.),
        params: Vec4::new(
            blur.max,
            1.25,
            textures.size.x as f32,
            textures.size.y as f32,
        ),
    }
}

fn pipeline_set(
    p: &Pipelines,
    cache: &PipelineCache,
    format: TextureFormat,
    samples: u32,
    world: bool,
) -> [CachedRenderPipelineId; 4] {
    [
        pipeline(
            p,
            cache,
            "reduce_raw",
            TextureFormat::Rgba16Float,
            samples,
            world,
        ),
        pipeline(
            p,
            cache,
            "reduce_tiles",
            TextureFormat::Rgba16Float,
            1,
            world,
        ),
        pipeline(p, cache, "dilate", TextureFormat::Rgba16Float, 1, world),
        pipeline(p, cache, "composite", format, samples, world),
    ]
}

#[cfg(test)]
mod tests {
    use super::same_pair;
    #[test]
    fn source_ping_pong_reuses_groups_but_resize_invalidates_them() {
        assert!(same_pair(&[11, 22], &[11, 22]));
        assert!(same_pair(&[11, 22], &[22, 11]));
        assert!(!same_pair(&[11, 22], &[11, 33]));
        assert!(!same_pair(&[11, 22], &[33, 22]));
    }
}
