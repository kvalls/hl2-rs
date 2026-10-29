//! Non-modal dialog windows over a base layer (the main or pause menu, or the game), as retail
//! VGUI frames behave: the Options dialog and the developer console stay open while the menu
//! behind them keeps working. This model keeps their z-order, which layer has the keyboard
//! focus, which layer the pointer goes to, and the focus fade of each window. It knows nothing
//! of what the layers draw; callers pass hit tests and the time.
//!
//! - A press on a window raises it to the top and focuses it (SDK `Frame::Activate` on a mouse
//!   press); a press on the base focuses the base, leaving the windows where they are. A base
//!   that takes no pointer (gameplay) ignores presses and never takes the focus.
//! - A press belongs to the layer it started on until the button is released, so it never
//!   moves between layers while dragged.
//! - A modal layer of a window (a sub-dialog, an open list) takes every press.
//! - The focused window fades its background in over `Frame.FocusTransitionEffectTime`
//!   (SDK `Frame::OnFrameFocusChanged`); an unfocused one fades out.

use glam::Vec2;

/// What the toolkit needs of an app's window ids.
pub trait WindowId: Copy + Eq + std::fmt::Debug {
    /// The developer console's window.
    const CONSOLE: Self;
    /// Whether this window is modal (captures focus until closed).
    fn is_modal(&self) -> bool;
}

/// A layer that can take the pointer or the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer<W> {
    /// The menu (or the game) under every window.
    Base,
    Window(W),
}

/// The left button this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pointer {
    pub at: Option<Vec2>,
    /// Went down this frame.
    pub click: bool,
    /// Is down.
    pub held: bool,
    /// Went up this frame.
    pub released: bool,
}

/// How far a window is faded towards its focused look (0 out of focus, 1 in focus).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fade {
    pub from: f32,
    pub to: f32,
    pub start: f64,
}

impl Fade {
    pub const fn new(from: f32, to: f32, start: f64) -> Self {
        Self { from, to, start }
    }

    pub fn at(&self, now: f64, duration: f32) -> f32 {
        if duration <= 0. {
            return self.to;
        }
        let t = (((now - self.start) / duration as f64) as f32).clamp(0., 1.);
        self.from + (self.to - self.from) * t
    }

    pub fn is_finished(&self, now: f64, duration: f32) -> bool {
        if duration <= 0. {
            return true;
        }
        (now - self.start) as f32 >= duration
    }
}

/// The state of an open or fading dialog window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowRecord<W> {
    pub window: W,
    /// How far the window is faded towards its focused look (0 out of focus, 1 in focus).
    pub focus_fade: Fade,
    /// How far the window is faded towards full visibility (0 hidden/closed, 1 fully visible).
    pub alpha_fade: Fade,
    /// Whether this window has begun closing and is fading out.
    pub closing: bool,
}

/// The open windows, bottom to top, and the focus.
#[derive(Clone, Debug)]
pub struct Windows<W> {
    order: Vec<WindowRecord<W>>,
    /// Fade state of the base (main menu / gameplay). 1.0 when base is focused, 0.0 when out of focus.
    base_fade: Fade,
    /// The base has the focus although windows are open.
    base_focused: bool,
    /// The layer the held button belongs to.
    press: Option<Layer<W>>,
    /// An in-progress press began as an unfocused window click to activate/focus it.
    activating_press: bool,
    /// Whether the current frame's pointer input is part of an activating click.
    activating_click: bool,
    /// Time of the current frame, stamped on focus changes.
    now: f64,
    /// `Frame.FocusTransitionEffectTime` and `Frame.TransitionEffectTime` in seconds.
    pub fade_time: f32,
    /// Unfocused alpha factor (ratio of Frame.OutOfFocusBgColor to Frame.BgColor).
    pub unfocused_alpha: f32,
}

