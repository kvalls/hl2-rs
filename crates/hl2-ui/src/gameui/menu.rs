//! The game menu (main menu, its HL2/Episode categories and the in-game menu) and its quit and
//! return-to-main-menu confirmations (toolkit message boxes).
//!
//! Entries come from `GameMenu.res`. Out of game the current `MenuCategory` is shown in file
//! order; in game the uncategorized `OnlyInGame` entries are shown sorted by `InGameOrder`.
//! Commands without a Rust implementation (Load/Save Game, Achievements, Workshop, New Game for a
//! game whose maps are not mounted) are shown disabled, so they are visibly placeholders.

use super::localize::Localization;
use super::resources::{MenuItem, Visibility};
use glam::{vec2, Vec2};
use source_assets::keyvalues::Entry;
use source_ui::canvas::Rect;
use source_ui::message_box::MessageBox;
use std::collections::BTreeMap;

/// A parsed `GameMenu.res` command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuCommand {
    /// `SwitchExpansion <game>`: opens that game's category.
    SwitchExpansion(String),
    SwitchCategory(String),
    OpenOptions,
    Quit,
    Resume,
    /// `OpenNewGameDialog <game>`.
    NewGame(String),
    LoadGame,
    SaveGame,
    Achievements,
    Workshop,
    ReturnToMainMenu,
    Unsupported(String),
}

impl MenuCommand {
    pub fn parse(command: &str) -> Self {
        let mut words = command.split_whitespace();
        let verb = words.next().unwrap_or_default().to_ascii_lowercase();
        let argument = words.next().unwrap_or_default().to_ascii_lowercase();
        match verb.as_str() {
            "switchexpansion" => Self::SwitchExpansion(argument),
            "switchcategory" => Self::SwitchCategory(argument),
            "openoptionsdialog" => Self::OpenOptions,
            "quit" | "quitnoconfirm" => Self::Quit,
            "resumegame" => Self::Resume,
            "opennewgamedialog" => Self::NewGame(if argument.is_empty() {
                "hl2".into()
            } else {
                argument
            }),
            "openloadgamedialog" => Self::LoadGame,
            "opensavegamedialog" => Self::SaveGame,
            "openachievementsdialog" => Self::Achievements,
            "openworkshopdialog" => Self::Workshop,
            "openmainmenudialog" | "disconnect" => Self::ReturnToMainMenu,
            _ => Self::Unsupported(command.to_owned()),
        }
    }
}

/// One visible row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuEntry {
    pub label: String,
    pub command: MenuCommand,
    pub spacer: bool,
    pub enabled: bool,
    pub kind: source_ui::widgets::WidgetKind,
    pub rollover_sound: Option<&'static str>,
}

impl MenuEntry {
    /// Returns whether this menu entry is eligible to play a rollover sound.
    /// Only enabled non-spacer menu buttons play rollover sounds.
    pub fn has_rollover_sound(&self) -> bool {
        self.kind.has_rollover_sound()
            && self.rollover_sound.is_some()
            && self.enabled
            && !self.spacer
    }
}

/// What activating an entry asks the owner to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuOutcome {
    None,
    Resume,
    OpenOptions,
    ConfirmQuit,
    /// Open the named game's chapter browser.
    NewGame(String),
    ReturnToMainMenu,
    /// Switched to an expansion submenu (e.g. "hl2", "episodic", "ep2").
    SwitchExpansion(String),
    /// Switched to a category submenu (e.g. "main", "extras").
    SwitchCategory(String),
    /// A disabled placeholder was activated; the label is reported to the console.
    Placeholder(String),
}

pub const MAIN_CATEGORY: &str = "main";

pub struct GameMenu {
    items: Vec<MenuItem>,
    new_game_maps: BTreeMap<String, String>,
    category: String,
    in_game: bool,
    entries: Vec<MenuEntry>,
    pub selection: Option<usize>,
}

impl GameMenu {
    pub fn new(
        items: Vec<MenuItem>,
        new_game_maps: BTreeMap<String, String>,
        loc: &Localization,
    ) -> Self {
        let mut menu = Self {
            items,
            new_game_maps,
            category: MAIN_CATEGORY.into(),
            in_game: false,
            entries: Vec::new(),
            selection: None,
        };
        menu.rebuild(loc);
        menu
    }

    pub fn entries(&self) -> &[MenuEntry] {
        &self.entries
    }

    pub fn category(&self) -> &str {
        &self.category
    }

    pub fn in_game(&self) -> bool {
        self.in_game
    }

    /// Switches between the main menu and the in-game menu; leaving the game resets the category.
    pub fn set_in_game(&mut self, in_game: bool, loc: &Localization) {
        if self.in_game != in_game {
            self.in_game = in_game;
            self.category = MAIN_CATEGORY.into();
            self.selection = None;
            self.rebuild(loc);
        }
    }

    fn set_category(&mut self, category: &str, loc: &Localization) {
        self.category = category.to_ascii_lowercase();
        self.selection = None;
        self.rebuild(loc);
    }

    fn rebuild(&mut self, loc: &Localization) {
        let mut items: Vec<&MenuItem> = if self.in_game {
            let mut items: Vec<_> = self
                .items
                .iter()
                .filter(|i| i.category.is_none() && i.visibility != Visibility::NotInGame)
                .collect();
            items.sort_by_key(|i| i.order);
            items
        } else {
            self.items
                .iter()
                .filter(|i| {
                    i.category.as_deref() == Some(self.category.as_str())
                        && i.visibility != Visibility::OnlyInGame
                })
                .collect()
        };
        // A trailing or doubled spacer adds nothing visible.
        items.dedup_by(|a, b| a.is_spacer() && b.is_spacer());
        self.entries = items
            .into_iter()
            .map(|item| {
                let command = MenuCommand::parse(&item.command);
                let label = if item.is_spacer() {
                    String::new()
                } else {
                    let text = loc.localize(&item.label);
                    if item.caps {
                        text.to_uppercase()
                    } else {
                        text
                    }
                };
                let enabled = !item.is_spacer() && self.implemented(&command);
                let (kind, rollover_sound) = if item.is_spacer() {
                    (source_ui::widgets::WidgetKind::Other, None)
                } else {
                    (
                        source_ui::widgets::WidgetKind::MenuButton,
                        Some("ui/buttonrollover.wav"),
                    )
                };
                MenuEntry {
                    enabled,
                    spacer: item.is_spacer(),
                    label,
                    command,
                    kind,
                    rollover_sound,
                }
            })
            .collect();
    }

