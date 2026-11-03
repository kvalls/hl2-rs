//! One frame of UI input as the host delivers it: keys pressed and held, typed text, the
//! pointer and its left button, the wheel, modifiers and clipboard, and the frame time.
use crate::widgets::HoldRepeat;
use glam::Vec2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Key {
    Up,
    Down,
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
    /// Presses the focused dialog button, check box or radio button (SDK `Button`).
    Space,
}
impl Key {
    /// Keys that repeat while held: moving through lists and text, deleting. Enter, Tab, Home
    /// and End act once per press.
    pub const fn repeats(self) -> bool {
        matches!(
            self,
            Self::Up
                | Self::Down
                | Self::Left
                | Self::Right
                | Self::Backspace
                | Self::Delete
                | Self::PageUp
                | Self::PageDown
        )
    }
}

/// Keyboard repeat for every UI layer at once, applied to [`Input::keys`] before any layer reads
/// them: like an operating system, the latest pressed key repeats while held ([`HoldRepeat`]), and
/// any other press takes over or (when it does not repeat) stops it. Typed text already arrives
/// with the system's own repeat and is left alone.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KeyRepeat {
    key: Option<Key>,
    timer: HoldRepeat,
}
impl KeyRepeat {
    /// Adds this frame's repeat of the held key to `input.keys`.
    pub fn apply(&mut self, input: &mut Input) {
        if let Some(&pressed) = input.keys.iter().next_back() {
            self.key = pressed.repeats().then_some(pressed);
            self.timer.press(input.time);
        } else if let Some(key) = self.key {
            if !input.held_keys.contains(&key) {
                self.cancel();
            } else if self.timer.held(input.time) {
                input.keys.insert(key);
            }
        }
    }

    /// Stops the repeat until the next press.
    pub fn cancel(&mut self) {
        *self = Self::default();
    }
}

#[derive(Clone, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Input {
    pub keys: std::collections::BTreeSet<Key>,
    /// Keys held down this frame (pressed earlier or now); [`KeyRepeat`] repeats them.
    pub held_keys: std::collections::BTreeSet<Key>,
    pub text: String,
    pub pointer: Option<Vec2>,
    /// Left button went down this frame.
    pub click: bool,
    /// Left button is held (slider drags).
    pub held: bool,
    /// Left button was released this frame.
    pub released: bool,
    /// Mouse wheel steps this frame; positive scrolls up.
    pub wheel: f32,
    /// Keys and buttons pressed this frame as persisted binding names (`KeyW`, `SuperLeft`,
    /// `MOUSE1`, `MWHEELUP`), used when the keyboard page captures a new key.
    pub binding_keys: Vec<String>,
    /// Monotonic real time of this frame in seconds, used for double clicks. `None` (scripted
    /// input without times) disables double-click detection.
    pub time: Option<f64>,
    /// Control is held (console editing shortcuts); typed text is ignored meanwhile.
    pub ctrl: bool,
    /// Shift is held (Shift+Tab steps the completion list back).
    pub shift: bool,
    /// Alt (Option) is held: Alt+Up/Down opens a focused combo box's list (SDK `ComboBox`).
    pub alt: bool,
    /// Text pasted this frame (the host reads the clipboard on its paste shortcut).
    pub paste: Option<String>,
    /// The copy shortcut (Cmd+C on macOS, Ctrl+C elsewhere) was pressed this frame.
    pub copy: bool,
    /// The host platform select-all shortcut was pressed.
    pub select_all: bool,
}
impl Input {
    pub fn pressed(&self, key: Key) -> bool {
        self.keys.contains(&key)
    }

