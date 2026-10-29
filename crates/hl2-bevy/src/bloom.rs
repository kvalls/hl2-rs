//! Source HDR bloom (SDK viewpostprocess.cpp Generate8BitBloomTexture + Engine_Post): a
//! gamma-space shaped quarter-size downsample, a 13-tap Gaussian blur in X and Y (Y scaled
//! by the bloom amount) and an additive composite before the HUD.
use bevy::{
    core_pipeline::{
        FullscreenShader,
        schedule::{Core3d, Core3dSystems},
        tonemapping::tonemapping,
    },
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        camera::ExtractedCamera,
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        render_resource::{
            binding_types::{sampler, texture_2d, uniform_buffer},
            *,
        },
        renderer::{RenderContext, RenderDevice, ViewQuery},
        texture::{CachedTexture, TextureCache},
        view::{ExtractedView, ViewTarget},
    },
};

pub mod color_correction;
mod correction_pass;
pub mod motion_blur;
mod motion_blur_pass;
mod viewmodel_blur;
mod weapon_blur;
mod world_blur;

/// Bloom amount on the last 3D camera of the main view, and the image that receives the
/// pre-bloom frame for auto exposure.
#[derive(Component, Clone, ExtractComponent)]
pub struct SourceBloom {
    pub amount: f32,
    pub presample: Option<Handle<Image>>,
}

pub struct SourceBloomPlugin;
impl Plugin for SourceBloomPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<SourceBloom>::default());
        app.add_plugins(ExtractComponentPlugin::<
            color_correction::SourceColorCorrection,
        >::default())
            .init_resource::<color_correction::Lookups>()
            .add_systems(Update, color_correction::update);
        app.add_plugins((
            motion_blur_pass::MotionBlurPlugin,
            weapon_blur::WeaponBlurPlugin,
        ))
        .init_resource::<motion_blur::MotionBlurState>()
        .add_systems(Update, motion_blur::update)
        .add_systems(
            PostUpdate,
            (
                (viewmodel_blur::update, world_blur::update),
                weapon_blur::depth::update,
            )
                .chain()
                .after(crate::camera::present_player_view)
                .after(crate::rendering::present_weapons)
                .after(crate::rendering::present_entities)
                .before(TransformSystems::Propagate),
        );
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(
                Render,
                (
                    prepare_pipelines.in_set(RenderSystems::Prepare),
                    prepare_textures.in_set(RenderSystems::PrepareResources),
                    correction_pass::prepare.in_set(RenderSystems::PrepareBindGroups),
                ),
            )
            // After the main pass (PostProcess set), before tonemapping: ordered only
            // `.before(tonemapping)`, the executor could run it before the viewmodel camera's
            // main pass, and the post-process swap then discarded the weapon on some frames.
            .add_systems(
                Core3d,
                source_bloom
                    .in_set(Core3dSystems::PostProcess)
                    .before(tonemapping),
            );
    }
}

const SMALL_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

#[derive(Resource)]
struct Pipelines {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    /// Group 1 of the composite that applies color correction.
    correction_layout: BindGroupLayoutDescriptor,
    shape: CachedRenderPipelineId,
    blur: CachedRenderPipelineId,
    presample: CachedRenderPipelineId,
    fullscreen: FullscreenShader,
    shader: Handle<Shader>,
}

#[derive(ShaderType, Clone, Copy)]
struct PassData {
    texel_direction: Vec4,
    params: Vec4,
}