    fn implemented(&self, command: &MenuCommand) -> bool {
        match command {
            MenuCommand::SwitchExpansion(game) => self
                .items
                .iter()
                .any(|i| i.category.as_deref() == Some(game.as_str())),
            // The Extras category opens the extras submenu (Lost Coast, Workshop placeholder).
            MenuCommand::SwitchCategory(category) => {
                category == MAIN_CATEGORY
                    || self
                        .items
                        .iter()
                        .any(|i| i.category.as_deref() == Some(category.as_str()))
            }
            MenuCommand::NewGame(game) => self.new_game_maps.contains_key(game),
            MenuCommand::OpenOptions
            | MenuCommand::Quit
            | MenuCommand::Resume
            | MenuCommand::ReturnToMainMenu => true,
            MenuCommand::LoadGame
            | MenuCommand::SaveGame
            | MenuCommand::Achievements
            | MenuCommand::Workshop
            | MenuCommand::Unsupported(_) => false,
        }
    }

    /// Moves the keyboard selection by `step`, skipping spacers and disabled rows.
    pub fn step(&mut self, step: i32) {
        let count = self.entries.len() as i32;
        if count == 0 || !self.entries.iter().any(|e| e.enabled) {
            return;
        }
        let mut index = self
            .selection
            .map_or(if step > 0 { -1 } else { count }, |i| i as i32);
        loop {
            index = (index + step).rem_euclid(count);
            if self.entries[index as usize].enabled {
                self.selection = Some(index as usize);
                return;
            }
        }
    }

    /// Escape inside a category returns to the main category. Returns whether it was consumed.
    pub fn back(&mut self, loc: &Localization) -> bool {
        if !self.in_game && self.category != MAIN_CATEGORY {
            self.set_category(MAIN_CATEGORY, loc);
            true
        } else {
            false
        }
    }

    pub fn activate(&mut self, index: usize, loc: &Localization) -> MenuOutcome {
        let Some(entry) = self.entries.get(index).cloned() else {
            return MenuOutcome::None;
        };
        if entry.spacer {
            return MenuOutcome::None;
        }
        if !entry.enabled {
            return MenuOutcome::Placeholder(entry.label);
        }
        match entry.command {
            MenuCommand::SwitchExpansion(game) => {
                self.set_category(&game, loc);
                MenuOutcome::SwitchExpansion(game)
            }
            MenuCommand::SwitchCategory(category) => {
                self.set_category(&category, loc);
                MenuOutcome::SwitchCategory(category)
            }
            MenuCommand::OpenOptions => MenuOutcome::OpenOptions,
            MenuCommand::Quit => MenuOutcome::ConfirmQuit,
            MenuCommand::Resume => MenuOutcome::Resume,
            MenuCommand::NewGame(game) => MenuOutcome::NewGame(game),
            MenuCommand::ReturnToMainMenu => MenuOutcome::ReturnToMainMenu,
            _ => MenuOutcome::Placeholder(entry.label),
        }
    }
}

/// The Quit confirmation (`#GameUI_QuitConfirmationTitle`/`Text`, `#GameUI_Quit`, `#GameUI_Cancel`).
pub fn quit_box(loc: &Localization) -> MessageBox {
    MessageBox::new(
        loc.text("#GameUI_QuitConfirmationTitle"),
        loc.text("#GameUI_QuitConfirmationText"),
        loc.text("#GameUI_Quit"),
        Some(loc.text("#GameUI_Cancel")),
    )
}

/// The Return to Main Menu confirmation dialog (`#GameUI_ConfirmMainMenu_Title`, `#GameUI_MainMenuWarning`, `#GameUI_OK`, `#GameUI_Cancel`).
pub fn return_to_main_menu_box(loc: &Localization) -> MessageBox {
    MessageBox::new(
        loc.text("#GameUI_ConfirmMainMenu_Title"),
        loc.text("#GameUI_MainMenuWarning"),
        loc.text("#GameUI_OK"),
        Some(loc.text("#GameUI_Cancel")),
    )
}

/// The Developer Commentary explanation confirmation dialog (`#GameUI_CommentaryDialogTitle`,
/// `#GAMEUI_Commentary_Console_Explanation`, `#vgui_ok`, `#vgui_cancel`, matching
/// `hl2/resource/commentaryexplanationdialog.res`).
pub fn commentary_explanation_box(loc: &Localization, measure: impl Fn(&str) -> f32) -> MessageBox {
    let raw = loc.text("#GAMEUI_Commentary_Console_Explanation");
    let wrapped = source_ui::widgets::wrap(&raw, 320.0, measure).join("\n");
    MessageBox::new(
        loc.text("#GameUI_CommentaryDialogTitle"),
        wrapped,
        loc.text("#vgui_ok"),
        Some(loc.text("#vgui_cancel")),
    )
}

/// Reference design vertical resolution for Source UI coordinates (480p).
pub const BASE_SCREEN_HEIGHT: f32 = 480.;
/// Default X offset for the main menu title in 480p units (`Main.Title1.X`).
pub const DEFAULT_TITLE_X: f32 = 76.;
/// Default Y offset for the main menu title in 480p units (`Main.Title1.Y`).
pub const DEFAULT_TITLE_Y: f32 = 145.;
/// Default X offset for the submenu secondary title in 480p units (`Main.Title2.X`).
pub const DEFAULT_TITLE2_X: f32 = 76.;
/// Default Y offset for the submenu secondary title in 480p units (`Main.Title2.Y`).
pub const DEFAULT_TITLE2_Y: f32 = 172.;
/// Default X offset for the menu buttons in 480p units (`Main.Menu.X`).
pub const DEFAULT_MENU_X: f32 = 76.;
/// Default Y offset for the menu buttons in 480p units (`Main.Menu.Y`).
pub const DEFAULT_MENU_Y: f32 = 240.;
/// Default title text height in 480p units.
pub const DEFAULT_TITLE_TALL: f32 = 32.;
/// Default item height from scheme (`MainMenu.MenuItemHeight`).
pub const DEFAULT_ITEM_TALL: f32 = 26.;
/// Default menu font cell height.
pub const DEFAULT_FONT_TALL: f32 = 11.;
/// Base hit area width for menu items in 480p units.
pub const MENU_ROW_BASE_WIDTH: f32 = 250.;

