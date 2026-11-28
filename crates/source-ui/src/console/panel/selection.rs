//! Character-level history selection, word lookup, and highlighted row drawing.
use super::super::dialog::{DialogLayout, HistoryPos, Selection};
use super::super::editor::{char_index, word_bounds};
use super::printed::chat_label_end;
use super::{Canvas, Color, ConsolePanel, HistoryRow, HistoryView, CONSOLE_ROW, TAB_SPACES, WHITE};
use glam::Vec2;
use std::ops::Range;

pub(super) struct RowDraw<'a> {
    pub shown: &'a HistoryRow<'a>,
    pub area_x: f32,
    pub area_w: f32,
    pub top: f32,
    pub row_h: f32,
    pub baseline: f32,
    pub scale: f32,
    pub base_color: Color,
}

impl ConsolePanel {
    fn expanded_width(&self, slice: &str, scale: f32) -> f32 {
        if slice.contains('\t') {
            self.text_width(&slice.replace('\t', &" ".repeat(TAB_SPACES)), scale)
        } else {
            self.text_width(slice, scale)
        }
    }

    pub(super) fn history_offset_at(&self, content: &str, rel_x: f32, scale: f32) -> usize {
        if rel_x <= 0. {
            return 0;
        }
        let em = super::console_em(scale);
        let mut x = 0.;
        for (byte_idx, ch) in content.char_indices() {
            let w = self.advance(ch, em);
            if rel_x < x + w * 0.5 {
                return byte_idx;
            }
            x += w;
        }
        content.len()
    }

    pub(super) fn history_pos_at(
        &self,
        canvas: &Canvas,
        layout: &DialogLayout,
        view: &HistoryView,
        p_local: Vec2,
        p_canvas: Vec2,
        clamp: bool,
    ) -> Option<HistoryPos> {
        let history = self.history_rows(canvas);
        if history.is_empty() {
            return None;
        }
        let rows = view.rows.min(history.len());
        let scroll = self.scroll.min(history.len() - rows);
        let first = history.len() - rows - scroll;
        let visible = &history[first..first + rows];
        let row_idx = view.row_at(p_local.y, clamp)?;
        let area = layout.canvas(view.text);
        if row_idx >= visible.len() {
            let last = visible.last()?;
            return Some(HistoryPos::new(last.number, last.text.end));
        }
        let row = &visible[row_idx];
        let full = self.shown_text(row.line);
        let raw = &full[row.text.clone()];
        let rel_offset = self.history_offset_at(raw, p_canvas.x - area.x, layout.scale);
        Some(HistoryPos::new(row.number, row.text.start + rel_offset))
    }

    pub(super) fn history_word_selection(&self, pos: HistoryPos) -> Option<Selection> {
        let (_, line) = self
            .shown_lines()
            .into_iter()
            .find(|(num, _)| *num == pos.line)?;
        let text = self.shown_text(line);
        let clamped = pos.offset.min(text.len());
        let char_pos = text[..clamped].chars().count();
        let chars = word_bounds(&text, char_pos);
        let start = char_index(&text, chars.start);
        let end = char_index(&text, chars.end);
        Some(Selection {
            anchor: HistoryPos::new(pos.line, start),
            head: HistoryPos::new(pos.line, end),
        })
    }

