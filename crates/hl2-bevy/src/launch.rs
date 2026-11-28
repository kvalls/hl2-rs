//! The default plugin set configured from the command line, the host systems and the final report.
use super::{
    Options, Status, assets, audio, camera, campaign, canvas_renderer, console_host, effects, eyes,
    flashlight, material_proxies, material_slots, monitor, monitors, rendering, runtime_npcs,
    runtime_props, save_capture, setup, sky, studio_decals, tonemap, video, visibility,
};
use anyhow::{Result, bail};
use bevy::app::PluginGroupBuilder;
use bevy::prelude::{
    App, AppExit, AssetPlugin, DefaultPlugins, IVec2, IntoScheduleConfigs, Last, PluginGroup,
    PostUpdate, PreUpdate, RunFixedMainLoop, Startup, TransformSystems, Update, Window,
    WindowPlugin, default, resource_exists,
};
use bevy::window::WindowResolution;

pub(super) fn default_plugins(
    options: &Options,
    asset_root: &std::path::Path,
) -> PluginGroupBuilder {
    let default_plugins = DefaultPlugins
        .set(crate::priority::task_pools())
        .set(bevy::log::LogPlugin {
            custom_layer: console_host::log_layer,
            ..default()
        })
        .set(AssetPlugin {
            file_path: asset_root.to_string_lossy().into_owned(),
            ..default()
        })
        .set(bevy::audio::AudioPlugin {
            global_volume: bevy::audio::GlobalVolume::from(bevy::audio::Volume::Linear(
                options.volume,
            )),
            ..default()
        })
        .set(WindowPlugin {
            primary_window: Some(Window {
                // A background run has no OS window to lose focus to: it counts as
                // focused, so gameplay does not pause itself.
                focused: options.background || !options.no_focus,
                position: options
                    .window_at
                    .map_or(bevy::window::WindowPosition::Automatic, |(x, y)| {
                        bevy::window::WindowPosition::At(IVec2::new(x, y))
                    }),
                present_mode: if options.uncapped {
                    bevy::window::PresentMode::AutoNoVsync
                } else {
                    bevy::window::PresentMode::AutoVsync
                },
                title: "HL2-RS | Bevy/wgpu migration | gameplay migration preview".into(),
                resolution: WindowResolution::new(options.width, options.height),
                // A placed borderless window starts windowed where it was placed
                // (`borderless_on_placed_monitor`).
                mode: if placed_borderless(options) {
                    bevy::window::WindowMode::Windowed
                } else {
                    video::mode::startup_mode(options.display_mode)
                },
                // macOS: Bevy's default hides the menu bar and Dock outright in
                // borderless full screen; without it the native full-screen Space shows
                // them (and the title bar) when the pointer reaches the top edge.
                borderless_game: false,
                ..default()
            }),
            ..default()
        })
        // Bevy 0.20 puts its UI widgets in DefaultPlugins. The game draws no Bevy menus, and
        // MenuPlugin (after VisibilityPropagate, before UiSystems::PostLayout, whose text layout
        // runs before AssetEventSystems and so before CalculateBounds) leaves no room for
        // `visibility::present` between CalculateBounds and VisibilityPropagate: PostUpdate
        // would not build.
        .disable::<bevy::ui_widgets::MenuPlugin>()
        // TextInputPlugin sets `Window::ime_enabled` every frame from whether a Bevy
        // `EditableText` has focus. The game has none (the console is drawn on the CPU canvas),
        // so it turned macOS composition off each frame and the console lost dead-key quotes;
        // `ui_system::native_text` owns that flag instead. The setup window keeps the plugin.
        .disable::<bevy::ui_widgets::TextInputPlugin>();
    // Background runs have no operating-system window: without winit nothing is created on the
    // desktop or activated (winit activates the app on launch). Profiling runs uncapped;
    // ordinary background replays retain the fixed-rate loop.
    if options.background {
        default_plugins.disable::<bevy::winit::WinitPlugin>()
    } else {
        default_plugins
    }
}

/// `--window-at` with borderless full screen: a window can only be made full screen on the
/// primary monitor before it exists, so it is created windowed at that position.
fn placed_borderless(options: &Options) -> bool {
    options.window_at.is_some()
        && matches!(
            video::mode::startup_mode(options.display_mode),
            bevy::window::WindowMode::BorderlessFullscreen(_)
        )
}

/// Once the placed window exists, it goes borderless full screen on its own monitor.
fn borderless_on_placed_monitor(
    mut windows: bevy::prelude::Query<
        &mut Window,
        (
            bevy::prelude::With<bevy::window::PrimaryWindow>,
            bevy::prelude::With<bevy::window::RawHandleWrapper>,
        ),
    >,
    options: bevy::prelude::Res<Options>,
    mut done: bevy::prelude::Local<bool>,
) {
    if *done {
        return;
    }
    if let Ok(mut window) = windows.single_mut() {
        window.mode = video::mode::desired_mode(options.display_mode);
        *done = true;
    }
}

