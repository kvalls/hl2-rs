//! Direct 2D presentation pass onto the window swapchain without intermediate copies.
use bevy::{
    app::{App, Plugin},
    asset::{Asset, AssetApp, AssetId, Assets, Handle},
    camera::{CameraOutputMode, ClearColor, ClearColorConfig, visibility::Visibility},
    color::Color,
    core_pipeline::FullscreenShader,
    ecs::{
        component::Component,
        entity::Entity,
        query::With,
        resource::Resource,
        schedule::{IntoScheduleConfigs, Schedule, ScheduleBuildSettings, ScheduleLabel},
        system::{Commands, Query, Res, ResMut, SystemParam},
    },
    image::Image,
    math::{UVec2, Vec4},
    reflect::TypePath,
    render::{
        Extract, ExtractSchedule, GpuResourceAppExt, Render, RenderApp, RenderStartup,
        RenderSystems,
        camera::ExtractedCamera,
        diagnostic::RecordDiagnostics,
        extract_component::ExtractComponentPlugin,
        render_asset::RenderAssets,
        render_resource::{
            binding_types::{sampler, texture_2d, uniform_buffer},
            *,
        },
        renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
        texture::GpuImage,
        view::ViewTarget,
    },
    shader::Shader,
    shader::ShaderRef,
    sprite_render::{Material2d, MeshMaterial2d},
    utils::default,
};

/// Draws the game frame (3D view, HUD and menus) in the window through the monitor gamma ramp.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct PresentMaterial {
    /// x: [`super::monitor_gamma_exponent`].
    #[uniform(0)]
    pub params: Vec4,
    #[texture(1)]
    #[sampler(2)]
    pub frame: Handle<Image>,
}

impl Material2d for PresentMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/present.wgsl".into()
    }
}

#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Present2d;

pub struct PresentPassPlugin;

#[derive(Resource, Default)]
struct ExtractedPresent {
    frame: Option<AssetId<Image>>,
    gamma_exponent: f32,
    visible: bool,
    physical_pos: UVec2,
    physical_size: UVec2,
}

#[derive(Resource)]
struct PresentPipeline {
    layout: BindGroupLayoutDescriptor,
    fullscreen: FullscreenShader,
    shader: Handle<Shader>,
    uniform: Buffer,
}

#[derive(Component)]
struct ViewPresentPipeline(CachedRenderPipelineId, TextureFormat);

#[derive(Resource, Default)]
struct PresentCache {
    gamma: Option<f32>,
    bind_group: Option<(TextureViewId, SamplerId, BindGroup)>,
}

impl SpecializedRenderPipeline for PresentPipeline {
    type Key = TextureFormat;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("present_pipeline".into()),
            layout: vec![self.layout.clone()],
            vertex: self.fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                entry_point: Some("fragment".into()),
                targets: vec![Some(ColorTargetState {
                    format: key,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            ..default()
        }
    }
}

fn present_schedule() -> Schedule {
    let mut schedule = Schedule::new(Present2d);
    schedule.set_build_settings(ScheduleBuildSettings {
        auto_insert_apply_deferred: false,
        ..Default::default()
    });
    schedule.add_systems(present_pass_2d);
    schedule
}

impl Plugin for PresentPassPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<PresentMaterial>()
            .add_plugins(ExtractComponentPlugin::<super::PresentCamera>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<ExtractedPresent>()
            .init_resource::<PresentCache>()
            .init_gpu_resource::<SpecializedRenderPipelines<PresentPipeline>>()
            .add_systems(RenderStartup, init_present_pipeline)
            .add_systems(ExtractSchedule, extract_present_state)
            .add_systems(
                Render,
                prepare_present_pipeline.in_set(RenderSystems::Prepare),
            )
            .add_schedule(present_schedule());
    }
}

fn init_present_pipeline(
    mut commands: Commands,
    device: Res<RenderDevice>,
    fullscreen: Res<FullscreenShader>,
    assets: Res<bevy::asset::AssetServer>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "present_pass_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<Vec4>(false),
            ),
        ),
    );
    let mut buffer = encase::UniformBuffer::new(Vec::new());
    buffer.write(&Vec4::ONE).expect("present uniform");
    let uniform = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("present_uniform"),
        contents: buffer.as_ref(),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    });
    commands.insert_resource(PresentPipeline {
        layout,
        fullscreen: fullscreen.clone(),
        shader: assets.load("shaders/present.wgsl"),
        uniform,
    });
}