impl<W> Default for Windows<W> {
    fn default() -> Self {
        Self {
            order: Vec::new(),
            base_fade: Fade {
                from: 1.,
                to: 1.,
                start: 0.,
            },
            base_focused: false,
            press: None,
            activating_press: false,
            activating_click: false,
            now: 0.,
            fade_time: 0.3,
            unfocused_alpha: 0.3,
        }
    }
}

impl<W: WindowId> Windows<W> {
    /// No windows; focus and window fades take `fade_time` seconds (`Frame.FocusTransitionEffectTime`)
    /// and an unfocused window keeps `unfocused_alpha` of its focused look.
    pub fn new(fade_time: f32, unfocused_alpha: f32) -> Self {
        Self {
            fade_time,
            unfocused_alpha,
            ..Self::default()
        }
    }

    /// Sets the time used for focus changes from now on (seconds, monotonic).
    pub fn set_time(&mut self, now: f64) {
        self.now = now;
    }

    /// Whether `window` is present in the window stack (open or currently fading out).
    pub fn is_open(&self, window: W) -> bool {
        self.order.iter().any(|r| r.window == window)
    }

    /// Whether `window` is active (open and not closing).
    pub fn is_active(&self, window: W) -> bool {
        self.order.iter().any(|r| r.window == window && !r.closing)
    }

    /// Whether `window` is currently closing and fading out.
    pub fn is_closing(&self, window: W) -> bool {
        self.order.iter().any(|r| r.window == window && r.closing)
    }

    /// Whether there are no active (non-closing) windows.
    pub fn is_empty(&self) -> bool {
        !self.order.iter().any(|r| !r.closing)
    }

    /// Whether any modal window (SubDialog or Confirmation) is actively open.
    pub fn has_modal(&self) -> bool {
        self.order.iter().any(|r| r.window.is_modal() && !r.closing)
    }

    /// Whether the specified layer holds focus.
    pub fn is_focused(&self, layer: Layer<W>) -> bool {
        self.focus() == layer
    }

    /// Whether the current frame's pointer interaction is an activating click
    /// that raised or focused an inactive window or the base.
    pub fn activating_click(&self) -> bool {
        self.activating_click
    }

