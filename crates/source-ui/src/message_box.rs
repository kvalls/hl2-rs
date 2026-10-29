//! VGUI `MessageBox` / `QueryBox`: a title, a message and OK (+ optional Cancel), laid out by
//! the SDK `vgui::MessageBox::PerformLayout` rules.

use crate::canvas::Rect;
use crate::widgets::{
    close_button_rect, ButtonState, ButtonTracker, Painter, CLIENT_INSET_X, CLIENT_TOP, FONT_TALL,
};
use glam::{vec2, Vec2};

/// A VGUI `MessageBox`/`QueryBox`: title, one centered line and OK (+ optional Cancel).
#[derive(Clone, Debug, PartialEq)]
pub struct MessageBox {
    pub title: String,
    pub text: String,
    pub ok: String,
    pub cancel: Option<String>,
    pub armed: Option<usize>,
    pub pressed: Option<usize>,
    pub has_close: bool,
    pub escape_held: bool,
    last_armed: Option<usize>,
    sounds: Vec<String>,
    tracker: ButtonTracker<usize>,
    /// Button with keyboard focus: OK when the box opens (SDK `MessageBox` focuses its OK button).
    focus: usize,
    key_press: crate::widgets::KeyPress<usize>,
    /// The focus box shows after Tab / Shift+Tab only.
    focus_visible: bool,
    /// Text the box is sized for, when it differs from the shown text.
    layout_text: Option<String>,
}

/// Message box geometry in layout units relative to the box.
#[derive(Clone, Debug, PartialEq)]
pub struct MessageBoxLayout {
    pub size: Vec2,
    pub text_at: Vec2,
    pub buttons: Vec<Rect>,
    pub close: Option<Rect>,
}

impl MessageBox {
    pub fn new(title: String, text: String, ok: String, cancel: Option<String>) -> Self {
        Self {
            title,
            text,
            ok,
            cancel,
            armed: None,
            pressed: None,
            has_close: false,
            escape_held: false,
            last_armed: None,
            sounds: Vec::new(),
            tracker: ButtonTracker::capturing(),
            focus: 0,
            key_press: crate::widgets::KeyPress::default(),
            focus_visible: false,
            layout_text: None,
        }
    }

    /// Configures whether the message box displays a close (X) button in the title bar.
    pub fn with_close(mut self, has_close: bool) -> Self {
        self.has_close = has_close;
        self
    }

    /// Drains any queued UI sound cues (e.g. rollover or release activation).
    pub fn take_sounds(&mut self) -> Vec<String> {
        std::mem::take(&mut self.sounds)
    }

    /// Starts with the keyboard focus on button `index` (0 = OK, 1 = Cancel).
    pub fn with_focus(mut self, index: usize) -> Self {
        self.focus = index;
        self
    }

    /// Sizes the box for `text` instead of the shown text (a countdown keeps its size).
    pub fn with_layout_text(mut self, text: String) -> Self {
        self.layout_text = Some(text);
        self
    }

    /// Replaces the shown text, keeping the box's state.
    pub fn set_text(&mut self, text: String) {
        self.text = text;
    }