/// Layout parameters for the GameMenu's title and items, loaded from schemes or defaulting to desktop coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuLayout {
    pub title_x: f32,
    pub title_y: f32,
    pub title2_x: f32,
    pub title2_y: f32,
    pub menu_x: f32,
    pub menu_y: f32,
    pub title_tall: f32,
    /// `MainMenu.MenuItemHeight` in 480p units, and the `MainMenu.MaxPropMenuItemHeight` cap.
    pub item_tall: f32,
    pub max_item_tall: Option<f32>,
    /// The menu font's scheme records in file order (`MainMenuFont`, else `MenuLarge`).
    pub font_rules: Vec<FontRule>,
}

impl Default for MenuLayout {
    fn default() -> Self {
        // Stock desktop defaults, overridden by the mounted schemes below.
        Self {
            title_x: DEFAULT_TITLE_X,
            title_y: DEFAULT_TITLE_Y,
            title2_x: DEFAULT_TITLE2_X,
            title2_y: DEFAULT_TITLE2_Y,
            menu_x: DEFAULT_MENU_X,
            menu_y: DEFAULT_MENU_Y,
            title_tall: DEFAULT_TITLE_TALL,
            item_tall: DEFAULT_ITEM_TALL,
            max_item_tall: None,
            font_rules: vec![(16., Some((1., 1080.))), (DEFAULT_FONT_TALL, None)],
        }
    }
}

/// Scaled screen coordinates and sizes for rendering and hit-testing the GameMenu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuGeometry {
    pub title: Vec2,
    pub title2: Vec2,
    pub menu: Vec2,
    pub title_tall: f32,
    pub font_tall: f32,
    pub item_tall: f32,
    /// Width of a row's hit area.
    pub row_wide: f32,
}

impl MenuGeometry {
    /// Canvas hit rectangle for menu row `index`.
    pub fn row_rect(&self, index: usize) -> Rect {
        Rect::new(
            self.menu.x,
            self.menu.y + index as f32 * self.item_tall,
            self.row_wide,
            self.item_tall,
        )
    }

    /// Hit rectangles for all menu rows up to `count`.
    pub fn row_rects(&self, count: usize) -> Vec<Rect> {
        (0..count).map(|i| self.row_rect(i)).collect()
    }

    /// Canvas position for vertically centering the text inside row `index`.
    pub fn row_text_pos(&self, index: usize) -> Vec2 {
        self.menu
            + vec2(
                0.,
                (index as f32 * self.item_tall + (self.item_tall - self.font_tall) * 0.5).floor(),
            )
    }
}

impl MenuLayout {
    pub fn geometry(&self, height: f32) -> MenuGeometry {
        self.geometry_scaled(height, 1.0)
    }

    /// The geometry on a `canvas`. The titles and the menu's position follow the UI's design
    /// height ([`crate::proportional::design_height`]), which on a screen narrower than the
    /// design shrinks to fit the width. The rows' text and spacing keep following the screen
    /// height there (so they do not shrink with it), and the hit width is limited to what fits
    /// beside the menu's margin.
    pub fn geometry_for(&self, canvas: Vec2) -> MenuGeometry {
        let design = crate::proportional::design_height(canvas);
        let mut geometry = self.geometry(design);
        if design < canvas.y {
            let rows = self.geometry(canvas.y);
            geometry.font_tall = rows.font_tall;
            geometry.item_tall = rows.item_tall;
            geometry.row_wide = rows.row_wide.min(canvas.x - 2. * geometry.menu.x);
        } else if crate::proportional::widescreen_menu_scale(canvas) > 1. {
            // A wide screen is not taller: grow the whole block by the widescreen factor and let
            // it grow upward, the bottom of the (tallest) row stack staying where it was.
            let grown = self.geometry(design * crate::proportional::widescreen_menu_scale(canvas));
            let rows = crate::tuning::menu_stack_rows();
            let bottom = |g: &MenuGeometry| g.menu.y + rows * g.item_tall;
            let shift = bottom(&geometry) - bottom(&grown);
            geometry = MenuGeometry {
                title: grown.title + vec2(0., shift),
                title2: grown.title2 + vec2(0., shift),
                menu: grown.menu + vec2(0., shift),
                ..grown
            };
        }
        geometry
    }

    /// Computes geometry taking into account logical height and display scale factor.
    pub fn geometry_scaled(&self, height: f32, scale_factor: f32) -> MenuGeometry {
        let effective_height = height * scale_factor.max(1.0);
        let scale = effective_height / BASE_SCREEN_HEIGHT;
        // Retail CScheme::ReloadFontGlyphs takes the first record whose yres range holds the
        // screen height; only a record without a range is proportionally scaled (truncated).
        let font_tall = self
            .font_rules
            .iter()
            .find(|(_, range)| {
                range.is_none_or(|(low, high)| (low..=high).contains(&effective_height))
            })
            .map_or(DEFAULT_FONT_TALL, |&(tall, range)| match range {
                Some(_) => tall,
                None => (tall * scale).trunc(),
            });
        // Retail CGameMenu::ApplySchemeSettings: the proportional MainMenu.MenuItemHeight,
        // capped by MainMenu.MaxPropMenuItemHeight when that is set.
        let item_tall = (self.item_tall * scale)
            .trunc()
            .min(self.max_item_tall.unwrap_or(f32::INFINITY));
        MenuGeometry {
            title: vec2(
                (self.title_x * scale).trunc(),
                (self.title_y * scale).trunc(),
            ),
            title2: vec2(
                (self.title2_x * scale).trunc(),
                (self.title2_y * scale).trunc(),
            ),
            menu: vec2((self.menu_x * scale).trunc(), (self.menu_y * scale).trunc()),
            title_tall: (self.title_tall * scale).round(),
            font_tall,
            item_tall,
            row_wide: MENU_ROW_BASE_WIDTH * scale,
        }
    }

    pub fn apply_source_scheme(&mut self, root: &Entry) {
        if let Some(tall) = resource_number(root, "MainMenu.MenuItemHeight")
            .filter(|value| *value > 0. && *value <= 128.)
        {
            self.item_tall = tall;
        }
        if let Some(tall) =
            resource_number(root, "MainMenu.MaxPropMenuItemHeight").filter(|value| *value > 0.)
        {
            self.max_item_tall = Some(tall);
        }
        if let Some(fonts) =
            resource_font(root, "MainMenuFont").or_else(|| resource_font(root, "MenuLarge"))
        {
            self.font_rules = fonts;
        }
    }

