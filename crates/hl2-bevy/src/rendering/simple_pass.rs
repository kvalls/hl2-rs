//! Lean single-pass 3D render schedule shared by the sky and monitor cameras.
use bevy::{
    app::{App, Plugin},
    camera::{CameraOutputMode, ClearColor, MainPassResolutionOverride, Viewport},
    color::Color,
    core_pipeline::{
        core_3d::{AlphaMask3d, Opaque3d, Transparent3d},
        upscaling::upscaling,
    },
    ecs::{
        prelude::World,
        query::{With, Without},
        resource::Resource,
        schedule::{
            IntoScheduleConfigs, Schedule, ScheduleBuildSettings, ScheduleLabel,
            common_conditions::any_match_filter,
        },
        system::{Res, SystemParam},
    },
    log::error,
    render::{
        RenderApp,
        batching::gpu_preprocessing::GpuPreprocessingSupport,
        camera::ExtractedCamera,
        diagnostic::RecordDiagnostics,
        render_phase::{ViewBinnedRenderPhases, ViewSortedRenderPhases},
        render_resource::{RenderPassDescriptor, StoreOp, TextureFormat},
        renderer::{RenderContext, ViewQuery},
        view::{ExtractedView, Msaa, NoIndirectDrawing, ViewDepthStencilTexture, ViewTarget},
    },
};
use std::sync::atomic::{AtomicBool, Ordering};

/// Single-pass 3D schedule for auxiliary cameras without prepass, bloom, tonemapping, or MSAA
/// writeback (draws `Opaque3d`, `AlphaMask3d`, and `Transparent3d` in one pass with depth discard).
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Simple3d;

pub struct SimplePassPlugin;

#[derive(Resource, Default)]
struct NeedsSimpleUpscaling(AtomicBool);

fn simple_schedule() -> Schedule {
    let mut schedule = Schedule::new(Simple3d);
    schedule.set_build_settings(ScheduleBuildSettings {
        auto_insert_apply_deferred: false,
        ..Default::default()
    });
    schedule.add_systems(
        (
            simple_pass_3d,
            upscaling.run_if(|flag: Res<NeedsSimpleUpscaling>| flag.0.load(Ordering::Relaxed)),
        )
            .chain(),
    );
    schedule
}

impl Plugin for SimplePassPlugin {
    fn build(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<NeedsSimpleUpscaling>()
            .add_schedule(simple_schedule());
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        // `PreprocessPipelines` is inserted later in `RenderStartup` via `init_gpu_resource`, so
        // `finish()` checks `GpuPreprocessingSupport` just like Bevy's `GpuMeshPreprocessPlugin`.
        let gpu_preprocess_ready = render_app
            .world()
            .get_resource::<GpuPreprocessingSupport>()
            .is_some_and(GpuPreprocessingSupport::is_available);
        if gpu_preprocess_ready {
            render_app.add_systems(
                Simple3d,
                (
                    bevy::pbr::allocate_uniforms,
                    bevy::pbr::unpack_bins,
                    bevy::pbr::early_gpu_preprocess,
                    bevy::pbr::main_build_indirect_parameters.run_if(
                        any_match_filter::<(
                            With<bevy::pbr::PreprocessBindGroups>,
                            Without<bevy::pbr::SkipGpuPreprocess>,
                            Without<NoIndirectDrawing>,
                        )>,
                    ),
                )
                    .chain()
                    .before(simple_pass_3d),
            );
        }
    }
}

#[derive(SystemParam)]
struct SimplePhases<'w> {
    opaque: Res<'w, ViewBinnedRenderPhases<Opaque3d>>,
    alpha_mask: Res<'w, ViewBinnedRenderPhases<AlphaMask3d>>,
    transparent: Res<'w, ViewSortedRenderPhases<Transparent3d>>,
    clear_color_global: Res<'w, ClearColor>,
    needs_upscaling: Res<'w, NeedsSimpleUpscaling>,
}

fn use_direct_output(
    has_write: bool,
    msaa: Msaa,
    out_format: Option<TextureFormat>,
    main_format: TextureFormat,
) -> bool {
    has_write && msaa == Msaa::Off && out_format == Some(main_format)
}

