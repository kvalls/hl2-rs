//! Replaceable output follows ordinary log traffic without growing the buffer.
use super::{Canvas, ConsolePanel, Level};

impl ConsolePanel {
    /// Remove the previous snapshot and append the latest one after all ordinary output.
    pub fn replace_snapshot(&mut self, canvas: &Canvas, text: impl Into<String>) {
        if let Some(index) = self
            .output
            .iter()
            .rposition(|line| line.level == Level::Snapshot)
        {
            self.remove_snapshot(canvas, index);
        }
        self.print(canvas, Level::Snapshot, text);
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
            if selection.anchor == number || selection.head == number {
                self.selection = None;
                self.selecting = false;
            } else {
                selection.anchor -= u64::from(selection.anchor > number);
                selection.head -= u64::from(selection.head > number);
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
        panel.selection = Some(Selection::at(2));
        panel.replace_snapshot(&canvas, "latest");
        assert_eq!(panel.selected_text().as_deref(), Some("b"));
        let rows: Vec<_> = panel
            .history_rows(&canvas)
            .into_iter()
            .map(|r| panel.shown_text(r.line)[r.text].to_owned())
            .collect();
        assert_eq!(rows, ["a", "b", "c", "latest"]);
        panel.print(&canvas, Level::Normal, "tail");
        panel.selection = Some(Selection { anchor: 0, head: 4 });
        panel.replace_snapshot(&canvas, "newest");
        assert_eq!(panel.selected_text().as_deref(), Some("a\nb\nc\ntail"));
        panel.selection = Some(Selection::at(4));
        panel.replace_snapshot(&canvas, "final");
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
}
