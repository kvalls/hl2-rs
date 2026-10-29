//! The UI shell around an app's own screens: the window stack with its focus
//! ([`crate::windows`]), the developer console dialog ([`ConsolePanel`]), Escape held over a
//! dialog it will close, keyboard repeat and the latest pointer. The app (its menus, dialogs and
//! base layer) keeps everything that is its own.

use crate::canvas::{Canvas, Rect};
use crate::console::dialog::Scaling;
use crate::console::panel::ConsolePanel;
use crate::input::{Input, KeyRepeat};
use crate::windows::{Layer, Pointer, WindowId, Windows};
use glam::Vec2;

/// The shell's state.
pub struct Shell<W> {
    /// The app's dialog windows and the console: non-modal windows over the base (retail VGUI
    /// frames), in z-order with the keyboard focus.
    pub windows: Windows<W>,
    /// The developer console dialog: output, history, command line, completion, its placement
    /// and drawing (kept while the game runs).
    pub panel: ConsolePanel,
    /// Escape is held and closes the top dialog on release.
    pub escape_held: bool,
    pub key_repeat: KeyRepeat,
    /// The pointer of the latest input frame (cursor hints).
    pub pointer: Option<Vec2>,
}

impl<W: WindowId> Shell<W> {
    /// No windows open; window focus fades take `fade_time` seconds and an unfocused window keeps
    /// `unfocused_alpha` of its focused look; the console dialog is sized with `scaling`.
    pub fn new(fade_time: f32, unfocused_alpha: f32, scaling: Scaling) -> Self {
        Self {
            windows: Windows::new(fade_time, unfocused_alpha),
            panel: ConsolePanel::new(scaling),
            escape_held: false,
            key_repeat: KeyRepeat::default(),
            pointer: None,
        }
    }
}

/// What the shell's control flow asks the app, in the order it asks: the app's mode, its modal
/// layers, its dialog windows and its base layer.
pub trait Host {
    type Window: WindowId;
    /// What input hands back to the app's host.
    type Effect;
    /// The app's mode: the key repeat stops when input changes it.
    type Mode: Copy + PartialEq;
    /// What preparing a frame's windows tells the app's own window input.
    type Frame;

    fn shell(&self) -> &Shell<Self::Window>;
    fn shell_mut(&mut self) -> &mut Shell<Self::Window>;
    fn mode(&self) -> Self::Mode;
    /// A key binding is being captured: held keys do not repeat into it.
    fn capturing_key(&self) -> bool;
    /// The input a modal layer of the app takes (a loading dialog, a message box), which also
    /// occludes what is under it; `None` without one.
    fn modal_input(&mut self, input: &Input) -> Option<Vec<Self::Effect>>;
    /// Prepares the windows for this frame: the modal layer that takes every press, and what the
    /// app's window input needs.
    fn prepare_windows(&mut self) -> (Option<Self::Window>, Self::Frame);
    /// A window's frame on the canvas.
    fn window_bounds(&self, window: Self::Window) -> Rect;
    /// The console's completion popup on the canvas, while it shows (it reaches past the frame).
    fn console_popup_bounds(&self) -> Option<Rect>;
    /// The base layer takes the pointer (a menu; gameplay does not).
    fn base_takes_pointer(&self) -> bool;
    /// The focused layer may have changed.
    fn focus_changed(&mut self);
    /// A window's input: its `share` of the frame, and the whole `input` for windows that take
    /// it all.
    fn window_input(
        &mut self,
        frame: &Self::Frame,
        window: Self::Window,
        share: &Input,
        input: &Input,
    ) -> Vec<Self::Effect>;
    /// A modal layer opened: the windows and the base get nothing more this frame.
    fn input_blocked(&self) -> bool;
    /// A window has the pointer: the base only follows the button.
    fn occlude_base(&mut self, down: bool);
    /// The base's input: its share of the frame, and whether it owns the pointer.
    fn base_input(&mut self, share: &Input, owns_pointer: bool) -> Vec<Self::Effect>;
    /// Synchronizes the app's own windows (its dialogs' layers) with the window stack.
    fn sync_windows(&mut self);
    /// A state of the app in which Escape closes no window (a loading dialog, the app's own box
    /// being dragged).
    fn escape_blocked(&self) -> bool;
    /// Escape closes the app's window `window` on top (on release, after showing its button).
    fn escape_closes(&self, window: Self::Window) -> bool;
    /// Escape is held over `top`: the app shows the button Escape will press.
    fn hold_escape(&mut self, top: Option<Self::Window>);
    /// Escape was let go, or a window closed: the app's held looks end.
    fn release_escape(&mut self);
    /// Undoes a drag or resize of the app's own in progress (a box over the windows).
    fn cancel_drag(&mut self) -> bool;
    /// Escape in a modal state of the app (a loading dialog, a box over the windows); `true`
    /// when that took it.
    fn escape_modal(&mut self) -> bool;
    /// Escape on the app's window `window`, on top.
    fn escape_window(&mut self, window: Self::Window);
    /// Escape on the base (no window open).
    fn escape_base(&mut self);
    /// A window closed: the app's interaction state follows.
    fn window_closed(&mut self);
    /// The console can neither open nor close now (a modal state of the app).
    fn console_blocked(&self) -> bool;
    /// The console may open (the app's setting).
    fn console_enabled(&self) -> bool;
    /// The console is about to open over the base.
    fn console_opening(&mut self);
    /// The console closed.
    fn console_closed(&mut self);
    /// The canvas the console dialog is placed on.
    fn canvas(&self) -> &Canvas;
}