fn resolve_target_attachment<'a>(
    target: &'a ViewTarget,
    msaa: Msaa,
    clear: Option<Option<Color>>,
    needs_upscaling: &NeedsSimpleUpscaling,
) -> bevy::render::render_resource::RenderPassColorAttachment<'a> {
    if let Some(clear) = clear
        && use_direct_output(
            true,
            msaa,
            target.out_texture_view_format(),
            target.main_texture_format(),
        )
        && let Some(out) = target.out_texture_color_attachment(clear.map(Into::into))
    {
        return out;
    }
    if clear.is_some() {
        needs_upscaling.0.store(true, Ordering::Relaxed);
    }
    target.get_color_attachment()
}

type SimpleView<'w, 's> = ViewQuery<
    'w,
    's,
    (
        &'static ExtractedCamera,
        &'static ExtractedView,
        &'static ViewTarget,
        &'static ViewDepthStencilTexture,
        &'static Msaa,
        Option<&'static MainPassResolutionOverride>,
    ),
>;

fn simple_pass_3d(world: &World, view: SimpleView, phases: SimplePhases, mut ctx: RenderContext) {
    phases.needs_upscaling.0.store(false, Ordering::Relaxed);
    let view_entity = view.entity();
    let (camera, extracted_view, target, depth, msaa, resolution_override) = view.into_inner();
    let (Some(opaque_phase), Some(alpha_mask_phase)) = (
        phases.opaque.get(&extracted_view.retained_view_entity),
        phases.alpha_mask.get(&extracted_view.retained_view_entity),
    ) else {
        return;
    };
    let transparent_phase = phases.transparent.get(&extracted_view.retained_view_entity);
    let clear = match camera.output_mode {
        CameraOutputMode::Write { .. } => crate::video::present_pass::target_clear_color(
            camera.output_mode,
            phases.clear_color_global.0,
        ),
        CameraOutputMode::Skip => None,
    };
    let color_attachment = resolve_target_attachment(target, *msaa, clear, &phases.needs_upscaling);
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let color_attachments = [Some(color_attachment)];
    let depth_stencil_attachment = Some(depth.get_attachment(StoreOp::Discard));
    let mut render_pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("simple_pass_3d"),
        color_attachments: &color_attachments,
        depth_stencil_attachment,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    let pass_span = diagnostics.pass_span(&mut render_pass, "simple_pass_3d");
    if let Some(viewport) =
        Viewport::from_viewport_and_override(camera.viewport.as_ref(), resolution_override)
    {
        render_pass.set_camera_viewport(&viewport);
    }
    if !opaque_phase.is_empty()
        && let Err(err) = opaque_phase.render(&mut render_pass, world, view_entity)
    {
        error!("Error encountered while rendering simple 3D opaque phase {err:?}");
    }
    if !alpha_mask_phase.is_empty()
        && let Err(err) = alpha_mask_phase.render(&mut render_pass, world, view_entity)
    {
        error!("Error encountered while rendering simple 3D alpha-mask phase {err:?}");
    }
    if let Some(transparent_phase) = transparent_phase
        && !transparent_phase.items.is_empty()
        && let Err(err) = transparent_phase.render(&mut render_pass, world, view_entity)
    {
        error!("Error encountered while rendering simple 3D transparent phase {err:?}");
    }
    pass_span.end(&mut render_pass);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_schedule_initializes_without_param_conflicts() {
        let mut world = World::new();
        let mut schedule = simple_schedule();
        schedule.initialize(&mut world).expect("Simple3d schedule");
    }

    #[test]
    fn direct_target_selection_matches_monitor_and_skips_sky_msaa() {
        let srgb = TextureFormat::Rgba8UnormSrgb;
        let bgra = TextureFormat::Bgra8UnormSrgb;
        // MonitorCamera: Write output, Msaa::Off, matching Rgba8UnormSrgb target -> direct.
        assert!(use_direct_output(true, Msaa::Off, Some(srgb), srgb));
        // SkyCamera: CameraOutputMode::Skip -> shared MSAA target, never direct or upscaling.
        assert!(!use_direct_output(false, Msaa::Sample4, Some(srgb), srgb));
        // MSAA enabled with Write output -> falls back to get_color_attachment() + upscaling.
        assert!(!use_direct_output(true, Msaa::Sample4, Some(srgb), srgb));
        // Mismatched output format -> falls back to get_color_attachment() + upscaling.
        assert!(!use_direct_output(true, Msaa::Off, Some(bgra), srgb));
        assert!(!use_direct_output(true, Msaa::Off, None, srgb));
    }
}
