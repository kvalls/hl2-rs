//! Keep externally controlled settings in sync without discarding pending dialog edits.
use super::OptionsDialog;

/// Picks one section of plugin settings in a configuration.
pub type ExtensionStore = fn(&mut crate::config::Config) -> &mut crate::config::StoredExtensions;

impl OptionsDialog {
    /// The window's display mode changed outside the dialog (for example macOS native full
    /// screen entered with the green button): the applied state follows it, and so does the
    /// draft unless the user already picked a different mode there.
    pub fn sync_display_mode(&mut self, mode: crate::config::DisplayMode) {
        if self.draft.config.video.display_mode == self.applied.video.display_mode {
            self.draft.config.video.display_mode = mode;
        }
        self.applied.video.display_mode = mode;
    }

    /// A plugin setting with no Options control changed outside the dialog: applying or
    /// cancelling a draft keeps it. `store` picks its section (`|c| &mut c.extensions`,
    /// `|c| &mut c.video.extensions`).
    pub fn sync_extension(&mut self, store: ExtensionStore, key: &str, value: bool) {
        store(&mut self.applied).set_bool(key, value);
        store(&mut self.draft.config).set_bool(key, value);
        if let Some(saved) = &mut self.sub_saved {
            store(saved).set_bool(key, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OptionsDialog;
    use crate::config::Config;
    use crate::gameui::options::OptionsOutcome;

    #[test]
    fn an_external_plugin_setting_survives_options_without_losing_pending_edits() {
        let mut options = OptionsDialog::default();
        options.open(Config::default());
        options.sync_extension(|c| &mut c.video.extensions, "plugin_flag", true);
        assert!(
            !options.changed(),
            "a hidden setting must not create an Options edit"
        );
        options.draft.config.video.gamma = 2.4;
        options.sub_saved = Some(options.draft.config.clone());
        options.sync_extension(|c| &mut c.video.extensions, "plugin_flag", false);
        assert!(options.changed(), "retain the existing gamma edit");
        let saved = options.sub_saved.as_ref().unwrap();
        assert_eq!(saved.video.extensions.bool("plugin_flag"), Some(false));
        assert_eq!(options.draft.config.video.gamma, 2.4);
        assert_eq!(options.applied.video.gamma, Config::default().video.gamma);
        let OptionsOutcome::Apply(config) = options.apply() else {
            panic!("pending edit applies");
        };
        assert_eq!(config.video.extensions.bool("plugin_flag"), Some(false));
        assert_eq!(config.video.gamma, 2.4);
    }

    #[test]
    fn a_general_plugin_setting_survives_applying_options() {
        let mut options = OptionsDialog::default();
        options.open(Config::default());
        options.draft.config.video.gamma = 2.4;
        options.sync_extension(|c| &mut c.extensions, "plugin_flag", true);
        let OptionsOutcome::Apply(config) = options.apply() else {
            panic!("pending edit applies");
        };
        assert_eq!(config.extensions.bool("plugin_flag"), Some(true));
    }
}
