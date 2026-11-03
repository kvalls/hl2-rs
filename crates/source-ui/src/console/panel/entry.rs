//! Input text, selection and caret in the console entry.
use super::{
    Color, ConsolePanel, Context, DialogLayout, Look, Painter, CONSOLE_DESCENT, CONSOLE_ROW,
};
use crate::widgets::TextInputModel;
impl ConsolePanel {
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
