//! Menu flow: the main menu and its categories, titles and backgrounds, chapter backgrounds
//! after a level, and the quit / return-to-menu box.

use super::{ConfirmKind, Console, Effect, GameMap, Mode};
use crate::gameui::menu::GameMenu;
use crate::gameui::menu::MenuOutcome;
use glam::vec2;
use glam::Vec2;
use source_ui::canvas::Rect;
use source_ui::canvas::Texture2D;
use source_ui::input::Input;
use source_ui::input::Key;
use source_ui::message_box::MessageBox;
use source_ui::message_box::MessageBoxLayout;
use source_ui::placement;

impl Console {
    /// The pre-blurred image of `game`'s background map `map`, if loaded.
    pub fn background_texture(
        &self,
        game: &str,
        map: &str,
        widescreen: bool,
    ) -> Option<&Texture2D> {
        let key = |name: String| (game.to_owned(), name);
        let map = crate::gameui::resources::blur(map);
        widescreen
            .then(|| {
                self.background_textures
                    .get(&key(format!("{map}_widescreen")))
            })
            .flatten()
            .or_else(|| self.background_textures.get(&key(map.to_owned())))
    }

    /// Checks whether the primary title font contains glyphs for all non-whitespace characters in `line`.
    pub(crate) fn title_font_can_render(&self, line: &str) -> bool {
        if let Some(face) = &self.title_face {
            !line.is_empty()
                && line
                    .chars()
                    .all(|c| c.is_whitespace() || face.lookup_glyph_index(c) != 0)
        } else {
            false
        }
    }

    /// Returns the main menu title for the given category.
    pub fn category_title(&self, category: &str) -> &str {
        let canonical = match category {
            "hl2ep1" => "episodic",
            "hl2ep2" => "ep2",
            "hl2lc" => "lostcoast",
            other => other,
        };
        self.category_titles
            .get(canonical)
            .or_else(|| self.category_titles.get(category))
            .map(String::as_str)
            .unwrap_or(&self.title)
    }

    /// The active menu title for the current mode and selected category.
    pub fn current_title(&self) -> &str {
        let menu_cat = if self.base == Mode::Pause {
            self.pause_menu.category()
        } else {
            self.main_menu.category()
        };
        let category = self.background_transition.displayed_category(menu_cat);
        self.category_title(category)
    }

    /// Pure calculation of the background dim overlay opacity based on context and base focus amount.
    pub fn dim_overlay_opacity(is_mid_gameplay: bool, menu_open: bool, base_focus: f32) -> f32 {
        if !menu_open {
            return 0.0;
        }
        let backdrop = crate::tuning::backdrop_alpha();
        let gameplay_dim = crate::tuning::gameplay_dim_alpha();
        let unfocused = (1.0 - base_focus).clamp(0.0, 1.0);

        if is_mid_gameplay {
            // Mid-gameplay mode: gameplay_dim when menu is focused, fades to backdrop when dialog opens.
            gameplay_dim + (backdrop - gameplay_dim) * unfocused
        } else {
            // Menu background mode: no dim when menu is focused, fades to backdrop when unfocused.
            backdrop * unfocused
        }
    }

    /// Current dim overlay alpha at `time`.
    pub fn dim_overlay_alpha(&self, time: f64) -> f32 {
        let menu_open = matches!(
            self.mode,
            Mode::MainMenu | Mode::Pause | Mode::Options | Mode::NewGame | Mode::Console
        );
        let is_mid_gameplay =
            !self.background_level && matches!(self.base, Mode::Pause | Mode::Gameplay);
        let base_focus = self.shell.windows.base_focus_amount(time);
        Self::dim_overlay_opacity(is_mid_gameplay, menu_open, base_focus)
    }

    /// The active menu category for the current mode.
    pub fn menu_category(&self) -> &str {
        if self.base == Mode::Pause {
            self.pause_menu.category()
        } else {
            self.main_menu.category()
        }
    }

    /// Whether the loaded map only serves as the main-menu background.
    pub fn background_level(&self) -> bool {
        self.background_level
    }

    /// The effective main-menu background map, prioritizing the persisted config override
    /// before falling back to the installed `scripts/chapterbackgrounds.txt`.
    pub fn background_map(&self) -> Option<&str> {
        self.config
            .gameplay
            .background_map
            .as_deref()
            .or(self.gameui.background_map.as_deref())
    }