/// One frame of input: held keys repeat for every layer (except into a key binding being
/// captured), then a modal layer of the app takes it, else the windows and the base share it.
pub fn input<H: Host>(host: &mut H, input: &Input) -> Vec<H::Effect> {
    let mut repeated = input.clone();
    if host.capturing_key() {
        host.shell_mut().key_repeat.cancel();
    } else {
        host.shell_mut().key_repeat.apply(&mut repeated);
    }
    let mode = host.mode();
    let effects = layer_input(host, &repeated);
    if host.mode() != mode {
        host.shell_mut().key_repeat.cancel();
    }
    effects
}

fn layer_input<H: Host>(host: &mut H, input: &Input) -> Vec<H::Effect> {
    if let Some(time) = input.time {
        host.shell_mut().windows.set_time(time);
    }
    if let Some(effects) = host.modal_input(input) {
        return effects;
    }
    windows_input(host, input)
}

/// The windows are not modal (retail VGUI frames): the pointer goes to the topmost window under
/// it, else to the base behind them; a press raises and focuses its layer and stays with it
/// until release; the keyboard goes to the focused layer. A modal layer of a window takes every
/// click. Every layer sees every frame, with only its share of it.
fn windows_input<H: Host>(host: &mut H, input: &Input) -> Vec<H::Effect> {
    let (modal, frame) = host.prepare_windows();
    let completion = host.console_popup_bounds();
    let bounds: Vec<(H::Window, Rect)> = host
        .shell()
        .windows
        .order()
        .map(|w| (w, host.window_bounds(w)))
        .collect();
    let pointer = Pointer {
        at: input.pointer,
        click: input.click,
        held: input.held,
        released: input.released,
    };
    let base_takes_pointer = host.base_takes_pointer();
    let owner = host
        .shell_mut()
        .windows
        .route(pointer, modal, base_takes_pointer, |window, at| {
            bounds.iter().any(|(w, r)| *w == window && r.contains(at))
                || window == H::Window::CONSOLE && completion.is_some_and(|r| r.contains(at))
        });
    host.focus_changed();
    let activating_click = host.shell().windows.activating_click();
    let focus = host.shell().windows.focus();
    let mut effects = Vec::new();
    let windows: Vec<H::Window> = host.shell().windows.order().collect();
    for window in windows {
        if !host.shell().windows.is_open(window) {
            continue;
        }
        let layer = Layer::Window(window);
        let owns = owner == Some(layer);
        let mut share = input.share(owns, focus == layer);
        if activating_click && owns {
            share.click = false;
            share.held = false;
            share.released = false;
        }
        effects.extend(host.window_input(&frame, window, &share, input));
        if host.input_blocked() {
            return effects;
        }
    }
    if host.base_takes_pointer() {
        let owns = owner == Some(Layer::Base);
        if !owns {
            // A window has the pointer: the base only follows the button (occluded).
            host.occlude_base(input.click || input.held);
        }
        let focus = host.shell().windows.focus();
        let share = input.share(owns, focus == Layer::Base);
        effects.extend(host.base_input(&share, owns));
    }
    effects
}

