//! The default plugin set configured from the command line, the host systems and the final report.
use super::{
    Options, Status, assets, audio, camera, campaign, canvas_renderer, console_host, effects, eyes,
    flashlight, material_proxies, monitor, monitors, rendering, runtime_props, save_capture, setup,
    sky, studio_decals, tonemap, video, visibility,
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
        });
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
    app.add_systems(
        RunFixedMainLoop,
        campaign::poll
            .in_set(bevy::app::RunFixedMainLoopSystems::AfterFixedMainLoop)
            .before(camera::present_player_view),
    )
    .add_systems(
        PostUpdate,
        (
            rendering::present_entities,
            rendering::lod::switch_levels.after(rendering::present_entities),
            rendering::present_weapons,
            effects::present,
            effects::present_gibs,
            studio_decals::present,
            sky::present,
            monitors::present,
            monitors::present_materials,
            eyes::attention_pvs,
            material_proxies::present,
            canvas_renderer::present.after(flashlight::update),
            tonemap::update,
        )
            .before(TransformSystems::Propagate),
    )
    .add_systems(
        PostUpdate,
        runtime_props::poll
            .run_if(resource_exists::<runtime_props::RuntimeProps>)
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
        rendering::present_render_colors.after(rendering::present_entities),
    )
    .add_systems(
        PostUpdate,
        rendering::present_lighting
            .after(rendering::present_entities)
            .after(effects::present)
            .before(TransformSystems::Propagate),
    )
    .add_systems(
        PostUpdate,
        eyes::present
            .after(rendering::present_entities)
            .before(TransformSystems::Propagate),
    )
    .add_systems(
        PostUpdate,
        visibility::present
            .after(bevy::camera::visibility::VisibilitySystems::CalculateBounds)
            .before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
    )
    .add_systems(Startup, setup)
    .add_systems(
        PostUpdate,
        audio::queue
            .after(canvas_renderer::present)
            .before(TransformSystems::Propagate),
    )
    .add_systems(Last, effects::observe.before(monitor::monitor))
    .add_systems(Last, audio::observe.before(monitor::monitor))
    .add_systems(
        Update,
        (
            console_host::sync_sources,
            console_host::drain_log,
            console_host::save_history,
            console_host::apply_cursor_hint,
        ),
    )
    .add_systems(Last, monitor::monitor)
    .add_systems(Last, video::skip_covered_outputs);
}

/// What the startup map reports; the worker replaces it after each map change.
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
                    .map(|(key, image)| (key.clone(), image.rgba.len()))
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
