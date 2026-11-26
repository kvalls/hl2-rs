//! Direct 2D presentation pass onto the window swapchain without intermediate copies when
//! formats match.
use bevy::{
    app::{App, Plugin},
    asset::{Asset, Handle},
    camera::{CameraOutputMode, ClearColor, ClearColorConfig},
    color::Color,
    core_pipeline::{core_2d::Opaque2d, upscaling::upscaling},
    ecs::{
        prelude::{Res, Resource, World},
        schedule::{IntoScheduleConfigs, Schedule, ScheduleBuildSettings, ScheduleLabel},
    },
    image::Image,
    log::error,
    math::Vec4,
    reflect::TypePath,
    render::{
        RenderApp,
        camera::ExtractedCamera,
        diagnostic::RecordDiagnostics,
        render_phase::ViewBinnedRenderPhases,
        render_resource::{AsBindGroup, RenderPassDescriptor, StoreOp},
        renderer::{RenderContext, ViewQuery},
        view::{ExtractedView, ViewDepthTexture, ViewTarget},
    },
    shader::ShaderRef,
    sprite_render::{Material2d, Material2dPlugin},
};
use std::sync::atomic::{AtomicBool, Ordering};

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
struct NeedsUpscaling(AtomicBool);

fn present_schedule() -> Schedule {
    let mut schedule = Schedule::new(Present2d);
    schedule.set_build_settings(ScheduleBuildSettings {
        auto_insert_apply_deferred: false,
        ..Default::default()
    });
    schedule.add_systems(
        (
            present_pass_2d,
            upscaling.run_if(|flag: Res<NeedsUpscaling>| flag.0.load(Ordering::Relaxed)),
        )
            .chain(),
    );
    schedule
}

impl Plugin for PresentPassPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Material2dPlugin::<PresentMaterial>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<NeedsUpscaling>()
            .add_schedule(present_schedule());
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

fn present_pass_2d(
    world: &World,
    view: ViewQuery<(
        &ExtractedCamera,
        &ExtractedView,
        &ViewTarget,
        &ViewDepthTexture,
    )>,
    opaque_phases: Res<ViewBinnedRenderPhases<Opaque2d>>,
    clear_color_global: Res<ClearColor>,
    needs_upscaling: Res<NeedsUpscaling>,
    mut ctx: RenderContext,
) {
    needs_upscaling.0.store(false, Ordering::Relaxed);
    let view_entity = view.entity();
    let (camera, extracted_view, target, depth) = view.into_inner();
    let Some(opaque_phase) = opaque_phases.get(&extracted_view.retained_view_entity) else {
        return;
    };
    let Some(clear) = target_clear_color(camera.output_mode, clear_color_global.0) else {
        return;
    };
    let clear = clear.map(Into::into);
    let (color_attachment, used_direct) = if target.out_texture_view_format()
        == Some(target.main_texture_format())
        && let Some(out) = target.out_texture_color_attachment(clear)
    {
        (out, true)
    } else {
        needs_upscaling.0.store(true, Ordering::Relaxed);
        (target.get_color_attachment(), false)
    };
    if used_direct && opaque_phase.is_empty() && clear.is_none() {
        return;
    }
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let color_attachments = [Some(color_attachment)];
    let depth_stencil_attachment = Some(depth.get_attachment(StoreOp::Store));
    let mut render_pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("present_pass_2d"),
        color_attachments: &color_attachments,
        depth_stencil_attachment,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    let pass_span = diagnostics.pass_span(&mut render_pass, "present_pass_2d");
    if let Some(viewport) = camera.viewport.as_ref() {
        render_pass.set_camera_viewport(viewport);
    }
    if !opaque_phase.is_empty()
        && let Err(err) = opaque_phase.render(&mut render_pass, world, view_entity)
    {
        error!("Error encountered while rendering the 2d present phase {err:?}");
    }
    pass_span.end(&mut render_pass);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn present_schedule_initializes_without_param_conflicts() {
        let mut world = World::new();
        let mut schedule = present_schedule();
        schedule.initialize(&mut world).expect("Present2d schedule");
    }
}
