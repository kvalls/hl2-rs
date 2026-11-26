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
        renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
        texture::{CachedTexture, TextureCache},
        view::{ExtractedView, ViewTarget},
    },
};

pub mod color_correction;
mod correction_pass;
pub(crate) mod fullscreen_pass;
pub mod motion_blur;
mod motion_blur_pass;
/// Bloom amount on the last 3D camera of the main view, and the image that receives the
/// pre-bloom frame for auto exposure.
#[derive(Component, Clone, ExtractComponent)]
pub struct SourceBloom {
    pub amount: f32,
    pub presample: Option<Handle<Image>>,
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct BloomSet;
pub struct SourceBloomPlugin;
impl Plugin for SourceBloomPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<SourceBloom>::default());
        app.add_plugins(ExtractComponentPlugin::<
            color_correction::SourceColorCorrection,
        >::default())
            .init_resource::<color_correction::Lookups>()
            .add_systems(Update, color_correction::update);
        app.add_plugins(motion_blur_pass::MotionBlurPlugin)
            .init_resource::<motion_blur::MotionBlurState>()
            .add_systems(Update, motion_blur::update);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<BloomCache>()
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
                    .in_set(BloomSet)
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
    constant_uniform: Buffer,
    blur_h_uniform: Buffer,
    blur_v_uniform: Buffer,
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
    let make_uniform = |label, data: PassData| {
        let mut buffer = encase::UniformBuffer::new(Vec::new());
        buffer.write(&data).expect("bloom uniform");
        device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some(label),
            contents: buffer.as_ref(),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        })
    };
    let one = pass_data(Vec4::ZERO, Vec4::new(1., 0., 0., 0.));
    commands.insert_resource(Pipelines {
        layout,
        correction_layout: correction_pass::layout(),
        sampler,
        shape,
        blur,
        presample,
        fullscreen,
        shader,
        constant_uniform: make_uniform("source_bloom_const", one),
        blur_h_uniform: make_uniform("source_bloom_blur_h", one),
        blur_v_uniform: make_uniform("source_bloom_blur_v", one),
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

const MAX_CACHED_BIND_GROUPS: usize = 16;
const UNIFORM_CONSTANT: u8 = 0;
const UNIFORM_BLUR_H: u8 = 1;
const UNIFORM_BLUR_V: u8 = 2;

/// Cached bind groups and last blur uniform parameters for `source_bloom`.
#[derive(Resource, Default)]
struct BloomCache {
    last_blur: Option<(u32, u32, u32)>,
    groups: std::collections::HashMap<(TextureViewId, TextureViewId, u8), BindGroup>,
}

impl BloomCache {
    fn should_sync_blur(&mut self, width: u32, height: u32, amount: f32) -> bool {
        let bits = if amount == 0. { 0 } else { amount.to_bits() };
        let key = (width, height, bits);
        if self.last_blur == Some(key) {
            return false;
        }
        self.last_blur = Some(key);
        true
    }

    fn sync_blur_uniforms(
        &mut self,
        queue: &RenderQueue,
        pipelines: &Pipelines,
        width: u32,
        height: u32,
        amount: f32,
    ) {
        if !self.should_sync_blur(width, height, amount) {
            return;
        }
        let texel = Vec2::new(1. / width as f32, 1. / height as f32);
        write_uniform(
            queue,
            &pipelines.blur_h_uniform,
            pass_data(
                Vec4::new(texel.x, texel.y, 1., 0.),
                Vec4::new(1., 0., 0., 0.),
            ),
        );
        write_uniform(
            queue,
            &pipelines.blur_v_uniform,
            pass_data(
                Vec4::new(texel.x, texel.y, 0., 1.),
                Vec4::new(amount, 0., 0., 0.),
            ),
        );
    }

    fn bind_group<'a>(
        &'a mut self,
        device: &RenderDevice,
        layout: &BindGroupLayout,
        sampler: &Sampler,
        source: &TextureView,
        glow: &TextureView,
        uniform: (u8, &Buffer),
    ) -> &'a BindGroup {
        let key = (source.id(), glow.id(), uniform.0);
        if self.groups.len() >= MAX_CACHED_BIND_GROUPS && !self.groups.contains_key(&key) {
            self.groups.clear();
        }
        self.groups.entry(key).or_insert_with(|| {
            device.create_bind_group(
                Some("source_bloom_bind_group"),
                layout,
                &BindGroupEntries::sequential((
                    source,
                    glow,
                    sampler,
                    uniform.1.as_entire_binding(),
                )),
            )
        })
    }
}

