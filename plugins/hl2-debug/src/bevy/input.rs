//! Bevy keyboard adaptation for debug tools.
use bevy::prelude::{ButtonInput, KeyCode};
use modkit_core::viewmodel::ViewmodelFraming;
use source_ui::widgets::HoldRepeat;

pub const VIEWMODEL_ADJUST_KEYS: &[KeyCode] = &[
    KeyCode::BracketLeft,
    KeyCode::BracketRight,
    KeyCode::Comma,
    KeyCode::Period,
    KeyCode::ArrowUp,
    KeyCode::ArrowDown,
    KeyCode::ArrowLeft,
    KeyCode::ArrowRight,
];

#[derive(bevy::prelude::Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct DebugRepeats {
    pub vm: KeyAdjustRepeat,
    pub bg_fov: KeyAdjustRepeat,
    pub bg_opacity: KeyAdjustRepeat,
}

/// Latest adjust key repeats while held, using the same [`HoldRepeat`] timing as
/// Options > Keyboard list arrows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KeyAdjustRepeat {
    key: Option<KeyCode>,
    timer: HoldRepeat,
}

impl KeyAdjustRepeat {
    pub fn update(
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

/// The key that zeroes the current weapon's framing.
pub fn reset_key_pressed(keys: &ButtonInput<KeyCode>) -> bool {
    keys.any_just_pressed([KeyCode::Digit0, KeyCode::Numpad0])
}

/// Zero FOV and position offsets for the selected weapon's current aspect profile.
pub fn reset_viewmodel_debug_weapon(debug: &mut crate::viewmodel::ViewmodelDebug) -> bool {
    if debug.framing == ViewmodelFraming::ZERO {
        return false;
    }
    debug.framing = ViewmodelFraming::ZERO;
    true
}

/// Step per viewmodel adjust key press: 1.0 with Shift, 0.01 with Alt/Option (which wins over
/// Shift), else 0.1.
pub fn viewmodel_adjust_step(keys: &ButtonInput<KeyCode>) -> f32 {
    if keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]) {
        0.01
    } else if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
        1.0
    } else {
        0.1
    }
}

/// Applies one-frame viewmodel debug key presses. Returns whether the profile changed.
pub fn adjust_viewmodel_framing(
    framing: &mut ViewmodelFraming,
    fired: &[KeyCode],
    step: f32,
) -> bool {
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

/// Debug-only shortcut bindings. Ordinary bindable F-keys remain available to the host.
pub const OVERLAY_KEY: KeyCode = KeyCode::F1;
pub const FLIGHT_KEY: KeyCode = KeyCode::F2;
pub const SPAWN_KEYS: &[(KeyCode, crate::spawn::OverlayKey)] = &[
    (KeyCode::F4, crate::spawn::OverlayKey::Toggle),
    (KeyCode::ArrowLeft, crate::spawn::OverlayKey::Left),
    (KeyCode::ArrowRight, crate::spawn::OverlayKey::Right),
    (KeyCode::Enter, crate::spawn::OverlayKey::Enter),
    (KeyCode::Tab, crate::spawn::OverlayKey::Tab),
];
