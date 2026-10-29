//! Image-space motion blur, render half: after the world camera's main pass the frame is copied
//! to a scratch texture and drawn back blurred (into the multisampled attachment too, so the
//! viewmodel camera that follows draws its weapon sharp over the blurred world, as the engine
//! blurs before the viewmodel). No pass, texture or buffer exists while the component is absent.
use super::motion_blur::SourceMotionBlur;
use bevy::render::extract_component::ExtractComponentPlugin;
use bevy::{
    core_pipeline::{
        FullscreenShader,
        schedule::{Core3d, Core3dSystems},
        tonemapping::tonemapping,
    },
    ecs::{
        entity::Entity,
        query::{Or, With, Without},
        schedule::IntoScheduleConfigs,
        system::{Commands, Local, Query, Res, ResMut},
    },
    prelude::{App, AssetServer, Handle, Msaa, Plugin, Shader},
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        camera::ExtractedCamera,
        render_resource::{
            BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
            CachedRenderPipelineId, ColorTargetState, ColorWrites, Extent3d, FilterMode,
            FragmentState, MultisampleState, PipelineCache, RenderPassDescriptor,
            RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages,
            ShaderType, TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType,
            TextureUsages, TextureViewId, UniformBuffer,
            binding_types::{sampler, texture_2d, uniform_buffer},
        },
        renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
        texture::{CachedTexture, TextureCache},
        view::{ExtractedView, ViewTarget},
    },
};
use std::collections::HashMap;

pub(super) struct MotionBlurPlugin;
impl Plugin for MotionBlurPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<SourceMotionBlur>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init)
            .add_systems(
                Render,
                (
                    prepare_pipelines.in_set(RenderSystems::Prepare),
                    prepare_scratch.in_set(RenderSystems::PrepareResources),
                    prepare_groups.in_set(RenderSystems::PrepareBindGroups),
                ),
            )
            // After the world camera's main pass (the viewmodel camera follows it).
            .add_systems(
                Core3d,
                source_motion_blur
                    .in_set(Core3dSystems::PostProcess)
                    .before(tonemapping),
            );
    }
}

#[derive(bevy::ecs::resource::Resource)]
struct Pipelines {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    fullscreen: FullscreenShader,
    shader: Handle<Shader>,
}

#[derive(ShaderType, Clone, Copy, Default)]
struct BlurData {
    values: bevy::math::Vec4,
    params: bevy::math::Vec4,
}

fn init(
    mut commands: Commands,
    device: Res<RenderDevice>,
    fullscreen: Res<FullscreenShader>,
    assets: Res<AssetServer>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "source_motion_blur_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<BlurData>(false),
            ),
        ),
    );
    let sampler = device.create_sampler(&SamplerDescriptor {
        min_filter: FilterMode::Linear,
        mag_filter: FilterMode::Linear,
        ..Default::default()
    });
    commands.insert_resource(Pipelines {
        layout,
        sampler,
        fullscreen: fullscreen.clone(),
        shader: assets.load("shaders/motion_blur.wgsl"),
    });
}

/// The pipeline of a view: its target format and sample count.
#[derive(bevy::ecs::component::Component)]
struct BlurPipeline(CachedRenderPipelineId);

