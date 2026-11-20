//! Lines printed since they were last taken, for readers outside the dialog (a log file, a
//! remote console client). Kept only while a reader asked for them.
use super::{ConsolePanel, Line};

impl ConsolePanel {
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
        Redirect { before, kept }
    }

    /// What was printed since [`Self::start_redirect`], taken out of the history and the kept
    /// lines.
    pub fn finish_redirect(&mut self, redirect: Redirect) -> Vec<Line> {
        let printed = self.printed.get_or_insert_with(Vec::new);
        let lines = printed.split_off(redirect.before.min(printed.len()));
        self.keep_printed(redirect.kept);
        let keep = self.output.len().saturating_sub(lines.len());
        self.output.truncate(keep);
        self.scroll = 0;
        self.selection = None;
        lines
    }
}

/// A redirect in progress: where its lines start among the kept ones.
#[must_use]
pub struct Redirect {
    before: usize,
    kept: bool,
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
}
