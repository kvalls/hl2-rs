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
use fonts::load_ui_fonts;
mod menu_flow;
mod options_dialog;
mod panel;
mod video_panel;

use crate::gameui::{
    menu::{GameMenu, MenuLayout},
    options::OptionsDialog,
    resources::{self, fallback_game_menu, GameUiResources},
    video_confirm::VideoConfirm,
};
pub use effect::{Effect, GameMap};
use glam::{vec2, Vec2};
use serde::Serialize;
use source_assets::{keyvalues, vpk::Vfs};
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
    KeepVideo(VideoConfirm),
}

pub struct Console {
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
    completion: complete::Sources,
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
    pub background_debug_mode: bool,
    pub background_blur_opacity: u8,
    pub config: crate::config::Config,
    /// Map -> chapter of every game (menu backgrounds after leaving a map).
    chapter_index: crate::gameui::chapters::ChapterIndex,
}

/// Positive native font heights describe Windows character cells, not pixels per EM.
#[derive(Clone, Copy)]
pub(crate) struct CellMetrics {
    em_per_cell: f32,
    ascent_per_cell: f32,
}
impl Default for CellMetrics {
    fn default() -> Self {
        Self {
            em_per_cell: 1.,
            ascent_per_cell: 0.8,
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
        })
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
    let em = (tall * metrics.em_per_cell).max(1.);
    let raster_size = em.round().clamp(1., 1024.);
    if let Some(font) = font {
        font.draw_baseline_outlined(
            value,
            origin + vec2(0., tall * metrics.ascent_per_cell),
            raster_size,
            em / raster_size,
            color,
            outline,
        );
    }
}

fn text(
    _canvas: &Canvas,
    value: &str,
    origin: Vec2,
    em: f32,
    scale: f32,
    font: Option<&FontFace>,
    color: Color,
) {
    if let Some(font) = font {
        font.draw_baseline(value, origin, em, scale, color);
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
            completion: complete::Sources::default(),
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
            background_debug_mode: false,
            background_blur_opacity: 50,
            config,
            chapter_index: Default::default(),
        };
        console.set_resources(gameui);
        console.log("HL2-RS developer console. Type help for implemented commands.");
        console.log("Other Source commands are not implemented in this runtime.");
        console
    }
}