    pub fn apply_client_scheme(&mut self, root: &Entry) {
        for (key, target) in [
            ("Main.Title1.X", &mut self.title_x),
            ("Main.Title1.Y", &mut self.title_y),
            ("Main.Title2.X", &mut self.title2_x),
            ("Main.Title2.Y", &mut self.title2_y),
            ("Main.Menu.X", &mut self.menu_x),
            ("Main.Menu.Y", &mut self.menu_y),
        ] {
            if let Some(value) = resource_number(root, key) {
                *target = value;
            }
        }
        // In the original 2004 HL2 ClientScheme.res, Title2.X was 291 and Title2.Y was 207 (the
        // numeral '2' logo suffix next to 'HALF-LIFE'). In the 20th Anniversary combined launcher
        // and episode submenus, Title2 is an expansion subtitle (e.g. '== episode one ==') that
        // aligns horizontally with the menu button column (Title2.X == Menu.X == 76, Title2.Y == 172
        // in hl2_complete ClientScheme.res). If a legacy scheme supplies the old logo-suffix coordinate
        // (>= 200 or significantly divergent from menu_x), normalize it to align with the menu column.
        if self.title2_x >= 200.0 || (self.title2_x - self.menu_x).abs() > 50.0 {
            self.title2_x = self.menu_x;
            if self.title2_y > 200.0 {
                self.title2_y = DEFAULT_TITLE2_Y;
            }
        }
        if let Some(fonts) = resource_font(root, "ClientTitleFont") {
            self.title_tall = fonts[0].0;
        }
    }
}

pub fn resource_number(root: &Entry, key: &str) -> Option<f32> {
    root.get("BaseSettings")?
        .get(key)?
        .text()?
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
}

pub type FontRule = (f32, Option<(f32, f32)>);

