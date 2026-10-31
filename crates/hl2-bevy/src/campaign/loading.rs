//! Coordinate pending requests with the authoritative active worker, not UI queue flags.
use super::Request;
use crate::ui_host;

/// Resolve zero-time map-spawn I/O and the authored background view before revealing it.
/// Gameplay maps retain their ordinary first-tick startup and command-line overrides.
pub(super) fn install_view(
    request: &Request,
    options: &crate::Options,
    loaded: &mut crate::assets::LoadedMap,
    sim: &mut crate::simulation::Simulation,
    eye: glam::Vec3,
) {
    sim.change_map(&loaded.world, eye, request.direct);
    sim.background_level = request.background;
    sim.background_audio_pending = loaded.pending_audio.is_some();
    if request.direct {
        if let Some(yaw) = options.yaw {
            sim.yaw = yaw;
        }
        if options.pitch != 0. {
            sim.pitch = options.pitch;
        }
        if options.fly {
            sim.fly = true;
        }
    }
    if request.background {
        sim.prepare_background_view(&mut loaded.gameplay);
    }
}

pub(super) fn activate_request(
    request: Request,
    options: &crate::Options,
    campaign: &mut super::Campaign,
    ui: &mut ui_host::Console,
    sim: &mut crate::simulation::Simulation,
    (audio, pending): (
        Option<&crate::audio::Audio>,
        Option<&crate::audio::deferred::PendingAudio>,
    ),
) -> bool {
    if reuse_background(
        &request,
        campaign,
        ui,
        sim,
        audio.is_some_and(crate::audio::Audio::is_ready),
    ) {
        return false;
    }
    if let Some(pending) = pending {
        pending.cancel();
    }
    match begin(request, options, ui, sim, campaign.audio_reuse.clone()) {
        Ok(job) => {
            campaign.job = Some(job);
            true
        }
        Err(error) => {
            ui.source.log(&error);
            ui.source.loading_failed(&error);
            campaign.failures.push(error);
            sim.loading_failed();
            false
        }
    }
}

pub(super) fn begin(
    request: Request,
    options: &crate::Options,
    ui: &mut ui_host::Console,
    sim: &mut crate::simulation::Simulation,
    reuse: std::sync::Arc<crate::audio::reuse::AudioReuse>,
) -> Result<super::Job, String> {
    sim.background_audio_pending = false;
    let started = std::time::Instant::now();
    let result = spawn_worker(&request, options, ui, reuse)?;
    sim.notify_loading(ui, &request.map, request.background, None);
    start(&request, ui);
    sim.loading = true;
    Ok(super::Job {
        request,
        result,
        started,
    })
}

pub(super) fn spawn_worker(
    request: &Request,
    options: &crate::Options,
    ui: &ui_host::Console,
    audio_reuse: std::sync::Arc<crate::audio::reuse::AudioReuse>,
) -> Result<super::ResultSlot, String> {
    let map = request.map.clone();
    let map_game = request.game.clone();
    let root = options.game.clone();
    let canvas = ui.source.canvas.clone();
    let new_game = request.direct;
    let background = request.background;
    let config = ui.source.config.clone();
    let progress = ui.source.loading_progress();
    super::assets::work::Work::spawn("hl2-owned-map-loader", move |cancel| {
        crate::assets::loading::load(
            crate::assets::loading::LoadInput {
                root: &root,
                game: &map_game,
                map: &map,
                canvas,
                new_game,
                config: &config,
                progress: &progress,
            },
            crate::assets::loading::LoadMode {
                background,
                cancel,
                audio_reuse,
            },
        )
    })
}

pub(super) fn notify_finished(
    request: &Request,
    sim: &mut crate::simulation::Simulation,
    ui: &mut ui_host::Console,
) {
    sim.notify_loading(ui, &request.map, request.background, Some(true));
}

pub(super) fn start(request: &Request, ui: &mut ui_host::Console) {
    if request.background {
        ui.source
            .start_background_loading(&request.game, &request.map);
    } else {
        ui.source
            .start_loading_for_game(&request.game, &request.map);
    }
}

pub(super) fn preserve_gameplay_request(request: &Request, ui: &mut ui_host::Console) {
    if !request.background && ui.background_request {
        ui.source
            .log("Gameplay map load in progress; background map request ignored");
        ui.background_request = false;
        ui.map_request = None;
        ui.map_request_game = None;
        ui.source
            .start_loading_for_game(&request.game, &request.map);
    }
}

