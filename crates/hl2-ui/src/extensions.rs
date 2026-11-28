//! Per-application extension contributions. Core owns contracts, callers supply behavior.
use crate::console::{
    commands::{Spec, COMMANDS},
    Console,
};
use std::sync::Arc;
mod confirmation;
mod framing;
mod video;
pub use confirmation::{Confirmation, ConfirmationHandler};
pub use framing::{FramingEdits, FramingProfiles, ViewmodelFraming, STANDARD_PROFILE};
pub use video::{RenderBudget, VideoExtension, VideoPolicy, RETAIL_VIEWMODEL_FOV};

/// A contributor's rule: the menu background keeps its standard (4:3) presentation (blur
/// artwork and field of view) on wide frames when it returns true for the video settings.
#[derive(Clone, Copy, Debug, Default)]
pub struct StandardBackground(pub Option<fn(&crate::config::VideoSettings) -> bool>);

/// The rule is fixed for a run (installed once), so settings compare by whether one is set:
/// function pointer addresses are not reliable identities.
impl PartialEq for StandardBackground {
    fn eq(&self, other: &Self) -> bool {
        self.0.is_some() == other.0.is_some()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Presentation {
    pub capture_menu_navigation: bool,
    pub background_opacity: Option<u8>,
}

/// Consumes a line entered through the console editor before command parsing.
pub type LocalSubmit = fn(&mut Console, &str) -> bool;

/// Handles custom console panel keyboard shortcuts before retail editing runs.
pub type ConsoleInputHook =
    fn(&mut source_ui::console::panel::ConsolePanel, &source_ui::input::Input) -> bool;

#[derive(Clone, Debug)]
pub struct Extensions {
    pub commands: Arc<[Spec]>,
    pub initialize: Vec<fn(&mut Console)>,
    pub video: VideoPolicy,
    video_owner: Option<&'static str>,
    pub layouts: Vec<(&'static str, &'static str)>,
    pub localization: Vec<(&'static str, &'static str)>,
    pub bindings: Vec<(&'static str, &'static str)>,
    pub after_apply: Vec<fn(&mut Console, &crate::config::Config)>,
    /// Commands that are not logged when they arrive over the remote console (`sv_rcon_log`),
    /// such as a contributor's status updates; `say` is never logged there either.
    pub quiet_remote: Vec<&'static str>,
    local_submit: Option<(&'static str, LocalSubmit)>,
    console_input: Option<(&'static str, ConsoleInputHook, Option<&'static str>)>,
    pub standard_background: StandardBackground,
}
impl Default for Extensions {
    fn default() -> Self {
        Self {
            commands: COMMANDS.into(),
            initialize: Vec::new(),
            video: VideoPolicy::default(),
            video_owner: None,
            layouts: Vec::new(),
            localization: Vec::new(),
            bindings: Vec::new(),
            after_apply: Vec::new(),
            quiet_remote: Vec::new(),
            local_submit: None,
            console_input: None,
            standard_background: StandardBackground::default(),
        }
    }
}
impl Extensions {
    /// Selects the one video policy; a second contributor is an error, never a silent replacement.
    pub fn select_video(&mut self, owner: &'static str, policy: VideoPolicy) -> Result<(), String> {
        if let Some(existing) = self.video_owner {
            return Err(format!(
                "video policy already selected by {existing}: {owner}"
            ));
        }
        self.video_owner = Some(owner);
        self.video = policy;
        Ok(())
    }
    /// Sets the standard-background rule; a second contributor is an error.
    pub fn select_standard_background(
        &mut self,
        rule: fn(&crate::config::VideoSettings) -> bool,
    ) -> Result<(), String> {
        if self.standard_background.0.is_some() {
            return Err("standard background rule already selected".into());
        }
        self.standard_background = StandardBackground(Some(rule));
        Ok(())
    }
    /// One owner can replace or clear its local submission handler.
    pub fn set_local_submit(
        &mut self,
        owner: &'static str,
        handler: Option<LocalSubmit>,
    ) -> Result<(), String> {
        if let Some((existing, _)) = self.local_submit.filter(|(active, _)| *active != owner) {
            return Err(format!("local submission already handled by {existing}"));
        }
        self.local_submit = handler.map(|handler| (owner, handler));
        Ok(())
    }

    pub fn has_local_submit(&self, owner: &str) -> bool {
        self.local_submit.is_some_and(|(active, _)| active == owner)
    }

    pub(crate) fn local_submit(&self) -> Option<LocalSubmit> {
        self.local_submit.map(|(_, handler)| handler)
    }

    /// Selects a contributor's extra console input shortcut hook (and optional localization token
    /// for its prompt); a second contributor is an error, never a silent replacement.
    pub fn select_console_input(
        &mut self,
        owner: &'static str,
        hook: ConsoleInputHook,
        search_prompt_token: Option<&'static str>,
    ) -> Result<(), String> {
        if let Some((existing, _, _)) = self.console_input.filter(|(active, _, _)| *active != owner)
        {
            return Err(format!("console input already handled by {existing}"));
        }
        self.console_input = Some((owner, hook, search_prompt_token));
        Ok(())
    }

    pub(crate) fn console_input(&self) -> Option<ConsoleInputHook> {
        self.console_input.map(|(_, hook, _)| hook)
    }

    pub(crate) fn console_search_prompt(&self) -> Option<&'static str> {
        self.console_input.and_then(|(_, _, token)| token)
    }

    /// Reject collisions so removing a contributor never changes another command's meaning.
    pub fn commands(&mut self, entries: &[Spec]) -> Result<(), String> {
        let mut commands = self.commands.to_vec();
        for entry in entries {
            if commands
                .iter()
                .any(|existing| existing.name.eq_ignore_ascii_case(entry.name))
            {
                return Err(format!("duplicate command: {}", entry.name));
            }
            commands.push(*entry);
        }
        self.commands = commands.into();
        Ok(())
    }
    pub fn prepare_config(&self, config: &mut crate::config::Config) {
        config.video.policy = self.video.clone();
        config.video.standard_background = self.standard_background;
        config.video.advanced.policy = self.video.clone();
        for &(command, key) in &self.bindings {
            if !config.keys.contains_key(command) && !config.keys.values().any(|bound| bound == key)
            {
                config.keys.insert(command.into(), key.into());
            }
        }
    }
    pub fn install(&self, console: &mut Console) {
        console.extensions = self.clone();
        self.prepare_config(&mut console.config);
        let mut resources = console.resources().clone();
        self.prepare_resources(&mut resources);
        console.set_resources(resources);
        console.set_commands(self.commands.clone());
        for initialize in &self.initialize {
            initialize(console);
        }
    }
    pub fn prepare_resources(&self, resources: &mut crate::gameui::resources::GameUiResources) {
        for &(token, value) in &self.localization {
            resources.localization.insert(token, value);
        }
        for &(name, text) in &self.layouts {
            let extra = source_ui::layout::Layout::parse(text.as_bytes())
                .expect("registered layout parses");
            if let Some(layout) = resources.layouts.get_mut(name) {
                for control in extra.controls {
                    if !layout.controls.iter().any(|old| old.name == control.name) {
                        layout.controls.push(control);
                    }
                }
            }
        }
        for &(command, key) in &self.bindings {
            resources
                .default_bindings
                .entry(command.into())
                .or_insert_with(|| key.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Extensions;
    use crate::console::{commands::Spec, Console};
    use source_ui::console::command::{Arg, Outcome};

    fn probe(console: &mut Console, _: &Spec, _: &[String]) -> Outcome<crate::console::Effect> {
        console.log("extension probe executed");
        Ok(None)
    }
    const PROBE: Spec = Spec {
        name: "extension_probe",
        help: "extension_probe: exercise an injected command",
        args: &[Arg::Free],
        run: probe,
        value: None,
        setting: None,
    };

    #[test]
    fn local_submission_handler_is_owned_and_keeps_programmatic_commands_separate() {
        let mut console = Console::default();
        let handler = |console: &mut Console, line: &str| {
            console.say_local(line);
            true
        };
        console
            .extensions
            .set_local_submit("test", Some(handler))
            .unwrap();
        assert!(console
            .extensions
            .set_local_submit("another", None)
            .is_err());
        console.submit_local("echo literal");
        assert_eq!(
            console.shell.panel.output.back().unwrap().text,
            "Player: echo literal"
        );
        console.submit("echo programmatic");
        assert_eq!(
            console.shell.panel.output.back().unwrap().text,
            "programmatic"
        );
        console.extensions.set_local_submit("test", None).unwrap();
        console.submit_local("echo command");
        assert_eq!(console.shell.panel.output.back().unwrap().text, "command");
    }

    #[test]
    fn command_registry_is_injected_per_console_and_rejects_collisions() {
        let mut extensions = Extensions::default();
        extensions.commands(&[PROBE]).unwrap();
        assert!(extensions.commands(&[PROBE]).is_err());
        let mut extended = Console::default();
        let plain = Console::default();
        extensions.install(&mut extended);
        assert!(extended
            .commands()
            .iter()
            .any(|entry| entry.name == PROBE.name));
        assert!(!plain
            .commands()
            .iter()
            .any(|entry| entry.name == PROBE.name));
        extended.submit(PROBE.name);
        assert!(extended
            .shell
            .panel
            .output
            .iter()
            .any(|line| line.text.contains("extension probe executed")));
    }

    #[test]
    fn console_input_hook_is_single_owner_and_isolated_per_extensions() {
        use source_ui::input::{Input, Key};

        fn custom_hook(panel: &mut source_ui::console::panel::ConsolePanel, input: &Input) -> bool {
            if input.ctrl && input.pressed(Key::Backspace) {
                panel.editor.kill_to_start();
                return true;
            }
            false
        }

        let mut extensions = Extensions::default();
        extensions
            .select_console_input("fake", custom_hook, Some("#FakeSearch"))
            .unwrap();
        assert!(extensions
            .select_console_input("other", custom_hook, None)
            .is_err());

        let mut extended = Console::default();
        extensions.install(&mut extended);
        assert_eq!(
            extended.extensions.console_search_prompt(),
            Some("#FakeSearch")
        );
        extended.toggle();
        extended.input(&Input {
            text: "echo hi".into(),
            ..Default::default()
        });
        extended.input(&Input {
            keys: [Key::Backspace].into(),
            ctrl: true,
            ..Default::default()
        });
        assert_eq!(extended.input_line(), "");

        // Core console without contributor ignores Ctrl+Backspace.
        let mut plain = Console::default();
        assert_eq!(plain.extensions.console_search_prompt(), None);
        plain.toggle();
        plain.input(&Input {
            text: "echo hi".into(),
            ..Default::default()
        });
        plain.input(&Input {
            keys: [Key::Backspace].into(),
            ctrl: true,
            ..Default::default()
        });
        assert_eq!(plain.input_line(), "echo hi");
    }
}