/// Height of the F1 developer overlay's header.
const DEBUG_HEADER_HEIGHT: f32 = 105.;

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
        // Installed desktop GameUI colors. Layout/window controls remain an initial implementation.
        let scheme_reader = crate::gameui::resources::Reader::new(vfs);
        if let Some(root) = scheme_reader
            .read("resource/SourceSchemeBase.res")
            .and_then(|bytes| keyvalues::parse_resource(&bytes).ok())
            .and_then(|roots| roots.into_iter().next())
        {
            console.theme = Theme::from_scheme(&root);
            if let Some(time) = root
                .get("BaseSettings")
                .and_then(|e| e.get("Frame.FocusTransitionEffectTime"))
                .and_then(|e| e.text())
                .and_then(|t| t.trim().parse::<f32>().ok())
            {
                console.shell.windows.fade_time = time;
            }
            console.style = Style {
                menu: scheme_color(&root, "MainMenu.TextColor").unwrap_or(console.style.menu),
                menu_armed: scheme_color(&root, "MainMenu.ArmedTextColor")
                    .unwrap_or(console.style.menu_armed),
                panel: scheme_color(&root, "Frame.BgColor").unwrap_or(console.style.panel),
            };
            let style = &mut console.shell.panel.style;
            *style = ConsoleStyle {
                entry: scheme_color(&root, "RichText.BgColor").unwrap_or(style.entry),
                text: scheme_color(&root, "Console.TextColor").unwrap_or(style.text),
                dev_text: scheme_color(&root, "Console.DevTextColor").unwrap_or(style.dev_text),
                ..*style
            };
            console.menu_layout.apply_source_scheme(&root);
        }
        // The console and history colors from the installed source scheme (the base scheme it
        // reads above is empty in the installed game).
        if let Some(root) = scheme_reader
            .read("resource/SourceScheme.res")
            .and_then(|bytes| keyvalues::parse_resource(&bytes).ok())
            .and_then(|roots| roots.into_iter().next())
        {
            for (name, color) in [
                ("Console.TextColor", &mut console.shell.panel.style.text),
                (
                    "Console.DevTextColor",
                    &mut console.shell.panel.style.dev_text,
                ),
                ("RichText.BgColor", &mut console.shell.panel.style.entry),
                (
                    "RichText.SelectedBgColor",
                    &mut console.shell.panel.style.selected_bg,
                ),
                (
                    "RichText.SelectedTextColor",
                    &mut console.shell.panel.style.selected_text,
                ),
            ] {
                if let Some(value) = scheme_color(&root, name) {
                    *color = value;
                }
            }
        }
        if let Some(root) = scheme_reader
            .read("resource/ClientScheme.res")
            .and_then(|bytes| keyvalues::parse_resource(&bytes).ok())
            .and_then(|roots| roots.into_iter().next())
        {
            console.menu_layout.apply_client_scheme(&root);
        }
        console
    }

    /// Replaces the GameUI resources and rebuilds both game menus from them.
    pub fn set_resources(&mut self, resources: GameUiResources) {
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
        if let Some((confirm, ConfirmKind::KeepVideo(keep))) = &mut self.confirm_box {
            if keep.expired(now) {
                let previous = keep.previous;
                self.closing_confirm_box = self.confirm_box.take();
                self.reset_menu_interaction();
                let effect = self.revert_video(previous);
                self.deferred.push(effect);
            } else {
                confirm.set_text(VideoConfirm::text(
                    &self.gameui.localization,
                    keep.remaining(now),
                ));
            }
        }
        self.sync_dialog_windows();
        std::mem::take(&mut self.deferred)
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
    /// Four compact header rows and a footer, preserving the retained host's developer layout.
    /// Font data is bundled under its own MIT license, independent of installed game content.
    /// Returns the height the overlay occupies at the top of the canvas.
    pub fn draw_debug(&self, lines: &[String]) -> f32 {
        self.canvas.rectangle(
            0.,
            0.,
            self.canvas.width(),
            DEBUG_HEADER_HEIGHT,
            Color::new(0.025, 0.045, 0.07, 0.88),
        );
        let rows = [
            (vec2(22., 30.), 27., WHITE),
            (vec2(22., 55.), 19., Color::new(0.784, 0.784, 0.784, 1.)),
            (vec2(22., 78.), 17., Color::new(0.784, 0.784, 0.784, 1.)),
            (vec2(22., 98.), 16., Color::from_rgba(240, 180, 90, 255)),
        ];
        let draw = |line: &str, origin: Vec2, size: f32, color| {
            if let Some(font) = &self.developer_font {
                font.draw_baseline(line, origin, size, 1., color);
            } else {
                text(
                    &self.canvas,
                    line,
                    origin,
                    size,
                    1.,
                    self.mono.as_ref().or(self.font.as_ref()),
                    color,
                );
            }
        };
        for (line, (origin, size, color)) in lines.iter().take(4).zip(rows) {
            draw(line, origin, size, color);
        }
        if let Some(footer) = lines.get(4) {
            self.canvas.rectangle(
                0.,
                self.canvas.height() - 38.,
                self.canvas.width(),
                38.,
                Color::new(0.025, 0.045, 0.07, 0.85),
            );
            draw(footer, vec2(20., self.canvas.height() - 13.), 18., WHITE);
        }
        DEBUG_HEADER_HEIGHT
    }

    fn draw_debug_lines_panel(&self, lines: &[String], top: f32, max_lines: usize) -> f32 {
        let count = lines.len().min(max_lines);
        let height = 20. + count as f32 * 22. + 10.;
        self.canvas.rectangle(
            0.,
            top,
            self.canvas.width(),
            height,
            Color::new(0.025, 0.045, 0.07, 0.9),
        );
        let draw = |line: &str, origin: Vec2, color| {
            if let Some(font) = &self.developer_font {
                font.draw_baseline(line, origin, 16., 1., color);
            } else {
                text(
                    &self.canvas,
                    line,
                    origin,
                    16.,
                    1.,
                    self.mono.as_ref().or(self.font.as_ref()),
                    color,
                );
            }
        };
        let normal = Color::new(0.784, 0.784, 0.784, 1.);
        let last_idx = count.saturating_sub(1);
        for (i, line) in lines.iter().take(count).enumerate() {
            let y = top + 20. + i as f32 * 22.;
            let color = if i == last_idx {
                Color::from_rgba(240, 180, 90, 255)
            } else if i == 0 {
                WHITE
            } else {
                normal
            };
            draw(line, vec2(22., y), color);
        }
        top + height
    }

    /// Draws the live viewmodel tuning values with their top edge at `top`; returns the bottom
    /// edge.
    pub fn draw_viewmodel_debug(&self, lines: &[String], top: f32) -> f32 {
        self.draw_debug_lines_panel(lines, top, 5)
    }

    /// Draws the live background visual debug values with their top edge at `top`; returns the bottom
    /// edge.
    pub fn draw_background_debug(&self, lines: &[String], top: f32) -> f32 {
        self.draw_debug_lines_panel(lines, top, lines.len())
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
                let alpha = if map_ready && self.background_debug_mode {
                    (self.background_blur_opacity as f32 / 100.0).clamp(0.0, 1.0)
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

/// Dialog push buttons play this when they activate.
const RELEASE_SOUND: &str = "ui/buttonclickrelease.wav";

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
mod viewmodel_tests;