fn descriptor(
    pipelines_layout: &BindGroupLayoutDescriptor,
    fullscreen: &FullscreenShader,
    shader: &Handle<Shader>,
    entry: &'static str,
    format: TextureFormat,
) -> RenderPipelineDescriptor {
    RenderPipelineDescriptor {
        label: Some(format!("source_bloom_{entry}").into()),
        layout: vec![pipelines_layout.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: shader.clone(),
            // The color correction code of the shader reads it; the plain pipelines compile it out.
            shader_defs: vec![bevy::shader::ShaderDefVal::UInt("LOOKUPS".into(), 0)],
            entry_point: Some(entry.into()),
            targets: vec![Some(ColorTargetState {
                format,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
        }),
        ..default()
    }
}

fn init_pipelines(
    mut commands: Commands,
    device: Res<RenderDevice>,
    fullscreen: Res<FullscreenShader>,
    assets: Res<AssetServer>,
    cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "source_bloom_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<PassData>(false),
            ),
        ),
    );
    let sampler = device.create_sampler(&SamplerDescriptor {
        min_filter: FilterMode::Linear,
        mag_filter: FilterMode::Linear,
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        ..default()
    });
    let shader = assets.load("shaders/bloom.wgsl");
    let fullscreen = fullscreen.clone();
    let shape = cache.queue_render_pipeline(descriptor(
        &layout,
        &fullscreen,
        &shader,
        "shape",
        SMALL_FORMAT,
    ));
    let blur = cache.queue_render_pipeline(descriptor(
        &layout,
        &fullscreen,
        &shader,
        "blur",
        SMALL_FORMAT,
    ));
    let presample = cache.queue_render_pipeline(descriptor(
        &layout,
        &fullscreen,
        &shader,
        "presample",
        SMALL_FORMAT,
    ));
    commands.insert_resource(Pipelines {
        layout,
        correction_layout: correction_pass::layout(),
        sampler,
        shape,
        blur,
        presample,
        fullscreen,
        shader,
    });
}

/// The composite pipeline depends on the view's target format.
#[derive(Component)]
struct CompositePipeline(CachedRenderPipelineId);

fn prepare_pipelines(
    mut commands: Commands,
    pipelines: Res<Pipelines>,
    cache: Res<PipelineCache>,
    mut known: Local<std::collections::HashMap<TextureFormat, CachedRenderPipelineId>>,
    views: Query<(Entity, &ExtractedView), With<SourceBloom>>,
) {
    for (entity, view) in &views {
        let id = *known.entry(view.target_format).or_insert_with(|| {
            cache.queue_render_pipeline(descriptor(
                &pipelines.layout,
                &pipelines.fullscreen,
                &pipelines.shader,
                "composite",
                view.target_format,
            ))
        });
        commands.entity(entity).insert(CompositePipeline(id));
    }
}

#[derive(Component)]
struct SmallTextures([CachedTexture; 2]);

fn prepare_textures(
    mut commands: Commands,
    mut cache: ResMut<TextureCache>,
    device: Res<RenderDevice>,
    views: Query<(Entity, &ExtractedCamera), With<SourceBloom>>,
) {
    for (entity, camera) in &views {
        let Some(size) = camera.physical_viewport_size else {
            continue;
        };
        // _rt_SmallFB0/1 are exactly a quarter of the frame.
        let descriptor = |label| TextureDescriptor {
            label: Some(label),
            size: Extent3d {
                width: (size.x / 4).max(1),
                height: (size.y / 4).max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: SMALL_FORMAT,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };
        let textures = [
            cache.get(&device, descriptor("source_bloom_small0")),
            cache.get(&device, descriptor("source_bloom_small1")),
        ];
        commands.entity(entity).insert(SmallTextures(textures));
    }
}

/// One fullscreen draw of the bloom shader.
struct Pass<'a> {
    pipeline: &'a RenderPipeline,
    source: &'a TextureView,
    glow: &'a TextureView,
    data: PassData,
    destination: &'a TextureView,
    /// Group 1, for the composite that applies color correction.
    extra: Option<&'a BindGroup>,
}

impl Pass<'_> {
    fn draw(self, ctx: &mut RenderContext, pipelines: &Pipelines, layout: &BindGroupLayout) {
        let device = ctx.render_device().clone();
        let mut buffer = encase::UniformBuffer::new(Vec::new());
        buffer.write(&self.data).expect("bloom uniform");
        let data = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("source_bloom_data"),
            contents: buffer.as_ref(),
            usage: BufferUsages::UNIFORM,
        });
        let bind_group = device.create_bind_group(
            Some("source_bloom_bind_group"),
            layout,
            &BindGroupEntries::sequential((
                self.source,
                self.glow,
                &pipelines.sampler,
                data.as_entire_binding(),
            )),
        );
        let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("source_bloom"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: self.destination,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_render_pipeline(self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        if let Some(extra) = self.extra {
            pass.set_bind_group(1, extra, &[]);
        }
        pass.draw(0..3, 0..1);
    }
}

