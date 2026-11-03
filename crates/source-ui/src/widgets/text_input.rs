//! Text input focus hierarchy and blinking insertion caret controllers.

/// Under Source VGUI hierarchy semantics, a child control (such as a text entry field) only holds
/// active keyboard focus and displays its active focus visuals (such as focus edge and blinking
/// insertion caret) when *both* conditions are met:
/// 1. Its parent window/dialog currently holds top-level window focus.
/// 2. The child control itself is currently focused / listening for keyboard events.
///
/// When the parent dialog loses focus (e.g. another window is raised, or a modal subdialog/confirmation
/// appears on top), child inputs immediately become inactive, hiding their carets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct InputFocus {
    /// Whether the parent dialog or window holds active window focus.
    pub parent_focused: bool,
    /// Whether this specific child element has keyboard focus / is listening for input.
    pub child_focused: bool,
}

impl InputFocus {
    /// Creates a new `InputFocus` describing parent and child focus states.
    pub const fn new(parent_focused: bool, child_focused: bool) -> Self {
        Self {
            parent_focused,
            child_focused,
        }
    }

    /// Both parent and child are out of focus.
    pub const fn unfocused() -> Self {
        Self {
            parent_focused: false,
            child_focused: false,
        }
    }

    /// Parent window is in focus, but this child control is not focused.
    pub const fn parent_only() -> Self {
        Self {
            parent_focused: true,
            child_focused: false,
        }
    }

    /// Whether this input element has active keyboard focus (both parent and child are focused).
    pub const fn is_active(&self) -> bool {
        self.parent_focused && self.child_focused
    }

    /// Updates parent focus state.
    pub fn set_parent(&mut self, parent_focused: bool) {
        self.parent_focused = parent_focused;
    }

    /// Updates child focus state.
    pub fn set_child(&mut self, child_focused: bool) {
        self.child_focused = child_focused;
    }

    /// Returns a copy with updated parent focus state.
    pub const fn with_parent(self, parent_focused: bool) -> Self {
        Self {
            parent_focused,
            child_focused: self.child_focused,
        }
    }

    /// Returns a copy with updated child focus state.
    pub const fn with_child(self, child_focused: bool) -> Self {
        Self {
            parent_focused: self.parent_focused,
            child_focused,
        }
    }
}

/// Blinking insertion caret (cursor) timing controller for text inputs.
///
/// Models the authentic Source SDK (`vgui_controls/TextEntry.cpp`) caret blink cycle.
/// The default toggle frequency is 2.0 Hz (500 ms on, 500 ms off, 1.0 s full cycle).
///
/// A caret is only visible when the input has active focus (`InputFocus::is_active`).
/// When out of focus, it is unconditionally hidden.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BlinkingCaret {
    /// Blink toggle rate in Hz (toggles per second). 2.0 Hz means 500 ms per phase.
    pub rate: f64,
}

impl Default for BlinkingCaret {
    fn default() -> Self {
        Self {
            rate: Self::DEFAULT_RATE,
        }
    }
}

impl BlinkingCaret {
    /// Authentic Source 2.0 Hz toggle rate (500 ms visible, 500 ms hidden).
    pub const DEFAULT_RATE: f64 = 2.0;

    /// Creates a caret controller with the specified blink rate.
    pub const fn new(rate: f64) -> Self {
        Self { rate }
    }

    /// Evaluates whether the caret is in its "on" (visible) phase at timestamp `time` (seconds).
    ///
    /// Phase 0 (at time 0.0s..0.5s) is visible; Phase 1 (0.5s..1.0s) is hidden, repeating periodically.
    pub fn is_phase_on(&self, time: f64) -> bool {
        if !time.is_finite() || self.rate <= 0.0 {
            return true;
        }
        let t = time.max(0.0);
        ((t * self.rate).floor() as u64).is_multiple_of(2)
    }

    /// Determines whether the vertical caret should be drawn at `time`, combining
    /// parent-child focus hierarchy with the periodic blink cycle.
    ///
    /// Returns `true` if and only if:
    /// 1. `focus.is_active()` is `true` (both parent dialog and child input element are focused).
    /// 2. `self.is_phase_on(time)` is `true` (current blink phase is visible).
    pub fn is_visible(&self, focus: InputFocus, time: f64) -> bool {
        focus.is_active() && self.is_phase_on(time)
    }
}

/// Model describing text input focus and blinking cursor presentation.
#[derive(Clone, Copy, Debug, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct TextInputModel {
    /// Parent-child focus state.
    pub focus: InputFocus,
    /// Caret blinking controller.
    pub caret: BlinkingCaret,
    /// Optional timestamp of last user typing/keystroke interaction to restart blink phase.
    pub last_interaction: Option<f64>,
}

impl TextInputModel {
    /// Creates a new `TextInputModel` with the given parent and child focus states and default caret rate.
    pub const fn new(parent_focused: bool, child_focused: bool) -> Self {
        Self {
            focus: InputFocus::new(parent_focused, child_focused),
            caret: BlinkingCaret::new(BlinkingCaret::DEFAULT_RATE),
            last_interaction: None,
        }
    }

    /// Sets the latest user interaction timestamp (e.g. typing a character) to restart the caret blink.
    pub fn with_interaction(mut self, time: Option<f64>) -> Self {
        self.last_interaction = time;
        self
    }

    /// Records user interaction at `time` to reset caret blink cycle (SDK `ResetCursorBlink`).
    pub fn record_interaction(&mut self, time: f64) {
        self.last_interaction = Some(time);
    }

    /// Whether this input element has active keyboard focus.
    pub const fn is_focused(&self) -> bool {
        self.focus.is_active()
    }

    /// Whether the focus edge border (`TextEntry.FocusEdgeColor`) should be drawn.
    ///
    /// The focus edge is continuously drawn while focused, independent of caret blink phase.
    pub const fn should_draw_focus_edge(&self) -> bool {
        self.focus.is_active()
    }

    /// Whether the blinking caret vertical line should be drawn at `time`.
    pub fn should_draw_caret(&self, time: f64) -> bool {
        if !self.focus.is_active() {
            return false;
        }
        let elapsed = match self.last_interaction {
            Some(last) => (time - last).max(0.0),
            None => time.max(0.0),
        };
        self.caret.is_phase_on(elapsed)
    }

    /// Updates parent focus state.
    pub fn set_parent_focused(&mut self, parent_focused: bool) {
        self.focus.set_parent(parent_focused);
    }

    /// Updates child focus state.
    pub fn set_child_focused(&mut self, child_focused: bool) {
        self.focus.set_child(child_focused);
    }
}
