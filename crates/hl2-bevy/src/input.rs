//! Bevy event adaptation for gameplay and character input.
//!
//! UI event handling lives in `ui`; this module only turns an already captured
//! keyboard/mouse frame into gameplay actions and a Source movement intent.
//!
//! Every gameplay action is read through its persisted binding (`Config::keys`): keyboard keys use
//! Bevy's `KeyCode` names, mouse buttons `MOUSE1`..`MOUSE5` and the wheel `MWHEELUP`/`MWHEELDOWN`,
//! exactly as the Options key capture writes them. An empty or unknown binding is unbound.

use std::{collections::HashMap, sync::LazyLock};

use bevy::{
    input::mouse::AccumulatedMouseMotion,
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};
use hl2_ui::{config::Config, tuning::ViewmodelFraming};
use modkit_core::movement::Input;
use source_ui::widgets::HoldRepeat;

type InputDevices<'w> = (
    Res<'w, ButtonInput<KeyCode>>,
    Res<'w, ButtonInput<MouseButton>>,
    Res<'w, AccumulatedMouseMotion>,
    Option<Res<'w, bevy::input::mouse::AccumulatedMouseScroll>>,
    Option<Res<'w, crate::video::GameFrame>>,
);

/// A physical input a binding name refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Key(KeyCode),
    Mouse(MouseButton),
    WheelUp,
    WheelDown,
}

/// Keys a binding may name. Binding names are the `Debug` spelling of `KeyCode`, which is what
/// the Options key capture stores, so the lookup table is generated from that spelling.
const BINDABLE_KEYS: &[KeyCode] = &[
    KeyCode::KeyA,
    KeyCode::KeyB,
    KeyCode::KeyC,
    KeyCode::KeyD,
    KeyCode::KeyE,
    KeyCode::KeyF,
    KeyCode::KeyG,
    KeyCode::KeyH,
    KeyCode::KeyI,
    KeyCode::KeyJ,
    KeyCode::KeyK,
    KeyCode::KeyL,
    KeyCode::KeyM,
    KeyCode::KeyN,
    KeyCode::KeyO,
    KeyCode::KeyP,
    KeyCode::KeyQ,
    KeyCode::KeyR,
    KeyCode::KeyS,
    KeyCode::KeyT,
    KeyCode::KeyU,
    KeyCode::KeyV,
    KeyCode::KeyW,
    KeyCode::KeyX,
    KeyCode::KeyY,
    KeyCode::KeyZ,
    KeyCode::Digit0,
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
    KeyCode::F1,
    KeyCode::F2,
    KeyCode::F3,
    KeyCode::F4,
    KeyCode::F5,
    KeyCode::F6,
    KeyCode::F7,
    KeyCode::F8,
    KeyCode::F9,
    KeyCode::F10,
    KeyCode::F11,
    KeyCode::F12,
    KeyCode::F13,
    KeyCode::F14,
    KeyCode::F15,
    KeyCode::F16,
    KeyCode::F17,
    KeyCode::F18,
    KeyCode::F19,
    KeyCode::F20,
    KeyCode::F21,
    KeyCode::F22,
    KeyCode::F23,
    KeyCode::F24,
    KeyCode::Space,
    KeyCode::ControlLeft,
    KeyCode::ControlRight,
    KeyCode::ShiftLeft,
    KeyCode::ShiftRight,
    KeyCode::AltLeft,
    KeyCode::AltRight,
    KeyCode::SuperLeft,
    KeyCode::SuperRight,
    KeyCode::Enter,
    KeyCode::Escape,
    KeyCode::Tab,
    KeyCode::Backspace,
    KeyCode::CapsLock,
    KeyCode::ArrowUp,
    KeyCode::ArrowDown,
    KeyCode::ArrowLeft,
    KeyCode::ArrowRight,
    KeyCode::Insert,
    KeyCode::Delete,
    KeyCode::Home,
    KeyCode::End,
    KeyCode::PageUp,
    KeyCode::PageDown,
    KeyCode::Pause,
    KeyCode::NumLock,
    KeyCode::ScrollLock,
    KeyCode::PrintScreen,
    KeyCode::ContextMenu,
    KeyCode::Backquote,
    KeyCode::Minus,
    KeyCode::Equal,
    KeyCode::BracketLeft,
    KeyCode::BracketRight,
    KeyCode::Backslash,
    KeyCode::IntlBackslash,
    KeyCode::Semicolon,
    KeyCode::Quote,
    KeyCode::Comma,
    KeyCode::Period,
    KeyCode::Slash,
    KeyCode::Numpad0,
    KeyCode::Numpad1,
    KeyCode::Numpad2,
    KeyCode::Numpad3,
    KeyCode::Numpad4,
    KeyCode::Numpad5,
    KeyCode::Numpad6,
    KeyCode::Numpad7,
    KeyCode::Numpad8,
    KeyCode::Numpad9,
    KeyCode::NumpadEnter,
    KeyCode::NumpadAdd,
    KeyCode::NumpadSubtract,
    KeyCode::NumpadMultiply,
    KeyCode::NumpadDivide,
    KeyCode::NumpadDecimal,
    KeyCode::NumpadEqual,
];

