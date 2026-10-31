//! Developer state, tuning keys and hook behavior, driven through the runtime extension contract.
use super::State;
use super::input::{DebugTuningInput, process_debug_tuning_keys, select_viewmodel_debug_target};
use crate::bevy::input::{
    DebugRepeats, KeyAdjustRepeat, VIEWMODEL_ADJUST_KEYS, adjust_viewmodel_framing,
    reset_viewmodel_debug_weapon, viewmodel_adjust_step,
};
use crate::commands::Request;
use crate::viewmodel::ViewmodelDebug;
use bevy::prelude::{ButtonInput, KeyCode};
use hl2_bevy::extensions::{Context, RuntimeExtension};
use hl2_bevy::simulation::Simulation;
use hl2_bevy::ui_host::Console;
use hl2_ui::console::Mode;
use modkit_core::viewmodel::{ViewmodelFraming, ViewmodelProfile};

const ASPECT: f32 = 16. / 9.;

fn simulation() -> Simulation {
    Simulation::new(
        &modkit_core::World::default(),
        glam::Vec3::ZERO,
        0.,
        0.,
        false,
        vec![],
    )
}

fn console() -> Console {
    Console::new(Default::default())
}

fn request(state: &mut State, sim: &mut Simulation, ui: &mut Console, request: Request) {
    let event = serde_json::to_value(request).unwrap();
    assert!(state.effect(
        &mut Context {
            simulation: sim,
            gameplay: None,
            console: Some(ui)
        },
        &event,
    ));
}

fn tune(
    state: &mut State,
    sim: &mut Simulation,
    ui: &mut Console,
    repeats: &mut DebugRepeats,
    keys: &ButtonInput<KeyCode>,
) {
    let input = DebugTuningInput {
        keys,
        time: Some(0.),
        aspect: ASPECT,
    };
    process_debug_tuning_keys(state, sim, None, Some(ui), input, repeats);
}

fn press(
    state: &mut State,
    sim: &mut Simulation,
    ui: &mut Console,
    repeats: &mut DebugRepeats,
    key: KeyCode,
) {
    let mut keys = ButtonInput::default();
    keys.press(key);
    tune(state, sim, ui, repeats, &keys);
}

fn held_keys(codes: &[KeyCode]) -> ButtonInput<KeyCode> {
    let mut keys = ButtonInput::default();
    for &key in codes {
        keys.press(key);
        keys.clear_just_pressed(key);
    }
    keys
}

/// What the tuning step does the frame after `viewmodel_debug_mode 1`.
fn enable_viewmodel_debug(debug: &mut ViewmodelDebug, weapon: &str, aspect: f32) {
    debug.enabled = true;
    select_viewmodel_debug_target(debug, weapon, hl2_ui::tuning::viewmodel_profile(aspect));
}

#[test]
fn tweak_mode_loads_the_selected_direct_profile() {
    let mut debug = ViewmodelDebug::default();
    enable_viewmodel_debug(&mut debug, "weapon_physcannon", 9. / 16.);
    assert!(debug.enabled);
    assert_eq!(debug.weapon, "weapon_physcannon");
    assert_eq!(debug.profile, Some(ViewmodelProfile::Portrait));
    assert_eq!(
        debug.framing,
        hl2_ui::tuning::viewmodel_profile_framing("weapon_physcannon", ViewmodelProfile::Portrait)
    );

    debug = ViewmodelDebug::default();
    assert!(!debug.enabled);
    assert!(debug.edits.is_empty());
}

#[test]
fn viewmodel_debug_keeps_separate_weapon_and_aspect_edits_until_reset() {
    let mut debug = ViewmodelDebug::default();
    enable_viewmodel_debug(&mut debug, "weapon_physcannon", 9. / 16.);
    debug.framing.fov_offset = 1.25;
    select_viewmodel_debug_target(&mut debug, "weapon_pistol", ViewmodelProfile::Portrait);
    assert_eq!(
        debug.framing,
        hl2_ui::tuning::viewmodel_profile_framing("weapon_pistol", ViewmodelProfile::Portrait)
    );
    debug.framing.offset[2] = -2.75;
    select_viewmodel_debug_target(&mut debug, "weapon_physcannon", ViewmodelProfile::Portrait);
    assert_eq!(debug.framing.fov_offset, 1.25);
    select_viewmodel_debug_target(&mut debug, "weapon_physcannon", ViewmodelProfile::Standard);
    assert_eq!(debug.framing, ViewmodelFraming::default());
    select_viewmodel_debug_target(&mut debug, "weapon_pistol", ViewmodelProfile::Portrait);
    assert_eq!(debug.framing.offset[2], -2.75);

    debug = ViewmodelDebug::default();
    assert!(!debug.enabled);
    assert!(debug.edits.is_empty());
}

