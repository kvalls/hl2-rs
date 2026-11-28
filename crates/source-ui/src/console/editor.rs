//! The console's input line: editing, the completion list, history and reverse search.
//!
//! Follows the SDK `CConsolePanel`: typing rebuilds a completion list for the line; Tab/Shift+Tab
//! step through completion candidates, and when a completion list is open Down/Up step forward and
//! backward through it. Without an open completion list, Up/Down browse the command history
//! (`MAX_HISTORY_ITEMS` 100; a repeated command moves to the end) in place without opening a
//! popup, saving any unsent draft and restoring it when stepping back past the newest entry. On
//! top of retail: fuzzy-ranked argument completion ([`super::complete`]), Tab on a single match
//! takes it with a space after it, Escape hides the list, Ctrl+R searches the history backwards,
//! and word and line editing keys.

use super::complete::{Candidate, Completer};
mod selection;

/// SDK `CConsolePanel::MAX_HISTORY_ITEMS`.
pub const HISTORY_MAX: usize = 100;
/// Longest input line.
const MAX_LINE: usize = 1024;
/// Undo history depth per unsubmitted input line.
const UNDO_MAX: usize = 64;

/// The completion list and what it completes.
#[derive(Clone, Debug, PartialEq, Eq)]
struct List {
    /// The line before the replaced token.
    head: String,
    items: Vec<Candidate>,
    /// The item stepped to, if any.
    index: Option<usize>,
    /// The line this list belongs to (the typed text, or the item stepped to): typing anything
    /// else rebuilds it.
    shows: String,
}

/// Active Up/Down command-history browsing (updates the entry in place without a popup,
/// restoring the unsent draft when stepping back past the newest entry).
#[derive(Clone, Debug, PartialEq, Eq)]
struct HistoryBrowse {
    index: usize,
    draft: String,
    draft_cursor: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineEditor {
    text: String,
    /// Cursor position in characters.
    cursor: usize,
    anchor: Option<usize>,
    undo_stack: Vec<(String, usize)>,
    history: Vec<String>,
    history_changed: bool,
    history_browse: Option<HistoryBrowse>,
    list: Option<List>,
    /// The line the list was hidden at with Escape; it comes back when the line changes.
    dismissed: Option<String>,
    /// Ctrl+R: the query and how many matches back.
    search: Option<(String, usize)>,
}

pub use selection::word_bounds;

pub(super) fn char_index(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(i, _)| i)
}

impl LineEditor {
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Cursor position in characters.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn history(&self) -> &[String] {
        &self.history
    }

    /// Whether Up/Down command-history browsing is currently active.
    pub fn browsing_history(&self) -> bool {
        self.history_browse.is_some()
    }

    /// Leaves history-browsing mode so the next refresh/Tab can open completions.
    pub fn stop_history_browse(&mut self) {
        self.history_browse = None;
    }

    /// Replaces the history (loaded by the host), keeping the newest `HISTORY_MAX`.
    pub fn set_history(&mut self, history: Vec<String>) {
        let skip = history.len().saturating_sub(HISTORY_MAX);
        self.history = history.into_iter().skip(skip).collect();
        self.history_browse = None;
    }

    /// Whether the history changed since the last call (the host saves it then).
    pub fn take_history_changed(&mut self) -> bool {
        std::mem::take(&mut self.history_changed)
    }

    /// The Ctrl+R query and the history entry it finds, while searching.
    pub fn search(&self) -> Option<(&str, Option<&str>)> {
        self.search
            .as_ref()
            .map(|(query, back)| (query.as_str(), self.found(query, *back)))
    }

    fn found(&self, query: &str, back: usize) -> Option<&str> {
        self.history
            .iter()
            .rev()
            .filter(|entry| entry.contains(query))
            .nth(back)
            .map(String::as_str)
    }

    /// The completion list to show and the item stepped to: not while searching, hidden with
    /// Escape, or empty.
    pub fn popup(&self) -> Option<(&[Candidate], Option<usize>)> {
        let list = self.list.as_ref()?;
        (self.search.is_none()
            && !list.items.is_empty()
            && self.dismissed.as_deref() != Some(self.text.as_str()))
        .then_some((list.items.as_slice(), list.index))
    }

