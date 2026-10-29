//! Isolated Bevy/wgpu host reusing engine-independent Source simulation.
mod assets;
mod audio;
mod background_debug;
mod bloom;
mod camera;
mod campaign;
mod canvas_renderer;
mod capture;
mod console_host;
mod coords;
use capture::{capture_complete, monitor_capture_complete, save_capture};
use {flashlight::FlashlightPlugin, shadows::ShadowPlugin};
mod effects;
mod eyes;
mod flashlight;
mod gameplay;
mod gpu_skinning;
mod input;
mod map_install;
mod material_proxies;
mod monitors;
mod performance;
mod rendering;
mod runtime_props;
mod setup;
mod shadows;
mod simulation;
mod sky;
mod spawn_panel;
mod startup;
mod studio_decals;
mod tonemap;
mod ui_host;
mod ui_system;
mod video;
mod visibility;
use anyhow::{Context, Result, bail};
use bevy::{
    app::AppExit,
    asset::AssetPlugin,
    core_pipeline::tonemapping::Tonemapping,
    prelude::*,
    render::{renderer::RenderAdapterInfo, view::screenshot::Screenshot},
    window::{CursorOptions, PrimaryWindow, WindowResolution},
};
pub(crate) use map_install::{StartupAssets, install_loaded_map};
use serde::Serialize;
use startup::{require_map_for, startup_display_mode};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone, Resource)]
struct Options {
    game: PathBuf,
    /// Map to load directly. Without it (or when it cannot be loaded) the main menu opens over
    /// the installed background map.
    map: Option<String>,
    /// Arguments that only make sense in a directly loaded `--map`, in command-line order.
    map_dependent: Vec<&'static str>,
    frames: Option<u64>,
    capture: Option<PathBuf>,
    /// Also save this many consecutive frames after the capture (flicker checks).
    capture_burst: u32,
    /// Keep the simulation running after a movement script ends, so burst frames show
    /// motion over time instead of one frozen frame.
    capture_live: bool,
    monitor_capture: Option<PathBuf>,
    report: PathBuf,
    position: Option<Vec3>,
    yaw: Option<f32>,
    pitch: f32,
    profile: bool,
    uncapped: bool,
    cpu_skinning: bool,
    no_pvs: bool,
    world_partition: bool,
    width: u32,
    height: u32,
    /// Window mode at startup: the stored Video setting, or `--windowed` / `--fullscreen` /
    /// `--borderless`. Unattended runs without a flag start windowed.
    display_mode: hl2_ui::config::DisplayMode,
    /// Video > Aspect Ratio "Fit Display": the game frame follows the window's aspect ratio.
    flexible_aspect: bool,
    fly: bool,
    movement_script: Option<PathBuf>,
    /// Master volume, like Source's `volume` convar (0..1).
    volume: f32,
    /// Silence the output for this run without touching the Audio settings (`--mute`).
    mute: bool,
    /// JSON-lines sound lifecycle trace (requests, sink starts, removals).
    audio_trace: Option<PathBuf>,
    /// Do not take focus at window creation, so unattended tests leave the desktop usable.
    no_focus: bool,
    /// Run without an operating-system window (unattended replays and captures): nothing takes
    /// focus or shows up on the desktop, and the game is stepped at 60 Hz. The game still
    /// renders its frame, which `--capture` reads.
    background: bool,
    /// `--capture` reads the presented window instead of the game frame (checks of the
    /// presentation itself: scaling, letterboxing, brightness).
    capture_window: bool,
    /// Windowed position on the desktop (top-left corner, virtual-screen pixels; may be
    /// on another monitor, e.g. negative X left of the primary).
    window_at: Option<(i32, i32)>,
    /// Fixed HDR tonemap scale (Source mat_force_tonemap_scale); None = auto exposure.
    tonemap_scale: Option<f32>,
    /// `--fake-monitor`: the Video page lists resolutions for this monitor instead of the
    /// host's (window placement and display mode changes still use the real monitor).
    fake_monitor: Option<hl2_ui::gameui::display::MonitorSpec>,
    /// Show the main menu over the loaded map, treating it as a menu background.
    main_menu: bool,
    /// Override the startup main-menu background map.
    background_map: Option<String>,
    /// Skip startup videos on launch (`-novid` / `--novid`).
    novid: bool,
    /// Persisted user configuration.
    config: hl2_ui::config::Config,
}
impl Options {
    fn parse() -> Result<Self> {
        let (options, installation) = Self::parse_from(
            std::env::args().skip(1),
            hl2_ui::config::Config::load_or_default(),
        )?;
        options.finish_startup(installation)
    }
    fn parse_from(
        args: impl IntoIterator<Item = String>,
        config: hl2_ui::config::Config,
    ) -> Result<(Self, startup::InstallationRequest)> {
        let mut args = args.into_iter();
        let mut game = None;
        let mut show_setup = false;
        let mut no_setup = false;
        let mut setup_preview = None;
        let mut options = Self {
            game: PathBuf::new(),
            map: None,
            map_dependent: Vec::new(),
            frames: None,
            capture: None,
            capture_burst: 0,
            capture_live: false,
            monitor_capture: None,
            report: "artifacts/bevy-report.json".into(),
            position: None,
            yaw: None,
            pitch: 0.,
            profile: false,
            uncapped: false,
            cpu_skinning: false,
            no_pvs: false,
            world_partition: false,
            width: config.video.width,
            height: config.video.height,
            display_mode: config.video.display_mode,
            flexible_aspect: config.video.flexible_aspect,
            fly: false,
            movement_script: None,
            volume: config.audio.volume,
            mute: false,
            audio_trace: None,
            no_focus: false,
            background: false,
            capture_window: false,
            window_at: None,
            tonemap_scale: None,
            fake_monitor: None,
            main_menu: false,
            background_map: None,
            novid: false,
            config,
        };
        let mut explicit_mode = None;
        while let Some(arg) = args.next() {
            let next = |args: &mut dyn Iterator<Item = String>| -> Result<String> {
                args.next()
                    .with_context(|| format!("missing value after {arg}"))
            };
            if let Some(name) = [
                "--position",
                "--yaw",
                "--pitch",
                "--fly",
                "--movement-script",
                "--capture-live",
                "--capture-burst",
                "--capture-monitor",
            ]
            .into_iter()
            .find(|name| *name == arg)
            {
                options.map_dependent.push(name);
            }
            match arg.as_str() {
                "--game" => game = Some(PathBuf::from(next(&mut args)?)),
                "--setup" => show_setup = true,
                "--no-setup" => no_setup = true,
                "--setup-preview" => setup_preview = Some(PathBuf::from(next(&mut args)?)),
                "--map" => options.map = Some(next(&mut args)?),
                "--frames" => options.frames = Some(next(&mut args)?.parse()?),
                "--capture" => options.capture = Some(next(&mut args)?.into()),
                "--capture-burst" => options.capture_burst = next(&mut args)?.parse()?,
                "--capture-live" => options.capture_live = true,
                "--capture-monitor" => options.monitor_capture = Some(next(&mut args)?.into()),
                "--report" => options.report = next(&mut args)?.into(),
                "--position" => {
                    options.position = Some(Vec3::new(
                        next(&mut args)?.parse()?,
                        next(&mut args)?.parse()?,
                        next(&mut args)?.parse()?,
                    ))
                }
                "--yaw" => options.yaw = Some(next(&mut args)?.parse::<f32>()?.to_radians()),
                "--no-pvs" => options.no_pvs = true,
                "--world-partition" => options.world_partition = true,
                "--cpu-skinning" => options.cpu_skinning = true,
                "--uncapped" => options.uncapped = true,
                "--profile" => options.profile = true,
                "--pitch" => options.pitch = next(&mut args)?.parse::<f32>()?.to_radians(),
                "--width" => options.width = next(&mut args)?.parse()?,
                "--height" => options.height = next(&mut args)?.parse()?,
                "--borderless" => explicit_mode = Some(hl2_ui::config::DisplayMode::NoBorder),
                "--windowed" => explicit_mode = Some(hl2_ui::config::DisplayMode::Windowed),
                "--fullscreen" => explicit_mode = Some(hl2_ui::config::DisplayMode::Fullscreen),
                "--fly" => options.fly = true,
                "--movement-script" => options.movement_script = Some(next(&mut args)?.into()),
                "--volume" => options.volume = next(&mut args)?.parse()?,
                "--audio-trace" => options.audio_trace = Some(next(&mut args)?.into()),
                "--mute" => options.mute = true,
                "--mute-ambient" => {
                    audio::MUTE_AMBIENT.store(true, std::sync::atomic::Ordering::Relaxed)
                }
                "--frag-swept-box" => {
                    assets::FRAG_RIGID_BODY.store(false, std::sync::atomic::Ordering::Relaxed)
                }
                "--no-focus" => options.no_focus = true,
                "--background" => options.background = true,
                "--capture-window" => options.capture_window = true,
                "--window-at" => {
                    let x = next(&mut args)?.parse()?;
                    let y = next(&mut args)?.parse()?;
                    options.window_at = Some((x, y));
                }
                // Runtime entity limit (Source: MAX_EDICT_BITS 11 = 2048, compiled in);
                // 0 = no limit.
                "--max-edicts" => hl2_simulation::ai::spawn::MAX_EDICTS.store(
                    next(&mut args)?.parse()?,
                    std::sync::atomic::Ordering::Relaxed,
                ),
                // Step 14 AI think loop (default on like Source's ai_disabled 0; console ai_enable).
                "--ai" => gameplay::AI_ENABLED.store(true, std::sync::atomic::Ordering::Relaxed),
                "--no-ai" => {
                    gameplay::AI_ENABLED.store(false, std::sync::atomic::Ordering::Relaxed)
                }
                "--tonemap-scale" => {
                    let scale: f32 = next(&mut args)?.parse()?;
                    if !(scale.is_finite() && scale > 0.) {
                        bail!("--tonemap-scale must be positive");
                    }
                    options.tonemap_scale = Some(scale);
                }
                "--fake-monitor" => {
                    let spec = next(&mut args)?;
                    options.fake_monitor = Some(
                        spec.parse()
                            .with_context(|| format!("--fake-monitor {spec}"))?,
                    );
                }
                "--main-menu" => options.main_menu = true,
                "-novid" | "--novid" => options.novid = true,
                "--background-map" => options.background_map = Some(next(&mut args)?),
                "--help" | "-h" => {
                    startup::print_help();
                    std::process::exit(0);
                }
                _ => bail!("unknown option {arg}; use --help"),
            }
        }
        require_map_for(options.map.as_deref(), &options.map_dependent)?;
        if !(0. ..=1.).contains(&options.volume) {
            bail!("--volume must be between 0 and 1");
        }
        if !(320..=8192).contains(&options.width) || !(240..=8192).contains(&options.height) {
            bail!("window size is outside 320x240..8192x8192");
        }
        if options.frames.is_some_and(|n| !(30..=36000).contains(&n)) {
            bail!("--frames must be between 30 and 36000");
        }
        if !options.position.is_none_or(|p| p.is_finite())
            || !options.pitch.is_finite()
            || !options.yaw.is_none_or(f32::is_finite)
        {
            bail!("camera coordinates and angles must be finite");
        }
        for path in [&options.capture, &options.monitor_capture]
            .into_iter()
            .flatten()
        {
            if path
                .extension()
                .is_none_or(|e| !e.eq_ignore_ascii_case("png"))
            {
                bail!("capture must be a .png path");
            }
        }
        if (options.capture.is_some() || options.monitor_capture.is_some())
            && options.frames.is_none()
            && options.movement_script.is_none()
        {
            options.frames = Some(120);
        }
        let unattended = options.frames.is_some()
            || options.capture.is_some()
            || options.monitor_capture.is_some()
            || options.movement_script.is_some();
        options.display_mode =
            startup_display_mode(options.display_mode, explicit_mode, unattended);
        if (show_setup || setup_preview.is_some()) && no_setup {
            bail!("--setup and --no-setup cannot be used together");
        }
        let installation = startup::InstallationRequest {
            explicit: game,
            force: show_setup || setup_preview.is_some(),
            noninteractive: no_setup || unattended || options.no_focus || options.background,
            preview: setup_preview,
        };
        Ok((options, installation))
    }
}
#[derive(Resource)]
/// The startup map and whether it is a main-menu background level.
struct PreparedMap(Option<assets::LoadedMap>, startup::StartupTarget);
#[derive(Default, Serialize)]
struct RunStatus {
    #[serde(skip)]
    captured_image: Option<Image>,
    #[serde(skip)]
    monitor_image: Option<Image>,
    monitor_capture_completed: bool,
    frames: u64,
    capture_completed: bool,
    capture_size: Option<[u32; 2]>,
    adapter: Option<String>,
    backend: Option<String>,
    meshes: usize,
    gpu_skinned_meshes: usize,
    cpu_skinned_meshes: usize,
    skin_joints: usize,
    world_partition: bool,
    partitioned_world_batches: usize,
    triangles: usize,
    materials: usize,
    skipped_background_surfaces: usize,
    camera_source: [f32; 3],
    simulation: serde_json::Value,
    presentation: serde_json::Value,
    map_metadata: serde_json::Value,
}
#[derive(Clone, Resource, Default)]
struct Status(Arc<Mutex<RunStatus>>);
#[derive(Component)]
struct FlyCamera;
#[derive(Resource, Default)]
struct CaptureControl {
    frames: u64,
    requested: bool,
    requested_frame: Option<u64>,
    completed_frame: Option<u64>,
}
use coords::{bevy_to_source, source_direction, source_to_bevy};

