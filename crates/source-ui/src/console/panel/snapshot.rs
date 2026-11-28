//! Replaceable output follows ordinary log traffic without growing the buffer.
use super::{Canvas, ConsolePanel, Level};

impl ConsolePanel {
    /// Remove the previous snapshot and append the latest one after all ordinary output.
    pub fn replace_snapshot(&mut self, canvas: &Canvas, text: impl Into<String>) {
        self.replace_snapshot_with_level(canvas, Level::Snapshot, text);
    }

    /// Remove the previous snapshot and append the latest one at `level` after all ordinary
    /// output, without recording it in the kept printed lines for external log readers.
    pub fn replace_snapshot_with_level(
        &mut self,
        canvas: &Canvas,
        level: Level,
        text: impl Into<String>,
    ) {
        self.clear_snapshot(canvas);
        self.push_output(canvas, level, text.into(), false);
        let number = self.output_start + self.output.len() as u64 - 1;
        self.snapshot = Some(number);
    }

    pub(super) fn active_snapshot_index(&self) -> Option<usize> {
        self.snapshot.and_then(|number| {
            number
                .checked_sub(self.output_start)
                .and_then(|offset| usize::try_from(offset).ok())
                .filter(|&index| index < self.output.len())
        })
    }

    /// Remove the current snapshot line, if any, without disturbing ordinary output or scroll.
    pub fn clear_snapshot(&mut self, canvas: &Canvas) {
        let index = self.active_snapshot_index();
        self.snapshot = None;
        if let Some(index) = index {
            self.remove_snapshot(canvas, index);
        }
    }