pub fn resource_font(root: &Entry, name: &str) -> Option<Vec<FontRule>> {
    let entries = root.get("Fonts")?.get(name)?.children();
    let values: Vec<_> = entries
        .iter()
        .filter_map(|entry| {
            let tall = entry.get("tall")?.text()?.parse::<f32>().ok()?;
            if !tall.is_finite() || tall <= 0. || tall > 256. {
                return None;
            }
            // Retail "%d %d": a zero minimum means no range, a missing maximum equals the minimum.
            let range = entry.get("yres").and_then(|y| y.text()).and_then(|text| {
                let mut parts = text.split_whitespace();
                let low = parts.next()?.parse::<f32>().ok()?;
                let high = parts
                    .next()
                    .and_then(|v| v.parse::<f32>().ok())
                    .filter(|v| *v != 0.)
                    .unwrap_or(low);
                (low != 0. && low.is_finite() && high.is_finite() && low <= high)
                    .then_some((low, high))
            });
            Some((tall, range))
        })
        .collect();
    (!values.is_empty()).then_some(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameui::resources::fallback_game_menu;

    fn loc() -> Localization {
        let mut loc = Localization::default();
        for (token, text) in [
            ("GameUI_HalfLife2", "Half-Life 2"),
            ("GameUI_HalfLife2EpisodeOne", "HL2: Episode One"),
            ("GameUI_HalfLife2EpisodeTwo", "HL2: Episode Two"),
            ("GameUI_Extras", "Extras"),
            ("GameUI_HalfLife2LostCoast", "Half-Life 2: Lost Coast"),
            ("GameUI_Worskhop_MenuName", "Workshop"),
            ("GameUI_Workshop_MenuName", "Workshop"),
            ("GameUI_LostCoast", "Lost Coast"),
            ("GameUI_GameMenu_Options", "Options"),
            ("GameUI_GameMenu_Quit", "Quit"),
            ("GameUI_GameMenu_NewGame", "New Game"),
            ("GameUI_GameMenu_LoadGame", "Load Game"),
            ("GameUI_GameMenu_Achievements", "Achievements"),
            ("GameUI_Back", "Back"),
            ("GameUI_QuitConfirmationTitle", "QUIT GAME"),
            (
                "GameUI_QuitConfirmationText",
                "Do you wish to stop playing now?",
            ),
            ("GameUI_Quit", "Quit game"),
            ("GameUI_Cancel", "Cancel"),
        ] {
            loc.insert(token, text);
        }
        loc
    }

    fn menu() -> GameMenu {
        let maps = BTreeMap::from([("hl2".to_owned(), "d1_trainstation_01".to_owned())]);
        GameMenu::new(fallback_game_menu(), maps, &loc())
    }

    fn labels(menu: &GameMenu) -> Vec<&str> {
        menu.entries().iter().map(|e| e.label.as_str()).collect()
    }

    #[test]
    fn main_menu_lists_games_spacer_extras_options_quit() {
        let menu = menu();
        assert_eq!(
            labels(&menu),
            [
                "HALF-LIFE 2",
                "HL2: EPISODE ONE",
                "HL2: EPISODE TWO",
                "",
                "EXTRAS",
                "OPTIONS",
                "QUIT"
            ]
        );
        assert!(menu.entries()[3].spacer);
        assert!(menu.entries()[4].enabled, "Extras opens the extras submenu");
        assert!(menu.entries()[5].enabled && menu.entries()[6].enabled);
    }

    #[test]
    fn game_categories_have_new_load_achievements_spacer_options_back() {
        let loc = loc();
        for (index, game, new_game_enabled) in
            [(0, "hl2", true), (1, "episodic", false), (2, "ep2", false)]
        {
            let mut menu = menu();
            assert_eq!(
                menu.activate(index, &loc),
                MenuOutcome::SwitchExpansion(game.into())
            );
            assert_eq!(menu.category(), game);
            assert_eq!(
                labels(&menu),
                [
                    "NEW GAME",
                    "LOAD GAME",
                    "ACHIEVEMENTS",
                    "",
                    "OPTIONS",
                    "BACK"
                ]
            );
            assert_eq!(menu.entries()[0].enabled, new_game_enabled, "{game}");
            assert!(
                !menu.entries()[1].enabled && !menu.entries()[2].enabled,
                "placeholders"
            );
            assert_eq!(menu.activate(4, &loc), MenuOutcome::OpenOptions);
            assert_eq!(
                menu.activate(5, &loc),
                MenuOutcome::SwitchCategory(MAIN_CATEGORY.into())
            );
            assert_eq!(
                menu.category(),
                MAIN_CATEGORY,
                "Back returns to the main category"
            );
        }
    }

    #[test]
    fn activation_outcomes_and_placeholders() {
        let loc = loc();
        let mut menu = menu();
        assert_eq!(menu.activate(6, &loc), MenuOutcome::ConfirmQuit);
        assert_eq!(menu.activate(3, &loc), MenuOutcome::None, "spacer");
        assert_eq!(
            menu.activate(4, &loc),
            MenuOutcome::SwitchCategory("extras".into())
        );
        assert_eq!(menu.category(), "extras");
        assert_eq!(
            menu.activate(1, &loc),
            MenuOutcome::Placeholder("WORKSHOP".into())
        );
        assert!(menu.back(&loc));
        assert_eq!(menu.category(), MAIN_CATEGORY);
        menu.activate(0, &loc);
        assert_eq!(menu.activate(0, &loc), MenuOutcome::NewGame("hl2".into()));
        assert!(menu.back(&loc));
        assert!(!menu.back(&loc), "main category has nothing to go back to");
    }

    #[test]
    fn keyboard_selection_skips_spacers_and_disabled_rows() {
        let mut menu = menu();
        menu.step(1);
        assert_eq!(menu.selection, Some(0));
        menu.step(1);
        assert_eq!(menu.selection, Some(1));
        menu.step(1);
        assert_eq!(menu.selection, Some(2));
        menu.step(1);
        assert_eq!(menu.selection, Some(4), "spacer skipped, Extras enabled");
        menu.step(1);
        assert_eq!(menu.selection, Some(5));
        menu.step(1);
        assert_eq!(menu.selection, Some(6));
        menu.step(1);
        assert_eq!(menu.selection, Some(0), "wraps around");
        menu.step(-1);
        assert_eq!(menu.selection, Some(6));
    }

    #[test]
    fn extras_submenu_and_lostcoast_game_submenu() {
        let loc = loc();
        let maps = BTreeMap::from([
            ("hl2".to_owned(), "d1_trainstation_01".to_owned()),
            ("lostcoast".to_owned(), "d2_lostcoast".to_owned()),
        ]);
        let mut menu = GameMenu::new(fallback_game_menu(), maps, &loc);

        // Main menu EXTRAS button opens extras submenu
        assert_eq!(
            menu.activate(4, &loc),
            MenuOutcome::SwitchCategory("extras".into())
        );
        assert_eq!(menu.category(), "extras");
        assert_eq!(
            labels(&menu),
            ["HALF-LIFE 2: LOST COAST", "WORKSHOP", "", "BACK"]
        );
        assert!(menu.entries()[0].enabled, "Lost Coast entry enabled");
        assert!(
            !menu.entries()[1].enabled,
            "Workshop button is disabled placeholder"
        );
        assert!(menu.entries()[2].spacer, "spacer");
        assert!(menu.entries()[3].enabled, "Back button enabled");

        // Activating Workshop returns placeholder outcome
        assert_eq!(
            menu.activate(1, &loc),
            MenuOutcome::Placeholder("WORKSHOP".into())
        );

        // Activating Lost Coast entry opens lostcoast game submenu
        assert_eq!(
            menu.activate(0, &loc),
            MenuOutcome::SwitchExpansion("lostcoast".into())
        );
        assert_eq!(menu.category(), "lostcoast");
        assert_eq!(
            labels(&menu),
            [
                "NEW GAME",
                "LOAD GAME",
                "ACHIEVEMENTS",
                "",
                "OPTIONS",
                "BACK"
            ]
        );
        assert!(menu.entries()[0].enabled, "New Game enabled for lostcoast");
        assert!(!menu.entries()[1].enabled, "Load Game placeholder");
        assert!(!menu.entries()[2].enabled, "Achievements placeholder");
        assert!(menu.entries()[3].spacer, "spacer");
        assert!(menu.entries()[4].enabled, "Options enabled");
        assert!(menu.entries()[5].enabled, "Back enabled");

        // New Game opens the selected game's chapter dialog
        assert_eq!(
            menu.activate(0, &loc),
            MenuOutcome::NewGame("lostcoast".into())
        );

        // Options opens options
        assert_eq!(menu.activate(4, &loc), MenuOutcome::OpenOptions);

        // Back returns to main category
        assert_eq!(
            menu.activate(5, &loc),
            MenuOutcome::SwitchCategory(MAIN_CATEGORY.into())
        );
        assert_eq!(menu.category(), MAIN_CATEGORY);

        // Re-entering extras and testing Escape
        assert_eq!(
            menu.activate(4, &loc),
            MenuOutcome::SwitchCategory("extras".into())
        );
        assert_eq!(menu.category(), "extras");
        assert!(menu.back(&loc));
        assert_eq!(menu.category(), MAIN_CATEGORY);
    }

    #[test]
    fn extras_keyboard_selection_skips_workshop_placeholder_and_spacer() {
        let loc = loc();
        let mut menu = menu();
        menu.activate(4, &loc);
        assert_eq!(menu.category(), "extras");
        menu.step(1);
        assert_eq!(menu.selection, Some(0)); // HALF-LIFE 2: LOST COAST
        menu.step(1);
        assert_eq!(
            menu.selection,
            Some(3),
            "skips disabled Workshop (1) and spacer (2)"
        );
        menu.step(1);
        assert_eq!(menu.selection, Some(0), "wraps around to Lost Coast");
    }

    #[test]
    fn in_game_menu_uses_uncategorized_entries_in_order() {
        let loc = loc();
        let mut menu = menu();
        menu.set_in_game(true, &loc);
        let commands: Vec<_> = menu.entries().iter().map(|e| e.command.clone()).collect();
        assert_eq!(commands.first(), Some(&MenuCommand::Resume));
        assert_eq!(commands.last(), Some(&MenuCommand::Quit));
        assert!(commands.contains(&MenuCommand::ReturnToMainMenu));
        assert!(!menu.back(&loc));
        menu.set_in_game(false, &loc);
        assert_eq!(menu.category(), MAIN_CATEGORY);
    }

    #[test]
    fn quit_box_uses_tokens_and_sdk_message_box_geometry() {
        let mut quit = quit_box(&loc());
        assert_eq!(quit.title, "QUIT GAME");
        assert_eq!(quit.text, "Do you wish to stop playing now?");
        assert_eq!(quit.cancel.as_deref(), Some("Cancel"));
        let measure = |s: &str| s.len() as f32 * 6.;
        let layout = quit.layout(measure);
        assert_eq!(layout.size, vec2(32. * 6. + 100., 116.));
        assert_eq!(layout.buttons.len(), 2);
        assert!(layout.buttons[0].x < layout.buttons[1].x);
        assert_eq!(layout.buttons[0].y + layout.buttons[0].h, 116. - 6. - 15.);
        assert!(
            (layout.text_at.x + 96. - layout.size.x / 2.).abs() <= 4.,
            "message centered"
        );
        let center = |r: Rect| Some(vec2(r.x + 2., r.y + 2.));

        // 1. Cancel button: press down -> pressed state active, no activation yet
        assert_eq!(
            quit.input(&layout, center(layout.buttons[1]), true, true, false, false),
            None
        );
        assert_eq!(quit.pressed, Some(1));

        // Release over Cancel button -> activates and returns Some(false)
        assert_eq!(
            quit.input(
                &layout,
                center(layout.buttons[1]),
                false,
                false,
                true,
                false
            ),
            Some(false)
        );
        assert_eq!(quit.pressed, None);

        // 2. OK button: press down and release
        assert_eq!(
            quit.input(&layout, center(layout.buttons[0]), true, true, false, false),
            None
        );
        assert_eq!(quit.pressed, Some(0));
        assert_eq!(
            quit.input(
                &layout,
                center(layout.buttons[0]),
                false,
                false,
                true,
                false
            ),
            Some(true)
        );
        assert_eq!(quit.pressed, None);

        // 3. Drag off: the press captures the mouse, so the button stays pressed while the
        // pointer is away, and releasing away does not trigger.
        assert_eq!(
            quit.input(&layout, center(layout.buttons[0]), true, true, false, false),
            None
        );
        assert_eq!(quit.pressed, Some(0));
        assert_eq!(quit.input(&layout, None, true, false, false, false), None);
        assert_eq!(quit.pressed, Some(0), "captured press survives drag-off");
        assert_eq!(quit.input(&layout, None, false, false, true, false), None);
        assert_eq!(quit.pressed, None);
        // Dragging off onto the other button neither arms it nor moves the press; dragging
        // back and releasing over the pressed button activates it.
        quit.input(&layout, center(layout.buttons[0]), true, true, false, false);
        quit.input(
            &layout,
            center(layout.buttons[1]),
            true,
            false,
            false,
            false,
        );
        assert_eq!((quit.armed, quit.pressed), (None, Some(0)));
        quit.input(
            &layout,
            center(layout.buttons[0]),
            true,
            false,
            false,
            false,
        );
        assert_eq!((quit.armed, quit.pressed), (Some(0), Some(0)));
        assert_eq!(
            quit.input(
                &layout,
                center(layout.buttons[0]),
                false,
                false,
                true,
                false
            ),
            Some(true)
        );

        // 4. Enter accepts immediately
        assert_eq!(
            quit.input(&layout, None, false, false, false, true),
            Some(true),
            "Enter accepts"
        );
    }

    #[test]
    fn return_to_main_menu_multiline_layout() {
        let loc = Localization::default();
        let msg_box = return_to_main_menu_box(&loc);
        assert_eq!(
            msg_box.text,
            "Are you sure?\nAll unsaved progress will be lost."
        );
        let measure = |s: &str| s.len() as f32 * 6.;
        let layout = msg_box.layout(measure);
        // Height expands for 2 lines: 2 * FONT_TALL (16) + 100 = 132
        assert_eq!(layout.size.y, 132.);
    }

    #[test]
    fn native_menu_uses_480_coordinate_scaling_with_proportional_rows() {
        let layout = MenuLayout::default();
        let at_720 = layout.geometry(720.);
        assert_eq!(at_720.title, vec2(114., 217.));
        assert_eq!(at_720.title2, vec2(114., 258.));
        assert_eq!(at_720.menu, vec2(114., 360.));
        assert_eq!(at_720.title_tall, 48.);
        assert_eq!(at_720.item_tall, 39.);
        assert_eq!(at_720.font_tall, 16.);
        let at_480 = layout.geometry(480.);
        assert_eq!(at_480.title, vec2(76., 145.));
        assert_eq!(at_480.title2, vec2(76., 172.));
        assert_eq!(at_480.menu, vec2(76., 240.));
        assert_eq!(at_480.font_tall, 16.);
        assert_eq!(at_480.title_tall, 32.);
        assert_eq!(at_480.item_tall, 26.);
        // Above the ranged record's 1080 lines the unranged one scales (truncated).
        assert_eq!(layout.geometry(1440.).font_tall, 33.);
        assert_eq!(layout.geometry(1440.).item_tall, 78.);

        let at_800 = layout.geometry(800.);
        assert_eq!(at_800.font_tall, 16.);
        assert_eq!(at_800.item_tall, 43.);
        assert_eq!(at_800.title, vec2(126., 241.));
        assert_eq!(at_800.title2, vec2(126., 286.));
        assert_eq!(at_800.menu, vec2(126., 400.));
        assert_eq!(at_800.title_tall, 53.);

        // High-DPI / Retina consistency: scale factor 2.0 at 800 logical height is 1600 lines.
        let at_retina = layout.geometry_scaled(800., 2.0);
        assert_eq!(at_retina, layout.geometry(1600.));
        assert_eq!(at_retina.font_tall, 36.);
        assert_eq!(at_retina.item_tall, 86.);

        // Verify row rects and text position helpers
        let rects = at_480.row_rects(3);
        assert_eq!(rects.len(), 3);
        assert_eq!(rects[0], Rect::new(76., 240., 250., 26.));
        assert_eq!(rects[1], Rect::new(76., 266., 250., 26.));
        assert_eq!(at_480.row_text_pos(0), vec2(76., 245.));
        let rects_800 = at_800.row_rects(2);
        assert_eq!(rects_800[0].h, 43.);
        assert_eq!(rects_800[1].h, 43.);
        assert_eq!(at_800.row_text_pos(0), vec2(126., 413.));
    }

    #[test]
    fn installed_scheme_sizes_the_menu_like_retail_at_1080_lines() {
        // hl2_complete resource/SourceScheme.res: one proportional GorDIN 12 record and
        // MainMenu.MenuItemHeight 22. Retail draws 27 px text on 49 px rows at 1920x1080 (an
        // owner capture of the native pause menu), where the platform base scheme's fixed
        // Windows record (16 up to 1080 lines) gave half-size text.
        let resource = source_assets::keyvalues::parse_resource(
            br#"Scheme {
            BaseSettings { MainMenu.MenuItemHeight 22 }
            Fonts { MenuLarge { 1 { name GorDIN tall 12 weight 600 } } }
        }"#,
        )
        .unwrap();
        let mut layout = MenuLayout::default();
        layout.apply_source_scheme(&resource[0]);
        let at_1080 = layout.geometry(1080.);
        assert_eq!((at_1080.font_tall, at_1080.item_tall), (27., 49.));
        let at_720 = layout.geometry(720.);
        assert_eq!((at_720.font_tall, at_720.item_tall), (18., 33.));

        // ReloadFontGlyphs takes the first record that fits the screen height, unscaled when it
        // has a range; MaxPropMenuItemHeight caps the scaled row height.
        let resource = source_assets::keyvalues::parse_resource(
            br#"Scheme {
            BaseSettings { MainMenu.MenuItemHeight 22 MainMenu.MaxPropMenuItemHeight 40 }
            Fonts { MenuLarge { 1 { tall 12 } 2 { tall 16 yres "1 1080" } } }
        }"#,
        )
        .unwrap();
        let mut layout = MenuLayout::default();
        layout.apply_source_scheme(&resource[0]);
        let at_1080 = layout.geometry(1080.);
        assert_eq!((at_1080.font_tall, at_1080.item_tall), (27., 40.));
    }

    #[test]
    fn wide_screens_grow_the_menu_upward_within_the_screen() {
        let layout = MenuLayout::default();
        let rows = crate::tuning::menu_stack_rows();
        // Up to 16:9 nothing changes (including 16:10 and 5:4).
        for canvas in [vec2(1280., 1024.), vec2(1680., 1050.), vec2(1920., 1080.)] {
            assert_eq!(
                layout.geometry_for(canvas),
                layout.geometry(canvas.y),
                "{canvas}"
            );
        }
        for canvas in [
            vec2(2560., 1080.),
            vec2(3440., 1440.),
            vec2(3840., 1080.),
            vec2(5120., 1440.),
            vec2(5120., 2160.),
        ] {
            let retail = layout.geometry(canvas.y);
            let wide = layout.geometry_for(canvas);
            assert!(wide.font_tall > retail.font_tall, "{canvas}: larger text");
            assert!(wide.item_tall > retail.item_tall, "{canvas}: larger rows");
            let bottom = |g: &MenuGeometry| g.menu.y + rows * g.item_tall;
            assert!(
                (bottom(&wide) - bottom(&retail)).abs() < 1.,
                "{canvas}: bottom stays"
            );
            assert!(
                wide.title.y > 0. && wide.title.y < retail.title.y,
                "{canvas}: grows upward"
            );
            assert!(bottom(&wide) <= canvas.y, "{canvas}: inside the screen");
            assert!(wide.row_rect(0).x + wide.row_wide <= canvas.x);
        }
    }

    #[test]
    fn narrow_screens_keep_row_text_by_height_and_fit_the_hit_width() {
        let layout = MenuLayout::default();
        // Landscape from 5:4 up to 16:9: the same as the height-only geometry.
        for canvas in [vec2(1280., 1024.), vec2(1920., 1080.)] {
            assert_eq!(
                layout.geometry_for(canvas),
                layout.geometry(canvas.y),
                "{canvas}"
            );
        }
        // 9:16: titles and position by the design height, rows by the screen height.
        let canvas = vec2(1080., 1920.);
        let narrow = layout.geometry_for(canvas);
        let design = layout.geometry(864.);
        let rows = layout.geometry(1920.);
        assert_eq!(
            (narrow.title, narrow.menu, narrow.title_tall),
            (design.title, design.menu, design.title_tall)
        );
        assert_eq!(
            (narrow.font_tall, narrow.item_tall),
            (rows.font_tall, rows.item_tall)
        );
        assert!(
            narrow.font_tall > design.font_tall,
            "not shrunk with the width"
        );
        let hit = narrow.row_rect(0);
        assert_eq!(hit.w, canvas.x - 2. * narrow.menu.x, "limited to what fits");
        assert!(hit.x + hit.w <= canvas.x);
    }

    #[test]
    fn menu_resources_resolve_desktop_colors_ranges_and_reject_bad_heights() {
        use source_ui::canvas::WHITE;
        use source_ui::widgets::scheme_color;
        let resource = source_assets::keyvalues::parse_resource(br#"Scheme {
            Colors { White "255 255 255 255" } BaseSettings { MainMenu.TextColor White Main.Menu.X 76 }
            Fonts { MenuLarge { 1 { tall 16 yres "1 1080" } [$WINDOWS]
                1 { tall 24 yres "1 1080" } [$LINUX] 2 { tall 11 } 3 { tall NaN } } }
        }"#).unwrap();
        let root = &resource[0];
        assert_eq!(scheme_color(root, "MainMenu.TextColor"), Some(WHITE));
        assert_eq!(resource_number(root, "Main.Menu.X"), Some(76.));
        let ranges = resource_font(root, "MenuLarge").unwrap();
        #[cfg(target_os = "windows")]
        assert_eq!(ranges, vec![(16., Some((1., 1080.))), (11., None)]);
        #[cfg(target_os = "linux")]
        assert_eq!(ranges, vec![(24., Some((1., 1080.))), (11., None)]);
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        assert_eq!(ranges, vec![(11., None)]);
        assert!(resource_font(root, "Absent").is_none());
    }

    #[test]
    fn rollover_sound_eligibility_only_for_menu_buttons() {
        use source_ui::layout::ControlKind;
        use source_ui::widgets::WidgetKind;

        // Eligibility check on WidgetKind
        assert!(
            WidgetKind::MenuButton.has_rollover_sound(),
            "menu button yes"
        );
        assert!(
            !WidgetKind::DialogButton.has_rollover_sound(),
            "dialog button no"
        );
        assert!(!WidgetKind::RadioButton.has_rollover_sound(), "radio no");
        assert!(!WidgetKind::Dropdown.has_rollover_sound(), "dropdown no");
        assert!(!WidgetKind::TextInput.has_rollover_sound(), "text input no");

        // Eligibility check on ControlKind
        assert!(
            !ControlKind::Button.widget_kind().has_rollover_sound(),
            "dialog button no"
        );
        assert!(
            !ControlKind::RadioButton.widget_kind().has_rollover_sound(),
            "radio no"
        );
        assert!(
            !ControlKind::ComboBox.widget_kind().has_rollover_sound(),
            "dropdown no"
        );
        assert!(
            !ControlKind::TextEntry.widget_kind().has_rollover_sound(),
            "text input no"
        );

        // Eligibility check on MenuEntry
        let menu = menu();
        let main_entries = menu.entries();
        assert!(
            main_entries[0].has_rollover_sound(),
            "enabled menu button has rollover sound"
        );
        assert!(
            !main_entries[3].has_rollover_sound(),
            "spacer has no rollover sound"
        );
    }

    #[test]
    fn test_menu_title_and_subtitle_layout_alignment_across_scales_and_resolutions() {
        let layout = MenuLayout::default();

        // 1. Scale factor 1.0 (480p reference height)
        let geom_1x = layout.geometry_scaled(480., 1.0);
        assert_eq!(geom_1x.title.x, DEFAULT_TITLE_X);
        assert_eq!(geom_1x.title2.x, DEFAULT_TITLE2_X);
        assert_eq!(geom_1x.menu.x, DEFAULT_MENU_X);
        assert_eq!(
            geom_1x.title2.x, geom_1x.menu.x,
            "subtitle must align horizontally with menu button column at 1.0x"
        );
        assert_eq!(
            geom_1x.title.x, geom_1x.title2.x,
            "title and subtitle must align on x at 1.0x"
        );
        assert!(
            geom_1x.title.y < geom_1x.title2.y && geom_1x.title2.y < geom_1x.menu.y,
            "vertical ordering must be Title1 -> Title2 (subtitle) -> Menu"
        );

        // 2. Scale factor 2.0 (960p effective height)
        let geom_2x = layout.geometry_scaled(480., 2.0);
        assert_eq!(geom_2x.title.x, DEFAULT_TITLE_X * 2.0);
        assert_eq!(geom_2x.title2.x, DEFAULT_TITLE2_X * 2.0);
        assert_eq!(geom_2x.menu.x, DEFAULT_MENU_X * 2.0);
        assert_eq!(
            geom_2x.title2.x, geom_2x.menu.x,
            "subtitle must align horizontally with menu button column at 2.0x"
        );
        assert_eq!(
            geom_2x.title.x, geom_2x.title2.x,
            "title and subtitle must align on x at 2.0x"
        );
        assert!(
            geom_2x.title.y < geom_2x.title2.y && geom_2x.title2.y < geom_2x.menu.y,
            "vertical ordering must be Title1 -> Title2 -> Menu at 2.0x"
        );

        // 3. 1280x800 reference resolution (scale = 800 / 480 = 5/3)
        let geom_800p = layout.geometry(800.);
        let expected_x = (DEFAULT_TITLE2_X * (800. / BASE_SCREEN_HEIGHT)).trunc();
        assert_eq!(geom_800p.title2.x, expected_x);
        assert_eq!(geom_800p.menu.x, expected_x);
        assert_eq!(
            geom_800p.title2.x, geom_800p.menu.x,
            "subtitle must align horizontally with menu buttons at 1280x800"
        );
        assert_eq!(
            geom_800p.title.x, geom_800p.title2.x,
            "title and subtitle must align on x at 1280x800"
        );
        assert!(
            geom_800p.title.y < geom_800p.title2.y && geom_800p.title2.y < geom_800p.menu.y,
            "vertical ordering must be Title1 -> Title2 -> Menu at 800p"
        );

        // 4. Test scheme parsing: legacy 2004 HL2 ClientScheme with Title2.X = 291 must normalize to menu_x (76)
        let legacy_kv_text = r#"
            "Scheme"
            {
                "BaseSettings"
                {
                    "Main.Title1.X" "76"
                    "Main.Title1.Y" "145"
                    "Main.Title2.X" "291"
                    "Main.Title2.Y" "207"
                    "Main.Menu.X"   "76"
                    "Main.Menu.Y"   "240"
                }
            }
        "#;
        let legacy_entries = source_assets::keyvalues::parse_resource(legacy_kv_text.as_bytes())
            .expect("parse legacy kv");
        let mut legacy_layout = MenuLayout::default();
        legacy_layout.apply_client_scheme(&legacy_entries[0]);
        assert_eq!(
            legacy_layout.title2_x, legacy_layout.menu_x,
            "legacy Title2.X (291) must be normalized to menu_x (76)"
        );
        assert_eq!(legacy_layout.title2_y, DEFAULT_TITLE2_Y);
        let legacy_800p = legacy_layout.geometry(800.);
        assert_eq!(
            legacy_800p.title2.x, legacy_800p.menu.x,
            "geometry under legacy scheme must still align title2 with menu buttons"
        );

        // 5. Anniversary ClientScheme with Title2.X = 76 and Title2.Y = 172
        let anniv_kv_text = r#"
            "Scheme"
            {
                "BaseSettings"
                {
                    "Main.Title1.X" "76"
                    "Main.Title1.Y" "145"
                    "Main.Title2.X" "76"
                    "Main.Title2.Y" "172"
                    "Main.Menu.X"   "76"
                    "Main.Menu.Y"   "240"
                }
            }
        "#;
        let anniv_entries = source_assets::keyvalues::parse_resource(anniv_kv_text.as_bytes())
            .expect("parse anniv kv");
        let mut anniv_layout = MenuLayout::default();
        anniv_layout.apply_client_scheme(&anniv_entries[0]);
        assert_eq!(anniv_layout.title2_x, 76.);
        assert_eq!(anniv_layout.title2_y, 172.);
        let anniv_800p = anniv_layout.geometry(800.);
        assert_eq!(anniv_800p.title2.x, anniv_800p.menu.x);
    }
}