static KEYS_BY_NAME: LazyLock<HashMap<String, KeyCode>> = LazyLock::new(|| {
    BINDABLE_KEYS
        .iter()
        .map(|key| (format!("{key:?}"), *key))
        .collect()
});

/// Defaults used when no UI configuration exists (headless or tool runs).
static DEFAULTS: LazyLock<Config> = LazyLock::new(Config::default);

/// Adapts a persisted binding name to a Bevy input. `""` (explicitly unbound) and names this
/// host cannot press (joystick buttons, for example) return `None`.
pub fn parse_binding(name: &str) -> Option<Trigger> {
    let name = name.trim();
    let mouse = match name {
        "MOUSE1" => Some(MouseButton::Left),
        "MOUSE2" => Some(MouseButton::Right),
        "MOUSE3" => Some(MouseButton::Middle),
        "MOUSE4" => Some(MouseButton::Back),
        "MOUSE5" => Some(MouseButton::Forward),
        _ => None,
    };
    if let Some(button) = mouse {
        return Some(Trigger::Mouse(button));
    }
    match name {
        "" => None,
        "MWHEELUP" => Some(Trigger::WheelUp),
        "MWHEELDOWN" => Some(Trigger::WheelDown),
        // Spellings written by older configuration files.
        "Ctrl" => Some(Trigger::Key(KeyCode::ControlLeft)),
        "Shift" => Some(Trigger::Key(KeyCode::ShiftLeft)),
        "Alt" => Some(Trigger::Key(KeyCode::AltLeft)),
        "Up" => Some(Trigger::Key(KeyCode::ArrowUp)),
        "Down" => Some(Trigger::Key(KeyCode::ArrowDown)),
        "Left" => Some(Trigger::Key(KeyCode::ArrowLeft)),
        "Right" => Some(Trigger::Key(KeyCode::ArrowRight)),
        _ => KEYS_BY_NAME.get(name).copied().map(Trigger::Key),
    }
}

/// The input bound to `action`: the configured binding, or the default binding when the
/// configuration has no entry for it.
/// The weapon slot commands, slot 0 first.
const SLOTS: [&str; 6] = ["slot1", "slot2", "slot3", "slot4", "slot5", "slot6"];

fn trigger(action: &str, config: Option<&Config>) -> Option<Trigger> {
    let default = DEFAULTS.keys.get(action).map_or("", String::as_str);
    parse_binding(hl2_ui::input::binding(config, action, default))
}

/// One frame of keyboard, mouse-button and wheel state.
struct Devices<'a> {
    keys: &'a ButtonInput<KeyCode>,
    buttons: &'a ButtonInput<MouseButton>,
    /// Vertical wheel movement this frame; positive is away from the user (`MWHEELUP`).
    wheel: f32,
}

impl Devices<'_> {
    fn pressed(&self, action: &str, config: Option<&Config>) -> bool {
        match trigger(action, config) {
            Some(Trigger::Key(key)) => self.keys.pressed(key),
            Some(Trigger::Mouse(button)) => self.buttons.pressed(button),
            Some(trigger) => self.wheel_moved(trigger),
            None => false,
        }
    }

    fn just_pressed(&self, action: &str, config: Option<&Config>) -> bool {
        match trigger(action, config) {
            Some(Trigger::Key(key)) => self.keys.just_pressed(key),
            Some(Trigger::Mouse(button)) => self.buttons.just_pressed(button),
            Some(trigger) => self.wheel_moved(trigger),
            None => false,
        }
    }

    /// A wheel step is a single-frame press, like Source's `MWHEELUP`/`MWHEELDOWN` key events.
    fn wheel_moved(&self, trigger: Trigger) -> bool {
        match trigger {
            Trigger::WheelUp => self.wheel > 0.,
            Trigger::WheelDown => self.wheel < 0.,
            _ => false,
        }
    }
}