    /// Box size from the message width plus 100 in both directions, at least wide enough for twice
    /// the buttons plus 30; buttons sit at the quarter points, 15 above the client bottom (the SDK
    /// `vgui::MessageBox::PerformLayout` rules).
    pub fn layout(&self, measure: impl Fn(&str) -> f32) -> MessageBoxLayout {
        let lines: Vec<&str> = self
            .layout_text
            .as_deref()
            .unwrap_or(&self.text)
            .lines()
            .collect();
        let line_count = lines.len().max(1);
        let text_w = lines.iter().map(|l| measure(l)).fold(0.0, f32::max).ceil();
        let button = |label: &str| {
            vec2(
                (measure(label) + 10.).ceil().max(64.),
                (FONT_TALL + 10.).max(24.),
            )
        };
        let ok = button(&self.ok);
        let cancel = self.cancel.as_deref().map(button);
        let width = (text_w + 100.).max((ok.x + cancel.map_or(0., |c| c.x)) * 2. + 30.);
        let size = vec2(width, FONT_TALL * line_count as f32 + 100.);
        let right = size.x - CLIENT_INSET_X;
        let bottom = size.y - 6.;
        let y = bottom - ok.y - 15.;
        let text_at = vec2(
            (right / 2. - text_w / 2. + CLIENT_INSET_X).floor(),
            CLIENT_TOP + 5.,
        );
        let buttons = match cancel {
            None => vec![Rect::new(
                (right / 2. - ok.x / 2. + CLIENT_INSET_X).floor(),
                y,
                ok.x,
                ok.y,
            )],
            Some(cancel) => vec![
                Rect::new(
                    (right / 4. - ok.x / 2. + CLIENT_INSET_X).floor(),
                    y,
                    ok.x,
                    ok.y,
                ),
                Rect::new(
                    (3. * right / 4. - ok.x / 2. + CLIENT_INSET_X).floor(),
                    y,
                    cancel.x,
                    cancel.y,
                ),
            ],
        };
        let close = self.has_close.then(|| close_button_rect(size));
        MessageBoxLayout {
            size,
            text_at,
            buttons,
            close,
        }
    }

    /// Pointer in box-relative layout units. Returns `Some(true)` for OK, `Some(false)` for Cancel.
    pub fn input(
        &mut self,
        layout: &MessageBoxLayout,
        pointer: Option<Vec2>,
        down: bool,
        click: bool,
        released: bool,
        enter: bool,
    ) -> Option<bool> {
        let hovered = pointer.and_then(|p| {
            layout
                .buttons
                .iter()
                .position(|b| b.contains(p))
                .or_else(|| {
                    layout
                        .close
                        .and_then(|c| c.contains(p).then_some(layout.buttons.len()))
                })
        });
        let activated = self.tracker.update(hovered, down, click, released);
        self.pressed = self.tracker.pressed();
        self.armed = self.tracker.armable(hovered);
        self.last_armed = self.armed;
        let enter_activated = enter.then_some(0);
        let final_activated = activated.or(enter_activated);
        if final_activated.is_some() {
            self.sounds.push("ui/buttonclickrelease.wav".to_string());
        }
        match final_activated {
            Some(0) => Some(true),
            Some(_) => Some(false),
            None => None,
        }
    }

    /// Keyboard: Tab / Shift+Tab move focus between the buttons (wrapping), Space or Enter
    /// presses the focused one (activation on release, SDK `Button`). Returns `Some(true)` for OK,
    /// `Some(false)` for Cancel.
    pub fn keys(&mut self, input: &crate::input::Input) -> Option<bool> {
        use crate::input::Key;
        let count = 1 + usize::from(self.cancel.is_some());
        if input.keys.contains(&Key::Tab) {
            let order: Vec<usize> = (0..count).collect();
            self.focus =
                crate::widgets::focus_step(Some(self.focus), &order, !input.shift).unwrap_or(0);
            self.focus_visible = true;
            self.key_press.reset();
            return None;
        }
        let key = [Key::Space, Key::Enter]
            .into_iter()
            .find(|k| input.keys.contains(k));
        let activated = self
            .key_press
            .update(Some(self.focus), key, |k| input.held_keys.contains(&k))?;
        self.sounds.push("ui/buttonclickrelease.wav".to_string());
        Some(activated == 0)
    }

