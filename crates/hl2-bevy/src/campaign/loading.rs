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

pub(super) fn spawn_worker(
    request: &Request,
    options: &crate::Options,
    ui: &ui_host::Console,
) -> Result<super::ResultSlot, String> {
    let result: super::ResultSlot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let output = result.clone();
    let map = request.map.clone();
    let map_game = request.game.clone();
    let root = options.game.clone();
    let canvas = ui.source.canvas.clone();
    let new_game = request.direct;
    let config = ui.source.config.clone();
    let progress = ui.source.loading_progress();
    std::thread::Builder::new()
        .name("hl2-owned-map-loader".into())
        .spawn(move || {
            let value = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::assets::load_with_progress(
                    &root, &map_game, &map, canvas, new_game, &config, &progress,
                )
            }))
            .map_err(|_| "owned map loader panicked".to_owned())
            .and_then(|r| r.map_err(|e| format!("{e:#}")));
            *output.lock().expect("map loader result") = Some(value);
        })
        .map(|_| result)
        .map_err(|error| format!("Cannot start map loader: {error}"))
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
) -> bool {
    if !request.background
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
    let result = campaign.job.as_ref()?.result.try_lock().ok()?.take()?;
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

pub(super) fn log_debug_load_start(
    request: &Request,
    sim: &mut crate::simulation::Simulation,
    ui: &mut ui_host::Console,
) {
    if request.background {
        if sim.background_debug.enabled {
            let is_wide = ui.source.is_widescreen();
            let blur_name = ui.source.blur_texture_name(&request.map, is_wide);
            ui.source.log(format!(
                "Started loading {}, applied {}.",
                request.map, blur_name
            ));
            sim.background_debug.map_name = request.map.clone();
            sim.background_debug.blur_texture_name = blur_name;
            sim.background_debug.state_label = "Loading".to_owned();
        }
    } else if sim.background_debug.enabled {
        let is_wide = ui.source.is_widescreen();
        let bg_map = ui
            .source
            .loading_background
            .as_ref()
            .map(|m| m.map.as_str())
            .unwrap_or("background01");
        let blur_name = ui.source.blur_texture_name(bg_map, is_wide);
        ui.source.log(format!(
            "Started loading map {}, applied {}.",
            request.map, blur_name
        ));
        sim.background_debug.pending_gameplay_finish = true;
        sim.background_debug.enabled = false;
        ui.source.background_debug_mode = false;
        sim.background_debug.state_label = "Gameplay loading".to_owned();
    }
}

pub(super) fn log_debug_load_finish(
    request: &Request,
    success: bool,
    sim: &mut crate::simulation::Simulation,
    ui: &mut ui_host::Console,
) {
    if request.background {
        if sim.background_debug.enabled {
            if success {
                let opacity = sim.background_debug.blur_opacity;
                let blur_name = &sim.background_debug.blur_texture_name;
                ui.source.log(format!(
                    "Finished loading {}, success, set blur overlay {} opacity to {}.",
                    request.map, blur_name, opacity
                ));
                sim.background_debug.state_label = "Ready".to_owned();
            } else {
                ui.source
                    .log(format!("Finished loading {}, failure.", request.map));
                sim.background_debug.state_label = "Failure".to_owned();
            }
        }
    } else if sim.background_debug.pending_gameplay_finish {
        sim.background_debug.pending_gameplay_finish = false;
        let status = if success { "success." } else { "failure." };
        ui.source
            .log(format!("Finished loading map {}, {}", request.map, status));
    }
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
            &mut sim
        ));
        assert!(!sim.loading);
        assert!(!ui.source.is_background_loading());
        assert!(!ui.source.is_loading());
        campaign.last_event.as_mut().unwrap().background = false;
        assert!(!super::reuse_background(
            &request(true),
            &campaign,
            &mut ui,
            &mut sim
        ));
    }
}
