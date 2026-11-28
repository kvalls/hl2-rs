//! Console mouse interaction: buttons, history scroll bar/wheel, entry caret/word/triple-click
//! selection, and output history character/word selection.
use super::super::dialog::{DialogLayout, HistoryPos, Selection};
use super::super::editor::word_bounds;
use super::{Action, Canvas, ConsoleButton, ConsolePanel, HistoryView, WHEEL_ROWS};
use crate::input::Input;
use crate::widgets::{ScrollBarLayout, ScrollMode, DOUBLE_CLICK_SECONDS};
use glam::Vec2;

/// Maximum pointer drift between clicks in a double/triple-click sequence (canvas pixels).
const MULTI_CLICK_DIST_SQ: f32 = 16.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum WordAnchor {
    #[default]
    None,
    Entry {
        start: usize,
        end: usize,
    },
    History(Selection),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClickTarget {
    Entry,
    History,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ClickStreak {
    target: Option<ClickTarget>,
    pos: Vec2,
    time: f64,
    count: u8,
}

impl ClickStreak {
    fn register(&mut self, target: ClickTarget, at: Vec2, time: Option<f64>) -> u8 {
        let Some(now) = time else {
            self.target = Some(target);
            self.pos = at;
            self.count = 1;
            return 1;
        };
        if self.target == Some(target)
            && (0. ..=DOUBLE_CLICK_SECONDS).contains(&(now - self.time))
            && (at - self.pos).length_squared() <= MULTI_CLICK_DIST_SQ
        {
            self.count = (self.count % 3) + 1;
        } else {
            self.count = 1;
        }
        self.target = Some(target);
        self.pos = at;
        self.time = now;
        self.count
    }
}

impl ConsolePanel {
    /// Pointer input of the console dialog: close/Submit buttons, scrollbar, wheel, entry mouse
    /// selection, and history character/word selection.
    pub(super) fn pointer_input(&mut self, canvas: &Canvas, input: &Input) -> Option<Action> {
        let layout = self.dialog_layout(canvas);
        let view = HistoryView::new(&layout);
        let point = input.pointer.map(|p| layout.local(p));
        let down = input.click || input.held;
        let total = self.history_rows(canvas).len();
        let max_scroll = total.saturating_sub(view.rows);
        if self.handle_scroll_and_wheel(&view, point, input, down, total, max_scroll) {
            return None;
        }
        let (hovered, action) = self.handle_buttons(&layout, point, input, down);
        if action.is_some() {
            return action;
        }
        if let (Some(p_local), Some(p_canvas)) = (point, input.pointer) {
            if self.entry_selecting || (input.click && layout.entry.contains(p_local)) {
                self.pointer_entry(canvas, &layout, p_canvas, input);
            }
            if !self.entry_selecting
                && (self.selecting
                    || (input.click && hovered.is_none() && view.text.contains(p_local)))
            {
                self.pointer_history(canvas, &layout, &view, p_canvas, max_scroll, input);
            }
        }
        if !down {
            if self.entry_selecting {
                self.editor.end_drag();
                self.entry_selecting = false;
            }
            if self.selecting {
                self.selecting = false;
                if self.selection.is_some_and(Selection::is_empty) {
                    self.selection = None;
                }
            }
            self.drag_word = WordAnchor::None;
        }
        None
    }

    fn handle_scroll_and_wheel(
        &mut self,
        view: &HistoryView,
        point: Option<Vec2>,
        input: &Input,
        down: bool,
        total: usize,
        max_scroll: usize,
    ) -> bool {
        let mode = ScrollMode::platform();
        let steps = self.history_wheel.steps(input.wheel, mode) as i64
            * mode.rows_per_step(WHEEL_ROWS) as i64;
        self.scroll = (self.scroll as i64 + steps).clamp(0, max_scroll as i64) as usize;
        self.scroll = self.scroll.min(max_scroll);
        let first = total - view.rows.min(total) - self.scroll;
        let bar = ScrollBarLayout::new(view.bar, first, view.rows, total);
        if !self.selecting
            && !self.entry_selecting
            && self.history_bar.wants(&bar, point, input.click)
        {
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
            return true;
        }
        false
    }

    fn handle_buttons(
        &mut self,
        layout: &DialogLayout,
        point: Option<Vec2>,
        input: &Input,
        down: bool,
    ) -> (Option<ConsoleButton>, Option<Action>) {
        let hovered = point.and_then(|p| {
            [
                (ConsoleButton::Close, layout.close),
                (ConsoleButton::Submit, layout.submit),
            ]
            .into_iter()
            .find(|(_, rect)| rect.contains(p))
            .map(|(button, _)| button)
        });
        if self.selecting || self.entry_selecting {
            self.buttons.occlude(down);
            self.hover = None;
            return (hovered, None);
        }
        self.hover = hovered;
        let action = match self
            .buttons
            .update(hovered, down, input.click, input.released)
        {
            Some(ConsoleButton::Close) => {
                self.hover = None;
                Some(Action::Close)
            }
            Some(ConsoleButton::Submit) => {
                let line = self.editor.submit();
                self.scroll = 0;
                self.entry_scroll = 0;
                Some(Action::Submit {
                    line,
                    clicked: true,
                })
            }
            None => None,
        };
        (hovered, action)
    }

    fn pointer_entry(
        &mut self,
        _canvas: &Canvas,
        layout: &DialogLayout,
        p_canvas: Vec2,
        input: &Input,
    ) {
        let scale = layout.scale;
        let entry = layout.canvas(layout.entry);
        let inner_x = entry.x + (3. * scale).round();
        let inner_w = entry.w - (6. * scale).round();
        let rel_x = p_canvas.x - inner_x;
        if self.editor.search().is_some() && input.click {
            self.editor.accept_search();
        }
        let chars: Vec<char> = self.editor.text().chars().collect();
        let start = self.effective_entry_start(&chars, self.editor.cursor(), inner_w, scale);
        if input.click {
            self.selection = None;
            self.selecting = false;
            if let Some(t) = input.time {
                self.entry_interaction = Some(t);
            }
            let count = self
                .click_streak
                .register(ClickTarget::Entry, p_canvas, input.time);
            match count {
                3 => {
                    self.editor.select_all();
                    self.drag_word = WordAnchor::None;
                    self.entry_selecting = false;
                    return;
                }
                2 => {
                    let pos = self.entry_char_at(&chars, start, rel_x, scale);
                    self.editor.select_word_at(pos);
                    self.drag_word =
                        self.editor
                            .selection()
                            .map_or(WordAnchor::None, |r| WordAnchor::Entry {
                                start: r.start,
                                end: r.end,
                            });
                    self.entry_scroll =
                        self.effective_entry_start(&chars, self.editor.cursor(), inner_w, scale);
                    self.entry_selecting = true;
                    return;
                }
                _ => {
                    let pos = self.entry_char_at(&chars, start, rel_x, scale);
                    self.editor.set_cursor(pos, input.shift);
                    self.drag_word = WordAnchor::None;
                    self.entry_scroll = self.effective_entry_start(&chars, pos, inner_w, scale);
                    self.entry_selecting = true;
                    return;
                }
            }
        }
        if self.entry_selecting && (input.click || input.held) {
            if let Some(t) = input.time {
                self.entry_interaction = Some(t);
            }
            let pos = if rel_x < 0. {
                let new_start = start.saturating_sub(1);
                self.entry_scroll = new_start;
                new_start
            } else if rel_x > inner_w {
                let end = self.entry_visible_end(&chars, start, inner_w, scale);
                let pos = (end + 1).min(chars.len());
                self.entry_scroll = self.effective_entry_start(&chars, pos, inner_w, scale);
                pos
            } else {
                self.entry_scroll = start;
                self.entry_char_at(&chars, start, rel_x, scale)
            };
            self.extend_entry_drag(pos);
        }
    }

    fn extend_entry_drag(&mut self, pos: usize) {
        if let WordAnchor::Entry { start: ws, end: we } = self.drag_word {
            let cur = word_bounds(self.editor.text(), pos);
            if pos < ws {
                self.editor.select_range(we, cur.start);
            } else if pos > we {
                self.editor.select_range(ws, cur.end);
            } else {
                self.editor.select_range(ws, we);
            }
        } else {
            self.editor.drag_to(pos);
        }
    }

    fn pointer_history(
        &mut self,
        canvas: &Canvas,
        layout: &DialogLayout,
        view: &HistoryView,
        p_canvas: Vec2,
        max_scroll: usize,
        input: &Input,
    ) {
        let p_local = layout.local(p_canvas);
        if input.click {
            self.editor.clear_selection();
            self.entry_selecting = false;
            self.selecting = true;
            let count = self
                .click_streak
                .register(ClickTarget::History, p_canvas, input.time);
            let pos = self.history_pos_at(canvas, layout, view, p_local, p_canvas, false);
            self.selection = pos.and_then(|pos| {
                if count == 2 {
                    self.history_word_selection(pos)
                } else {
                    Some(Selection::at_pos(pos))
                }
            });
            self.drag_word = if count == 2 {
                self.selection.map_or(WordAnchor::None, WordAnchor::History)
            } else {
                WordAnchor::None
            };
        } else if self.selecting && (input.click || input.held) {
            if p_local.y < view.text.y {
                self.scroll = (self.scroll + 1).min(max_scroll);
            } else if p_local.y > view.text.y + view.text.h {
                self.scroll = self.scroll.saturating_sub(1);
            }
            if let Some(pos) = self.history_pos_at(canvas, layout, view, p_local, p_canvas, true) {
                self.extend_history_drag(pos);
            }
        }
    }

    fn extend_history_drag(&mut self, pos: HistoryPos) {
        if let WordAnchor::History(anchor_word) = self.drag_word {
            let (ws, we) = anchor_word.ordered();
            let cur = self
                .history_word_selection(pos)
                .map(Selection::ordered)
                .unwrap_or((pos, pos));
            self.selection = Some(if pos < ws {
                Selection {
                    anchor: we,
                    head: cur.0,
                }
            } else if pos > we {
                Selection {
                    anchor: ws,
                    head: cur.1,
                }
            } else {
                Selection {
                    anchor: ws,
                    head: we,
                }
            });
        } else if let Some(selection) = self.selection.as_mut() {
            selection.head = pos;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::panel::tests::SCALING;
    use crate::console::panel::Level;
    use glam::vec2;

    fn click_at(at: Vec2, time: f64) -> Input {
        Input {
            pointer: Some(at),
            click: true,
            held: true,
            time: Some(time),
            ..Default::default()
        }
    }

    #[test]
    fn entry_mouse_drag_double_and_triple_click_select_characters_words_and_all() {
        let canvas = Canvas::default();
        let mut panel = ConsolePanel::new(SCALING);
        let layout = panel.dialog_layout(&canvas);
        let entry = layout.canvas(layout.entry);
        let inner_x = entry.x + (3. * layout.scale).round();
        let y = entry.y + entry.h * 0.5;
        let x_he = inner_x + panel.text_width("echo he", layout.scale) + 0.1;
        let x_hello = inner_x + panel.text_width("echo hello", layout.scale) + 0.1;
        let nothing = &crate::console::panel::test_helpers::Nothing;
        panel.input(
            &canvas,
            &Input {
                text: "echo hello world".into(),
                ..Default::default()
            },
            nothing,
        );

        // Single click places caret inside `"hello"` (`"echo he|llo world"`).
        panel.input(&canvas, &click_at(vec2(x_he, y), 1.0), nothing);
        assert_eq!(panel.editor.cursor(), 7);
        assert_eq!(panel.editor.selection(), None);

        // Drag to end of `"hello"` selects `"llo"`.
        panel.input(
            &canvas,
            &Input {
                pointer: Some(vec2(x_hello, y)),
                held: true,
                time: Some(1.1),
                ..Default::default()
            },
            nothing,
        );
        panel.input(
            &canvas,
            &Input {
                pointer: Some(vec2(x_hello, y)),
                released: true,
                time: Some(1.2),
                ..Default::default()
            },
            nothing,
        );
        assert_eq!(panel.editor.selected_text(), Some("llo"));

        // Double click at same spot + held frame preserves full word `"hello"`, and dragging right extends by word.
        panel.input(&canvas, &click_at(vec2(x_he, y), 2.0), nothing);
        panel.input(
            &canvas,
            &Input {
                pointer: Some(vec2(x_he, y)),
                released: true,
                time: Some(2.05),
                ..Default::default()
            },
            nothing,
        );
        panel.input(&canvas, &click_at(vec2(x_he, y), 2.2), nothing);
        assert_eq!(panel.editor.selected_text(), Some("hello"));
        panel.input(
            &canvas,
            &Input {
                pointer: Some(vec2(x_he, y)),
                held: true,
                time: Some(2.22),
                ..Default::default()
            },
            nothing,
        );
        assert_eq!(panel.editor.selected_text(), Some("hello"));
        let x_wor = inner_x + panel.text_width("echo hello wor", layout.scale) + 0.1;
        panel.input(
            &canvas,
            &Input {
                pointer: Some(vec2(x_wor, y)),
                held: true,
                time: Some(2.25),
                ..Default::default()
            },
            nothing,
        );
        assert_eq!(panel.editor.selected_text(), Some("hello world"));
        let x_ec = inner_x + panel.text_width("ec", layout.scale) + 0.1;
        panel.input(
            &canvas,
            &Input {
                pointer: Some(vec2(x_ec, y)),
                held: true,
                time: Some(2.28),
                ..Default::default()
            },
            nothing,
        );
        assert_eq!(panel.editor.selected_text(), Some("echo hello"));
        panel.input(
            &canvas,
            &Input {
                pointer: Some(vec2(x_ec, y)),
                released: true,
                time: Some(2.3),
                ..Default::default()
            },
            nothing,
        );

        // Triple click at one spot selects the entire input line.
        for (i, t) in [3.0, 3.2, 3.4].into_iter().enumerate() {
            panel.input(&canvas, &click_at(vec2(x_he, y), t), nothing);
            if i < 2 {
                panel.input(
                    &canvas,
                    &Input {
                        pointer: Some(vec2(x_he, y)),
                        released: true,
                        time: Some(t + 0.05),
                        ..Default::default()
                    },
                    nothing,
                );
            }
        }
        assert_eq!(panel.editor.selected_text(), Some("echo hello world"));
    }

    #[test]
    fn history_double_click_and_empty_click_follow_retail() {
        let canvas = Canvas::default();
        let mut panel = ConsolePanel::new(SCALING);
        let nothing = &crate::console::panel::test_helpers::Nothing;
        panel.print(&canvas, Level::Normal, "map d1_trainstation_02 end");
        let layout = panel.dialog_layout(&canvas);
        let area = layout.canvas(HistoryView::new(&layout).text);
        let x = area.x + panel.text_width("map d1_", layout.scale);
        let y = area.y + CONSOLE_ROW_MID * layout.scale;

        // Single click without drag clears empty selection on release.
        panel.input(&canvas, &click_at(vec2(x, y), 1.0), nothing);
        panel.input(
            &canvas,
            &Input {
                pointer: Some(vec2(x, y)),
                released: true,
                time: Some(1.1),
                ..Default::default()
            },
            nothing,
        );
        assert_eq!(panel.selection, None);

        // Double click + held frame keeps `"d1_trainstation_02"` selected, and dragging right extends by word.
        panel.input(&canvas, &click_at(vec2(x, y), 1.25), nothing);
        panel.input(
            &canvas,
            &Input {
                pointer: Some(vec2(x, y)),
                held: true,
                time: Some(1.27),
                ..Default::default()
            },
            nothing,
        );
        assert_eq!(panel.selected_text().as_deref(), Some("d1_trainstation_02"));
        let x_en = area.x + panel.text_width("map d1_trainstation_02 en", layout.scale);
        panel.input(
            &canvas,
            &Input {
                pointer: Some(vec2(x_en, y)),
                held: true,
                time: Some(1.29),
                ..Default::default()
            },
            nothing,
        );
        panel.input(
            &canvas,
            &Input {
                pointer: Some(vec2(x_en, y)),
                released: true,
                time: Some(1.31),
                ..Default::default()
            },
            nothing,
        );
        panel.input(
            &canvas,
            &Input {
                copy: true,
                time: Some(1.33),
                ..Default::default()
            },
            nothing,
        );
        assert_eq!(
            panel.take_copied().as_deref(),
            Some("d1_trainstation_02 end")
        );
    }

    const CONSOLE_ROW_MID: f32 = super::super::CONSOLE_ROW * 0.5;
}