/// Escape went down. When it would close a dialog that shows a close or Cancel button, that
/// button shows pressed until Escape is released and the dialog closes then; everything else
/// Escape does happens at once.
pub fn escape_pressed<H: Host>(host: &mut H) {
    host.sync_windows();
    if escape_closes_dialog(host) {
        host.shell_mut().escape_held = true;
        let top = host.shell().windows.top();
        host.hold_escape(top);
    } else {
        escape(host);
    }
}

/// Escape went up: closes the dialog whose close or Cancel button it held.
pub fn escape_released<H: Host>(host: &mut H) {
    host.sync_windows();
    if std::mem::take(&mut host.shell_mut().escape_held) {
        host.release_escape();
        if escape_closes_dialog(host) {
            escape(host);
        }
    }
}

/// Whether Escape closes the top window (on release, after showing the button it presses).
fn escape_closes_dialog<H: Host>(host: &H) -> bool {
    if host.escape_blocked() {
        return false;
    }
    match host.shell().windows.top() {
        Some(window) if window == H::Window::CONSOLE => {
            let panel = &host.shell().panel;
            !panel.placement.active() && !panel.editor.can_dismiss()
        }
        Some(window) => host.escape_closes(window),
        None => false,
    }
}

/// Escape: undoes a drag or resize in progress first, then in the console ends a history
/// search or hides the completion list, then answers the app's modal state, then closes the top
/// window, else it is the base's.
pub fn escape<H: Host>(host: &mut H) {
    // A drag or resize in progress is undone first.
    let console_on_top = host.shell().windows.top() == Some(H::Window::CONSOLE);
    if host.cancel_drag() || (console_on_top && host.shell_mut().panel.placement.cancel()) {
        return;
    }
    // In the console Escape first ends a history search, then hides the completion list.
    if console_on_top && host.shell_mut().panel.editor.dismiss() {
        return;
    }
    if host.escape_modal() {
        return;
    }
    // Escape closes the top window first, whichever layer has the focus.
    match host.shell().windows.top() {
        Some(window) if window == H::Window::CONSOLE => {
            close_window(host, window);
            host.console_closed();
        }
        Some(window) => host.escape_window(window),
        None => host.escape_base(),
    }
}

/// Opens the console on top with the focus, or closes it (focused or not); nothing happens while
/// a modal layer is open, and it opens only while the app enables it.
pub fn toggle<H: Host>(host: &mut H) {
    if host.shell().windows.has_modal() || host.console_blocked() {
        return;
    }
    if host.shell().windows.is_active(H::Window::CONSOLE) {
        close_window(host, H::Window::CONSOLE);
        host.console_closed();
    } else if host.console_enabled() {
        host.console_opening();
        host.shell_mut().windows.open(H::Window::CONSOLE);
        host.focus_changed();
        let canvas = host.canvas().clone();
        let panel = &mut host.shell_mut().panel;
        let frame = panel.frame(&canvas);
        panel.placement.reset_if_off_screen(&frame);
    }
}

