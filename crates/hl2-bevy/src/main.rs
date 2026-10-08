//! Isolated Bevy/wgpu host reusing engine-independent Source simulation.
mod assets;
mod audio;
mod bloom;
mod campaign;
mod console;
mod details;
mod effects;
mod eyes;
mod gameplay;
mod gpu_skinning;
mod hud;
mod material_proxies;
mod monitors;
mod movement;
mod performance;
mod rendering;
mod sky;
mod tonemap;
mod visibility;
use anyhow::{Context, Result, bail};
use bevy::{
    app::AppExit,
    asset::AssetPlugin,
    core_pipeline::tonemapping::Tonemapping,
    prelude::*,
    render::{
        renderer::RenderAdapterInfo,
        view::screenshot::{Screenshot, ScreenshotCaptured},
    },
    window::{MonitorSelection, WindowMode, WindowResolution},
};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Clone, Resource)]
struct Options {
    game: PathBuf,
    map: String,
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
    borderless: bool,
    fly: bool,
    movement_script: Option<PathBuf>,
    /// Master volume, like Source's `volume` convar (0..1).
    volume: f32,
    /// JSON-lines sound lifecycle trace (requests, sink starts, removals).
    audio_trace: Option<PathBuf>,
    /// Do not take focus at window creation, so unattended tests leave the desktop usable.
    no_focus: bool,
    /// Fixed HDR tonemap scale (Source mat_force_tonemap_scale); None = auto exposure.
    tonemap_scale: Option<f32>,
}
impl Options {
    fn parse() -> Result<Self> {
        let mut args = std::env::args().skip(1);
        let mut game = None;
        let mut options = Self {
            game: PathBuf::new(),
            map: "d1_trainstation_02".into(),
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
            width: 1280,
            height: 720,
            borderless: false,
            fly: false,
            movement_script: None,
            volume: 1.,
            audio_trace: None,
            no_focus: false,
            tonemap_scale: None,
        };
        while let Some(arg) = args.next() {
            let next = |args: &mut std::iter::Skip<std::env::Args>| -> Result<String> {
                args.next()
                    .with_context(|| format!("missing value after {arg}"))
            };
            match arg.as_str() {
                "--game" => game = Some(PathBuf::from(next(&mut args)?)),
                "--map" => options.map = next(&mut args)?,
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
                "--borderless" => options.borderless = true,
                "--fly" => options.fly = true,
                "--movement-script" => options.movement_script = Some(next(&mut args)?.into()),
                "--volume" => options.volume = next(&mut args)?.parse()?,
                "--audio-trace" => options.audio_trace = Some(next(&mut args)?.into()),
                "--mute-ambient" => {
                    audio::MUTE_AMBIENT.store(true, std::sync::atomic::Ordering::Relaxed)
                }
                "--no-focus" => options.no_focus = true,
                "--tonemap-scale" => {
                    let scale: f32 = next(&mut args)?.parse()?;
                    if !(scale.is_finite() && scale > 0.) {
                        bail!("--tonemap-scale must be positive");
                    }
                    options.tonemap_scale = Some(scale);
                }
                "--help" | "-h" => {
                    println!(
                        "HL2-RS Bevy migration preview (campaign incomplete).\n--game PATH --map NAME --borderless --width N --height N\n--position X Y Z --yaw DEGREES --pitch DEGREES\n--frames N --capture PNG --capture-burst N --capture-monitor PNG --report JSON\n--fly --movement-script JSON --profile --uncapped --cpu-skinning --no-pvs --world-partition\n--volume 0..1 --audio-trace JSONL --mute-ambient --no-focus (start unfocused for unattended tests) --tonemap-scale S (fixed HDR exposure)\nClick to capture mouse; WASD move, Space jump, Ctrl crouch, Shift sprint, Alt walk. F1 toggles developer diagnostics/FPS; F2 toggles fly. F3 gives weapons; slots/wheel select; mouse buttons fire/confirm; R reloads; Q last weapon; E uses. Esc cancels selection then opens the pause menu; select Resume to continue. Tilde toggles the console. F10 quits."
                    );
                    std::process::exit(0);
                }
                _ => bail!("unknown option {arg}; use --help"),
            }
        }
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
        options.game = match game {
            Some(path) => source_assets::install::validate(path)?,
            None => source_assets::install::discover()?,
        };
        for path in [
            Some(&options.report),
            options.capture.as_ref(),
            options.monitor_capture.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
        }
        Ok(options)
    }
}
#[derive(Resource)]
struct PreparedMap(Option<assets::LoadedMap>);
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
/// Source right-handed Z-up coordinates become Bevy right-handed Y-up.
fn source_to_bevy(p: Vec3) -> Vec3 {
    Vec3::new(p.x, p.z, -p.y)
}
fn bevy_to_source(p: Vec3) -> Vec3 {
    Vec3::new(p.x, -p.z, p.y)
}
fn source_direction(yaw: f32, pitch: f32) -> Vec3 {
    Vec3::new(
        pitch.cos() * yaw.cos(),
        pitch.cos() * yaw.sin(),
        pitch.sin(),
    )
}

