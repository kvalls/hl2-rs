//! Asynchronous owned-map loading. Keep the current world intact until decoding succeeds.
use crate::{Options, StartupAssets, Status, assets, console, gameplay, movement};
use bevy::{
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};
use std::sync::{Arc, Mutex};

#[derive(Component)]
pub(crate) struct MapOwned;
struct Request {
    map: String,
    landmark: String,
    direct: bool,
}
type ResultSlot = Arc<Mutex<Option<Result<assets::LoadedMap, String>>>>;
struct Job {
    request: Request,
    result: ResultSlot,
}
#[derive(Resource, Default)]
pub struct Campaign {
    job: Option<Job>,
    pub metadata: serde_json::Value,
    history: Vec<serde_json::Value>,
    failures: Vec<String>,
    loading_frames: u64,
}
impl Campaign {
    pub fn new(loaded: &assets::LoadedMap) -> Self {
        Self {
            metadata: assets::map_metadata(loaded),
            ..default()
        }
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"loading":self.job.as_ref().map(|j|&j.request.map),"loading_frames":self.loading_frames,
            "active_map":self.metadata["map"],"history":self.history,"failures":self.failures,
            "limitations":"landmark eye, inventory and global-state transfer; saved entity/AI state, full player transfer and complete campaign remain unfinished"})
    }
}
type Host<'w> = (
    Res<'w, Options>,
    ResMut<'w, Campaign>,
    ResMut<'w, console::Console>,
    ResMut<'w, gameplay::Gameplay>,
    ResMut<'w, movement::Simulation>,
);
pub fn poll(
    mut commands: Commands,
    (options, mut campaign, mut ui, mut game, mut sim): Host,
    (mut meshes, mut materials, mut images, mut sounds, mut effect_materials, mut inverse_binds): StartupAssets,
    old_draws: Query<Entity, With<MapOwned>>,
    mut windows: Query<&mut CursorOptions, With<PrimaryWindow>>,
    status: Res<Status>,
) {
    if campaign.job.is_none() {
        let request = ui
            .map_request
            .take()
            .map(|map| Request {
                map,
                landmark: String::new(),
                direct: true,
            })
            .or_else(|| {
                game.scene.transition.take().map(|(map, landmark)| Request {
                    map,
                    landmark,
                    direct: false,
                })
            });
        let Some(request) = request else {
            return;
        };
        let result: ResultSlot = Arc::new(Mutex::new(None));
        let output = result.clone();
        let map = request.map.clone();
        let root = options.game.clone();
        let canvas = ui.source.canvas.clone();
        let new_game = request.direct;
        let worker = std::thread::Builder::new()
            .name("hl2-owned-map-loader".into())
            .spawn(move || {
                let value = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    assets::load_with_canvas(&root, &map, canvas, new_game)
                }))
                .map_err(|_| "owned map loader panicked".to_owned())
                .and_then(|r| r.map_err(|e| format!("{e:#}")));
                *output.lock().expect("map loader result") = Some(value);
            });
        if let Err(error) = worker {
            let error = format!("Cannot start map loader: {error}");
            ui.source.log(&error);
            campaign.failures.push(error);
            sim.loading_failed();
            return;
        }
        campaign.job = Some(Job { request, result });
    }
    campaign.loading_frames += 1;
    let result = campaign
        .job
        .as_ref()
        .and_then(|j| j.result.try_lock().ok()?.take());
    let Some(result) = result else {
        return;
    };
    let job = campaign.job.take().expect("completed map job");
    let mut loaded = match result {
        Ok(loaded) => loaded,
        Err(error) => {
            let error = format!("Map transition failed for {}: {error}", job.request.map);
            ui.source.log(&error);
            campaign.failures.push(error);
            sim.loading_failed();
            return;
        }
    };
    let previous_eye = glam::Vec3::from_array(sim.eye().to_array());
    let eye = if job.request.direct {
        loaded.world.spawn().0
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
        loaded.gameplay.player_damage = std::mem::take(&mut game.player_damage);
        loaded.gameplay.suit = std::mem::take(&mut game.suit);
        loaded
            .gameplay
            .suit
            .rebase_clock(game.scene.time, loaded.gameplay.scene.time);
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
    for entity in &old_draws {
        commands.entity(entity).despawn();
    }
    sim.change_map(&loaded.world, eye, job.request.direct);
    if job.request.direct {
        ui.source.mode = hl2_ui::console::Mode::Gameplay;
        if let Ok(mut cursor) = windows.single_mut() {
            cursor.grab_mode = CursorGrabMode::Locked;
            cursor.visible = false;
        }
    }
    ui.source.log(format!("Loaded {}", loaded.world.name));
    let _unused_new_console = crate::install_map(
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
}