/// Whether the input bound to a held action is down (keyboard and mouse buttons).
pub(crate) fn action_pressed(
    action: &str,
    keys: &ButtonInput<KeyCode>,
    buttons: &ButtonInput<MouseButton>,
    config: Option<&Config>,
) -> bool {
    Devices {
        keys,
        buttons,
        wheel: 0.,
    }
    .pressed(action, config)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn controls(
    (keys, buttons, mouse, scroll, frame): InputDevices,
    windows: Query<(&Window, &CursorOptions), With<PrimaryWindow>>,
    mut sim: ResMut<crate::simulation::Simulation>,
    mut game: Option<ResMut<crate::gameplay::Gameplay>>,
    mut ui: Option<ResMut<crate::ui_host::Console>>,
    mut mouse_filter: Local<hl2_ui::input::MouseFilter>,
    mut repeats: Local<DebugRepeats>,
    real_time: Option<Res<Time<Real>>>,
) {
    // CGameRules::Think reads `skill` every frame, and weapon selection reads
    // `hud_fastswitch` on each key, so changed settings apply at once.
    if let (Some(game), Some(ui)) = (game.as_deref_mut(), ui.as_deref()) {
        game.set_skill(ui.source.config.gameplay.skill);
        game.selection.fast_switch = ui.source.config.keyboard.fast_switch;
    }
    let Ok((window, cursor)) = windows.single() else {
        return;
    };
    if keys.just_pressed(KeyCode::F1) && window.focused {
        sim.dev_overlay = !sim.dev_overlay;
    }
    let aspect = crate::video::frame_aspect(frame.as_deref());
    let time = real_time.as_deref().map(Time::elapsed_secs_f64);
    let tuning_input = DebugTuningInput {
        keys: &keys,
        time,
        aspect,
    };
    process_debug_tuning_keys(
        &mut sim,
        game.as_deref(),
        ui.as_deref_mut(),
        tuning_input,
        &mut repeats,
    );
    if !sim.commands.is_empty() {
        return;
    }
    let overlay_open = game.as_ref().is_some_and(|g| g.spawn_overlay.open);
    let typing = game.as_ref().is_some_and(|g| g.spawn_overlay.typing);
    let cfg = ui.as_ref().map(|u| &u.source.config);
    let devices = Devices {
        keys: &keys,
        buttons: &buttons,
        wheel: scroll.as_deref().map_or(0., |scroll| scroll.delta.y),
    };
    if !devices.pressed("+jump", cfg) {
        sim.jump_suppressed = false;
    }
    if let Some(game) = game.as_deref_mut() {
        game.buttons(
            devices.pressed("+attack", cfg) && !overlay_open,
            devices.pressed("+attack2", cfg) && !overlay_open,
        );
    }
    sim.input = Input::default();
    if sim.paused() || sim.transition || (cursor.grab_mode == CursorGrabMode::None && !overlay_open)
    {
        if let Some(game) = game.as_deref_mut() {
            game.consume_attacks();
        }
        return;
    }
    if let Some(game) = game.as_deref_mut()
        && !typing
    {
        use crate::gameplay::Action;
        for (name, action) in [
            ("loadout", Action::Loadout),
            ("+use", Action::Use),
            ("+reload", Action::Reload),
            ("lastinv", Action::Previous),
            ("impulse", Action::Impulse),
            ("invprev", Action::Wheel { delta: -1 }),
            ("invnext", Action::Wheel { delta: 1 }),
        ] {
            if devices.just_pressed(name, cfg) {
                game.actions.push(action);
            }
        }
        let pressed = |(_, name): &(usize, &str)| devices.just_pressed(name, cfg);
        let slots = SLOTS.iter().copied().enumerate().filter(pressed);
        game.actions
            .extend(slots.map(|(slot, _)| Action::Slot { slot }));
        sim.flashlight_toggles += u32::from(devices.just_pressed("impulse 100", cfg));
    }
    if typing {
        return;
    }
    if !overlay_open || buttons.pressed(MouseButton::Right) {
        let settings = cfg.map_or(&DEFAULTS.mouse, |c| &c.mouse);
        let [dx, dy] = mouse_filter.apply([mouse.delta.x, mouse.delta.y], settings);
        let radians = hl2_ui::input::MOUSE_DEGREES_PER_COUNT.to_radians();
        sim.yaw -= dx * radians;
        let pitch_sign = if settings.invert { -1.0 } else { 1.0 };
        sim.pitch = (sim.pitch - dy * radians * pitch_sign).clamp(-1.55, 1.55);
    }
    if keys.just_pressed(KeyCode::F2) {
        let fly = !sim.flying();
        sim.set_fly(fly);
    }
    sim.input = Input {
        forward: f32::from(devices.pressed("+forward", cfg))
            - f32::from(devices.pressed("+back", cfg)),
        side: f32::from(devices.pressed("+moveright", cfg))
            - f32::from(devices.pressed("+moveleft", cfg)),
        yaw: sim.yaw,
        jump: devices.pressed("+jump", cfg) && !sim.jump_suppressed,
        crouch: devices.pressed("+duck", cfg),
        sprint: devices.pressed("+speed", cfg),
        slow: devices.pressed("+walk", cfg),
        // max_speed and the ground surface are set per fixed step by the simulation.
        ..Input::default()
    };
}

fn select_viewmodel_debug_target(
    debug: &mut crate::simulation::ViewmodelDebug,
    weapon: &str,
    profile: hl2_ui::tuning::ViewmodelProfile,
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
        .unwrap_or_else(|| hl2_ui::tuning::viewmodel_profile_framing(weapon, profile));
}

const VIEWMODEL_ADJUST_KEYS: &[KeyCode] = &[
    KeyCode::BracketLeft,
    KeyCode::BracketRight,
    KeyCode::Comma,
    KeyCode::Period,
    KeyCode::ArrowUp,
    KeyCode::ArrowDown,
    KeyCode::ArrowLeft,
    KeyCode::ArrowRight,
];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct DebugRepeats {
    pub vm: KeyAdjustRepeat,
    pub bg_fov: KeyAdjustRepeat,
    pub bg_opacity: KeyAdjustRepeat,
}

/// Latest adjust key repeats while held, using the same [`HoldRepeat`] timing as
/// Options > Keyboard list arrows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct KeyAdjustRepeat {
    key: Option<KeyCode>,
    timer: HoldRepeat,
}

