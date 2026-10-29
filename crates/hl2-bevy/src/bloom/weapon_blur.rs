//! Shared Advanced motion reconstruction for world and weapon cameras.
use bevy::{
    core_pipeline::{
        FullscreenShader,
        core_3d::main_opaque_pass_3d,
        schedule::{Core3d, Core3dSystems},
    },
    ecs::{
        component::Component,
        resource::Resource,
        schedule::IntoScheduleConfigs,
        system::{Commands, Res},
    },
    prelude::{App, AssetServer, Handle, Plugin, Shader},
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        diagnostic::RecordDiagnostics,
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        render_resource::{
            BindGroup, BindGroupLayoutDescriptor, BindGroupLayoutEntries, CachedRenderPipelineId,
            ColorTargetState, ColorWrites, FilterMode, FragmentState, MultisampleState,
            PipelineCache, RenderPassColorAttachment, RenderPassDescriptor,
            RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages,
            ShaderType, TextureFormat, TextureSampleType, TextureView, TextureViewId,
            UniformBuffer,
            binding_types::{sampler, texture_2d, texture_2d_multisampled, uniform_buffer},
        },
        renderer::{RenderContext, RenderDevice, ViewQuery},
        texture::CachedTexture,
        view::ViewTarget,
    },
};

pub(super) mod depth;
mod prepare;
mod region;
mod world;

#[derive(Component, Clone, Copy, PartialEq, ExtractComponent)]
pub(super) struct WeaponBlur {
    pub max: f32,
    pub world: bool,
    pub camera_moving: bool,
}

pub(super) struct WeaponBlurPlugin;
impl Plugin for WeaponBlurPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<WeaponBlur>::default());
        region::install(app);
        let Some(render) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render
            .add_systems(RenderStartup, init)
            .add_systems(
                Render,
                (
                    prepare::textures.in_set(RenderSystems::PrepareResources),
                    prepare::groups.in_set(RenderSystems::PrepareBindGroups),
                ),
            )
            .add_systems(
                Core3d,
                snapshot
                    .in_set(Core3dSystems::MainPass)
                    .before(main_opaque_pass_3d),
            )
            .add_systems(
                Core3d,
                reconstruct
                    .in_set(Core3dSystems::PostProcess)
                    .before(super::source_bloom),
            );
    }
}

#[derive(Resource)]
struct Pipelines {
    raw_layouts: [BindGroupLayoutDescriptor; 2],
    reduce_layout: BindGroupLayoutDescriptor,
    composite_layouts: [BindGroupLayoutDescriptor; 2],
    image_layout: BindGroupLayoutDescriptor,
    fullscreen: FullscreenShader,
    shader: Handle<Shader>,
    sampler: Sampler,
}

#[derive(Clone, Copy, Default, ShaderType)]
struct Data {
    params: bevy::math::Vec4,
    reprojection: bevy::math::Mat4,
    flags: bevy::math::Vec4,
}

fn motion_binding(
    msaa: bool,
    kind: TextureSampleType,
) -> bevy::render::render_resource::BindGroupLayoutEntryBuilder {
    if msaa {
        texture_2d_multisampled(kind)
    } else {
        texture_2d(kind)
    }
}

fn init(
    mut commands: Commands,
    device: Res<RenderDevice>,
    fullscreen: Res<FullscreenShader>,
    assets: Res<AssetServer>,
) {
    let float = TextureSampleType::Float { filterable: false };
    let filtered = TextureSampleType::Float { filterable: true };
    let raw_layouts = [false, true].map(|msaa| {
        BindGroupLayoutDescriptor::new(
            "weapon_motion_reduce_raw",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    motion_binding(msaa, float),
                    motion_binding(msaa, TextureSampleType::Depth),
                    uniform_buffer::<Data>(false),
                ),
            ),
        )
    });
    let reduce_layout = BindGroupLayoutDescriptor::new(
        "weapon_motion_reduce",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (texture_2d(float), uniform_buffer::<Data>(false)),
        ),
    );
    let composite_layouts = [false, true].map(|msaa| {
        BindGroupLayoutDescriptor::new(
            "weapon_motion_composite",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(filtered),
                    sampler(SamplerBindingType::Filtering),
                    uniform_buffer::<Data>(false),
                    texture_2d(filtered),
                    motion_binding(msaa, float),
                    motion_binding(msaa, TextureSampleType::Depth),
                    texture_2d(float),
                    texture_2d(filtered),
                ),
            ),
        )
    });
    let image_layout = BindGroupLayoutDescriptor::new(
        "world_blur_image",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(filtered),
                sampler(SamplerBindingType::Filtering),
                texture_2d(filtered),
            ),
        ),
    );
    commands.insert_resource(Pipelines {
        image_layout,
        raw_layouts,
        reduce_layout,
        composite_layouts,
        fullscreen: fullscreen.clone(),
        shader: assets.load("shaders/weapon_blur.wgsl"),
        sampler: device.create_sampler(&SamplerDescriptor {
            min_filter: FilterMode::Linear,
            mag_filter: FilterMode::Linear,
            ..Default::default()
        }),
    });
}