/// A completed background must not replace a newer requested world or loading dialog.
/// Identical pending backgrounds are coalesced into this completion.
pub(super) fn superseded(request: &Request, ui: &mut ui_host::Console) -> bool {
    if !request.background || ui.map_request.is_none() {
        return false;
    }
    if ui.background_request
        && ui.map_request.as_deref() == Some(&request.map)
        && ui.map_request_game.as_deref() == Some(&request.game)
    {
        ui.map_request = None;
        ui.map_request_game = None;
        ui.background_request = false;
        return false;
    }
    true
}

/// A rapid switch back can keep the already installed background after discarding its
/// replacement. Finish the presentation without loading or installing that world twice.
pub(super) fn reuse_background(
    request: &Request,
    campaign: &super::Campaign,
    ui: &mut ui_host::Console,
    sim: &mut crate::simulation::Simulation,
    audio_ready: bool,
) -> bool {
    if !audio_ready
        || !request.background
        || !campaign.last_event.as_ref().is_some_and(|event| {
            event.background && event.game == request.game && event.map == request.map
        })
    {
        return false;
    }
    ui.source
        .level_loaded_for_map(&request.game, &request.map, true);
    sim.loading = false;
    true
}

pub(super) fn completed(
    campaign: &mut super::Campaign,
    ui: &mut ui_host::Console,
) -> Option<(super::Job, Result<crate::assets::LoadedMap, String>)> {
    campaign.loading_frames += 1;
    ui.source.update_loading();
    if let Some(job) = &campaign.job
        && job.request.background
        && ui.map_request.is_some()
        && !(ui.background_request
            && ui.map_request.as_deref() == Some(&job.request.map)
            && ui.map_request_game.as_deref() == Some(&job.request.game))
    {
        job.result.cancel.cancel();
    }
    let result = campaign.job.as_ref()?.result.poll()?;
    Some((campaign.job.take().expect("completed map job"), result))
}

#[cfg(test)]
pub(super) fn exercise_menu_wait_during_worker(app: &mut bevy::prelude::App) {
    app.update();
    assert!(app.world().resource::<super::Campaign>().job.is_some());
    assert!(
        app.world()
            .resource::<crate::simulation::Simulation>()
            .loading()
    );
    let mut ui = app.world_mut().resource_mut::<ui_host::Console>();
    assert!(!ui.background_request, "worker has consumed the queue flag");
    assert!(ui.source.activate_menu(0).is_empty());
    assert!(!ui.source.menu_report()["visible"].as_bool().unwrap());
    ui.source.input(&source_ui::input::Input {
        keys: [source_ui::input::Key::Right].into(),
        ..Default::default()
    });
    assert!(ui.source.chapters_report().is_none());
    ui.source.escape_pressed();
    ui.source.escape_released();
    assert_eq!(ui.source.mode(), hl2_ui::console::Mode::MainMenu);
    assert!(ui.source.is_background_loading());
    assert_eq!(ui.source.menu_category(), "episodic");
}

/// Transfer gameplay state only after a worker succeeds and wins request arbitration.
pub(super) fn carry_state(
    request: &Request,
    loaded: &mut crate::assets::LoadedMap,
    game: &mut crate::gameplay::Gameplay,
) {
    if !request.direct {
        // changelevel keeps the global table and the player's damage state.
        let previous = game.scene.globals.clone();
        loaded.gameplay.scene.globals.carry(&previous);
        super::carry_player_state(&mut loaded.gameplay, game);
        loaded.gameplay.inventory = std::mem::take(&mut game.inventory);
        loaded
            .gameplay
            .inventory
            .rebase_clock(game.scene.time, loaded.gameplay.scene.time);
    }
}

