//! Asset-independent installation bootstrap. Never uses the owned GameUI/menu.
use anyhow::{Context, Result, bail};
use bevy::{
    input_focus::{
        AutoFocus,
        tab_navigation::{TabGroup, TabIndex, TabNavigationPlugin},
    },
    prelude::*,
    text::{EditableText, TextCursorStyle},
    window::WindowResolution,
};
use serde::{Deserialize, Serialize};
use source_assets::install;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize)]
struct Installation {
    game: PathBuf,
}

fn path_text(path: &Path) -> String {
    let text = path.to_string_lossy().into_owned();
    #[cfg(windows)]
    {
        if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{unc}");
        }
        if let Some(local) = text.strip_prefix(r"\\?\") {
            return local.to_owned();
        }
    }
    text
}

fn config_file() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("HL2_RS_CONFIG_DIR") {
        return Ok(PathBuf::from(path).join("installation.json"));
    }
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base =
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")));
    Ok(base
        .context("Cannot find your user settings folder")?
        .join("hl2-rs/installation.json"))
}

fn read_saved(path: &Path) -> Result<Option<PathBuf>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(
            serde_json::from_slice::<Installation>(&bytes)
                .context("The remembered installation file is invalid")?
                .game,
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("Cannot read the remembered installation"),
    }
}

fn save(path: &Path, game: &Path) -> Result<()> {
    std::fs::create_dir_all(path.parent().context("Settings file has no parent")?)?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let result = (|| {
        std::fs::write(
            &temp,
            serde_json::to_vec_pretty(&Installation { game: game.into() })?,
        )?;
        std::fs::rename(&temp, path)?;
        Ok(())
    })();
    let _ = std::fs::remove_file(temp);
    result
}

fn checked(path: PathBuf, map: Option<&str>) -> Result<PathBuf> {
    install::validate_content(path, map)
}

fn select_install(
    explicit: Option<PathBuf>,
    environment: Option<PathBuf>,
    saved: Option<PathBuf>,
    map: Option<&str>,
    discover: impl FnOnce() -> Result<PathBuf>,
) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return checked(path, map).context("The --game folder is not ready");
    }
    if let Some(path) = environment {
        return checked(path, map).context(
            "The HL2_ROOT folder is not ready; update or unset HL2_ROOT if the game moved",
        );
    }
    if let Some(path) = saved {
        match checked(path, map) {
            Ok(path) => return Ok(path),
            Err(error) => {
                return discover().and_then(|p| checked(p, map)).with_context(|| {
                    format!("The remembered game folder is not ready: {error:#}")
                });
            }
        }
    }
    checked(discover()?, map)
}

pub fn resolve(
    explicit: Option<PathBuf>,
    map: Option<&str>,
    force: bool,
    noninteractive: bool,
    preview: Option<PathBuf>,
) -> Result<Option<PathBuf>> {
    let environment = std::env::var_os("HL2_ROOT").map(PathBuf::from);
    let config = config_file();
    let saved = match config.as_ref().ok().map(|p| read_saved(p)).transpose() {
        Ok(path) => path.flatten(),
        Err(error) => {
            eprintln!("Installation settings: {error:#}");
            None
        }
    };
    let initial = explicit
        .clone()
        .or_else(|| environment.clone())
        .or_else(|| saved.clone());
    let remember_automatic = explicit.is_none() && environment.is_none();
    let previous = saved.clone();
    let result = select_install(explicit, environment, saved, map, install::discover_steam);
    if !force {
        match &result {
            Ok(path) => {
                if remember_automatic
                    && previous.as_ref() != Some(path)
                    && let Ok(config) = &config
                    && let Err(error) = save(config, path)
                {
                    eprintln!("Could not remember the detected game folder: {error:#}");
                }
                println!("Using Half-Life 2: {}", path.display());
                return Ok(Some(path.clone()));
            }
            Err(error) if noninteractive => bail!(
                "Installation unavailable: {error:#}. Run with --setup interactively, or pass --game PATH"
            ),
            _ => {}
        }
    }
    let (initial, message) = match result {
        Ok(path) => (
            Some(path),
            "Game found. You can keep this folder or choose another installation.".into(),
        ),
        Err(error) => (initial, format!("{error:#}")),
    };
    // A helper owns the setup event loop; the caller will create the gameplay loop.
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let response = TemporaryResponse(
        std::env::temp_dir().join(format!("hl2-rs-setup-{}-{nonce}.json", std::process::id())),
    );
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&response.0)?;
    let status = std::process::Command::new(std::env::current_exe()?)
        .arg("--setup-window")
        .arg(&response.0)
        // Positional helper argument: empty means no requested map.
        .arg(map.unwrap_or_default())
        .arg(initial.unwrap_or_default())
        .arg(message)
        .arg(preview.unwrap_or_default())
        .status()
        .context("Open installation setup")?;
    if !status.success() {
        bail!(
            "Installation setup exited unexpectedly ({status}); use --game PATH to select the installation"
        );
    }
    let bytes = std::fs::read(&response.0)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    let selected: Installation = serde_json::from_slice(&bytes)?;
    let game = checked(selected.game, map)?;
    if let Err(error) = config.and_then(|path| save(&path, &game)) {
        rfd::MessageDialog::new().set_title("HL2-RS: folder selected")
            .set_description(format!("You can play now, but the folder could not be remembered.\n{error:#}\nNext time, use Browse again or --game PATH."))
            .set_level(rfd::MessageLevel::Warning).show();
    }
    Ok(Some(game))
}

