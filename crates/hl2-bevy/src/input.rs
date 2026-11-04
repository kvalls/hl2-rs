//! Bevy event adaptation for gameplay and character input.
//!
//! UI event handling lives in `ui`; this module only turns an already captured
//! keyboard/mouse frame into gameplay actions and a Source movement intent.
//!
//! Every gameplay action is read through its persisted binding (`Config::keys`): keyboard keys use
//! Bevy's `KeyCode` names, mouse buttons `MOUSE1`..`MOUSE5` and the wheel `MWHEELUP`/`MWHEELDOWN`,
//! exactly as the Options key capture writes them. An empty or unknown binding is unbound.

use hl2_ui::config::Config;
use modkit_core::movement::Input;
use std::{collections::HashMap, sync::LazyLock};

use bevy::{
    input::mouse::AccumulatedMouseMotion,
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};

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

/// Both attacks reach the weapon unless an extension claimed them.
fn route_attacks(
    game: &mut crate::gameplay::Gameplay,
    devices: &Devices,
    cfg: Option<&Config>,
    claimed: bool,
) {
    game.buttons(
        devices.pressed("+attack", cfg) && !claimed,
        devices.pressed("+attack2", cfg) && !claimed,
    );
}

/// A claimed pointer leaves the view still, unless the right button holds it for looking.
fn mouse_look_allowed(claimed: bool, buttons: &ButtonInput<MouseButton>) -> bool {
    !claimed || buttons.pressed(MouseButton::Right)
}

/// A claimed pointer keeps the wheel for what claimed it: no binding (`invprev`, `invnext`) fires.
fn free_wheel(claimed: bool, delta: f32) -> f32 {
    if claimed { 0. } else { delta }
}

fn held_devices<'a>(
    keys: &'a ButtonInput<KeyCode>,
    buttons: &'a ButtonInput<MouseButton>,
) -> Devices<'a> {
    Devices {
        keys,
        buttons,
        wheel: 0.,
    }
}

