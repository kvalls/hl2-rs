//! One render pass for a 3D camera's opaque, alpha-mask, skybox and transparent draws (O1).
//! Bevy's `Core3d` encodes `main_opaque_pass_3d` and `main_transparent_pass_3d` as two render
//! passes, so every camera stores its multisampled colour and depth to memory after the opaque
//! draws and loads them again for the transparent ones. At 4x MSAA in Retina that round trip cost
//! about as much as the viewmodel camera's own drawing. The draws and their order are Bevy's; only
//! the pass boundary between them goes. Order-independent transparency is not used by HL2-RS.
use bevy::{
    app::{App, Plugin},
    camera::{MainPassResolutionOverride, Viewport},
    core_pipeline::{
        core_3d::{
            AlphaMask3d, Opaque3d, Transparent3d, main_opaque_pass_3d, main_transparent_pass_3d,
        },
        schedule::{Core3d, Core3dSystems},
        skybox::{SkyboxBindGroup, SkyboxPipelineId},
    },
    ecs::{
        prelude::World,
        schedule::{IntoScheduleConfigs, ScheduleCleanupPolicy},
        system::{Res, SystemParam},
    },
    log::error,
    render::{
        RenderApp,
        camera::ExtractedCamera,
        diagnostic::RecordDiagnostics,
        render_phase::{ViewBinnedRenderPhases, ViewSortedRenderPhases},
        render_resource::{PipelineCache, RenderPassDescriptor, StoreOp},
        renderer::{RenderContext, ViewQuery},
        view::{ExtractedView, ViewDepthStencilTexture, ViewTarget, ViewUniformOffset},
    },
};

pub struct MainPassPlugin;

impl Plugin for MainPassPlugin {
    fn build(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        for removed in [
            render_app.remove_systems_in_set(
                Core3d,
                main_opaque_pass_3d,
                ScheduleCleanupPolicy::RemoveSystemsOnly,
            ),
            render_app.remove_systems_in_set(
                Core3d,
                main_transparent_pass_3d,
                ScheduleCleanupPolicy::RemoveSystemsOnly,
            ),
        ] {
            debug_assert_eq!(removed.ok(), Some(1), "Bevy's main 3D pass systems");
        }
        render_app.add_systems(Core3d, main_pass_3d.in_set(Core3dSystems::MainPass));
    }
}

#[derive(SystemParam)]
struct MainPhases<'w> {
    opaque: Res<'w, ViewBinnedRenderPhases<Opaque3d>>,
    alpha_mask: Res<'w, ViewBinnedRenderPhases<AlphaMask3d>>,
    transparent: Res<'w, ViewSortedRenderPhases<Transparent3d>>,
    pipelines: Res<'w, PipelineCache>,
}

type MainView<'w, 's> = ViewQuery<
    'w,
    's,
    (
        &'static ExtractedCamera,
        &'static ExtractedView,
        &'static ViewTarget,
        &'static ViewDepthStencilTexture,
        Option<&'static SkyboxPipelineId>,
        Option<&'static SkyboxBindGroup>,
        &'static ViewUniformOffset,
        Option<&'static MainPassResolutionOverride>,
    ),
>;

fn main_pass_3d(world: &World, view: MainView, phases: MainPhases, mut ctx: RenderContext) {
    let view_entity = view.entity();
    let (camera, extracted_view, target, depth, skybox, skybox_group, uniform, resolution) =
        view.into_inner();
    let key = &extracted_view.retained_view_entity;
    let (Some(opaque), Some(alpha_mask)) = (phases.opaque.get(key), phases.alpha_mask.get(key))
    else {
        return;
    };
    let transparent = phases.transparent.get(key);
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let color_attachments = [Some(target.get_color_attachment())];
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("main_pass_3d"),
        color_attachments: &color_attachments,
        depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store)),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    let span = diagnostics.pass_span(&mut pass, "main_pass_3d");
    if let Some(viewport) =
        Viewport::from_viewport_and_override(camera.viewport.as_ref(), resolution)
    {
        pass.set_camera_viewport(&viewport);
    }
    if !opaque.is_empty()
        && let Err(err) = opaque.render(&mut pass, world, view_entity)
    {
        error!("Error encountered while rendering the opaque phase {err:?}");
    }
    if !alpha_mask.is_empty()
        && let Err(err) = alpha_mask.render(&mut pass, world, view_entity)
    {
        error!("Error encountered while rendering the alpha mask phase {err:?}");
    }
    if let (Some(skybox), Some(SkyboxBindGroup(group))) = (skybox, skybox_group)
        && let Some(pipeline) = phases.pipelines.get_render_pipeline(skybox.0)
    {
        pass.set_render_pipeline(pipeline);
        pass.set_bind_group(0, &group.0, &[uniform.offset, group.1]);
        pass.draw(0..3, 0..1);
    }
    if let Some(transparent) = transparent.filter(|phase| !phase.items.is_empty())
        && let Err(err) = transparent.render(&mut pass, world, view_entity)
    {
        error!("Error encountered while rendering the transparent phase {err:?}");
    }
    span.end(&mut pass);
}
