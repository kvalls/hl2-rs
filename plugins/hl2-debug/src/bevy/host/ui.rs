use super::State;
use bevy::prelude::{KeyCode, MouseButton};
use bevy::window::CursorGrabMode;
use hl2_bevy::extensions::{Context, UiInput};
impl State {
    pub(super) fn before_ui(&mut self, context: &mut Context<'_>, input: &UiInput<'_>) {
        self.overlay.rows_visible = self.rows;
        // The panel belongs to a loaded game: without one it neither takes Escape nor the pointer.
        if context.gameplay.is_none() {
            return;
        }
        if input.keys.just_pressed(KeyCode::Escape) {
            let paused = context.simulation.paused;
            context.simulation.input_claims.escape |=
                self.panel_takes_escape(paused, input.selection_cancelled);
        }
        context.simulation.input_claims.pointer |= self.overlay.open;
    }
    /// Escape cancels a weapon selection first, then this panel's typing, then the panel; only
    /// after that does it reach the pause menu.
    pub(super) fn panel_takes_escape(&mut self, paused: bool, selection_cancelled: bool) -> bool {
        !paused && !selection_cancelled && self.overlay.escape()
    }
    pub(super) fn after_ui(&mut self, context: &mut Context<'_>, input: &UiInput<'_>) {
        let sim = &mut *context.simulation;
        let game = context.gameplay.as_deref_mut();
        let ui = context.console.as_deref_mut();
        let (keys, buttons, text, wheel, window, cursor) = (
            input.keys,
            input.buttons,
            input.text,
            input.wheel,
            input.window,
            input.cursor,
        );
        let overlay_open = self.overlay.open;

        let typing = self.overlay.typing;
        if let Some(game) = game
            && !sim.paused
            && !sim.transition
        {
            use crate::spawn::OverlayKey;
            let mut pressed: Vec<OverlayKey> = crate::bevy::input::SPAWN_KEYS
                .iter()
                .copied()
                .filter(|(key, _)| keys.just_pressed(*key))
                .map(|(_, overlay)| overlay)
                .collect();
            let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
            if overlay_open
                && !typing
                && (text.contains('/') || (ctrl && keys.just_pressed(KeyCode::KeyF)))
            {
                pressed.push(OverlayKey::Filter);
            }
            let dt = input.delta;
            if let Some((key, presses)) = self.repeat.update(
                keys,
                &[
                    KeyCode::ArrowUp,
                    KeyCode::ArrowDown,
                    KeyCode::PageUp,
                    KeyCode::PageDown,
                    KeyCode::Backspace,
                ],
                dt,
            ) {
                let overlay = match key {
                    KeyCode::ArrowUp => OverlayKey::Up,
                    KeyCode::ArrowDown => OverlayKey::Down,
                    KeyCode::PageUp => OverlayKey::PageUp,
                    KeyCode::PageDown => OverlayKey::PageDown,
                    _ => OverlayKey::Backspace,
                };
                pressed.extend(std::iter::repeat_n(overlay, presses));
            }
            if typing {
                self.overlay.type_text(text);
            }
            let mut messages = Vec::new();
            for overlay in pressed {
                messages.extend(self.spawn_overlay_key(sim, game, overlay));
            }
            let pointer = window
                .cursor_position()
                .filter(|_| overlay_open && cursor.grab_mode == CursorGrabMode::None);
            if let Some(pointer) = pointer {
                if buttons.just_pressed(MouseButton::Left)
                    && let Some(target) = crate::spawn_panel::hit(&self.hits, pointer.x, pointer.y)
                {
                    let command = self.overlay.click(target);
                    messages.extend(self.spawn_overlay_command(sim, game, command));
                }
                if wheel != 0. {
                    let rows = -wheel * 3.;
                    let column = crate::spawn_panel::column_at(&self.hits, pointer.x);
                    self.overlay.scroll(column, rows.round() as isize);
                }
            }
            if let Some(ui) = ui {
                for message in messages {
                    ui.source.log(message);
                }
            }
        }
    }
    pub(super) fn replay_action(
        &mut self,
        context: &mut Context<'_>,
        action: &serde_json::Value,
    ) -> bool {
        let Some(game) = context.gameplay.as_deref_mut() else {
            return false;
        };
        let command = match action["action"].as_str() {
            Some("spawn_overlay") => action
                .get("key")
                .and_then(|key| serde_json::from_value(key.clone()).ok())
                .and_then(|key| self.overlay.key(key)),
            Some("spawn_overlay_text") => {
                self.overlay
                    .type_text(action["text"].as_str().unwrap_or_default());
                None
            }
            Some("spawn_overlay_click") => action
                .get("target")
                .and_then(|target| serde_json::from_value(target.clone()).ok())
                .and_then(|target| self.overlay.click(target)),
            Some("spawn_overlay_scroll") => {
                let column = action
                    .get("column")
                    .and_then(|column| serde_json::from_value(column.clone()).ok());
                self.overlay
                    .scroll(column, action["rows"].as_i64().unwrap_or_default() as isize);
                None
            }
            _ => return false,
        };
        if let Some(message) = self.spawn_overlay_command(context.simulation, game, command)
            && let Some(ui) = context.console.as_deref_mut()
        {
            ui.source.log(message);
        }
        true
    }
}
