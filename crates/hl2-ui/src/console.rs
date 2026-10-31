//! A bounded standalone console. Commands without a Rust implementation report an error.
pub(crate) mod background_debug;
mod chapter_dialog;
pub mod commands;
pub mod complete;
mod draw_menu;
mod draw_windows;
mod effect;
mod focus;
mod fonts;
mod loading;
mod scheme;
use fonts::load_ui_fonts;
mod menu_flow;
mod options_dialog;
mod panel;
mod video_panel;

use crate::gameui::{
    menu::{GameMenu, MenuLayout},
    options::OptionsDialog,
    resources::{self, fallback_game_menu, GameUiResources},
};
pub use effect::{Effect, GameMap};
use glam::{vec2, Vec2};
use serde::Serialize;
use source_assets::vpk::Vfs;
use source_ui::{
    canvas::{Canvas, Color, Texture2D, WHITE},
    console::panel::Style as ConsoleStyle,
    font::{FontFace, TextOutline},
    message_box::MessageBox,
    placement::Placement,
    shell::Shell,
    widgets::{scheme_color, ButtonTracker, Fonts, Painter, Theme},
    windows::WindowId,
};
use std::collections::BTreeMap;
use std::sync::Arc;

/// A dialog window over the base layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum Window {
    NewGame,
    Options,
    Console,
    /// A sub-dialog over a parent dialog (e.g. Video Advanced, Keyboard Advanced, Gamma, Credits).
    SubDialog,
    /// A modal confirmation / message box dialog (Quit, Return to menu, Video keep, Reset defaults).
    Confirmation,
}

impl WindowId for Window {
    const CONSOLE: Self = Window::Console;