fn main() -> Result<()> {
    if setup::run_helper()? {
        return Ok(());
    }
    let options = Options::parse()?;
    let target = startup::target_map(&options)?;
    let loaded = startup::load_stub(&options.game, &target.game)?;
    let map_name = loaded.world.name.clone();
    let revision = loaded.revision;
    let model_report = loaded.models.clone();
    let texture_errors = loaded.texture_errors.clone();
    let warnings = loaded.world.warnings.clone();
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
    let texture_summary = serde_json::json!({
        "source_materials": loaded.materials.len(), "unique_base_textures": unique_textures.len(),
        "decoded_base_bytes": unique_textures.values().sum::<usize>(), "decoded_base_budget": 512 * 1024 * 1024,
        "owned_eye_dx8_fallbacks":loaded.materials.iter().filter(|(_,m)|m.eye_fallback).map(|(name,_)|name).collect::<Vec<_>>(),
    });
    let spawn_sky_visibility = format!("{:?}", loaded.bsp.sky_visibility(loaded.world.spawn().0));
    let (spawn, spawn_yaw) = loaded.world.spawn();
    let movement_commands = options
        .movement_script
        .as_ref()
        .map(|p| simulation::read_script(p))
        .transpose()?
        .unwrap_or_default();
    let mut simulation = simulation::Simulation::new(
        &loaded.world,
        options
            .position
            .map(|p| glam::Vec3::from_array(p.to_array()))
            .unwrap_or(spawn),
        options.yaw.unwrap_or(spawn_yaw),
        options.pitch,
        options.fly,
        movement_commands,
    );
    simulation.apply_startup_settings(&options, &target);
    let status = Status::default();
    let asset_root = startup::resolve_asset_root()?;
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
                mode: video::mode::startup_mode(options.display_mode),
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
    let default_plugins = if options.background {
        default_plugins.disable::<bevy::winit::WinitPlugin>()
    } else {
        default_plugins
    };
    let mut app = startup::make_app(options.background, options.profile);
    if let Some(path) = &options.audio_trace {
        app.insert_resource(audio::AudioTrace::create(path)?);
    }
    app.insert_resource(options.clone())
        .insert_resource(audio::OutputMute(options.mute))
        .insert_resource(tonemap::Tonemap::new(options.tonemap_scale))
        .insert_resource(status.clone())
        .insert_resource(PreparedMap(Some(loaded), target))
        .insert_resource(simulation)
        .init_resource::<campaign::Campaign>()
        .init_resource::<CaptureControl>()
        .init_resource::<studio_decals::StudioDecals>()
        .insert_resource(console_host::InstalledMaps(
            source_assets::install::installed_maps(&options.game),
        ))
        .init_resource::<console_host::MapEntityNames>()
        .init_resource::<crate::video::VideoViewport>()
        .insert_resource(ClearColor(Color::BLACK))
        .add_plugins(default_plugins)
        .add_plugins(MaterialPlugin::<rendering::SourceMaterial>::default())
        .add_plugins(bevy::sprite_render::Material2dPlugin::<
            video::PresentMaterial,
        >::default())
        .add_plugins(MaterialPlugin::<effects::EffectMaterial>::default())
        .add_plugins((bloom::SourceBloomPlugin, ShadowPlugin, FlashlightPlugin))
        .add_plugins(bevy::sprite_render::Material2dPlugin::<
            canvas_renderer::HudMaterial,
        >::default())
        .add_plugins(ui_system::UiPlugin)
        .add_plugins(simulation::MovementPlugin)
        .add_systems(PreUpdate, crate::video::present_frame)
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
        )
        .add_systems(
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
            rendering::present_ragdoll_fades.after(rendering::present_entities),
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
        .add_systems(Last, effects::observe.before(monitor))
        .add_systems(Last, audio::observe.before(monitor))
        .add_systems(
            Update,
            (
                console_host::sync_sources,
                console_host::drain_log,
                console_host::save_history,
                console_host::apply_cursor_hint,
            ),
        )
        .add_systems(Last, monitor);
    if options.profile {
        performance::install(&mut app);
    }
    // macOS: native full screen (green button) is Windowed (No Border) there, and windowed
    // placement uses the screen's visible frame (without the menu bar and Dock).
    #[cfg(target_os = "macos")]
    app.add_systems(
        PreUpdate,
        (
            crate::video::sync_native_fullscreen.before(crate::video::present_frame),
            crate::video::read_desktop_area,
        ),
    );
    let exit = app.run();
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
        "runtime": "Bevy 0.19.1 / wgpu gameplay migration preview", "map": map_name, "bsp_revision": revision,
        "render": &*status, "models": model_report, "textures": texture_summary, "texture_errors": texture_errors, "asset_warnings": warnings,
        "capture_file_exists": capture_exists, "capture_write_error": capture_write_error,
        "monitor_capture_file_exists":monitor_capture_exists,"monitor_capture_write_error":monitor_capture_write_error, "spawn_sky_visibility": spawn_sky_visibility,
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
fn setup(
    mut commands: Commands,
    mut prepared: ResMut<PreparedMap>,
    options: Res<Options>,
    status: Res<Status>,
    mut assets: StartupAssets,
    mut present_materials: ResMut<Assets<video::PresentMaterial>>,
    (adapter, _windows): (
        Option<Res<RenderAdapterInfo>>,
        Query<&mut CursorOptions, With<PrimaryWindow>>,
    ),
) {
    let loaded = prepared.0.take().expect("startup map consumed once");
    let target = prepared.1.clone();
    let (spawn, yaw) = loaded.world.spawn();
    let position = options
        .position
        .unwrap_or_else(|| Vec3::from_array(spawn.to_array()));
    let yaw = options.yaw.unwrap_or(yaw);
    let ui = hl2_ui::console::Console::load(&loaded.vfs, loaded.hud.canvas.clone());
    install_loaded_map(
        loaded,
        &mut commands,
        &mut assets,
        &status,
        &options,
        options.config.video.advanced.filtering,
        None,
    );
    commands.insert_resource(init_console_ui(ui, adapter.as_deref(), &target, &options));
    video::install_frame(
        &mut commands,
        &mut assets.2,
        &mut assets.0,
        &mut present_materials,
        UVec2::new(options.width, options.height),
        options.flexible_aspect,
        options.display_mode.effective() == hl2_ui::config::DisplayMode::Windowed,
    );
    // Every game camera below renders into the game frame (see `video::present_frame`).
    let max_samples = video::adapter_max_msaa_samples(adapter.as_deref());
    let msaa = video::msaa_for_hardware(options.config.video.advanced.antialiasing, max_samples);
    commands.spawn((
        Camera2d,
        Camera {
            order: 2,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        Tonemapping::None,
        msaa,
        bevy::camera::visibility::RenderLayers::layer(2),
    ));
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: 1,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        bevy::camera::visibility::RenderLayers::layer(1),
        camera::ViewmodelCamera,
        Tonemapping::None,
        bevy::camera::Exposure::default(),
        tonemap::ToneMapped,
        // The viewmodel camera draws last in the 3D view, so bloom covers the whole scene.
        bloom::SourceBloom {
            amount: 1.,
            presample: None,
        },
        msaa,
        Projection::Perspective(PerspectiveProjection {
            fov: 2.
                * ((hl2_simulation::viewmodel_effects::VIEWMODEL_FOV.to_radians() / 2.).tan()
                    / (4. / 3.))
                    .atan(),
            near: 0.1,
            far: 1000.,
            ..default()
        }),
        Transform::IDENTITY.looking_to(Vec3::X, Vec3::Y),
    ));
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        msaa,
        Projection::Perspective(PerspectiveProjection {
            fov: 2. * ((75f32.to_radians() / 2.).tan() / (4. / 3.)).atan(),
            near: 1.,
            far: 32000.,
            ..default()
        }),
        Transform::from_translation(source_to_bevy(position)).looking_to(
            source_to_bevy(source_direction(yaw, options.pitch)),
            Vec3::Y,
        ),
        FlyCamera,
        bevy::camera::Exposure::default(),
        tonemap::ToneMapped,
        bevy::camera::visibility::RenderLayers::layer(0).with(5),
    ));
}