    /// All windows currently present (including those fading out), bottom to top.
    pub fn order(&self) -> impl DoubleEndedIterator<Item = W> + '_ {
        self.order.iter().map(|r| r.window)
    }

    /// All active windows (excluding those fading out), bottom to top.
    pub fn active_order(&self) -> impl DoubleEndedIterator<Item = W> + '_ {
        self.order.iter().filter(|r| !r.closing).map(|r| r.window)
    }

    /// The topmost active (non-closing) window.
    pub fn top(&self) -> Option<W> {
        self.order
            .iter()
            .rev()
            .find(|r| !r.closing)
            .map(|r| r.window)
    }

    /// The layer with the focus: exactly one element holds focus at any time.
    /// If a modal window is open, the topmost modal window holds focus.
    /// Otherwise, the top active window holds focus unless the base took it.
    /// If no windows are active, the base holds focus.
    pub fn focus(&self) -> Layer<W> {
        if self.has_modal() {
            if let Some(modal) = self
                .order
                .iter()
                .rev()
                .find(|r| r.window.is_modal() && !r.closing)
            {
                return Layer::Window(modal.window);
            }
        }
        match self.top() {
            Some(top) if !self.base_focused => Layer::Window(top),
            _ => Layer::Base,
        }
    }

    /// How far the base layer is faded towards its focused look (0 out of focus, 1 in focus).
    pub fn base_focus_amount(&self, now: f64) -> f32 {
        self.base_fade.at(now, self.fade_time)
    }

    /// Opens `window` on top with the focus (or raises it if it is open).
    pub fn open(&mut self, window: W) {
        let now = self.now;
        let rec = if let Some(mut existing) = self.take_record(window) {
            if existing.closing {
                let current_alpha = existing.alpha_fade.at(now, self.fade_time);
                existing.alpha_fade = Fade::new(current_alpha, 1.0, now);
                existing.closing = false;
            }
            existing
        } else {
            WindowRecord {
                window,
                focus_fade: Fade::new(1.0, 1.0, now),
                alpha_fade: Fade::new(0.0, 1.0, now),
                closing: false,
            }
        };
        self.order.push(rec);
        self.set_focus(Layer::Window(window));
    }

    /// Raises an open `window` to the top and focuses it.
    pub fn raise(&mut self, window: W) {
        if self.is_open(window) {
            self.open(window);
        }
    }

    fn drop_press_for(&mut self, window: W) {
        if self.press == Some(Layer::Window(window)) {
            self.press = None;
            self.activating_press = false;
            self.activating_click = false;
        }
    }

    /// Closes `window`. If `fade_time > 0`, it begins fading out over `fade_time` while focus
    /// immediately restores to the next active surface.
    pub fn close(&mut self, window: W) {
        if self.fade_time <= 0. {
            self.close_immediate(window);
            return;
        }

        let now = self.now;
        if let Some(rec) = self
            .order
            .iter_mut()
            .find(|r| r.window == window && !r.closing)
        {
            rec.closing = true;
            let current_alpha = rec.alpha_fade.at(now, self.fade_time);
            rec.alpha_fade = Fade::new(current_alpha, 0.0, now);
            let current_focus = rec.focus_fade.at(now, self.fade_time);
            rec.focus_fade = Fade::new(current_focus, 0.0, now);
        }
        self.drop_press_for(window);
        let focus = self.focus();
        self.set_focus(focus);
    }

    /// Immediately removes `window` without fading.
    pub fn close_immediate(&mut self, window: W) {
        self.take_record(window);
        self.drop_press_for(window);
        if !self.order.iter().any(|r| !r.closing) {
            self.base_focused = false;
        }
        let focus = self.focus();
        self.set_focus(focus);
    }

    /// Closes every window immediately.
    pub fn clear(&mut self) {
        self.order.clear();
        self.base_focused = false;
        self.press = None;
        self.activating_press = false;
        self.activating_click = false;
        let now = self.now;
        let duration = self.fade_time;
        if self.base_fade.to != 1. {
            self.base_fade = Fade {
                from: self.base_fade.at(now, duration),
                to: 1.,
                start: now,
            };
        }
    }

    /// Cleans up any windows that have completed their closing fade-out.
    /// Returns the list of windows that finished closing.
    pub fn clean_closed(&mut self) -> Vec<W> {
        let now = self.now;
        let duration = self.fade_time;
        let mut closed = Vec::new();
        self.order.retain(|r| {
            if r.closing && r.alpha_fade.is_finished(now, duration) {
                closed.push(r.window);
                false
            } else {
                true
            }
        });
        if !self.order.iter().any(|r| !r.closing) {
            self.base_focused = false;
        }
        closed
    }

    /// Gives the keyboard focus to the base; the windows keep their order.
    pub fn focus_base(&mut self) {
        if !self.has_modal() && !self.is_empty() {
            self.set_focus(Layer::Base);
        }
    }

    /// How far `window` is faded towards full visibility (0 hidden, 1 visible).
    pub fn window_alpha(&self, window: W, now: f64) -> f32 {
        self.order
            .iter()
            .find(|r| r.window == window)
            .map_or(0., |r| r.alpha_fade.at(now, self.fade_time))
    }

    /// How far `window` is faded towards its focused look at `now` (0..1).
    pub fn focus_amount(&self, window: W, now: f64) -> f32 {
        self.order
            .iter()
            .find(|r| r.window == window)
            .map_or(0., |r| r.focus_fade.at(now, self.fade_time))
    }

    /// The combined composite alpha multiplier for drawing `window` (combining visibility fade
    /// and unfocused translucency).
    pub fn window_composite_alpha(&self, window: W, now: f64) -> f32 {
        let vis = self.window_alpha(window, now);
        let focus = self.focus_amount(window, now);
        let factor = self.unfocused_alpha + (1.0 - self.unfocused_alpha) * focus;
        vis * factor
    }

    /// Which layer gets the pointer this frame, if any: the layer a held press belongs to,
    /// else a modal window, else the topmost window `hit` says is under the pointer, else the
    /// base when it takes the pointer; with no pointer position, the focused layer. A press focuses its layer and raises a window.
    pub fn route(
        &mut self,
        pointer: Pointer,
        modal: Option<W>,
        base_takes_pointer: bool,
        hit: impl Fn(W, Vec2) -> bool,
    ) -> Option<Layer<W>> {
        let pressed = self
            .press
            .filter(|_| pointer.held || pointer.released)
            .filter(|layer| match layer {
                Layer::Window(w) => self.is_active(*w),
                Layer::Base => true,
            });
        let layer = pressed.or_else(|| {
            if let Some(top_modal) = self
                .order
                .iter()
                .rev()
                .find(|r| r.window.is_modal() && !r.closing)
                .map(|r| r.window)
            {
                return Some(Layer::Window(top_modal));
            }
            if let Some(window) = modal.filter(|w| self.is_active(*w)) {
                return Some(Layer::Window(window));
            }
            // Without a position (scripted or keyboard-only input) the focused layer gets the
            // button and the wheel.
            let Some(at) = pointer.at else {
                return match self.focus() {
                    Layer::Base => base_takes_pointer.then_some(Layer::Base),
                    window => Some(window),
                };
            };
            self.order
                .iter()
                .rev()
                .filter(|r| !r.closing)
                .map(|r| r.window)
                .find(|w| hit(*w, at))
                .map(Layer::Window)
                .or(base_takes_pointer.then_some(Layer::Base))
        });
        let prior_focus = self.focus();
        let was_activating = self.activating_press;
        let activating = if pointer.click && pressed.is_none() {
            match layer {
                Some(Layer::Window(window)) => {
                    let changed = prior_focus != Layer::Window(window);
                    self.raise(window);
                    changed
                }
                Some(Layer::Base) => {
                    self.focus_base();
                    false
                }
                None => false,
            }
        } else {
            false
        };
        let ending = pointer.released || !(pointer.click || pointer.held);
        self.press = if ending {
            None
        } else if pointer.click && pressed.is_none() {
            layer
        } else {
            self.press
        };
        self.activating_press = if ending {
            false
        } else if activating {
            true
        } else {
            self.activating_press
        };
        self.activating_click = activating || was_activating;
        layer
    }

    fn take_record(&mut self, window: W) -> Option<WindowRecord<W>> {
        let index = self.order.iter().position(|r| r.window == window)?;
        Some(self.order.remove(index))
    }

    fn set_focus(&mut self, focus: Layer<W>) {
        self.base_focused = focus == Layer::Base;
        let now = self.now;
        let duration = self.fade_time;

        // Base focus fade
        let has_active = self.order.iter().any(|r| !r.closing);
        let base_to = if self.base_focused || !has_active {
            1.
        } else {
            0.
        };
        if self.base_fade.to != base_to {
            self.base_fade = Fade {
                from: self.base_fade.at(now, duration),
                to: base_to,
                start: now,
            };
        }

        // Each window focus fade
        for rec in &mut self.order {
            if rec.closing {
                continue;
            }
            let to = if focus == Layer::Window(rec.window) {
                1.
            } else {
                0.
            };
            if rec.focus_fade.to != to {
                rec.focus_fade = Fade {
                    from: rec.focus_fade.at(now, duration),
                    to,
                    start: now,
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Window ids like an app's: two dialogs, the console and two modal layers.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Window {
        Options,
        Console,
        SubDialog,
        Confirmation,
    }

    impl WindowId for Window {
        const CONSOLE: Self = Window::Console;
        fn is_modal(&self) -> bool {
            matches!(self, Window::SubDialog | Window::Confirmation)
        }
    }
    use glam::vec2;

    /// Options covers x 0..100, the console x 50..150; the menu is everywhere else.
    fn hit(window: Window, p: Vec2) -> bool {
        match window {
            Window::Options => p.x < 100.,
            Window::Console => (50. ..150.).contains(&p.x),
            Window::SubDialog => (20. ..80.).contains(&p.x),
            Window::Confirmation => (30. ..70.).contains(&p.x),
        }
    }

    fn click(x: f32) -> Pointer {
        Pointer {
            at: Some(vec2(x, 0.)),
            click: true,
            held: true,
            released: false,
        }
    }

    fn held(x: f32) -> Pointer {
        Pointer {
            at: Some(vec2(x, 0.)),
            held: true,
            ..Default::default()
        }
    }

    fn release(x: f32) -> Pointer {
        Pointer {
            at: Some(vec2(x, 0.)),
            released: true,
            ..Default::default()
        }
    }

    fn hover(x: f32) -> Pointer {
        Pointer {
            at: Some(vec2(x, 0.)),
            ..Default::default()
        }
    }

    fn both() -> Windows<Window> {
        let mut windows = Windows::default();
        windows.open(Window::Options);
        windows.open(Window::Console);
        windows
    }

    #[test]
    fn the_pointer_goes_to_the_topmost_window_under_it_else_the_base() {
        let mut w = both();
        let route = |w: &mut Windows<Window>, x| w.route(hover(x), None, true, hit);
        assert_eq!(
            route(&mut w, 75.),
            Some(Layer::Window(Window::Console)),
            "overlap: top"
        );
        assert_eq!(route(&mut w, 25.), Some(Layer::Window(Window::Options)));
        assert_eq!(route(&mut w, 200.), Some(Layer::Base));
        assert_eq!(
            w.focus(),
            Layer::Window(Window::Console),
            "hovering never focuses"
        );
    }

    #[test]
    fn a_press_raises_and_focuses_its_window_or_focuses_the_base() {
        let mut w = both();
        w.route(click(25.), None, true, hit);
        w.route(release(25.), None, true, hit);
        assert_eq!(w.top(), Some(Window::Options));
        assert_eq!(w.focus(), Layer::Window(Window::Options));
        assert_eq!(
            w.route(hover(75.), None, true, hit),
            Some(Layer::Window(Window::Options)),
            "the raised window now wins the overlap"
        );
        w.route(click(200.), None, true, hit);
        w.route(release(200.), None, true, hit);
        assert_eq!(w.focus(), Layer::Base);
        assert_eq!(
            w.order().collect::<Vec<_>>(),
            [Window::Console, Window::Options],
            "focusing the base keeps the windows' order"
        );
    }

    #[test]
    fn clicking_an_unfocused_window_activates_it_and_subsequent_clicks_are_normal() {
        let mut w = both();
        assert_eq!(w.focus(), Layer::Window(Window::Console));
        assert!(!w.activating_click());

        // Hover over unfocused window: does not activate
        w.route(hover(25.), None, true, hit);
        assert!(!w.activating_click());
        assert_eq!(w.focus(), Layer::Window(Window::Console));

        // Click on unfocused window: activates focus
        w.route(click(25.), None, true, hit);
        assert!(w.activating_click());
        assert_eq!(w.focus(), Layer::Window(Window::Options));

        // Release: still part of the activating gesture
        w.route(release(25.), None, true, hit);
        assert!(w.activating_click());

        // Hover over now-focused window: not activating
        w.route(hover(25.), None, true, hit);
        assert!(!w.activating_click());

        // Next click on the now-focused window: normal click, not activating
        w.route(click(25.), None, true, hit);
        assert!(!w.activating_click());
        w.route(release(25.), None, true, hit);
        assert!(!w.activating_click());
    }

    #[test]
    fn a_press_stays_with_the_layer_it_started_on_until_release() {
        let mut w = both();
        assert_eq!(w.route(click(200.), None, true, hit), Some(Layer::Base));
        assert_eq!(
            w.route(held(75.), None, true, hit),
            Some(Layer::Base),
            "dragged onto a window"
        );
        assert_eq!(
            w.route(release(75.), None, true, hit),
            Some(Layer::Base),
            "released there"
        );
        assert_eq!(
            w.route(hover(75.), None, true, hit),
            Some(Layer::Window(Window::Console)),
            "free again after the release"
        );
        assert_eq!(
            w.route(click(25.), None, true, hit),
            Some(Layer::Window(Window::Options))
        );
        assert_eq!(
            w.route(held(200.), None, true, hit),
            Some(Layer::Window(Window::Options))
        );
        assert_eq!(
            w.route(release(200.), None, true, hit),
            Some(Layer::Window(Window::Options))
        );
        assert_eq!(
            w.focus(),
            Layer::Window(Window::Options),
            "the drag never refocused"
        );
    }

    #[test]
    fn a_modal_window_takes_every_press() {
        let mut w = both();
        assert_eq!(
            w.route(click(200.), Some(Window::Options), true, hit),
            Some(Layer::Window(Window::Options))
        );
        assert_eq!(w.top(), Some(Window::Options), "and is raised by it");
    }

    #[test]
    fn without_a_position_the_focused_layer_gets_the_pointer() {
        let mut w = both();
        let wheel = Pointer::default();
        assert_eq!(
            w.route(wheel, None, true, hit),
            Some(Layer::Window(Window::Console))
        );
        w.focus_base();
        assert_eq!(w.route(wheel, None, true, hit), Some(Layer::Base));
        assert_eq!(
            w.route(wheel, None, false, hit),
            None,
            "gameplay takes no pointer"
        );
    }

    #[test]
    fn a_base_without_pointer_takes_no_press_and_no_focus() {
        let mut w = Windows::default();
        w.open(Window::Console);
        assert_eq!(w.route(click(200.), None, false, hit), None);
        w.route(release(200.), None, false, hit);
        assert_eq!(w.focus(), Layer::Window(Window::Console));
    }

    #[test]
    fn closing_moves_the_focus_to_the_new_top_unless_the_base_had_it() {
        let mut w = both();
        w.close(Window::Console);
        assert_eq!(w.focus(), Layer::Window(Window::Options));
        w.open(Window::Console);
        w.focus_base();
        w.close(Window::Console);
        assert_eq!(w.focus(), Layer::Base);
        w.close(Window::Options);
        assert!(w.is_empty());
        assert_eq!(w.focus(), Layer::Base);
    }

    #[test]
    fn a_press_on_a_window_that_closes_is_forgotten() {
        let mut w = both();
        w.route(click(75.), None, true, hit);
        w.close(Window::Console);
        assert_eq!(
            w.route(held(200.), None, true, hit),
            Some(Layer::Base),
            "the rest of the press is not the console's any more"
        );
    }

    #[test]
    fn focus_fades_linearly_over_the_transition_time() {
        let mut w = Windows {
            fade_time: 0.3,
            ..Default::default()
        };
        w.set_time(10.);
        w.open(Window::Options);
        assert_eq!(w.focus_amount(Window::Options, 10.), 1., "opens focused");
        w.focus_base();
        assert_eq!(w.focus_amount(Window::Options, 10.), 1.);
        assert!((w.focus_amount(Window::Options, 10.15) - 0.5).abs() < 1e-4);
        assert!(w.focus_amount(Window::Options, 10.3) < 1e-4);
        w.set_time(10.15);
        w.raise(Window::Options);
        assert!(
            (w.focus_amount(Window::Options, 10.15) - 0.5).abs() < 1e-4,
            "a refocus mid-fade starts where it was"
        );
        assert!(w.focus_amount(Window::Options, 10.45) > 1. - 1e-4);
        assert_eq!(w.focus_amount(Window::Console, 10.45), 0., "not open");
    }

    #[test]
    fn single_focus_state_machine_across_menus_dialogs_and_subdialogs() {
        let mut w = Windows::new(0.3, 0.3);
        w.set_time(0.0);

        // 1. Main menu alone (focused)
        assert_eq!(w.focus(), Layer::Base);
        assert!(w.is_focused(Layer::Base));
        assert_eq!(w.base_focus_amount(0.0), 1.0);

        // 2. Main menu + dialog (dialog focused, main menu unfocused)
        w.open(Window::Options);
        assert_eq!(w.focus(), Layer::Window(Window::Options));
        assert!(w.is_focused(Layer::Window(Window::Options)));
        assert!(!w.is_focused(Layer::Base));
        assert_eq!(w.focus_amount(Window::Options, 0.0), 1.0);
        // Base fades out towards 0.0
        assert!((w.base_focus_amount(0.3) - 0.0).abs() < 1e-4);

        // 3. Parent dialog + sub-dialog (sub-dialog focused, parent and main menu unfocused)
        w.set_time(1.0);
        w.open(Window::SubDialog);
        assert_eq!(w.focus(), Layer::Window(Window::SubDialog));
        assert!(w.is_focused(Layer::Window(Window::SubDialog)));
        assert!(!w.is_focused(Layer::Window(Window::Options)));
        assert!(!w.is_focused(Layer::Base));
        assert_eq!(w.focus_amount(Window::SubDialog, 1.0), 1.0);
        // Options fades out towards 0.0
        assert!((w.focus_amount(Window::Options, 1.3) - 0.0).abs() < 1e-4);

        // 4. Options + Dev console open simultaneously (strictly one focused)
        let mut w2 = Windows::new(0.3, 0.3);
        w2.set_time(0.0);
        w2.open(Window::Options);
        assert_eq!(w2.focus(), Layer::Window(Window::Options));
        w2.open(Window::Console);
        assert_eq!(w2.focus(), Layer::Window(Window::Console));
        assert!(!w2.is_focused(Layer::Window(Window::Options)));
        // Switch focus back to options (raising it)
        w2.raise(Window::Options);
        assert_eq!(w2.focus(), Layer::Window(Window::Options));
        assert!(!w2.is_focused(Layer::Window(Window::Console)));

        // 5. Sequential closing restoring focus step by step
        // Start from Options -> SubDialog -> Confirmation
        w.set_time(2.0);
        w.open(Window::Confirmation);
        assert_eq!(w.focus(), Layer::Window(Window::Confirmation));
        assert!(!w.is_focused(Layer::Window(Window::SubDialog)));
        assert!(!w.is_focused(Layer::Window(Window::Options)));
        assert!(!w.is_focused(Layer::Base));

        // Close Confirmation -> restores to SubDialog
        w.set_time(3.0);
        w.close(Window::Confirmation);
        assert_eq!(w.focus(), Layer::Window(Window::SubDialog));
        assert!(w.is_focused(Layer::Window(Window::SubDialog)));
        assert!(!w.is_focused(Layer::Window(Window::Options)));
        assert!(!w.is_focused(Layer::Base));

        // Close SubDialog -> restores to Options
        w.set_time(4.0);
        w.close(Window::SubDialog);
        assert_eq!(w.focus(), Layer::Window(Window::Options));
        assert!(w.is_focused(Layer::Window(Window::Options)));
        assert!(!w.is_focused(Layer::Base));

        // Close Options -> restores to Main Menu (Base)
        w.set_time(5.0);
        w.close(Window::Options);
        assert_eq!(w.focus(), Layer::Base);
        assert!(w.is_focused(Layer::Base));
        assert!((w.base_focus_amount(5.3) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn shared_fade_state_and_interpolation_logic_with_deterministic_timesteps() {
        let mut w = Windows::new(0.3, 0.3);
        w.set_time(0.0);

        // 1. Opening fade-in: Window opens at t=0.0
        w.open(Window::Options);
        assert!(w.is_active(Window::Options));
        assert!(!w.is_closing(Window::Options));
        assert_eq!(w.focus(), Layer::Window(Window::Options));
        // At t=0.0: alpha is 0.0, focused
        assert!((w.window_alpha(Window::Options, 0.0) - 0.0).abs() < 1e-4);
        assert!((w.focus_amount(Window::Options, 0.0) - 1.0).abs() < 1e-4);
        // At t=0.15 (halfway): alpha is 0.5, composite alpha is 0.5
        assert!((w.window_alpha(Window::Options, 0.15) - 0.5).abs() < 1e-4);
        assert!((w.window_composite_alpha(Window::Options, 0.15) - 0.5).abs() < 1e-4);
        // At t=0.30 (complete): alpha is 1.0, fully opaque
        assert!((w.window_alpha(Window::Options, 0.30) - 1.0).abs() < 1e-4);
        assert!((w.window_composite_alpha(Window::Options, 0.30) - 1.0).abs() < 1e-4);

        // 2. Focused-to-unfocused alpha transition: Child opens at t=1.0
        w.set_time(1.0);
        w.open(Window::SubDialog);
        assert_eq!(w.focus(), Layer::Window(Window::SubDialog));
        // SubDialog is focused and fading in
        assert!((w.window_alpha(Window::SubDialog, 1.0) - 0.0).abs() < 1e-4);
        // Parent Options loses focus and transitions towards 0.0 (unfocused alpha 0.3)
        assert!((w.focus_amount(Window::Options, 1.0) - 1.0).abs() < 1e-4);
        assert!((w.window_composite_alpha(Window::Options, 1.0) - 1.0).abs() < 1e-4);
        // At t=1.15 (halfway): Options focus is 0.5, composite is 1.0 * (0.3 + 0.7 * 0.5) = 0.65
        assert!((w.focus_amount(Window::Options, 1.15) - 0.5).abs() < 1e-4);
        assert!((w.window_composite_alpha(Window::Options, 1.15) - 0.65).abs() < 1e-4);
        // At t=1.30 (complete): Options focus is 0.0, composite is 0.30 (translucent)
        assert!((w.focus_amount(Window::Options, 1.30) - 0.0).abs() < 1e-4);
        assert!((w.window_composite_alpha(Window::Options, 1.30) - 0.30).abs() < 1e-4);

        // 3. Unfocused-to-focused transition & Closing fade-out completion:
        // Child closes at t=2.0
        w.set_time(2.0);
        w.close(Window::SubDialog);
        // Focus hands off to Parent (Options) IMMEDIATELY, SubDialog is not active
        assert_eq!(w.focus(), Layer::Window(Window::Options));
        assert!(!w.is_active(Window::SubDialog));
        assert!(w.is_closing(Window::SubDialog));

        // Options immediately starts transitioning back from 0.0 towards 1.0
        assert!((w.focus_amount(Window::Options, 2.0) - 0.0).abs() < 1e-4);
        assert!((w.window_composite_alpha(Window::Options, 2.0) - 0.30).abs() < 1e-4);
        // At t=2.15: Options focus is 0.5, composite is 0.65
        assert!((w.focus_amount(Window::Options, 2.15) - 0.5).abs() < 1e-4);
        assert!((w.window_composite_alpha(Window::Options, 2.15) - 0.65).abs() < 1e-4);
        // At t=2.30: Options focus is 1.0, fully opaque again
        assert!((w.focus_amount(Window::Options, 2.30) - 1.0).abs() < 1e-4);
        assert!((w.window_composite_alpha(Window::Options, 2.30) - 1.0).abs() < 1e-4);

        // 4. Closing fade-out progression on SubDialog:
        // At t=2.0: SubDialog alpha is 1.0
        assert!((w.window_alpha(Window::SubDialog, 2.0) - 1.0).abs() < 1e-4);
        // At t=2.15: SubDialog alpha is 0.5
        assert!((w.window_alpha(Window::SubDialog, 2.15) - 0.5).abs() < 1e-4);
        // At t=2.30: SubDialog alpha reaches 0.0
        assert!((w.window_alpha(Window::SubDialog, 2.30) - 0.0).abs() < 1e-4);

        // clean_closed at t=2.30 purges the finished closing window
        w.set_time(2.30);
        let closed = w.clean_closed();
        assert_eq!(closed, vec![Window::SubDialog]);
        assert!(!w.is_open(Window::SubDialog));
        assert!(w.is_active(Window::Options));
    }
}
