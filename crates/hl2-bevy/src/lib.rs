//! Isolated Bevy/wgpu host reusing engine-independent Source simulation.
mod assets;
mod audio;
pub mod bloom;
pub mod camera;
mod campaign;
pub mod canvas_renderer;
mod capture;
mod console_host;
mod coords;
use capture::{capture_complete, monitor_capture_complete, save_capture};
use {flashlight::FlashlightPlugin, shadows::ShadowPlugin};
mod effects;
pub mod extensions;
mod eyes;
mod flashlight;
pub mod gameplay;
mod gpu_skinning;
pub mod input;
mod launch;
mod map_install;
mod material_proxies;
mod monitor;
mod monitors;
mod options;
pub mod performance;
pub mod rendering;
pub mod runtime_props;
mod setup;
mod shadows;
pub mod simulation;
mod sky;
mod startup;
mod startup_cameras;
mod studio_decals;
mod tonemap;
pub mod ui_host;
mod ui_system;
pub mod video;
mod visibility;
use anyhow::Result;
use bevy::{prelude::*, render::renderer::RenderAdapterInfo};
pub(crate) use map_install::{StartupAssets, install_loaded_map};
use serde::Serialize;
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
pub struct Status(Arc<Mutex<RunStatus>>);
#[derive(Component)]
pub struct FlyCamera;
#[derive(Resource, Default)]
struct CaptureControl {
    frames: u64,
    requested: bool,
    requested_frame: Option<u64>,
    completed_frame: Option<u64>,
}
use coords::{bevy_to_source, source_direction, source_to_bevy};

pub fn run(application: extensions::Application) -> Result<()> {
    if setup::run_helper()? {
        return Ok(());
    }
    let mut config = hl2_ui::config::Config::load_or_default();
    application.ui.prepare_config(&mut config);
    let options = Options::parse(config)?;
    let target = startup::target_map(&options)?;
    let loaded = startup::load_stub(&options.game, &target.game)?;
    let summary = launch::MapSummary::of(&loaded);
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
    simulation.extensions = application.runtime;
    simulation.apply_startup_settings(&options, &target);
    let status = Status::default();
    let asset_root = startup::resolve_asset_root()?;
    let default_plugins = launch::default_plugins(&options, &asset_root);
    let mut app = startup::make_app(options.background, options.profile);
    if let Some(path) = &options.audio_trace {
        app.insert_resource(audio::AudioTrace::create(path)?);
    }
    app.insert_resource(extensions::UiContributions(application.ui))
        .insert_resource(options.clone())
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
        .add_plugins(simulation::MovementPlugin);
    launch::add_video_systems(&mut app);
    launch::add_presentation_systems(&mut app);
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
    for install in application.install {
        install(&mut app);
    }
    let exit = app.run();
    launch::finish(&status, &options, summary, exit)
}
fn setup(
    mut commands: Commands,
    mut prepared: ResMut<PreparedMap>,
    options: Res<Options>,
    status: Res<Status>,
    mut assets: StartupAssets,
    mut present_materials: ResMut<Assets<video::PresentMaterial>>,
    (adapter, contributions): (
        Option<Res<RenderAdapterInfo>>,
        Res<extensions::UiContributions>,
    ),
) {
    let loaded = prepared.0.take().expect("startup map consumed once");
    let target = prepared.1.clone();
    let (spawn, yaw) = loaded.world.spawn();
    let position = options
        .position
        .unwrap_or_else(|| Vec3::from_array(spawn.to_array()));
    let yaw = options.yaw.unwrap_or(yaw);
    let mut ui = hl2_ui::console::Console::load(&loaded.vfs, loaded.hud.canvas.clone());
    contributions.0.install(&mut ui);
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
        video::FrameRequest {
            resolution: UVec2::new(options.width, options.height),
            flexible: options.flexible_aspect,
            windowed: options.display_mode.effective() == hl2_ui::config::DisplayMode::Windowed,
            policy: options.config.video.policy.clone(),
        },
    );
    startup_cameras::spawn(&mut commands, adapter.as_deref(), &options, position, yaw);
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

impl Status {
    pub fn geometry_statistics(&self) -> (usize, usize, u64) {
        let stats = self.0.lock().expect("render statistics");
        (
            stats.triangles,
            stats.materials,
            stats.map_metadata["textures"]["unique_base_textures"]
                .as_u64()
                .unwrap_or(0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::startup::{require_map_for, startup_display_mode};
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
