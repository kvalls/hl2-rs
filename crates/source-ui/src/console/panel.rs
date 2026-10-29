//! The developer console dialog (SDK `CConsoleDialog` / `CConsolePanel`): the output buffer, the
//! history it shows (filtered, wrapped, scrolled, selectable), the command line with its
//! completion popup, the Close and Submit buttons, the dialog's placement, its input and its
//! drawing. The app runs the commands: it hands the panel its canvas, scaling, texts, look and
//! command table, and turns the panel's [`Action`]s into what they do.

use super::command::{self, Context, Spec};
use super::complete::{self, Completer};
use super::dialog::{wrap, DialogLayout, Scaling, Selection, MIN_SIZE};
use super::editor::LineEditor;
use crate::canvas::{Canvas, Color, Rect, WHITE};
use crate::font::FontFace;
use crate::input::{Input, Key};
use crate::placement::{self, Placement};
use crate::widgets::{
    close_button_rect, ButtonState, ButtonTracker, Fonts, FrameSpec, Painter, ScrollAccumulator,
    ScrollBarLayout, ScrollBarState, ScrollMode, TextInputModel, Theme, SCROLLBAR_WIDTH,
};
use glam::{vec2, Vec2};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::sync::Mutex;

/// Output lines kept for scrollback.
pub const OUTPUT_LINES: usize = 1024;
/// Warning and error lines (not in the scheme; the toolkit's own colors).
const WARNING_TEXT: Color = Color::new(1.0, 0.8, 0.3, 1.0);
const ERROR_TEXT: Color = Color::new(1.0, 0.4, 0.35, 1.0);
/// SDK `CConsolePanel` shows at most this many completion rows, the last one `...` when more.
const COMPLETION_ROWS: usize = 10;
/// History rows scrolled per mouse wheel notch.
pub const WHEEL_ROWS: usize = 3;
/// Console history and completion row height, text size and baseline distance from the row
/// bottom, in layout units.
pub const CONSOLE_ROW: f32 = 14.;
const CONSOLE_TEXT: f32 = 12.;
const CONSOLE_DESCENT: f32 = 3.;
/// Space between the history's border and its rows.
const HISTORY_PAD: f32 = 3.;
/// A tab is this many spaces wide in the history.
const TAB_SPACES: usize = 8;
/// Advance per EM of every character when the console font is missing.
pub const NOMINAL_ADVANCE: f32 = 0.6;

/// How a console line was printed, which picks its color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// The echo of a submitted line (`] command`).
    Input,
    /// Ordinary output (`Msg`): scheme `Console.TextColor`.
    Normal,
    /// Developer output (`DevMsg`, mirrored engine log): scheme `Console.DevTextColor`.
    Developer,
    /// A snapshot the app replaces in place ([`ConsolePanel::replace_snapshot`]) instead of
    /// growing the scrollback.
    Snapshot,
    Warning,
    Error,
}

/// A console output line. It reads and serializes as its text.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub level: Level,
    pub text: String,
    /// Seconds since start when printed (`con_timestamp`).
    pub time: f64,
}

impl std::ops::Deref for Line {
    type Target = str;
    fn deref(&self) -> &str {
        &self.text
    }
}

impl PartialEq<str> for Line {
    fn eq(&self, other: &str) -> bool {
        self.text == other
    }
}

impl Serialize for Line {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.text)
    }
}

/// The console's colors from the scheme.
#[derive(Clone, Copy, Debug)]
pub struct Style {
    /// The history's background (`RichText.BgColor`).
    pub entry: Color,
    /// `Console.TextColor`.
    pub text: Color,
    /// `Console.DevTextColor`.
    pub dev_text: Color,
    /// Selected history rows (`RichText.SelectedBgColor` / `RichText.SelectedTextColor`).
    pub selected_bg: Color,
    pub selected_text: Color,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            entry: Color::from_rgba(0, 0, 0, 128),
            text: Color::from_rgba(221, 221, 221, 255),
            dev_text: WHITE,
            selected_bg: Color::from_rgba(255, 155, 0, 255),
            selected_text: Color::from_rgba(0, 0, 0, 255),
        }
    }
}

/// What the console's input asks the app to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// The Close button was clicked: the app closes the console (and plays the button's sound).
    Close,
    /// A line was submitted, with Enter or by clicking Submit (the button plays its sound).
    Submit {
        line: String,
        clicked: bool,
    },
}

/// What the app supplies to draw the console.
pub struct Look<'a, C: Context> {
    /// The fonts of the dialog's controls.
    pub fonts: &'a Fonts,
    /// The window's theme, faded by its focus.
    pub theme: &'a Theme,
    /// The completion popup's theme.
    pub popup_theme: &'a Theme,
    /// The dialog's title, the Submit button's label and the reverse history search prompt.
    pub title: &'a str,
    pub submit: &'a str,
    pub reverse_search: &'a str,
    /// The console window has the keyboard focus.
    pub focused: bool,
    /// Escape is held and closes the console on release: its Close button looks pressed.
    pub closing: bool,
    /// The commands the help line and the cvar values in the popup come from.
    pub commands: &'static [Spec<C>],
    pub context: &'a C,
}

/// The console dialog's push buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsoleButton {
    Close,
    Submit,
}

/// The console's completion popup: its rows (colored runs), the highlighted row, how many help
/// rows lead, and its rectangle in canvas pixels.
struct Completion {
    rows: Vec<Vec<(String, Color)>>,
    highlighted: Option<usize>,
    help_rows: usize,
    rect: Rect,
}

/// What history rows wrap to: the text width and font size in canvas pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WrapMetrics {
    pub width: f32,
    pub em: f32,
    /// Whether the console font measures characters (else they take a nominal advance).
    pub font: bool,
}

