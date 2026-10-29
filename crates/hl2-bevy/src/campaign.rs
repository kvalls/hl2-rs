//! Asynchronous owned-map loading. Keep the current world intact until decoding succeeds.
use crate::{Options, StartupAssets, Status, assets, gameplay, simulation, ui_host};
use bevy::{
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};
use std::sync::{Arc, Mutex};

#[derive(Component)]
pub(crate) struct MapOwned;

/// Observable completion marker recorded when a map has finished loading and is installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapLoaded {
    /// Installation folder of the map's game (`hl2`, `lostcoast`, ...).
    pub game: String,
    pub map: String,
    pub background: bool,
}

struct Request {
    map: String,
    /// Installation folder of the map's game (`hl2`, `lostcoast`, ...).
    game: String,
    landmark: String,
    direct: bool,
    /// Load as the main-menu background level (menu shown, HUD hidden).
    background: bool,
}
mod loading;

type ResultSlot = Arc<Mutex<Option<Result<assets::LoadedMap, String>>>>;
struct Job {
    request: Request,
    result: ResultSlot,
}
#[derive(Resource, Default)]
pub struct Campaign {
    job: Option<Job>,
    pub metadata: serde_json::Value,
    /// The currently active, fully loaded map and its game.
    pub loaded_map: Option<hl2_ui::console::GameMap>,
    pub last_event: Option<MapLoaded>,
    history: Vec<serde_json::Value>,
    failures: Vec<String>,
    loading_frames: u64,
}
impl Campaign {
    #[cfg(test)]
    pub fn new(loaded: &assets::LoadedMap) -> Self {
        let name = loaded.world.name.clone();
        Self {
            metadata: assets::map_metadata(loaded),
            loaded_map: Some(hl2_ui::console::GameMap::new(&loaded.game, &name)),
            last_event: Some(MapLoaded {
                game: loaded.game.clone(),
                map: name,
                background: false,
            }),
            ..default()
        }
    }
    /// Whether map loading has finished and the map is in an observable gameplay-ready state.
    pub fn is_gameplay_ready(&self) -> bool {
        self.job.is_none() && self.loaded_map.is_some()
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"loading":self.job.as_ref().map(|j|&j.request.map),"loading_frames":self.loading_frames,
            "active_map":self.metadata["map"],"loaded_map":self.loaded_map.as_ref().map(|m| &m.map),"loaded_game":self.loaded_map.as_ref().map(|m| &m.game),"gameplay_ready":self.is_gameplay_ready(),"last_event":self.last_event.as_ref().map(|e| &e.map),"last_event_game":self.last_event.as_ref().map(|e| &e.game),"failures":self.failures,
            "limitations":"landmark eye, inventory and global-state transfer; saved entity/AI state, full player transfer and complete campaign remain unfinished"})
    }
}
type Host<'w> = (
    Res<'w, Options>,
    ResMut<'w, Campaign>,
    ResMut<'w, ui_host::Console>,
    ResMut<'w, gameplay::Gameplay>,
    ResMut<'w, simulation::Simulation>,
);
fn next_map_request(
    options: &Options,
    campaign: &Campaign,
    ui: &mut ui_host::Console,
    game: &mut gameplay::Gameplay,
) -> Option<Request> {
    let background = std::mem::take(&mut ui.background_request);
    let requested_game = ui.map_request_game.take();
    let current_game = campaign.loaded_map.as_ref().map(|m| m.game.clone());
    // Menu backgrounds name their game and level transitions stay in the running game;
    // a bare map name (`map`, `--map`, respawn) loads in the game that ships it.
    ui.map_request
        .take()
        .map(|map| Request {
            game: requested_game.unwrap_or_else(|| {
                source_assets::install::game_for_map(&options.game, &map, current_game.as_deref())
                    .unwrap_or_else(|| source_assets::install::BASE_GAME.to_owned())
            }),
            map,
            landmark: String::new(),
            direct: true,
            background,
        })
        .or_else(|| {
            game.scene.transition.take().map(|(map, landmark)| Request {
                map,
                game: current_game
                    .clone()
                    .unwrap_or_else(|| source_assets::install::BASE_GAME.to_owned()),
                landmark,
                direct: false,
                background: false,
            })
        })
}