    /// Whether this window is modal (captures focus until closed).
    fn is_modal(&self) -> bool {
        matches!(self, Window::SubDialog | Window::Confirmation)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum Mode {
    #[default]
    Gameplay,
    Pause,
    Console,
    MainMenu,
    NewGame,
    Options,
}

/// Action to perform upon confirming a message box dialog.
#[derive(Clone, Debug, PartialEq)]
pub enum ConfirmKind {
    Quit,
    ReturnToMainMenu,
    /// Confirm starting a chapter in developer commentary mode (`CommentaryExplanationDialog`).
    StartCommentary(GameMap),
    /// Keep an aspect ratio change, else revert it (counts down on [`Console::tick`]).
    Extension(crate::extensions::Confirmation),
}

pub struct Console {
    pub extensions: crate::extensions::Extensions,
    pub presentation: crate::extensions::Presentation,
    pub canvas: Canvas,
    /// The focused layer: a window ([`Mode::Options`], [`Mode::Console`]) or the base. Read it
    /// with [`Console::mode`], change it with [`Console::set_mode`].
    mode: Mode,
    /// What the windows are over: the main menu, the pause menu or the game.
    base: Mode,
    /// The window stack and focus (the Options dialog and the console), the console dialog,
    /// Escape held over a dialog, key repeat and the latest pointer.
    pub shell: Shell<Window>,
    pub cheats: bool,
    /// Where the quit / main-menu box was dragged (centered on opening).
    quit_placement: Placement,
    /// Names arguments complete from (installed maps, the loaded map's entities), set by the host.
    completion: complete::Context,
    font: Option<FontFace>,
    menu_font: Option<FontFace>,
    menu_metrics: CellMetrics,
    title_font: Option<FontFace>,
    title_face: Option<Arc<fontdue::Font>>,
    title_metrics: CellMetrics,
    lostcoast_title_font: Option<FontFace>,
    lostcoast_title_face: Option<Arc<fontdue::Font>>,
    lostcoast_title_metrics: CellMetrics,
    title: String,
    category_titles: BTreeMap<String, String>,
    menu_layout: MenuLayout,
    mono: Option<FontFace>,
    developer_font: Option<FontFace>,
    /// Installed GameUI resources (localization, menu structure, dialog layouts, key actions).
    gameui: Arc<GameUiResources>,
    /// The out-of-game menu (main menu and its HL2/Episode categories).
    main_menu: GameMenu,
    /// The in-game (pause) menu.
    pause_menu: GameMenu,
    pub(crate) confirm_box: Option<(MessageBox, ConfirmKind)>,
    pub(crate) closing_confirm_box: Option<(MessageBox, ConfirmKind)>,
    /// UI clock in seconds (real time), set every frame by [`Console::tick`].
    ui_clock: f64,
    /// Effects of calls that return none ([`Console::escape`]), handed out by the next
    /// [`Console::tick`] or [`Console::input`].
    deferred: Vec<Effect>,
    pub loading_dialog: Option<crate::gameui::loading::LoadingDialog>,
    /// Full-screen movie playback panel (`playvideo` / `playvideo_exitcommand`).
    pub video_panel: Option<crate::gameui::video::VideoPanelState>,
    /// The progress the map loader reports for [`Console::loading_dialog`].
    loading_progress: crate::gameui::loading::ProgressReport,
    pub background_transition: crate::gameui::loading::BackgroundTransitionState,
    /// The loaded map is a menu background: no HUD, and the main menu cannot be dismissed.
    background_level: bool,
    /// Currently loaded map, if any.
    pub loaded_map: Option<GameMap>,
    /// Chapter background for the gameplay map currently loading in [`Console::loading_dialog`].
    pub loading_background: Option<GameMap>,
    theme: Theme,
    widget_fonts: Fonts,
    style: Style,
    pub menu_tracker: ButtonTracker<usize>,
    last_menu_hover: Option<usize>,
    pub options: OptionsDialog,
    chapter_dialogs: BTreeMap<String, crate::gameui::new_game::Dialog>,
    chapter_game: Option<String>,

    /// `(game, image)` -> pre-blurred background (see `GameUiResources::background_images`).
    pub background_textures: BTreeMap<(String, String), Texture2D>,

    pub config: crate::config::Config,
    /// Map -> chapter of every game (menu backgrounds after leaving a map).
    chapter_index: crate::gameui::chapters::ChapterIndex,
}

/// Positive native font heights describe Windows character cells, not pixels per EM. Fonts
/// that `resource/FontInfo.kv` forces to freetype2 (GorDIN) use retail CLinuxFont cells instead;
/// faces with exact GDI cells (the menu titles) follow their VDMX table.
#[derive(Clone, Copy)]
pub(crate) struct CellMetrics {
    em_per_cell: f32,
    ascent_per_cell: f32,
    freetype: Option<source_ui::font::FreeTypeCell>,
    gdi: Option<source_ui::font::GdiCell>,
}
impl Default for CellMetrics {
    fn default() -> Self {
        Self {
            em_per_cell: 1.,
            ascent_per_cell: 0.8,
            freetype: None,
            gdi: None,
        }
    }
}
impl CellMetrics {
    pub(crate) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let face = ttf_parser::Face::parse(bytes, 0).ok()?;
        let units = f32::from(face.units_per_em());
        let (ascent, descent) = face.tables().os2.map_or_else(
            || {
                (
                    f32::from(face.ascender()),
                    f32::from(face.descender()).abs(),
                )
            },
            |table| {
                (
                    f32::from(table.windows_ascender()),
                    f32::from(table.windows_descender()).abs(),
                )
            },
        );
        let height = ascent + descent;
        if height <= 0. || units <= 0. {
            return None;
        }
        Some(Self {
            em_per_cell: units / height,
            ascent_per_cell: ascent / height,
            freetype: None,
            gdi: None,
        })
    }
    /// Retail CLinuxFont cells for a FontInfo.kv freetype2 face.
    pub(crate) fn freetype(bytes: &[u8]) -> Option<Self> {
        Some(Self {
            freetype: Some(source_ui::font::FreeTypeCell::from_bytes(bytes)?),
            ..Self::from_bytes(bytes)?
        })
    }
    /// Retail CWin32Font cells (GDI size from the VDMX table) for a GDI face.
    pub(crate) fn gdi(bytes: &[u8]) -> Option<Self> {
        Some(Self {
            gdi: Some(source_ui::font::GdiCell::from_bytes(bytes)?),
            ..Self::from_bytes(bytes)?
        })
    }
    /// The pixel size (EM) and baseline below the cell top for a requested `tall`.
    pub(crate) fn size_for(self, tall: f32) -> (f32, f32) {
        if let Some(cell) = self.freetype {
            cell.size_for(tall)
        } else if let Some(cell) = self.gdi {
            cell.size_for(tall)
        } else {
            (
                (tall * self.em_per_cell).max(1.),
                tall * self.ascent_per_cell,
            )
        }
    }
}

fn draw_cell_text_outlined(
    value: &str,
    origin: Vec2,
    tall: f32,
    font: Option<&FontFace>,
    metrics: CellMetrics,
    color: Color,
    outline: TextOutline,
) {
    let (em, ascent) = metrics.size_for(tall);
    let raster_size = em.round().clamp(1., 1024.);
    if let Some(font) = font {
        font.draw_baseline_outlined(
            value,
            origin + vec2(0., ascent),
            raster_size,
            em / raster_size,
            color,
            outline,
        );
    }
}

/// The menu colors from the scheme (the console's are the panel's).
struct Style {
    menu: Color,
    menu_armed: Color,
    panel: Color,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            menu: WHITE,
            menu_armed: Color::from_rgba(200, 200, 200, 255),
            panel: Color::from_rgba(132, 132, 132, 212),
        }
    }
}
impl Default for Console {
    fn default() -> Self {
        let config = crate::config::Config::default();
        let gameui = GameUiResources {
            game_menu: fallback_game_menu(),
            ..Default::default()
        };
        let main_menu = GameMenu::new(Vec::new(), Default::default(), &gameui.localization);
        let pause_menu = GameMenu::new(Vec::new(), Default::default(), &gameui.localization);
        let mut console = Self {
            canvas: Canvas::default(),
            mode: Mode::Gameplay,
            base: Mode::Gameplay,
            // `Frame.FocusTransitionEffectTime` of the installed desktop scheme.
            shell: Shell::new(
                0.3,
                crate::tuning::unfocused_alpha_factor(),
                crate::proportional::SCALING,
            ),
            cheats: false,
            quit_placement: Placement::default(),
            completion: complete::Context::default(),
            font: None,
            menu_font: None,
            menu_metrics: CellMetrics::default(),
            title_font: None,
            title_face: None,
            title_metrics: CellMetrics::default(),
            lostcoast_title_font: None,
            lostcoast_title_face: None,
            lostcoast_title_metrics: CellMetrics::default(),
            title: "HL2-RS".into(),
            category_titles: resources::fallback_category_titles(&gameui.localization),
            menu_layout: MenuLayout::default(),
            mono: None,
            developer_font: None,
            gameui: Arc::default(),
            main_menu,
            pause_menu,
            confirm_box: None,
            closing_confirm_box: None,
            ui_clock: 0.,
            deferred: Vec::new(),
            loading_dialog: None,
            loading_progress: Default::default(),
            background_transition: crate::gameui::loading::BackgroundTransitionState::Idle,
            background_level: false,
            video_panel: None,
            loaded_map: None,
            loading_background: None,
            theme: Theme::default(),
            widget_fonts: Fonts::default(),
            style: Style::default(),
            menu_tracker: ButtonTracker::new(),
            last_menu_hover: None,
            options: OptionsDialog::default(),
            chapter_dialogs: BTreeMap::new(),
            chapter_game: None,
            background_textures: BTreeMap::new(),
            extensions: Default::default(),
            presentation: Default::default(),
            config,
            chapter_index: Default::default(),
        };
        console.set_resources(gameui);
        console.log("HL2-RS developer console. Type help for implemented commands.");
        console.log("Other Source commands are not implemented in this runtime.");
        console
    }
}