struct TemporaryResponse(PathBuf);
impl Drop for TemporaryResponse {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

enum Work {
    Folder(Option<PathBuf>),
    Detected(Result<PathBuf, String>),
    Validated(Result<PathBuf, String>),
}
#[derive(Resource)]
struct Setup {
    response: PathBuf,
    map: Option<String>,
    initial: String,
    message: String,
    worker: Option<Mutex<mpsc::Receiver<Work>>>,
}

#[derive(Resource)]
struct Preview {
    path: PathBuf,
    frames: u32,
    started: Instant,
    requested: bool,
}

#[derive(Resource, Clone, Default)]
struct PreviewReady(Arc<AtomicBool>);

fn preview_pipelines(
    cache: Res<bevy::render::render_resource::PipelineCache>,
    ready: Res<PreviewReady>,
) {
    ready.0.store(
        cache.waiting_pipelines().next().is_none(),
        Ordering::Relaxed,
    );
}
#[derive(Component)]
struct PathField;
#[derive(Component)]
struct StatusText;
#[derive(Component, Clone, Copy)]
enum Action {
    Browse,
    Detect,
    Play,
    Cancel,
}

/// Internal helper entry point, handled before parsing gameplay arguments.
pub fn run_helper() -> Result<bool> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--setup-window")) {
        return Ok(false);
    }
    let response = PathBuf::from(args.next().context("Missing setup response file")?);
    let map = Some(
        args.next()
            .context("Missing setup map")?
            .to_string_lossy()
            .into_owned(),
    )
    .filter(|map| !map.is_empty());
    let initial = path_text(&PathBuf::from(args.next().unwrap_or_default()));
    let message = args
        .next()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let preview = args
        .next()
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty());
    let mut app = App::new();
    if let Some(path) = preview {
        if path
            .extension()
            .is_none_or(|e| !e.eq_ignore_ascii_case("png"))
        {
            bail!("--setup-preview requires a .png path");
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        app.insert_resource(Preview {
            path,
            frames: 0,
            started: Instant::now(),
            requested: false,
        })
        .add_systems(Update, capture_preview);
    }
    app.insert_resource(Setup {
        response,
        map,
        initial,
        message,
        worker: None,
    })
    .insert_resource(ClearColor(Color::srgb(0.055, 0.065, 0.08)))
    .add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "HL2-RS | Locate Half-Life 2".into(),
                    resolution: WindowResolution::new(960, 700).with_scale_factor_override(1.),
                    resizable: false,
                    ..default()
                }),
                ..default()
            })
            .disable::<bevy::audio::AudioPlugin>(),
    )
    .add_plugins(TabNavigationPlugin)
    .add_systems(Startup, screen)
    .add_systems(Update, (actions, poll_worker, show_status).chain());
    if app.world().contains_resource::<Preview>() {
        use bevy::render::{Render, RenderApp, RenderSystems};
        let ready = PreviewReady::default();
        app.insert_resource(ready.clone());
        app.sub_app_mut(RenderApp)
            .insert_resource(ready)
            .add_systems(Render, preview_pipelines.after(RenderSystems::Cleanup));
    }
    app.run();
    Ok(true)
}

