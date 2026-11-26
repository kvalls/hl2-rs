//! Character-based input selection, independent of the host clipboard.
use super::{char_index, LineEditor};
use crate::input::Key;
use std::ops::Range;

/// Word (or contiguous whitespace) range in character indices around `char_pos`, matching SDK
/// double-click selection boundaries (trims trailing whitespace around words).
pub fn word_bounds(text: &str, char_pos: usize) -> Range<usize> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return 0..0;
    }
    let pos = char_pos.min(chars.len());
    let probe = if pos < chars.len() && !chars[pos].is_whitespace() {
        pos
    } else if pos > 0 && !chars[pos - 1].is_whitespace() {
        pos - 1
    } else {
        pos.min(chars.len() - 1)
    };
    let is_space = chars[probe].is_whitespace();
    let mut start = probe;
    while start > 0 && chars[start - 1].is_whitespace() == is_space {
        start -= 1;
    }
    let mut end = probe + 1;
    while end < chars.len() && chars[end].is_whitespace() == is_space {
        end += 1;
    }
    start..end
}

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
        let end = self.text.chars().count();
        self.anchor = (end > 0).then_some(0);
        self.cursor = end;
    }

    /// Places the caret on pointer press (`extend` preserves the selection anchor for Shift+click).
    pub fn set_cursor(&mut self, pos: usize, extend: bool) {
        self.accept_search();
        let target = pos.min(self.text.chars().count());
        if extend {
            let anchor = self.anchor.unwrap_or(self.cursor);
            self.anchor = Some(anchor);
        } else {
            self.anchor = Some(target);
        }
        self.cursor = target;
    }

    /// Extends mouse drag selection to character index `pos`.
    pub fn drag_to(&mut self, pos: usize) {
        if self.anchor.is_none() {
            self.anchor = Some(self.cursor);
        }
        self.cursor = pos.min(self.text.chars().count());
    }

    /// Selects the word (or whitespace run) around character index `pos` (SDK double-click).
    pub fn select_word_at(&mut self, pos: usize) {
        self.accept_search();
        let range = word_bounds(&self.text, pos);
        if range.start < range.end {
            self.anchor = Some(range.start);
            self.cursor = range.end;
        } else {
            self.anchor = None;
            self.cursor = range.end;
        }
    }

    /// Collapses a zero-width selection when a mouse drag finishes.
    pub fn end_drag(&mut self) {
        if self.anchor == Some(self.cursor) {
            self.anchor = None;
        }
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
        assert!(editor.undo());
        assert_eq!(editor.text(), "λ; βz");
    }

    #[test]
    fn cut_undo_and_mouse_word_bounds_match_retail() {
        let mut editor = LineEditor::default();
        editor.insert("sv_cheats  1");
        editor.select_word_at(2);
        assert_eq!(editor.selected_text(), Some("sv_cheats"));
        assert_eq!(editor.cut().as_deref(), Some("sv_cheats"));
        assert_eq!(editor.text(), "  1");
        assert!(editor.undo());
        assert_eq!(editor.text(), "sv_cheats  1");
        // Double-click on trailing boundary picks word; middle of multi-space picks whitespace.
        assert_eq!(word_bounds("fov   90", 3), 0..3);
        assert_eq!(word_bounds("fov   90", 4), 3..6);
        assert_eq!(word_bounds("fov   90", 8), 6..8);
        // Click + drag + release, then empty click collapses anchor.
        editor.set_cursor(0, false);
        assert_eq!(editor.selection(), None);
        editor.drag_to(9);
        assert_eq!(editor.selected_text(), Some("sv_cheats"));
        editor.end_drag();
        assert_eq!(editor.selected_text(), Some("sv_cheats"));
        editor.set_cursor(4, false);
        editor.end_drag();
        assert_eq!(editor.selection(), None);
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