    /// The effective main-menu background: the persisted override (a map saved without its
    /// game belongs to the game that ships it), else chapter 1 of the first game's
    /// `scripts/chapterbackgrounds.txt`.
    pub fn background(&self) -> Option<GameMap> {
        let gameplay = &self.config.gameplay;
        match (&gameplay.background_game, &gameplay.background_map) {
            (Some(game), Some(map)) => Some(GameMap::new(game, map)),
            (None, Some(map)) => Some(self.game_map(map, None)),
            (_, None) => self
                .gameui
                .background_map
                .as_ref()
                .map(|map| GameMap::new(self.gameui.main_game(), map)),
        }
    }

    /// Overrides the persistent main-menu background (`None` restores the installed default).
    pub fn set_background(&mut self, background: Option<GameMap>) {
        let (game, map) = background.map(|b| (b.game, b.map)).unzip();
        self.config.gameplay.background_game = game;
        self.config.gameplay.background_map = map;
    }

    /// Called by the host after a map finished loading. A background level shows the main menu;
    /// any other map starts gameplay.
    pub fn level_loaded(&mut self, background: bool) {
        if background && self.background_transition.is_active() {
            self.background_transition.finish();
        }
        self.loading_dialog = None;
        self.loading_background = None;
        self.loading_progress.reset();
        self.background_level = background;
        // A pending keep-video confirmation outlives the load: closing it would keep the change.
        if !matches!(self.confirm_box, Some((_, ConfirmKind::KeepVideo(_)))) {
            self.confirm_box = None;
        }
        if !background {
            self.main_menu.back(&self.gameui.localization);
        }
        self.clear_menu_hover();
        if self.loaded_map.is_none() && background {
            self.loaded_map = self.background();
        }
        if background {
            // A menu background (a submenu's) loads behind the open windows: they stay.
            self.base = Mode::MainMenu;
            self.sync_mode();
        } else {
            self.set_mode(Mode::Gameplay);
        }
    }

    /// Completes loading `game`'s `map`. Ignores stale background completions.
    pub fn level_loaded_for_map(&mut self, game: &str, map: &str, background: bool) {
        if background
            && self.background_transition.is_active()
            && !self.background_transition.finish_map(game, map)
        {
            return;
        }
        self.loaded_map = Some(GameMap::new(game, map));
        if !background {
            // Saved now, not on the way back to the menu, so quitting mid-game still starts
            // over this chapter's background.
            self.remember_chapter_background();
        }
        self.level_loaded(background);
    }

    /// `map` of the game that ships it: `current` first, then Half-Life 2, then the other
    /// installed games (see `source_assets::install::game_for_map`).
    pub(super) fn game_map(&self, map: &str, current: Option<&str>) -> GameMap {
        let game = self
            .gameui
            .install_root
            .as_deref()
            .and_then(|root| source_assets::install::game_for_map(root, map, current))
            .unwrap_or_else(|| current.unwrap_or(self.gameui.main_game()).to_owned());
        GameMap::new(game, map)
    }

    /// Starts building the map -> chapter index of every game in the background (host, once).
    pub fn index_chapters(&mut self) {
        if let Some(root) = self.gameui.install_root.clone() {
            self.chapter_index =
                crate::gameui::chapters::ChapterIndex::build(root, self.gameui.chapters.clone());
        }
    }

    /// The chapter `game`'s `map` belongs to: first matching a chapter's starting map, then from
    /// the background-built transition index, or worked out for that game when the index is not
    /// built yet.
    fn chapter_of(&self, game: &str, map: &str) -> Option<String> {
        use crate::gameui::chapters;
        if let Some(chapters) = self.gameui.chapters.get(game) {
            if let Some(ch) = chapters.iter().find(|c| {
                c.start
                    .as_deref()
                    .is_some_and(|s| s.eq_ignore_ascii_case(map))
            }) {
                return Some(ch.key.clone());
            }
        }
        if let Some(found) = self.chapter_index.get(game, map) {
            return found.map(str::to_owned);
        }
        let root = self.gameui.install_root.as_ref()?;
        let list = self.gameui.chapters.get(game)?;
        chapters::assign(&chapters::map_links(root, game), list).remove(&map.to_ascii_lowercase())
    }

