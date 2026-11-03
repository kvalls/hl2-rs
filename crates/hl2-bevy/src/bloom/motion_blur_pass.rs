//! Image-space motion blur, render half: after the world camera's main pass the view's two main
//! textures swap and the previous one is drawn back blurred (into the multisampled attachment too,
//! so the viewmodel camera that follows draws its weapon sharp over the blurred world, as the
//! engine blurs before the viewmodel). No pass or buffer exists while the component is absent.
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
        system::{Commands, Local, Query, Res},
    },
    prelude::{App, AssetServer, Handle, Msaa, Plugin, Shader},
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        render_resource::{
            BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
            CachedRenderPipelineId, ColorTargetState, ColorWrites, FilterMode, FragmentState,
            LoadOp, MultisampleState, Operations, PipelineCache, RenderPassColorAttachment,
            RenderPassDescriptor, RenderPipelineDescriptor, Sampler, SamplerBindingType,
            SamplerDescriptor, ShaderStages, ShaderType, StoreOp, TextureFormat, TextureSampleType,
            TextureView, TextureViewId, UniformBuffer,
            binding_types::{sampler, texture_2d, uniform_buffer},
        },
        renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
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
                    (remove_stale, prepare_pipelines).in_set(RenderSystems::Prepare),
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
    Or<(With<BlurPipeline>, With<BlurGroup>)>,
    Without<SourceMotionBlur>,
);

fn remove_stale(mut commands: Commands, stale: Query<Entity, Stale>) {
    // A view without the component keeps no pipeline key or bind group.
    for entity in &stale {
        commands
            .entity(entity)
            .remove::<(BlurPipeline, BlurGroup)>();
    }
}

/// The bind groups of this frame's blur, one per main texture it may read.
#[derive(bevy::ecs::component::Component)]
struct BlurGroup([(TextureViewId, BindGroup); 2]);

struct Prepared {
    buffer: UniformBuffer<BlurData>,
    groups: Option<[(TextureViewId, BindGroup); 2]>,
}

/// Writes the uniform of a view that blurs this frame and binds it to both main textures; the
/// buffer and the groups live as long as the view carries the component and are rebuilt only when
/// the main textures change.
fn prepare_groups(
    mut commands: Commands,
    (pipelines, cache): (Res<Pipelines>, Res<PipelineCache>),
    (device, queue): (Res<RenderDevice>, Res<RenderQueue>),
    mut prepared: Local<HashMap<Entity, Prepared>>,
    views: Query<(Entity, &SourceMotionBlur, &ViewTarget)>,
) {
    prepared.retain(|entity, _| views.contains(*entity));
    for (entity, blur, target) in &views {
        let samples = hl2_simulation::motion_blur::samples(
            target.main_texture().height(),
            blur.values.to_array(),
        );
        if samples == 0 {
            commands.entity(entity).remove::<BlurGroup>();
            continue;
        }
        let state = prepared.entry(entity).or_insert_with(|| Prepared {
            buffer: UniformBuffer::default(),
            groups: None,
        });
        state.buffer.set(BlurData {
            values: blur.values,
            params: bevy::math::Vec4::new(blur.max, samples as f32, 0., 0.),
        });
        state.buffer.write_buffer(&device, &queue);
        let sources = [target.main_texture_view(), target.main_texture_other_view()];
        let known = state
            .groups
            .as_ref()
            .is_some_and(|groups| sources.iter().all(|view| group_of(groups, view).is_some()));
        if !known {
            let Some(buffer) = state.buffer.binding() else {
                continue;
            };
            let layout = cache.get_bind_group_layout(&pipelines.layout);
            state.groups = Some(sources.map(|view| {
                let group = device.create_bind_group(
                    Some("source_motion_blur_bind_group"),
                    &layout,
                    &BindGroupEntries::sequential((view, &pipelines.sampler, buffer.clone())),
                );
                (view.id(), group)
            }));
        }
        if let Some(groups) = &state.groups {
            commands.entity(entity).insert(BlurGroup(groups.clone()));
        }
    }
}

fn group_of<'a>(
    groups: &'a [(TextureViewId, BindGroup); 2],
    view: &TextureView,
) -> Option<&'a BindGroup> {
    groups
        .iter()
        .find_map(|(id, group)| (*id == view.id()).then_some(group))
}

fn source_motion_blur(
    view: ViewQuery<(&ViewTarget, &BlurPipeline, &BlurGroup)>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (target, pipeline, groups) = view.into_inner();
    let Some(pipeline) = cache.get_render_pipeline(pipeline.0) else {
        return;
    };
    let Some(group) = group_of(&groups.0, target.main_texture_view()) else {
        return;
    };
    // The frame stays readable in the texture it was resolved to; the attachment now resolves
    // into the other one, so no copy of the frame is needed.
    let destination = target.post_process_write().destination;
    // The triangle overwrites every sample, so loading the old image would only cost bandwidth.
    // The multisampled image must be stored: the viewmodel camera continues from it.
    let (view, resolve_target) = match target.sampled_main_texture_view() {
        Some(sampled) => (sampled, Some(destination)),
        None => (destination, None),
    };
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("source_motion_blur"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: resolve_target.map(|view| &**view),
            ops: Operations {
                load: LoadOp::Clear(Default::default()),
                store: StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_render_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.draw(0..3, 0..1);
}