impl KeyAdjustRepeat {
    pub(crate) fn update(
        &mut self,
        keys: &ButtonInput<KeyCode>,
        time: Option<f64>,
        watch_keys: &[KeyCode],
    ) -> Vec<KeyCode> {
        let pressed: Vec<KeyCode> = watch_keys
            .iter()
            .copied()
            .filter(|key| keys.just_pressed(*key))
            .collect();
        if let Some(&key) = pressed.last() {
            self.key = Some(key);
            self.timer.press(time);
            return pressed;
        }
        if let Some(key) = self.key {
            if !keys.pressed(key) {
                *self = Self::default();
            } else if self.timer.held(time) {
                return vec![key];
            }
        }
        Vec::new()
    }
}

fn commit_viewmodel_debug_edit(
    debug: &mut crate::simulation::ViewmodelDebug,
    weapon: String,
    profile: hl2_ui::tuning::ViewmodelProfile,
    ui: Option<&mut crate::ui_host::Console>,
) {
    debug.edits.insert((weapon, profile), debug.framing);
    match hl2_ui::tuning::tuning_json_with_viewmodel_profiles(&debug.edits) {
        Ok(json) => {
            if let Some(ui) = ui {
                ui.source.log_viewmodel_tuning(json);
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

struct DebugTuningInput<'a> {
    keys: &'a ButtonInput<KeyCode>,
    time: Option<f64>,
    aspect: f32,
}

fn process_debug_tuning_keys(
    sim: &mut crate::simulation::Simulation,
    game: Option<&crate::gameplay::Gameplay>,
    mut ui: Option<&mut crate::ui_host::Console>,
    input: DebugTuningInput,
    repeats: &mut DebugRepeats,
) {
    if sim.loading() || sim.background_level {
        sim.viewmodel_debug = crate::simulation::ViewmodelDebug::default();
    }
    let in_gameplay = !sim.background_level
        && !sim.loading()
        && ui
            .as_ref()
            .is_some_and(|u| !u.source.can_enable_background_debug());
    if in_gameplay {
        if sim.background_debug.enabled {
            sim.background_debug.fov = None;
        }
        sim.background_debug.enabled = false;
        if let Some(ref mut ui) = ui {
            ui.source.background_debug_mode = false;
        }
    }
    if sim.viewmodel_debug.enabled {
        let Some(weapon) = game.map(|g| g.inventory.active.clone()) else {
            sim.viewmodel_debug = crate::simulation::ViewmodelDebug::default();
            repeats.vm = KeyAdjustRepeat::default();
            return;
        };
        let profile = hl2_ui::tuning::viewmodel_profile(input.aspect);
        select_viewmodel_debug_target(&mut sim.viewmodel_debug, &weapon, profile);
        let adjusting = !sim.paused() && !sim.transition;
        let changed = if std::mem::take(&mut sim.viewmodel_debug.reset_pending) {
            reset_viewmodel_debug_weapon(&mut sim.viewmodel_debug)
        } else if adjusting {
            let fired = repeats
                .vm
                .update(input.keys, input.time, VIEWMODEL_ADJUST_KEYS);
            let step = viewmodel_adjust_step(input.keys);
            adjust_viewmodel_framing(&mut sim.viewmodel_debug.framing, &fired, step)
        } else {
            repeats.vm = KeyAdjustRepeat::default();
            false
        };
        if changed {
            commit_viewmodel_debug_edit(
                &mut sim.viewmodel_debug,
                weapon,
                profile,
                ui.as_deref_mut(),
            );
        }
    } else {
        repeats.vm = KeyAdjustRepeat::default();
    }
    if sim.background_debug.enabled {
        process_background_debug_keys(sim, ui, input, &mut repeats.bg_fov, &mut repeats.bg_opacity);
    } else {
        repeats.bg_fov = KeyAdjustRepeat::default();
        repeats.bg_opacity = KeyAdjustRepeat::default();
    }
}

fn process_background_debug_keys(
    sim: &mut crate::simulation::Simulation,
    mut ui: Option<&mut crate::ui_host::Console>,
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
            crate::background_debug::BACKGROUND_ADJUST_KEYS,
        );
        let step = viewmodel_adjust_step(input.keys);
        sim.background_debug.adjust_fov(&fired, step, input.aspect);

        let opacity_fired = opacity_repeat.update(
            input.keys,
            input.time,
            crate::background_debug::BACKGROUND_OPACITY_KEYS,
        );
        sim.background_debug.adjust_opacity_step(&opacity_fired);

        if input.keys.just_pressed(KeyCode::ArrowLeft) {
            sim.background_debug.toggle_zero_opacity();
        }
        if input.keys.just_pressed(KeyCode::ArrowRight) {
            sim.background_debug.toggle_full_opacity();
        }
        if input.keys.just_pressed(KeyCode::Digit0) || input.keys.just_pressed(KeyCode::Numpad0) {
            sim.background_debug.fov = None;
        }
        if let Some(ref mut ui) = ui {
            ui.source.background_blur_opacity = sim.background_debug.blur_opacity;
            handle_background_swap_keys(sim, ui, input.keys);
        }
    } else {
        *fov_repeat = KeyAdjustRepeat::default();
        *opacity_repeat = KeyAdjustRepeat::default();
    }
    if let Some(ui) = ui {
        let is_flexible = ui.source.config.video.flexible_aspect;
        let label = crate::background_debug::aspect_label(input.aspect, is_flexible);
        sim.background_debug
            .check_aspect_change(input.aspect, label, &mut ui.source);
    }
}

fn handle_background_swap_keys(
    sim: &mut crate::simulation::Simulation,
    ui: &mut crate::ui_host::Console,
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
            swap_background_map(sim, ui, &game, &map);
        }
    } else if keys.just_pressed(KeyCode::BracketLeft) {
        cycle_chapter_background(sim, ui, false);
    } else if keys.just_pressed(KeyCode::BracketRight) {
        cycle_chapter_background(sim, ui, true);
    }
}

