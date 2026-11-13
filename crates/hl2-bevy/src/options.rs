//! Command-line options: parsing and validation.
use super::{Options, assets, audio, gameplay, startup};
use anyhow::{Context, Result, bail};
use bevy::prelude::Vec3;
use startup::{require_map_for, startup_display_mode};
use std::path::PathBuf;

impl Options {
    pub(super) fn parse(config: hl2_ui::config::Config) -> Result<Self> {
        let (options, installation) = Self::parse_from(std::env::args().skip(1), config)?;
        options.finish_startup(installation)
    }
    pub(super) fn parse_from(
        args: impl IntoIterator<Item = String>,
        config: hl2_ui::config::Config,
    ) -> Result<(Self, startup::InstallationRequest)> {
        let mut args = args.into_iter();
        let mut cli = Cli::default();
        let mut options = Self::defaults(config);
        while let Some(arg) = args.next() {
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
            Self::parse_flag(&mut options, &arg, &mut args, &mut cli)?;
        }
        Self::validated(options, cli)
    }

    fn defaults(config: hl2_ui::config::Config) -> Self {
        Self {
            game: PathBuf::new(),
            map: None,
            map_dependent: Vec::new(),
            frames: None,
            capture: None,
            capture_burst: 0,
            capture_every: None,
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
            follows_window: config.video.frame_follows_window(),
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
        }
    }

    fn parse_flag(
        options: &mut Self,
        arg: &str,
        args: &mut dyn Iterator<Item = String>,
        cli: &mut Cli,
    ) -> Result<()> {
        let mut next = || -> Result<String> {
            args.next()
                .with_context(|| format!("missing value after {arg}"))
        };
        match arg {
            "--game" => cli.game = Some(PathBuf::from(next()?)),
            "--setup" => cli.show_setup = true,
            "--no-setup" => cli.no_setup = true,
            "--setup-preview" => cli.setup_preview = Some(PathBuf::from(next()?)),
            "--map" => options.map = Some(next()?),
            "--frames" => options.frames = Some(next()?.parse()?),
            "--capture" => options.capture = Some(next()?.into()),
            "--capture-burst" => options.capture_burst = next()?.parse()?,
            "--capture-every" => options.capture_every = Some(next()?.parse()?),
            "--capture-live" => options.capture_live = true,
            "--capture-monitor" => options.monitor_capture = Some(next()?.into()),
            "--report" => options.report = next()?.into(),
            "--position" => {
                options.position = Some(Vec3::new(
                    next()?.parse()?,
                    next()?.parse()?,
                    next()?.parse()?,
                ))
            }
            "--yaw" => options.yaw = Some(next()?.parse::<f32>()?.to_radians()),
            "--no-pvs" => options.no_pvs = true,
            "--world-partition" => options.world_partition = true,
            "--cpu-skinning" => options.cpu_skinning = true,
            "--uncapped" => options.uncapped = true,
            "--profile" => options.profile = true,
            "--pitch" => options.pitch = next()?.parse::<f32>()?.to_radians(),
            "--width" => options.width = next()?.parse()?,
            "--height" => options.height = next()?.parse()?,
            "--borderless" => cli.explicit_mode = Some(hl2_ui::config::DisplayMode::NoBorder),
            "--windowed" => cli.explicit_mode = Some(hl2_ui::config::DisplayMode::Windowed),
            "--fullscreen" => cli.explicit_mode = Some(hl2_ui::config::DisplayMode::Fullscreen),
            "--fly" => options.fly = true,
            "--movement-script" => options.movement_script = Some(next()?.into()),
            "--volume" => options.volume = next()?.parse()?,
            "--audio-trace" => options.audio_trace = Some(next()?.into()),
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
                let x = next()?.parse()?;
                let y = next()?.parse()?;
                options.window_at = Some((x, y));
            }
            // Runtime entity limit (Source: MAX_EDICT_BITS 11 = 2048, compiled in);
            // 0 = no limit.
            "--max-edicts" => hl2_simulation::ai::spawn::MAX_EDICTS
                .store(next()?.parse()?, std::sync::atomic::Ordering::Relaxed),
            // Step 14 AI think loop (default on like Source's ai_disabled 0; console ai_enable).
            "--ai" => gameplay::AI_ENABLED.store(true, std::sync::atomic::Ordering::Relaxed),
            "--no-ai" => gameplay::AI_ENABLED.store(false, std::sync::atomic::Ordering::Relaxed),
            "--tonemap-scale" => {
                let scale: f32 = next()?.parse()?;
                if !(scale.is_finite() && scale > 0.) {
                    bail!("--tonemap-scale must be positive");
                }
                options.tonemap_scale = Some(scale);
            }
            "--fake-monitor" => {
                let spec = next()?;
                options.fake_monitor = Some(
                    spec.parse()
                        .with_context(|| format!("--fake-monitor {spec}"))?,
                );
            }
            "--main-menu" => options.main_menu = true,
            "-novid" | "--novid" => options.novid = true,
            "--background-map" => options.background_map = Some(next()?),
            "--help" | "-h" => {
                startup::print_help();
                std::process::exit(0);
            }
            _ => bail!("unknown option {arg}; use --help"),
        }
        Ok(())
    }

    fn validated(mut options: Self, cli: Cli) -> Result<(Self, startup::InstallationRequest)> {
        let Cli {
            game,
            show_setup,
            no_setup,
            setup_preview,
            explicit_mode,
        } = cli;
        require_map_for(options.map.as_deref(), &options.map_dependent)?;
        if !(0. ..=1.).contains(&options.volume) {
            bail!("--volume must be between 0 and 1");
        }
        if !(320..=8192).contains(&options.width) || !(240..=8192).contains(&options.height) {
            bail!("window size is outside 320x240..8192x8192");
        }
        if options.frames.is_some_and(|n| !(30..=200_000).contains(&n)) {
            bail!("--frames must be between 30 and 200000");
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

/// Installation and display-mode requests read from the command line.
#[derive(Default)]
struct Cli {
    game: Option<PathBuf>,
    show_setup: bool,
    no_setup: bool,
    setup_preview: Option<PathBuf>,
    explicit_mode: Option<hl2_ui::config::DisplayMode>,
}
