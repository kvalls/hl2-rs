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
pub use video::{VideoExtension, VideoPolicy, RETAIL_VIEWMODEL_FOV};

#[derive(Clone, Debug, Default)]
pub struct Presentation {
    pub capture_menu_navigation: bool,
    pub background_opacity: Option<u8>,
}

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
}