/// Closes `window`: the console's drag or resize ends, Escape's held look goes and the window
/// fades out.
pub fn close_window<H: Host>(host: &mut H, window: H::Window) {
    if window == H::Window::CONSOLE {
        host.shell_mut().panel.placement.end();
    }
    host.shell_mut().escape_held = false;
    host.release_escape();
    host.shell_mut().windows.close(window);
    host.window_closed();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Key;
    use crate::widgets::Painter;
    use glam::vec2;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Window {
        Dialog,
        Console,
        Modal,
    }

    impl WindowId for Window {
        const CONSOLE: Self = Window::Console;
        fn is_modal(&self) -> bool {
            *self == Window::Modal
        }
    }

    fn dialog_scale(canvas: Vec2) -> f32 {
        Painter::scale_for(canvas.y)
    }

    fn design_height(canvas: Vec2) -> f32 {
        canvas.y
    }

    const SCALING: Scaling = Scaling {
        dialog_scale,
        design_height,
    };

    /// An app over the shell: a dialog at x 0..100, the console at x 50..150, a modal window
    /// at x 20..80 and a menu as the base everywhere. It logs the hooks that change or decide
    /// something, in the order the shell calls them; its input effects say what each layer got.
    struct App {
        shell: Shell<Window>,
        canvas: Canvas,
        log: Vec<String>,
        mode: u32,
        capturing: bool,
        /// A modal layer of the app (a loading dialog) takes every frame.
        loading: bool,
        escape_blocked: bool,
        /// A drag of the app's own box is in progress.
        dragging: bool,
        /// The app's modal state answers Escape.
        modal_escape: bool,
        console_enabled: bool,
        console_blocked: bool,
        /// The input of this window opens a modal layer of the app.
        blocks: Option<Window>,
        blocked: bool,
        /// Enter on the base changes the app's mode.
        enter_changes_mode: bool,
    }

    impl App {
        fn new() -> Self {
            Self {
                shell: Shell::new(0., 0.3, SCALING),
                canvas: Canvas::default(),
                log: Vec::new(),
                mode: 0,
                capturing: false,
                loading: false,
                escape_blocked: false,
                dragging: false,
                modal_escape: false,
                console_enabled: true,
                console_blocked: false,
                blocks: None,
                blocked: false,
                enter_changes_mode: false,
            }
        }

        fn with(windows: &[Window]) -> Self {
            let mut app = Self::new();
            for &window in windows {
                app.shell.windows.open(window);
            }
            app
        }

        fn note(&mut self, entry: impl Into<String>) {
            self.log.push(entry.into());
        }

        fn take_log(&mut self) -> Vec<String> {
            std::mem::take(&mut self.log)
        }
    }

    impl Host for App {
        type Window = Window;
        type Effect = String;
        type Mode = u32;
        type Frame = ();

        fn shell(&self) -> &Shell<Window> {
            &self.shell
        }
        fn shell_mut(&mut self) -> &mut Shell<Window> {
            &mut self.shell
        }
        fn mode(&self) -> u32 {
            self.mode
        }
        fn capturing_key(&self) -> bool {
            self.capturing
        }
        fn modal_input(&mut self, _: &Input) -> Option<Vec<String>> {
            self.loading.then(|| vec!["loading".to_owned()])
        }
        fn prepare_windows(&mut self) -> (Option<Window>, ()) {
            let modal = self
                .shell
                .windows
                .is_active(Window::Modal)
                .then_some(Window::Modal);
            (modal, ())
        }
        fn window_bounds(&self, window: Window) -> Rect {
            match window {
                Window::Dialog => Rect::new(0., 0., 100., 100.),
                Window::Console => Rect::new(50., 0., 100., 100.),
                Window::Modal => Rect::new(20., 0., 60., 100.),
            }
        }
        fn console_popup_bounds(&self) -> Option<Rect> {
            None
        }
        fn base_takes_pointer(&self) -> bool {
            true
        }
        fn focus_changed(&mut self) {}
        fn window_input(
            &mut self,
            _: &(),
            window: Window,
            share: &Input,
            _: &Input,
        ) -> Vec<String> {
            if self.blocks == Some(window) && share.click {
                self.blocked = true;
            }
            vec![format!(
                "{window:?} click {} keys {:?}",
                share.click, share.keys
            )]
        }
        fn input_blocked(&self) -> bool {
            self.blocked
        }
        fn occlude_base(&mut self, down: bool) {
            self.note(format!("occlude base {down}"));
        }
        fn base_input(&mut self, share: &Input, owns_pointer: bool) -> Vec<String> {
            if self.enter_changes_mode && share.pressed(Key::Enter) {
                self.mode += 1;
            }
            vec![format!(
                "base click {} owns {owns_pointer} keys {:?}",
                share.click, share.keys
            )]
        }
        fn sync_windows(&mut self) {
            self.note("sync");
        }
        fn escape_blocked(&self) -> bool {
            self.escape_blocked
        }
        fn escape_closes(&self, window: Window) -> bool {
            window == Window::Dialog
        }
        fn hold_escape(&mut self, top: Option<Window>) {
            self.note(format!("hold {top:?}"));
        }
        fn release_escape(&mut self) {
            self.note("release");
        }
        fn cancel_drag(&mut self) -> bool {
            self.note("cancel drag");
            std::mem::take(&mut self.dragging)
        }
        fn escape_modal(&mut self) -> bool {
            self.note("escape modal");
            self.modal_escape
        }
        fn escape_window(&mut self, window: Window) {
            self.note(format!("escape {window:?}"));
            close_window(self, window);
        }
        fn escape_base(&mut self) {
            self.note("escape base");
        }
        fn window_closed(&mut self) {
            self.note("closed");
        }
        fn console_blocked(&self) -> bool {
            self.console_blocked
        }
        fn console_enabled(&self) -> bool {
            self.console_enabled
        }
        fn console_opening(&mut self) {
            self.note("console opening");
        }
        fn console_closed(&mut self) {
            self.note("console closed");
        }
        fn canvas(&self) -> &Canvas {
            &self.canvas
        }
    }

    fn click(x: f32) -> Input {
        Input {
            pointer: Some(vec2(x, 50.)),
            click: true,
            held: true,
            ..Default::default()
        }
    }

    #[test]
    fn escape_undoes_a_drag_then_ends_the_console_search_then_the_modal_state_then_closes_the_top()
    {
        let mut app = App::with(&[Window::Dialog, Window::Console]);
        app.dragging = true;
        escape(&mut app);
        assert_eq!(app.take_log(), ["cancel drag"]);
        app.shell.panel.editor.search_back();
        escape(&mut app);
        assert!(
            app.shell.panel.editor.search().is_none(),
            "the history search ends"
        );
        assert_eq!(app.take_log(), ["cancel drag"]);
        app.modal_escape = true;
        escape(&mut app);
        assert_eq!(app.take_log(), ["cancel drag", "escape modal"]);
        assert!(app.shell.windows.is_active(Window::Console));
        app.modal_escape = false;
        escape(&mut app);
        assert_eq!(
            app.take_log(),
            [
                "cancel drag",
                "escape modal",
                "release",
                "closed",
                "console closed"
            ]
        );
        assert!(!app.shell.windows.is_open(Window::Console));
        escape(&mut app);
        assert_eq!(
            app.take_log(),
            [
                "cancel drag",
                "escape modal",
                "escape Dialog",
                "release",
                "closed"
            ]
        );
        escape(&mut app);
        assert_eq!(
            app.take_log(),
            ["cancel drag", "escape modal", "escape base"]
        );
    }

    #[test]
    fn a_held_escape_shows_the_close_button_and_closes_on_release() {
        let mut app = App::with(&[Window::Dialog]);
        escape_pressed(&mut app);
        assert_eq!(app.take_log(), ["sync", "hold Some(Dialog)"]);
        assert!(app.shell.escape_held && app.shell.windows.is_active(Window::Dialog));
        escape_released(&mut app);
        assert_eq!(
            app.take_log(),
            [
                "sync",
                "release",
                "cancel drag",
                "escape modal",
                "escape Dialog",
                "release",
                "closed"
            ]
        );
        assert!(!app.shell.escape_held && app.shell.windows.is_empty());
        // The console closes the same way unless Escape first ends something in it.
        let mut app = App::with(&[Window::Console]);
        escape_pressed(&mut app);
        assert!(app.shell.escape_held);
        app.shell.escape_held = false;
        app.shell.panel.editor.search_back();
        escape_pressed(&mut app);
        assert!(!app.shell.escape_held, "a history search ends at once");
        assert!(app.shell.windows.is_active(Window::Console));
        // While the app blocks it, Escape acts at once and holds nothing.
        let mut app = App::with(&[Window::Dialog]);
        app.escape_blocked = true;
        escape_pressed(&mut app);
        assert!(!app.shell.escape_held);
        assert_eq!(
            app.take_log(),
            [
                "sync",
                "cancel drag",
                "escape modal",
                "escape Dialog",
                "release",
                "closed"
            ]
        );
        // A release with nothing held does nothing.
        escape_released(&mut app);
        assert_eq!(app.take_log(), ["sync"]);
    }

    #[test]
    fn the_console_toggle_opens_it_on_top_with_the_focus_and_closes_it() {
        let mut app = App::with(&[Window::Dialog]);
        app.console_enabled = false;
        toggle(&mut app);
        assert!(
            !app.shell.windows.is_open(Window::Console),
            "disabled by the app"
        );
        app.console_enabled = true;
        toggle(&mut app);
        assert_eq!(app.take_log(), ["console opening"]);
        assert_eq!(app.shell.windows.top(), Some(Window::Console));
        assert_eq!(app.shell.windows.focus(), Layer::Window(Window::Console));
        app.shell.windows.open(Window::Dialog);
        toggle(&mut app);
        assert_eq!(
            app.take_log(),
            ["release", "closed", "console closed"],
            "closes unfocused too"
        );
        assert!(!app.shell.windows.is_open(Window::Console));
        app.console_blocked = true;
        toggle(&mut app);
        assert!(
            !app.shell.windows.is_open(Window::Console),
            "blocked by the app"
        );
        app.console_blocked = false;
        app.shell.windows.open(Window::Modal);
        toggle(&mut app);
        assert!(
            !app.shell.windows.is_open(Window::Console),
            "not over a modal window"
        );
        assert!(app.take_log().is_empty());
    }

    #[test]
    fn close_window_ends_the_held_escape_and_tells_the_app() {
        let mut app = App::with(&[Window::Dialog, Window::Console]);
        app.shell.escape_held = true;
        close_window(&mut app, Window::Console);
        assert!(!app.shell.escape_held);
        assert_eq!(app.take_log(), ["release", "closed"]);
        assert_eq!(app.shell.windows.top(), Some(Window::Dialog));
        assert_eq!(
            app.shell.windows.focus(),
            Layer::Window(Window::Dialog),
            "focus goes back"
        );
    }

    #[test]
    fn the_pointer_goes_to_the_topmost_window_under_it_else_to_the_base() {
        let mut app = App::with(&[Window::Dialog, Window::Console]);
        let effects = input(&mut app, &click(75.));
        assert_eq!(
            effects,
            [
                "Dialog click false keys {}",
                "Console click true keys {}",
                "base click false owns false keys {}"
            ]
        );
        assert_eq!(
            app.take_log(),
            ["occlude base true"],
            "the base only follows the button"
        );
        // The press stays with the console until the button is released.
        let mut drag = click(20.);
        drag.click = false;
        assert_eq!(input(&mut app, &drag)[1], "Console click false keys {}");
        let release = Input {
            pointer: Some(vec2(20., 50.)),
            released: true,
            ..Default::default()
        };
        input(&mut app, &release);
        app.take_log();
        // An activating press on the unfocused dialog raises it and focuses it without a click.
        let mut press = click(20.);
        press.keys.insert(Key::Enter);
        let effects = input(&mut app, &press);
        assert_eq!(
            effects,
            [
                "Console click false keys {}",
                "Dialog click false keys {Enter}",
                "base click false owns false keys {}"
            ]
        );
        assert_eq!(app.shell.windows.top(), Some(Window::Dialog));
        input(&mut app, &release);
        app.take_log();
        // The next click on the now-focused dialog hits the control.
        let effects = input(&mut app, &press);
        assert_eq!(
            effects,
            [
                "Console click false keys {}",
                "Dialog click true keys {Enter}",
                "base click false owns false keys {}"
            ]
        );
        input(&mut app, &release);
        app.take_log();
        let effects = input(&mut app, &click(400.));
        assert_eq!(effects.last().unwrap(), "base click true owns true keys {}");
        assert!(
            app.take_log().is_empty(),
            "the base owns the pointer: not occluded"
        );
    }

    #[test]
    fn modal_layers_take_the_frame_and_stop_the_layers_under_them() {
        // The app's own modal layer (a loading dialog) takes the whole frame.
        let mut app = App::with(&[Window::Dialog]);
        app.loading = true;
        assert_eq!(input(&mut app, &click(10.)), ["loading"]);
        // A modal window takes every press, even outside its frame.
        let mut app = App::with(&[Window::Dialog, Window::Modal]);
        let effects = input(&mut app, &click(90.));
        assert_eq!(
            effects[..2],
            ["Dialog click false keys {}", "Modal click true keys {}"]
        );
        // A window whose input opens a modal layer ends the frame there: the first press raises
        // the dialog over the console, and the next click hits it to open the modal layer.
        let mut app = App::with(&[Window::Dialog, Window::Console]);
        app.blocks = Some(Window::Dialog);
        assert_eq!(
            input(&mut app, &click(10.)),
            [
                "Console click false keys {}",
                "Dialog click false keys {}",
                "base click false owns false keys {}"
            ]
        );
        let release = Input {
            pointer: Some(vec2(10., 50.)),
            released: true,
            ..Default::default()
        };
        input(&mut app, &release);
        app.take_log();
        assert_eq!(
            input(&mut app, &click(10.)),
            ["Console click false keys {}", "Dialog click true keys {}"]
        );
        assert!(app.take_log().is_empty(), "nor is it occluded");
    }

    #[test]
    fn held_keys_repeat_for_every_layer_except_into_a_key_capture_or_across_a_mode_change() {
        let frame = |t: f64, pressed: &[Key], held: &[Key]| Input {
            keys: pressed.iter().copied().collect(),
            held_keys: held.iter().chain(pressed).copied().collect(),
            time: Some(t),
            ..Default::default()
        };
        let base_keys = |effects: Vec<String>| effects.last().unwrap().contains("keys {Down}");
        let mut app = App::new();
        assert!(base_keys(input(&mut app, &frame(0., &[Key::Down], &[]))));
        assert!(!base_keys(input(&mut app, &frame(0.2, &[], &[Key::Down]))));
        assert!(
            base_keys(input(&mut app, &frame(0.45, &[], &[Key::Down]))),
            "repeats"
        );
        app.capturing = true;
        assert!(
            !base_keys(input(&mut app, &frame(0.6, &[], &[Key::Down]))),
            "not into a capture"
        );
        app.capturing = false;
        assert!(
            !base_keys(input(&mut app, &frame(1.5, &[], &[Key::Down]))),
            "stays stopped"
        );
        // A frame that changes the app's mode stops the repeat it started.
        app.enter_changes_mode = true;
        input(&mut app, &frame(2., &[Key::Down, Key::Enter], &[]));
        assert_eq!(app.mode, 1);
        assert!(!base_keys(input(&mut app, &frame(3., &[], &[Key::Down]))));
    }
}