fn main() -> Result<()> {
    let options = Options::parse()?;
    println!(
        "Loading owned {} for Bevy/wgpu migration host...",
        options.map
    );
    let loaded = assets::load(&options.game, &options.map)?;
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
        .map(|p| movement::read_script(p))
        .transpose()?
        .unwrap_or_default();
    let mut simulation = movement::Simulation::new(
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
    simulation.run_after_script = options.capture_live;
    let status = Status::default();
    let packaged_assets = std::env::current_exe()?
        .parent()
        .context("executable has no directory")?
        .join("bevy-assets");
    let asset_root = if packaged_assets.join("shaders/source.wgsl").is_file() {
        packaged_assets
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets")
    };
    let mut app = App::new();
    if let Some(path) = &options.audio_trace {
        app.insert_resource(audio::AudioTrace::create(path)?);
    }
    app.insert_resource(options.clone())
        .insert_resource(tonemap::Tonemap::new(options.tonemap_scale))
        .insert_resource(status.clone())
        .insert_resource(PreparedMap(Some(loaded)))
        .insert_resource(simulation)
        .init_resource::<campaign::Campaign>()
        .init_resource::<CaptureControl>()
        .insert_resource(ClearColor(Color::srgb(0.08, 0.09, 0.1)))
        .add_plugins(
            DefaultPlugins
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
                        focused: !options.no_focus,
                        present_mode: if options.uncapped {
                            bevy::window::PresentMode::AutoNoVsync
                        } else {
                            bevy::window::PresentMode::AutoVsync
                        },
                        title: "HL2-RS | Bevy/wgpu migration | gameplay migration preview".into(),
                        resolution: WindowResolution::new(options.width, options.height)
                            .with_scale_factor_override(1.),
                        mode: if options.borderless {
                            WindowMode::BorderlessFullscreen(MonitorSelection::Primary)
                        } else {
                            WindowMode::Windowed
                        },
                        ..default()
                    }),
                    ..default()
                }),
        )
        .add_plugins(MaterialPlugin::<rendering::SourceMaterial>::default())
        .add_plugins(MaterialPlugin::<effects::EffectMaterial>::default())
        .add_plugins(MaterialPlugin::<details::DetailMaterial>::default())
        .add_systems(Update, details::spawn_pending)
        .add_plugins(bloom::SourceBloomPlugin)
        .add_plugins(bevy::sprite_render::Material2dPlugin::<hud::HudMaterial>::default())
        .add_plugins(movement::MovementPlugin)
        .add_systems(
            RunFixedMainLoop,
            campaign::poll
                .in_set(bevy::app::RunFixedMainLoopSystems::AfterFixedMainLoop)
                .before(movement::present),
        )
        .add_systems(
            PostUpdate,
            (
                rendering::present_entities,
                rendering::present_weapons,
                effects::present,
                sky::present,
                monitors::present,
                monitors::present_materials,
                material_proxies::present,
                hud::present,
            )
                .before(TransformSystems::Propagate),
        )
        .add_systems(
            PostUpdate,
            rendering::present_lighting
                .after(rendering::present_entities)
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
            tonemap::update.before(TransformSystems::Propagate),
        )
        .add_systems(
            PostUpdate,
            audio::queue
                .after(hud::present)
                .before(TransformSystems::Propagate),
        )
        .add_systems(Last, effects::observe.before(monitor))
        .add_systems(Last, audio::observe.before(monitor))
        .add_systems(Last, monitor);
    if options.profile {
        performance::install(&mut app);
    }
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
        "runtime": "Bevy 0.19.1 / wgpu gameplay migration preview", "map": options.map, "bsp_revision": revision,
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
type StartupAssets<'w> = (
    ResMut<'w, Assets<Mesh>>,
    ResMut<'w, Assets<rendering::SourceMaterial>>,
    ResMut<'w, Assets<Image>>,
    ResMut<'w, Assets<AudioSource>>,
    ResMut<'w, Assets<effects::EffectMaterial>>,
    ResMut<'w, Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>,
);
fn setup(
    mut commands: Commands,
    mut prepared: ResMut<PreparedMap>,
    options: Res<Options>,
    status: Res<Status>,
    (mut meshes, mut materials, mut images, mut sounds, mut effect_materials, mut inverse_binds): StartupAssets,
) {
    let loaded = prepared.0.take().expect("startup map consumed once");
    commands.insert_resource(campaign::Campaign::new(&loaded));
    let (spawn, yaw) = loaded.world.spawn();
    let position = options
        .position
        .unwrap_or_else(|| Vec3::from_array(spawn.to_array()));
    let yaw = options.yaw.unwrap_or(yaw);
    let ui = install_map(
        loaded,
        &mut commands,
        (
            &mut meshes,
            &mut materials,
            &mut images,
            &mut sounds,
            &mut effect_materials,
            &mut inverse_binds,
        ),
        &status,
        (
            options.cpu_skinning,
            options.no_pvs,
            options.world_partition,
        ),
    );
    commands.insert_resource(console::Console::new(ui));
    commands.spawn((
        Camera2d,
        Camera {
            order: 2,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        Tonemapping::None,
        Msaa::Off,
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
        Tonemapping::None,
        bevy::camera::Exposure::default(),
        tonemap::ToneMapped,
        // The viewmodel camera draws last in the 3D view, so bloom covers the whole scene.
        bloom::SourceBloom {
            amount: 1.,
            presample: None,
        },
        Msaa::Off,
        Projection::Perspective(PerspectiveProjection {
            fov: 2. * ((54f32.to_radians() / 2.).tan() / (4. / 3.)).atan(),
            near: 0.1,
            far: 1000.,
            ..default()
        }),
        Transform::IDENTITY.looking_to(Vec3::X, Vec3::Y),
    ));
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        Msaa::Off,
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
pub(crate) type MapAssets<'a> = (
    &'a mut Assets<Mesh>,
    &'a mut Assets<rendering::SourceMaterial>,
    &'a mut Assets<Image>,
    &'a mut Assets<AudioSource>,
    &'a mut Assets<effects::EffectMaterial>,
    &'a mut Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>,
);
pub(crate) fn install_map(
    loaded: assets::LoadedMap,
    commands: &mut Commands,
    (meshes, materials, images, sounds, effect_materials, inverse_binds): MapAssets,
    status: &Status,
    (cpu_skinning, no_pvs, world_partition): (bool, bool, bool),
) -> hl2_ui::console::Console {
    {
        let mut stats = status.0.lock().expect("map stats");
        stats.meshes = 0;
        stats.partitioned_world_batches = 0;
        stats.gpu_skinned_meshes = 0;
        stats.cpu_skinned_meshes = 0;
        stats.skin_joints = 0;
        stats.triangles = 0;
        stats.materials = 0;
        stats.skipped_background_surfaces = 0;
    }
    commands.insert_resource(visibility::SourceVisibility::new(
        loaded.bsp.visibility_index(),
        no_pvs,
    ));
    commands.insert_resource(rendering::FlexModels(loaded.flexes.clone()));
    let camera_target = monitors::install(&loaded, commands, images);
    rendering::spawn_map(
        &loaded,
        commands,
        (meshes, inverse_binds),
        materials,
        images,
        status,
        (&camera_target, cpu_skinning, world_partition),
    );
    effects::install(
        &loaded,
        commands,
        meshes,
        materials,
        effect_materials,
        images,
    );
    commands.insert_resource(eyes::Eyes::new(&loaded));
    sky::install(&loaded, commands, meshes, materials, images);
    commands.insert_resource(sky::Sky::new(
        loaded.bsp,
        loaded.world.background_camera.clone(),
        loaded.sky.as_ref(),
    ));
    commands.insert_resource(details::PendingDetails(loaded.details));
    effects::adopt(loaded.effects.sprites, commands);
    commands.insert_resource(loaded.gameplay);
    commands.insert_resource(hud::Hud::new(loaded.hud));
    audio::install(commands, sounds, loaded.audio);
    loaded.console
}
type EntityDrawQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static rendering::SourceEntity,
        &'static Transform,
        &'static Visibility,
        Option<&'static visibility::PvsDraw>,
    ),
    With<Mesh3d>,