    /// Resolves the chapter background map for `game`'s `map` without blocking on unindexed BSP
    /// traversal: checks chapter start maps and the transition index first, then falls back to an
    /// active background from the same game or the game's chapter 1 background.
    pub(super) fn chapter_background_for(&self, game: &str, map: &str) -> Option<GameMap> {
        let chapter_key = self
            .gameui
            .chapters
            .get(game)
            .and_then(|chapters| {
                chapters
                    .iter()
                    .find(|c| {
                        c.start
                            .as_deref()
                            .is_some_and(|s| s.eq_ignore_ascii_case(map))
                    })
                    .map(|c| c.key.as_str())
            })
            .or_else(|| self.chapter_index.get(game, map)?);
        if let Some(ch) = chapter_key.and_then(|key| {
            self.gameui
                .chapters
                .get(game)?
                .iter()
                .find(|c| c.key == key)
        }) {
            return Some(GameMap::new(game, &ch.background));
        }
        if self.background_level {
            if let Some(loaded) = self.loaded_map.as_ref().filter(|m| m.game == game) {
                return Some(loaded.clone());
            }
        }
        if let Some(saved) = self.background().filter(|b| b.game == game) {
            return Some(saved);
        }
        self.gameui
            .background_maps
            .get(game)
            .map(|bg| GameMap::new(game, bg))
    }

    /// The menu background of the running map: the background of the chapter it belongs to in its
    /// game (`gameui::chapters`), else that game's chapter 1 background. It becomes the saved menu
    /// background (written only when it changes), so a Half-Life 2 map played from the Episode
    /// Two menu returns to, and next launches over, a Half-Life 2 background.
    fn remember_chapter_background(&mut self) -> Option<GameMap> {
        let running = self.loaded_map.clone()?;
        let game = running.game;
        let chapter_background = self.chapter_of(&game, &running.map).and_then(|key| {
            let chapters = self.gameui.chapters.get(&game)?;
            chapters
                .iter()
                .find(|c| c.key == key)
                .map(|c| c.background.clone())
        });
        let map = chapter_background.or_else(|| self.gameui.background_maps.get(&game).cloned())?;
        let background = GameMap::new(game, map);
        if self.background().as_ref() != Some(&background) {
            self.set_background(Some(background.clone()));
            let _ = self.config.save();
        }
        Some(background)
    }

    /// The background to show when leaving a map for the main menu: the running map's chapter
    /// background (already saved when the map loaded), else the saved one.
    fn menu_background_after_game(&mut self) -> Option<GameMap> {
        self.remember_chapter_background()
            .or_else(|| self.background())
    }

    /// Shows a game's menu category over that game's background. When a background level from any
    /// chapter of the same game is already active, keeps it without reloading chapter 1 (matching
    /// retail `SwitchExpansion`); otherwise loads that game's chapter 1 background.
    fn show_category(&mut self, category: &str) -> Vec<Effect> {
        let game = self.gameui.category_game(category).to_owned();
        if self.background_level
            && self
                .loaded_map
                .as_ref()
                .is_some_and(|loaded| loaded.game == game)
            && !self.background_transition.is_active()
        {
            self.background_transition.set_displayed_category(&game);
            return Vec::new();
        }
        let Some(map) = self.gameui.background_maps.get(&game).cloned() else {
            self.background_transition.set_displayed_category(category);
            return Vec::new();
        };
        let background = GameMap::new(game, map);
        self.set_background(Some(background.clone()));
        let _ = self.config.save();
        self.start_background_loading(&background.game, &background.map);
        vec![Effect::MainMenuMap(background)]
    }

    /// The main menu's geometry on this canvas, for run reports.
    pub fn menu_report(&self) -> serde_json::Value {
        let g = self.menu_layout.geometry_for(self.canvas_size());
        serde_json::json!({
            "x": g.menu.x, "y": g.menu.y, "font_tall": g.font_tall, "item_tall": g.item_tall,
            "row_wide": g.row_wide, "canvas": [self.canvas.width(), self.canvas.height()],
            "visible": !self.is_background_loading(),
        })
    }

    /// Clears hovered and selected state on both main and pause menus so underlying buttons
    /// are not armed while or immediately after a modal dialog is displayed.
    pub fn clear_menu_hover(&mut self) {
        self.main_menu.selection = None;
        self.pause_menu.selection = None;
        self.last_menu_hover = None;
        self.menu_tracker.reset();
    }

    /// Resets mouse tracker and hover state on the menus without wiping keyboard selection.
    pub fn reset_menu_interaction(&mut self) {
        self.last_menu_hover = None;
        self.menu_tracker.reset();
    }