pub(super) fn add_video_systems(app: &mut App) {
    app.add_systems(
        Update,
        borderless_on_placed_monitor
            .run_if(|options: bevy::prelude::Res<Options>| placed_borderless(&options)),
    );
    app.add_systems(PreUpdate, crate::video::present_frame)
        .add_systems(PostUpdate, crate::video::update_present_gamma)
        .add_systems(PostUpdate, crate::video::sync_present_mode)
        .add_systems(PostUpdate, crate::video::sync_antialiasing)
        .add_systems(PostUpdate, crate::video::sync_filtering)
        .add_systems(
            Update,
            (
                crate::video::apply_video_settings,
                crate::video::resize_window,
            )
                .chain(),
        )
        .add_systems(
            PostUpdate,
            crate::video::present_frame
                .before(rendering::present_entities)
                .before(sky::present),
        )
        .add_systems(
            PostUpdate,
            (
                crate::video::suspend_for_resize,
                crate::video::record_frame_size,
                crate::camera::record_camera_fov,
            )
                .chain()
                .after(TransformSystems::Propagate),
        );
}

pub(super) fn add_presentation_systems(app: &mut App) {
    rendering::drawing::install_previous_transforms(app);
    app.init_resource::<rendering::actor_poses::ActorPoses>();
    app.add_systems(
        RunFixedMainLoop,
        campaign::poll
            .in_set(bevy::app::RunFixedMainLoopSystems::AfterFixedMainLoop)
            .before(camera::present_player_view),
    )
    .add_systems(
        PostUpdate,
        (
            rendering::present_entities.run_if(crate::isolation::run_animation),
            rendering::lod::switch_levels.after(rendering::present_entities),
            rendering::variant_warmup::warm_hidden_variants
                .after(rendering::present_entities)
                .before(bevy::transform::TransformSystems::Propagate),
            rendering::present_weapons,
            rendering::variant_warmup::warm_weapon_variants
                .after(rendering::present_weapons)
                .before(rendering::sync_viewmodel_camera)
                .before(rendering::variant_warmup::warm_hidden_variants),
            effects::present.run_if(crate::isolation::run_effects),
            effects::present_gibs.run_if(crate::isolation::run_effects),
            studio_decals::present,
            sky::present,
            monitors::present,
            monitors::present_materials,
            eyes::attention_pvs,
            material_proxies::present,
            canvas_renderer::present
                .after(flashlight::update)
                .run_if(crate::isolation::run_hud),
            tonemap::update,
            rendering::sync_viewmodel_camera
                .after(rendering::present_weapons)
                .after(effects::present)
                .after(tonemap::update),
        )
            .before(TransformSystems::Propagate),
    )
    .add_systems(
        PostUpdate,
        runtime_props::poll
            .run_if(resource_exists::<runtime_props::RuntimeProps>)
            .before(rendering::present_entities),
    );
    add_runtime_entity_systems(app);
    app.add_plugins(material_slots::plugin);
    app.add_systems(Startup, setup)
        .add_systems(
            PostUpdate,
            audio::queue
                .after(canvas_renderer::present)
                .before(TransformSystems::Propagate)
                .run_if(crate::isolation::run_audio),
        )
        .add_systems(Last, effects::observe.before(monitor::monitor))
        .add_systems(Last, audio::observe.before(monitor::monitor))
        .add_systems(
            Update,
            (
                console_host::sync_sources,
                console_host::drain_log,
                console_host::apply_cursor_hint,
            ),
        )
        .add_systems(Last, monitor::monitor)
        .add_systems(Last, video::skip_covered_outputs);
}

fn add_runtime_entity_systems(app: &mut App) {
    app.add_systems(
        PostUpdate,
        runtime_npcs::poll
            .run_if(resource_exists::<runtime_npcs::RuntimeNpcs>)
            .before(rendering::spawn_runtime_draws)
            .before(rendering::present_entities),
    )
    .add_systems(
        PostUpdate,
        (
            rendering::despawn_killed_runtime_draws,
            rendering::spawn_runtime_draws,
        )
            .chain()
            .before(rendering::present_entities)
            .before(eyes::present),
    )
    .add_systems(
        PostUpdate,
        rendering::swap_runtime_bodies
            .after(rendering::spawn_runtime_draws)
            .before(rendering::present_entities),
    )
    .add_systems(
        PostUpdate,
        rendering::present_ragdoll_fades.after(rendering::present_entities),
    )
    .add_systems(
        PostUpdate,
        (
            rendering::present_render_colors,
            rendering::apply_lightstyles.run_if(crate::isolation::run_lightstyles),
        )
            .after(rendering::present_entities),
    )
    .add_systems(
        PostUpdate,
        rendering::present_lighting
            .after(rendering::present_entities)
            .after(effects::present)
            .before(TransformSystems::Propagate)
            .run_if(crate::isolation::run_model_lighting),
    )
    .add_systems(
        PostUpdate,
        eyes::present
            .after(rendering::present_entities)
            .before(TransformSystems::Propagate)
            .run_if(crate::isolation::run_eyes),
    )
    .add_systems(
        PostUpdate,
        visibility::present
            .after(bevy::camera::visibility::VisibilitySystems::CalculateBounds)
            .before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
    );
}