type PresentQuads<'w, 's> = Query<
    'w,
    's,
    (
        &'static MeshMaterial2d<PresentMaterial>,
        &'static Visibility,
    ),
    With<super::PresentQuad>,
>;

fn extract_present_state(
    frame: Extract<Option<Res<super::GameFrame>>>,
    viewport: Extract<Option<Res<super::VideoViewport>>>,
    quads: Extract<PresentQuads>,
    materials: Extract<Res<Assets<PresentMaterial>>>,
    mut extracted: ResMut<ExtractedPresent>,
) {
    let quad = quads.iter().next();
    let material = quad.and_then(|(handle, _)| materials.get(&handle.0));
    extracted.frame = material
        .map(|m| m.frame.id())
        .or_else(|| frame.as_ref().map(|f| f.image.id()));
    extracted.gamma_exponent = material.map_or(1., |m| m.params.x);
    let (physical_pos, physical_size) = viewport.as_ref().map_or((UVec2::ZERO, UVec2::ZERO), |v| {
        (
            v.physical_rect.physical_position,
            v.physical_rect.physical_size,
        )
    });
    extracted.physical_pos = physical_pos;
    extracted.physical_size = physical_size;
    let quad_visible = quad.is_none_or(|(_, vis)| *vis != Visibility::Hidden);
    extracted.visible = frame.as_ref().is_some_and(|f| f.suspended == 0)
        && quad_visible
        && physical_size.x > 0
        && physical_size.y > 0;
}

fn prepare_present_pipeline(
    mut commands: Commands,
    mut pipeline_cache: ResMut<PipelineCache>,
    mut pipelines: ResMut<SpecializedRenderPipelines<PresentPipeline>>,
    present_pipeline: Res<PresentPipeline>,
    views: Query<(Entity, &ViewTarget, Option<&ViewPresentPipeline>), With<super::PresentCamera>>,
) {
    for (entity, target, current) in &views {
        let Some(format) = target.out_texture_view_format() else {
            continue;
        };
        if current.is_some_and(|p| p.1 == format) {
            continue;
        }
        let id = pipelines.specialize(&pipeline_cache, &present_pipeline, format);
        pipeline_cache.block_on_render_pipeline(id);
        commands
            .entity(entity)
            .insert(ViewPresentPipeline(id, format));
    }
}

pub(crate) fn target_clear_color(
    output_mode: CameraOutputMode,
    global: Color,
) -> Option<Option<Color>> {
    let clear_color = match output_mode {
        CameraOutputMode::Write { clear_color, .. } => clear_color,
        CameraOutputMode::Skip => return None,
    };
    Some(match clear_color {
        ClearColorConfig::Default => Some(global),
        ClearColorConfig::Custom(color) => Some(color),
        ClearColorConfig::None => None,
    })
}

fn clamped_viewport(pos: UVec2, size: UVec2, target_size: Option<UVec2>) -> (u32, u32, u32, u32) {
    let Some(target) = target_size else {
        return (pos.x, pos.y, size.x, size.y);
    };
    let x = pos.x.min(target.x);
    let y = pos.y.min(target.y);
    let w = size.x.min(target.x.saturating_sub(x));
    let h = size.y.min(target.y.saturating_sub(y));
    (x, y, w, h)
}

fn sync_present_cache(
    device: &RenderDevice,
    queue: &RenderQueue,
    pipeline_cache: &PipelineCache,
    present_pipeline: &PresentPipeline,
    images: &RenderAssets<GpuImage>,
    state: &ExtractedPresent,
    present_cache: &mut PresentCache,
) -> bool {
    let Some(frame_id) = state.frame else {
        return false;
    };
    let Some(image) = images.get(frame_id) else {
        return false;
    };
    if present_cache.gamma != Some(state.gamma_exponent) {
        let mut buffer = encase::UniformBuffer::new(Vec::new());
        buffer
            .write(&Vec4::new(state.gamma_exponent, 0., 0., 0.))
            .expect("present uniform");
        queue.write_buffer(&present_pipeline.uniform, 0, buffer.as_ref());
        present_cache.gamma = Some(state.gamma_exponent);
    }
    let ids = (image.texture_view.id(), image.sampler.id());
    if present_cache
        .bind_group
        .as_ref()
        .is_none_or(|(vid, sid, _)| (*vid, *sid) != ids)
    {
        let layout = pipeline_cache.get_bind_group_layout(&present_pipeline.layout);
        let group = device.create_bind_group(
            Some("present_pass_bind_group"),
            &layout,
            &BindGroupEntries::sequential((
                &image.texture_view,
                &image.sampler,
                present_pipeline.uniform.as_entire_binding(),
            )),
        );
        present_cache.bind_group = Some((ids.0, ids.1, group));
    }
    true
}

