//! Installation/output side effects after argument parsing has succeeded.
mod app;
use super::Options;
use anyhow::{Context, Result, bail};
pub(super) use app::make_app;
use std::{path::PathBuf, sync::Arc};

/// Rejects map-dependent arguments when no `--map` was given.
pub(crate) fn require_map_for(map: Option<&str>, map_dependent: &[&str]) -> Result<()> {
    if map.is_none() && !map_dependent.is_empty() {
        bail!(
            "--map is required by {}: they apply to a directly loaded map, and without --map the main menu opens",
            map_dependent.join(", ")
        );
    }
    Ok(())
}

/// The window mode to start in: an explicit `--windowed`/`--fullscreen`/`--borderless` wins;
/// otherwise unattended runs start windowed and interactive runs use the stored Video setting.
pub(crate) fn startup_display_mode(
    stored: hl2_ui::config::DisplayMode,
    explicit: Option<hl2_ui::config::DisplayMode>,
    unattended: bool,
) -> hl2_ui::config::DisplayMode {
    explicit.unwrap_or(if unattended {
        hl2_ui::config::DisplayMode::Windowed
    } else {
        stored
    })
}

/// Resolved target map and mode for initial application startup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StartupTarget {
    pub game: String,
    pub map: String,
    pub background: bool,
}

/// Resolves the startup map target without loading map geometry or heavy assets.
pub(crate) fn target_map(options: &crate::Options) -> Result<StartupTarget> {
    let base_game = source_assets::install::BASE_GAME;
    let config = hl2_ui::config::Config::load_or_default();
    if let Some(map) = options.map.as_ref() {
        let is_bg = options.main_menu;
        let game = source_assets::install::game_for_map(&options.game, map, None)
            .unwrap_or_else(|| base_game.to_owned());
        return Ok(StartupTarget {
            game,
            map: map.clone(),
            background: is_bg,
        });
    }
    let vfs =
        source_assets::vpk::Vfs::mount(&options.game).context("mount installed owned content")?;
    let default_background = hl2_ui::gameui::resources::startup_background_map(&vfs);
    let (game, map) = match (&options.background_map, &config.gameplay.background_map) {
        (Some(map), _) => (
            source_assets::install::game_for_map(&options.game, map, None)
                .unwrap_or_else(|| base_game.to_owned()),
            map.clone(),
        ),
        (None, Some(map)) => (
            config
                .gameplay
                .background_game
                .clone()
                .or_else(|| source_assets::install::game_for_map(&options.game, map, None))
                .unwrap_or_else(|| base_game.to_owned()),
            map.clone(),
        ),
        (None, None) => (
            base_game.to_owned(),
            default_background.context(
                "no main-menu background map in scripts/chapterbackgrounds.txt or config; pass a loadable --map",
            )?,
        ),
    };
    Ok(StartupTarget {
        game,
        map,
        background: true,
    })
}

/// Creates a lightweight stub [`crate::assets::LoadedMap`] for instant window presentation
/// before asynchronous map loading begins in the worker thread.
pub(crate) fn load_stub(root: &std::path::Path, game: &str) -> Result<crate::assets::LoadedMap> {
    let vfs = Arc::new(
        source_assets::vpk::Vfs::mount_game(root, game)
            .context("mount installed owned content for startup stub")?,
    );
    let canvas = source_ui::canvas::Canvas::default();
    let hud = hl2_ui::hud::WeaponHud::load(&vfs, canvas)?;
    let world = Arc::new(modkit_core::World::default());
    let gameplay = crate::gameplay::Gameplay::synthetic(modkit_core::World::default());
    let audio = crate::audio::PreparedAudio::silent();
    let effects = crate::effects::PreparedEffects::load(&vfs, &world);
    let bsp = source_assets::bsp::Bsp::empty();
    let shadow_receivers = Arc::new(hl2_simulation::shadows::receivers::Receivers::new(
        &[],
        |_| true,
    ));
    Ok(crate::assets::LoadedMap {
        load_timings: Default::default(),
        game: game.to_owned(),
        world,
        gameplay,
        hud,
        audio,
        pending_audio: None,
        sky: None,
        effects,
        gaze: std::collections::BTreeMap::new(),
        flexes: std::collections::BTreeMap::new(),
        eyes: std::collections::BTreeMap::new(),
        bsp,
        revision: 0,
        materials: std::collections::BTreeMap::new(),
        flashlight_cookie: None,
        shadow_receivers,
        texture_errors: Vec::new(),
        models: serde_json::Value::Null,
        vfs,
    })
}

