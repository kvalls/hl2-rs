use super::State;
use crate::bevy::input::{
    DebugRepeats, KeyAdjustRepeat, VIEWMODEL_ADJUST_KEYS, adjust_viewmodel_framing,
    reset_viewmodel_debug_weapon, viewmodel_adjust_step,
};
use bevy::prelude::{ButtonInput, KeyCode};
pub(super) fn select_viewmodel_debug_target(
    debug: &mut crate::viewmodel::ViewmodelDebug,
    weapon: &str,
    profile: &'static str,
    profiles: &dyn hl2_ui::extensions::FramingProfiles,
) {
    if debug.weapon == weapon && debug.profile == Some(profile) {
        return;
    }
    if let Some(previous_profile) = debug.profile {
        debug
            .edits
            .insert((debug.weapon.clone(), previous_profile), debug.framing);
    }
    debug.weapon = weapon.to_owned();
    debug.profile = Some(profile);
    debug.framing = debug
        .edits
        .get(&(weapon.to_owned(), profile))
        .copied()
        .unwrap_or_else(|| profiles.endpoint(weapon, profile));
}

fn commit_viewmodel_debug_edit(
    debug: &mut crate::viewmodel::ViewmodelDebug,
    weapon: String,
    profile: &'static str,
    profiles: &dyn hl2_ui::extensions::FramingProfiles,
    ui: Option<&mut hl2_bevy::ui_host::Console>,
) {
    debug.edits.insert((weapon, profile), debug.framing);
    match profiles.snapshot(&debug.edits) {
        Ok(json) => {
            if let Some(ui) = ui {
                ui.source.log_snapshot(json);
            } else {
                bevy::log::info!(target: "hl2_bevy::viewmodel_debug", "{json}");
            }
        }
        Err(error) => bevy::log::error!(
            target: "hl2_bevy::viewmodel_debug",
            "Could not serialize tuning.json snapshot: {error}"
        ),
    }
}

pub(super) struct DebugTuningInput<'a> {
    pub(super) keys: &'a ButtonInput<KeyCode>,
    pub(super) time: Option<f64>,
    pub(super) aspect: f32,
}

pub(super) fn process_debug_tuning_keys(
    state: &mut State,
    sim: &mut hl2_bevy::simulation::Simulation,
    game: Option<&hl2_bevy::gameplay::Gameplay>,
    mut ui: Option<&mut hl2_bevy::ui_host::Console>,
    input: DebugTuningInput,
    repeats: &mut DebugRepeats,
) {
    if sim.loading() || sim.background_level {
        state.viewmodel_debug = crate::viewmodel::ViewmodelDebug::default();
    }
    let in_gameplay = !sim.background_level
        && !sim.loading()
        && ui
            .as_ref()
            .is_some_and(|u| !u.source.has_background_context());
    if in_gameplay {
        if state.background_debug.enabled {
            state.background_debug.fov = None;
        }
        state.background_debug.enabled = false;
        if let Some(ref mut ui) = ui {
            ui.source.presentation.capture_menu_navigation = false;
        }
    }
    if state.viewmodel_debug.enabled {
        let Some(weapon) = game.map(|g| g.inventory.active.clone()) else {
            state.viewmodel_debug = crate::viewmodel::ViewmodelDebug::default();
            repeats.vm = KeyAdjustRepeat::default();
            return;
        };
        let profiles = sim.video_policy.0.framing_profiles();
        let profile = profiles.profile_at(input.aspect);
        select_viewmodel_debug_target(&mut state.viewmodel_debug, &weapon, profile, profiles);
        let adjusting = !sim.paused() && !sim.transition;
        let reset = std::mem::take(&mut state.viewmodel_debug.reset_pending)
            || (adjusting && crate::bevy::input::reset_key_pressed(input.keys));
        let changed = if reset {
            reset_viewmodel_debug_weapon(&mut state.viewmodel_debug)
        } else if adjusting {
            let fired = repeats
                .vm
                .update(input.keys, input.time, VIEWMODEL_ADJUST_KEYS);
            let step = viewmodel_adjust_step(input.keys);
            adjust_viewmodel_framing(&mut state.viewmodel_debug.framing, &fired, step)
        } else {
            repeats.vm = KeyAdjustRepeat::default();
            false
        };
        if changed {
            commit_viewmodel_debug_edit(
                &mut state.viewmodel_debug,
                weapon,
                profile,
                profiles,
                ui.as_deref_mut(),
            );
        }
    } else {
        repeats.vm = KeyAdjustRepeat::default();
    }
    if state.background_debug.enabled {
        process_background_debug_keys(
            state,
            sim,
            ui,
            input,
            &mut repeats.bg_fov,
            &mut repeats.bg_opacity,
        );
    } else {
        repeats.bg_fov = KeyAdjustRepeat::default();
        repeats.bg_opacity = KeyAdjustRepeat::default();
    }
}