pub(super) fn failed(
    request: &Request,
    error: String,
    options: &crate::Options,
    campaign: &mut super::Campaign,
    ui: &mut ui_host::Console,
    sim: &mut crate::simulation::Simulation,
) {
    sim.notify_loading(ui, &request.map, request.background, Some(false));
    if request.background {
        let warning = format!("Warning: background map {} failed: {error}", request.map);
        ui.source.log(&warning);
        campaign.failures.push(warning);
        sim.loading_failed();
        ui.source
            .background_load_failed_for_map(&request.game, &request.map, &error);
        return;
    }
    let error = format!("Map transition failed for {}: {error}", request.map);
    ui.source.log(&error);
    ui.source.loading_failed(&error);
    campaign.failures.push(error.clone());
    sim.loading_failed();
    if !options.map_dependent.is_empty() && options.map.as_deref() == Some(&request.map) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

pub(super) fn record_loaded(
    campaign: &mut super::Campaign,
    loaded: &crate::assets::LoadedMap,
    job: &super::Job,
) {
    campaign.metadata = crate::assets::map_metadata(loaded);
    campaign.metadata["loading_ms"]["request_to_worker_ms"] =
        (job.started.elapsed().as_secs_f64() * 1000.).into();
    campaign.loaded_map = Some(hl2_ui::console::GameMap::new(
        &loaded.game,
        &loaded.world.name,
    ));
    campaign.last_event = Some(super::MapLoaded {
        game: loaded.game.clone(),
        map: loaded.world.name.clone(),
        background: job.request.background,
    });
}

#[cfg(test)]
mod tests {
    use super::{Request, preserve_gameplay_request, superseded, ui_host};
    fn request(background: bool) -> Request {
        Request {
            map: "background01".into(),
            game: "hl2".into(),
            landmark: String::new(),
            direct: true,
            background,
        }
    }
    fn queued(background: bool, game: &str, map: &str) -> ui_host::Console {
        let mut ui = ui_host::Console::new(Default::default());
        ui.map_request = Some(map.into());
        ui.map_request_game = Some(game.into());
        ui.background_request = background;
        ui
    }
    #[test]
    fn gameplay_worker_restores_modal_after_rejecting_background_request() {
        let mut ui = queued(true, "ep2", "ep2_background01");
        ui.source
            .start_background_loading("ep2", "ep2_background01");
        preserve_gameplay_request(&request(false), &mut ui);
        assert!(ui.map_request.is_none());
        assert!(!ui.background_request);
        assert!(ui.source.modal_dialog_open());
        assert!(!ui.source.is_background_loading());
    }
    #[test]
    fn background_completion_preserves_newer_background_or_chapter_request() {
        for background in [true, false] {
            let mut ui = queued(background, "ep2", "ep2_outland_01");
            assert!(superseded(&request(true), &mut ui));
            assert_eq!(ui.map_request.as_deref(), Some("ep2_outland_01"));
        }
        let mut ui = queued(true, "lostcoast", "background01");
        assert!(
            superseded(&request(true), &mut ui),
            "identity includes the game"
        );
    }
    #[test]
    fn identical_pending_background_is_coalesced_but_browser_close_keeps_worker() {
        let mut ui = queued(true, "hl2", "background01");
        assert!(!superseded(&request(true), &mut ui));
        assert!(ui.map_request.is_none());
        assert!(!ui.background_request);
        ui.source.set_mode(hl2_ui::console::Mode::NewGame);
        ui.source.set_mode(hl2_ui::console::Mode::MainMenu);
        assert!(!superseded(&request(true), &mut ui));
    }
    #[test]
    fn rapid_switch_back_reuses_installed_background_and_finishes_loading() {
        let mut campaign = crate::campaign::Campaign::default();
        campaign.last_event = Some(crate::campaign::MapLoaded {
            game: "hl2".into(),
            map: "background01".into(),
            background: true,
        });
        let mut ui = queued(true, "hl2", "background01");
        ui.source.start_background_loading("hl2", "background01");
        let mut sim = crate::simulation::Simulation::new(
            &modkit_core::World::default(),
            glam::Vec3::ZERO,
            0.,
            0.,
            false,
            vec![],
        );
        sim.loading = true;
        assert!(super::reuse_background(
            &request(true),
            &campaign,
            &mut ui,
            &mut sim,
            true
        ));
        assert!(!sim.loading);
        assert!(!ui.source.is_background_loading());
        assert!(!ui.source.is_loading());
        sim.loading = true;
        assert!(
            !super::reuse_background(&request(true), &campaign, &mut ui, &mut sim, false),
            "a cancelled or failed late-audio result requires a fresh load, not permanent silent reuse"
        );
        assert!(sim.loading);
        campaign.last_event.as_mut().unwrap().background = false;
        assert!(!super::reuse_background(
            &request(true),
            &campaign,
            &mut ui,
            &mut sim,
            true
        ));
    }
    #[test]
    fn newer_background_and_gameplay_requests_cancel_active_background_work() {
        use std::{sync::mpsc, time::Duration};
        for background in [true, false] {
            let (started_tx, started_rx) = mpsc::channel();
            let (finished_tx, finished_rx) = mpsc::channel();
            let work = crate::assets::work::Work::spawn("superseded-map-test", move |cancel| {
                started_tx.send(()).unwrap();
                while cancel.check().is_ok() {
                    std::thread::yield_now();
                }
                finished_tx.send(()).unwrap();
                cancel.check()?;
                unreachable!()
            })
            .unwrap();
            let token = work.cancel.clone();
            started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            let mut campaign = crate::campaign::Campaign::default();
            campaign.job = Some(crate::campaign::Job {
                request: request(true),
                result: work,
                started: std::time::Instant::now(),
            });
            let mut ui = queued(background, "ep2", "ep2_background01");
            let _ = super::completed(&mut campaign, &mut ui);
            assert!(token.cancelled());
            finished_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(ui.map_request.as_deref(), Some("ep2_background01"));
            assert!(campaign.failures.is_empty());
        }
    }
}