impl Pipelines {
    fn uniform(&self, id: u8) -> (u8, &Buffer) {
        let buf = match id {
            UNIFORM_BLUR_H => &self.blur_h_uniform,
            UNIFORM_BLUR_V => &self.blur_v_uniform,
            _ => &self.constant_uniform,
        };
        (id, buf)
    }
}

/// One fullscreen draw of the bloom shader.
struct Pass<'a> {
    pipeline: &'a RenderPipeline,
    source: &'a TextureView,
    glow: &'a TextureView,
    uniform: u8,
    destination: &'a TextureView,
    /// Group 1, for the composite that applies color correction.
    extra: Option<&'a BindGroup>,
}

impl Pass<'_> {
    fn draw(
        self,
        ctx: &mut RenderContext,
        pipelines: &Pipelines,
        layout: &BindGroupLayout,
        bind_cache: &mut BloomCache,
    ) {
        let device = ctx.render_device().clone();
        let bind_group = bind_cache.bind_group(
            &device,
            layout,
            &pipelines.sampler,
            self.source,
            self.glow,
            pipelines.uniform(self.uniform),
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
        pass.set_bind_group(0, bind_group, &[]);
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

fn write_uniform(queue: &RenderQueue, target: &Buffer, data: PassData) {
    let mut buffer = encase::UniformBuffer::new(Vec::new());
    buffer.write(&data).expect("bloom uniform");
    queue.write_buffer(target, 0, buffer.as_ref());
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
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
    images: Res<bevy::render::render_asset::RenderAssets<bevy::render::texture::GpuImage>>,
    mut bloom_cache: ResMut<BloomCache>,
    mut ctx: RenderContext,
) {
    let (target, bloom, small, composite, correction) = view.into_inner();
    let layout = cache.get_bind_group_layout(&pipelines.layout);
    if let (Some(pipeline), Some(image)) = (
        cache.get_render_pipeline(pipelines.presample),
        bloom.presample.as_ref().and_then(|h| images.get(h)),
    ) {
        let frame = target.main_texture_view();
        Pass {
            pipeline,
            source: frame,
            glow: frame,
            uniform: UNIFORM_CONSTANT,
            destination: &image.texture_view,
            extra: None,
        }
        .draw(&mut ctx, &pipelines, &layout, &mut bloom_cache);
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
                uniform: UNIFORM_CONSTANT,
                destination: post.destination,
                extra: Some(group),
            }
            .draw(&mut ctx, &pipelines, &layout, &mut bloom_cache);
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
    bloom_cache.sync_blur_uniforms(&queue, &pipelines, size.width, size.height, bloom.amount);
    let (a, b) = (&small.0[0].default_view, &small.0[1].default_view);
    let post = target.post_process_write();
    let pass = |pipeline, source, glow, uniform, destination, extra| Pass {
        pipeline,
        source,
        glow,
        uniform,
        destination,
        extra,
    };
    let (pipeline, extra) = corrected.map_or((composite, None), |(p, group)| (p, Some(group)));
    let passes = [
        pass(shape, post.source, post.source, UNIFORM_CONSTANT, a, None),
        pass(blur, a, a, UNIFORM_BLUR_H, b, None),
        // The vertical blur carries the bloom amount.
        pass(blur, b, b, UNIFORM_BLUR_V, a, None),
        pass(
            pipeline,
            post.source,
            a,
            UNIFORM_CONSTANT,
            post.destination,
            extra,
        ),
    ];
    for pass in passes {
        pass.draw(&mut ctx, &pipelines, &layout, &mut bloom_cache);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_uniform_cache_skips_redundant_writes() {
        let mut cache = BloomCache::default();
        assert!(cache.should_sync_blur(480, 270, 0.5));
        assert!(!cache.should_sync_blur(480, 270, 0.5));
        assert!(cache.should_sync_blur(480, 270, 0.75));
        assert!(!cache.should_sync_blur(480, 270, 0.75));
        assert!(cache.should_sync_blur(864, 542, 0.75));
        assert!(cache.should_sync_blur(864, 542, 0.0));
        assert!(!cache.should_sync_blur(864, 542, -0.0));
    }
}
