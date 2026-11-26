//! Keyboard text from the window, including macOS dead-key composition.

use bevy::{
    input::ButtonState,
    prelude::{Entity, Mut},
    window::{Ime, Window, WindowEvent},
};

pub(super) fn read<'a>(
    events: impl Iterator<Item = &'a WindowEvent>,
    window: Entity,
    composition_enabled: bool,
) -> String {
    events
        .filter_map(|event| match event {
            WindowEvent::KeyboardInput(key)
                if key.window == window && key.state == ButtonState::Pressed =>
            {
                key.text.as_deref()
            }
            WindowEvent::Ime(Ime::Commit {
                window: source,
                value,
            }) if composition_enabled && *source == window => Some(value.as_str()),
            _ => None,
        })
        .collect()
}

pub(super) fn sync_console_focus(window: &mut Mut<Window>, console: &hl2_ui::console::Console) {
    if cfg!(target_os = "macos") {
        let enabled = window.focused
            && console.mode() == hl2_ui::console::Mode::Console
            && !console.modal_dialog_open();
        if window.ime_enabled != enabled {
            window.ime_enabled = enabled;
        }
    }
}

/// Winit 0.30.13 can receive a late IME commit while dropping a macOS window.
/// Clear the composition while the native window is still alive.
#[cfg(target_os = "macos")]
pub(super) fn end_composition_on_close(
    _main_thread: bevy::ecs::system::NonSendMarker,
    mut close: bevy::ecs::message::MessageReader<bevy::window::WindowCloseRequested>,
    mut exit: bevy::ecs::message::MessageReader<bevy::app::AppExit>,
    mut windows: bevy::prelude::Query<
        (Entity, &mut Window),
        bevy::prelude::With<bevy::window::PrimaryWindow>,
    >,
) {
    let exiting = !exit.is_empty();
    exit.clear();
    if !exiting && close.is_empty() {
        return;
    }
    if let Ok((entity, mut window)) = windows.single_mut()
        && (close.read().any(|event| event.window == entity) || exiting)
    {
        window.ime_enabled = false;
        bevy::winit::WINIT_WINDOWS.with_borrow(|native| {
            if let Some(window) = native.get_window(entity) {
                window.set_ime_allowed(false);
            }
        });
    }
    close.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::input::keyboard::{Key, KeyCode, KeyboardInput};

    #[test]
    fn preserves_composed_quotes_in_keyboard_event_order() {
        let window = Entity::from_raw_u32(1).unwrap();
        let other = Entity::from_raw_u32(2).unwrap();
        let key = |text: &str| {
            WindowEvent::KeyboardInput(KeyboardInput {
                key_code: KeyCode::KeyA,
                logical_key: Key::Character(text.into()),
                state: ButtonState::Pressed,
                text: Some(text.into()),
                repeat: false,
                window,
            })
        };
        let events = [
            key("a"),
            WindowEvent::Ime(Ime::Preedit {
                window,
                value: "'".into(),
                cursor: None,
            }),
            WindowEvent::Ime(Ime::Commit {
                window,
                value: "'".into(),
            }),
            key("b"),
            WindowEvent::Ime(Ime::Commit {
                window,
                value: "\"".into(),
            }),
            WindowEvent::Ime(Ime::Commit {
                window: other,
                value: "unrelated".into(),
            }),
            key("c"),
        ];
        assert_eq!(read(events.iter(), window, true), "a'b\"c");
        assert_eq!(read(events.iter(), window, false), "abc");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn composition_follows_the_console_keyboard_focus() {
        use bevy::prelude::World;
        use hl2_ui::console::{Console, Mode};

        let mut world = World::new();
        let id = world
            .spawn(Window {
                focused: true,
                ..Default::default()
            })
            .id();
        let mut window = world.get_mut::<Window>(id).unwrap();
        let mut console = Console::default();
        console.set_mode(Mode::Console);
        sync_console_focus(&mut window, &console);
        assert!(window.ime_enabled);

        window.focused = false;
        sync_console_focus(&mut window, &console);
        assert!(!window.ime_enabled);

        window.focused = true;
        sync_console_focus(&mut window, &console);
        assert!(window.ime_enabled);

        console.set_mode(Mode::Pause);
        sync_console_focus(&mut window, &console);
        assert!(!window.ime_enabled);
    }
}