/// A line's wrapped rows and what they were wrapped from.
struct Wrapped {
    metrics: WrapMetrics,
    /// The time shown before the text (`con_timestamp`).
    stamp: Option<f64>,
    text: String,
    rows: Vec<Range<usize>>,
}

/// A history row: a byte range of a shown line's [`ConsolePanel::shown_text`].
pub struct HistoryRow<'a> {
    pub number: u64,
    pub line: &'a Line,
    pub text: Range<usize>,
}

/// The history's text rows and scroll bar inside its border, in frame layout units.
pub struct HistoryView {
    pub text: Rect,
    pub bar: Rect,
    /// Whole rows that fit.
    pub rows: usize,
}

impl HistoryView {
    pub fn new(layout: &DialogLayout) -> Self {
        let h = layout.history;
        let bar = Rect::new(
            h.x + h.w - 1. - SCROLLBAR_WIDTH,
            h.y + 1.,
            SCROLLBAR_WIDTH,
            (h.h - 2.).max(0.),
        );
        let text = Rect::new(
            h.x + HISTORY_PAD,
            h.y + HISTORY_PAD,
            (bar.x - h.x - 2. * HISTORY_PAD).max(0.),
            (h.h - 2. * HISTORY_PAD).max(0.),
        );
        let rows = ((text.h / CONSOLE_ROW) as usize).max(1);
        Self { text, bar, rows }
    }

    /// The row at layout height `y`; `clamp` puts points above or below on the first or last.
    fn row_at(&self, y: f32, clamp: bool) -> Option<usize> {
        let row = ((y - self.text.y) / CONSOLE_ROW).floor();
        if clamp {
            Some(row.clamp(0., (self.rows - 1) as f32) as usize)
        } else {
            (row >= 0. && (row as usize) < self.rows).then_some(row as usize)
        }
    }
}

/// Draws `value` from its baseline at `origin` in `font`, when there is one.
fn text(value: &str, origin: Vec2, em: f32, scale: f32, font: Option<&FontFace>, color: Color) {
    if let Some(font) = font {
        font.draw_baseline(value, origin, em, scale, color);
    }
}

fn rectangle_lines(canvas: &Canvas, x: f32, y: f32, w: f32, h: f32, thickness: f32, color: Color) {
    canvas.rectangle(x, y, w, thickness, color);
    canvas.rectangle(x, y + h - thickness, w, thickness, color);
    canvas.rectangle(x, y, thickness, h, color);
    canvas.rectangle(x + w - thickness, y, thickness, h, color);
}

/// The developer console dialog's state.
pub struct ConsolePanel {
    scaling: Scaling,
    pub output: VecDeque<Line>,
    /// Line number (since start) of `output[0]`: lines dropped from the front count on.
    pub output_start: u64,
    /// Output rows selected by dragging over the history, copied by the copy shortcut.
    pub selection: Option<Selection>,
    /// A drag that started in the history is extending the selection.
    selecting: bool,
    /// Where the dialog was dragged or resized.
    pub placement: Placement,
    /// Text the copy shortcut put on the clipboard, until the app takes it.
    copied: Option<String>,
    /// The dialog's close (×) and Submit buttons.
    buttons: ButtonTracker<ConsoleButton>,
    hover: Option<ConsoleButton>,
    history_bar: ScrollBarState,
    history_wheel: ScrollAccumulator,
    /// The input line: editing, completion, history and reverse search.
    pub editor: LineEditor,
    /// Seconds since start of the latest input frame, stamped on printed lines.
    clock: f64,
    /// `con_filter_enable`: 0 off, 1 only lines containing `con_filter_text`, 2 those brighter.
    pub con_filter: u8,
    pub con_filter_text: String,
    /// `con_timestamp`: prefix lines with the time they were printed.
    pub con_timestamp: bool,
    /// History rows scrolled back from the newest.
    pub scroll: usize,
    /// Each output line's wrapped rows by line number, rebuilt when its shown text or the
    /// history width changes.
    wrapped: Mutex<HashMap<u64, Wrapped>>,
    /// ASCII advances of the console font at one size (rewrapping on resize measures every
    /// character of the history).
    ascii_advances: Mutex<Option<(u32, [f32; 128])>>,
    /// The console font (monospace).
    pub font: Option<FontFace>,
    pub style: Style,
}

impl ConsolePanel {
    /// An empty console whose dialog the app sizes with `scaling`.
    pub fn new(scaling: Scaling) -> Self {
        Self {
            scaling,
            output: VecDeque::new(),
            output_start: 0,
            selection: None,
            selecting: false,
            placement: Placement::default(),
            copied: None,
            buttons: ButtonTracker::capturing(),
            hover: None,
            history_bar: ScrollBarState::default(),
            history_wheel: ScrollAccumulator::default(),
            editor: LineEditor::default(),
            clock: 0.,
            con_filter: 0,
            con_filter_text: String::new(),
            con_timestamp: false,
            scroll: 0,
            wrapped: Mutex::default(),
            ascii_advances: Mutex::default(),
            font: None,
            style: Style::default(),
        }
    }

    /// Replaces the latest [`Level::Snapshot`] line instead of growing the scrollback (prints
    /// one when there is none).
    pub fn replace_snapshot(&mut self, canvas: &Canvas, text: impl Into<String>) {
        let text = text.into();
        if let Some((index, _)) = self
            .output
            .iter()
            .enumerate()
            .rev()
            .find(|(_, line)| line.level == Level::Snapshot)
        {
            let number = self.output_start + index as u64;
            if let Some(line) = self.output.get_mut(index) {
                line.text = text;
                line.time = self.clock;
            }
            if let Ok(mut wrapped) = self.wrapped.lock() {
                wrapped.remove(&number);
            }
        } else {
            self.print(canvas, Level::Snapshot, text);
        }
    }

