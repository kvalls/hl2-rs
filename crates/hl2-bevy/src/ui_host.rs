//! Bevy host resource and input adapter for the portable Source GameUI/console.
use bevy::prelude::*;
use hl2_ui::console::Mode;
use serde::Deserialize;

#[allow(dead_code)]
pub type UiHost = Console;

#[derive(Resource)]
pub struct Console {
    pub source: hl2_ui::console::Console,
    pub map_request: Option<String>,
    /// Installation folder of the game `map_request` belongs to, when the request names it
    /// (menu backgrounds); otherwise the loader finds the game that ships the map.
    pub map_request_game: Option<String>,
    /// The pending `map_request` loads a main-menu background level.
    pub background_request: bool,
    pub quit_requested: bool,
}
impl Console {
    pub fn new(source: hl2_ui::console::Console) -> Self {
        Self {
            source,
            map_request: None,
            map_request_game: None,
            background_request: false,
            quit_requested: false,
        }
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"mode":self.source.mode(),"background_level":self.source.background_level(),"sv_cheats":self.source.cheats,"input":self.source.input_line(),"output":self.source.shell.panel.output,"pending_map":self.map_request,"pending_game":self.map_request_game,"options":self.source.options_report(),"chapters":self.source.chapters_report(),"windows":self.source.open_windows(),"confirm":self.source.confirm_report(),"menu":self.source.menu_report(),"console_frame":self.source.console_frame_report(),"video":self.source.video_report()})
    }
}
#[derive(Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Escape,
    ToggleConsole,
    Resume,
    Command {
        command: String,
    },
    Input {
        input: source_ui::input::Input,
    },
    /// One F4 overlay key, as if pressed (Toggle, Up, Down, Left, Right, Enter).
    SpawnOverlay {
        key: hl2_simulation::ai::spawn::OverlayKey,
    },
    /// Text typed into the F4 Props filter (while it is typing).
    SpawnOverlayText {
        text: String,
    },
    /// A pointer click on an F4 row, e.g. {"Row": ["Options", 3]} or "Filter".
    SpawnOverlayClick {
        target: hl2_simulation::ai::spawn::PanelTarget,
    },
    /// Mouse wheel rows over an F4 column (null: the focused one).
    SpawnOverlayScroll {
        column: Option<hl2_simulation::ai::spawn::Column>,
        rows: isize,
    },
}
impl Action {
    pub fn valid(&self) -> bool {
        match self {
            Self::Command { command } => command.len() <= 1024,
            Self::SpawnOverlayText { text } => text.len() <= 256,
            Self::Input { input } => {
                input.text.len() <= 4096 && input.pointer.is_none_or(|p| p.is_finite())
            }
            _ => true,
        }
    }
}
impl crate::simulation::Simulation {
    pub fn ui_action(
        &mut self,
        ui: &mut Console,
        game: &mut crate::gameplay::Gameplay,
        action: &Action,
    ) {
        let effects = match action {
            Action::Escape => {
                if !ui.source.paused() && game.selection.pending.is_some() {
                    game.actions.push(crate::gameplay::Action::Cancel);
                } else {
                    ui.source.escape();
                }
                vec![]
            }
            Action::ToggleConsole => {
                ui.source.toggle();
                vec![]
            }
            Action::Resume => {
                ui.source.set_mode(Mode::Gameplay);
                vec![]
            }
            Action::Command { command } => ui.source.submit(command),
            Action::Input { input } => ui.source.input(input),
            Action::SpawnOverlay { key } => {
                if let Some(message) = self.spawn_overlay_key(game, *key) {
                    ui.source.log(message);
                }
                vec![]
            }
            Action::SpawnOverlayText { text } => {
                game.spawn_overlay.type_text(text);
                vec![]
            }
            Action::SpawnOverlayClick { target } => {
                let command = game.spawn_overlay.click(*target);
                if let Some(message) = self.spawn_overlay_command(game, command) {
                    ui.source.log(message);
                }
                vec![]
            }
            Action::SpawnOverlayScroll { column, rows } => {
                game.spawn_overlay.scroll(*column, *rows);
                vec![]
            }
        };
        self.console_effects(ui, game, effects);
    }
}
/// The UI keys and the native keys they come from.
const UI_KEYS: [(KeyCode, source_ui::input::Key); 13] = {
    use source_ui::input::Key;
    [
        (KeyCode::ArrowUp, Key::Up),
        (KeyCode::ArrowDown, Key::Down),
        (KeyCode::Enter, Key::Enter),
        (KeyCode::Backspace, Key::Backspace),
        (KeyCode::Delete, Key::Delete),
        (KeyCode::ArrowLeft, Key::Left),
        (KeyCode::ArrowRight, Key::Right),
        (KeyCode::Home, Key::Home),
        (KeyCode::End, Key::End),
        (KeyCode::PageUp, Key::PageUp),
        (KeyCode::PageDown, Key::PageDown),
        (KeyCode::Tab, Key::Tab),
        (KeyCode::Space, Key::Space),
    ]
};