    pub fn draw(&self, painter: &Painter, layout: &MessageBoxLayout) {
        let close_state = layout.close.map(|_| {
            let close_index = layout.buttons.len();
            ButtonState {
                armed: self.armed == Some(close_index),
                pressed: self.pressed == Some(close_index) || (self.has_close && self.escape_held),
                enabled: true,
                default: false,
            }
        });
        painter.frame_with_close(layout.size, &self.title, close_state);
        painter.text(&self.text, layout.text_at, painter.theme.text, false);
        let labels = std::iter::once(&self.ok).chain(self.cancel.as_ref());
        for (index, (rect, label)) in layout.buttons.iter().zip(labels).enumerate() {
            let is_cancel = index == 1 && self.cancel.is_some() && !self.has_close;
            let armed = self.armed == Some(index) || (is_cancel && self.escape_held);
            let pressed = self.pressed == Some(index)
                || self.key_press.pressed() == Some(index)
                || (is_cancel && self.escape_held);
            painter.button(
                *rect,
                label,
                ButtonState {
                    armed,
                    pressed,
                    enabled: true,
                    default: false,
                },
            );
            if self.focus_visible && index == self.focus {
                painter.focus_box(*rect);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_box_buttons_take_keyboard_focus() {
        use crate::input::{Input, Key};
        let mut quit = MessageBox::new("T".into(), "x".into(), "OK".into(), Some("Cancel".into()));
        let key = |k: Key, shift: bool| Input {
            keys: [k].into(),
            shift,
            ..Default::default()
        };
        assert_eq!(quit.keys(&key(Key::Tab, false)), None);
        assert_eq!(
            quit.keys(&key(Key::Enter, false)),
            Some(false),
            "Cancel has focus"
        );
        assert_eq!(quit.keys(&key(Key::Tab, true)), None);
        assert_eq!(quit.keys(&key(Key::Space, false)), Some(true), "back on OK");
        let held = Input {
            keys: [Key::Enter].into(),
            held_keys: [Key::Enter].into(),
            ..Default::default()
        };
        assert_eq!(quit.keys(&held), None, "a held Enter waits for the release");
        assert_eq!(quit.keys(&Input::default()), Some(true));
    }

    #[test]
    fn message_box_sound_events_hover_and_release() {
        let mut quit = MessageBox::new(
            "Title".into(),
            "Text".into(),
            "OK".into(),
            Some("Cancel".into()),
        );
        let layout = quit.layout(|_| 100.);
        let center = |r: Rect| Some(vec2(r.x + r.w / 2., r.y + r.h / 2.));

        // 1. Moving from outside to over button 0 emits no rollover sound (dialog button)
        quit.input(
            &layout,
            center(layout.buttons[0]),
            false,
            false,
            false,
            false,
        );
        assert!(quit.take_sounds().is_empty());

        // 2. Staying over button 0 on next frame does not emit sound
        quit.input(
            &layout,
            center(layout.buttons[0]),
            false,
            false,
            false,
            false,
        );
        assert!(quit.take_sounds().is_empty());

        // 3. Moving to button 1 emits no rollover sound
        quit.input(
            &layout,
            center(layout.buttons[1]),
            false,
            false,
            false,
            false,
        );
        assert!(quit.take_sounds().is_empty());

        // 4. Moving outside emits nothing
        quit.input(&layout, None, false, false, false, false);
        assert!(quit.take_sounds().is_empty());

        // 5. Press and release on button 0 emits release sound on activation (no rollover on entry)
        quit.input(&layout, center(layout.buttons[0]), true, true, false, false);
        assert!(quit.take_sounds().is_empty());
        quit.input(
            &layout,
            center(layout.buttons[0]),
            false,
            false,
            true,
            false,
        );
        assert_eq!(quit.take_sounds(), vec!["ui/buttonclickrelease.wav"]);

        // 6. Press, move away, and release emits no release sound
        quit.input(&layout, center(layout.buttons[0]), true, true, false, false);
        let _ = quit.take_sounds();
        quit.input(&layout, None, true, false, false, false);
        quit.input(&layout, None, false, false, true, false);
        assert!(quit.take_sounds().is_empty());
    }

    #[test]
    fn message_box_close_button_data_driven_cancel() {
        let mut msg_box = MessageBox::new(
            "Title".into(),
            "Text".into(),
            "OK".into(),
            Some("Cancel".into()),
        );
        // By default, MessageBox has no close button (matches Source SDK SetCloseButtonVisible(false))
        assert!(!msg_box.has_close);
        let layout_no_close = msg_box.layout(|_| 100.);
        assert_eq!(layout_no_close.close, None);

        // Data-driven close button enabled
        msg_box = msg_box.with_close(true);
        assert!(msg_box.has_close);
        let layout_with_close = msg_box.layout(|_| 100.);
        let close_rect = layout_with_close.close.expect("close button rect");
        assert_eq!(close_rect, close_button_rect(layout_with_close.size));

        let center = |r: Rect| Some(vec2(r.x + r.w / 2., r.y + r.h / 2.));

        // Hover over close button: does not emit rollover sound (dialog button)
        msg_box.input(
            &layout_with_close,
            center(close_rect),
            false,
            false,
            false,
            false,
        );
        assert!(msg_box.take_sounds().is_empty());

        // Click and release on close button: activates Cancel outcome (Some(false))
        msg_box.input(
            &layout_with_close,
            center(close_rect),
            true,
            true,
            false,
            false,
        );
        let outcome = msg_box.input(
            &layout_with_close,
            center(close_rect),
            false,
            false,
            true,
            false,
        );
        assert_eq!(outcome, Some(false), "Close button behaves like Cancel");
        assert_eq!(msg_box.take_sounds(), vec!["ui/buttonclickrelease.wav"]);
    }

    #[test]
    fn message_box_escape_held_arms_cancel_when_no_close_button() {
        use crate::canvas::Canvas;
        use crate::widgets::{Fonts, Theme};

        let mut msg_box = MessageBox::new(
            "Quit".into(),
            "Quit game?".into(),
            "Quit".into(),
            Some("Cancel".into()),
        );
        let layout = msg_box.layout(|_| 100.);
        let canvas = Canvas::default();
        let fonts = Fonts::default();
        let theme = Theme::default();
        let painter = Painter {
            canvas: &canvas,
            fonts: &fonts,
            theme: &theme,
            origin: Vec2::ZERO,
            scale: 1.,
        };

        // When escape_held is false:
        assert!(!msg_box.escape_held);
        msg_box.draw(&painter, &layout);
        let normal_quads = canvas.drain();

        // When escape_held is true on a box with cancel and no close button:
        msg_box.escape_held = true;
        msg_box.draw(&painter, &layout);
        let armed_quads = canvas.drain();
        assert!(!armed_quads.is_empty());
        let normal_colors: Vec<_> = normal_quads
            .iter()
            .map(|q| (q.destination, q.color))
            .collect();
        let armed_colors: Vec<_> = armed_quads
            .iter()
            .map(|q| (q.destination, q.color))
            .collect();
        assert_ne!(normal_colors, armed_colors);

        // When box has close button, escape_held does not arm/depress cancel button
        let mut msg_box_close = MessageBox::new(
            "Dialog".into(),
            "Text".into(),
            "OK".into(),
            Some("Cancel".into()),
        )
        .with_close(true);
        let layout_close = msg_box_close.layout(|_| 100.);
        msg_box_close.draw(&painter, &layout_close);
        let normal_close_quads = canvas.drain();
        msg_box_close.escape_held = true;
        msg_box_close.draw(&painter, &layout_close);
        let armed_close_quads = canvas.drain();
        let normal_close_colors: Vec<_> = normal_close_quads
            .iter()
            .map(|q| (q.destination, q.color))
            .collect();
        let armed_close_colors: Vec<_> = armed_close_quads
            .iter()
            .map(|q| (q.destination, q.color))
            .collect();
        // Since cancel is not armed when has_close is true, button borders remain identical
        assert_eq!(normal_close_colors, armed_close_colors);
    }
}