fn init_console_ui(
    ui: hl2_ui::console::Console,
    adapter: Option<&RenderAdapterInfo>,
    target: &startup::StartupTarget,
    options: &Options,
) -> ui_host::Console {
    let mut console_ui = ui_host::Console::new(ui);
    console_ui.source.set_history(console_host::load_history());
    console_ui.source.index_chapters();
    set_system_recommendation(&mut console_ui, adapter);
    request_startup_map(&mut console_ui, target);
    simulation::movie::startup_video_sequence(options, target.background, &mut console_ui);
    console_ui
}

fn set_system_recommendation(
    console_ui: &mut ui_host::Console,
    adapter: Option<&RenderAdapterInfo>,
) {
    let Some(adapter) = adapter else {
        return;
    };
    let max_samples = video::adapter_max_msaa_samples(Some(adapter));
    let device_type_str = format!("{:?}", adapter.0.device_type);
    let device_type = match device_type_str.as_str() {
        "DiscreteGpu" => hl2_ui::gameui::recommendation::GpuDeviceType::Discrete,
        "IntegratedGpu" => hl2_ui::gameui::recommendation::GpuDeviceType::Integrated,
        "Cpu" => hl2_ui::gameui::recommendation::GpuDeviceType::Cpu,
        "VirtualGpu" => hl2_ui::gameui::recommendation::GpuDeviceType::Virtual,
        _ => hl2_ui::gameui::recommendation::GpuDeviceType::Other,
    };
    let gpu = hl2_ui::gameui::recommendation::GpuSpecs {
        name: adapter.0.name.clone(),
        device_type,
        vendor_id: Some(adapter.0.vendor),
        device_id: Some(adapter.0.device),
        backend: Some(format!("{:?}", adapter.0.backend)),
        max_msaa_samples: Some(max_samples),
    };
    let cpu = hl2_ui::gameui::recommendation::CpuSpecs::new(
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4),
    );
    let rec = hl2_ui::gameui::recommendation::SystemRecommendation::from_specs(cpu, gpu);
    console_ui.source.set_system_recommendation(rec);
}