#[test]
fn viewmodel_debug_adjustment_keys_use_expected_axes_and_precision() {
    let mut framing = ViewmodelFraming {
        fov_offset: 0.0,
        offset: [0.0, 0.0, -11.5],
    };
    assert!(adjust_viewmodel_framing(
        &mut framing,
        &[
            KeyCode::BracketRight,
            KeyCode::Period,
            KeyCode::ArrowUp,
            KeyCode::ArrowLeft
        ],
        0.1
    ));
    assert_eq!(framing.fov_offset, 0.1);
    assert_eq!(framing.offset, [0.1, 0.1, -11.4]);

    assert!(adjust_viewmodel_framing(
        &mut framing,
        &[
            KeyCode::BracketLeft,
            KeyCode::Comma,
            KeyCode::ArrowDown,
            KeyCode::ArrowRight
        ],
        0.01
    ));
    assert_eq!(framing.fov_offset, 0.09);
    assert_eq!(framing.offset, [0.09, 0.09, -11.41]);
}

#[test]
fn viewmodel_adjust_step_follows_shift_and_alt() {
    assert_eq!(viewmodel_adjust_step(&held_keys(&[])), 0.1);
    assert_eq!(
        viewmodel_adjust_step(&held_keys(&[KeyCode::ShiftRight])),
        1.0
    );
    assert_eq!(viewmodel_adjust_step(&held_keys(&[KeyCode::AltLeft])), 0.01);
    assert_eq!(
        viewmodel_adjust_step(&held_keys(&[KeyCode::ShiftLeft, KeyCode::AltRight])),
        0.01
    );
}

#[test]
fn viewmodel_debug_adjust_keys_repeat_like_options_keyboard_arrows() {
    let mut repeat = KeyAdjustRepeat::default();
    let mut pressed = ButtonInput::default();
    pressed.press(KeyCode::ArrowUp);
    assert_eq!(
        repeat.update(&pressed, Some(0.0), VIEWMODEL_ADJUST_KEYS),
        vec![KeyCode::ArrowUp]
    );

    let held = held_keys(&[KeyCode::ArrowUp]);
    assert!(
        repeat
            .update(&held, Some(0.2), VIEWMODEL_ADJUST_KEYS)
            .is_empty()
    );
    assert_eq!(
        repeat.update(&held, Some(0.4), VIEWMODEL_ADJUST_KEYS),
        vec![KeyCode::ArrowUp]
    );
    assert!(
        repeat
            .update(&held, Some(0.44), VIEWMODEL_ADJUST_KEYS)
            .is_empty()
    );
    assert_eq!(
        repeat.update(&held, Some(0.45), VIEWMODEL_ADJUST_KEYS),
        vec![KeyCode::ArrowUp]
    );

    let mut both = held.clone();
    both.press(KeyCode::ArrowLeft);
    assert_eq!(
        repeat.update(&both, Some(0.5), VIEWMODEL_ADJUST_KEYS),
        vec![KeyCode::ArrowLeft]
    );
    both.clear_just_pressed(KeyCode::ArrowLeft);
    assert_eq!(
        repeat.update(&both, Some(0.9), VIEWMODEL_ADJUST_KEYS),
        vec![KeyCode::ArrowLeft]
    );
    assert!(
        repeat
            .update(&held, Some(1.0), VIEWMODEL_ADJUST_KEYS)
            .is_empty()
    );
}

