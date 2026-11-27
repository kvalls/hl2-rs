//! Direct 1x HUD pass onto the game frame image without MSAA writeback or an extra copy.
use bevy::{
    app::{App, Plugin},
    camera::ClearColor,
    core_pipeline::core_2d::Transparent2d,
    ecs::{
        prelude::{Res, World},
        schedule::{Schedule, ScheduleBuildSettings, ScheduleLabel},
    },
    log::{error, warn_once},
    render::{
        RenderApp,
        camera::ExtractedCamera,
        diagnostic::RecordDiagnostics,
        render_phase::ViewSortedRenderPhases,
        render_resource::{RenderPassDescriptor, StoreOp},
        renderer::{RenderContext, ViewQuery},
        view::{ExtractedView, ViewDepthStencilTexture, ViewTarget},
    },
};

#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Hud2d;

pub struct HudPassPlugin;

impl Plugin for HudPassPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy::sprite_render::Material2dPlugin::<super::HudMaterial>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        let mut schedule = Schedule::new(Hud2d);
        schedule.set_build_settings(ScheduleBuildSettings {
            auto_insert_apply_deferred: false,
            ..Default::default()
        });
        render_app
            .add_schedule(schedule)
            .add_systems(Hud2d, hud_pass_2d);
    }
}

fn hud_pass_2d(
    world: &World,
    view: ViewQuery<(
        &ExtractedCamera,
        &ExtractedView,
        &ViewTarget,
        &ViewDepthStencilTexture,
    )>,
    transparent_phases: Res<ViewSortedRenderPhases<Transparent2d>>,
    clear_color_global: Res<ClearColor>,
    mut ctx: RenderContext,
) {
    let view_entity = view.entity();
    let (camera, extracted_view, target, depth) = view.into_inner();
    let Some(transparent_phase) = transparent_phases.get(&extracted_view.retained_view_entity)
    else {
        return;
    };
    let Some(clear) =
        crate::video::present_pass::target_clear_color(camera.output_mode, clear_color_global.0)
    else {
        return;
    };
    let clear = clear.map(Into::into);
    if transparent_phase.items.is_empty() && clear.is_none() {
        return;
    }
    if target.out_texture_view_format() != Some(target.main_texture_format()) {
        warn_once!("Hud2d direct pass skipped due to output format mismatch");
        return;
    }
    let Some(out_attachment) = target.out_texture_color_attachment(clear) else {
        return;
    };
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let color_attachments = [Some(out_attachment)];
    let depth_stencil_attachment = Some(depth.get_attachment(StoreOp::Discard));
    let mut render_pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("hud_pass_2d"),
        color_attachments: &color_attachments,
        depth_stencil_attachment,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    let pass_span = diagnostics.pass_span(&mut render_pass, "hud_pass_2d");
    if let Some(viewport) = camera.viewport.as_ref() {
        render_pass.set_camera_viewport(viewport);
    }
    if !transparent_phase.items.is_empty()
        && let Err(err) = transparent_phase.render(&mut render_pass, world, view_entity)
    {
        error!("Error encountered while rendering the transparent 2D HUD phase {err:?}");
    }
    pass_span.end(&mut render_pass);
}