fn cycle_chapter_background(
    sim: &mut crate::simulation::Simulation,
    ui: &mut crate::ui_host::Console,
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
    swap_background_map(sim, ui, game, map);
}

fn swap_background_map(
    sim: &mut crate::simulation::Simulation,
    ui: &mut crate::ui_host::Console,
    game: &str,
    map: &str,
) {
    ui.source.swap_background(game, map);
    let is_wide = ui.source.is_widescreen();
    let blur_name = ui.source.blur_texture_name(map, is_wide);
    sim.background_debug.map_name = map.to_owned();
    sim.background_debug.blur_texture_name = blur_name;
    ui.map_request = Some(map.to_owned());
    ui.map_request_game = Some(game.to_owned());
    ui.background_request = true;
    sim.loading = true;
}

/// Zero FOV and position offsets for the selected weapon's current aspect profile.
fn reset_viewmodel_debug_weapon(debug: &mut crate::simulation::ViewmodelDebug) -> bool {
    if debug.framing == ViewmodelFraming::ZERO {
        return false;
    }
    debug.framing = ViewmodelFraming::ZERO;
    true
}

/// Step per viewmodel adjust key press: 1.0 with Shift, 0.01 with Alt/Option (which wins over
/// Shift), else 0.1.
fn viewmodel_adjust_step(keys: &ButtonInput<KeyCode>) -> f32 {
    if keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]) {
        0.01
    } else if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
        1.0
    } else {
        0.1
    }
}

