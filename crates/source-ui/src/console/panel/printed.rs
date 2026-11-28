//! Lines printed since they were last taken, for readers outside the dialog (a log file, a
//! remote console client). Kept only while a reader asked for them.
use super::{Canvas, ConsolePanel, Level, Line, Selection, OUTPUT_LINES};

impl ConsolePanel {
    /// Prints a line at `level`, stamped with the current time.
    pub fn print(&mut self, canvas: &Canvas, level: Level, text: impl Into<String>) {
        self.push_output(canvas, level, text.into(), true);
    }

    /// Prints a line with alternate text for readers outside the dialog.
    pub fn print_with_log_text(
        &mut self,
        canvas: &Canvas,
        level: Level,
        text: String,
        log_text: impl FnOnce() -> String,
    ) {
        self.print(canvas, level, text);
        if let Some(line) = self.printed.as_mut().and_then(|lines| lines.last_mut()) {
            line.text = log_text();
        }
    }

    pub(super) fn push_output(
        &mut self,
        canvas: &Canvas,
        level: Level,
        text: String,
        record_printed: bool,
    ) {
        let line = Line {
            level,
            text,
            time: self.clock,
        };
        if record_printed {
            if let Some(printed) = &mut self.printed {
                printed.push(line.clone());
            }
        }
        let pinned = (record_printed && self.redirecting == 0)
            .then(|| self.active_snapshot_index())
            .flatten()
            .filter(|&index| index + 1 == self.output.len())
            .and_then(|_| self.output.pop_back());
        self.output.push_back(line);
        let mut inserted_index = self.output.len() - 1;
        if let Some(snapshot) = pinned {
            let old_number = self.output_start + inserted_index as u64;
            let new_number = old_number + 1;
            self.output.push_back(snapshot);
            self.move_pinned_snapshot(old_number, new_number);
        }
        if self.output.len() > OUTPUT_LINES {
            self.output.pop_front();
            self.output_start += 1;
            inserted_index = inserted_index.saturating_sub(1);
        }
        // Scrolled back, the view stays on its rows while new ones arrive above any pinned line.
        let number = self.output_start + inserted_index as u64;
        if let Some(line) = self
            .output
            .get(inserted_index)
            .filter(|line| self.scroll > 0 && (self.con_filter != 1 || self.filter_match(line)))
        {
            self.scroll += self
                .line_rows(number, line, self.wrap_metrics(canvas))
                .len();
        }
    }

    fn move_pinned_snapshot(&mut self, old_number: u64, new_number: u64) {
        self.snapshot = Some(new_number);
        if let Some(mut selection) = self.selection {
            if selection.anchor.line == old_number {
                selection.anchor.line = new_number;
            }
            if selection.head.line == old_number {
                selection.head.line = new_number;
            }
            self.selection = Some(selection);
        }
        if let Ok(mut wrapped) = self.wrapped.lock() {
            if let Some(row) = wrapped.remove(&old_number) {
                wrapped.insert(new_number, row);
            }
        }
    }

    /// Starts or stops keeping printed lines for [`Self::take_printed`]. Off, printing keeps
    /// nothing extra and lines not yet taken are dropped.
    pub fn keep_printed(&mut self, on: bool) {
        if on {
            self.printed.get_or_insert_with(Vec::new);
        } else {
            self.printed = None;
        }
    }

    /// Whether printed lines are being kept.
    pub fn keeps_printed(&self) -> bool {
        self.printed.is_some()
    }

    /// The lines printed since the last call, oldest first; none while not kept.
    pub fn take_printed(&mut self) -> Vec<Line> {
        self.printed
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }

    /// Starts sending what is printed to a remote client instead of the history; end with
    /// [`Self::finish_redirect`].
    pub fn start_redirect(&mut self) -> Redirect {
        let kept = self.keeps_printed();
        let before = self.printed.get_or_insert_with(Vec::new).len();
        self.redirecting += 1;
        Redirect {
            before,
            kept,
            output_start: self.output_start,
            scroll: self.scroll,
            selection: self.selection,
        }
    }