/// What the startup map reports; the worker replaces it after each map change.
#[derive(Clone)]
pub(super) struct MapSummary {
    map_name: String,
    revision: u32,
    models: serde_json::Value,
    textures: serde_json::Value,
    texture_errors: Vec<String>,
    warnings: Vec<String>,
    spawn_sky_visibility: String,
}

impl MapSummary {
    pub(super) fn of(loaded: &assets::LoadedMap) -> Self {
        let unique_textures: std::collections::BTreeMap<_, _> = loaded
            .materials
            .values()
            .filter_map(|material| {
                material
                    .base_path
                    .as_ref()
                    .zip(material.base.as_ref())
                    .map(|(key, image)| (key.clone(), image.data.len()))
            })
            .collect();
        Self {
            map_name: loaded.world.name.clone(),
            revision: loaded.revision,
            models: loaded.models.clone(),
            textures: serde_json::json!({
                "source_materials": loaded.materials.len(), "unique_base_textures": unique_textures.len(),
                "decoded_base_bytes": unique_textures.values().sum::<usize>(), "decoded_base_budget": 512 * 1024 * 1024,
                "owned_eye_dx8_fallbacks":loaded.materials.iter().filter(|(_,m)|m.eye_fallback).map(|(name,_)|name).collect::<Vec<_>>(),
            }),
            texture_errors: loaded.texture_errors.clone(),
            warnings: loaded.world.warnings.clone(),
            spawn_sky_visibility: format!(
                "{:?}",
                loaded.bsp.sky_visibility(loaded.world.spawn().0)
            ),
        }
    }
}

/// The startup map's summary, for the report written in the exit frame.
#[derive(bevy::prelude::Resource)]
pub(super) struct ExitSummary(pub(super) MapSummary);

/// The frame the app exits: writes the run's captures and report ([`finish`]) and ends the process
/// without dropping the app. Dropping it closes the CoreAudio output, which waits for the mixing
/// callback to let go of the device lock; after a crowd fight that wait can last forever, the
/// window gone and a core still busy. The operating system releases the device instead. Both
/// trace files flush as they write.
pub(super) fn exit_process(world: &mut bevy::prelude::World) {
    let exit = world
        .get_resource::<bevy::ecs::message::Messages<AppExit>>()
        .and_then(|messages| messages.iter_current_update_messages().next().cloned());
    let Some(exit) = exit else {
        return;
    };
    if let Some(mut simulation) = world.get_resource_mut::<crate::simulation::Simulation>() {
        simulation.with_extensions(|extension, _| extension.shutdown());
    }
    let status = world.resource::<Status>().clone();
    let options = world.resource::<Options>().clone();
    let summary = world.resource::<ExitSummary>().0.clone();
    // The process exits without dropping resources: finish the trace file first.
    if let Some(switch) = world.get_resource::<crate::performance::status::TraceSwitch>() {
        (switch.0)(crate::performance::status::TraceCommand::Finish);
    }
    let code = match finish(&status, &options, summary, exit) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("Error: {error:?}");
            1
        }
    };
    std::process::exit(code);
}