>;
type HostResources<'w> = (
    Res<'w, Options>,
    Res<'w, movement::Simulation>,
    Res<'w, gameplay::Gameplay>,
    Res<'w, hud::Hud>,
    Res<'w, audio::Audio>,
    Res<'w, sky::Sky>,
    Res<'w, monitors::Monitors>,
    Res<'w, eyes::Eyes>,
    Res<'w, effects::Effects>,
    Res<'w, console::Console>,
    Res<'w, campaign::Campaign>,
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
fn monitor(
    mut commands: Commands,
    (options, simulation, game, hud, audio, sky, monitors, eyes, effects, console, campaign): HostResources,
    mut control: ResMut<CaptureControl>,
    status: Res<Status>,
    (adapter, pvs, tonemap, source_materials): RenderResources,
    (cameras, draws, map_entities, all_cameras, meshes, images, geometry, two_textures): DiagnosticQueries,
    (mut exit, performance, render_diagnostics, detail_report): (
        MessageWriter<AppExit>,
        Option<Res<performance::Performance>>,
        Option<Res<bevy::diagnostic::DiagnosticsStore>>,
        Option<Res<details::DetailReport>>,
    ),
) {
    let _timing = performance::scope(performance.as_deref(), "diagnostics");
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
        for (owner, transform, visibility, pvs) in &draws {
            owned_meshes += 1;
            let state = &game.scene.states[owner.0];
            let (origin, rotation) = simulation
                .physics
                .entity_pose(owner.0)
                .unwrap_or((state.origin, state.rotation));
            let expected = rendering::entity_transform(origin, rotation);
            let hidden = !state.visible || state.killed || pvs.is_some_and(|pvs| pvs.culled);
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
        report.presentation = serde_json::json!({"source_visibility":pvs.report(),"owned_meshes":owned_meshes,"pose_or_visibility_mismatches":mismatches,"station_entrance_draws":doors,"hud":hud.report(),"audio":audio.report(),"sky":sky.report(),"monitors":monitors.report(&game),"eyes":eyes.report(),"effects":effects.report(),"details":detail_report.as_deref().cloned(),"console":console.report(),"campaign":campaign.report(),"tonemap":tonemap.report(),
        "lifecycle":{"map_entities":map_entities.iter().count(),"cameras":all_cameras.iter().count(),"live_mesh_assets":meshes.len(),"live_image_assets":images.len()}});
        report.presentation["two_texture_materials"] = serde_json::json!(
            two_textures
                .iter()
                .filter_map(|(name, draw, handle)| {
                    let material = source_materials.get(&handle.0)?;
                    Some(
                        serde_json::json!({"name":name.as_str(),"entity":draw.entity,
                "base_frames":draw.base.len(),"second_frames":draw.second.len(),
                "base_frame":draw.base.iter().position(|h|h==&material.base),
                "second_frame":draw.second.iter().position(|h|h==&material.iris),
                "secondary_uv":material.secondary_uv.to_cols_array(),"time":game.scene.time}),
                    )
                })
                .collect::<Vec<_>>()
        );
        let mut visible_meshes = 0;
        let mut visible_triangles = 0;
        let mut total_triangles = 0;
        for (visible, triangles) in &geometry {
            total_triangles += triangles.0;
            if visible.get() {
                visible_meshes += 1;
                visible_triangles += triangles.0;
            }
        }
        report.presentation["render_candidates"] = serde_json::json!({
            "tagged_meshes":geometry.iter().count(),"tagged_triangles":total_triangles,
            "visible_any_view_meshes":visible_meshes,"visible_any_view_triangles":visible_triangles,
            "scope":"world/entity candidates after PVS/frustum/layer checks; excludes sky and viewmodels; union of all views, not GPU draw-call counts"
        });
        if let Some(performance) = performance.as_deref() {
            report.presentation["cpu_stages"] = performance.report();
            if let Some(diagnostics) = render_diagnostics.as_deref() {
                report.presentation["render_diagnostics"] = performance::render_report(diagnostics);
            }
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
                commands
                    .spawn(Screenshot::primary_window())
                    .observe(capture_complete);
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
                    .spawn(Screenshot::primary_window())
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
fn capture_complete(event: On<ScreenshotCaptured>, status: Res<Status>) {
    let mut report = status.0.lock().expect("status lock");
    report.capture_completed = true;
    report.capture_size = Some([event.image.width(), event.image.height()]);
    report.captured_image = Some(event.image.clone());
}
fn monitor_capture_complete(event: On<ScreenshotCaptured>, status: Res<Status>) {
    let mut report = status.0.lock().expect("monitor capture");
    report.monitor_capture_completed = true;
    report.monitor_image = Some(event.image.clone());
}
fn save_capture(image: Option<Image>, path: Option<&PathBuf>) -> Option<String> {
    let path = path?;
    match image {
        Some(image) => match image.try_into_dynamic() {
            Ok(image) => image
                .to_rgb8()
                .save(path)
                .err()
                .map(|error| error.to_string()),
            Err(error) => Some(format!("screenshot conversion: {error}")),
        },
        None => Some("screenshot readback did not finish".into()),
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
}