#[test]
fn reset_zeroes_the_selected_weapon_profile_to_zero_offsets() {
    let mut debug = ViewmodelDebug::default();
    enable_viewmodel_debug(&mut debug, "weapon_physcannon", 9. / 16.);
    debug.framing.fov_offset = 1.5;
    debug.framing.offset = [2.0, -3.0, -4.0];
    debug.edits.insert(
        ("weapon_pistol".into(), ViewmodelProfile::Portrait),
        ViewmodelFraming {
            fov_offset: 8.0,
            offset: [0.0, 0.0, -9.0],
        },
    );
    assert!(reset_viewmodel_debug_weapon(&mut debug));
    assert_eq!(debug.framing, ViewmodelFraming::ZERO);
    debug.edits.insert(
        (debug.weapon.clone(), debug.profile.unwrap()),
        debug.framing,
    );
    assert!(!reset_viewmodel_debug_weapon(&mut debug));
    select_viewmodel_debug_target(&mut debug, "weapon_pistol", ViewmodelProfile::Portrait);
    assert_eq!(debug.framing.fov_offset, 8.0);
    select_viewmodel_debug_target(&mut debug, "weapon_physcannon", ViewmodelProfile::Portrait);
    assert_eq!(debug.framing, ViewmodelFraming::ZERO);
}

#[test]
fn background_debug_keys_adjust_fov_and_opacity_in_paused_main_menu() {
    let (mut state, mut sim, mut ui) = (State::default(), simulation(), console());
    let mut repeats = DebugRepeats::default();
    sim.background_level = true;
    sim.paused = true;
    state.background_debug.enabled = true;
    assert!(sim.paused());
    ui.source.presentation.capture_menu_navigation = true;
    let mut key = |state: &mut State, sim: &mut Simulation, ui: &mut Console, code: KeyCode| {
        press(state, sim, ui, &mut repeats, code);
    };

    // Period raises opacity by 1%.
    key(&mut state, &mut sim, &mut ui, KeyCode::Period);
    assert_eq!(state.background_debug.blur_opacity, 51);
    assert_eq!(ui.source.presentation.background_opacity, Some(51));

    // Quick background swaps.
    for (code, map, game) in [
        (KeyCode::Digit2, "ep1_background01", "episodic"),
        (KeyCode::Digit3, "ep2_background01", "ep2"),
        (KeyCode::Digit4, "background01", "lostcoast"),
        (KeyCode::Digit1, "background01", "hl2"),
    ] {
        key(&mut state, &mut sim, &mut ui, code);
        assert_eq!(ui.map_request.as_deref(), Some(map));
        assert_eq!(ui.map_request_game.as_deref(), Some(game));
        assert_eq!(state.background_debug.map_name, map);
    }

    // Brackets rotate through chapter backgrounds, wrapping at both ends, and leave opacity alone.
    key(&mut state, &mut sim, &mut ui, KeyCode::BracketLeft);
    assert_eq!(ui.map_request.as_deref(), Some("background01"));
    assert_eq!(ui.map_request_game.as_deref(), Some("lostcoast"));
    key(&mut state, &mut sim, &mut ui, KeyCode::BracketRight);
    assert_eq!(ui.map_request.as_deref(), Some("background01"));
    assert_eq!(ui.map_request_game.as_deref(), Some("hl2"));
    assert_eq!(state.background_debug.blur_opacity, 51);

    // Comma lowers opacity by 1%.
    key(&mut state, &mut sim, &mut ui, KeyCode::Comma);
    assert_eq!(state.background_debug.blur_opacity, 50);
    assert_eq!(ui.source.presentation.background_opacity, Some(50));

    // Left toggles 0, right toggles 100.
    for (code, expected) in [
        (KeyCode::ArrowLeft, 0),
        (KeyCode::ArrowLeft, 50),
        (KeyCode::ArrowRight, 100),
        (KeyCode::ArrowRight, 50),
    ] {
        key(&mut state, &mut sim, &mut ui, code);
        assert_eq!(state.background_debug.blur_opacity, expected);
    }

    // Up and down adjust FOV, 0 resets it.
    key(&mut state, &mut sim, &mut ui, KeyCode::ArrowUp);
    let raised = state.background_debug.fov.unwrap();
    key(&mut state, &mut sim, &mut ui, KeyCode::ArrowDown);
    assert!(state.background_debug.fov.unwrap() < raised);
    key(&mut state, &mut sim, &mut ui, KeyCode::Digit0);
    assert_eq!(state.background_debug.fov, None);

    // The open console keeps the keys.
    ui.source.set_mode(Mode::Console);
    key(&mut state, &mut sim, &mut ui, KeyCode::Comma);
    assert_eq!(state.background_debug.blur_opacity, 50);
}

