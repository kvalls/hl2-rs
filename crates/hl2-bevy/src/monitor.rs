//! Per-frame run diagnostics, captures and the unattended exit.
use super::{
    CaptureControl, FlyCamera, Options, RunStatus, Status, audio, bevy_to_source, campaign,
    canvas_renderer, capture_complete, effects, eyes, flashlight, gameplay, material_proxies,
    monitor_capture_complete, monitors, performance, rendering, shadows, simulation, sky,
    studio_decals, tonemap, ui_host, video, visibility,
};
use bevy::prelude::{
    AppExit, Assets, Camera, Commands, Entity, Image, Mesh, Mesh3d, MeshMaterial3d, MessageWriter,
    Name, Query, Res, ResMut, Transform, ViewVisibility, Visibility, With, info,
};
use bevy::render::{renderer::RenderAdapterInfo, view::screenshot::Screenshot};

type EntityDrawQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static rendering::SourceEntity,
        &'static Transform,
        &'static Visibility,
        Option<&'static visibility::PvsDraw>,
        Option<&'static rendering::NpcWeaponDraw>,
    ),
    With<Mesh3d>,
>;
type HostResources<'w> = (
    Res<'w, Options>,
    Res<'w, simulation::Simulation>,
    Res<'w, gameplay::Gameplay>,
    Res<'w, canvas_renderer::Hud>,
    Res<'w, audio::Audio>,
    Res<'w, sky::Sky>,
    Res<'w, monitors::Monitors>,
    Res<'w, eyes::Eyes>,
    Res<'w, effects::Effects>,
    Res<'w, ui_host::Console>,
    Res<'w, campaign::Campaign>,
    Option<Res<'w, shadows::MapShadows>>,
    Res<'w, flashlight::HostFlashlight>,
);
type DiagnosticQueries<'w, 's> = (
    Query<'w, 's, &'static Transform, With<FlyCamera>>,
    EntityDrawQuery<'w, 's>,
    Query<'w, 's, Entity, With<campaign::MapOwned>>,
    Query<'w, 's, Entity, With<Camera>>,
    Res<'w, Assets<Mesh>>,
    Res<'w, Assets<Image>>,
    Query<'w, 's, (&'static ViewVisibility, &'static rendering::DrawTriangles)>,
    Query<
        'w,
        's,
        (
            &'static Name,
            &'static material_proxies::TwoTextureDraw,
            &'static MeshMaterial3d<rendering::SourceMaterial>,
        ),
    >,
);
type RenderResources<'w> = (
    Option<Res<'w, RenderAdapterInfo>>,
    Res<'w, visibility::SourceVisibility>,
    Res<'w, tonemap::Tonemap>,
    Res<'w, Assets<rendering::SourceMaterial>>,
);
/// Exit writer and optional diagnostics read by `monitor`.
type ExitResources<'w> = (
    MessageWriter<'w, AppExit>,
    Option<Res<'w, performance::Performance>>,
    Option<Res<'w, bevy::diagnostic::DiagnosticsStore>>,
    Res<'w, studio_decals::StudioDecals>,
    Option<Res<'w, video::GameFrame>>,
);
pub(super) fn monitor(
    mut commands: Commands,
    (
        options,
        simulation,
        game,
        hud,
        audio,
        sky,
        monitors,
        eyes,
        effects,
        console,
        campaign,
        shadows,
        flashlight,
    ): HostResources,
    mut control: ResMut<CaptureControl>,
    status: Res<Status>,
    (adapter, pvs, tonemap, source_materials): RenderResources,
    (cameras, draws, map_entities, all_cameras, meshes, images, geometry, two_textures): DiagnosticQueries,
    (mut exit, performance, render_diagnostics, studio, frame): ExitResources,
) {
    let _timing = performance::scope(performance.as_deref(), "diagnostics");
    if simulation.loading() {
        return;
    }
    control.frames += 1;
    let mut report = status.0.lock().expect("status lock");
    report.frames = control.frames;
    // Full diagnostic serialization is sampled for play; captures and exits always get a fresh snapshot.
    let snapshot = control.frames == 1
        || control.frames.is_multiple_of(60)
        || control.requested
        || simulation.finished
        || options.frames.is_some_and(|limit| control.frames >= limit)
        || console.quit_requested;
    if snapshot {
        report.map_metadata = campaign.metadata.clone();
        report.simulation = simulation.report();
        report.simulation["gameplay"] = simulation.gameplay_report(&game);
        let (mismatches, owned_meshes, doors) = draw_consistency(&draws, &simulation, &game);
        report.presentation = serde_json::json!({"source_visibility":pvs.report(),"owned_meshes":owned_meshes,"pose_or_visibility_mismatches":mismatches,"station_entrance_draws":doors,"hud":simulation.hud_report(&hud),"audio":audio.report(),"sky":sky.report(),"monitors":monitors.report(&game),"eyes":eyes.report(),"effects":effects.report(),"studio_decals":studio.report(),"console":console.report(),"campaign":campaign.report(),"tonemap":tonemap.report(),"shadows":shadows.as_deref().map(shadows::MapShadows::report),"flashlight":flashlight.report(game.scene.flashlight.state()),
        "lifecycle":{"map_entities":map_entities.iter().count(),"cameras":all_cameras.iter().count(),"live_mesh_assets":meshes.len(),"live_image_assets":images.len()}});
        report.presentation["two_texture_materials"] =
            material_proxies::report(two_textures.iter(), &source_materials, game.scene.time);
        report.presentation["render_candidates"] = performance::render_candidates(geometry.iter());
        if let Some(performance) = performance.as_deref() {
            performance.write_report(&mut report.presentation, render_diagnostics.as_deref());
        }
        if let Some(adapter) = adapter {
            report.adapter = Some(adapter.name.clone());
            report.backend = Some(format!("{:?}", adapter.backend));
        }
        if let Ok(camera) = cameras.single() {
            report.camera_source = bevy_to_source(camera.translation).to_array();
            report.simulation["camera_forward"] =
                serde_json::json!(bevy_to_source(*camera.forward()).to_array());
        }
    }
    advance_capture(
        &mut commands,
        &mut control,
        &report,
        &CaptureContext {
            options: &options,
            simulation: &simulation,
            game: &game,
            monitors: &monitors,
            frame: frame.as_deref(),
        },
        &mut exit,
    );
}

