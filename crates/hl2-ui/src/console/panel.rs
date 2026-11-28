//! The developer console as HL2 runs it: the toolkit's console dialog
//! ([`source_ui::console::panel::ConsolePanel`]) with HL2's commands, completion sources, texts
//! and window look; printing, submitting lines and turning the dialog's actions into effects.

use super::{commands, complete, Console, Effect, Window};
use source_ui::canvas::Rect;
use source_ui::console::command;
use source_ui::console::panel::{Action, Level, Look};
use source_ui::input::Input;
use source_ui::windows::Layer;

impl Console {
    pub fn log(&mut self, text: impl Into<String>) {
        self.print(Level::Normal, text);
    }

    /// Updates the most recent live snapshot instead of growing scrollback.
    pub fn log_snapshot(&mut self, text: impl Into<String>) {
        self.log_snapshot_with_level(Level::Snapshot, text);
    }

    /// Updates the most recent live snapshot with an explicit display level.
    pub fn log_snapshot_with_level(&mut self, level: Level, text: impl Into<String>) {
        if self.remote.from_rcon {
            self.remote.snapshot = Some(Some((level, text.into())));
        } else {
            self.shell
                .panel
                .replace_snapshot_with_level(&self.canvas, level, text);
        }
    }

    /// Removes the current live snapshot line, if any.
    pub fn clear_snapshot(&mut self) {
        if self.remote.from_rcon {
            self.remote.snapshot = Some(None);
        } else {
            self.shell.panel.clear_snapshot(&self.canvas);
        }
    }

    /// Prints a line at `level`, stamped with the current time.
    pub fn print(&mut self, level: Level, text: impl Into<String>) {
        self.shell.panel.print(&self.canvas, level, text);
    }

    /// Submits text entered through the local console editor.
    pub fn submit_local(&mut self, line: &str) -> Vec<Effect> {
        let line = line.trim();
        if line.is_empty() {
            return Vec::new();
        }
        if self
            .extensions
            .local_submit()
            .is_some_and(|route| route(self, line))
        {
            self.keep_for_session(line, true);
            return Vec::new();
        }
        let commands = command::tokenize(line);
        self.keep_for_session(line, super::remote::chat_only(&commands));
        self.submit_parsed(line, commands)
    }

    /// Marks a submitted history line as chat (kept only for this run) or as a command (saved).
    fn keep_for_session(&mut self, line: &str, chat: bool) {
        if chat {
            self.session_only_history.insert(line.to_owned());
        } else {
            self.session_only_history.remove(line);
        }
    }

    pub fn say_local(&mut self, text: &str) {
        let speaker = super::remote::PLAYER_SPEAKER;
        self.print(Level::ChatLocal, format!("{speaker}: {text}"));
    }

    pub fn submit(&mut self, line: &str) -> Vec<Effect> {
        let line = line.trim();
        if line.is_empty() {
            return Vec::new();
        }
        self.submit_parsed(line, command::tokenize(line))
    }

    /// Echoes a tokenized line (chat prints its own line) and runs it.
    fn submit_parsed(
        &mut self,
        line: &str,
        commands: Result<Vec<Vec<String>>, String>,
    ) -> Vec<Effect> {
        if !super::remote::chat_only(&commands) {
            self.print(Level::Input, format!("] {line}"));
        }
        self.execute_parsed(commands)
    }

    /// Runs a command line without echoing it (the command line's `+` commands, the remote
    /// console).
    pub fn execute(&mut self, line: &str) -> Vec<Effect> {
        self.execute_parsed(command::tokenize(line))
    }

    /// Runs a line already parsed for console echo or remote logging.
    pub fn execute_parsed(&mut self, commands: Result<Vec<Vec<String>>, String>) -> Vec<Effect> {
        let commands = match commands {
            Ok(commands) => commands,
            Err(error) => {
                self.log(error);
                return Vec::new();
            }
        };
        let mut effects = Vec::new();
        for tokens in commands {
            match self.interpret(&tokens) {
                Ok(Some(effect)) => effects.push(effect),
                Ok(None) => (),
                Err(error) => self.log(error),
            }
        }
        effects
    }

    /// Queues the command line's `+` commands: like Source's command buffer they run after the
    /// startup level loaded, so menus they open stay open.
    pub fn queue_startup_commands(&mut self, commands: Vec<String>) {
        self.startup_commands = commands;
    }