    /// What was printed since [`Self::start_redirect`], taken out of the history and the kept
    /// lines. Preserves the user's scroll position and text selection when no earlier history
    /// lines were evicted during the redirect.
    pub fn finish_redirect(&mut self, redirect: Redirect) -> Vec<Line> {
        self.redirecting = self.redirecting.saturating_sub(1);
        let printed = self.printed.get_or_insert_with(Vec::new);
        let lines = printed.split_off(redirect.before.min(printed.len()));
        self.keep_printed(redirect.kept);
        let keep = self.output.len().saturating_sub(lines.len());
        self.output.truncate(keep);
        if self
            .snapshot
            .is_some_and(|n| n >= self.output_start + self.output.len() as u64)
        {
            self.snapshot = None;
        }
        if self.output_start == redirect.output_start {
            self.scroll = redirect.scroll;
            self.selection = redirect.selection;
        } else {
            self.scroll = 0;
            self.selection = None;
        }
        lines
    }
}

/// Byte offset after the speaker label (`"Speaker:"`, including any timestamp prefix) on the
/// first row of a chat line, so the label draws in white before the colored message body.
pub fn chat_label_end(level: Level, content: &str) -> Option<usize> {
    matches!(level, Level::ChatLocal | Level::ChatRemote)
        .then(|| content.find(": ").map(|i| i + 1))
        .flatten()
}

/// A redirect in progress: where its lines start among the kept ones.
#[must_use]
pub struct Redirect {
    before: usize,
    kept: bool,
    output_start: u64,
    scroll: usize,
    selection: Option<Selection>,
}

#[cfg(test)]
mod tests {
    use super::super::Level;
    use super::*;
    use crate::canvas::Canvas;

    fn texts(lines: &[Line]) -> Vec<&str> {
        lines.iter().map(|line| line.text.as_str()).collect()
    }

    #[test]
    fn nothing_is_kept_until_a_reader_asks() {
        let canvas = Canvas::default();
        let mut panel = ConsolePanel::new(super::super::tests::SCALING);
        panel.print(&canvas, Level::Normal, "before");
        assert!(panel.take_printed().is_empty());
        panel.keep_printed(true);
        panel.print(&canvas, Level::Input, "] echo a");
        panel.print(&canvas, Level::Normal, "a");
        assert_eq!(texts(&panel.take_printed()), ["] echo a", "a"]);
        assert!(panel.take_printed().is_empty());
        panel.keep_printed(false);
        panel.print(&canvas, Level::Normal, "after");
        assert!(!panel.keeps_printed() && panel.take_printed().is_empty());
    }

    #[test]
    fn alternate_log_text_keeps_the_displayed_line() {
        let canvas = Canvas::default();
        let mut panel = ConsolePanel::new(super::super::tests::SCALING);
        panel.keep_printed(true);
        panel.print_with_log_text(&canvas, Level::ChatLocal, "visible".into(), || {
            "logged".into()
        });
        assert_eq!(panel.output.back().unwrap().text, "visible");
        assert_eq!(texts(&panel.take_printed()), ["logged"]);
    }

    #[test]
    fn redirected_output_leaves_the_history_and_the_kept_lines() {
        let canvas = Canvas::default();
        let mut panel = ConsolePanel::new(super::super::tests::SCALING);
        panel.keep_printed(true);
        panel.print(&canvas, Level::Normal, "local");
        let redirect = panel.start_redirect();
        panel.print(&canvas, Level::Normal, "one");
        panel.print(&canvas, Level::Warning, "two");
        let lines = panel.finish_redirect(redirect);
        assert_eq!(texts(&lines), ["one", "two"]);
        assert_eq!(panel.output.len(), 1);
        assert_eq!(texts(&panel.take_printed()), ["local"]);
    }

    #[test]
    fn a_redirect_without_a_reader_keeps_nothing_after_it() {
        let canvas = Canvas::default();
        let mut panel = ConsolePanel::new(super::super::tests::SCALING);
        let redirect = panel.start_redirect();
        panel.print(&canvas, Level::Normal, "reply");
        assert_eq!(texts(&panel.finish_redirect(redirect)), ["reply"]);
        assert!(!panel.keeps_printed() && panel.output.is_empty());
    }

    #[test]
    fn a_redirect_preserves_scroll_and_selection() {
        let canvas = Canvas::default();
        let mut panel = ConsolePanel::new(super::super::tests::SCALING);
        for i in 0..20 {
            panel.print(&canvas, Level::Normal, format!("line {i}"));
        }
        panel.scroll = 7;
        let selection = Selection::lines(2, 5);
        panel.selection = Some(selection);
        let redirect = panel.start_redirect();
        panel.print(&canvas, Level::Normal, "remote reply");
        assert_eq!(texts(&panel.finish_redirect(redirect)), ["remote reply"]);
        assert_eq!(panel.scroll, 7);
        assert_eq!(panel.selection, Some(selection));
    }
}