    fn set(&mut self, text: String, cursor: usize) {
        self.anchor = None;
        self.text = text;
        self.cursor = cursor.min(self.text.chars().count());
    }

    /// The line changed by editing: leaves history browse and lets the list follow at refresh.
    fn edited(&mut self, text: String, cursor: usize) {
        if text != self.text {
            self.list = None;
            self.history_browse = None;
        }
        self.set(text, cursor);
    }

    /// Rebuilds the completion list when the line no longer is what the list shows.
    pub fn refresh(&mut self, completer: &impl Completer) {
        if self.history_browse.is_some() || self.list.as_ref().is_some_and(|l| l.shows == self.text)
        {
            return;
        }
        self.list = (!self.text.trim().is_empty()).then(|| {
            let completion = completer.complete(&self.text);
            List {
                head: completion.head,
                items: completion.items,
                index: None,
                shows: self.text.clone(),
            }
        });
    }

    fn push_undo(&mut self) {
        if self
            .undo_stack
            .last()
            .is_some_and(|(t, c)| *t == self.text && *c == self.cursor)
        {
            return;
        }
        if self.undo_stack.len() >= UNDO_MAX {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push((self.text.clone(), self.cursor));
    }

    /// Restores the text and cursor prior to the most recent edit (`Cmd/Ctrl+Z`).
    pub fn undo(&mut self) -> bool {
        self.accept_search();
        let Some((text, cursor)) = self.undo_stack.pop() else {
            return false;
        };
        self.edited(text, cursor);
        true
    }

    /// Deletes and returns the active selection, if any (`Cmd/Ctrl+X` / `Shift+Delete`).
    pub fn cut(&mut self) -> Option<String> {
        let selected = self.selected_text()?.to_owned();
        self.push_undo();
        self.delete_selection();
        Some(selected)
    }

    pub fn insert(&mut self, text: &str) {
        if let Some((query, _)) = &mut self.search {
            query.extend(text.chars().filter(|c| !c.is_control()));
            self.search = self.search.take().map(|(q, _)| (q, 0));
            return;
        }
        if text.chars().all(char::is_control) {
            return;
        }
        let has_selection = self.selection().is_some();
        let insert_non_empty = text.chars().any(|c| !c.is_control());
        if !has_selection && !insert_non_empty {
            return;
        }
        self.push_undo();
        self.delete_selection();
        let mut chars: Vec<char> = self.text.chars().collect();
        let insert: Vec<char> = text
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_LINE.saturating_sub(chars.len()))
            .collect();
        let at = self.cursor.min(chars.len());
        chars.splice(at..at, insert.iter().copied());
        self.edited(chars.into_iter().collect(), at + insert.len());
    }

    /// Pasted text: one line, line breaks become command separators.
    pub fn paste(&mut self, text: &str) {
        let line = text
            .trim_end()
            .replace("\r\n", "; ")
            .replace(['\n', '\r'], "; ");
        self.insert(&line);
    }

    pub fn backspace(&mut self) {
        if self.selection().is_some() {
            self.push_undo();
            self.delete_selection();
            return;
        }
        if let Some((query, _)) = &mut self.search {
            query.pop();
            self.search = self.search.take().map(|(q, _)| (q, 0));
            return;
        }
        if self.cursor > 0 {
            self.push_undo();
            let mut chars: Vec<char> = self.text.chars().collect();
            chars.remove(self.cursor - 1);
            self.edited(chars.into_iter().collect(), self.cursor - 1);
        }
    }

    pub fn delete(&mut self) {
        if self.selection().is_some() {
            self.push_undo();
            self.delete_selection();
            return;
        }
        let mut chars: Vec<char> = self.text.chars().collect();
        if self.cursor < chars.len() {
            self.push_undo();
            chars.remove(self.cursor);
            self.edited(chars.into_iter().collect(), self.cursor);
        }
    }