#[test]
fn minus_toggles_once_per_press_only_in_background_debug_outside_console() {
    let (mut state, mut sim, mut ui) = (State::default(), simulation(), console());
    let mut repeats = DebugRepeats::default();
    sim.background_level = true;
    state.background_debug.enabled = true;
    ui.source.canvas.resize(1280., 720.);
    ui.source.presentation.capture_menu_navigation = true;
    let mut keys = ButtonInput::default();
    keys.press(KeyCode::Minus);

    tune(&mut state, &mut sim, &mut ui, &mut repeats, &keys);
    assert!(ui.source.config.video.background_force_default_blur);
    assert_eq!(state.background_debug.blur_texture_name, "background01");
    keys.clear();
    assert!(keys.pressed(KeyCode::Minus));
    tune(&mut state, &mut sim, &mut ui, &mut repeats, &keys);
    assert!(
        ui.source.config.video.background_force_default_blur,
        "held key must not repeat"
    );
    keys.release(KeyCode::Minus);
    keys.press(KeyCode::Minus);
    tune(&mut state, &mut sim, &mut ui, &mut repeats, &keys);
    assert!(!ui.source.config.video.background_force_default_blur);
    assert_eq!(
        state.background_debug.blur_texture_name,
        "background01_widescreen"
    );
    ui.source.set_mode(Mode::Console);
    keys.release(KeyCode::Minus);
    keys.press(KeyCode::Minus);
    tune(&mut state, &mut sim, &mut ui, &mut repeats, &keys);
    assert!(!ui.source.config.video.background_force_default_blur);
    ui.source.set_mode(Mode::MainMenu);
    state.background_debug.enabled = false;
    tune(&mut state, &mut sim, &mut ui, &mut repeats, &keys);
    assert!(!ui.source.config.video.background_force_default_blur);
}

#[test]
fn viewmodel_tweak_commands_follow_the_gameplay_map_state() {
    let (mut state, mut sim, mut ui) = (State::default(), simulation(), console());

    request(&mut state, &mut sim, &mut ui, Request::ViewmodelTweakReset);
    assert!(!state.viewmodel_debug.reset_pending);
    sim.background_level = true;
    request(
        &mut state,
        &mut sim,
        &mut ui,
        Request::ViewmodelDebugMode(true),
    );
    assert!(!state.viewmodel_debug.enabled);
    sim.background_level = false;
    sim.loading = true;
    request(
        &mut state,
        &mut sim,
        &mut ui,
        Request::ViewmodelDebugMode(true),
    );
    assert!(!state.viewmodel_debug.enabled);
    sim.loading = false;
    request(
        &mut state,
        &mut sim,
        &mut ui,
        Request::ViewmodelDebugMode(true),
    );
    assert!(state.viewmodel_debug.enabled);
    request(&mut state, &mut sim, &mut ui, Request::ViewmodelTweakReset);
    assert!(state.viewmodel_debug.reset_pending);
    state.map_changed(&mut sim);
    assert!(!state.viewmodel_debug.enabled && !state.viewmodel_debug.reset_pending);
    request(
        &mut state,
        &mut sim,
        &mut ui,
        Request::ViewmodelDebugMode(true),
    );
    request(
        &mut state,
        &mut sim,
        &mut ui,
        Request::ViewmodelDebugMode(false),
    );
    assert!(!state.viewmodel_debug.enabled);
}

#[test]
fn background_and_viewmodel_debug_modes_mutually_exclusive() {
    let (mut state, mut sim, mut ui) = (State::default(), simulation(), console());

    sim.background_level = true;
    request(
        &mut state,
        &mut sim,
        &mut ui,
        Request::BackgroundDebugMode(true),
    );
    assert!(state.background_debug.enabled);
    assert!(!state.viewmodel_debug.enabled);

    sim.background_level = false;
    request(
        &mut state,
        &mut sim,
        &mut ui,
        Request::ViewmodelDebugMode(true),
    );
    assert!(state.viewmodel_debug.enabled);
    assert!(!state.background_debug.enabled);

    sim.background_level = true;
    request(
        &mut state,
        &mut sim,
        &mut ui,
        Request::BackgroundDebugMode(true),
    );
    assert!(state.background_debug.enabled);
    assert!(!state.viewmodel_debug.enabled);
}

