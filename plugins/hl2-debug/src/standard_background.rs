//! `background_force_default_blur`: the menu background keeps its standard (4:3) blur artwork
//! and field of view on wide frames. Saved with the plugin values of the video settings; core
//! asks through the rule this plugin registers (`Extensions::select_standard_background`).

use hl2_ui::config::VideoSettings;

/// The saved value's name, also the console command's.
pub const SETTING: &str = "background_force_default_blur";

/// The registered rule: whether the standard background is forced.
pub fn forced(video: &VideoSettings) -> bool {
    video.extensions.bool(SETTING).unwrap_or(false)
}

/// Saves the choice, keeps an open Options dialog from undoing it, and asks the host to apply
/// the settings.
pub fn set_forced(console: &mut hl2_ui::console::Console, on: bool) -> hl2_ui::console::Effect {
    console.config.video.extensions.set_bool(SETTING, on);
    console
        .options
        .sync_extension(|config| &mut config.video.extensions, SETTING, on);
    let _ = console.config.save();
    hl2_ui::console::Effect::ApplyConfig(Box::new(console.config.clone()))
}

#[cfg(test)]
mod tests {
    use super::{SETTING, forced};

    #[test]
    fn older_files_keep_their_saved_choice() {
        let config = hl2_ui::config::Config::from_json(
            br#"{"video":{"width":1280,"height":720,"background_force_default_blur":true}}"#,
        )
        .expect("load");
        assert!(forced(&config.video));
        let plain = hl2_ui::config::Config::from_json(br#"{"video":{}}"#).expect("load");
        assert!(!forced(&plain.video));
        let saved = serde_json::to_value(&config).expect("save");
        assert_eq!(saved["video"][SETTING], true);
    }
}