fn capture_preview(
    mut commands: Commands,
    mut preview: ResMut<Preview>,
    nodes: Query<(&ComputedNode, &InheritedVisibility)>,
    cameras: Query<&Camera>,
    ready: Res<PreviewReady>,
) {
    preview.frames += 1;
    if !preview.requested
        && preview.frames >= 90
        && preview.started.elapsed().as_secs_f32() >= 3.
        && ready.0.load(Ordering::Relaxed)
        && nodes
            .iter()
            .any(|(node, visibility)| visibility.get() && !node.is_empty())
    {
        preview.requested = true;
        eprintln!(
            "Setup layout: {:?}; cameras: {:?}",
            nodes
                .iter()
                .map(|(node, visibility)| (node.size(), visibility.get()))
                .collect::<Vec<_>>(),
            cameras
                .iter()
                .map(|camera| camera.physical_viewport_size())
                .collect::<Vec<_>>()
        );
        use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(preview.path.clone()))
            .observe(
                |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                    exit.write(AppExit::Success);
                },
            );
    }
}

fn screen(mut commands: Commands, mut fonts: ResMut<Assets<Font>>, setup: Res<Setup>) {
    let font = fonts.add(Font::from_bytes(source_ui::PROGGY_CLEAN.to_vec()));
    let normal = TextFont {
        font: font.into(),
        font_size: FontSize::Px(20.),
        ..default()
    };
    commands.spawn(Camera2d);
    commands.spawn((Node {
        width: percent(100), height: percent(100), padding: px(32).all(),
        flex_direction: FlexDirection::Column, row_gap: px(14), ..default()
    }, TabGroup::default())).with_children(|parent| {
        parent.spawn((Text::new("HL2-RS / SETUP"), normal.clone(), TextColor(Color::srgb(1., 0.65, 0.25))));
        parent.spawn((Text::new("Locate your Half-Life 2 installation"), normal.clone().with_font_size(32.)));
        parent.spawn((Text::new("HL2-RS reads the game files from your installed Steam copy.\nChoose the Half-Life 2 folder (or its hl2 subfolder)."), normal.clone()));
        parent.spawn((Text::new("GAME FILES FOLDER"), normal.clone()));
        parent.spawn((
            Node { width: percent(100), min_height: px(52), padding: px(12).all(), border: px(1).all(), overflow: Overflow::clip_x(), ..default() },
            BorderColor::all(Color::srgb(0.45, 0.55, 0.65)), BackgroundColor(Color::srgb(0.1, 0.12, 0.15)),
            EditableText::new(&setup.initial), TextCursorStyle::default(), TextLayout::no_wrap(), normal.clone(),
            PathField, AutoFocus, TabIndex(0),
        ));
        parent.spawn((Text::new("Type or paste a path (Ctrl+V / Cmd+V). Press Enter to continue."), normal.clone().with_font_size(18.)));
        parent.spawn(Node { column_gap: px(12), ..default() }).with_children(|row| {
            for (index, (label, action)) in [("Browse...", Action::Browse), ("Retry detection", Action::Detect), ("Continue", Action::Play), ("Cancel", Action::Cancel)].into_iter().enumerate() {
                row.spawn((Button, action, TabIndex(index as i32 + 1), Node {
                    padding: px(12).all(), border: px(1).all(), ..default()
                }, BorderColor::all(Color::srgb(0.35, 0.42, 0.5)), BackgroundColor(Color::srgb(0.16, 0.21, 0.27))))
                    .with_child((Text::new(label), normal.clone()));
            }
        });
        parent.spawn((Text::new(&setup.message), normal.clone().with_font_size(18.), TextColor(Color::srgb(1., 0.77, 0.48)), StatusText,
            Node { min_height: px(76), max_height: px(120), overflow: Overflow::scroll_y(), ..default() }));
        parent.spawn((Text::new("Need the game? Install Half-Life 2 in Steam first.\nTo find its folder: Steam > Half-Life 2 > Properties > Installed Files > Browse.\nYour selected folder will be remembered. Game files are read in place."), normal.with_font_size(18.)));
    });
}