#[test]
fn artwork_changes_reset_temporary_fov_and_other_config_updates_keep_it() {
    let (mut state, mut sim, mut ui) = (State::default(), simulation(), console());
    let apply = |state: &mut State, force: bool, ui: &mut Console, sim: &mut Simulation| {
        let mut config = ui.source.config.clone();
        config.video.background_force_default_blur = force;
        sim.background_force_default_blur = force;
        state.config_changed(
            &mut Context {
                simulation: sim,
                gameplay: None,
                console: Some(ui),
            },
            &config,
        );
    };

    state.background_debug.fov = Some(65.);
    apply(&mut state, false, &mut ui, &mut sim);
    assert_eq!(
        state.background_debug.fov,
        Some(65.),
        "unrelated config update"
    );
    for force in [true, false, true, false] {
        state.background_debug.fov = Some(70.);
        apply(&mut state, force, &mut ui, &mut sim);
        assert_eq!(state.background_debug.fov, None, "artwork change");
        let expected = hl2_bevy::camera::background_fov(ASPECT, force).to_degrees();
        assert_eq!(state.background_debug.fov(expected), expected);
        state
            .background_debug
            .adjust_fov(&[KeyCode::ArrowUp], 1., expected);
        assert!((state.background_debug.fov(expected) - expected - 1.).abs() < 0.02);
    }
}

#[test]
fn forced_resize_uses_the_normal_projection_baseline() {
    let mut state = State::default();
    let mut ui = console();
    ui.source.config.video.background_force_default_blur = true;
    state.background_debug.enabled = true;
    state.background_debug.last_aspect = Some(4. / 3.);
    crate::bevy::host::background::aspect_changed(
        &mut state.background_debug,
        32. / 9.,
        "32:9",
        &mut ui.source,
    );
    let expected = hl2_bevy::camera::background_fov(32. / 9., true).to_degrees();
    assert_eq!(state.background_debug.fov, Some(expected));
    assert!(state.background_debug.lines(expected, true)[4].contains(&format!("{expected:.2}")));
}

#[test]
fn developer_shortcuts_follow_gameplay_input_gates() {
    let (mut state, mut sim) = (State::default(), simulation());
    let keys = ButtonInput::default();
    let buttons = ButtonInput::default();
    let input = |cursor_grabbed| hl2_bevy::extensions::Input {
        keys: &keys,
        buttons: &buttons,
        focused: true,
        cursor_grabbed,
        aspect: ASPECT,
        time: Some(0.),
    };
    assert!(state.shortcuts_allowed(&sim, &input(true)));
    assert!(!state.shortcuts_allowed(&sim, &input(false)), "free cursor");
    state.overlay.open = true;
    assert!(
        state.shortcuts_allowed(&sim, &input(false)),
        "F4 frees the cursor"
    );
    state.overlay.typing = true;
    assert!(!state.shortcuts_allowed(&sim, &input(true)), "typing");
    state.overlay.typing = false;
    sim.paused = true;
    assert!(!state.shortcuts_allowed(&sim, &input(true)), "paused");
    sim.paused = false;
    sim.transition = true;
    assert!(!state.shortcuts_allowed(&sim, &input(true)), "transition");
}

#[test]
fn the_saved_artwork_choice_does_not_count_as_a_change_at_startup() {
    let (mut state, mut sim, mut ui) = (State::default(), simulation(), console());
    let mut config = ui.source.config.clone();
    config.video.background_force_default_blur = true;
    state.startup(&config);
    state.background_debug.fov = Some(65.);
    state.config_changed(
        &mut Context {
            simulation: &mut sim,
            gameplay: None,
            console: Some(&mut ui),
        },
        &config,
    );
    assert_eq!(state.background_debug.fov, Some(65.));
}

#[test]
fn unknown_script_events_are_not_claimed() {
    let (mut state, mut sim) = (State::default(), simulation());
    assert!(state.script(&mut sim, &serde_json::json!({"developer_overlay": true})));
    assert!(state.dev_overlay);
    assert!(!state.script(&mut sim, &serde_json::json!({"dev_overlay": true})));
    assert!(!state.script(&mut sim, &serde_json::json!({"developer_overlay": "yes"})));
}