pub(super) struct InstallationRequest {
    pub explicit: Option<PathBuf>,
    pub force: bool,
    pub noninteractive: bool,
    pub preview: Option<PathBuf>,
}

impl Options {
    pub(super) fn finish_startup(mut self, installation: InstallationRequest) -> Result<Self> {
        self.game = match crate::setup::resolve(
            installation.explicit,
            self.map.as_deref(),
            installation.force,
            installation.noninteractive,
            installation.preview,
        )? {
            Some(path) => path,
            None => std::process::exit(0), // Setup cancelled; no game was loaded.
        };
        for path in [
            Some(&self.report),
            self.capture.as_ref(),
            self.monitor_capture.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
        }
        Ok(self)
    }
}

const HELP_LINES: &[&str] = &[
    "Usage: hl2-bevy [OPTIONS]\n",
    "Options:",
    "      --ai                            Enable NPC AI think loop (default: enabled)",
    "      --audio-trace <path.jsonl>      Record audio cue and soundscape events to a JSONL trace file",
    "      --background                    Run headlessly without opening an OS window (for background replay / test runs)",
    "      --background-map <name>         Override the 3D background map loaded behind the main menu (default: background01)",
    "      --borderless                    Start in borderless windowed mode (native full-screen Space on macOS)",
    "      --capture <path.png>            Save screenshot of the final rendered frame to a PNG file",
    "      --capture-burst <count>         Capture a sequence of N frames to PNG files (requires --map)",
    "      --capture-live                  Capture frames live during interactive execution (requires --map)",
    "      --capture-monitor <path.png>    Capture an in-game camera monitor texture to a PNG file (requires --map)",
    "      --capture-window                Capture the presented OS window buffer rather than the internal game viewport",
    "      --cpu-skinning                  Use CPU vertex skinning instead of GPU compute skinning",
    "      --fake-monitor <spec>           Simulate a monitor with specified modes (e.g. 1920x1080,1280x720)",
    "      --fly                           Start with flight/noclip mode enabled (requires --map)",
    "      --frag-swept-box                Use swept-box physics query for frag grenade collision",
    "      --frames <count>                Exit automatically after rendering N frames (30..=36000)",
    "      --fullscreen                    Start in exclusive fullscreen display mode",
    "      --game <path>                   Path to Half-Life 2 installation (overrides saved preference and HL2_ROOT)",
    "      --height <pixels>               Window height in pixels (240..=8192, default from video config)",
    "  -h, --help                          Show this help message and exit",
    "      --main-menu                     Open the main menu (overlaid on top of --map if specified)",
    "      --map <name>                    Map to load directly into gameplay (e.g. d1_trainstation_01)",
    "      --max-edicts <count>            Runtime entity limit (default: 2048, 0 for unlimited)",
    "      --movement-script <path.json>   Replay recorded player inputs from a movement script fixture (requires --map)",
    "      --mute                          Silence all audio output for this run (Audio settings unchanged)",
    "      --mute-ambient                  Mute looping ambient soundscapes and background environment audio",
    "      --no-ai                         Disable NPC AI think loop",
    "      --no-focus                      Launch window without requesting input focus (for unattended test runs)",
    "      --novid, -novid                 Skip startup videos on launch",
    "      --no-pvs                        Disable BSP potential visible set (PVS) occlusion culling",
    "      --no-setup                      Fail with an error instead of opening setup if installation is missing",
    "      --pitch <degrees>               Set initial camera pitch angle in degrees (requires --map)",
    "      --position <x> <y> <z>          Set initial camera spawn coordinates (requires --map)",
    "      --profile                       Record CPU stage execution timings in the run report",
    "      --report <path.json>            Path for simulation report output (default: artifacts/bevy-report.json)",
    "      --setup                         Open the setup dialog to choose/remember the Half-Life 2 folder",
    "      --setup-preview <path.png>      Capture a screenshot of the setup dialog and exit without loading a map",
    "      --tonemap-scale <scale>         Override HDR tonemapping exposure multiplier (positive float)",
    "      --uncapped                      Disable VSync to render and present frames unconstrained",
    "      --volume <0.0..1.0>             Set master audio volume level (default from audio config)",
    "      --width <pixels>                Window width in pixels (320..=8192, default from video config)",
    "      --window-at <x> <y>             Initial window position on desktop in pixels",
    "      --windowed                      Start in windowed display mode",
    "      --world-partition               Enable spatial world partition culling",
    "      --yaw <degrees>                 Set initial camera yaw angle in degrees (requires --map)",
];