    /// Prints a line at `level`, stamped with the current time.
    pub fn print(&mut self, canvas: &Canvas, level: Level, text: impl Into<String>) {
        self.output.push_back(Line {
            level,
            text: text.into(),
            time: self.clock,
        });
        if self.output.len() > OUTPUT_LINES {
            self.output.pop_front();
            self.output_start += 1;
        }
        // Scrolled back, the view stays on its rows while new ones arrive below.
        let number = self.output_start + self.output.len() as u64 - 1;
        if let Some(line) = self
            .output
            .back()
            .filter(|line| self.scroll > 0 && (self.con_filter != 1 || self.filter_match(line)))
        {
            self.scroll += self
                .line_rows(number, line, self.wrap_metrics(canvas))
                .len();
        }
    }

    /// Keyboard and pointer input of the focused console: editing keys, history scrolling,
    /// completion, copying, the dialog's placement and buttons. `completer` completes the line.
    pub fn input(&mut self, canvas: &Canvas, input: &Input, completer: &impl Completer) -> Action {
        if let Some(time) = input.time {
            self.clock = time;
        }
        // The caption and corner grips own the pointer before the history's row selection.
        let frame = self.frame(canvas);
        let placing =
            self.placement
                .input(&frame, input.pointer, input.click, input.held, input.time);
        if !placing {
            if let Some(action) = self.pointer_input(canvas, input) {
                return action;
            }
        }
        let ctrl_key = |name: &str| input.ctrl && input.binding_keys.iter().any(|k| k == name);
        let editor = &mut self.editor;
        if !input.ctrl {
            let typed: String = input
                .text
                .chars()
                .filter(|c| *c != '`' && *c != '~')
                .collect();
            editor.insert(&typed);
        }
        if let Some(paste) = &input.paste {
            editor.paste(paste);
        }
        if ctrl_key("KeyR") {
            editor.search_back();
        }
        if ctrl_key("KeyU") {
            editor.kill_to_start();
        }
        if ctrl_key("KeyK") {
            editor.kill_to_end();
        }
        if ctrl_key("KeyW") {
            editor.delete_word_back();
        }
        if ctrl_key("KeyA") {
            editor.home();
        }
        if ctrl_key("KeyE") {
            editor.end();
        }
        if input.pressed(Key::Backspace) {
            if input.ctrl {
                editor.delete_word_back();
            } else {
                editor.backspace();
            }
        }
        if input.pressed(Key::Delete) {
            if input.ctrl {
                editor.delete_word_forward();
            } else {
                editor.delete();
            }
        }
        if input.pressed(Key::Left) {
            if input.ctrl {
                editor.word_left();
            } else {
                editor.left();
            }
        }
        if input.pressed(Key::Right) {
            if input.ctrl {
                editor.word_right();
            } else {
                editor.right();
            }
        }
        if input.pressed(Key::Home) {
            if input.ctrl {
                self.scroll = self.history_rows(canvas).len();
            } else {
                self.editor.home();
            }
        }
        if input.pressed(Key::End) {
            if input.ctrl {
                self.scroll = 0;
            } else {
                self.editor.end();
            }
        }
        if input.pressed(Key::PageUp) {
            self.scroll = (self.scroll + 10).min(self.history_rows(canvas).len().saturating_sub(1));
        }
        if input.pressed(Key::PageDown) {
            self.scroll = self.scroll.saturating_sub(10);
        }
        // Completion follows the edited line before Tab/Up/Down step through it.
        self.editor.refresh(completer);
        if input.pressed(Key::Up) {
            self.editor.step(true, false);
        }
        if input.pressed(Key::Down) {
            self.editor.step(false, false);
        }
        if input.pressed(Key::Tab) {
            self.editor.step(input.shift, true);
            self.editor.refresh(completer);
        }
        if ctrl_key("KeyL") {
            self.clear_output();
        }
        if input.copy {
            self.copied = self.selected_text();
        }
        if input.pressed(Key::Enter) {
            let line = self.editor.submit();
            self.scroll = 0;
            return Action::Submit {
                line,
                clicked: false,
            };
        }
        Action::None
    }

