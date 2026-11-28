//! Console command history kept across launches (retail keeps it for one run only):
//! `hl2-console-history.txt` next to the settings file, never in the game install. Loaded into
//! the console the first time it is seen and written whenever its saved history changed (chat
//! lines are left out by `Console::take_changed_history`).
use hl2_ui::console::Console;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub(super) struct SavedHistory {
    loaded: bool,
}

impl SavedHistory {
    /// Loads the saved history once, then saves it when it changed. A cheap flag check on
    /// frames without a change.
    pub(super) fn sync(&mut self, console: &mut Console) {
        self.sync_at(console, &path());
    }

    fn sync_at(&mut self, console: &mut Console, path: &Path) {
        if !self.loaded {
            self.loaded = true;
            console.set_history(load(path));
            return;
        }
        if let Some(history) = console.take_changed_history()
            && let Err(error) = std::fs::write(path, history.join("\n"))
        {
            bevy::log::warn!("Could not save the console history: {error}");
        }
    }
}

fn path() -> PathBuf {
    hl2_ui::config::Config::config_path().with_file_name("hl2-console-history.txt")
}

/// The saved command history, oldest first.
fn load(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|text| {
            text.lines()
                .filter(|line| !line.trim().is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_restored_once_and_saved_when_it_changes() {
        let dir = std::env::temp_dir().join(format!("hl2-debug-history-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hl2-console-history.txt");
        std::fs::write(&path, "echo saved\n\nfov 90\n").unwrap();

        let mut console = Console::default();
        let mut history = SavedHistory::default();
        history.sync_at(&mut console, &path);
        assert!(
            console.take_changed_history().is_none(),
            "loading is not a change"
        );
        console.toggle();
        console.input(&source_ui::input::Input {
            text: "echo new".into(),
            keys: [source_ui::input::Key::Enter].into_iter().collect(),
            ..Default::default()
        });
        history.sync_at(&mut console, &path);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "echo saved\nfov 90\necho new"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