/// Whether each owned draw matches its simulated pose and visibility, and the checked doors.
fn draw_consistency(
    draws: &EntityDrawQuery,
    simulation: &simulation::Simulation,
    game: &gameplay::Gameplay,
) -> (usize, usize, Vec<serde_json::Value>) {
    let mut mismatches = 0;
    let mut owned_meshes = 0;
    let mut doors = Vec::new();
    for (owner, transform, visibility, pvs, weapon) in draws {
        owned_meshes += 1;
        let state = &game.scene.states[owner.0];
        let (origin, rotation) = simulation
            .physics
            .entity_pose(owner.0)
            .unwrap_or((state.origin, state.rotation));
        let expected = rendering::entity_transform(origin, rotation);
        let hidden =
            rendering::draw_hidden(game, owner.0, weapon) || pvs.is_some_and(|pvs| pvs.culled);
        if transform.translation.distance(expected.translation) > 0.001
            || transform.rotation.dot(expected.rotation).abs() < 0.99999
            || (*visibility == Visibility::Hidden) != hidden
        {
            mismatches += 1;
        }
        if game.world.entities[owner.0].get("targetname") == Some("station_entrance") {
            doors.push(serde_json::json!({"entity":owner.0,"origin":bevy_to_source(transform.translation).to_array(),
            "bevy_rotation":transform.rotation.to_array(),"hidden":hidden}));
        }
    }
    (mismatches, owned_meshes, doors)
}

struct CaptureContext<'a> {
    options: &'a Options,
    simulation: &'a simulation::Simulation,
    game: &'a gameplay::Gameplay,
    monitors: &'a monitors::Monitors,
    frame: Option<&'a video::GameFrame>,
}

/// Requests the screenshots of an unattended run and ends it once they completed.
fn advance_capture(
    commands: &mut Commands,
    control: &mut CaptureControl,
    report: &RunStatus,
    context: &CaptureContext,
    exit: &mut MessageWriter<AppExit>,
) {
    let CaptureContext {
        options,
        simulation,
        game,
        monitors,
        frame,
    } = *context;
    // Captures read the game frame (the picture before it is scaled into the window), so they do
    // not depend on the window being shown; `--capture-window` reads the window.
    let capture_source = || match frame {
        Some(frame) if !options.capture_window => Screenshot::image(frame.image.clone()),
        _ => Screenshot::primary_window(),
    };
    if control.requested
        && (options.capture.is_none() || report.capture_completed)
        && (options.monitor_capture.is_none() || report.monitor_capture_completed)
        && control.completed_frame.is_none()
    {
        control.completed_frame = Some(control.frames);
    }
    if options.frames.is_some() || options.movement_script.is_some() {
        let limit = options.frames.unwrap_or(u64::MAX - 600);
        if (control.frames >= limit || simulation.finished)
            && !simulation.loading()
            && !control.requested
        {
            control.requested = true;
            control.requested_frame = Some(control.frames);
            if options.capture.is_some() {
                commands.spawn(capture_source()).observe(capture_complete);
            }
            if options.monitor_capture.is_some() {
                commands
                    .spawn(Screenshot::image(monitors.target.clone()))
                    .observe(monitor_capture_complete);
            }
            if options.capture.is_none() && options.monitor_capture.is_none() {
                exit.write(AppExit::Success);
            }
        }
        // Burst frames: PNG-N.png for each of the frames after the requested capture.
        if let (Some(path), Some(frame)) = (&options.capture, control.requested_frame) {
            let index = control.frames - frame;
            if (1..=u64::from(options.capture_burst)).contains(&index) {
                let burst = path.with_file_name(format!(
                    "{}-{index}.png",
                    path.file_stem().unwrap_or_default().to_string_lossy()
                ));
                // Scene time per burst frame, so live bursts can be compared over time.
                info!("capture burst {index} scene_time {:.4}", game.scene.time);
                commands
                    .spawn(capture_source())
                    .observe(bevy::render::view::screenshot::save_to_disk(burst));
            }
        }
        if control.completed_frame.is_some_and(|frame| {
            control.frames >= frame + 5
                && control.frames
                    >= control.requested_frame.unwrap_or(0) + u64::from(options.capture_burst) + 5
        }) {
            exit.write(AppExit::Success);
        }
        if control
            .requested_frame
            .is_some_and(|frame| control.frames > frame + 600)
        {
            exit.write(AppExit::error());
        }
    }
}