    /// What one UI layer receives of this frame: the pointer (position, button, wheel) when it
    /// owns it, the keyboard (keys, text, modifiers, clipboard) when it has the focus, and the
    /// time either way.
    pub fn share(&self, pointer: bool, keyboard: bool) -> Input {
        let mut share = if keyboard {
            self.clone()
        } else {
            Input {
                time: self.time,
                ..Input::default()
            }
        };
        if pointer {
            share.pointer = self.pointer;
            share.click = self.click;
            share.held = self.held;
            share.released = self.released;
            share.wheel = self.wheel;
        } else {
            share.pointer = None;
            share.click = false;
            share.held = false;
            share.released = false;
            share.wheel = 0.;
        }
        share
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_frame(t: f64, pressed: &[Key], held: &[Key]) -> Input {
        Input {
            keys: pressed.iter().copied().collect(),
            held_keys: held.iter().chain(pressed).copied().collect(),
            time: Some(t),
            ..Default::default()
        }
    }

    #[test]
    fn key_repeat_follows_the_latest_press_while_it_is_held() {
        let mut repeat = KeyRepeat::default();
        let mut frame = |t: f64, pressed: &[Key], held: &[Key]| {
            let mut input = key_frame(t, pressed, held);
            repeat.apply(&mut input);
            input.keys.into_iter().collect::<Vec<_>>()
        };
        assert_eq!(frame(0., &[Key::Down], &[]), [Key::Down]);
        assert_eq!(frame(0.2, &[], &[Key::Down]), []);
        assert_eq!(frame(0.4, &[], &[Key::Down]), [Key::Down]);
        // A newer press takes over; the earlier key, still held, no longer repeats.
        assert_eq!(frame(0.5, &[Key::Left], &[Key::Down]), [Key::Left]);
        assert_eq!(frame(0.9, &[], &[Key::Down, Key::Left]), [Key::Left]);
        // Releasing stops it, and it stays stopped when the key goes down again unpressed.
        assert_eq!(frame(1.0, &[], &[Key::Down]), []);
        assert_eq!(frame(2.0, &[], &[Key::Down, Key::Left]), []);
        // Keys that act once never repeat, and their press stops the repeating key.
        assert_eq!(frame(3.0, &[Key::Up], &[]), [Key::Up]);
        assert_eq!(frame(3.1, &[Key::Enter], &[Key::Up]), [Key::Enter]);
        assert_eq!(frame(4.0, &[], &[Key::Up, Key::Enter]), []);
        // Scripted input carries no time: nothing repeats.
        let mut input = key_frame(0., &[Key::Down], &[]);
        input.time = None;
        repeat.apply(&mut input);
        let mut held = Input {
            held_keys: [Key::Down].into(),
            ..Default::default()
        };
        repeat.apply(&mut held);
        assert!(held.keys.is_empty());
    }

    #[test]
    fn a_layer_gets_the_pointer_when_it_owns_it_and_the_keyboard_when_it_has_the_focus() {
        let frame = Input {
            keys: [Key::Enter].into(),
            text: "a".into(),
            pointer: Some(Vec2::new(10., 20.)),
            click: true,
            held: true,
            wheel: 1.,
            time: Some(2.5),
            ctrl: true,
            copy: true,
            ..Default::default()
        };
        let both = frame.share(true, true);
        assert!(both.pressed(Key::Enter) && both.click && both.ctrl && both.copy);
        assert_eq!(
            (both.pointer, both.wheel, both.time),
            (frame.pointer, 1., Some(2.5))
        );
        let pointer = frame.share(true, false);
        assert!(pointer.keys.is_empty() && pointer.text.is_empty() && !pointer.ctrl);
        assert!(pointer.click && pointer.held && pointer.pointer.is_some());
        let keyboard = frame.share(false, true);
        assert!(keyboard.pressed(Key::Enter) && keyboard.text == "a" && keyboard.copy);
        assert!(keyboard.pointer.is_none() && !keyboard.click && !keyboard.held);
        assert_eq!(keyboard.wheel, 0.);
        let neither = frame.share(false, false);
        assert!(neither.keys.is_empty() && neither.pointer.is_none() && !neither.click);
        assert_eq!(neither.time, Some(2.5), "every layer keeps the clock");
    }
}