    /// Pointer input of the console dialog: the close and Submit buttons, the history scroll
    /// bar, the wheel, and dragging over history rows to select them. `Some` when a button
    /// ended this frame's input (the console closes, or the line is submitted).
    fn pointer_input(&mut self, canvas: &Canvas, input: &Input) -> Option<Action> {
        let layout = self.dialog_layout(canvas);
        let view = HistoryView::new(&layout);
        let point = input.pointer.map(|p| layout.local(p));
        let down = input.click || input.held;
        let rows: Vec<u64> = self
            .history_rows(canvas)
            .iter()
            .map(|row| row.number)
            .collect();
        let total = rows.len();
        let max_scroll = total.saturating_sub(view.rows);
        let mode = ScrollMode::platform();
        let steps = self.history_wheel.steps(input.wheel, mode) as i64
            * mode.rows_per_step(WHEEL_ROWS) as i64;
        self.scroll = (self.scroll as i64 + steps).clamp(0, max_scroll as i64) as usize;
        self.scroll = self.scroll.min(max_scroll);
        let first = total - view.rows.min(total) - self.scroll;
        let bar = ScrollBarLayout::new(view.bar, first, view.rows, total);
        if !self.selecting && self.history_bar.wants(&bar, point, input.click) {
            if let Some(first) = self.history_bar.pointer(
                &bar,
                point,
                down,
                input.click,
                input.time,
                first,
                view.rows,
            ) {
                self.scroll = max_scroll.saturating_sub(first);
            }
            self.buttons.occlude(down);
            self.hover = None;
            return None;
        }
        let hovered = point.and_then(|p| {
            [
                (ConsoleButton::Close, layout.close),
                (ConsoleButton::Submit, layout.submit),
            ]
            .into_iter()
            .find(|(_, rect)| rect.contains(p))
            .map(|(button, _)| button)
        });
        if self.selecting {
            self.buttons.occlude(down);
            self.hover = None;
        } else {
            self.hover = hovered;
            match self
                .buttons
                .update(hovered, down, input.click, input.released)
            {
                Some(ConsoleButton::Close) => {
                    self.hover = None;
                    return Some(Action::Close);
                }
                Some(ConsoleButton::Submit) => {
                    let line = self.editor.submit();
                    self.scroll = 0;
                    return Some(Action::Submit {
                        line,
                        clicked: true,
                    });
                }
                None => {}
            }
        }
        if let Some(p) = point {
            if input.click && hovered.is_none() && view.text.contains(p) {
                self.selecting = true;
                self.selection = view
                    .row_at(p.y, false)
                    .and_then(|row| rows.get(first + row))
                    .map(|number| Selection::at(*number));
            } else if self.selecting && down {
                // Dragging past the top or bottom scrolls the history a row per frame.
                if p.y < view.text.y {
                    self.scroll = (self.scroll + 1).min(max_scroll);
                } else if p.y > view.text.y + view.text.h {
                    self.scroll = self.scroll.saturating_sub(1);
                }
                let first = total - view.rows.min(total) - self.scroll;
                let row = view.row_at(p.y, true).unwrap_or(0);
                let line = rows.get(first + row).or(rows.last()).copied();
                if let (Some(selection), Some(line)) = (self.selection.as_mut(), line) {
                    selection.head = line;
                }
            }
        }
        if !down {
            self.selecting = false;
        }
        None
    }

    /// Empties the history (`clear`, Ctrl+L). Line numbers keep counting, so nothing printed
    /// later can take a selected line's number.
    pub fn clear_output(&mut self) {
        self.output_start += self.output.len() as u64;
        self.output.clear();
        self.scroll = 0;
        self.selection = None;
        self.selecting = false;
    }

    /// The console dialog's frame: retail placement and minimum size, sizeable by its corners.
    pub fn frame(&self, canvas: &Canvas) -> placement::Frame {
        let canvas = vec2(canvas.width(), canvas.height());
        let (origin, size) = DialogLayout::default_frame(canvas, self.scaling);
        placement::Frame {
            origin,
            size,
            scale: (self.scaling.dialog_scale)(canvas),
            canvas,
            min_size: Some(MIN_SIZE),
            close: close_button_rect,
        }
    }

    /// Where the dialog and its controls are, after dragging and resizing.
    pub fn dialog_layout(&self, canvas: &Canvas) -> DialogLayout {
        let frame = self.frame(canvas);
        let placed = self.placement.place(&frame);
        DialogLayout::at(frame.scale, placed.origin, placed.size)
    }

    /// The output the history shows (`con_filter_enable` 1 hides lines without
    /// `con_filter_text`), with each line's number.
    pub fn shown_lines(&self) -> Vec<(u64, &Line)> {
        (self.output_start..)
            .zip(self.output.iter())
            .filter(|(_, line)| self.con_filter != 1 || self.filter_match(line))
            .collect()
    }

    /// A line as the history shows it, with its time under `con_timestamp`.
    pub fn shown_text(&self, line: &Line) -> String {
        if self.con_timestamp {
            format!("[{:7.2}] {}", line.time, line.text)
        } else {
            line.text.clone()
        }
    }

    /// The history's text width and font size in canvas pixels, which rows wrap to.
    pub fn wrap_metrics(&self, canvas: &Canvas) -> WrapMetrics {
        let layout = self.dialog_layout(canvas);
        WrapMetrics {
            width: layout.canvas(HistoryView::new(&layout).text).w,
            em: (CONSOLE_TEXT * layout.scale) as u16 as f32,
            font: self.font.is_some(),
        }
    }

    /// Advance of `c` in the console font. A tab is eight spaces wide (SDK `RichText`); without
    /// the font every character takes a nominal monospace advance.
    pub fn advance(&self, c: char, em: f32) -> f32 {
        if c == '\t' {
            return TAB_SPACES as f32 * self.advance(' ', em);
        }
        let Some(font) = self.font.as_ref() else {
            return em * NOMINAL_ADVANCE;
        };
        match self.ascii_table(em) {
            Some(table) if c.is_ascii() => table[c as usize],
            _ => font.advance_em(c, em),
        }
    }

    /// The console font's ASCII advances at `em` (tab already eight spaces), built once per size.
    fn ascii_table(&self, em: f32) -> Option<[f32; 128]> {
        let font = self.font.as_ref()?;
        let mut table = self
            .ascii_advances
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if table.is_none_or(|(size, _)| size != em.to_bits()) {
            let mut advances = [0.; 128];
            for (code, advance) in advances.iter_mut().enumerate() {
                *advance = font.advance_em(char::from(code as u8), em);
            }
            advances[usize::from(b'\t')] = TAB_SPACES as f32 * advances[usize::from(b' ')];
            *table = Some((em.to_bits(), advances));
        }
        table.map(|(_, advances)| advances)
    }