    fn remove_snapshot(&mut self, canvas: &Canvas, index: usize) {
        let number = self.output_start + index as u64;
        if self.scroll > 0 {
            let rows = self.history_rows(canvas);
            let bottom = rows.len().saturating_sub(self.scroll);
            let below = rows
                .iter()
                .skip(bottom)
                .filter(|row| row.number == number)
                .count();
            self.scroll = self.scroll.saturating_sub(below);
        }
        self.output.remove(index);
        if let Some(mut selection) = self.selection {
            if selection.anchor.line == number || selection.head.line == number {
                self.selection = None;
                self.selecting = false;
            } else {
                selection.anchor.line -= u64::from(selection.anchor.line > number);
                selection.head.line -= u64::from(selection.head.line > number);
                self.selection = Some(selection);
            }
        }
        // Ordinary rows shift down one index. Move their cached wrapping with them,
        // without rewrapping or cloning the rest of the console history.
        if let Ok(mut wrapped) = self.wrapped.lock() {
            wrapped.remove(&number);
            let last = self.output_start + self.output.len() as u64;
            for key in number..last {
                if let Some(row) = wrapped.remove(&(key + 1)) {
                    wrapped.insert(key, row);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Canvas, ConsolePanel, Level};
    use crate::console::dialog::{Scaling, Selection};
    use crate::console::panel::OUTPUT_LINES;

    fn panel() -> (Canvas, ConsolePanel) {
        (
            Canvas::default(),
            ConsolePanel::new(Scaling {
                dialog_scale: |_| 1.,
                design_height: |size| size.y,
            }),
        )
    }

    #[test]
    fn cached_wrapping_and_ordinary_selection_follow_their_rows() {
        let (canvas, mut panel) = panel();
        panel.print(&canvas, Level::Normal, "a");
        panel.replace_snapshot(&canvas, "old\nsnapshot");
        panel.print(&canvas, Level::Normal, "b");
        panel.print(&canvas, Level::Normal, "c");
        panel.history_rows(&canvas);
        // Pinned snapshot stays last: ["a", "b", "c", "old\nsnapshot"].
        panel.selection = Some(Selection::at(1));
        panel.replace_snapshot(&canvas, "latest");
        assert_eq!(panel.selected_text().as_deref(), Some("b"));
        let rows: Vec<_> = panel
            .history_rows(&canvas)
            .into_iter()
            .map(|r| panel.shown_text(r.line)[r.text].to_owned())
            .collect();
        assert_eq!(rows, ["a", "b", "c", "latest"]);
        panel.print(&canvas, Level::Normal, "tail");
        // ["a", "b", "c", "tail", "latest"]
        panel.selection = Some(Selection::lines(0, 3));
        panel.replace_snapshot(&canvas, "newest");
        assert_eq!(panel.selected_text().as_deref(), Some("a\nb\nc\ntail"));
        // Selecting the pinned snapshot follows it past interleaved ordinary output.
        panel.selection = Some(Selection::at(4));
        panel.print(&canvas, Level::Normal, "more");
        assert_eq!(panel.selected_text().as_deref(), Some("newest"));
        panel.clear_snapshot(&canvas);
        assert!(panel.output.iter().all(|l| l.level != Level::Snapshot));
        assert_eq!(
            panel.selection, None,
            "removed snapshot cannot select unrelated text"
        );
    }

    #[test]
    fn wrapped_and_filtered_snapshot_moves_preserve_scrolled_back_ordinary_content() {
        for position in [10, 90] {
            for filtered in [false, true] {
                let (canvas, mut panel) = panel();
                canvas.resize(1024., 768.);
                panel.con_filter = u8::from(filtered);
                panel.con_filter_text = "keep".into();
                for i in 0..100 {
                    if i == position {
                        panel.replace_snapshot(&canvas, "old\nsnapshot");
                    }
                    panel.print(&canvas, Level::Normal, format!("keep line {i}"));
                }
                panel.scroll = 20;
                let rows = panel.history_rows(&canvas);
                let old_row = &rows[rows.len() - panel.scroll - 1];
                let bottom = panel.shown_text(old_row.line)[old_row.text.clone()].to_owned();
                panel.replace_snapshot(&canvas, "keep latest\nthree\nrows");
                let rows = panel.history_rows(&canvas);
                let new_row = &rows[rows.len() - panel.scroll - 1];
                assert_eq!(
                    &panel.shown_text(new_row.line)[new_row.text.clone()],
                    bottom
                );
            }
        }
    }

    #[test]
    fn replacing_in_a_full_buffer_keeps_ordinary_lines_and_normal_eviction() {
        let (canvas, mut panel) = panel();
        panel.replace_snapshot(&canvas, "old");
        for i in 0..OUTPUT_LINES - 1 {
            panel.print(&canvas, Level::Normal, format!("line {i}"));
        }
        panel.replace_snapshot(&canvas, "latest");
        assert_eq!(panel.output.len(), OUTPUT_LINES);
        assert_eq!(panel.output_start, 0);
        assert_eq!(panel.output.front().unwrap().text, "line 0");
        assert_eq!(panel.output.back().unwrap().text, "latest");
        for i in 0..OUTPUT_LINES {
            panel.print(&canvas, Level::Normal, i.to_string());
        }
        panel.replace_snapshot(&canvas, "after eviction");
        assert_eq!(panel.output.len(), OUTPUT_LINES);
        assert_eq!(panel.output.back().unwrap().text, "after eviction");
        assert_eq!(
            panel
                .output
                .iter()
                .filter(|l| l.level == Level::Snapshot)
                .count(),
            1
        );
    }

    #[test]
    fn styled_snapshot_uses_requested_level_and_stays_out_of_printed_log() {
        let (canvas, mut panel) = panel();
        panel.keep_printed(true);
        panel.print(&canvas, Level::Normal, "ordinary");
        panel.replace_snapshot_with_level(&canvas, Level::ChatRemote, "Console: Thinking...");
        panel.replace_snapshot_with_level(&canvas, Level::ChatRemote, "Console: Waiting...");
        assert_eq!(panel.output.len(), 2);
        assert_eq!(panel.output[1].level, Level::ChatRemote);
        assert_eq!(panel.output[1].text, "Console: Waiting...");

        let rows = panel.history_rows(&canvas);
        let drawn = std::cell::RefCell::new(Vec::new());
        panel.draw_history_row(
            &canvas,
            &super::super::selection::RowDraw {
                shown: &rows[1],
                area_x: 0.,
                area_w: 200.,
                top: 0.,
                row_h: 14.,
                baseline: 0.,
                scale: 1.,
                base_color: panel.line_color(rows[1].line),
            },
            &|text, _, _, color| drawn.borrow_mut().push((text.to_owned(), color)),
        );
        assert_eq!(
            *drawn.borrow(),
            vec![
                ("Console:".into(), crate::canvas::WHITE),
                (" Waiting...".into(), super::super::CHAT_REMOTE_TEXT),
            ]
        );

        let printed: Vec<_> = panel.take_printed().into_iter().map(|l| l.text).collect();
        assert_eq!(printed, ["ordinary"]);

        panel.print(&canvas, Level::ChatRemote, "Console: reply");
        panel.clear_snapshot(&canvas);
        let remaining: Vec<_> = panel.output.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(remaining, ["ordinary", "Console: reply"]);
    }

    #[test]
    fn pinned_snapshot_stays_last_across_prints_redirects_and_scrolling() {
        let (canvas, mut panel) = panel();
        canvas.resize(1024., 768.);
        for i in 0..30 {
            panel.print(&canvas, Level::Normal, format!("line {i}"));
        }
        panel.replace_snapshot_with_level(&canvas, Level::ChatRemote, "Console: Working...");
        panel.print(&canvas, Level::Normal, "rcon cmd 1");
        panel.print(&canvas, Level::Normal, "rcon cmd 2");
        assert_eq!(
            panel.output.back().map(|l| (l.level, l.text.as_str())),
            Some((Level::ChatRemote, "Console: Working..."))
        );

        let redirect = panel.start_redirect();
        panel.print(&canvas, Level::Normal, "redirected output");
        let redirected = panel.finish_redirect(redirect);
        assert_eq!(redirected.len(), 1);
        assert_eq!(redirected[0].text, "redirected output");
        assert_eq!(
            panel.output.back().map(|l| (l.level, l.text.as_str())),
            Some((Level::ChatRemote, "Console: Working..."))
        );

        panel.print(&canvas, Level::Normal, "rcon cmd 3");
        let len = panel.output.len();
        assert_eq!(panel.output[len - 2].text, "rcon cmd 3");
        assert_eq!(panel.output[len - 1].text, "Console: Working...");

        panel.scroll = 10;
        let rows = panel.history_rows(&canvas);
        let before = &rows[rows.len() - panel.scroll - 1];
        let bottom = panel.shown_text(before.line)[before.text.clone()].to_owned();
        panel.print(&canvas, Level::Normal, "wrapped\ntwo rows");
        let rows = panel.history_rows(&canvas);
        let after = &rows[rows.len() - panel.scroll - 1];
        assert_eq!(&panel.shown_text(after.line)[after.text.clone()], bottom);
        assert_eq!(panel.output.back().unwrap().text, "Console: Working...");
    }
}