impl Console {
    pub fn load(vfs: &Vfs, canvas: Canvas) -> Self {
        let fonts = load_ui_fonts(vfs, &canvas);
        let config = crate::config::Config::load_or_default();
        let mut console = Self {
            canvas: canvas.clone(),
            developer_font: fonts.developer,
            widget_fonts: Fonts {
                text: fonts.ui.clone(),
                bold: fonts.bold,
                marlett: fonts.marlett,
            },
            font: fonts.ui,
            menu_font: fonts.menu,
            mono: fonts.mono,
            config,
            ..Default::default()
        };
        let gameui = GameUiResources::load(vfs);
        for file in &gameui.missing {
            console.log(format!("GameUI resource unavailable: {file}"));
        }
        console.set_resources(gameui);
        // Saved dialog placements (checked against the canvas when each dialog opens).
        console.shell.panel.font = console.mono.clone();
        console.shell.panel.placement = Placement::from_saved(console.config.dialogs.console);
        console
            .options
            .set_placement(Placement::from_saved(console.config.dialogs.options));
        console.menu_metrics = fonts.menu_metrics;
        fonts::load_title_fonts(&mut console, vfs, &canvas);
        console.load_scheme(vfs);
        console
    }

    /// Replaces the GameUI resources and rebuilds both game menus from them.
    pub fn set_resources(&mut self, mut resources: GameUiResources) {
        self.extensions.prepare_resources(&mut resources);
        let loc = &resources.localization;
        self.main_menu = GameMenu::new(
            resources.game_menu.clone(),
            resources.new_game_maps.clone(),
            loc,
        );
        self.pause_menu = GameMenu::new(
            resources.game_menu.clone(),
            resources.new_game_maps.clone(),
            loc,
        );
        self.pause_menu.set_in_game(true, loc);
        self.category_titles = resources.category_titles.clone();
        self.background_textures.clear();
        for (key, img) in &resources.background_images {
            self.background_textures.insert(
                key.clone(),
                Texture2D::from_rgba8(img.width, img.height, &img.rgba),
            );
        }
        self.prepare_chapters(&resources);
        self.gameui = Arc::new(resources);
        self.options.set_resources(self.gameui.clone());
    }