fn process_background_debug_keys(
    state: &mut State,
    sim: &mut hl2_bevy::simulation::Simulation,
    mut ui: Option<&mut hl2_bevy::ui_host::Console>,
    input: DebugTuningInput,
    fov_repeat: &mut KeyAdjustRepeat,
    opacity_repeat: &mut KeyAdjustRepeat,
) {
    let console_open = ui
        .as_ref()
        .is_some_and(|u| u.source.mode() == hl2_ui::console::Mode::Console);
    if !console_open {
        let fired = fov_repeat.update(
            input.keys,
            input.time,
            crate::bevy::host::background::BACKGROUND_ADJUST_KEYS,
        );
        let step = viewmodel_adjust_step(input.keys);
        state.background_debug.adjust_fov(
            &fired,
            step,
            hl2_bevy::camera::background_fov(input.aspect, sim.background_force_default_blur)
                .to_degrees(),
        );

        let opacity_fired = opacity_repeat.update(
            input.keys,
            input.time,
            crate::bevy::host::background::BACKGROUND_OPACITY_KEYS,
        );
        state.background_debug.adjust_opacity_step(&opacity_fired);

        if input.keys.just_pressed(KeyCode::ArrowLeft) {
            state.background_debug.toggle_zero_opacity();
        }
        if input.keys.just_pressed(KeyCode::ArrowRight) {
            state.background_debug.toggle_full_opacity();
        }
        if input.keys.just_pressed(KeyCode::Digit0) || input.keys.just_pressed(KeyCode::Numpad0) {
            state.background_debug.fov = None;
        }
        if let Some(ref mut ui) = ui {
            if input.keys.just_pressed(KeyCode::Minus)
                || input.keys.just_pressed(KeyCode::NumpadSubtract)
            {
                ui.source.toggle_background_force_default_blur();
                state.background_debug.blur_texture_name = ui
                    .source
                    .blur_texture_name(&state.background_debug.map_name, ui.source.is_widescreen());
            }
            ui.source.presentation.background_opacity = Some(state.background_debug.blur_opacity);
            handle_background_swap_keys(state, sim, ui, input.keys);
        }
    } else {
        *fov_repeat = KeyAdjustRepeat::default();
        *opacity_repeat = KeyAdjustRepeat::default();
    }
    if let Some(ui) = ui {
        crate::bevy::host::background::aspect_changed(
            &mut state.background_debug,
            input.aspect,
            &mut ui.source,
        );
    }
}

fn handle_background_swap_keys(
    state: &mut State,
    sim: &mut hl2_bevy::simulation::Simulation,
    ui: &mut hl2_bevy::ui_host::Console,
    keys: &ButtonInput<KeyCode>,
) {
    let slot = if keys.just_pressed(KeyCode::Digit1) || keys.just_pressed(KeyCode::Numpad1) {
        Some(1)
    } else if keys.just_pressed(KeyCode::Digit2) || keys.just_pressed(KeyCode::Numpad2) {
        Some(2)
    } else if keys.just_pressed(KeyCode::Digit3) || keys.just_pressed(KeyCode::Numpad3) {
        Some(3)
    } else if keys.just_pressed(KeyCode::Digit4) || keys.just_pressed(KeyCode::Numpad4) {
        Some(4)
    } else {
        None
    };

    if let Some(s) = slot {
        let target = ui.source.game_and_initial_background_for_slot(s);
        if let Some((game, map)) = target {
            swap_background_map(state, sim, ui, &game, &map);
        }
    } else if keys.just_pressed(KeyCode::BracketLeft) {
        cycle_chapter_background(state, sim, ui, false);
    } else if keys.just_pressed(KeyCode::BracketRight) {
        cycle_chapter_background(state, sim, ui, true);
    }
}

fn cycle_chapter_background(
    state: &mut State,
    sim: &mut hl2_bevy::simulation::Simulation,
    ui: &mut hl2_bevy::ui_host::Console,
    next: bool,
) {
    let list = ui.source.all_chapter_backgrounds();
    if list.is_empty() {
        return;
    }
    let (cur_game, cur_map) = ui.source.current_background_target();
    let pos = list
        .iter()
        .position(|(g, m)| g == &cur_game && m == &cur_map);
    let target_idx = match pos {
        Some(i) => {
            if next {
                (i + 1) % list.len()
            } else if i == 0 {
                list.len() - 1
            } else {
                i - 1
            }
        }
        None => {
            if next {
                0
            } else {
                list.len() - 1
            }
        }
    };
    let (game, map) = &list[target_idx];
    swap_background_map(state, sim, ui, game, map);
}

fn swap_background_map(
    state: &mut State,
    sim: &mut hl2_bevy::simulation::Simulation,
    ui: &mut hl2_bevy::ui_host::Console,
    game: &str,
    map: &str,
) {
    ui.source.swap_background(game, map);
    let is_wide = ui.source.is_widescreen();
    let blur_name = ui.source.blur_texture_name(map, is_wide);
    state.background_debug.map_name = map.to_owned();
    state.background_debug.blur_texture_name = blur_name;
    ui.map_request = Some(map.to_owned());
    ui.map_request_game = Some(game.to_owned());
    ui.background_request = true;
    sim.loading = true;
}
