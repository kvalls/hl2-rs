//! Input text, selection and caret in the console entry.
use super::{
    Color, ConsolePanel, Context, DialogLayout, Look, Painter, CONSOLE_DESCENT, CONSOLE_ROW,
};
use crate::widgets::TextInputModel;

impl ConsolePanel {
    pub(super) fn effective_entry_start(
        &self,
        chars: &[char],
        cursor: usize,
        inner_w: f32,
        scale: f32,
    ) -> usize {
        let mut start = self.entry_scroll.min(cursor);
        while start < cursor
            && self.entry_slice_width(chars, start, cursor, scale) > inner_w - 2. * scale
        {
            start += 1;
        }
        start
    }

    pub(super) fn entry_visible_end(
        &self,
        chars: &[char],
        start: usize,
        inner_w: f32,
        scale: f32,
    ) -> usize {
        let mut end = start;
        while end < chars.len() && self.entry_slice_width(chars, start, end + 1, scale) <= inner_w {
            end += 1;
        }
        end
    }

    pub(super) fn entry_char_at(
        &self,
        chars: &[char],
        start: usize,
        rel_x: f32,
        scale: f32,
    ) -> usize {
        if rel_x <= 0. || start >= chars.len() {
            return start;
        }
        let mut x = 0.;
        for i in start..chars.len() {
            let next_x = self.entry_slice_width(chars, start, i + 1, scale);
            let char_w = next_x - x;
            if rel_x < x + char_w * 0.5 {
                return i;
            }
            x = next_x;
        }
        chars.len()
    }

    fn entry_slice_width(&self, chars: &[char], from: usize, to: usize, scale: f32) -> f32 {
        let slice: String = chars[from..to].iter().collect();
        self.text_width(&slice, scale)
    }

    pub(super) fn draw_entry<C: Context>(
        &self,
        painter: &Painter,
        layout: &DialogLayout,
        look: &Look<'_, C>,
        time: f64,
        text: &impl Fn(&str, f32, f32, Color),
    ) {
        // The entry (`TextEntry`), or the reverse history search in its place.
        let scale = layout.scale;
        let entry_focused = !self.placement.active();
        let model = TextInputModel::new(look.focused, entry_focused)
            .with_interaction(self.entry_interaction);
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
        let width = |from: usize, to: usize| self.entry_slice_width(&chars, from, to, scale);
        let start = self.effective_entry_start(&chars, cursor, inner_w, scale);
        let end = self
            .entry_visible_end(&chars, start, inner_w, scale)
            .max(cursor);
        let row_top = entry.y + ((entry.h - CONSOLE_ROW * scale) * 0.5).round();
        let selection = self
            .editor
            .selection()
            .filter(|_| self.editor.search().is_none());
        let selected = selection
            .map(|r| r.start.max(start)..r.end.min(end))
            .filter(|r| r.start < r.end);
        if let Some(r) = &selected {
            painter.canvas.rectangle(
                inner_x + width(start, r.start),
                row_top,
                width(r.start, r.end),
                CONSOLE_ROW * scale,
                painter.theme.selection_bg,
            );
        }
        let spans = match selected {
            Some(r) => [
                (start..r.start, painter.theme.text),
                (r.clone(), painter.theme.selection_text),
                (r.end..end, painter.theme.text),
            ],
            None => [
                (start..end, painter.theme.text),
                (end..end, painter.theme.text),
                (end..end, painter.theme.text),
            ],
        };
        for (span, color) in spans {
            if span.is_empty() {
                continue;
            }
            let from = super::super::editor::char_index(&line, span.start);
            let to = super::super::editor::char_index(&line, span.end);
            text(
                &line[from..to],
                inner_x + width(start, span.start),
                row_top + (CONSOLE_ROW - CONSOLE_DESCENT) * scale,
                color,
            );
        }
        if model.should_draw_caret(time) {
            let caret_x = inner_x + width(start, cursor) + 1.;
            let dialog_x = (caret_x - painter.origin.x) / scale;
            painter.caret(layout.entry, dialog_x);
        }
    }
}
