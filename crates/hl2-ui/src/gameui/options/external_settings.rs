//! Keep externally controlled settings in sync without discarding pending dialog edits.
use super::OptionsDialog;

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

    /// This cvar has no Options control; applying/cancelling a draft must preserve it.
    pub(crate) fn sync_background_force_default_blur(&mut self, force: bool) {
        self.applied.video.background_force_default_blur = force;
        self.draft.config.video.background_force_default_blur = force;
        if let Some(saved) = &mut self.sub_saved {
            saved.video.background_force_default_blur = force;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OptionsDialog;
    use crate::config::Config;
    use crate::gameui::options::OptionsOutcome;

    #[test]
    fn external_blur_override_survives_options_without_losing_pending_edits() {
        let mut options = OptionsDialog::default();
        options.open(Config::default());
        options.sync_background_force_default_blur(true);
        assert!(
            !options.changed(),
            "hidden cvar must not create an Options edit"
        );
        options.draft.config.video.gamma = 2.4;
        options.sub_saved = Some(options.draft.config.clone());
        options.sync_background_force_default_blur(false);
        assert!(options.changed(), "retain the existing gamma edit");
        assert!(
            !options
                .sub_saved
                .as_ref()
                .unwrap()
                .video
                .background_force_default_blur
        );
        assert_eq!(options.draft.config.video.gamma, 2.4);
        assert_eq!(options.applied.video.gamma, Config::default().video.gamma);
        let OptionsOutcome::Apply(config) = options.apply() else {
            panic!("pending edit applies");
        };
        assert!(!config.video.background_force_default_blur);
        assert_eq!(config.video.gamma, 2.4);
    }
}