    pub fn resources(&self) -> &GameUiResources {
        &self.gameui
    }

    /// A modal dialog is up: the quit / return confirmation box or a game loading dialog. It
    /// takes every click; the Options dialog and the console are not modal (see [`Windows`]).
    /// The canvas size in pixels.
    fn canvas_size(&self) -> Vec2 {
        vec2(self.canvas.width(), self.canvas.height())
    }
    /// Called before gameplay input collection. Opening characters are drained by the host.
    /// Advances the UI clock to `now` (real seconds), once per frame whatever drives the input
    /// (scripted replays too): a keep-video confirmation counts down and reverts at its timeout.
    pub fn tick(&mut self, now: f64) -> Vec<Effect> {
        let dt = if self.ui_clock > 0.0 {
            (now - self.ui_clock).max(0.0) as f32
        } else {
            0.0
        };
        self.ui_clock = now;
        self.shell.windows.set_time(now);
        let video_effects = self.tick_video(dt);
        self.deferred.extend(video_effects);
        if let Some((_, ConfirmKind::Extension(extension))) = &self.confirm_box {
            let extension = extension.clone();
            if extension.0.expired(now) {
                self.closing_confirm_box = self.confirm_box.take();
                self.reset_menu_interaction();
                let effects = extension.0.cancel(self);
                self.deferred.extend(effects);
            } else if let Some((confirm, _)) = &mut self.confirm_box {
                confirm.set_text(extension.0.message(&self.gameui.localization, now).text);
            }
        }
        self.sync_dialog_windows();
        std::mem::take(&mut self.deferred)
    }