fn request_startup_map(console_ui: &mut ui_host::Console, target: &startup::StartupTarget) {
    if target.background {
        console_ui
            .source
            .start_background_loading(&target.game, &target.map);
        console_ui.background_request = true;
    } else {
        console_ui
            .source
            .start_loading_for_game(&target.game, &target.map);
        console_ui.background_request = false;
    }
    console_ui.map_request = Some(target.map.clone());
    console_ui.map_request_game = Some(target.game.clone());
}
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
fn monitor(
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
    // Captures read the game frame (the picture before it is scaled into the window), so they do
    // not depend on the window being shown; `--capture-window` reads the window.
    let capture_source = || match frame.as_deref() {
        Some(frame) if !options.capture_window => Screenshot::image(frame.image.clone()),
        _ => Screenshot::primary_window(),
    };
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
        report.simulation["gameplay"] = game.report(&simulation.physics);
        let mut mismatches = 0;
        let mut owned_meshes = 0;
        let mut doors = Vec::new();
        for (owner, transform, visibility, pvs, weapon) in &draws {
            owned_meshes += 1;
            let state = &game.scene.states[owner.0];
            let (origin, rotation) = simulation
                .physics
                .entity_pose(owner.0)
                .unwrap_or((state.origin, state.rotation));
            let expected = rendering::entity_transform(origin, rotation);
            let hidden =
                rendering::draw_hidden(&game, owner.0, weapon) || pvs.is_some_and(|pvs| pvs.culled);
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
        report.presentation = serde_json::json!({"source_visibility":pvs.report(),"owned_meshes":owned_meshes,"pose_or_visibility_mismatches":mismatches,"station_entrance_draws":doors,"hud":hud.report(),"audio":audio.report(),"sky":sky.report(),"monitors":monitors.report(&game),"eyes":eyes.report(),"effects":effects.report(),"studio_decals":studio.report(),"console":console.report(),"campaign":campaign.report(),"tonemap":tonemap.report(),"shadows":shadows.as_deref().map(shadows::MapShadows::report),"flashlight":flashlight.report(game.scene.flashlight.state()),
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_axes_round_trip_and_preserve_handedness() {
        let p = Vec3::new(12., -34., 56.);
        assert_eq!(bevy_to_source(source_to_bevy(p)), p);
        assert_eq!(
            source_to_bevy(Vec3::X).cross(source_to_bevy(Vec3::Y)),
            source_to_bevy(Vec3::Z)
        );
    }
    #[test]
    fn source_camera_positive_pitch_looks_up() {
        assert!(source_to_bevy(source_direction(0., 0.5)).y > 0.);
        assert!(source_to_bevy(source_direction(std::f32::consts::FRAC_PI_2, 0.)).z < -0.99);
    }
    #[test]
    fn map_dependent_arguments_require_a_map() {
        assert!(
            require_map_for(None, &[]).is_ok(),
            "plain launch opens the main menu"
        );
        assert!(require_map_for(Some("d1_trainstation_01"), &["--fly"]).is_ok());
        let error = require_map_for(None, &["--position", "--fly"]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("--map is required by --position, --fly")
        );
    }
    #[test]
    fn parsing_rejects_map_dependent_arguments_before_loading_anything() {
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        for (list, flag) in [
            (&["--position", "1", "2", "3"][..], "--position"),
            (&["--yaw", "90"], "--yaw"),
            (&["--pitch", "0"], "--pitch"),
            (&["--fly"], "--fly"),
            (&["--movement-script", "m.json"], "--movement-script"),
            (&["--capture-live"], "--capture-live"),
            (&["--capture-burst", "4"], "--capture-burst"),
            (&["--capture-monitor", "m.png"], "--capture-monitor"),
        ] {
            let error = Options::parse_from(args(list), Default::default())
                .err()
                .expect(flag)
                .to_string();
            assert!(
                error.contains(flag) && error.contains("--map is required"),
                "{error}"
            );
        }
    }
    #[test]
    fn startup_uses_stored_mode_unless_unattended_or_overridden() {
        use hl2_ui::config::DisplayMode::*;
        assert_eq!(startup_display_mode(Fullscreen, None, false), Fullscreen);
        assert_eq!(startup_display_mode(NoBorder, None, false), NoBorder);
        assert_eq!(startup_display_mode(Fullscreen, None, true), Windowed);
        assert_eq!(
            startup_display_mode(Windowed, Some(NoBorder), true),
            NoBorder
        );
        assert_eq!(
            startup_display_mode(Fullscreen, Some(Windowed), false),
            Windowed
        );
    }
    #[test]
    fn a_bad_fake_monitor_fails_the_launch() {
        for spec in ["8k", "320x240", "1920x1080,3840x2160"] {
            let args = ["--fake-monitor".to_owned(), spec.to_owned()];
            let error = Options::parse_from(args, Default::default())
                .err()
                .expect(spec)
                .to_string();
            assert!(error.contains("--fake-monitor"), "{error}");
        }
    }
    #[test]
    fn startup_parses_background_map_override() {
        let args = ["--background-map".to_owned(), "background02".to_owned()];
        let (parsed, installation) = Options::parse_from(args, Default::default())
            .expect("parse options with background map");
        assert!(installation.explicit.is_none());
        assert!(parsed.game.as_os_str().is_empty());
        assert_eq!(parsed.background_map.as_deref(), Some("background02"));
    }
    #[test]
    #[ignore = "requires an owned Half-Life 2 installation"]
    fn chapter_background_levels_run_live_behind_the_menu() {
        let root = source_assets::install::discover().expect("owned install");
        for game in ["hl2", "episodic", "ep2", "lostcoast"] {
            let vfs = source_assets::vpk::Vfs::mount_game(&root, game).expect("mount");
            let text = vfs
                .read("scripts/chapterbackgrounds.txt")
                .unwrap()
                .and_then(|b| source_assets::keyvalues::decode_text(&b).ok())
                .expect("chapterbackgrounds.txt");
            let menu_background = hl2_ui::gameui::resources::background_map(&text);
            for map in hl2_ui::gameui::resources::chapter_background_maps(&text) {
                let loaded = crate::assets::load(&root, game, &map)
                    .unwrap_or_else(|e| panic!("{game}/{map}: {e:#}"));
                let mut game_state = loaded.gameplay;
                let mut ui = crate::ui_host::Console::new(Default::default());
                ui.source.level_loaded(true);
                let (spawn, yaw) = loaded.world.spawn();
                let mut sim =
                    simulation::Simulation::new(&loaded.world, spawn, yaw, 0., false, vec![]);
                sim.paused = ui.source.paused();
                // Ten seconds of 15 ms ticks.
                for _ in 0..667 {
                    sim.step(Some(&mut game_state), Some(&mut ui));
                }
                let context = format!("{game}/{map}");
                assert!(game_state.scene.time > 5., "{context}: world did not run");
                assert!(
                    game_state.scene.transition.is_none(),
                    "{context}: level change"
                );
                assert!(!sim.player.dead, "{context}: player died");
                assert!(!sim.loading(), "{context}: started loading");
                // The view is the map's point_viewcontrol, enabled by its logic_auto.
                let camera = loaded
                    .world
                    .entities
                    .iter()
                    .find(|e| e.class() == "point_viewcontrol")
                    .and_then(|e| e.get("origin"))
                    .map(|o| {
                        let v: Vec<f32> = o
                            .split_whitespace()
                            .filter_map(|n| n.parse().ok())
                            .collect();
                        glam::Vec3::new(v[0], v[1], v[2])
                    })
                    .unwrap_or_else(|| panic!("{context}: no point_viewcontrol"));
                let view = sim
                    .view_control
                    .unwrap_or_else(|| panic!("{context}: no view control"));
                assert!(view.origin.distance(camera) < 0.01, "{context}: {view:?}");
                assert!(view.freeze_player, "{context}");
                let zoomed = loaded
                    .world
                    .entities
                    .iter()
                    .any(|e| e.class() == "env_zoom");
                assert_eq!(sim.zoom_fov, zoomed.then_some(75.), "{context}");
                println!(
                    "{context}: view {:?} pitch {:.2} yaw {:.2} fov {:?}",
                    view.origin, view.pitch, view.yaw, sim.zoom_fov
                );
                // Ambience: the soundscape picked at the player's position, and ambient_generic
                // loops that play from spawn (SDK CAmbientGeneric, neither flag 16 nor 32).
                let ambient_loops = loaded
                    .world
                    .entities
                    .iter()
                    .filter(|e| e.class() == "ambient_generic")
                    .filter(|e| {
                        e.get("spawnflags")
                            .and_then(|f| f.parse::<u32>().ok())
                            .is_some_and(|f| f & (16 | 32) == 0)
                    })
                    .count();
                let soundscape = game_state.scene.soundscape.params.is_some();
                println!("{context}: soundscape {soundscape}, spawn ambient loops {ambient_loops}");
                // The background each game's menu shows has ambience at the spawn; later
                // chapters may not (ep1_background02's only soundscape is out of range there).
                if menu_background.as_deref() == Some(map.as_str()) {
                    assert!(soundscape || ambient_loops > 0, "{context}: no ambience");
                }
            }
        }
    }

    #[test]
    #[ignore = "requires an owned Half-Life 2 installation"]
    fn background01_loads_from_the_named_game() {
        let root = source_assets::install::discover().expect("owned install");
        let hl2 = crate::assets::load(&root, "hl2", "background01").expect("hl2 background01");
        let lostcoast = crate::assets::load(&root, "lostcoast", "background01")
            .expect("lostcoast background01");
        assert_eq!(
            (hl2.game.as_str(), lostcoast.game.as_str()),
            ("hl2", "lostcoast")
        );
        // Both are called background01; their worlds differ.
        let materials = |loaded: &crate::assets::LoadedMap| {
            loaded
                .materials
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
        };
        assert_ne!(materials(&hl2), materials(&lostcoast));
    }
    #[test]
    #[ignore]
    fn episode_maps_load_successfully() {
        let game = source_assets::install::discover().expect("owned install");
        for (map_game, map) in [
            ("episodic", "ep1_citadel_00"),
            ("ep2", "ep2_outland_01"),
            ("lostcoast", "d2_lostcoast"),
        ] {
            let loaded = crate::assets::load(&game, map_game, map)
                .unwrap_or_else(|err| panic!("Failed to load {}: {:?}", map, err));
            assert_eq!(loaded.world.name, map);
            assert_eq!(loaded.gameplay.world.name, map);

            // Observable completion signals
            let campaign = crate::campaign::Campaign::new(&loaded);
            assert!(campaign.is_gameplay_ready());
            assert_eq!(
                campaign.loaded_map,
                Some(hl2_ui::console::GameMap::new(map_game, map))
            );

            // Verify simulation transition to gameplay
            let (spawn, spawn_yaw) = loaded.world.spawn();
            let mut sim =
                simulation::Simulation::new(&loaded.world, spawn, spawn_yaw, 0., false, vec![]);
            sim.change_map(&loaded.world, spawn, true);
            assert!(!sim.loading());
            assert!(!sim.player.dead);

            // Verify Console transitions to gameplay state on direct level load
            let mut console = hl2_ui::console::Console::default();
            console.start_loading(map);
            assert!(console.is_loading());
            console.level_loaded(false);
            assert!(!console.is_loading());
            assert_eq!(console.mode(), hl2_ui::console::Mode::Gameplay);
        }
    }
}