fn prepare_pipelines(
    mut commands: Commands,
    pipelines: Res<Pipelines>,
    cache: Res<PipelineCache>,
    mut known: Local<HashMap<(TextureFormat, u32), CachedRenderPipelineId>>,
    views: Query<(Entity, &ExtractedView, &Msaa), With<SourceMotionBlur>>,
) {
    for (entity, view, msaa) in &views {
        let key = (view.target_format, msaa.samples());
        let id = *known.entry(key).or_insert_with(|| {
            cache.queue_render_pipeline(RenderPipelineDescriptor {
                label: Some("source_motion_blur".into()),
                layout: vec![pipelines.layout.clone()],
                vertex: pipelines.fullscreen.to_vertex_state(),
                fragment: Some(FragmentState {
                    shader: pipelines.shader.clone(),
                    shader_defs: Vec::new(),
                    entry_point: Some("motion_blur".into()),
                    targets: vec![Some(ColorTargetState {
                        format: key.0,
                        blend: None,
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                multisample: MultisampleState {
                    count: key.1,
                    ..Default::default()
                },
                ..Default::default()
            })
        });
        commands.entity(entity).insert(BlurPipeline(id));
    }
}

/// Views that still carry render state of a blur whose component is gone.
type Stale = (
    Or<(With<Scratch>, With<BlurPipeline>, With<BlurGroup>)>,
    Without<SourceMotionBlur>,
);

/// The frame copy the blur reads (it cannot read the texture it resolves into).
#[derive(bevy::ecs::component::Component)]
struct Scratch(CachedTexture);

fn prepare_scratch(
    mut commands: Commands,
    mut cache: ResMut<TextureCache>,
    device: Res<RenderDevice>,
    views: Query<(Entity, &ExtractedCamera, &ExtractedView), With<SourceMotionBlur>>,
    stale: Query<Entity, Stale>,
) {
    // A view without the component keeps no scratch texture, pipeline key or bind group.
    for entity in &stale {
        commands
            .entity(entity)
            .remove::<(Scratch, BlurPipeline, BlurGroup)>();
    }
    for (entity, camera, view) in &views {
        let Some(size) = camera.physical_viewport_size else {
            continue;
        };
        let texture = cache.get(
            &device,
            TextureDescriptor {
                label: Some("source_motion_blur_frame"),
                size: Extent3d {
                    width: size.x,
                    height: size.y,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: view.target_format,
                usage: TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
        );
        commands.entity(entity).insert(Scratch(texture));
    }
}

/// The bind group of this frame's blur: the scratch texture, the sampler and the uniform.
#[derive(bevy::ecs::component::Component)]
struct BlurGroup(BindGroup);

struct Prepared {
    buffer: UniformBuffer<BlurData>,
    group: Option<(TextureViewId, BindGroup)>,
}

/// Writes the uniform of a view that blurs this frame and binds it to its scratch texture; the
/// buffer and the group live as long as the view carries the component and are rebuilt only when
/// the scratch texture changes.
fn prepare_groups(
    mut commands: Commands,
    (pipelines, cache): (Res<Pipelines>, Res<PipelineCache>),
    (device, queue): (Res<RenderDevice>, Res<RenderQueue>),
    mut prepared: Local<HashMap<Entity, Prepared>>,
    views: Query<(Entity, &SourceMotionBlur, &Scratch)>,
) {
    prepared.retain(|entity, _| views.contains(*entity));
    for (entity, blur, scratch) in &views {
        let samples = hl2_simulation::motion_blur::samples(
            scratch.0.texture.height(),
            blur.values.to_array(),
        );
        if samples == 0 {
            commands.entity(entity).remove::<BlurGroup>();
            continue;
        }
        let state = prepared.entry(entity).or_insert_with(|| Prepared {
            buffer: UniformBuffer::default(),
            group: None,
        });
        state.buffer.set(BlurData {
            values: blur.values,
            params: bevy::math::Vec4::new(blur.max, samples as f32, 0., 0.),
        });
        state.buffer.write_buffer(&device, &queue);
        let id = scratch.0.default_view.id();
        if state.group.as_ref().is_none_or(|(known, _)| *known != id) {
            let Some(buffer) = state.buffer.binding() else {
                continue;
            };
            let layout = cache.get_bind_group_layout(&pipelines.layout);
            let group = device.create_bind_group(
                Some("source_motion_blur_bind_group"),
                &layout,
                &BindGroupEntries::sequential((
                    &scratch.0.default_view,
                    &pipelines.sampler,
                    buffer,
                )),
            );
            state.group = Some((id, group));
        }
        if let Some((_, group)) = &state.group {
            commands.entity(entity).insert(BlurGroup(group.clone()));
        }
    }
}

fn source_motion_blur(
    view: ViewQuery<(&ViewTarget, &BlurPipeline, &Scratch, &BlurGroup)>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (target, pipeline, scratch, group) = view.into_inner();
    let Some(pipeline) = cache.get_render_pipeline(pipeline.0) else {
        return;
    };
    ctx.command_encoder().copy_texture_to_texture(
        target.main_texture().as_image_copy(),
        scratch.0.texture.as_image_copy(),
        scratch.0.texture.size(),
    );
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("source_motion_blur"),
        color_attachments: &[Some(target.get_color_attachment())],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_render_pipeline(pipeline);
    pass.set_bind_group(0, &group.0, &[]);
    pass.draw(0..3, 0..1);
}
