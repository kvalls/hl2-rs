//! The Options dialog's lifecycle: display modes and recommendations it offers, opening it,
//! its input, applying settings and reverting a video change.

use super::{ConfirmKind, Console, Effect, Mode, Window};
use crate::gameui::display::DisplayModes;
use crate::gameui::options::OptionsOutcome;
use crate::gameui::video_confirm::VideoConfirm;
use crate::gameui::video_confirm::VideoMode;
use glam::vec2;
use glam::Vec2;
use source_ui::input::Input;
use source_ui::shell;
use source_ui::widgets::Painter;

impl Console {
    /// Resolutions for the Video page, from the host's monitor query.
    pub fn set_display_modes(&mut self, modes: DisplayModes) {
        self.options.set_display_modes(modes);
    }

    /// Sets system hardware recommendation profile for video settings.
    pub fn set_system_recommendation(
        &mut self,
        recommendation: crate::gameui::recommendation::SystemRecommendation,
    ) {
        self.options.set_system_recommendation(recommendation);
    }

    /// The detected or configured system hardware recommendation.
    pub fn system_recommendation(&self) -> &crate::gameui::recommendation::SystemRecommendation {
        self.options.system_recommendation()
    }

    /// The Options dialog is open (focused or not).
    pub fn options_open(&self) -> bool {
        self.shell.windows.is_active(Window::Options)
    }

    /// Opens the Options dialog with the applied settings, or raises it as it is when open
    /// (retail activates the existing dialog).
    pub fn open_options(&mut self) {
        if !self.options_open() {
            self.options.open(self.config.clone());
            self.options
                .reset_placement_if_off_screen(vec2(self.canvas.width(), self.canvas.height()));
        }
        if !self.background_level && self.base == Mode::Gameplay {
            self.base = Mode::Pause;
        }
        self.shell.windows.open(Window::Options);
        self.reset_menu_interaction();
        self.sync_mode();
    }

    /// The `mat_monitorgamma` to display: the brightness dialog's live value while it is open,
    /// otherwise the applied setting.
    pub fn display_gamma(&self) -> f32 {
        let preview = self
            .options_open()
            .then(|| self.options.preview_gamma())
            .flatten();
        preview.unwrap_or(self.config.video.gamma)
    }

    /// The window's display mode changed outside the game's settings (macOS native full screen):
    /// the applied configuration and an open Options dialog follow it.
    pub fn set_display_mode_external(&mut self, mode: crate::config::DisplayMode) {
        self.config.video.display_mode = mode;
        self.options.sync_display_mode(mode);
    }

    /// Escape closes the quit box first, then leaves a menu category, then toggles the menus.
    /// The main menu over a background level has nothing to return to.
    /// The Options dialog's state for run reports while it is open.
    pub fn options_report(&self) -> Option<crate::gameui::options::OptionsReport> {
        self.options_open().then(|| {
            let bounds = self.options.frame_bounds(self.canvas_size());
            crate::gameui::options::OptionsReport {
                frame: Some([bounds.x, bounds.y, bounds.w, bounds.h]),
                ..self.options.report()
            }
        })
    }

    /// Puts back the video mode an unconfirmed change replaced, and applies it.
    pub(super) fn revert_video(&mut self, previous: VideoMode) -> Effect {
        previous.restore(&mut self.config.video);
        if let Err(error) = self.config.save() {
            self.log(format!("Could not save settings: {error}"));
        }
        self.options.reset_applied(self.config.clone());
        self.log(format!(
            "Video mode reverted to {}x{}",
            previous.width, previous.height
        ));
        Effect::ApplyConfig(Box::new(self.config.clone()))
    }

    pub(super) fn options_input(&mut self, input: &Input) -> Vec<Effect> {
        let measure = Painter {
            canvas: &self.canvas,
            fonts: &self.widget_fonts,
            theme: &self.theme,
            origin: Vec2::ZERO,
            scale: 1.,
        };
        let outcome = self.options.input(input, &measure);
        for line in self.options.take_log() {
            self.log(line);
        }
        let mut effects: Vec<Effect> = self
            .options
            .take_sounds()
            .into_iter()
            .map(Effect::PlaySound)
            .collect();
        let (config, close) = match outcome {
            OptionsOutcome::None => return effects,
            OptionsOutcome::Apply(config) => (Some(config), false),
            OptionsOutcome::Close(config) => (config, true),
        };
        if close {
            shell::close_window(self, Window::Options);
        }
        let Some(config) = config else {
            return effects;
        };
        effects.extend(self.apply_settings(config));
        effects
    }

    /// Saves and applies settings from the Options dialog. An aspect ratio change opens the
    /// keep-or-revert confirmation ([`VideoConfirm`]).
    pub(super) fn apply_settings(&mut self, config: crate::config::Config) -> Vec<Effect> {
        let mut effects = Vec::new();
        // The dialog's copy of the settings predates any drag made while it was open.
        let mut config = config;
        config.dialogs = self.config.dialogs;
        if let Err(error) = config.save() {
            self.log(format!("Could not save settings: {error}"));
        }
        let confirm = VideoConfirm::needed(&self.config.video, &config.video)
            .then(|| VideoConfirm::new(VideoMode::of(&self.config.video), self.ui_clock));
        self.config = config.clone();
        effects.push(Effect::ApplyConfig(Box::new(config)));
        if let Some(keep) = confirm {
            self.reset_menu_interaction();
            self.quit_placement.reset();
            let message = keep.message_box(&self.gameui.localization, self.ui_clock);
            self.confirm_box = Some((message, ConfirmKind::KeepVideo(keep)));
        }
        effects
    }
}