fn actions(
    mut setup: ResMut<Setup>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    focus: Res<bevy::input_focus::InputFocus>,
    mut buttons: Query<(Entity, &Interaction, &Action, &mut BackgroundColor)>,
    field: Single<(Entity, &EditableText), With<PathField>>,
    mut exit: MessageWriter<AppExit>,
) {
    let enter = keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter);
    let mut action = if enter && focus.get() == Some(field.0) {
        Some(Action::Play)
    } else {
        None
    };
    for (entity, interaction, button, mut color) in &mut buttons {
        *color = BackgroundColor(if *interaction == Interaction::Hovered {
            Color::srgb(0.25, 0.33, 0.42)
        } else {
            Color::srgb(0.16, 0.21, 0.27)
        });
        // Trigger on the press edge, not every held frame.
        if (*interaction == Interaction::Pressed && mouse.just_pressed(MouseButton::Left))
            || ((enter || keys.just_pressed(KeyCode::Space)) && focus.get() == Some(entity))
        {
            action = Some(*button);
        }
    }
    if keys.just_pressed(KeyCode::Escape) {
        action = Some(Action::Cancel);
    }
    if matches!(action, Some(Action::Cancel)) {
        exit.write(AppExit::Success);
        return;
    }
    if setup.worker.is_some() {
        return;
    }
    if let Some(action) = action {
        start_action(&mut setup, action, field.1.value().to_string());
    }
}

fn start_action(setup: &mut Setup, action: Action, path: String) {
    let (tx, rx) = mpsc::channel();
    let map = setup.map.clone();
    setup.message = match action {
        Action::Browse => "Choose the installed game folder...",
        Action::Detect => "Searching Steam libraries...",
        _ => "Checking game files...",
    }
    .into();
    setup.worker = Some(Mutex::new(rx));
    std::thread::spawn(move || {
        let work = match action {
            Action::Browse => {
                let mut dialog =
                    rfd::FileDialog::new().set_title("Select Half-Life 2 (or its hl2 folder)");
                let initial = PathBuf::from(path.trim().trim_matches('"'));
                if initial.is_dir() {
                    dialog = dialog.set_directory(initial);
                }
                Work::Folder(dialog.pick_folder())
            }
            Action::Detect => Work::Detected(
                install::discover_steam()
                    .and_then(|p| checked(p, map.as_deref()))
                    .map_err(|e| format!("{e:#}")),
            ),
            _ => Work::Validated(
                checked(PathBuf::from(path.trim().trim_matches('"')), map.as_deref())
                    .map_err(|e| format!("{e:#}")),
            ),
        };
        let _ = tx.send(work);
    });
}

fn poll_worker(
    mut setup: ResMut<Setup>,
    mut field: Single<&mut EditableText, With<PathField>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(worker) = &setup.worker else {
        return;
    };
    let result = worker.lock().expect("setup worker mutex").try_recv();
    let work = match result {
        Ok(work) => work,
        Err(mpsc::TryRecvError::Empty) => return,
        Err(_) => {
            setup.worker = None;
            setup.message =
                "The folder operation failed. Please try again or type the path.".into();
            return;
        }
    };
    setup.worker = None;
    match work {
        Work::Folder(Some(path)) | Work::Detected(Ok(path)) => {
            **field = EditableText::new(path_text(&path));
            setup.message =
                "Folder selected. Choose Continue to check the files and start playing.".into();
        }
        Work::Folder(None) => {
            setup.message =
                "Folder selection cancelled. You can browse again or paste a path.".into()
        }
        Work::Detected(Err(message)) | Work::Validated(Err(message)) => setup.message = message,
        Work::Validated(Ok(game)) => {
            match serde_json::to_vec(&Installation { game })
                .map_err(anyhow::Error::from)
                .and_then(|bytes| std::fs::write(&setup.response, bytes).map_err(Into::into))
            {
                Ok(()) => {
                    exit.write(AppExit::Success);
                }
                Err(error) => setup.message = format!("Could not continue: {error:#}"),
            }
        }
    }
}