    /// Selected history span as text, joining lines with newlines.
    pub(super) fn selected_text(&self) -> Option<String> {
        let selection = self.selection.filter(|s| !s.is_empty())?;
        let mut lines = Vec::new();
        for (number, line) in self.shown_lines() {
            if !selection.contains(number) {
                continue;
            }
            let text = self.shown_text(line);
            if let Some(range) = selection.line_range(number, &text) {
                lines.push(text[range].to_owned());
            }
        }
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    pub(super) fn draw_history_row(
        &self,
        canvas: &Canvas,
        row: &RowDraw<'_>,
        draw_text: &impl Fn(&str, f32, f32, Color),
    ) {
        let full = self.shown_text(row.shown.line);
        let span = &row.shown.text;
        let raw = &full[span.clone()];
        let label_end = (span.start == 0)
            .then(|| chat_label_end(row.shown.line.level, &full))
            .flatten();
        let line_sel = self
            .selection
            .and_then(|s| s.line_range(row.shown.number, &full));
        let sel_in_row = line_sel.as_ref().and_then(|sel| {
            let ends_later = self
                .selection
                .is_some_and(|s| s.ordered().1.line > row.shown.number);
            let overlaps = sel.start < span.end && sel.end > span.start;
            let on_line_end = span.end == full.len() && (ends_later || sel.end > span.end);
            (overlaps || on_line_end).then(|| {
                let s = sel.start.saturating_sub(span.start).min(raw.len());
                let e = sel.end.saturating_sub(span.start).min(raw.len());
                let includes_eol = (ends_later && span.end == full.len()) || sel.end > span.end;
                (s..e, includes_eol)
            })
        });
        let Some((sel_range, includes_eol)) = sel_in_row else {
            self.draw_unselected_slice(raw, 0..raw.len(), label_end, row, draw_text);
            return;
        };
        let bg_x = row.area_x + self.expanded_width(&raw[..sel_range.start], row.scale);
        let bg_w = if includes_eol {
            (row.area_x + row.area_w - bg_x).max(4. * row.scale)
        } else {
            self.expanded_width(&raw[sel_range.clone()], row.scale)
        };
        if bg_w > 0. {
            canvas.rectangle(bg_x, row.top, bg_w, row.row_h, self.style.selected_bg);
        }
        self.draw_unselected_slice(raw, 0..sel_range.start, label_end, row, draw_text);
        self.draw_slice(
            raw,
            sel_range.clone(),
            self.style.selected_text,
            row,
            draw_text,
        );
        self.draw_unselected_slice(raw, sel_range.end..raw.len(), label_end, row, draw_text);
    }

    fn draw_unselected_slice(
        &self,
        raw: &str,
        slice: Range<usize>,
        label_end: Option<usize>,
        row: &RowDraw<'_>,
        draw_text: &impl Fn(&str, f32, f32, Color),
    ) {
        if slice.is_empty() {
            return;
        }
        if let Some(split) = label_end {
            if slice.end <= split {
                self.draw_slice(raw, slice, WHITE, row, draw_text);
                return;
            }
            if slice.start < split {
                self.draw_slice(raw, slice.start..split, WHITE, row, draw_text);
                self.draw_slice(raw, split..slice.end, row.base_color, row, draw_text);
                return;
            }
        }
        self.draw_slice(raw, slice, row.base_color, row, draw_text);
    }

    fn draw_slice(
        &self,
        raw: &str,
        slice: Range<usize>,
        color: Color,
        row: &RowDraw<'_>,
        draw_text: &impl Fn(&str, f32, f32, Color),
    ) {
        if slice.is_empty() {
            return;
        }
        let x = row.area_x + self.expanded_width(&raw[..slice.start], row.scale);
        let expanded = raw[slice].replace('\t', &" ".repeat(TAB_SPACES));
        draw_text(&expanded, x, row.baseline, color);
    }
}

pub(super) fn draw_history_rows(
    panel: &ConsolePanel,
    canvas: &Canvas,
    layout: &DialogLayout,
    view: &HistoryView,
    text: &impl Fn(&str, f32, f32, Color),
) {
    let history = panel.history_rows(canvas);
    let rows = view.rows.min(history.len());
    let scroll = panel.scroll.min(history.len() - rows);
    let first = history.len() - rows - scroll;
    let area = layout.canvas(view.text);
    let row_h = CONSOLE_ROW * layout.scale;
    for (i, shown) in history[first..first + rows].iter().enumerate() {
        let top = area.y + i as f32 * row_h;
        let baseline = top + row_h - super::CONSOLE_DESCENT * layout.scale;
        panel.draw_history_row(
            canvas,
            &RowDraw {
                shown,
                area_x: area.x,
                area_w: area.w,
                top,
                row_h,
                baseline,
                scale: layout.scale,
                base_color: panel.line_color(shown.line),
            },
            text,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::panel::tests::SCALING;
    use crate::console::panel::Level;

    #[test]
    fn partial_and_tabbed_history_selections_map_and_copy_cleanly() {
        let canvas = Canvas::default();
        let mut panel = ConsolePanel::new(SCALING);
        panel.print(&canvas, Level::Normal, "a\tb\tc");
        panel.print(&canvas, Level::Normal, "sv_cheats 1");
        let em = super::super::console_em(1.0);
        let adv_a = panel.advance('a', em);
        let adv_tab = panel.advance('\t', em);
        assert!((adv_tab - 8. * panel.advance(' ', em)).abs() < 1e-3);
        assert_eq!(
            panel.history_offset_at("a\tb\tc", adv_a + adv_tab + 0.1, 1.0),
            2
        );
        // Word select on second line picks `sv_cheats`.
        panel.selection = panel.history_word_selection(HistoryPos::new(1, 3));
        assert_eq!(panel.selected_text().as_deref(), Some("sv_cheats"));
        // Partial span across lines joins the sliced substrings with a newline.
        panel.selection = Some(Selection {
            anchor: HistoryPos::new(0, 2),
            head: HistoryPos::new(1, 2),
        });
        assert_eq!(panel.selected_text().as_deref(), Some("b\tc\nsv"));
    }
}
