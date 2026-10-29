//! Map loading: the gameplay loading dialog and the background-map transition.

use super::{Console, Mode};
use source_ui::input::Input;

impl Console {
    /// Displays the modal loading dialog for `game`'s `map` over that chapter's pre-blurred
    /// background image.
    pub fn start_loading_for_game(&mut self, game: &str, map: &str) {
        self.clear_menu_hover();
        self.background_transition.reset();
        self.loading_background = self.chapter_background_for(game, map);
        self.loading_dialog = Some(crate::gameui::loading::LoadingDialog::new(map));
        self.loading_progress.reset();
    }

    /// Displays the modal loading dialog for the given map.
    pub fn start_loading(&mut self, map: &str) {
        let current = self.loaded_map.as_ref().map(|m| m.game.as_str());
        let target = self.game_map(map, current);
        self.start_loading_for_game(&target.game, &target.map);
    }

    /// Starts the background transition to `game`'s `map` and shows the corner loading plaque.
    /// The transition's category is the game, whose subtitle shows once the map has loaded.
    pub fn start_background_loading(&mut self, game: &str, map: &str) {
        self.clear_menu_hover();
        self.occlude_background_controls(&Input::default());
        self.background_transition.start(map, game);
        self.loading_background = None;
        self.loading_dialog = Some(crate::gameui::loading::LoadingDialog::new_background(map));
        self.loading_progress.reset();
        self.base = Mode::MainMenu;
        self.sync_mode();
    }

    /// Draws the full-screen gameplay loading screen (pre-blurred chapter background texture and
    /// modal [`LoadingDialog`], hiding menu titles, buttons, and windows) when a gameplay map load
    /// is active or when in [`Mode::Gameplay`]. Returns `true` if `Console::draw` is complete.
    pub(super) fn draw_loading_screen(&self) -> bool {
        if let Some(loading) = self
            .loading_dialog
            .as_ref()
            .filter(|l| !l.is_background() && !l.is_failed())
        {
            let _ = loading;
            let is_widescreen =
                self.canvas.width() / self.canvas.height().max(1.0) > 4. / 3. + 1e-3;
            let bg = self
                .loading_background
                .as_ref()
                .or(self.loaded_map.as_ref());
            if let Some(tex) =
                bg.and_then(|m| self.background_texture(&m.game, &m.map, is_widescreen))
            {
                crate::gameui::loading::draw_background_texture(&self.canvas, tex);
            }
            self.draw_loading();
            return true;
        }
        if self.mode == Mode::Gameplay && !self.is_background_loading() {
            self.draw_loading();
            return true;
        }
        false
    }

    /// Hidden controls consume physical input so its release cannot activate restored UI.
    pub(super) fn occlude_background_controls(&mut self, input: &Input) {
        self.menu_tracker.occlude(input.click || input.held);
        self.shell.key_repeat.cancel();
        self.options.occlude(input);
        if let Some(dialog) = self.chapter_dialog_mut() {
            dialog.browser.occlude(input.click || input.held);
        }
    }

    /// Updates loading progress from the latest background loader report.
    pub fn update_loading(&mut self) {
        if self.background_transition.is_active() {
            self.background_transition.update(1.0 / 60.0);
        }
        if let Some(loading) = &mut self.loading_dialog {
            loading.update(self.loading_progress.current());
        }
    }

    /// Ends the background transition due to a failure and dismisses the loading plaque.
    pub fn background_load_failed(&mut self, reason: &str) {
        self.background_transition.fail(reason);
        if let Some(loading) = &mut self.loading_dialog {
            loading.set_error(reason);
        }
        self.loading_dialog = None;
        self.loading_progress.reset();
    }

    /// Ends the background transition if `game`'s `map` is the target, ignoring stale failures.
    pub fn background_load_failed_for_map(&mut self, game: &str, map: &str, reason: &str) {
        if !self.background_transition.fail_map(game, map, reason) {
            return;
        }
        self.loading_dialog = None;
        self.loading_progress.reset();
    }

    /// Marks the active loading dialog as failed with the given error message.
    pub fn loading_failed(&mut self, error: &str) {
        if let Some(loading) = &mut self.loading_dialog {
            loading.set_error(error);
            if loading.is_background() {
                self.background_transition.fail(error);
                self.loading_dialog = None;
                self.loading_progress.reset();
            }
        } else if self.background_transition.is_active() {
            self.background_transition.fail(error);
        }
    }

    /// Clears the loading dialog once the level has completely loaded.
    pub fn loading_complete(&mut self) {
        if let Some(loading) = &mut self.loading_dialog {
            if loading.is_background() {
                self.background_transition.finish();
            }
            loading.set_complete();
        } else if self.background_transition.is_active() {
            self.background_transition.finish();
        }
        self.loading_dialog = None;
        self.loading_background = None;
        self.clear_menu_hover();
        self.loading_progress.reset();
    }

    /// The report the map loader fills for this console's loading dialog (shared, cheap to clone).
    pub fn loading_progress(&self) -> crate::gameui::loading::ProgressReport {
        self.loading_progress.clone()
    }

    /// Whether a map load is currently in progress.
    pub fn is_loading(&self) -> bool {
        self.loading_dialog.as_ref().is_some_and(|l| !l.is_failed())
    }

    /// Whether background map loading is currently active.
    pub fn is_background_loading(&self) -> bool {
        self.background_transition.is_active()
            || self
                .loading_dialog
                .as_ref()
                .is_some_and(|l| l.is_background() && !l.is_failed() && !l.is_complete())
    }

    /// Returns the current background transition state.
    pub fn background_transition_state(
        &self,
    ) -> &crate::gameui::loading::BackgroundTransitionState {
        &self.background_transition
    }
}