fn show_status(setup: Res<Setup>, mut text: Single<&mut Text, With<StatusText>>) {
    if setup.is_changed() {
        text.0.clone_from(&setup.message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            // Parallel tests can read the same clock value (microseconds on macOS): the counter
            // keeps their folders apart, so one test's cleanup never deletes another's.
            static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let count = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "hl2-setup-test-{}-{nonce}-{count}",
                std::process::id()
            ));
            for file in [
                "hl2/gameinfo.txt",
                "hl2/scripts/game_sounds_manifest.txt",
                "hl2/resource/clientscheme.res",
                "hl2/models/weapons/v_pistol.mdl",
                "hl2/maps/test.bsp",
            ] {
                let path = root.join(file);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, b"VBSP\x14\0\0\0").unwrap();
            }
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn headless(response: PathBuf) -> App {
        let mut app = App::new();
        app.add_message::<AppExit>()
            .insert_resource(Setup {
                response,
                map: Some("test".into()),
                initial: String::new(),
                message: String::new(),
                worker: None,
            })
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<bevy::input_focus::InputFocus>();
        app.world_mut().spawn((PathField, EditableText::default()));
        app
    }

    #[test]
    fn continue_click_validates_typed_folder_and_cancel_exits_cleanly() {
        let fixture = Fixture::new();
        let mut app = headless(fixture.0.join("response.json"));
        app.add_systems(Update, actions);
        *app.world_mut()
            .query::<&mut EditableText>()
            .single_mut(app.world_mut())
            .unwrap() = EditableText::new(fixture.0.to_string_lossy());
        let button = app
            .world_mut()
            .spawn((
                Button,
                Interaction::Pressed,
                Action::Play,
                BackgroundColor::default(),
            ))
            .id();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        let setup = app.world().resource::<Setup>();
        let result = setup
            .worker
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(matches!(result, Work::Validated(Ok(_))));
        app.world_mut().entity_mut(button).insert(Action::Cancel);
        app.update();
        assert!(
            app.world_mut()
                .resource_mut::<Messages<AppExit>>()
                .drain()
                .any(|exit| exit == AppExit::Success)
        );
        assert!(!fixture.0.join("response.json").exists());
    }

    #[test]
    fn failed_validation_stays_in_setup_and_success_hands_off_selected_path() {
        let fixture = Fixture::new();
        let response = fixture.0.join("response.json");
        let mut app = headless(response.clone());
        app.add_systems(Update, poll_worker);
        let (tx, rx) = mpsc::channel();
        app.world_mut().resource_mut::<Setup>().worker = Some(Mutex::new(rx));
        tx.send(Work::Validated(Err("Missing map".into()))).unwrap();
        app.update();
        assert_eq!(app.world().resource::<Setup>().message, "Missing map");
        assert!(!response.exists());
        let (tx, rx) = mpsc::channel();
        app.world_mut().resource_mut::<Setup>().worker = Some(Mutex::new(rx));
        tx.send(Work::Validated(Ok(fixture.0.clone()))).unwrap();
        app.update();
        assert_eq!(
            serde_json::from_slice::<Installation>(&std::fs::read(response).unwrap())
                .unwrap()
                .game,
            fixture.0
        );
        assert!(
            app.world_mut()
                .resource_mut::<Messages<AppExit>>()
                .drain()
                .any(|exit| exit == AppExit::Success)
        );
    }
    #[test]
    fn overrides_take_precedence_and_invalid_override_is_reported() {
        let explicit = Fixture::new();
        let environment = Fixture::new();
        let saved = Fixture::new();
        let unexpected = || panic!("discovery must not run for a valid override");
        assert_eq!(
            select_install(
                Some(explicit.0.clone()),
                Some(environment.0.clone()),
                Some(saved.0.clone()),
                Some("test"),
                unexpected
            )
            .unwrap(),
            explicit.0.canonicalize().unwrap()
        );
        assert_eq!(
            select_install(
                None,
                Some(environment.0.clone()),
                Some(saved.0.clone()),
                Some("test"),
                unexpected
            )
            .unwrap(),
            environment.0.canonicalize().unwrap()
        );
        assert_eq!(
            select_install(None, None, Some(saved.0.clone()), Some("test"), unexpected).unwrap(),
            saved.0.canonicalize().unwrap()
        );
        assert!(
            select_install(
                Some(explicit.0.join("missing")),
                None,
                Some(saved.0.clone()),
                Some("test"),
                unexpected
            )
            .is_err()
        );
    }
    #[test]
    fn stale_saved_path_recovers_through_steam() {
        let game = Fixture::new();
        assert_eq!(
            select_install(None, None, Some(game.0.join("moved")), Some("test"), || Ok(
                game.0.clone()
            ))
            .unwrap(),
            game.0.canonicalize().unwrap()
        );
    }
    #[test]
    fn config_handles_first_launch_unicode_replacement_and_corruption() {
        let fixture = Fixture::new();
        let config = fixture.0.join("settings/installation.json");
        assert!(read_saved(&config).unwrap().is_none());
        let unicode = PathBuf::from("D:/Juegos de José/Half-Life 2");
        save(&config, &unicode).unwrap();
        assert_eq!(read_saved(&config).unwrap(), Some(unicode));
        save(&config, &fixture.0).unwrap();
        assert_eq!(read_saved(&config).unwrap(), Some(fixture.0.clone()));
        std::fs::write(&config, b"{truncated").unwrap();
        assert!(read_saved(&config).is_err());
    }
}