    /// The rows line `number` wraps to, as byte ranges into its [`Self::shown_text`].
    fn line_rows(&self, number: u64, line: &Line, metrics: WrapMetrics) -> Vec<Range<usize>> {
        let stamp = self.con_timestamp.then_some(line.time);
        let mut cache = self.wrapped.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = cache.get(&number).filter(|entry| {
            entry.metrics == metrics && entry.stamp == stamp && entry.text == line.text
        }) {
            return entry.rows.clone();
        }
        let table = self.ascii_table(metrics.em);
        let rows = wrap(&self.shown_text(line), metrics.width, |c| match table {
            Some(table) if c.is_ascii() => table[c as usize],
            _ => self.advance(c, metrics.em),
        });
        if cache.len() >= 2 * OUTPUT_LINES {
            cache.retain(|n, _| *n >= self.output_start);
        }
        cache.insert(
            number,
            Wrapped {
                metrics,
                stamp,
                text: line.text.clone(),
                rows: rows.clone(),
            },
        );
        rows
    }

    /// The history's rows: the shown lines wrapped to its width at word boundaries, a row for
    /// each line of text that holds newlines.
    pub fn history_rows(&self, canvas: &Canvas) -> Vec<HistoryRow<'_>> {
        let metrics = self.wrap_metrics(canvas);
        self.shown_lines()
            .into_iter()
            .flat_map(|(number, line)| {
                self.line_rows(number, line, metrics)
                    .into_iter()
                    .map(move |text| HistoryRow { number, line, text })
            })
            .collect()
    }

    /// The selected history lines as text, one line each and whole however they wrap.
    fn selected_text(&self) -> Option<String> {
        let selection = self.selection?;
        let lines: Vec<String> = self
            .shown_lines()
            .into_iter()
            .filter(|(number, _)| selection.contains(*number))
            .map(|(_, line)| self.shown_text(line))
            .collect();
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    /// Text the copy shortcut copied from the history, for the app to put on the clipboard.
    pub fn take_copied(&mut self) -> Option<String> {
        self.copied.take()
    }

    fn button_state(&self, button: ConsoleButton, closing: bool, focused: bool) -> ButtonState {
        let hovered = self.hover == Some(button);
        ButtonState {
            armed: focused && self.buttons.armable(self.hover) == Some(button),
            pressed: (hovered && self.buttons.is_pressed(button))
                || (button == ConsoleButton::Close && closing),
            enabled: true,
            // SDK `CConsolePanel` makes Submit the default button.
            default: button == ConsoleButton::Submit,
        }
    }

    /// Whether `line` passes `con_filter_enable` 1 (and is drawn brighter under 2).
    pub fn filter_match(&self, line: &Line) -> bool {
        self.con_filter_text.is_empty()
            || line
                .text
                .to_lowercase()
                .contains(&self.con_filter_text.to_lowercase())
    }

    fn line_color(&self, line: &Line) -> Color {
        if self.con_filter == 2 && !self.con_filter_text.is_empty() && self.filter_match(line) {
            return self.style.dev_text;
        }
        match line.level {
            Level::Input | Level::Normal => self.style.text,
            Level::Developer => self.style.dev_text,
            Level::Snapshot => self.style.text,
            Level::Warning => WARNING_TEXT,
            Level::Error => ERROR_TEXT,
        }
    }

    /// The console dialog (SDK `CConsoleDialog`): a titled frame with the history, the entry
    /// and the Submit button, then the completion popup over everything; the history, entry and
    /// buttons are clipped by `occluders` (windows above it).
    pub fn draw<C: Context>(
        &self,
        canvas: &Canvas,
        look: &Look<'_, C>,
        time: f64,
        occluders: &[Rect],
    ) {
        let layout = self.dialog_layout(canvas);
        let scale = layout.scale;
        let painter = Painter {
            canvas,
            fonts: look.fonts,
            theme: look.theme,
            origin: layout.origin,
            scale,
        };
        let em = (CONSOLE_TEXT * scale) as u16 as f32;
        let text = |content: &str, x: f32, y: f32, color: Color| {
            text(content, vec2(x, y), em, 1., self.font.as_ref(), color);
        };
        let spec = FrameSpec::new(layout.size, look.title);

        // Frame background (unclipped):
        painter.frame_background(&spec);

        // Push occluders for console content:
        for occ in occluders {
            canvas.push_occluder(*occ);
        }

        // Frame foreground (title, close button):
        painter.frame_foreground(
            &spec,
            Some(self.button_state(ConsoleButton::Close, look.closing, look.focused)),
        );

        // History (`RichText`, `DepressedButtonBorder`): filtered, wrapped, scrolled, one color
        // per level, the rows of selected lines highlighted.
        painter.fill(layout.history, self.style.entry);
        painter.sunken(layout.history);
        let view = HistoryView::new(&layout);
        let history = self.history_rows(canvas);
        let rows = view.rows.min(history.len());
        let scroll = self.scroll.min(history.len() - rows);
        let first = history.len() - rows - scroll;
        let area = layout.canvas(view.text);
        let row = CONSOLE_ROW * scale;
        for (i, shown) in history[first..first + rows].iter().enumerate() {
            let top = area.y + i as f32 * row;
            let selected = self.selection.is_some_and(|s| s.contains(shown.number));
            let color = if selected {
                canvas.rectangle(area.x, top, area.w, row, self.style.selected_bg);
                self.style.selected_text
            } else {
                self.line_color(shown.line)
            };
            let content = self.shown_text(shown.line)[shown.text.clone()]
                .replace('\t', &" ".repeat(TAB_SPACES));
            text(&content, area.x, top + row - CONSOLE_DESCENT * scale, color);
        }
        painter.scroll_bar(
            view.bar,
            first,
            view.rows,
            history.len(),
            self.history_bar.pressed(),
        );

        // The entry (`TextEntry`), or the reverse history search in its place.
        let entry_focused = !self.placement.active();
        let model = TextInputModel::new(look.focused, entry_focused);
        painter.text_entry_frame(layout.entry, model.should_draw_focus_edge());
        let (line, cursor) = match self.editor.search() {
            Some((query, found)) => {
                let line = format!(
                    "({})`{query}': {}",
                    look.reverse_search,
                    found.unwrap_or_default()
                );
                let cursor = line.chars().count();
                (line, cursor)
            }
            None => (self.editor.text().to_owned(), self.editor.cursor()),
        };
        let entry = layout.canvas(layout.entry);
        let inner_x = entry.x + (3. * scale).round();
        let inner_w = entry.w - (6. * scale).round();
        let chars: Vec<char> = line.chars().collect();
        let width = |from: usize, to: usize| {
            self.text_width(&chars[from..to].iter().collect::<String>(), scale)
        };
        let mut start = 0;
        while start < cursor && width(start, cursor) > inner_w - 2. * scale {
            start += 1;
        }
        let mut end = cursor;
        while end < chars.len() && width(start, end + 1) <= inner_w {
            end += 1;
        }
        let shown: String = chars[start..end].iter().collect();
        let row_top = entry.y + ((entry.h - CONSOLE_ROW * scale) * 0.5).round();
        text(
            &shown,
            inner_x,
            row_top + (CONSOLE_ROW - CONSOLE_DESCENT) * scale,
            painter.theme.text,
        );
        if model.should_draw_caret(time) {
            let caret_x = inner_x + width(start, cursor) + 1.;
            let dialog_x = (caret_x - painter.origin.x) / scale;
            painter.caret(layout.entry, dialog_x);
        }
        painter.button(
            layout.submit,
            look.submit,
            self.button_state(ConsoleButton::Submit, look.closing, look.focused),
        );
        self.draw_completion(
            canvas,
            look,
            scale,
            (entry.x, entry.y, entry.w, entry.h),
            &text,
        );

        for _ in 0..occluders.len() {
            canvas.pop_occluder();
        }
    }

    /// The completion popup's rectangle in canvas pixels, if it shows.
    pub fn completion_bounds<C: Context>(
        &self,
        canvas: &Canvas,
        theme: &Theme,
        commands: &'static [Spec<C>],
        context: &C,
    ) -> Option<Rect> {
        let layout = self.dialog_layout(canvas);
        let entry = layout.canvas(layout.entry);
        self.completion(
            canvas,
            theme,
            commands,
            context,
            layout.scale,
            (entry.x, entry.y, entry.w, entry.h),
        )
        .map(|completion| completion.rect)
    }

    /// The completion popup (SDK `CConsolePanel` completion list, VGUI menu colors): a help
    /// line for the command being typed, then up to `COMPLETION_ROWS` items, cvars with their
    /// value. It opens below the entry when it fits, else above it.
    /// The completion popup's rows and where it is drawn, if it shows.
    fn completion<C: Context>(
        &self,
        canvas: &Canvas,
        theme: &Theme,
        commands: &'static [Spec<C>],
        context: &C,
        scale: f32,
        (x, entry_y, w, entry_h): (f32, f32, f32, f32),
    ) -> Option<Completion> {
        let cvar_value = |name: &str| command::find(commands, name)?.value?(context);
        let mut rows: Vec<Vec<(String, Color)>> = Vec::new();
        let mut highlighted = None;
        if self.editor.search().is_none() {
            if let Some(help) = complete::arg_help(self.editor.text(), commands) {
                let mut runs = vec![(help.name.to_owned(), theme.text)];
                for (i, arg) in help.args.iter().enumerate() {
                    let color = if help.current == Some(i) {
                        theme.text
                    } else {
                        theme.text_dull
                    };
                    runs.push((format!(" {arg}"), color));
                }
                if help.cvar {
                    if let Some(value) = cvar_value(help.name) {
                        runs.push((format!(" = \"{value}\""), self.style.dev_text));
                    }
                }
                runs.push((format!("   {}", help.description), theme.label_text));
                rows.push(runs);
            }
        }
        let help_rows = rows.len();
        if let Some((items, index)) = self.editor.popup() {
            let more = items.len() > COMPLETION_ROWS;
            let shown = if more {
                COMPLETION_ROWS - 1
            } else {
                items.len()
            };
            // Keep the item stepped to in view.
            let first = index.map_or(0, |i| (i + 1).saturating_sub(shown));
            for (i, item) in items.iter().enumerate().skip(first).take(shown) {
                let mut runs = vec![(item.text.clone(), theme.text)];
                if let Some(value) = command::find(commands, &item.text)
                    .filter(|spec| spec.is_cvar())
                    .and_then(|spec| cvar_value(spec.name))
                {
                    runs.push((format!(" {value}"), self.style.dev_text));
                }
                if !item.detail.is_empty() {
                    runs.push((format!("   {}", item.detail), theme.label_text));
                }
                if index == Some(i) {
                    highlighted = Some(rows.len());
                }
                rows.push(runs);
            }
            if more {
                rows.push(vec![("...".to_owned(), theme.text_dull)]);
            }
        }
        if rows.is_empty() {
            return None;
        }
        // Like a VGUI menu, the list is as wide as its widest row (at least the entry), up to
        // the canvas edge.
        let widest = rows
            .iter()
            .map(|runs| {
                runs.iter()
                    .map(|(content, _)| self.text_width(content, scale))
                    .sum::<f32>()
            })
            .fold(0., f32::max);
        let w = w.max(widest + 12. * scale).min(canvas.width() - x).max(0.);
        let row = CONSOLE_ROW * scale;
        let h = rows.len() as f32 * row + 4. * scale;
        let below = entry_y + entry_h;
        let y = if below + h <= canvas.height() {
            below
        } else {
            (entry_y - h).max(0.)
        };
        Some(Completion {
            rows,
            highlighted,
            help_rows,
            rect: Rect::new(x, y, w, h),
        })
    }

    /// The completion popup over everything; it can reach past the console's frame.
    fn draw_completion<C: Context>(
        &self,
        canvas: &Canvas,
        look: &Look<'_, C>,
        scale: f32,
        entry: (f32, f32, f32, f32),
        text: &dyn Fn(&str, f32, f32, Color),
    ) {
        let theme = look.popup_theme;
        let Some(Completion {
            rows,
            highlighted,
            help_rows,
            rect,
        }) = self.completion(canvas, theme, look.commands, look.context, scale, entry)
        else {
            return;
        };
        let (x, y, w, h) = (rect.x, rect.y, rect.w, rect.h);
        let row = CONSOLE_ROW * scale;
        canvas.rectangle(
            x,
            y,
            w,
            h,
            Color::new(theme.menu_bg.r, theme.menu_bg.g, theme.menu_bg.b, 1.),
        );
        rectangle_lines(canvas, x, y, w, h, 1., theme.border_dark);
        let max_width = w - 12. * scale;
        for (r, runs) in rows.iter().enumerate() {
            let ry = y + 2. * scale + r as f32 * row;
            if highlighted == Some(r) {
                canvas.rectangle(x + 1., ry, w - 2., row, theme.selection_bg);
            }
            if r == help_rows && help_rows > 0 {
                canvas.rectangle(x + 1., ry, w - 2., 1., theme.border_dark);
            }
            let mut cx = x + 6. * scale;
            for (content, color) in runs {
                let color = if highlighted == Some(r) {
                    theme.selection_text
                } else {
                    *color
                };
                let mut clipped = String::new();
                for c in content.chars() {
                    clipped.push(c);
                    if cx - x + self.text_width(&clipped, scale) > max_width {
                        clipped.pop();
                        break;
                    }
                }
                text(
                    &clipped,
                    cx,
                    ry + (CONSOLE_ROW - CONSOLE_DESCENT) * scale,
                    color,
                );
                cx += self.text_width(&clipped, scale);
            }
        }
    }

    fn text_width(&self, content: &str, scale: f32) -> f32 {
        let em = (CONSOLE_TEXT * scale) as u16 as f32;
        self.font.as_ref().map_or_else(
            || content.chars().map(|c| self.advance(c, em)).sum(),
            |font| font.width_em(content, em),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::complete::Completion;

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

    /// An app whose console completes nothing.
    struct Nothing;

    impl Completer for Nothing {
        fn complete(&self, _: &str) -> Completion {
            Completion::default()
        }
    }

    fn texts(panel: &ConsolePanel) -> Vec<&str> {
        panel
            .shown_lines()
            .into_iter()
            .map(|(_, line)| line.text.as_str())
            .collect()
    }

    fn typed(text: &str) -> Input {
        Input {
            text: text.into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_scrollback_keeps_the_newest_lines_numbered_from_the_start() {
        let canvas = Canvas::default();
        let mut panel = ConsolePanel::new(SCALING);
        for i in 0..OUTPUT_LINES + 6 {
            panel.print(&canvas, Level::Normal, format!("line {i}"));
        }
        assert_eq!((panel.output.len(), panel.output_start), (OUTPUT_LINES, 6));
        let shown = panel.shown_lines();
        assert_eq!((shown[0].0, shown[0].1.text.as_str()), (6, "line 6"));
        panel.clear_output();
        assert!(panel.output.is_empty());
        panel.print(&canvas, Level::Normal, "after");
        assert_eq!(
            panel.shown_lines()[0].0,
            OUTPUT_LINES as u64 + 6,
            "numbers keep counting past a clear"
        );
    }

    #[test]
    fn a_snapshot_line_is_replaced_in_place() {
        let canvas = Canvas::default();
        let mut panel = ConsolePanel::new(SCALING);
        panel.print(&canvas, Level::Normal, "a");
        panel.replace_snapshot(&canvas, "tuning 1");
        panel.print(&canvas, Level::Normal, "b");
        panel.replace_snapshot(&canvas, "tuning 2");
        assert_eq!(texts(&panel), ["a", "tuning 2", "b"]);
        assert_eq!(panel.output[1].level, Level::Snapshot);
    }

    #[test]
    fn filters_and_timestamps_change_what_the_history_shows() {
        let canvas = Canvas::default();
        let mut panel = ConsolePanel::new(SCALING);
        let at = Input {
            time: Some(1.5),
            ..Default::default()
        };
        panel.input(&canvas, &at, &Nothing);
        for text in ["Loaded map", "fov 90", "MAP changed"] {
            panel.print(&canvas, Level::Normal, text);
        }
        panel.con_filter_text = "map".into();
        panel.con_filter = 1;
        assert_eq!(
            texts(&panel),
            ["Loaded map", "MAP changed"],
            "matches in any case"
        );
        panel.con_filter = 2;
        assert_eq!(texts(&panel).len(), 3, "every line, the matches brighter");
        assert_eq!(panel.line_color(&panel.output[0]), panel.style.dev_text);
        assert_eq!(panel.line_color(&panel.output[1]), panel.style.text);
        panel.con_timestamp = true;
        assert_eq!(panel.shown_text(&panel.output[1]), "[   1.50] fov 90");
    }

    #[test]
    fn enter_submits_the_line_and_the_console_keys_never_reach_it() {
        let canvas = Canvas::default();
        let mut panel = ConsolePanel::new(SCALING);
        assert_eq!(
            panel.input(&canvas, &typed("say hi"), &Nothing),
            Action::None
        );
        let enter = Input {
            keys: [Key::Enter].into(),
            ..Default::default()
        };
        assert_eq!(
            panel.input(&canvas, &enter, &Nothing),
            Action::Submit {
                line: "say hi".into(),
                clicked: false
            }
        );
        assert_eq!(
            (panel.editor.text(), panel.editor.history()),
            ("", &["say hi".to_owned()][..])
        );
        panel.input(&canvas, &typed("`~x"), &Nothing);
        assert_eq!(panel.editor.text(), "x");
    }

    #[test]
    fn close_and_submit_act_on_release_over_the_button() {
        let canvas = Canvas::default();
        let mut panel = ConsolePanel::new(SCALING);
        let layout = panel.dialog_layout(&canvas);
        let center = |rect: Rect| {
            let rect = layout.canvas(rect);
            vec2(rect.x + rect.w / 2., rect.y + rect.h / 2.)
        };
        let press = |at: Vec2| Input {
            pointer: Some(at),
            click: true,
            held: true,
            ..Default::default()
        };
        let release = |at: Vec2| Input {
            pointer: Some(at),
            released: true,
            ..Default::default()
        };
        let close = center(layout.close);
        assert_eq!(panel.input(&canvas, &press(close), &Nothing), Action::None);
        assert_eq!(
            panel.input(&canvas, &release(close), &Nothing),
            Action::Close
        );
        panel.input(&canvas, &typed("x"), &Nothing);
        let submit = center(layout.submit);
        panel.input(&canvas, &press(submit), &Nothing);
        assert_eq!(
            panel.input(&canvas, &release(submit), &Nothing),
            Action::Submit {
                line: "x".into(),
                clicked: true
            }
        );
        // Dragged off before the release, the press does nothing.
        panel.input(&canvas, &press(close), &Nothing);
        assert_eq!(
            panel.input(&canvas, &release(vec2(1., 1.)), &Nothing),
            Action::None
        );
    }

    #[test]
    fn the_copy_shortcut_copies_the_selected_lines_once() {
        let canvas = Canvas::default();
        let mut panel = ConsolePanel::new(SCALING);
        for text in ["one", "two", "three"] {
            panel.print(&canvas, Level::Normal, text);
        }
        panel.selection = Some(Selection { anchor: 2, head: 1 });
        let copy = Input {
            copy: true,
            ..Default::default()
        };
        panel.input(&canvas, &copy, &Nothing);
        assert_eq!(panel.take_copied().as_deref(), Some("two\nthree"));
        assert_eq!(panel.take_copied(), None);
    }

    #[test]
    fn text_width_measures_using_console_font_metrics_and_fallback() {
        let panel = ConsolePanel::new(SCALING);
        let em = (CONSOLE_TEXT * 1.0) as u16 as f32;
        let expected_fallback = 4. * (em * NOMINAL_ADVANCE);
        assert!((panel.text_width("test", 1.0) - expected_fallback).abs() < 1e-4);
    }

    #[test]
    fn console_input_entry_caret_positions_from_text_width() {
        let canvas = Canvas::default();
        let mut panel = ConsolePanel::new(SCALING);
        struct App;
        impl Context for App {
            type Effect = ();
            type Names = ();
        }
        impl Completer for App {
            fn complete(&self, _: &str) -> Completion {
                Completion::default()
            }
        }
        let app = App;
        panel.input(&canvas, &typed("hello"), &app);
        let fonts = Fonts::default();
        let theme = Theme::default();
        let look = Look {
            fonts: &fonts,
            theme: &theme,
            popup_theme: &theme,
            title: "Console",
            submit: "Submit",
            reverse_search: "bck-i-search",
            focused: true,
            closing: false,
            commands: &[],
            context: &app,
        };
        panel.draw(&canvas, &look, 0.0, &[]);
        let quads = canvas.drain();
        let layout = panel.dialog_layout(&canvas);
        let entry = layout.canvas(layout.entry);
        let inner_x = entry.x + (3. * layout.scale).round();
        let expected_caret_x = inner_x + panel.text_width("hello", layout.scale) + 1.;
        let expected_w = (1. * layout.scale).round().max(1.);
        let caret_quad = quads.iter().find(|q| {
            (q.destination.x - expected_caret_x).abs() < 1e-3 && q.destination.w == expected_w
        });
        assert!(
            caret_quad.is_some(),
            "caret quad at {expected_caret_x} should exist in quads with w={expected_w}"
        );

        // Scaled high-DPI canvas test: caret scales proportionally (matching Options text entry)
        canvas.resize(2560., 1440.);
        panel.draw(&canvas, &look, 0.0, &[]);
        let scaled_quads = canvas.drain();
        let scaled_layout = panel.dialog_layout(&canvas);
        assert_eq!(scaled_layout.scale, 2.0);
        let scaled_entry = scaled_layout.canvas(scaled_layout.entry);
        let scaled_inner_x = scaled_entry.x + (3. * scaled_layout.scale).round();
        let scaled_expected_caret_x =
            scaled_inner_x + panel.text_width("hello", scaled_layout.scale) + 1.;
        let scaled_caret = scaled_quads.iter().find(|q| {
            (q.destination.x - scaled_expected_caret_x).abs() < 1e-3 && q.destination.w == 2.0
        });
        assert!(
            scaled_caret.is_some(),
            "scaled caret quad at 1440p (scale 2.0) must have width 2.0"
        );
    }
}