    /// Startup commands are still waiting for the first level.
    pub fn startup_pending(&self) -> bool {
        !self.startup_commands.is_empty()
    }

    /// An effect for the host with the next tick (from code outside a console command).
    pub fn defer(&mut self, effect: Effect) {
        self.deferred.push(effect);
    }

    /// Runs the queued startup commands; what they ask of the host comes with the next tick.
    pub(super) fn run_startup_commands(&mut self) {
        for line in std::mem::take(&mut self.startup_commands) {
            let effects = self.execute(&line);
            self.deferred.extend(effects);
        }
    }

    /// Runs one command through its [`commands::Spec`] handler.
    pub(super) fn interpret(&mut self, tokens: &[String]) -> commands::Outcome {
        {
            let entries = self.completion.commands.clone();
            let entries = if entries.is_empty() {
                commands::COMMANDS
            } else {
                &entries
            };
            command::run(self, entries, tokens)
        }
    }

    /// The current value of an implemented cvar, as the console prints it.
    pub fn cvar_value(&self, name: &str) -> Option<String> {
        command::find(self.commands(), name)?.value?(self)
    }

    /// The text being typed on the console line.
    pub fn input_line(&self) -> &str {
        self.shell.panel.editor.text()
    }

    /// Names that command arguments complete from (installed maps, the loaded map's entities).
    pub fn set_completion_sources(&mut self, sources: complete::Sources) {
        self.completion.sources = sources;
        self.shell.panel.editor.refresh(&self.completion);
    }

    /// Restores a command history saved by an earlier run (retail keeps it for one run only).
    pub fn set_history(&mut self, history: Vec<String>) {
        self.shell.panel.editor.set_history(history);
    }

    /// The command history to save, once after it changed, for a contributor that keeps it
    /// across runs. Chat lines stay in the editor's history for this run but are not saved.
    pub fn take_changed_history(&mut self) -> Option<Vec<String>> {
        self.shell.panel.editor.take_history_changed().then(|| {
            let editor = &self.shell.panel.editor;
            editor
                .history()
                .iter()
                .filter(|line| !self.session_only_history.contains(*line))
                .cloned()
                .collect()
        })
    }

    /// Input of the focused console: the dialog edits, scrolls and selects; a click on Close
    /// closes it and a submitted line runs. Like retail vgui Buttons without sound_* keys in
    /// their resource file, neither button plays a sound (only GameMenu items do).
    pub(super) fn console_input(&mut self, input: &Input) -> Vec<Effect> {
        self.shell.pointer = input.pointer;
        let hook = self.extensions.console_input();
        match self
            .shell
            .panel
            .input_with(&self.canvas, input, &self.completion, |panel, frame| {
                hook.is_some_and(|h| h(panel, frame))
            }) {
            Action::None => Vec::new(),
            Action::Close => {
                self.toggle_console_window();
                Vec::new()
            }
            Action::Submit { line, .. } => self.submit_local(&line),
        }
    }

    /// Text the copy shortcut copied from the history, for the host to put on the clipboard.
    pub fn take_copied(&mut self) -> Option<String> {
        self.shell.panel.take_copied()
    }

    /// The console dialog with HL2's texts, the console window's look and focus, under
    /// `occluders` (windows above it).
    pub(super) fn draw_console_with_occluders(&self, time: f64, occluders: &[Rect]) {
        let theme = self.window_theme(Window::Console, time);
        let localization = &self.gameui.localization;
        let title = localization.text("#Console_Title");
        let submit = localization.text("#Console_Submit");
        let reverse_search = self
            .extensions
            .console_search_prompt()
            .map(|token| localization.text(token))
            .unwrap_or_default();
        let look = Look {
            fonts: &self.widget_fonts,
            theme: &theme,
            popup_theme: &self.theme,
            title: &title,
            submit: &submit,
            reverse_search: &reverse_search,
            focused: self.shell.windows.focus() == Layer::Window(Window::Console),
            closing: self.shell.escape_held && self.shell.windows.top() == Some(Window::Console),
            commands: self.commands(),
            context: self,
        };
        self.shell.panel.draw(&self.canvas, &look, time, occluders);
    }
}