/// Resolves the asset root for Bevy AssetPlugin: prefers packaged `bevy-assets` beside executable if it has shaders,
/// otherwise falls back to compile-time `assets` directory.
pub(crate) fn resolve_asset_root() -> Result<PathBuf> {
    let packaged_assets = std::env::current_exe()?
        .parent()
        .context("executable has no directory")?
        .join("bevy-assets");
    if packaged_assets.join("shaders/source.wgsl").is_file() {
        Ok(packaged_assets)
    } else {
        Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets"))
    }
}

pub(super) fn print_help() {
    for line in HELP_LINES {
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing_defers_installation_resolution_and_setup_requests() {
        let args = [
            "--game",
            "not-an-install",
            "--setup",
            "--report",
            "output/report.json",
        ];
        let (options, request) = Options::parse_from(args.map(str::to_owned), Default::default())
            .expect("parsing must not inspect or remember an installation or open setup");
        assert!(options.game.as_os_str().is_empty());
        assert_eq!(request.explicit, Some(PathBuf::from("not-an-install")));
        assert!(request.force);
        assert!(!request.noninteractive);
        assert_eq!(options.report, PathBuf::from("output/report.json"));
    }

    #[test]
    fn parsing_uses_supplied_settings_and_preserves_noninteractive_setup_policy() {
        let mut config = hl2_ui::config::Config::default();
        config.video.width = 1234;
        config.video.height = 777;
        for flag in ["--no-setup", "--background", "--no-focus", "--capture"] {
            let mut args = vec![flag.to_owned()];
            if flag == "--capture" {
                args.push("capture.png".into());
            }
            let (options, request) = Options::parse_from(args, config.clone()).unwrap();
            assert_eq!((options.width, options.height), (1234, 777));
            assert!(request.noninteractive, "{flag}");
            assert!(!request.force);
        }
        let args = ["--setup", "--no-setup"].map(str::to_owned);
        assert!(Options::parse_from(args, config).is_err());
    }

    #[test]
    fn target_map_distinguishes_gameplay_maps_and_backgrounds() {
        let (gameplay_opt, _) = Options::parse_from(
            ["--map".into(), "d1_trainstation_01".into()],
            Default::default(),
        )
        .unwrap();
        let target = target_map(&gameplay_opt).unwrap();
        assert_eq!(target.map, "d1_trainstation_01");
        assert!(!target.background);

        let (bg_opt, _) =
            Options::parse_from(["--map".into(), "background01".into()], Default::default())
                .unwrap();
        let bg_target = target_map(&bg_opt).unwrap();
        assert_eq!(bg_target.map, "background01");
        assert!(!bg_target.background);

        let (menu_opt, _) = Options::parse_from(
            [
                "--map".into(),
                "d1_trainstation_01".into(),
                "--main-menu".into(),
            ],
            Default::default(),
        )
        .unwrap();
        let menu_target = target_map(&menu_opt).unwrap();
        assert_eq!(menu_target.map, "d1_trainstation_01");
        assert!(menu_target.background);
    }

    #[test]
    #[ignore = "requires owned HL2 installation"]
    fn startup_stub_creates_empty_world_and_mounted_vfs() {
        let root = source_assets::install::discover().expect("owned install");
        let stub = load_stub(&root, "hl2").expect("startup stub");
        assert_eq!(stub.game, "hl2");
        assert!(stub.world.surfaces.is_empty());
        assert!(stub.world.entities.is_empty());
        assert!(stub.bsp.lump(0).is_empty());
        assert!(stub.materials.is_empty());
    }
}
