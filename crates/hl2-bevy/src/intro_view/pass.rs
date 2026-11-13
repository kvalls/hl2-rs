//! The intro pass: after the world camera's tonemapping (the engine composes the intro from
//! the finished frame buffers, before the viewmodel and the screen effects), the view's main
//! texture and the camera entity's image are blended by `shaders/intro_composite.wgsl` into
//! the other main texture (and the multisampled attachment, which the viewmodel camera
//! continues from). No pass, buffer or bind group exists while the component is absent.
use super::IntroComposite;
use crate::bloom::fullscreen_pass::{self, Fullscreen, group_of};
use bevy::render::extract_component::ExtractComponentPlugin;
use bevy::{
    core_pipeline::{
        FullscreenShader,
        schedule::{Core3d, Core3dSystems},
        tonemapping::tonemapping,
    },
    ecs::{
        entity::Entity,
        query::With,
        schedule::IntoScheduleConfigs,
        system::{Commands, Local, Query, Res},
    },
    prelude::{App, AssetServer, Handle, Msaa, Plugin, Shader},
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        render_asset::RenderAssets,
        render_resource::{
            BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
            CachedRenderPipelineId, PipelineCache, Sampler, SamplerBindingType, ShaderStages,
            ShaderType, TextureFormat, TextureSampleType, TextureViewId, UniformBuffer,
            binding_types::{sampler, texture_2d, uniform_buffer},
        },
        renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
        texture::{FallbackImage, GpuImage},
        view::{ExtractedView, ViewTarget},
    },
};
use std::collections::HashMap;

pub(super) struct IntroPassPlugin;
impl Plugin for IntroPassPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<IntroComposite>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init)
            .add_systems(
                Render,
                (
                    prepare_pipelines.in_set(RenderSystems::Prepare),
                    prepare_groups.in_set(RenderSystems::PrepareBindGroups),
                ),
            )
            .add_systems(
                Core3d,
                intro_composite
                    .in_set(Core3dSystems::PostProcess)
                    .after(tonemapping),
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

/// passes: (mode, alpha) x 2; flags.x: 1 when the camera entity's view exists.
#[derive(ShaderType, Clone, Copy, Default)]
struct IntroData {
    passes: bevy::math::Vec4,
    flags: bevy::math::Vec4,
}

fn init(
    mut commands: Commands,
    device: Res<RenderDevice>,
    fullscreen: Res<FullscreenShader>,
    assets: Res<AssetServer>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "intro_composite_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<IntroData>(false),
            ),
        ),
    );
    let sampler = fullscreen_pass::linear_sampler(&device);
    commands.insert_resource(Pipelines {
        layout,
        sampler,
        fullscreen: fullscreen.clone(),
        shader: assets.load("shaders/intro_composite.wgsl"),
    });
}

#[derive(bevy::ecs::component::Component)]
struct IntroPipeline(CachedRenderPipelineId);

fn prepare_pipelines(
    mut commands: Commands,
    pipelines: Res<Pipelines>,
    cache: Res<PipelineCache>,
    mut known: Local<HashMap<(TextureFormat, u32), CachedRenderPipelineId>>,
    views: Query<(Entity, &ExtractedView, &Msaa), With<IntroComposite>>,
) {
    for (entity, view, msaa) in &views {
        let key = (view.target_format, msaa.samples());
        let id = *known.entry(key).or_insert_with(|| {
            let pass = Fullscreen {
                label: "intro_composite",
                entry: "intro_composite",
                layout: &pipelines.layout,
                vertex: &pipelines.fullscreen,
                shader: &pipelines.shader,
            };
            fullscreen_pass::queue_pipeline(&cache, &pass, key)
        });
        commands.entity(entity).insert(IntroPipeline(id));
    }
}

/// This frame's bind groups, one per main texture the pass may read.
#[derive(bevy::ecs::component::Component)]
struct IntroGroup([(TextureViewId, BindGroup); 2]);

fn prepare_groups(
    mut commands: Commands,
    (pipelines, cache): (Res<Pipelines>, Res<PipelineCache>),
    (device, queue): (Res<RenderDevice>, Res<RenderQueue>),
    (images, fallback): (Res<RenderAssets<GpuImage>>, Res<FallbackImage>),
    mut buffers: Local<HashMap<Entity, UniformBuffer<IntroData>>>,
    views: Query<(Entity, &IntroComposite, &ViewTarget)>,
) {
    buffers.retain(|entity, _| views.contains(*entity));
    for (entity, intro, target) in &views {
        let scene = intro.scene.as_ref().and_then(|h| images.get(h));
        let buffer = buffers.entry(entity).or_default();
        buffer.set(IntroData {
            passes: intro.passes,
            flags: bevy::math::Vec4::new(f32::from(u8::from(scene.is_some())), 0., 0., 0.),
        });
        buffer.write_buffer(&device, &queue);
        let Some(binding) = buffer.binding() else {
            continue;
        };
        let scene_view = scene.map_or(&fallback.d2.texture_view, |s| &s.texture_view);
        let layout = cache.get_bind_group_layout(&pipelines.layout);
        let sources = [target.main_texture_view(), target.main_texture_other_view()];
        let groups = sources.map(|view| {
            let entries = (view, scene_view, &pipelines.sampler, binding.clone());
            let group = device.create_bind_group(
                Some("intro_composite_bind_group"),
                &layout,
                &BindGroupEntries::sequential(entries),
            );
            (view.id(), group)
        });
        commands.entity(entity).insert(IntroGroup(groups));
    }
}

fn intro_composite(
    view: ViewQuery<(&ViewTarget, &IntroPipeline, &IntroGroup)>,
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
    fullscreen_pass::draw(&mut ctx, "intro_composite", target, (pipeline, group));
}