pub fn poll(
    mut commands: Commands,
    (options, mut campaign, mut ui, mut game, mut sim): Host,
    mut assets: StartupAssets,
    old_draws: Query<Entity, With<MapOwned>>,
    mut windows: Query<&mut CursorOptions, With<PrimaryWindow>>,
    status: Res<Status>,
    frame: Option<Res<crate::video::GameFrame>>,
) {
    if let Some(job) = &campaign.job {
        loading::preserve_gameplay_request(&job.request, &mut ui);
    } else {
        let Some(request) = next_map_request(&options, &campaign, &mut ui, &mut game) else {
            return;
        };
        if loading::reuse_background(&request, &campaign, &mut ui, &mut sim) {
            return;
        }
        let result = match loading::spawn_worker(&request, &options, &ui) {
            Ok(result) => result,
            Err(error) => {
                ui.source.log(&error);
                ui.source.loading_failed(&error);
                campaign.failures.push(error);
                sim.loading_failed();
                return;
            }
        };
        loading::log_debug_load_start(&request, &mut sim, &mut ui);
        loading::start(&request, &mut ui);
        sim.loading = true;
        campaign.job = Some(Job { request, result });
    }
    let Some((job, result)) = loading::completed(&mut campaign, &mut ui) else {
        return;
    };
    if loading::superseded(&job.request, &mut ui) {
        if let Err(error) = result {
            ui.source.log(&error);
            campaign.failures.push(error);
        }
        return;
    }
    let mut loaded = match result {
        Ok(loaded) => loaded,
        Err(error) => {
            loading::log_debug_load_finish(&job.request, false, &mut sim, &mut ui);
            if job.request.background {
                let warning = format!(
                    "Warning: background map {} failed: {error}",
                    job.request.map
                );
                ui.source.log(&warning);
                campaign.failures.push(warning);
                sim.loading_failed();
                ui.source.background_load_failed_for_map(
                    &job.request.game,
                    &job.request.map,
                    &error,
                );
                return;
            }
            let error = format!("Map transition failed for {}: {error}", job.request.map);
            ui.source.log(&error);
            ui.source.loading_failed(&error);
            campaign.failures.push(error.clone());
            sim.loading_failed();
            if !options.map_dependent.is_empty() && options.map.as_deref() == Some(&job.request.map)
            {
                eprintln!("{error}");
                std::process::exit(1);
            }
            return;
        }
    };
    let previous_eye = glam::Vec3::from_array(sim.eye().to_array());
    let eye = if job.request.direct {
        options
            .position
            .map(|p| glam::Vec3::from_array(p.to_array()))
            .unwrap_or(loaded.world.spawn().0)
    } else {
        hl2_simulation::campaign::arrival(
            &game.world,
            &loaded.world,
            previous_eye,
            &job.request.landmark,
        )
    };
    if !job.request.direct {
        // changelevel keeps the global table and the player's damage state.
        let previous = game.scene.globals.clone();
        loaded.gameplay.scene.globals.carry(&previous);
        carry_player_state(&mut loaded.gameplay, &mut game);
        loaded.gameplay.inventory = std::mem::take(&mut game.inventory);
        loaded
            .gameplay
            .inventory
            .rebase_clock(game.scene.time, loaded.gameplay.scene.time);
    }
    loaded.gameplay.consume_attacks();
    campaign.history.push(serde_json::json!({"from":game.world.name,"to":loaded.world.name,"direct":job.request.direct,"landmark":job.request.landmark,
        "previous_eye":previous_eye.to_array(),"arrival_eye":eye.to_array(),"inventory":loaded.gameplay.inventory,"previous_time":game.scene.time}));
    campaign.metadata = assets::map_metadata(&loaded);
    campaign.loaded_map = Some(hl2_ui::console::GameMap::new(
        &loaded.game,
        &loaded.world.name,
    ));
    campaign.last_event = Some(MapLoaded {
        game: loaded.game.clone(),
        map: loaded.world.name.clone(),
        background: job.request.background,
    });
    for entity in &old_draws {
        commands.entity(entity).try_despawn();
    }
    loading::install_view(&job.request, &options, &mut loaded, &mut sim, eye);
    if job.request.direct {
        ui.source
            .level_loaded_for_map(&loaded.game, &loaded.world.name, job.request.background);
        if !job.request.background
            && let Ok(mut cursor) = windows.single_mut()
        {
            cursor.grab_mode = CursorGrabMode::Locked;
            cursor.visible = false;
        }
    } else {
        ui.source
            .level_loaded_for_map(&loaded.game, &loaded.world.name, false);
    }
    loading::log_debug_load_finish(&job.request, true, &mut sim, &mut ui);
    ui.source
        .log(format!("Loaded {}/{}", loaded.game, loaded.world.name));
    crate::map_install::install_loaded_map(
        loaded,
        &mut commands,
        &mut assets,
        &status,
        &options,
        ui.source.config.video.advanced.filtering,
        frame.as_ref().map(|f| &f.image),
    );
    crate::capture::installed_frame(&mut commands, options.capture.as_ref());
}