/// UI keys pressed this frame.
pub fn keys(native: &ButtonInput<KeyCode>) -> std::collections::BTreeSet<source_ui::input::Key> {
    ui_keys(|code| native.just_pressed(code))
}

/// UI keys held this frame (for key repeat).
pub fn held_keys(
    native: &ButtonInput<KeyCode>,
) -> std::collections::BTreeSet<source_ui::input::Key> {
    ui_keys(|code| native.pressed(code))
}

fn ui_keys(
    mut down: impl FnMut(KeyCode) -> bool,
) -> std::collections::BTreeSet<source_ui::input::Key> {
    UI_KEYS
        .into_iter()
        .filter_map(|(code, key)| down(code).then_some(key))
        .collect()
}

/// Wheel steps of this frame; positive scrolls up. Pixel deltas (touchpads) count one step per
/// 40 pixels.
pub fn wheel_steps<'a>(events: impl Iterator<Item = &'a bevy::input::mouse::MouseWheel>) -> f32 {
    use bevy::input::mouse::MouseScrollUnit;
    events
        .map(|e| match e.unit {
            MouseScrollUnit::Line => e.y,
            MouseScrollUnit::Pixel => e.y / 40.,
        })
        .sum()
}

/// Keys and buttons pressed this frame as persisted binding names: the `KeyCode` name for keys
/// (`KeyW`, `SuperLeft`), the Source names `MOUSE1`..`MOUSE5` for buttons and
/// `MWHEELUP`/`MWHEELDOWN` for the wheel. Unidentified native keys are skipped.
/// Control and Shift held, and whether this frame pastes or copies: Cmd+V / Cmd+C on macOS,
/// Ctrl+V / Ctrl+C elsewhere.
/// Typed text is dropped while Control or Cmd is held (shortcuts, not text).
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub command: bool,
    pub paste: bool,
    pub copy: bool,
}

pub fn modifiers(native: &ButtonInput<KeyCode>) -> Modifiers {
    let ctrl = native.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let command = native.any_pressed([KeyCode::SuperLeft, KeyCode::SuperRight]);
    let clipboard_modifier = if cfg!(target_os = "macos") {
        command
    } else {
        ctrl
    };
    Modifiers {
        ctrl,
        shift: native.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
        alt: native.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]),
        command,
        paste: clipboard_modifier && native.just_pressed(KeyCode::KeyV),
        copy: clipboard_modifier && native.just_pressed(KeyCode::KeyC),
    }
}

pub fn binding_keys(
    keys: &ButtonInput<KeyCode>,
    buttons: &ButtonInput<MouseButton>,
    wheel: f32,
) -> Vec<String> {
    let mut names: Vec<String> = keys
        .get_just_pressed()
        .map(|key| format!("{key:?}"))
        .filter(|name| name.chars().all(|c| c.is_ascii_alphanumeric()))
        .collect();
    for (button, name) in [
        (MouseButton::Left, "MOUSE1"),
        (MouseButton::Right, "MOUSE2"),
        (MouseButton::Middle, "MOUSE3"),
        (MouseButton::Back, "MOUSE4"),
        (MouseButton::Forward, "MOUSE5"),
    ] {
        if buttons.just_pressed(button) {
            names.push(name.into());
        }
    }
    if wheel > 0. {
        names.push("MWHEELUP".into());
    } else if wheel < 0. {
        names.push("MWHEELDOWN".into());
    }
    names
}

#[cfg(test)]
mod binding_tests {
    use super::*;

    #[test]
    fn pressed_inputs_become_binding_names() {
        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::SuperLeft);
        keys.press(KeyCode::KeyW);
        let mut buttons = ButtonInput::<MouseButton>::default();
        buttons.press(MouseButton::Back);
        let mut names = binding_keys(&keys, &buttons, -1.);
        names.sort();
        assert_eq!(names, ["KeyW", "MOUSE4", "MWHEELDOWN", "SuperLeft"]);
        assert!(binding_keys(&ButtonInput::default(), &ButtonInput::default(), 0.).is_empty());
    }
}
