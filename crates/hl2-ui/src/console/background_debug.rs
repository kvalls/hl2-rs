//! Console commands and state for background visual debug mode.

use super::commands::{Outcome, Spec};
use super::{Console, Effect};
use source_ui::console::command::{boolean, parse};

impl Console {
    /// Whether background debug mode can currently be enabled (background map or blur present).
    pub fn can_enable_background_debug(&self) -> bool {
        self.background_level()
            || self.is_background_loading()
            || self
                .loading_dialog
                .as_ref()
                .is_some_and(|l| l.is_background())
            || self.base == super::Mode::MainMenu
    }

    /// Pre-blurred image name applied to the given background map.
    pub fn blur_texture_name(&self, map: &str, widescreen: bool) -> String {
        let base_map = crate::gameui::resources::blur(map);
        if widescreen {
            let wide_key = format!("{base_map}_widescreen");
            let found = self
                .background_textures
                .keys()
                .any(|(_, name)| name == &wide_key);
            if found || self.background_textures.is_empty() {
                return wide_key;
            }
        }
        base_map.to_owned()
    }

    /// Whether the current canvas aspect ratio is widescreen (> 4:3).
    pub fn is_widescreen(&self) -> bool {
        self.canvas.width() / self.canvas.height().max(1.0) > 4. / 3. + 1e-3
    }

    /// Initial background map for a game (`hl2`, `episodic`, `ep2`, `lostcoast`),
    /// prioritizing game resources (`scripts/chapterbackgrounds.txt`) over default fallbacks.
    pub fn initial_background_for_game(&self, game: &str) -> &str {
        if let Some(map) = self.gameui.background_map_for(game) {
            return map;
        }
        if let Some(first) = self.gameui.chapters.get(game).and_then(|c| c.first()) {
            return &first.background;
        }
        match game {
            "episodic" => "ep1_background01",
            "ep2" => "ep2_background01",
            _ => "background01",
        }
    }

    /// Resolves the installed game directory and initial background map for a slot
    /// (1: HL2, 2: Episode One, 3: Episode Two, 4: Lost Coast) by reusing existing
    /// `GameUiResources::category_game` and `GameUiResources::background_map_for`.
    pub fn game_and_initial_background_for_slot(&self, slot: u8) -> Option<(String, String)> {
        let alias = match slot {
            1 => "main",
            2 => "hl2ep1",
            3 => "hl2ep2",
            4 => "hl2lc",
            _ => return None,
        };
        let game = self.gameui.category_game(alias);
        let map = self.initial_background_for_game(game);
        Some((game.to_owned(), map.to_owned()))
    }

    /// Returns the ordered list of all `(game, map)` chapter backgrounds across all games,
    /// resolved from game resources (`scripts/chapterbackgrounds.txt`).
    pub fn all_chapter_backgrounds(&self) -> Vec<(String, String)> {
        let games: Vec<&str> = if !self.gameui.games.is_empty() {
            self.gameui.games.iter().map(String::as_str).collect()
        } else {
            vec!["hl2", "episodic", "ep2", "lostcoast"]
        };
        let mut result = Vec::new();
        for game in games {
            if let Some(chapters) = self.gameui.chapters.get(game) {
                for chapter in chapters {
                    if !result
                        .iter()
                        .any(|(g, m)| g == game && m == &chapter.background)
                    {
                        result.push((game.to_owned(), chapter.background.clone()));
                    }
                }
            }
            if !result.iter().any(|(g, _)| g == game) {
                let initial = self.initial_background_for_game(game);
                result.push((game.to_owned(), initial.to_owned()));
            }
        }
        result
    }

    /// Target game and map of the current or upcoming background.
    pub fn current_background_target(&self) -> (String, String) {
        if let Some((game, map)) = self.current_background() {
            (game.to_owned(), map.to_owned())
        } else if let Some(bg) = self.background() {
            (bg.game.clone(), bg.map.clone())
        } else {
            ("hl2".to_owned(), "background01".to_owned())
        }
    }

    /// Initiates a background swap to `game`'s `map`.
    pub fn swap_background(&mut self, game: &str, map: &str) -> Effect {
        let background = super::GameMap::new(game, map);
        self.set_background(Some(background.clone()));
        let _ = self.config.save();
        self.start_background_loading(game, map);
        Effect::MainMenuMap(background)
    }
}

pub(super) fn background_debug_mode(
    console: &mut Console,
    spec: &'static Spec,
    args: &[String],
) -> Outcome {
    let [value] = args else {
        return Err(spec.usage());
    };
    let Some(on) = boolean(value) else {
        return Err(spec.usage());
    };
    if on {
        if !console.can_enable_background_debug() {
            console.log("background_debug_mode needs a background map or loading blur; start or return to the main menu.");
            return Ok(None);
        }
        console.background_debug_mode = true;
        console.log("background_debug_mode 1");
        console.shell.windows.focus_base();
        console.sync_mode();
        Ok(Some(Effect::BackgroundDebugMode(true)))
    } else {
        console.background_debug_mode = false;
        console.log("background_debug_mode 0");
        Ok(Some(Effect::BackgroundDebugMode(false)))
    }
}

pub(super) fn background_blur_opacity(
    console: &mut Console,
    spec: &'static Spec,
    args: &[String],
) -> Outcome {
    if args.is_empty() {
        console.log(format!(
            "\"background_blur_opacity\" = \"{}\"",
            console.background_blur_opacity
        ));
        return Ok(None);
    }
    let [value] = args else {
        return Err(spec.usage());
    };
    if !console.background_debug_mode {
        console.log("background_blur_opacity requires background_debug_mode 1.");
        return Ok(None);
    }
    let opacity: u8 = parse(value).map_err(|_| spec.usage())?;
    if opacity > 100 {
        return Err(spec.usage());
    }
    console.background_blur_opacity = opacity;
    console.log(format!("background_blur_opacity {opacity}"));
    Ok(Some(Effect::BackgroundBlurOpacity(opacity)))
}

pub(super) fn background_map_fov(
    _console: &mut Console,
    spec: &'static Spec,
    args: &[String],
) -> Outcome {
    if args.is_empty() {
        return Ok(Some(Effect::BackgroundMapFov(None)));
    }
    let [value] = args else {
        return Err(spec.usage());
    };
    let deg: f32 = parse(value).map_err(|_| spec.usage())?;
    if !(1.0..=170.0).contains(&deg) {
        return Err(spec.usage());
    }
    Ok(Some(Effect::BackgroundMapFov(Some(deg))))
}
