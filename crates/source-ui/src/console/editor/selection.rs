//! Character-based input selection, independent of the host clipboard.
use super::{char_index, LineEditor};
use crate::input::Key;
use std::ops::Range;

impl LineEditor {
    pub fn selection(&self) -> Option<Range<usize>> {
        let anchor = self.anchor?;
        (anchor != self.cursor).then_some(anchor.min(self.cursor)..anchor.max(self.cursor))
    }

    pub fn selected_text(&self) -> Option<&str> {
        let range = self.selection()?;
        Some(&self.text[char_index(&self.text, range.start)..char_index(&self.text, range.end)])
    }

    pub fn clear_selection(&mut self) {
        self.anchor = None;
    }

    pub fn select_all(&mut self) {
        self.accept_search();
        self.anchor = Some(0);
        self.cursor = self.text.chars().count();
    }

    pub(super) fn delete_selection(&mut self) -> bool {
        let Some(range) = self.selection() else {
            return false;
        };
        let mut text = self.text.clone();
        text.replace_range(
            char_index(&text, range.start)..char_index(&text, range.end),
            "",
        );
        self.edited(text, range.start);
        true
    }

    /// Shift extends from a stable anchor; plain arrows collapse a selection toward that edge.
    pub fn navigate(&mut self, key: Key, word: bool, selecting: bool) {
        self.accept_search();
        let start = self.cursor;
        let anchor = self.anchor.unwrap_or(start);
        let selection = self.selection();
        self.anchor = None;
        match (key, word, selecting, selection) {
            (Key::Left, false, false, Some(range)) => self.cursor = range.start,
            (Key::Right, false, false, Some(range)) => self.cursor = range.end,
            (Key::Left, true, _, _) => self.word_left(),
            (Key::Right, true, _, _) => self.word_right(),
            (Key::Left, false, _, _) => self.left(),
            (Key::Right, false, _, _) => self.right(),
            (Key::Home | Key::Up, _, _, _) => self.home(),
            (Key::End | Key::Down, _, _, _) => self.end(),
            _ => return,
        }
        if selecting {
            self.anchor = Some(anchor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_selection_reverses_collapses_and_replaces() {
        let mut editor = LineEditor::default();
        editor.insert("aé日本z");
        editor.navigate(Key::Left, false, true);
        editor.navigate(Key::Left, false, true);
        assert_eq!(editor.selected_text(), Some("本z"));
        editor.navigate(Key::Right, false, true);
        assert_eq!(editor.selected_text(), Some("z"));
        editor.navigate(Key::Left, false, false);
        assert_eq!(editor.cursor(), 4);
        assert_eq!(editor.selection(), None);
        editor.navigate(Key::Home, false, true);
        assert_eq!(editor.selected_text(), Some("aé日本"));
        editor.paste("λ\nβ");
        assert_eq!(editor.text(), "λ; βz");
        editor.select_all();
        editor.insert("");
        assert_eq!(editor.selected_text(), Some("λ; βz"));
        editor.backspace();
        assert_eq!(editor.text(), "");
    }
    #[test]
    fn selection_survives_refresh_and_is_replaced_by_deletion_or_history() {
        let mut editor = LineEditor::default();
        editor.insert("map 日本");
        editor.select_all();
        editor.delete_word_forward();
        assert_eq!(editor.text(), "");
        editor.insert("help");
        editor.navigate(Key::Left, false, true);
        assert_eq!(editor.selected_text(), Some("p"));
        editor.submit();
        assert!(editor.selection().is_none());
        editor.step(true, false);
        assert_eq!(editor.text(), "help");
        assert!(editor.selection().is_none());
    }
}