    pub fn commands(&self) -> &[commands::Spec] {
        self.completion.commands()
    }
    pub fn set_commands(&mut self, entries: Arc<[commands::Spec]>) {
        self.completion.commands = entries;
        self.shell.panel.editor.refresh(&self.completion);
    }
    fn painter(&self, origin: Vec2, scale: f32) -> Painter<'_> {
        Painter {
            canvas: &self.canvas,
            fonts: &self.widget_fonts,
            theme: &self.theme,
            origin,
            scale,
        }
    }
    pub fn overlay_font(&self) -> Option<&FontFace> {
        self.developer_font
            .as_ref()
            .or(self.mono.as_ref())
            .or(self.font.as_ref())
    }
    pub fn focus_base(&mut self) {
        self.shell.windows.focus_base();
        self.sync_mode();
    }
    pub fn clock(&self) -> f64 {
        self.ui_clock
    }
    pub fn open_extension_confirmation(&mut self, extension: crate::extensions::Confirmation) {
        self.reset_menu_interaction();
        self.quit_placement.reset();
        let message = extension
            .0
            .message(&self.gameui.localization, self.ui_clock);
        self.confirm_box = Some((message, ConfirmKind::Extension(extension)));
        self.sync_dialog_windows();
    }

    pub fn current_background(&self) -> Option<(&str, &str)> {
        let menu_cat = self.main_menu.category();
        self.upcoming_background(menu_cat)
    }

    fn upcoming_background<'a>(&'a self, menu_cat: &'a str) -> Option<(&'a str, &'a str)> {
        let transition = &self.background_transition;
        transition
            .target_category()
            .zip(transition.target_map())
            .or_else(|| {
                self.loaded_map
                    .as_ref()
                    .map(|m| (m.game.as_str(), m.map.as_str()))
            })
            .or_else(|| {
                let game = self.gameui.category_game(menu_cat);
                self.gameui
                    .background_maps
                    .get(game)
                    .map(|map| (game, map.as_str()))
            })
    }

    fn draw_menu_background(&self, shown: Mode, time: f64) {
        let menu_cat = if shown == Mode::Pause {
            self.pause_menu.category()
        } else {
            self.main_menu.category()
        };
        let is_widescreen = self.canvas.width() / self.canvas.height().max(1.0) > 4. / 3. + 1e-3;
        let upcoming = self.upcoming_background(menu_cat);
        let bg_tex =
            upcoming.and_then(|(game, map)| self.background_texture(game, map, is_widescreen));
        if let Some(loading) = &self.loading_dialog {
            if loading.is_background() {
                loading.draw_background_overlay(&self.canvas, bg_tex);
            }
        } else if self.background_transition.is_active() {
            let (w, h) = (self.canvas.width(), self.canvas.height());
            if let Some(tex) = bg_tex {
                crate::gameui::loading::draw_background_texture(&self.canvas, tex);
            }
            self.canvas.rectangle(
                0.,
                0.,
                w,
                h,
                crate::gameui::loading::BackgroundTransitionState::MAIN_MENU_BACKDROP,
            );
        } else {
            if let Some(tex) = bg_tex {
                let map_ready = self.background_level && !self.background_transition.is_failed();
                let alpha = if map_ready && self.presentation.capture_menu_navigation {
                    (self.presentation.background_opacity.unwrap_or(100) as f32 / 100.0)
                        .clamp(0.0, 1.0)
                } else if !map_ready {
                    1.0
                } else {
                    0.0
                };
                if alpha > 0.001 {
                    crate::gameui::loading::draw_background_texture_alpha(&self.canvas, tex, alpha);
                }
            }
            let dim_alpha = self.dim_overlay_alpha(time);
            if dim_alpha > 0.001 {
                let (w, h) = (self.canvas.width(), self.canvas.height());
                self.canvas
                    .rectangle(0., 0., w, h, Color::new(0., 0., 0., dim_alpha));
            }
        }
    }

    pub fn draw(&self, time: f64) {
        if self.draw_video() || self.draw_loading_screen() {
            return;
        }
        let shown = self.base;
        let menu_shown =
            shown == Mode::Pause || shown == Mode::MainMenu || self.is_background_loading();
        if menu_shown {
            self.draw_menu_background(shown, time);
            if self.is_background_loading() {
                self.draw_windows(time);
                return;
            }
            let geometry = self.menu_layout.geometry_for(self.canvas_size());
            let base_focus = self.shell.windows.base_focus_amount(time);
            let unfocused_alpha = crate::tuning::unfocused_alpha_factor();
            let alpha_mult = unfocused_alpha + (1.0 - unfocused_alpha) * base_focus;

            self.draw_menu_titles(geometry, alpha_mult, base_focus);
            self.draw_menu_buttons(shown, geometry, alpha_mult, base_focus);
        }
        self.draw_windows(time);
    }
}

fn vector(args: &[String]) -> Option<glam::Vec3> {
    if !(2..=3).contains(&args.len()) {
        return None;
    }
    let values: Vec<f32> = args
        .iter()
        .map(|v| v.parse())
        .collect::<Result<_, _>>()
        .ok()?;
    values
        .iter()
        .all(|v| v.is_finite())
        .then(|| glam::Vec3::new(values[0], values[1], values.get(2).copied().unwrap_or(0.)))
}
#[cfg(test)]
mod background_debug_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod button_padding_tests;

#[cfg(test)]
mod snapshot_tests;

#[cfg(test)]
mod background_blur_tests;