    /// The open message box for run reports: its kind and text.
    pub fn confirm_report(&self) -> Option<serde_json::Value> {
        self.confirm_box.as_ref().map(|(confirm, kind)| {
            let kind = match kind {
                ConfirmKind::Quit => "quit",
                ConfirmKind::ReturnToMainMenu => "return_to_main_menu",
                ConfirmKind::StartCommentary(_) => "start_commentary",
                ConfirmKind::KeepVideo(_) => "keep_video",
            };
            serde_json::json!({"kind": kind, "text": confirm.text})
        })
    }

    pub(super) fn confirm_input(&mut self, input: &Input) -> Vec<Effect> {
        let mut effects = Vec::new();
        if let Some((mut confirm, kind)) = self.confirm_box.take() {
            let frame = self.quit_frame(&confirm);
            if self
                .quit_placement
                .input(&frame, input.pointer, input.click, input.held, input.time)
            {
                self.confirm_box = Some((confirm, kind));
                return effects;
            }
            let (origin, scale, layout) = self.quit_geometry(&confirm);
            let pointer = input.pointer.map(|p| (p - origin) / scale);
            let down = input.click || input.held;
            let outcome = confirm
                .input(&layout, pointer, down, input.click, input.released, false)
                .or_else(|| confirm.keys(input));
            for sound in confirm.take_sounds() {
                effects.push(Effect::PlaySound(sound));
            }
            match outcome {
                Some(true) => {
                    self.closing_confirm_box = Some((confirm, kind.clone()));
                    self.sync_dialog_windows();
                    match kind {
                        ConfirmKind::Quit => {
                            effects.push(Effect::Quit);
                            return effects;
                        }
                        ConfirmKind::KeepVideo(_) => {
                            self.reset_menu_interaction();
                            return effects;
                        }
                        ConfirmKind::StartCommentary(target) => {
                            effects.extend(self.start_chapter(target, true));
                            return effects;
                        }
                        ConfirmKind::ReturnToMainMenu => {
                            self.set_mode(Mode::MainMenu);
                            self.clear_menu_hover();
                            match self.menu_background_after_game() {
                                Some(background) => effects.push(Effect::MainMenuMap(background)),
                                None => {
                                    self.log("No main-menu background map is installed; keeping the current map frozen behind the menu.");
                                    self.level_loaded(true);
                                }
                            }
                            return effects;
                        }
                    }
                }
                Some(false) => {
                    self.closing_confirm_box = Some((confirm, kind.clone()));
                    self.reset_menu_interaction();
                    if let ConfirmKind::KeepVideo(keep) = kind {
                        effects.push(self.revert_video(keep.previous));
                    }
                    self.sync_dialog_windows();
                }
                None => self.confirm_box = Some((confirm, kind)),
            }
        }
        effects
    }

    /// The quit / main-menu box's frame (centered, moveable, no close button).
    pub(super) fn quit_frame(&self, quit: &MessageBox) -> placement::Frame {
        let scale = crate::proportional::dialog_scale(self.canvas_size());
        let measure = self.painter(Vec2::ZERO, 1.);
        let size = quit.layout(|text| measure.text_width(text, false)).size;
        let canvas = vec2(self.canvas.width(), self.canvas.height());
        placement::Frame {
            origin: ((canvas - size * scale) * 0.5).round(),
            size,
            scale,
            canvas,
            min_size: None,
            close: |_| Rect::new(0., 0., 0., 0.),
        }
    }

    /// The menu shown in the current mode, if any.
    fn game_menu(&self) -> Option<&GameMenu> {
        match self.base {
            Mode::MainMenu => Some(&self.main_menu),
            Mode::Pause => Some(&self.pause_menu),
            _ => None,
        }
    }

    /// One hit rectangle per menu row (spacers included), in canvas pixels.
    fn menu_rows(&self, count: usize) -> Vec<Rect> {
        self.menu_layout
            .geometry_for(self.canvas_size())
            .row_rects(count)
    }

    /// Canvas origin, scale and layout of the open quit box (centered on the canvas).
    pub(super) fn quit_geometry(&self, quit: &MessageBox) -> (Vec2, f32, MessageBoxLayout) {
        let scale = crate::proportional::dialog_scale(self.canvas_size());
        let measure = self.painter(Vec2::ZERO, 1.);
        let layout = quit.layout(|text| measure.text_width(text, false));
        let origin = self.quit_placement.place(&self.quit_frame(quit)).origin;
        (origin, scale, layout)
    }