pub fn action_just_pressed(
    action: &str,
    keys: &ButtonInput<KeyCode>,
    buttons: &ButtonInput<MouseButton>,
    config: Option<&Config>,
) -> bool {
    held_devices(keys, buttons).just_pressed(action, config)
}
/// Whether the input bound to a held action is down (keyboard and mouse buttons).
pub(crate) fn action_pressed(
    action: &str,
    keys: &ButtonInput<KeyCode>,
    buttons: &ButtonInput<MouseButton>,
    config: Option<&Config>,
) -> bool {
    held_devices(keys, buttons).pressed(action, config)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn controls(
    (keys, buttons, mouse, scroll, _frame): InputDevices,
    windows: Query<(&Window, &CursorOptions), With<PrimaryWindow>>,
    mut sim: ResMut<crate::simulation::Simulation>,
    mut game: Option<ResMut<crate::gameplay::Gameplay>>,
    ui: Option<ResMut<crate::ui_host::Console>>,
    mut mouse_filter: Local<hl2_ui::input::MouseFilter>,
    real_time: Option<Res<Time<Real>>>,
) {
    let mut ui = ui;
    sim.input_claims = Default::default();
    let input = crate::extensions::Input {
        keys: &keys,
        buttons: &buttons,
        focused: windows.single().is_ok_and(|(window, _)| window.focused),
        cursor_grabbed: windows
            .single()
            .is_ok_and(|(_, cursor)| cursor.grab_mode != CursorGrabMode::None),
        aspect: crate::video::frame_aspect(_frame.as_deref()),
        time: real_time.as_deref().map(Time::elapsed_secs_f64),
    };
    sim.extensions_input(game.as_deref_mut(), ui.as_deref_mut(), &input);
    // CGameRules::Think reads `skill` every frame, and weapon selection reads
    // `hud_fastswitch` on each key, so changed settings apply at once.
    if let (Some(game), Some(ui)) = (game.as_deref_mut(), ui.as_deref()) {
        game.set_skill(ui.source.config.gameplay.skill);
        game.selection.fast_switch = ui.source.config.keyboard.fast_switch;
    }
    let Ok((_window, cursor)) = windows.single() else {
        return;
    };
    if !sim.commands.is_empty() {
        return;
    }
    let overlay_open = sim.input_claims.attacks;
    let typing = sim.input_claims.keyboard;
    let cfg = ui.as_ref().map(|u| &u.source.config);
    let devices = Devices {
        keys: &keys,
        buttons: &buttons,
        wheel: free_wheel(overlay_open, scroll.as_deref().map_or(0., |s| s.delta.y)),
    };
    if !devices.pressed("+jump", cfg) {
        sim.jump_suppressed = false;
    }
    if let Some(game) = game.as_deref_mut() {
        route_attacks(game, &devices, cfg, overlay_open);
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
    if mouse_look_allowed(overlay_open, &buttons) {
        let settings = cfg.map_or(&DEFAULTS.mouse, |c| &c.mouse);
        let [dx, dy] = mouse_filter.apply([mouse.delta.x, mouse.delta.y], settings);
        let radians = hl2_ui::input::MOUSE_DEGREES_PER_COUNT.to_radians();
        sim.yaw -= dx * radians;
        let pitch_sign = if settings.invert { -1.0 } else { 1.0 };
        sim.pitch = (sim.pitch - dy * radians * pitch_sign).clamp(-1.55, 1.55);
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

#[cfg(test)]
mod tests {
    use super::*;
    use source_ui::keys::NAMED_KEYS;

    /// Claims the attacks through the contract, as any contributor would.
    struct ClaimAttacks(bool);
    impl crate::extensions::RuntimeExtension for ClaimAttacks {
        fn input(
            &mut self,
            context: &mut crate::extensions::Context<'_>,
            _input: &crate::extensions::Input<'_>,
        ) {
            context.simulation.input_claims.attacks |= self.0;
        }
    }

    fn secondary_after_input(claim: bool) -> bool {
        let keys = ButtonInput::<KeyCode>::default();
        let mut buttons = ButtonInput::<MouseButton>::default();
        buttons.press(MouseButton::Right);
        let mut game = crate::gameplay::Gameplay::synthetic(modkit_core::World::default());
        let mut sim = crate::simulation::Simulation::new(
            &modkit_core::World::default(),
            glam::Vec3::ZERO,
            0.,
            0.,
            false,
            vec![],
        );
        sim.extensions.push(Box::new(ClaimAttacks(claim)));
        sim.extensions_input(
            None,
            None,
            &crate::extensions::Input {
                keys: &keys,
                buttons: &buttons,
                focused: true,
                cursor_grabbed: true,
                aspect: 16. / 9.,
                time: None,
            },
        );
        let devices = Devices {
            keys: &keys,
            buttons: &buttons,
            wheel: 0.,
        };
        route_attacks(&mut game, &devices, None, sim.input_claims.attacks);
        game.queued_secondary
    }

    #[test]
    fn secondary_attack_reaches_the_weapon_unless_claimed_and_returns_afterwards() {
        assert!(secondary_after_input(false), "no claim");
        assert!(!secondary_after_input(true), "claimed");
        assert!(secondary_after_input(false), "claim released");
    }

    #[test]
    fn a_claimed_pointer_only_looks_while_the_right_button_is_held() {
        let mut buttons = ButtonInput::<MouseButton>::default();
        assert!(mouse_look_allowed(false, &buttons));
        assert!(!mouse_look_allowed(true, &buttons));
        buttons.press(MouseButton::Right);
        assert!(mouse_look_allowed(true, &buttons));
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
    fn a_claimed_pointer_keeps_the_wheel_from_the_weapon_bindings() {
        let keys = ButtonInput::<KeyCode>::default();
        let buttons = ButtonInput::<MouseButton>::default();
        let wheel_up = |claimed| Devices {
            keys: &keys,
            buttons: &buttons,
            wheel: free_wheel(claimed, 1.),
        };
        assert!(wheel_up(false).just_pressed("invprev", None));
        assert!(!wheel_up(true).just_pressed("invprev", None));
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
}
