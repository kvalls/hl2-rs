//! Chapter window lifecycle and typed gameplay map requests.
use super::{ConfirmKind, Console, Effect, GameMap, Mode, Window};
use crate::gameui::{
    new_game::{Dialog, Outcome},
    resources::GameUiResources,
};
use glam::Vec2;
use source_ui::{input::Input, shell, widgets::Painter};

impl Console {
    pub(super) fn prepare_chapters(&mut self, resources: &GameUiResources) {
        self.chapter_dialogs = resources
            .new_games
            .iter()
            .map(|(game, catalog)| {
                let mut dialog = Dialog::new(game, catalog, &resources.localization);
                dialog.browser.prepare_text(&self.painter(Vec2::ZERO, 1.));
                (game.clone(), dialog)
            })
            .collect();
    }

    pub(super) fn open_chapters(&mut self, game: &str) {
        if self.is_background_loading() {
            return;
        }
        let Some(dialog) = self.chapter_dialogs.get_mut(game) else {
            return;
        };
        dialog.browser.open();
        self.chapter_game = Some(game.to_owned());
        self.set_mode(Mode::NewGame);
        self.clear_menu_hover();
    }

    pub(super) fn chapter_dialog(&self) -> Option<&Dialog> {
        self.chapter_dialogs.get(self.chapter_game.as_ref()?)
    }

    pub(super) fn chapter_dialog_mut(&mut self) -> Option<&mut Dialog> {
        self.chapter_dialogs.get_mut(self.chapter_game.as_ref()?)
    }

    pub fn chapters_report(&self) -> Option<serde_json::Value> {
        self.shell
            .windows
            .is_active(Window::NewGame)
            .then(|| self.chapter_dialog().map(|d| d.report(self.canvas_size())))
            .flatten()
    }

    pub(super) fn start_chapter(&mut self, target: GameMap, commentary: bool) -> Vec<Effect> {
        let mut effects = Vec::new();
        if self.config.gameplay.commentary != commentary || commentary {
            self.config.gameplay.commentary = commentary;
            let _ = self.config.save();
            effects.push(Effect::ApplyConfig(Box::new(self.config.clone())));
        }
        self.cheats = commentary;
        self.shell.windows.clear();
        self.sync_mode();
        self.start_loading_for_game(&target.game, &target.map);
        effects.push(Effect::ChapterMap(target));
        effects
    }

    pub(super) fn chapters_input(&mut self, input: &Input) -> Vec<Effect> {
        let canvas = self.canvas_size();
        let Some(dialog) = self.chapter_dialog_mut() else {
            return Vec::new();
        };
        let outcome = dialog.input(input, canvas);
        let commentary = dialog.commentary_enabled();
        let mut effects: Vec<_> = dialog
            .browser
            .take_sounds()
            .into_iter()
            .map(Effect::PlaySound)
            .collect();
        match outcome {
            Outcome::None => {}
            Outcome::Close => shell::close_window(self, Window::NewGame),
            Outcome::Start(target) => {
                if commentary {
                    let loc = &self.gameui.localization;
                    let measure = self.painter(Vec2::ZERO, 1.);
                    let msg = crate::gameui::menu::commentary_explanation_box(loc, |s| {
                        measure.text_width(s, false)
                    });
                    self.reset_menu_interaction();
                    self.quit_placement.reset();
                    self.confirm_box = Some((msg, ConfirmKind::StartCommentary(target)));
                    self.sync_dialog_windows();
                } else {
                    effects.extend(self.start_chapter(target, false));
                }
            }
        }
        effects
    }

    pub(super) fn draw_chapters(&self, time: f64, occluders: &[source_ui::canvas::Rect]) {
        let Some(dialog) = self.chapter_dialog() else {
            return;
        };
        let theme = self.window_theme(Window::NewGame, time);
        let alpha = self
            .shell
            .windows
            .window_composite_alpha(Window::NewGame, time);
        let old = self.canvas.set_alpha_multiplier(alpha);
        for &rect in occluders {
            self.canvas.push_occluder(rect);
        }
        dialog.browser.draw(
            &Painter {
                theme: &theme,
                ..self.painter(Vec2::ZERO, 1.)
            },
            self.canvas_size(),
        );
        for _ in occluders {
            self.canvas.pop_occluder();
        }
        self.canvas.set_alpha_multiplier(old);
    }
}