    /// The menu: arrows and Enter when it has the keyboard, hover and clicks when it owns the
    /// pointer (`pointer`); without the pointer its tracker only follows the button (occluded).
    pub(super) fn menu_input(&mut self, input: &Input, pointer: bool) -> Vec<Effect> {
        if self.modal_dialog_open() {
            return Vec::new();
        }
        let mut effects = Vec::new();
        let count = self.game_menu().map_or(0, |menu| menu.entries().len());
        let rows = self.menu_rows(count);
        let menu = match self.base {
            Mode::MainMenu => &mut self.main_menu,
            _ => &mut self.pause_menu,
        };
        if !self.background_debug_mode && input.pressed(Key::Up) {
            let prev = menu.selection;
            menu.step(-1);
            if menu.selection != prev
                && menu
                    .selection
                    .and_then(|i| menu.entries().get(i))
                    .is_some_and(|e| e.has_rollover_sound())
            {
                effects.push(Effect::PlaySound("ui/buttonrollover.wav".into()));
            }
        }
        if !self.background_debug_mode && input.pressed(Key::Down) {
            let prev = menu.selection;
            menu.step(1);
            if menu.selection != prev
                && menu
                    .selection
                    .and_then(|i| menu.entries().get(i))
                    .is_some_and(|e| e.has_rollover_sound())
            {
                effects.push(Effect::PlaySound("ui/buttonrollover.wav".into()));
            }
        }
        let hovered = input
            .pointer
            .filter(|_| pointer)
            .and_then(|p| rows.iter().position(|row| row.contains(p)))
            .filter(|&i| menu.entries().get(i).is_some_and(|e| e.enabled));
        // Hover selects only when the pointer enters a button, so a pointer resting on a row
        // does not take the selection back from the arrow keys.
        if let Some(index) = hovered.filter(|h| Some(*h) != self.last_menu_hover) {
            menu.selection = Some(index);
        }
        if hovered.is_some()
            && hovered != self.last_menu_hover
            && hovered
                .and_then(|i| menu.entries().get(i))
                .is_some_and(|e| e.has_rollover_sound())
        {
            effects.push(Effect::PlaySound("ui/buttonrollover.wav".into()));
        }
        self.last_menu_hover = hovered;
        let down = input.click || input.held;
        let mut activate = if pointer {
            self.menu_tracker
                .update(hovered, down, input.click, input.released)
        } else {
            None
        };
        if input.pressed(Key::Enter) {
            activate = activate.or(menu.selection);
        }
        if let Some(index) = activate {
            effects.push(Effect::PlaySound("ui/buttonclickrelease.wav".into()));
            effects.extend(self.activate_menu(index));
        }
        effects
    }

    pub fn activate_menu(&mut self, index: usize) -> Vec<Effect> {
        if self.is_background_loading() {
            return Vec::new();
        }
        let gameui = self.gameui.clone();
        let loc = &gameui.localization;
        let outcome = match self.base {
            Mode::MainMenu => self.main_menu.activate(index, loc),
            Mode::Pause => self.pause_menu.activate(index, loc),
            _ => MenuOutcome::None,
        };
        match outcome {
            MenuOutcome::None => {}
            // Resuming hides the UI: its windows close (Options without applying).
            MenuOutcome::Resume => {
                if !self.background_level {
                    self.set_mode(Mode::Gameplay);
                }
            }
            MenuOutcome::OpenOptions => self.open_options(),
            MenuOutcome::ConfirmQuit => {
                self.reset_menu_interaction();
                self.quit_placement.reset();
                self.confirm_box = Some((crate::gameui::menu::quit_box(loc), ConfirmKind::Quit));
            }
            MenuOutcome::NewGame(game) => self.open_chapters(&game),
            MenuOutcome::ReturnToMainMenu => {
                self.reset_menu_interaction();
                self.quit_placement.reset();
                self.confirm_box = Some((
                    crate::gameui::menu::return_to_main_menu_box(loc),
                    ConfirmKind::ReturnToMainMenu,
                ));
            }
            MenuOutcome::SwitchExpansion(category) => return self.show_category(&category),
            MenuOutcome::SwitchCategory(category) => {
                if category != "main" && category != "extras" {
                    return self.show_category(&category);
                } else if category == "main" {
                    self.background_transition.set_displayed_category("main");
                }
            }
            MenuOutcome::Placeholder(label) => {
                self.log(format!("{label}: not implemented in HL2-RS yet."));
            }
        }
        Vec::new()
    }
}