/// Pass data with the bloom factor of Engine_Post (1 while bloom is enabled) in `params.x`.
fn pass_data(texel_direction: Vec4, params: Vec4) -> PassData {
    PassData {
        texel_direction,
        params,
    }
}

fn source_bloom(
    view: ViewQuery<(
        &ViewTarget,
        &SourceBloom,
        &SmallTextures,
        &CompositePipeline,
        Option<&correction_pass::Correction>,
    )>,
    pipelines: Res<Pipelines>,
    cache: Res<PipelineCache>,
    images: Res<bevy::render::render_asset::RenderAssets<bevy::render::texture::GpuImage>>,
    mut ctx: RenderContext,
) {
    let (target, bloom, small, composite, correction) = view.into_inner();
    let layout = cache.get_bind_group_layout(&pipelines.layout);
    let one = Vec4::new(1., 0., 0., 0.);
    if let (Some(pipeline), Some(image)) = (
        cache.get_render_pipeline(pipelines.presample),
        bloom.presample.as_ref().and_then(|h| images.get(h)),
    ) {
        let frame = target.main_texture_view();
        Pass {
            pipeline,
            source: frame,
            glow: frame,
            data: pass_data(Vec4::ZERO, one),
            destination: &image.texture_view,
            extra: None,
        }
        .draw(&mut ctx, &pipelines, &layout);
    }
    let corrected = correction.and_then(|c| {
        cache
            .get_render_pipeline(c.pipeline)
            .map(|pipeline| (pipeline, &c.group))
    });
    if bloom.amount <= 0. {
        // No glow: the composite only applies the lookups, when there are any to apply.
        if let Some((pipeline, group)) = corrected {
            let post = target.post_process_write();
            Pass {
                pipeline,
                source: post.source,
                glow: post.source,
                data: pass_data(Vec4::ZERO, one),
                destination: post.destination,
                extra: Some(group),
            }
            .draw(&mut ctx, &pipelines, &layout);
        }
        return;
    }
    let (Some(shape), Some(blur), Some(composite)) = (
        cache.get_render_pipeline(pipelines.shape),
        cache.get_render_pipeline(pipelines.blur),
        cache.get_render_pipeline(composite.0),
    ) else {
        return;
    };
    let size = small.0[0].texture.size();
    let texel = Vec2::new(1. / size.width as f32, 1. / size.height as f32);
    let (a, b) = (&small.0[0].default_view, &small.0[1].default_view);
    let post = target.post_process_write();
    let pass = |pipeline, source, glow, data, destination, extra| Pass {
        pipeline,
        source,
        glow,
        data,
        destination,
        extra,
    };
    let (pipeline, extra) = corrected.map_or((composite, None), |(p, group)| (p, Some(group)));
    let passes = [
        pass(
            shape,
            post.source,
            post.source,
            pass_data(Vec4::ZERO, one),
            a,
            None,
        ),
        pass(
            blur,
            a,
            a,
            pass_data(Vec4::new(texel.x, texel.y, 1., 0.), one),
            b,
            None,
        ),
        // The vertical blur carries the bloom amount.
        pass(
            blur,
            b,
            b,
            pass_data(
                Vec4::new(texel.x, texel.y, 0., 1.),
                Vec4::new(bloom.amount, 0., 0., 0.),
            ),
            a,
            None,
        ),
        pass(
            pipeline,
            post.source,
            a,
            pass_data(Vec4::ZERO, one),
            post.destination,
            extra,
        ),
    ];
    for pass in passes {
        pass.draw(&mut ctx, &pipelines, &layout);
    }
}