pub(super) fn finish(
    status: &Status,
    options: &Options,
    summary: MapSummary,
    exit: AppExit,
) -> Result<()> {
    let mut status = status
        .0
        .lock()
        .map_err(|_| anyhow::anyhow!("render status lock poisoned"))?;
    // Readback is retained in memory during the app. PNG/report writes happen
    // only after the host exits, so renderer systems perform no blocking I/O.
    let capture_write_error = save_capture(status.captured_image.take(), options.capture.as_ref());
    let monitor_capture_write_error = save_capture(
        status.monitor_image.take(),
        options.monitor_capture.as_ref(),
    );
    let monitor_capture_exists = options.monitor_capture.as_ref().is_none_or(|p| p.is_file());
    let capture_exists = options.capture.as_ref().is_none_or(|path| path.is_file());
    let mut report = serde_json::json!({
        "runtime": "Bevy 0.19.1 / wgpu gameplay migration preview", "map": summary.map_name, "bsp_revision": summary.revision,
        "render": &*status, "models": summary.models, "textures": summary.textures, "texture_errors": summary.texture_errors, "asset_warnings": summary.warnings,
        "capture_file_exists": capture_exists, "capture_write_error": capture_write_error,
        "monitor_capture_file_exists":monitor_capture_exists,"monitor_capture_write_error":monitor_capture_write_error, "spawn_sky_visibility": summary.spawn_sky_visibility,
        "limitations": ["Pause/console and landmark/inventory map transitions migrated; complete command coverage, save/global state and native effects remain incomplete", "Audio uses shared script selection/decoding and Bevy sinks; mixing is 2D without Source DSP, spatialization or soundscapes", "Retained incomplete scene/AI/weapon behavior; missing animation clips remain bind poses", "Sky uses owned faces (RGBS HDR faces when present)/leaf visibility; sky polygon masks, area portals/occluders, material proxies and dynamic lighting remain unfinished", "Source HDR path (mat_hdr_level 2): HDR lightmaps capped at the integer range, auto exposure from a pre-bloom histogram and Source 8-bit bloom; LightmappedGeneric $envmap cubemaps from map patch materials (env_cubemap on entities, $envmapmask, bumped and model envmaps, $selfillum and an LDR mode are not implemented)"]
    });
    for key in [
        "map",
        "bsp_revision",
        "models",
        "textures",
        "texture_errors",
        "asset_warnings",
        "spawn_sky_visibility",
    ] {
        if let Some(value) = status.map_metadata.get(key) {
            report[key] = value.clone();
        }
    }
    std::fs::write(&options.report, serde_json::to_vec_pretty(&report)?)?;
    println!("Report: {}", options.report.display());
    if !matches!(exit, AppExit::Success) {
        bail!("Bevy runner exited with {exit:?}");
    }
    if options.movement_script.is_some()
        && !status.simulation["script_finished"]
            .as_bool()
            .unwrap_or(false)
    {
        bail!("movement script did not finish; inspect report/log");
    }
    if let Some(rejected) = status.simulation["extension_rejected"]
        .as_array()
        .filter(|rejected| !rejected.is_empty())
    {
        bail!("no selected extension handled: {rejected:?}");
    }
    if options.capture.is_some()
        && (!status.capture_completed || !capture_exists || capture_write_error.is_some())
    {
        bail!("requested screenshot did not complete; inspect report/log");
    }
    if options.monitor_capture.is_some()
        && (!status.monitor_capture_completed
            || !monitor_capture_exists
            || monitor_capture_write_error.is_some())
    {
        bail!("requested monitor screenshot did not complete; inspect report/log");
    }
    Ok(())
}

/// `--timescale`: the virtual clock runs `scale` times faster, so the fixed 15 ms ticks (and
/// everything on virtual time) come due that much sooner; one frame may run up to `scale` times
/// the normal 50 ms of ticks. The ticks themselves, and their order, do not change.
pub(crate) fn apply_timescale(app: &mut App, scale: f32) {
    if scale == 1. {
        return;
    }
    let mut time = app
        .world_mut()
        .resource_mut::<bevy::time::Time<bevy::time::Virtual>>();
    time.set_relative_speed(scale);
    time.set_max_delta(std::time::Duration::from_secs_f32(0.05 * scale));
}

/// `--max-fps`: each frame waits out the rest of its share of a second before the next begins.
/// Added only when asked for; play never runs it.
pub(crate) fn limit_frame_rate(app: &mut App, max_fps: Option<u32>) {
    let Some(fps) = max_fps else { return };
    let period = std::time::Duration::from_secs_f64(1. / f64::from(fps));
    let mut last = std::time::Instant::now();
    app.add_systems(bevy::app::Last, move || {
        let due = last + period;
        let now = std::time::Instant::now();
        if due > now {
            std::thread::sleep(due - now);
        }
        last = std::time::Instant::now();
    });
}

#[cfg(test)]
mod tests {
    /// Bevy's text-input widget plugin would set `Window::ime_enabled` to false every frame
    /// (no Bevy `EditableText` is focused in the game), which dropped the console's macOS
    /// dead-key quotes; the game window leaves that flag to `ui_system::native_text`.
    #[test]
    fn game_plugins_leave_text_composition_to_the_console() {
        let (options, _) =
            super::Options::parse_from(Vec::new(), hl2_ui::config::Config::default()).unwrap();
        let plugins = super::default_plugins(&options, std::path::Path::new("assets"));
        assert!(plugins.contains::<bevy::ui_widgets::TextInputPlugin>());
        assert!(!plugins.enabled::<bevy::ui_widgets::TextInputPlugin>());
        assert!(plugins.enabled::<bevy::window::WindowPlugin>());
    }
}