fn pipeline(
    p: &Pipelines,
    cache: &PipelineCache,
    entry: &'static str,
    format: TextureFormat,
    samples: u32,
    world: bool,
) -> CachedRenderPipelineId {
    let msaa = usize::from(samples > 1);
    let layout = match entry {
        "world_downsample" => &p.image_layout,
        "world_objects" => &p.composite_layouts[msaa],
        "reduce_raw" => &p.raw_layouts[msaa],
        "composite" => &p.composite_layouts[msaa],
        _ => &p.reduce_layout,
    };
    cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some(format!("weapon_blur_{entry}").into()),
        layout: vec![layout.clone()],
        vertex: p.fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: p.shader.clone(),
            shader_defs: {
                let mut defs = Vec::new();
                if entry == "world_downsample" {
                    defs.push("WORLD_IMAGE".into());
                } else if world {
                    defs.push("WORLD_RECONSTRUCTION".into());
                }
                if entry == "world_objects" {
                    defs.push("WORLD_OBJECTS".into());
                }
                if msaa == 1 {
                    defs.push("MULTISAMPLED".into());
                }
                if entry == "reduce_raw" {
                    defs.push("REDUCE_RAW".into());
                } else if !matches!(entry, "composite" | "world_objects") {
                    defs.push("REDUCE_FIELD".into());
                }
                defs
            },
            entry_point: Some(
                if entry == "world_objects" {
                    "composite"
                } else {
                    entry
                }
                .into(),
            ),
            targets: vec![Some(ColorTargetState {
                format,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
        }),
        multisample: MultisampleState::default(),
        ..Default::default()
    })
}

#[derive(Component)]
struct Textures {
    background: CachedTexture,
    size: bevy::math::UVec2,
    format: TextureFormat,
    reduced: [CachedTexture; 3],
    world: bool,
    output: Option<CachedTexture>,
}

#[derive(Component)]
struct Prepared {
    pipelines: [CachedRenderPipelineId; 4],
    groups: [BindGroup; 3],
    frames: [FrameGroups; 2],
    _buffer: UniformBuffer<Data>,
}

struct FrameGroups {
    source: TextureViewId,
    composite: BindGroup,
    world: Option<world::Prepared>,
}

fn snapshot(view: ViewQuery<(&ViewTarget, &Textures, &WeaponBlur)>, mut ctx: RenderContext) {
    let (target, textures, blur) = view.into_inner();
    if blur.max == 0. || blur.world {
        return;
    }
    ctx.command_encoder().copy_texture_to_texture(
        target.main_texture().as_image_copy(),
        textures.background.texture.as_image_copy(),
        textures.background.texture.size(),
    );
}

fn draw(
    ctx: &mut RenderContext,
    cache: &PipelineCache,
    pipeline: CachedRenderPipelineId,
    group: &BindGroup,
    target: &TextureView,
    label: &'static str,
) {
    draw_region(ctx, cache, pipeline, group, target, label, None);
}

fn draw_region(
    ctx: &mut RenderContext,
    cache: &PipelineCache,
    pipeline: CachedRenderPipelineId,
    group: &BindGroup,
    target: &TextureView,
    label: &'static str,
    scissor: Option<[u32; 4]>,
) {
    let Some(pipeline) = cache.get_render_pipeline(pipeline) else {
        return;
    };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: bevy::render::render_resource::Operations {
                load: bevy::render::render_resource::LoadOp::Clear(Default::default()),
                store: bevy::render::render_resource::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    let span = diagnostics.pass_span(&mut pass, label);
    if scissor.is_none_or(|r| r[2] > 0 && r[3] > 0) {
        if let Some([x, y, w, h]) = scissor {
            pass.set_scissor_rect(x, y, w, h);
        }
        pass.set_render_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }
    span.end(&mut pass);
}

fn reconstruct(
    view: ViewQuery<(
        &ViewTarget,
        &Textures,
        &Prepared,
        &WeaponBlur,
        Option<&region::MotionRegion>,
    )>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
    performance: Option<Res<crate::performance::Performance>>,
) {
    let (target, textures, prepared, blur, region) = view.into_inner();
    let Some(frame) = prepared
        .frames
        .iter()
        .find(|f| f.source == target.main_texture_view().id())
    else {
        return;
    };
    if blur.max == 0.
        || frame.world.as_ref().is_some_and(|w| {
            w.pipelines
                .iter()
                .any(|id| cache.get_render_pipeline(*id).is_none())
        })
        || prepared
            .pipelines
            .iter()
            .any(|id| cache.get_render_pipeline(*id).is_none())
    {
        return;
    }
    if let Some(world) = &frame.world
        && blur.camera_moving
    {
        draw(
            &mut ctx,
            &cache,
            world.pipelines[0],
            &world.downsample,
            &textures.background.default_view,
            "world_blur_downsample",
        );
    }
    for i in 0..3 {
        let scissor = (i == 0 && !blur.world)
            .then(|| region.map(|r| r.scissor(textures.size)))
            .flatten();
        if i == 0
            && let Some(performance) = performance.as_deref()
        {
            let size = textures.reduced[0].texture.size();
            let r = scissor.unwrap_or([0, 0, size.width, size.height]);
            performance.record_reduction(
                blur.world,
                u64::from(r[2]) * u64::from(r[3]),
                u64::from(size.width) * u64::from(size.height),
            );
        }
        draw_region(
            &mut ctx,
            &cache,
            prepared.pipelines[i],
            &prepared.groups[i],
            &textures.reduced[i].default_view,
            [
                "weapon_reduce_4",
                "weapon_reduce_32",
                "weapon_motion_dilate",
            ][i],
            scissor,
        );
    }
    if let Some(world) = &frame.world {
        world::reconstruct(
            &mut ctx,
            &cache,
            target,
            textures,
            prepared,
            world,
            &frame.composite,
        );
        return;
    }
    let write = target.post_process_write();
    draw(
        &mut ctx,
        &cache,
        prepared.pipelines[3],
        &frame.composite,
        write.destination,
        "viewmodel_animation_blur",
    );
}