#[derive(SystemParam)]
struct PresentGpu<'w> {
    clear_color_global: Res<'w, ClearColor>,
    pipeline_cache: Res<'w, PipelineCache>,
    present_pipeline: Res<'w, PresentPipeline>,
    images: Res<'w, RenderAssets<GpuImage>>,
    queue: Res<'w, RenderQueue>,
}

fn present_pass_2d(
    view: ViewQuery<(&ExtractedCamera, &ViewTarget, Option<&ViewPresentPipeline>)>,
    gpu: PresentGpu,
    state: Res<ExtractedPresent>,
    mut present_cache: ResMut<PresentCache>,
    mut ctx: RenderContext,
) {
    let (camera, target, view_pipeline) = view.into_inner();
    let Some(clear) = target_clear_color(camera.output_mode, gpu.clear_color_global.0) else {
        return;
    };
    let Some(out) = target.out_texture_color_attachment(clear.map(Into::into)) else {
        return;
    };
    let pipeline = view_pipeline.and_then(|p| gpu.pipeline_cache.get_render_pipeline(p.0));
    let has_group = state.visible
        && pipeline.is_some()
        && sync_present_cache(
            ctx.render_device(),
            &gpu.queue,
            &gpu.pipeline_cache,
            &gpu.present_pipeline,
            &gpu.images,
            &state,
            &mut present_cache,
        );
    let (x, y, w, h) = clamped_viewport(
        state.physical_pos,
        state.physical_size,
        camera.physical_target_size,
    );
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let color_attachments = [Some(out)];
    let mut render_pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("present_pass_2d"),
        color_attachments: &color_attachments,
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    let pass_span = diagnostics.pass_span(&mut render_pass, "present_pass_2d");
    if has_group
        && w > 0
        && h > 0
        && let (Some(pipeline), Some((_, _, bind_group))) =
            (pipeline, present_cache.bind_group.as_ref())
    {
        render_pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
        render_pass.set_scissor_rect(x, y, w, h);
        render_pass.set_render_pipeline(pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
    pass_span.end(&mut render_pass);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn present_schedule_initializes_without_param_conflicts() {
        let mut world = bevy::ecs::world::World::new();
        let mut schedule = present_schedule();
        schedule.initialize(&mut world).expect("Present2d schedule");
    }

    #[test]
    fn clamped_viewport_handles_fit_letterbox_oversized_and_zero_sizes() {
        assert_eq!(
            clamped_viewport(
                UVec2::ZERO,
                UVec2::new(3456, 1978),
                Some(UVec2::new(3456, 1978))
            ),
            (0, 0, 3456, 1978)
        );
        assert_eq!(
            clamped_viewport(
                UVec2::new(0, 120),
                UVec2::new(1920, 840),
                Some(UVec2::new(1920, 1080))
            ),
            (0, 120, 1920, 840)
        );
        assert_eq!(
            clamped_viewport(
                UVec2::new(100, 200),
                UVec2::new(2000, 1200),
                Some(UVec2::new(1920, 1080))
            ),
            (100, 200, 1820, 880)
        );
        assert_eq!(
            clamped_viewport(
                UVec2::new(2000, 0),
                UVec2::new(400, 300),
                Some(UVec2::new(1920, 1080))
            ),
            (1920, 0, 0, 300)
        );
        assert_eq!(
            clamped_viewport(UVec2::ZERO, UVec2::ZERO, Some(UVec2::new(1920, 1080))),
            (0, 0, 0, 0)
        );
    }
}