/// Applies one-frame viewmodel debug key presses. Returns whether the profile changed.
fn adjust_viewmodel_framing(framing: &mut ViewmodelFraming, fired: &[KeyCode], step: f32) -> bool {
    let mut changed = false;
    let mut apply = |value: &mut f32, delta: f32| {
        *value = ((*value + delta) * 100.0).round() / 100.0;
        changed = true;
    };
    for key in fired {
        match *key {
            KeyCode::BracketLeft => apply(&mut framing.fov_offset, -step),
            KeyCode::BracketRight => apply(&mut framing.fov_offset, step),
            KeyCode::Comma => apply(&mut framing.offset[0], -step),
            KeyCode::Period => apply(&mut framing.offset[0], step),
            KeyCode::ArrowUp => apply(&mut framing.offset[2], step),
            KeyCode::ArrowDown => apply(&mut framing.offset[2], -step),
            KeyCode::ArrowLeft => apply(&mut framing.offset[1], step),
            KeyCode::ArrowRight => apply(&mut framing.offset[1], -step),
            _ => {}
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use source_ui::keys::NAMED_KEYS;

    /// What `controls` does the frame after `viewmodel_tweak_mode 1`.
    fn enable_viewmodel_debug(
        debug: &mut crate::simulation::ViewmodelDebug,
        weapon: &str,
        aspect: f32,
    ) {
        debug.enabled = true;
        select_viewmodel_debug_target(debug, weapon, hl2_ui::tuning::viewmodel_profile(aspect));
    }

    #[test]
    fn tweak_mode_loads_the_selected_direct_profile() {
        let mut debug = crate::simulation::ViewmodelDebug::default();
        enable_viewmodel_debug(&mut debug, "weapon_physcannon", 9. / 16.);
        assert!(debug.enabled);
        assert_eq!(debug.weapon, "weapon_physcannon");
        assert_eq!(
            debug.profile,
            Some(hl2_ui::tuning::ViewmodelProfile::Portrait)
        );
        assert_eq!(
            debug.framing,
            hl2_ui::tuning::viewmodel_profile_framing(
                "weapon_physcannon",
                hl2_ui::tuning::ViewmodelProfile::Portrait
            )
        );

        debug = crate::simulation::ViewmodelDebug::default();
        assert!(!debug.enabled);
        assert!(debug.edits.is_empty());
    }

    #[test]
    fn viewmodel_debug_keeps_separate_weapon_and_aspect_edits_until_reset() {
        use hl2_ui::tuning::{ViewmodelFraming, ViewmodelProfile};

        let mut debug = crate::simulation::ViewmodelDebug::default();
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

        debug = crate::simulation::ViewmodelDebug::default();
        assert!(!debug.enabled);
        assert!(debug.edits.is_empty());
    }

    #[test]
    fn viewmodel_debug_adjustment_keys_use_expected_axes_and_precision() {
        let mut framing = hl2_ui::tuning::ViewmodelFraming {
            fov_offset: 0.0,
            offset: [0.0, 0.0, -11.5],
        };
        assert!(adjust_viewmodel_framing(
            &mut framing,
            &[
                KeyCode::BracketRight,
                KeyCode::Period,
                KeyCode::ArrowUp,
                KeyCode::ArrowLeft,
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
                KeyCode::ArrowRight,
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

    fn held_keys(codes: &[KeyCode]) -> ButtonInput<KeyCode> {
        let mut keys = ButtonInput::default();
        for &key in codes {
            keys.press(key);
            keys.clear_just_pressed(key);
        }
        keys
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
        use hl2_ui::tuning::ViewmodelProfile;

        let mut debug = crate::simulation::ViewmodelDebug::default();
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
    fn every_name_the_options_page_can_store_parses() {
        for (name, _) in NAMED_KEYS {
            assert!(
                matches!(parse_binding(name), Some(Trigger::Key(_))),
                "{name} should be a key"
            );
        }
        for c in 'A'..='Z' {
            assert!(parse_binding(&format!("Key{c}")).is_some());
        }
        for d in 0..=9 {
            assert!(parse_binding(&format!("Digit{d}")).is_some());
        }
        for f in 1..=12 {
            assert!(parse_binding(&format!("F{f}")).is_some());
        }
        assert_eq!(
            parse_binding("MOUSE4"),
            Some(Trigger::Mouse(MouseButton::Back))
        );
        assert_eq!(parse_binding("MWHEELDOWN"), Some(Trigger::WheelDown));
        assert_eq!(
            parse_binding("Ctrl"),
            Some(Trigger::Key(KeyCode::ControlLeft))
        );
        assert_eq!(parse_binding(source_ui::keys::UNBOUND), None);
        assert_eq!(parse_binding("JOY1"), None);
    }

    #[test]
    fn every_default_binding_parses() {
        for (action, name) in &DEFAULTS.keys {
            assert!(parse_binding(name).is_some(), "{action} -> {name}");
        }
    }

    #[test]
    fn bindings_follow_the_configuration_including_mouse_and_unbound_actions() {
        let mut keys = ButtonInput::<KeyCode>::default();
        let mut buttons = ButtonInput::<MouseButton>::default();
        let mut config = Config::default();
        config.keys.insert("+attack".into(), "KeyF".into());
        config.keys.insert("+jump".into(), "MOUSE2".into());
        config.keys.insert("+forward".into(), String::new());
        keys.press(KeyCode::KeyF);
        keys.press(KeyCode::KeyW);
        buttons.press(MouseButton::Right);
        let devices = Devices {
            keys: &keys,
            buttons: &buttons,
            wheel: 1.,
        };
        let cfg = Some(&config);
        assert!(devices.pressed("+attack", cfg));
        assert!(devices.pressed("+jump", cfg));
        assert!(!devices.pressed("+forward", cfg), "explicitly unbound");
        assert!(devices.just_pressed("invprev", cfg), "wheel up");
        assert!(!devices.just_pressed("invnext", cfg));
        assert!(
            devices.pressed("+forward", None),
            "defaults without a config"
        );
    }

    #[test]
    fn background_debug_keys_adjust_fov_and_opacity_in_paused_main_menu() {
        let mut sim = crate::simulation::Simulation::new(
            &modkit_core::World::default(),
            glam::Vec3::ZERO,
            0.,
            0.,
            false,
            vec![],
        );
        sim.background_level = true;
        sim.paused = true;
        sim.background_debug.enabled = true;
        assert!(sim.paused());

        let mut ui = crate::ui_host::Console::new(Default::default());
        ui.source.background_debug_mode = true;

        let mut repeats = DebugRepeats::default();

        // 1. Period increases opacity by 1%
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::Period);
        let tuning_input = DebugTuningInput {
            keys: &keys,
            time: Some(0.0),
            aspect: 16.0 / 9.0,
        };
        process_debug_tuning_keys(&mut sim, None, Some(&mut ui), tuning_input, &mut repeats);
        assert_eq!(sim.background_debug.blur_opacity, 51);
        assert_eq!(ui.source.background_blur_opacity, 51);

        // 2. Quick background swaps: Digit2 (ep1), Digit3 (ep2), Digit4 (lostcoast), Digit1 (hl2)
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::Digit2);
        let tuning_input = DebugTuningInput {
            keys: &keys,
            time: Some(0.0),
            aspect: 16.0 / 9.0,
        };
        process_debug_tuning_keys(&mut sim, None, Some(&mut ui), tuning_input, &mut repeats);
        assert_eq!(ui.map_request.as_deref(), Some("ep1_background01"));
        assert_eq!(ui.map_request_game.as_deref(), Some("episodic"));
        assert_eq!(sim.background_debug.map_name, "ep1_background01");

        let mut keys = ButtonInput::default();
        keys.press(KeyCode::Digit3);
        let tuning_input = DebugTuningInput {
            keys: &keys,
            time: Some(0.0),
            aspect: 16.0 / 9.0,
        };
        process_debug_tuning_keys(&mut sim, None, Some(&mut ui), tuning_input, &mut repeats);
        assert_eq!(ui.map_request.as_deref(), Some("ep2_background01"));
        assert_eq!(ui.map_request_game.as_deref(), Some("ep2"));
        assert_eq!(sim.background_debug.map_name, "ep2_background01");

        let mut keys = ButtonInput::default();
        keys.press(KeyCode::Digit4);
        let tuning_input = DebugTuningInput {
            keys: &keys,
            time: Some(0.0),
            aspect: 16.0 / 9.0,
        };
        process_debug_tuning_keys(&mut sim, None, Some(&mut ui), tuning_input, &mut repeats);
        assert_eq!(ui.map_request.as_deref(), Some("background01"));
        assert_eq!(ui.map_request_game.as_deref(), Some("lostcoast"));
        assert_eq!(sim.background_debug.map_name, "background01");

        let mut keys = ButtonInput::default();
        keys.press(KeyCode::Digit1);
        let tuning_input = DebugTuningInput {
            keys: &keys,
            time: Some(0.0),
            aspect: 16.0 / 9.0,
        };
        process_debug_tuning_keys(&mut sim, None, Some(&mut ui), tuning_input, &mut repeats);
        assert_eq!(ui.map_request.as_deref(), Some("background01"));
        assert_eq!(ui.map_request_game.as_deref(), Some("hl2"));
        assert_eq!(sim.background_debug.map_name, "background01");

        // 3. BracketLeft and BracketRight rotate through chapter backgrounds
        // From index 0 (hl2), BracketLeft wraps around to last game's last chapter background (lostcoast)
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::BracketLeft);
        let tuning_input = DebugTuningInput {
            keys: &keys,
            time: Some(0.0),
            aspect: 16.0 / 9.0,
        };
        process_debug_tuning_keys(&mut sim, None, Some(&mut ui), tuning_input, &mut repeats);
        assert_eq!(ui.map_request.as_deref(), Some("background01"));
        assert_eq!(ui.map_request_game.as_deref(), Some("lostcoast"));

        // From lostcoast, BracketRight wraps back to hl2 initial background
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::BracketRight);
        let tuning_input = DebugTuningInput {
            keys: &keys,
            time: Some(0.0),
            aspect: 16.0 / 9.0,
        };
        process_debug_tuning_keys(&mut sim, None, Some(&mut ui), tuning_input, &mut repeats);
        assert_eq!(ui.map_request.as_deref(), Some("background01"));
        assert_eq!(ui.map_request_game.as_deref(), Some("hl2"));

        // 4. Bracket keys do NOT alter opacity
        assert_eq!(sim.background_debug.blur_opacity, 51);

        // 5. Comma decreases opacity by 1%
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::Comma);
        let tuning_input = DebugTuningInput {
            keys: &keys,
            time: Some(0.0),
            aspect: 16.0 / 9.0,
        };
        process_debug_tuning_keys(&mut sim, None, Some(&mut ui), tuning_input, &mut repeats);
        assert_eq!(sim.background_debug.blur_opacity, 50);
        assert_eq!(ui.source.background_blur_opacity, 50);

        // 6. ArrowLeft toggles 0 opacity, ArrowRight toggles 100 opacity
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::ArrowLeft);
        let tuning_input = DebugTuningInput {
            keys: &keys,
            time: Some(0.0),
            aspect: 16.0 / 9.0,
        };
        process_debug_tuning_keys(&mut sim, None, Some(&mut ui), tuning_input, &mut repeats);
        assert_eq!(sim.background_debug.blur_opacity, 0);

        let mut keys = ButtonInput::default();
        keys.press(KeyCode::ArrowLeft);
        let tuning_input = DebugTuningInput {
            keys: &keys,
            time: Some(0.0),
            aspect: 16.0 / 9.0,
        };
        process_debug_tuning_keys(&mut sim, None, Some(&mut ui), tuning_input, &mut repeats);
        assert_eq!(sim.background_debug.blur_opacity, 50);

        let mut keys = ButtonInput::default();
        keys.press(KeyCode::ArrowRight);
        let tuning_input = DebugTuningInput {
            keys: &keys,
            time: Some(0.0),
            aspect: 16.0 / 9.0,
        };
        process_debug_tuning_keys(&mut sim, None, Some(&mut ui), tuning_input, &mut repeats);
        assert_eq!(sim.background_debug.blur_opacity, 100);

        let mut keys = ButtonInput::default();
        keys.press(KeyCode::ArrowRight);
        let tuning_input = DebugTuningInput {
            keys: &keys,
            time: Some(0.0),
            aspect: 16.0 / 9.0,
        };
        process_debug_tuning_keys(&mut sim, None, Some(&mut ui), tuning_input, &mut repeats);
        assert_eq!(sim.background_debug.blur_opacity, 50);

        // 7. ArrowUp and ArrowDown adjust FOV, Digit0 resets FOV
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::ArrowUp);
        let tuning_input = DebugTuningInput {
            keys: &keys,
            time: Some(0.0),
            aspect: 16.0 / 9.0,
        };
        process_debug_tuning_keys(&mut sim, None, Some(&mut ui), tuning_input, &mut repeats);
        let fov1 = sim.background_debug.fov.unwrap();

        let mut keys = ButtonInput::default();
        keys.press(KeyCode::ArrowDown);
        let tuning_input = DebugTuningInput {
            keys: &keys,
            time: Some(0.0),
            aspect: 16.0 / 9.0,
        };
        process_debug_tuning_keys(&mut sim, None, Some(&mut ui), tuning_input, &mut repeats);
        let fov2 = sim.background_debug.fov.unwrap();
        assert!(fov2 < fov1);

        let mut keys = ButtonInput::default();
        keys.press(KeyCode::Digit0);
        let tuning_input = DebugTuningInput {
            keys: &keys,
            time: Some(0.0),
            aspect: 16.0 / 9.0,
        };
        process_debug_tuning_keys(&mut sim, None, Some(&mut ui), tuning_input, &mut repeats);
        assert_eq!(sim.background_debug.fov, None);

        // 8. When console is open, keys do not adjust opacity or FOV
        ui.source.set_mode(hl2_ui::console::Mode::Console);
        let mut keys = ButtonInput::default();
        keys.press(KeyCode::Comma);
        let tuning_input = DebugTuningInput {
            keys: &keys,
            time: Some(0.0),
            aspect: 16.0 / 9.0,
        };
        process_debug_tuning_keys(&mut sim, None, Some(&mut ui), tuning_input, &mut repeats);
        assert_eq!(sim.background_debug.blur_opacity, 50);
    }
}
