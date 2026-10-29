//! The console's input line: editing, the completion list, history and reverse search.
//!
//! Follows the SDK `CConsolePanel`: typing rebuilds a completion list for the line; Tab and
//! Down step forward through it, Shift+Tab and Up back, each step putting that item in the
//! line, and typing again rebuilds the list from the new text. On an empty line the list is the
//! command history (`MAX_HISTORY_ITEMS` 100; a repeated command moves to the end). On top of
//! retail: fuzzy-ranked argument completion ([`super::complete`]), Tab on a single match takes
//! it with a space after it, Up/Down browse the history while the line has no matches, Escape
//! hides the list, Ctrl+R searches the history backwards, and word and line editing keys.

use super::complete::{Candidate, Completer};

/// SDK `CConsolePanel::MAX_HISTORY_ITEMS`.
pub const HISTORY_MAX: usize = 100;
/// Longest input line.
const MAX_LINE: usize = 1024;

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineEditor {
    text: String,
    /// Cursor position in characters.
    cursor: usize,
    history: Vec<String>,
    history_changed: bool,
    list: Option<List>,
    /// The line the list was hidden at with Escape; it comes back when the line changes.
    dismissed: Option<String>,
    /// Ctrl+R: the query and how many matches back.
    search: Option<(String, usize)>,
}

fn char_index(text: &str, chars: usize) -> usize {
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

    /// Replaces the history (loaded by the host), keeping the newest `HISTORY_MAX`.
    pub fn set_history(&mut self, history: Vec<String>) {
        let skip = history.len().saturating_sub(HISTORY_MAX);
        self.history = history.into_iter().skip(skip).collect();
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
        self.text = text;
        self.cursor = cursor.min(self.text.chars().count());
    }

    /// The line changed by editing: the list follows the new text at the next refresh.
    fn edited(&mut self, text: String, cursor: usize) {
        if text != self.text {
            self.list = None;
        }
        self.set(text, cursor);
    }

    /// Rebuilds the completion list when the line no longer is what the list shows.
    pub fn refresh(&mut self, completer: &impl Completer) {
        if self.list.as_ref().is_some_and(|l| l.shows == self.text) {
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

    pub fn insert(&mut self, text: &str) {
        if let Some((query, _)) = &mut self.search {
            query.extend(text.chars().filter(|c| !c.is_control()));
            self.search = self.search.take().map(|(q, _)| (q, 0));
            return;
        }
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
        if let Some((query, _)) = &mut self.search {
            query.pop();
            self.search = self.search.take().map(|(q, _)| (q, 0));
            return;
        }
        if self.cursor > 0 {
            let mut chars: Vec<char> = self.text.chars().collect();
            chars.remove(self.cursor - 1);
            self.edited(chars.into_iter().collect(), self.cursor - 1);
        }
    }

    pub fn delete(&mut self) {
        let mut chars: Vec<char> = self.text.chars().collect();
        if self.cursor < chars.len() {
            chars.remove(self.cursor);
            self.edited(chars.into_iter().collect(), self.cursor);
        }
    }

    /// Where the word before the cursor starts.
    fn word_start(&self) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut at = self.cursor.min(chars.len());
        while at > 0 && chars[at - 1] == ' ' {
            at -= 1;
        }
        while at > 0 && chars[at - 1] != ' ' {
            at -= 1;
        }
        at
    }

    /// Where the word after the cursor ends.
    fn word_end(&self) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut at = self.cursor.min(chars.len());
        while at < chars.len() && chars[at] == ' ' {
            at += 1;
        }
        while at < chars.len() && chars[at] != ' ' {
            at += 1;
        }
        at
    }

    /// Ctrl+Backspace / Ctrl+W.
    pub fn delete_word_back(&mut self) {
        let start = self.word_start();
        let chars: Vec<char> = self.text.chars().collect();
        let text = chars[..start].iter().chain(&chars[self.cursor..]).collect();
        self.edited(text, start);
    }

    /// Ctrl+Delete.
    pub fn delete_word_forward(&mut self) {
        let end = self.word_end();
        let chars: Vec<char> = self.text.chars().collect();
        let text = chars[..self.cursor].iter().chain(&chars[end..]).collect();
        self.edited(text, self.cursor);
    }

    /// Ctrl+U: delete up to the cursor.
    pub fn kill_to_start(&mut self) {
        let rest = self.text[char_index(&self.text, self.cursor)..].to_owned();
        self.edited(rest, 0);
    }

    /// Ctrl+K: delete from the cursor on.
    pub fn kill_to_end(&mut self) {
        let kept = self.text[..char_index(&self.text, self.cursor)].to_owned();
        let cursor = self.cursor;
        self.edited(kept, cursor);
    }

    pub fn left(&mut self) {
        self.accept_search();
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.accept_search();
        self.cursor = (self.cursor + 1).min(self.text.chars().count());
    }

    pub fn word_left(&mut self) {
        self.cursor = self.word_start();
    }

    pub fn word_right(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let mut at = self.cursor;
        while at < chars.len() && chars[at] != ' ' {
            at += 1;
        }
        while at < chars.len() && chars[at] == ' ' {
            at += 1;
        }
        self.cursor = at;
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.text.chars().count();
    }

    /// Up/Down, Tab/Shift+Tab: step through the completion list, putting the item in the line.
    /// An empty line, or Up/Down on a line without matches, steps through the history. `tab`:
    /// a single match is taken with a space after it.
    pub fn step(&mut self, back: bool, tab: bool) {
        if self.search.is_some() {
            if tab {
                self.accept_search();
            }
            return;
        }
        self.dismissed = None;
        let no_matches = self.list.as_ref().is_none_or(|l| l.items.is_empty());
        if self.text.is_empty() || (!tab && no_matches) {
            self.list = Some(List {
                head: String::new(),
                items: self
                    .history
                    .iter()
                    .map(|h| Candidate {
                        text: h.clone(),
                        detail: String::new(),
                    })
                    .collect(),
                index: None,
                shows: self.text.clone(),
            });
        }
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
        self.set(text, cursor);
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
        self.accept_search();
        let line = std::mem::take(&mut self.text);
        self.cursor = 0;
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
        e.refresh(&App::default());
        e.step(true, false);
        assert_eq!(e.text(), "map x");
        let (items, index) = e.popup().unwrap();
        assert_eq!((items.len(), index), (2, Some(0)));
        e.step(false, false);
        assert_eq!(e.text(), "fov 90");
        let mut many = LineEditor::default();
        many.set_history((0..150).map(|i| i.to_string()).collect());
        assert_eq!(
            (many.history().len(), many.history()[0].as_str()),
            (HISTORY_MAX, "50")
        );
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
