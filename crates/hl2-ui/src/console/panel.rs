//! The developer console as HL2 runs it: the toolkit's console dialog
//! ([`source_ui::console::panel::ConsolePanel`]) with HL2's commands, completion sources, texts
//! and window look; printing, submitting lines and turning the dialog's actions into effects.

use super::{commands, complete, Console, Effect, Window, RELEASE_SOUND};
use source_ui::canvas::Rect;
use source_ui::console::command;
use source_ui::console::panel::{Action, Level, Look};
use source_ui::input::Input;
use source_ui::windows::Layer;

impl Console {
    pub fn log(&mut self, text: impl Into<String>) {
        self.print(Level::Normal, text);
    }

    /// Updates the most recent live viewmodel tuning snapshot instead of growing scrollback.
    pub fn log_viewmodel_tuning(&mut self, text: impl Into<String>) {
        self.shell.panel.replace_snapshot(&self.canvas, text);
    }

    /// Prints a line at `level`, stamped with the current time.
    pub fn print(&mut self, level: Level, text: impl Into<String>) {
        self.shell.panel.print(&self.canvas, level, text);
    }

    pub fn submit(&mut self, line: &str) -> Vec<Effect> {
        let line = line.trim();
        if line.is_empty() {
            return Vec::new();
        }
        self.print(Level::Input, format!("] {line}"));
        let commands = match command::tokenize(line) {
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

    /// Runs one command through its [`commands::Spec`] handler.
    pub(super) fn interpret(&mut self, tokens: &[String]) -> commands::Outcome {
        command::run(self, commands::COMMANDS, tokens)
    }

    /// The current value of an implemented cvar, as the console prints it.
    pub fn cvar_value(&self, name: &str) -> Option<String> {
        commands::find(name)?.value?(self)
    }

    /// The text being typed on the console line.
    pub fn input_line(&self) -> &str {
        self.shell.panel.editor.text()
    }

    /// Names that command arguments complete from (installed maps, the loaded map's entities).
    pub fn set_completion_sources(&mut self, sources: complete::Sources) {
        self.completion = sources;
        self.shell.panel.editor.refresh(&self.completion);
    }

    /// Restores the command history the host saved.
    pub fn set_history(&mut self, history: Vec<String>) {
        self.shell.panel.editor.set_history(history);
    }

    /// The command history, once after it changed (the host saves it then).
    pub fn take_changed_history(&mut self) -> Option<Vec<String>> {
        self.shell
            .panel
            .editor
            .take_history_changed()
            .then(|| self.shell.panel.editor.history().to_vec())
    }

    /// Input of the focused console: the dialog edits, scrolls and selects; a click on Close
    /// closes it and a submitted line runs (both buttons play their release sound).
    pub(super) fn console_input(&mut self, input: &Input) -> Vec<Effect> {
        self.shell.pointer = input.pointer;
        match self
            .shell
            .panel
            .input(&self.canvas, input, &self.completion)
        {
            Action::None => Vec::new(),
            Action::Close => {
                self.toggle();
                vec![Effect::PlaySound(RELEASE_SOUND.into())]
            }
            Action::Submit { line, clicked } => {
                let mut effects = Vec::new();
                if clicked {
                    effects.push(Effect::PlaySound(RELEASE_SOUND.into()));
                }
                effects.extend(self.submit(&line));
                effects
            }
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
        let reverse_search =
            localization.text(crate::gameui::localize::placeholders::CONSOLE_REVERSE_SEARCH);
        let look = Look {
            fonts: &self.widget_fonts,
            theme: &theme,
            popup_theme: &self.theme,
            title: &title,
            submit: &submit,
            reverse_search: &reverse_search,
            focused: self.shell.windows.focus() == Layer::Window(Window::Console),
            closing: self.shell.escape_held && self.shell.windows.top() == Some(Window::Console),
            commands: commands::COMMANDS,
            context: self,
        };
        self.shell.panel.draw(&self.canvas, &look, time, occluders);
    }
}