    /// Where the word before the cursor starts.
    fn word_start(&self) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut at = self.cursor.min(chars.len());
        while at > 0 && chars[at - 1].is_whitespace() {
            at -= 1;
        }
        while at > 0 && !chars[at - 1].is_whitespace() {
            at -= 1;
        }
        at
    }

    /// Where the word after the cursor ends.
    fn word_end(&self) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut at = self.cursor.min(chars.len());
        while at < chars.len() && chars[at].is_whitespace() {
            at += 1;
        }
        while at < chars.len() && !chars[at].is_whitespace() {
            at += 1;
        }
        at
    }

    /// Deletes the word before the cursor (or the active selection).
    pub fn delete_word_back(&mut self) {
        if self.selection().is_some() {
            self.push_undo();
            self.delete_selection();
            return;
        }
        let start = self.word_start();
        if start == self.cursor {
            return;
        }
        self.push_undo();
        let chars: Vec<char> = self.text.chars().collect();
        let text = chars[..start].iter().chain(&chars[self.cursor..]).collect();
        self.edited(text, start);
    }

    /// Deletes the word after the cursor (or the active selection).
    pub fn delete_word_forward(&mut self) {
        if self.selection().is_some() {
            self.push_undo();
            self.delete_selection();
            return;
        }
        let end = self.word_end();
        if end == self.cursor {
            return;
        }
        self.push_undo();
        let chars: Vec<char> = self.text.chars().collect();
        let text = chars[..self.cursor].iter().chain(&chars[end..]).collect();
        self.edited(text, self.cursor);
    }

    /// Deletes from the start of the line up to the cursor.
    pub fn kill_to_start(&mut self) {
        if self.selection().is_some() {
            self.push_undo();
            self.delete_selection();
            return;
        }
        if self.cursor == 0 {
            return;
        }
        self.push_undo();
        let rest = self.text[char_index(&self.text, self.cursor)..].to_owned();
        self.edited(rest, 0);
    }

    /// Deletes from the cursor to the end of the line.
    pub fn kill_to_end(&mut self) {
        if self.selection().is_some() {
            self.push_undo();
            self.delete_selection();
            return;
        }
        let byte = char_index(&self.text, self.cursor);
        if byte == self.text.len() {
            return;
        }
        self.push_undo();
        let kept = self.text[..byte].to_owned();
        let cursor = self.cursor;
        self.edited(kept, cursor);
    }

    pub fn left(&mut self) {
        self.anchor = None;
        self.accept_search();
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.anchor = None;
        self.accept_search();
        self.cursor = (self.cursor + 1).min(self.text.chars().count());
    }

    pub fn word_left(&mut self) {
        self.anchor = None;
        self.cursor = self.word_start();
    }

    pub fn word_right(&mut self) {
        self.anchor = None;
        let chars: Vec<char> = self.text.chars().collect();
        let mut at = self.cursor;
        while at < chars.len() && !chars[at].is_whitespace() {
            at += 1;
        }
        while at < chars.len() && chars[at].is_whitespace() {
            at += 1;
        }
        self.cursor = at;
    }

    pub fn home(&mut self) {
        self.anchor = None;
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.anchor = None;
        self.cursor = self.text.chars().count();
    }

    /// Steps forward (`!back`) or backward (`back`) through completion candidates (`Tab`/`Shift+Tab`,
    /// or `Up`/`Down` while a popup is open), or recalls command history in place (`Up`/`Down`
    /// without an open popup, saving and restoring any unsent draft).
    pub fn step(&mut self, back: bool, tab: bool) {
        if self.search.is_some() {
            if tab {
                self.accept_search();
            }
            return;
        }
        if tab {
            self.dismissed = None;
            self.history_browse = None;
            self.step_completion(back, true);
        } else if self.popup().is_some() {
            self.step_completion(back, false);
        } else {
            self.step_history(back);
        }
    }

    fn step_completion(&mut self, back: bool, tab: bool) {
        let Some(list) = self.list.as_mut().filter(|l| !l.items.is_empty()) else {
            return;
        };
        let token = &self.text[list.head.len()..];
        let single_prefix = !token.is_empty()
            && list.items.first().is_some_and(|c| {
                c.text
                    .to_ascii_lowercase()
                    .starts_with(&token.to_ascii_lowercase())
            })
            && list
                .items
                .iter()
                .filter(|c| {
                    c.text
                        .to_ascii_lowercase()
                        .starts_with(&token.to_ascii_lowercase())
                })
                .count()
                == 1;
        if tab && (list.items.len() == 1 || single_prefix) && list.index.is_none() {
            let text = format!("{}{} ", list.head, list.items[0].text);
            self.list = None;
            let cursor = text.chars().count();
            self.push_undo();
            self.set(text, cursor);
            return;
        }
        let len = list.items.len();
        let index = match (list.index, back) {
            (None, false) => 0,
            (None, true) => len - 1,
            (Some(i), false) => (i + 1) % len,
            (Some(i), true) => (i + len - 1) % len,
        };
        list.index = Some(index);
        let text = format!("{}{}", list.head, list.items[index].text);
        list.shows = text.clone();
        let cursor = text.chars().count();
        self.push_undo();
        self.set(text, cursor);
    }

    fn step_history(&mut self, back: bool) {
        if self.history.is_empty() {
            return;
        }
        match (self.history_browse.as_mut(), back) {
            (None, false) => {}
            (None, true) => {
                self.push_undo();
                let draft = self.text.clone();
                let draft_cursor = self.cursor;
                self.list = None;
                let index = self.history.len() - 1;
                let entry = self.history[index].clone();
                let cursor = entry.chars().count();
                self.history_browse = Some(HistoryBrowse {
                    index,
                    draft,
                    draft_cursor,
                });
                self.set(entry, cursor);
            }
            (Some(browse), true) => {
                if browse.index > 0 {
                    browse.index -= 1;
                    let entry = self.history[browse.index].clone();
                    let cursor = entry.chars().count();
                    self.set(entry, cursor);
                }
            }
            (Some(browse), false) => {
                if browse.index + 1 < self.history.len() {
                    browse.index += 1;
                    let entry = self.history[browse.index].clone();
                    let cursor = entry.chars().count();
                    self.set(entry, cursor);
                } else {
                    let browse = self.history_browse.take().expect("history browse state");
                    self.set(browse.draft, browse.draft_cursor);
                }
            }
        }
    }

    /// Escape: ends a search, else hides the list. Returns whether it did either (otherwise
    /// Escape closes the console).
    /// Whether [`Self::dismiss`] would end a search or hide the completion list.
    pub fn can_dismiss(&self) -> bool {
        self.search.is_some() || self.popup().is_some()
    }

    pub fn dismiss(&mut self) -> bool {
        if self.search.take().is_some() {
            return true;
        }
        if self.popup().is_some() {
            self.dismissed = Some(self.text.clone());
            return true;
        }
        false
    }

    /// Ctrl+R: start a reverse history search, or find the next older match.
    pub fn search_back(&mut self) {
        self.anchor = None;
        self.history_browse = None;
        self.search = Some(match self.search.take() {
            Some((query, back)) => {
                let next = back + 1;
                let exists = self.found(&query, next).is_some();
                (query, if exists { next } else { back })
            }
            None => (String::new(), 0),
        });
    }

    /// Ends a search, taking the entry it found into the line.
    pub fn accept_search(&mut self) {
        if let Some((query, back)) = self.search.take() {
            if let Some(found) = self.found(&query, back).map(str::to_owned) {
                let cursor = found.chars().count();
                self.edited(found, cursor);
            }
        }
    }

    /// Enter: the line to run (a search's entry when searching); it joins the history.
    pub fn submit(&mut self) -> String {
        self.anchor = None;
        self.accept_search();
        let line = std::mem::take(&mut self.text);
        self.cursor = 0;
        self.undo_stack.clear();
        self.history_browse = None;
        self.list = None;
        self.dismissed = None;
        let entry = line.trim();
        if !entry.is_empty() {
            self.history.retain(|h| h != entry);
            self.history.push(entry.to_owned());
            if self.history.len() > HISTORY_MAX {
                self.history.remove(0);
            }
            self.history_changed = true;
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::complete::Completion;

    const COMMANDS: &[&str] = &["fov", "mainmenu", "map", "maps", "voice_scale", "volume"];

    /// An app's completion: the first word from its commands, `map`'s argument from its maps,
    /// by prefix.
    #[derive(Default)]
    struct App {
        maps: Vec<&'static str>,
    }

    impl Completer for App {
        fn complete(&self, line: &str) -> Completion {
            let at = line.rfind(' ').map_or(0, |i| i + 1);
            let (head, token) = line.split_at(at);
            let names = match head.trim() {
                "" => COMMANDS.to_vec(),
                "map" => self.maps.clone(),
                _ => Vec::new(),
            };
            Completion {
                head: head.to_owned(),
                items: names
                    .into_iter()
                    .filter(|name| name.starts_with(token))
                    .map(|name| Candidate {
                        text: name.to_owned(),
                        detail: String::new(),
                    })
                    .collect(),
            }
        }
    }

    fn typed(text: &str) -> LineEditor {
        let mut editor = LineEditor::default();
        editor.insert(text);
        editor.refresh(&App::default());
        editor
    }

    #[test]
    fn editing_keys_work_on_characters_and_words() {
        let mut e = typed("ent_fire lab_door Open");
        assert_eq!(e.cursor(), 22);
        e.delete_word_back();
        assert_eq!(e.text(), "ent_fire lab_door ");
        e.word_left();
        assert_eq!(e.cursor(), 9);
        e.delete_word_forward();
        assert_eq!(e.text(), "ent_fire  ");
        e.home();
        e.insert("é");
        assert_eq!((e.text(), e.cursor()), ("éent_fire  ", 1));
        e.right();
        e.kill_to_start();
        assert_eq!((e.text(), e.cursor()), ("nt_fire  ", 0));
        e.word_right();
        e.kill_to_end();
        assert_eq!(e.text(), "nt_fire  ");
        e.end();
        e.backspace();
        assert_eq!(e.text(), "nt_fire ");
        e.home();
        e.delete();
        assert_eq!(e.text(), "t_fire ");
        e.paste("a\r\nb\n");
        assert_eq!(e.text(), "a; bt_fire ");
    }

    #[test]
    fn tab_steps_the_list_and_typing_rebuilds_it() {
        let mut e = typed("ma");
        let (items, index) = e.popup().unwrap();
        assert_eq!((items[0].text.as_str(), index), ("mainmenu", None));
        e.step(false, true);
        assert_eq!(e.text(), "mainmenu");
        e.step(false, true);
        assert_eq!(e.text(), "map");
        e.step(true, true);
        assert_eq!(e.text(), "mainmenu", "Shift+Tab steps back");
        e.step(false, true);
        e.refresh(&App::default());
        assert_eq!(e.popup().unwrap().1, Some(1), "stepping keeps the list");
        e.insert(" d");
        e.refresh(&App {
            maps: vec!["d1_trainstation_01"],
        });
        e.step(false, true);
        assert_eq!(
            e.text(),
            "map d1_trainstation_01 ",
            "one match: taken with a space"
        );
        assert!(e.popup().is_none());
    }

    #[test]
    fn an_empty_line_steps_through_the_history_and_submit_dedupes() {
        let mut e = LineEditor::default();
        for line in ["fov 90", "map x", "fov 90", "  "] {
            e.insert(line);
            e.submit();
        }
        assert_eq!(
            e.history(),
            ["map x", "fov 90"],
            "repeat moves to the end; blanks skipped"
        );
        assert!(e.take_history_changed());
        assert!(!e.take_history_changed());
        e.step(true, false);
        assert_eq!(e.text(), "fov 90", "Up starts at the newest");
        assert!(e.browsing_history() && e.popup().is_none());
        e.refresh(&App::default());
        assert!(e.popup().is_none(), "history recall opens no popup");
        e.step(true, false);
        assert_eq!(e.text(), "map x");
        e.step(true, false);
        assert_eq!(e.text(), "map x", "clamps at oldest history entry");
        e.step(false, false);
        assert_eq!(e.text(), "fov 90");
        e.step(false, false);
        assert_eq!(e.text(), "", "Down past newest restores empty line");
        assert!(!e.browsing_history() && e.popup().is_none());
        let mut many = LineEditor::default();
        many.set_history((0..150).map(|i| i.to_string()).collect());
        assert_eq!(
            (many.history().len(), many.history()[0].as_str()),
            (HISTORY_MAX, "50")
        );
    }

    #[test]
    fn history_browsing_preserves_and_restores_unsent_draft_without_popup() {
        let app = App {
            maps: vec!["d1_trainstation_01"],
        };
        let mut e = LineEditor::default();
        for cmd in ["map d1_trainstation_01", "fov 90"] {
            e.insert(cmd);
            e.submit();
        }
        e.insert("say hello world");
        e.word_left();
        e.refresh(&app);
        let saved_cursor = e.cursor();
        assert_eq!(e.text(), "say hello world");

        // Down on the unsent draft leaves it untouched.
        e.step(false, false);
        assert_eq!((e.text(), e.cursor()), ("say hello world", saved_cursor));
        assert!(!e.browsing_history());

        // Up recalls history without opening any popup even for known commands.
        e.step(true, false);
        e.refresh(&app);
        assert_eq!(e.text(), "fov 90");
        assert!(e.browsing_history() && e.popup().is_none());
        e.step(true, false);
        e.refresh(&app);
        assert_eq!(e.text(), "map d1_trainstation_01");
        assert!(e.popup().is_none());

        // Down steps forward through history and restores exact (draft, cursor) past newest.
        e.step(false, false);
        assert_eq!(e.text(), "fov 90");
        e.step(false, false);
        assert_eq!((e.text(), e.cursor()), ("say hello world", saved_cursor));
        assert!(!e.browsing_history());

        // Editing a recalled entry exits history browsing so refresh resumes completion.
        e.submit();
        e.insert("map");
        e.submit();
        e.step(true, false);
        e.backspace();
        assert!(!e.browsing_history());
        e.refresh(&app);
        assert!(e.popup().is_some());

        // Dismissed popup stays dismissed when returning to that draft via Down.
        assert!(e.dismiss());
        assert!(e.popup().is_none());
        let saved_partial = e.text().to_owned();
        e.step(true, false);
        assert_eq!(e.text(), "map");
        e.step(false, false);
        e.refresh(&app);
        assert_eq!(e.text(), saved_partial);
        assert!(e.popup().is_none());

        // Explicit Tab stops history browsing and completes the recalled entry.
        e.submit();
        e.insert("ma");
        e.submit();
        e.step(true, false);
        assert!(e.browsing_history());
        e.stop_history_browse();
        e.refresh(&app);
        e.step(false, true);
        assert!(!e.browsing_history());
        assert_eq!(e.text(), "mainmenu");
        assert!(e.popup().is_some());
    }

    #[test]
    fn escape_ends_a_search_then_hides_the_list() {
        let mut e = LineEditor::default();
        for line in ["volume 0.5", "fov 90", "volume 1"] {
            e.insert(line);
            e.submit();
        }
        e.search_back();
        e.insert("vol");
        assert_eq!(e.search(), Some(("vol", Some("volume 1"))));
        e.search_back();
        assert_eq!(e.search(), Some(("vol", Some("volume 0.5"))));
        e.search_back();
        assert_eq!(
            e.search().unwrap().1,
            Some("volume 0.5"),
            "stays on the oldest match"
        );
        assert!(e.dismiss(), "Escape ends the search");
        assert_eq!(e.text(), "");
        e.search_back();
        e.insert("fov");
        assert_eq!(e.submit(), "fov 90", "Enter runs the match");
        e.insert("vo");
        e.refresh(&App::default());
        assert!(e.popup().is_some());
        assert!(e.dismiss(), "Escape hides the list");
        assert!(e.popup().is_none());
        assert!(!e.dismiss(), "then Escape closes the console");
        e.insert("l");
        e.refresh(&App::default());
        assert!(e.popup().is_some(), "typing brings it back");
    }
}