/// Player-owned transient state survives changelevel, but fresh direct loads start anew.
fn carry_player_state(next: &mut gameplay::Gameplay, game: &mut gameplay::Gameplay) {
    next.player_damage = std::mem::take(&mut game.player_damage);
    next.suit = std::mem::take(&mut game.suit);
    next.suit.rebase_clock(game.scene.time, next.scene.time);
    next.scene.flashlight = std::mem::take(&mut game.scene.flashlight);
    next.scene
        .flashlight
        .rebase_clock(game.scene.time, next.scene.time);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::window::WindowResolution;

    fn test_options(game: &std::path::Path) -> crate::Options {
        let args = [
            "--no-setup".to_owned(),
            "--game".to_owned(),
            game.to_string_lossy().to_string(),
        ];
        let (options, installation) =
            crate::Options::parse_from(args, Default::default()).expect("valid options");
        options
            .finish_startup(installation)
            .expect("fixture installation")
    }

    #[test]
    #[ignore = "requires owned HL2 installation"]
    fn headless_bevy_app_submenu_entry_switches_background_map_and_scene() {
        let game = source_assets::install::discover().expect("owned install");
        let mut app = App::new();
        let options = test_options(&game);
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<crate::rendering::SourceMaterial>>()
            .init_resource::<Assets<Image>>()
            .init_resource::<Assets<bevy::audio::AudioSource>>()
            .init_resource::<Assets<crate::effects::EffectMaterial>>()
            .init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>()
            .init_resource::<crate::Status>()
            .insert_resource(options)
            .insert_resource(Campaign {
                loaded_map: Some(hl2_ui::console::GameMap::new("hl2", "background01")),
                last_event: Some(MapLoaded {
                    game: "hl2".into(),
                    map: "background01".into(),
                    background: true,
                }),
                ..default()
            });

        let vfs = source_assets::vpk::Vfs::mount(&game).expect("mount game");
        let canvas = source_ui::canvas::Canvas::default();
        let mut console = hl2_ui::console::Console::load(&vfs, canvas);
        console.loaded_map = Some(hl2_ui::console::GameMap::new("hl2", "background01"));
        console.level_loaded(true);

        let sim = simulation::Simulation::new(
            &modkit_core::World::default(),
            glam::Vec3::ZERO,
            0.,
            0.,
            false,
            vec![],
        );
        let gameplay = crate::gameplay::Gameplay::synthetic(modkit_core::World::default());
        let ui = ui_host::Console::new(console);

        app.insert_resource(sim)
            .insert_resource(gameplay)
            .insert_resource(ui);

        app.world_mut().spawn((
            Window {
                resolution: WindowResolution::new(1024, 768),
                ..default()
            },
            CursorOptions::default(),
            PrimaryWindow,
        ));

        let initial_scene_entity = app.world_mut().spawn(MapOwned).id();

        app.add_systems(Update, poll);
        app.add_systems(
            PostUpdate,
            (
                crate::rendering::present_entities,
                crate::sky::present.run_if(resource_exists::<crate::sky::Sky>),
                crate::effects::present.run_if(resource_exists::<crate::effects::Effects>),
            ),
        );

        // 1. Initial state assertion
        {
            let ui = app.world().resource::<ui_host::Console>();
            assert_eq!(ui.source.menu_category(), "main");
            assert_eq!(
                ui.source.loaded_map,
                Some(hl2_ui::console::GameMap::new("hl2", "background01"))
            );
            let campaign = app.world().resource::<Campaign>();
            assert_eq!(
                campaign.loaded_map,
                Some(hl2_ui::console::GameMap::new("hl2", "background01"))
            );
        }

        // 2. Enter Episode One submenu (row 1: SwitchExpansion episodic)
        let effects = {
            let mut ui = app.world_mut().resource_mut::<ui_host::Console>();
            ui.source.activate_menu(1)
        };
        assert!(
            effects.contains(&hl2_ui::console::Effect::MainMenuMap(
                hl2_ui::console::GameMap::new("episodic", "ep1_background01")
            )),
            "Effect::MainMenuMap emitted for ep1_background01"
        );

        // Route effects to simulation
        let mut pending_effects = Some(effects);
        app.world_mut()
            .run_system_once(
                move |mut sim: ResMut<simulation::Simulation>,
                      mut ui: ResMut<ui_host::Console>,
                      mut game: ResMut<crate::gameplay::Gameplay>| {
                    if let Some(effects) = pending_effects.take() {
                        sim.console_effects(&mut ui, &mut game, effects);
                    }
                },
            )
            .unwrap();

        // 3. Assert background state changes before tick
        {
            let ui = app.world().resource::<ui_host::Console>();
            let sim = app.world().resource::<simulation::Simulation>();
            assert_eq!(ui.source.menu_category(), "episodic");
            assert_eq!(
                ui.source.config.gameplay.background_map.as_deref(),
                Some("ep1_background01"),
                "persisted background map setting updated to ep1_background01"
            );
            assert!(ui.background_request);
            assert_eq!(ui.map_request.as_deref(), Some("ep1_background01"));
            assert_eq!(ui.map_request_game.as_deref(), Some("episodic"));
            assert!(sim.loading());
        }

        // Check the menu wait after the worker consumes the pending-request flag.
        loading::exercise_menu_wait_during_worker(&mut app);
        let mut completed = false;
        for _ in 0..100 {
            app.update();
            let campaign = app.world().resource::<Campaign>();
            if campaign.job.is_none()
                && campaign.loaded_map.as_ref()
                    == Some(&hl2_ui::console::GameMap::new(
                        "episodic",
                        "ep1_background01",
                    ))
            {
                completed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(
            completed,
            "background map load completed asynchronously in headless app"
        );

        // 5. Assert completion and that scene swapped
        {
            let campaign = app.world().resource::<Campaign>();
            assert_eq!(
                campaign.loaded_map,
                Some(hl2_ui::console::GameMap::new(
                    "episodic",
                    "ep1_background01"
                ))
            );
            assert_eq!(
                campaign.last_event,
                Some(MapLoaded {
                    game: "episodic".into(),
                    map: "ep1_background01".into(),
                    background: true,
                })
            );

            let ui = app.world().resource::<ui_host::Console>();
            assert_eq!(
                ui.source.loaded_map,
                Some(hl2_ui::console::GameMap::new(
                    "episodic",
                    "ep1_background01"
                ))
            );
            assert_eq!(ui.source.menu_category(), "episodic");
            assert!(!ui.source.is_loading());

            let sim = app.world().resource::<simulation::Simulation>();
            assert!(!sim.loading());

            // Assert active background scene entities swapped
            assert!(
                app.world().get_entity(initial_scene_entity).is_err(),
                "old background scene entity despawned"
            );

            let mut query = app.world_mut().query_filtered::<Entity, With<MapOwned>>();
            let current_scene_entities: Vec<Entity> = query.iter(app.world()).collect();
            assert!(
                !current_scene_entities.is_empty(),
                "new background scene entities spawned with MapOwned"
            );
            assert!(
                !current_scene_entities.contains(&initial_scene_entity),
                "new background scene replaced the initial scene"
            );
        }

        // 6. From Episode One, switch to Lost Coast
        let ep1_scene_entity = {
            let mut query = app.world_mut().query_filtered::<Entity, With<MapOwned>>();
            query.iter(app.world()).next().expect("ep1 scene entity")
        };
        // In episodic submenu, row 5 is Back to main
        let back_effects = {
            let mut ui = app.world_mut().resource_mut::<ui_host::Console>();
            ui.source.activate_menu(5)
        };
        assert!(
            !back_effects
                .iter()
                .any(|e| matches!(e, hl2_ui::console::Effect::MainMenuMap(_))),
            "Back button does not trigger redundant map reload"
        );

        // In main menu: row 4 is EXTRAS
        let _ = {
            let mut ui = app.world_mut().resource_mut::<ui_host::Console>();
            ui.source.activate_menu(4)
        };
        // In extras: row 0 is LOST COAST (SwitchExpansion lostcoast)
        let lc_effects = {
            let mut ui = app.world_mut().resource_mut::<ui_host::Console>();
            ui.source.activate_menu(0)
        };
        assert!(
            lc_effects.contains(&hl2_ui::console::Effect::MainMenuMap(
                hl2_ui::console::GameMap::new("lostcoast", "background01")
            )),
            "Effect::MainMenuMap emitted for lostcoast/background01"
        );
        let mut pending_effects = Some(lc_effects);
        app.world_mut()
            .run_system_once(
                move |mut sim: ResMut<simulation::Simulation>,
                      mut ui: ResMut<ui_host::Console>,
                      mut game: ResMut<crate::gameplay::Gameplay>| {
                    if let Some(effects) = pending_effects.take() {
                        sim.console_effects(&mut ui, &mut game, effects);
                    }
                },
            )
            .unwrap();

        assert_eq!(
            app.world()
                .resource::<ui_host::Console>()
                .source
                .config
                .gameplay
                .background_map
                .as_deref(),
            Some("background01"),
            "persisted background map updated to Lost Coast's background01"
        );
        assert_eq!(
            app.world()
                .resource::<ui_host::Console>()
                .source
                .config
                .gameplay
                .background_game
                .as_deref(),
            Some("lostcoast")
        );

        let mut completed = false;
        for _ in 0..100 {
            app.update();
            let campaign = app.world().resource::<Campaign>();
            if campaign.job.is_none()
                && campaign.loaded_map.as_ref()
                    == Some(&hl2_ui::console::GameMap::new("lostcoast", "background01"))
            {
                completed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(
            completed,
            "lostcoast background map load completed asynchronously in headless app"
        );

        {
            let campaign = app.world().resource::<Campaign>();
            assert_eq!(
                campaign.loaded_map,
                Some(hl2_ui::console::GameMap::new("lostcoast", "background01"))
            );
            assert_eq!(
                campaign.last_event,
                Some(MapLoaded {
                    game: "lostcoast".into(),
                    map: "background01".into(),
                    background: true,
                })
            );

            // Assert active background scene entities swapped again
            assert!(
                app.world().get_entity(ep1_scene_entity).is_err(),
                "ep1 scene entity despawned on lost coast load"
            );
        }
    }

    #[test]
    #[ignore = "requires owned HL2 installation"]
    fn headless_bevy_app_ep2_submenu_entry_switches_background_map() {
        let game = source_assets::install::discover().expect("owned install");
        let mut app = App::new();
        let options = test_options(&game);
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<crate::rendering::SourceMaterial>>()
            .init_resource::<Assets<Image>>()
            .init_resource::<Assets<bevy::audio::AudioSource>>()
            .init_resource::<Assets<crate::effects::EffectMaterial>>()
            .init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>()
            .init_resource::<crate::Status>()
            .insert_resource(options)
            .insert_resource(Campaign {
                loaded_map: Some(hl2_ui::console::GameMap::new("hl2", "background01")),
                last_event: Some(MapLoaded {
                    game: "hl2".into(),
                    map: "background01".into(),
                    background: true,
                }),
                ..default()
            });

        let vfs = source_assets::vpk::Vfs::mount(&game).expect("mount game");
        let canvas = source_ui::canvas::Canvas::default();
        let mut console = hl2_ui::console::Console::load(&vfs, canvas);
        console.loaded_map = Some(hl2_ui::console::GameMap::new("hl2", "background01"));
        console.level_loaded(true);

        let sim = simulation::Simulation::new(
            &modkit_core::World::default(),
            glam::Vec3::ZERO,
            0.,
            0.,
            false,
            vec![],
        );
        let gameplay = crate::gameplay::Gameplay::synthetic(modkit_core::World::default());
        let ui = ui_host::Console::new(console);

        app.insert_resource(sim)
            .insert_resource(gameplay)
            .insert_resource(ui);

        app.world_mut().spawn((
            Window {
                resolution: WindowResolution::new(1024, 768),
                ..default()
            },
            CursorOptions::default(),
            PrimaryWindow,
        ));

        let initial_scene_entity = app.world_mut().spawn(MapOwned).id();
        app.add_systems(Update, poll);
        app.add_systems(
            PostUpdate,
            (
                crate::rendering::present_entities,
                crate::sky::present.run_if(resource_exists::<crate::sky::Sky>),
                crate::effects::present.run_if(resource_exists::<crate::effects::Effects>),
            ),
        );

        // Enter Episode Two submenu (row 2: SwitchExpansion ep2)
        let effects = {
            let mut ui = app.world_mut().resource_mut::<ui_host::Console>();
            ui.source.activate_menu(2)
        };
        assert!(
            effects.contains(&hl2_ui::console::Effect::MainMenuMap(
                hl2_ui::console::GameMap::new("ep2", "ep2_background01")
            )),
            "Effect::MainMenuMap emitted for ep2_background01"
        );
        let mut pending_effects = Some(effects);
        app.world_mut()
            .run_system_once(
                move |mut sim: ResMut<simulation::Simulation>,
                      mut ui: ResMut<ui_host::Console>,
                      mut game: ResMut<crate::gameplay::Gameplay>| {
                    if let Some(effects) = pending_effects.take() {
                        sim.console_effects(&mut ui, &mut game, effects);
                    }
                },
            )
            .unwrap();

        let mut completed = false;
        for _ in 0..100 {
            app.update();
            let campaign = app.world().resource::<Campaign>();
            if campaign.job.is_none()
                && campaign.loaded_map.as_ref()
                    == Some(&hl2_ui::console::GameMap::new("ep2", "ep2_background01"))
            {
                completed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(
            completed,
            "ep2 background map load completed asynchronously in headless app"
        );

        let campaign = app.world().resource::<Campaign>();
        assert_eq!(
            campaign.loaded_map,
            Some(hl2_ui::console::GameMap::new("ep2", "ep2_background01"))
        );
        assert_eq!(
            campaign.last_event,
            Some(MapLoaded {
                game: "ep2".into(),
                map: "ep2_background01".into(),
                background: true,
            })
        );
        assert!(
            app.world().get_entity(initial_scene_entity).is_err(),
            "initial scene entity despawned on ep2 background map load"
        );
    }

    #[test]
    fn headless_bevy_app_background_map_failure_logs_warning_and_does_not_crash() {
        let mut app = App::new();
        let temp = std::env::temp_dir().join(format!("hl2_mock_install_{}", std::process::id()));
        let _ = std::fs::create_dir_all(temp.join("hl2"));
        let _ = std::fs::write(
            temp.join("hl2/gameinfo.txt"),
            b"\"GameInfo\" { \"game\" \"Half-Life 2\" }",
        );
        // The launch check needs the shared content a real installation always has.
        for file in [
            "scripts/game_sounds_manifest.txt",
            "resource/ClientScheme.res",
            "models/weapons/v_pistol.mdl",
        ] {
            let path = temp.join("hl2").join(file);
            let _ = std::fs::create_dir_all(path.parent().unwrap());
            let _ = std::fs::write(path, b"fixture");
        }
        let options = test_options(&temp);
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<crate::rendering::SourceMaterial>>()
            .init_resource::<Assets<Image>>()
            .init_resource::<Assets<bevy::audio::AudioSource>>()
            .init_resource::<Assets<crate::effects::EffectMaterial>>()
            .init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>()
            .init_resource::<crate::Status>()
            .insert_resource(options)
            .insert_resource(Campaign {
                loaded_map: Some(hl2_ui::console::GameMap::new("hl2", "background01")),
                ..default()
            });

        let mut console = hl2_ui::console::Console::default();
        console.loaded_map = Some(hl2_ui::console::GameMap::new("hl2", "background01"));
        console.level_loaded(true);

        let sim = simulation::Simulation::new(
            &modkit_core::World::default(),
            glam::Vec3::ZERO,
            0.,
            0.,
            false,
            vec![],
        );
        let gameplay = crate::gameplay::Gameplay::synthetic(modkit_core::World::default());
        let mut ui = ui_host::Console::new(console);
        ui.map_request = Some("invalid_nonexistent_background_map".into());
        ui.background_request = true;

        app.insert_resource(sim)
            .insert_resource(gameplay)
            .insert_resource(ui);

        app.world_mut().spawn((
            Window {
                resolution: WindowResolution::new(1024, 768),
                ..default()
            },
            CursorOptions::default(),
            PrimaryWindow,
        ));

        app.add_systems(Update, poll);
        app.add_systems(
            PostUpdate,
            (
                crate::rendering::present_entities,
                crate::sky::present.run_if(resource_exists::<crate::sky::Sky>),
                crate::effects::present.run_if(resource_exists::<crate::effects::Effects>),
            ),
        );

        // Run updates until worker fails
        let mut failed = false;
        for _ in 0..50 {
            app.update();
            let campaign = app.world().resource::<Campaign>();
            if campaign.job.is_none() && !campaign.failures.is_empty() {
                failed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(failed, "background failure detected");

        let campaign = app.world().resource::<Campaign>();
        assert!(
            campaign
                .failures
                .iter()
                .any(|f| f
                    .contains("Warning: background map invalid_nonexistent_background_map failed")),
            "warning recorded in campaign.failures"
        );

        let ui = app.world().resource::<ui_host::Console>();
        assert!(
            ui.source.shell.panel.output.iter().any(|line| line
                .contains("Warning: background map invalid_nonexistent_background_map failed")),
            "warning logged to console"
        );
        assert!(!ui.source.is_loading());
        assert_eq!(ui.source.mode(), hl2_ui::console::Mode::MainMenu);
    }
}

#[cfg(test)]
mod flashlight_tests {
    use super::*;
    use hl2_simulation::flashlight::Context;

    #[test]
    fn campaign_carries_flashlight_permissions_power_and_rebases_the_clock() {
        let mut old = gameplay::Gameplay::synthetic(Default::default());
        old.scene.time = 100.;
        let context = Context {
            suit: true,
            now: 100.,
            ..Default::default()
        };
        old.scene.flashlight.step(true, 1, true, Some(1.), context);
        old.scene.flashlight.request_enabled(false);
        old.scene.flashlight.step(true, 0, true, None, context);
        let power = old.scene.flashlight.state().unwrap().power();
        let mut next = gameplay::Gameplay::synthetic(Default::default());
        assert!(
            next.scene.flashlight.state().is_none(),
            "direct loads get fresh player state"
        );
        carry_player_state(&mut next, &mut old);
        assert!(old.scene.flashlight.state().is_none());
        let state = next.scene.flashlight.state().unwrap();
        assert!(state.disabled && !state.on);
        assert_eq!(state.power(), power);
        let context = Context {
            now: 0.25,
            ..context
        };
        assert_eq!(
            next.scene
                .flashlight
                .step(true, 1, true, Some(0.25), context)
                .power(),
            power
        );
    }
}
