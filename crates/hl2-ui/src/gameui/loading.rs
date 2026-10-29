//! GameUI map loading dialog with real loader stage progress reporting.
//!
//! Layout matches retail Source `LoadingDialogNoBannerSingle.res` / `LoadingDialogNoBanner.res`:
//! - Centered frame (380 x 112) with title `#GameUI_Loading` ("LOADING..."), no close button.
//! - Info label (x: 20, y: 34, w: 340, h: 24) showing the current localized stage or loading text.
//! - Progress bar (x: 20, y: 64, w: 340, h: 24) with sunken depressed border, transparent background,
//!   and segmented chunks (width 8, gap 4, inset 4 per Source SDK `ProgressBar.cpp`).

use crate::gameui::localize::Localization;
use glam::Vec2;
use source_ui::{
    canvas::{Color, Rect},
    widgets::{FrameSpec, Painter},
};
use std::sync::{Arc, Mutex};

/// Dimensions of the loading dialog frame (380 x 112 per `LoadingDialogNoBanner.res`).
pub const LOADING_DIALOG_SIZE: Vec2 = Vec2::new(380., 112.);

/// A real loader progress report: fraction completed (0.0..=1.0) and stage identifier.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoadProgress {
    pub fraction: f32,
    pub stage: &'static str,
}

/// Where a map load reports its progress: the loader writes it from its thread, the console
/// that shows the loading dialog reads and resets it. Clones share one report.
#[derive(Clone, Debug, Default)]
pub struct ProgressReport(Arc<Mutex<Option<LoadProgress>>>);

impl ProgressReport {
    /// Reports loader progress (fraction clamped to 0..=1) from any thread.
    pub fn report(&self, fraction: f32, stage: &'static str) {
        if let Ok(mut lock) = self.0.lock() {
            *lock = Some(LoadProgress {
                fraction: fraction.clamp(0.0, 1.0),
                stage,
            });
        }
    }

    /// The latest report since the last reset.
    pub fn current(&self) -> Option<LoadProgress> {
        self.0.lock().ok().and_then(|lock| *lock)
    }

    /// Clears the report before a new map load and once a load is over.
    pub fn reset(&self) {
        if let Ok(mut lock) = self.0.lock() {
            *lock = None;
        }
    }
}

/// Kind of loading interface to display.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LoadingKind {
    /// Modal progress dialog (380x112 centered) for gameplay maps.
    #[default]
    Gameplay,
    /// Non-blocking bottom-right corner plaque with localized loading label for background maps.
    Background,
}

/// Dimensions of the background loading plaque (130 x 36 per Source VGUI `CLoadingDiscPanel`).
pub const BACKGROUND_LOADING_SIZE: Vec2 = Vec2::new(130., 36.);

/// Explicit state sequence for background map transitions.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum BackgroundTransitionState {
    /// No background transition is active.
    #[default]
    Idle,
    /// Background transition initiated for the target map and category.
    Starting {
        map: String,
        target_category: String,
        displayed_category: String,
    },
    /// Background loader is actively streaming/processing the target map.
    Loading {
        map: String,
        target_category: String,
        displayed_category: String,
        elapsed: f32,
    },
    /// Background map has successfully loaded; transition completed.
    Finished {
        map: String,
        displayed_category: String,
    },
    /// Background map loading failed; transition ended with error.
    Failed {
        map: String,
        displayed_category: String,
        reason: String,
    },
}

impl BackgroundTransitionState {
    /// Initiates a background map transition to the given map and category.
    pub fn start(&mut self, map: impl Into<String>, target_category: impl Into<String>) {
        let prev = self.current_displayed().to_owned();
        *self = Self::Starting {
            map: map.into(),
            target_category: target_category.into(),
            displayed_category: prev,
        };
    }

    /// Convenience starter when category is omitted.
    pub fn start_map(&mut self, map: impl Into<String>) {
        self.start(map, "");
    }

    /// Advances the transition state over time (dt in seconds).
    pub fn update(&mut self, dt: f32) {
        match self {
            Self::Starting {
                map,
                target_category,
                displayed_category,
            } => {
                *self = Self::Loading {
                    map: std::mem::take(map),
                    target_category: std::mem::take(target_category),
                    displayed_category: std::mem::take(displayed_category),
                    elapsed: dt.max(0.0),
                };
            }
            Self::Loading { elapsed, .. } => {
                *elapsed += dt.max(0.0);
            }
            Self::Finished { .. } | Self::Failed { .. } | Self::Idle => {}
        }
    }

    /// Completes the transition successfully, applying the target category as displayed.
    pub fn finish(&mut self) {
        match self {
            Self::Starting {
                map,
                target_category,
                ..
            }
            | Self::Loading {
                map,
                target_category,
                ..
            } => {
                let map = std::mem::take(map);
                let target = std::mem::take(target_category);
                *self = Self::Finished {
                    map,
                    displayed_category: target,
                };
            }
            _ => *self = Self::Idle,
        }
    }

    /// Whether `game`'s `map` is the target. A background transition's category is the game
    /// whose background loads; Half-Life 2 and Lost Coast both have a `background01`.
    fn targets(&self, game: &str, map: &str) -> bool {
        self.target_map() == Some(map) && self.target_category() == Some(game)
    }

    /// Completes the transition if `game`'s `map` is the target. Returns false if stale.
    pub fn finish_map(&mut self, game: &str, map: &str) -> bool {
        if self.targets(game, map) {
            self.finish();
            true
        } else {
            false
        }
    }

    /// Ends the transition due to a loading failure, still applying the target category.
    pub fn fail(&mut self, reason: impl Into<String>) {
        let (map, target) = match self {
            Self::Starting {
                map,
                target_category,
                ..
            }
            | Self::Loading {
                map,
                target_category,
                ..
            } => (std::mem::take(map), std::mem::take(target_category)),
            Self::Finished {
                map,
                displayed_category,
            }
            | Self::Failed {
                map,
                displayed_category,
                ..
            } => (std::mem::take(map), std::mem::take(displayed_category)),
            Self::Idle => (String::new(), "main".to_string()),
        };
        *self = Self::Failed {
            map,
            displayed_category: target,
            reason: reason.into(),
        };
    }

    /// Fails the transition if `game`'s `map` is the target. Returns false if stale.
    pub fn fail_map(&mut self, game: &str, map: &str, reason: impl Into<String>) -> bool {
        if self.targets(game, map) {
            self.fail(reason);
            true
        } else {
            false
        }
    }

    /// Resets the transition to idle.
    pub fn reset(&mut self) {
        *self = Self::Idle;
    }

    /// Whether a background transition is actively in progress (Starting or Loading).
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Starting { .. } | Self::Loading { .. })
    }

    /// Whether the transition has reached a terminal state (Finished or Failed).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Finished { .. } | Self::Failed { .. })
    }

    /// Whether the transition ended with a failure.
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }

    /// Whether the transition completed successfully.
    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Finished { .. })
    }

    /// Target map name if a transition is active or completed/failed.
    pub fn target_map(&self) -> Option<&str> {
        match self {
            Self::Starting { map, .. }
            | Self::Loading { map, .. }
            | Self::Finished { map, .. }
            | Self::Failed { map, .. } => Some(map.as_str()).filter(|s| !s.is_empty()),
            Self::Idle => None,
        }
    }

    /// Target expansion/submenu category being loaded.
    pub fn target_category(&self) -> Option<&str> {
        match self {
            Self::Starting {
                target_category, ..
            }
            | Self::Loading {
                target_category, ..
            } => Some(target_category.as_str()).filter(|s| !s.is_empty()),
            Self::Finished {
                displayed_category, ..
            }
            | Self::Failed {
                displayed_category, ..
            } => Some(displayed_category.as_str()).filter(|s| !s.is_empty()),
            Self::Idle => None,
        }
    }

    /// Returns the currently displayed category recorded in this transition state.
    pub fn current_displayed(&self) -> &str {
        match self {
            Self::Idle => "main",
            Self::Starting {
                displayed_category, ..
            }
            | Self::Loading {
                displayed_category, ..
            }
            | Self::Finished {
                displayed_category, ..
            }
            | Self::Failed {
                displayed_category, ..
            } => {
                if displayed_category.is_empty() {
                    "main"
                } else {
                    displayed_category.as_str()
                }
            }
        }
    }

    /// Returns the category to display for the given menu category.
    ///
    /// If the menu is at root `main`, "main" is always returned.
    /// While a transition is active, the previously displayed category is preserved so the
    /// old subtitle remains visible until the background load finishes or fails.
    /// When terminal (Finished or Failed), the target category is returned.
    /// If Idle, the requested menu_category is returned.
    pub fn displayed_category<'a>(&'a self, menu_category: &'a str) -> &'a str {
        if menu_category == "main" {
            return "main";
        }
        match self {
            Self::Starting {
                displayed_category, ..
            }
            | Self::Loading {
                displayed_category, ..
            } => {
                if displayed_category.is_empty() {
                    menu_category
                } else {
                    displayed_category.as_str()
                }
            }
            Self::Finished {
                displayed_category, ..
            }
            | Self::Failed {
                displayed_category, ..
            } => {
                if displayed_category.is_empty() {
                    menu_category
                } else {
                    displayed_category.as_str()
                }
            }
            Self::Idle => menu_category,
        }
    }

    /// Manually sets the displayed category (e.g. when an expansion background map was already loaded).
    pub fn set_displayed_category(&mut self, category: impl Into<String>) {
        let cat = category.into();
        match self {
            Self::Finished {
                displayed_category, ..
            } => *displayed_category = cat,
            Self::Failed {
                displayed_category, ..
            } => *displayed_category = cat,
            Self::Starting {
                displayed_category, ..
            }
            | Self::Loading {
                displayed_category, ..
            } => *displayed_category = cat,
            Self::Idle => {
                *self = Self::Finished {
                    map: String::new(),
                    displayed_category: cat,
                };
            }
        }
    }

    /// Authentic dimming overlay color (`rgba(0, 0, 0, 156)` per `sourcescheme.res` `MainMenu.Backdrop`).
    pub const MAIN_MENU_BACKDROP: Color = Color::from_rgba(0, 0, 0, 156);
    pub const BACKDROP_ALPHA: f32 = 156.0 / 255.0;

    /// Transition overlay dimming alpha (0.0 or 156/255 ~= 0.61 per `sourcescheme.res`).
    pub fn overlay_alpha(&self) -> f32 {
        if self.is_active() {
            Self::BACKDROP_ALPHA
        } else {
            0.0
        }
    }
}

/// State of an active map loading dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadingDialog {
    pub map: String,
    pub kind: LoadingKind,
    pub stage: Option<&'static str>,
    pub fraction: f32,
    pub status_override: Option<String>,
    pub error: Option<String>,
    pub complete: bool,
    pub transition: BackgroundTransitionState,
}

/// The part of a pre-blurred background texture to draw on a `canvas`, in texture pixels
/// (`texture` is its size). The 4:3 texture on screens narrower than 16:10 keeps the full height
/// and crops the sides when narrower than 4:3; the 16:9 widescreen texture crops the sides on 16:10,
/// and crops the top and bottom on screens wider than 16:9 ([`crate::tuning::widest_design_aspect`]),
/// so the blurred image does not stretch relative to the background camera's 3D projection.
pub fn background_source(canvas: glam::Vec2, texture: glam::Vec2) -> source_ui::canvas::Rect {
    let aspect = canvas.x / canvas.y.max(1.);
    let tex_aspect = texture.x / texture.y.max(1.);
    let base_aspect = if tex_aspect >= 1.5 {
        16. / 9.
    } else if tex_aspect > 1.05 {
        4. / 3.
    } else if aspect > 4. / 3. + 1e-3 {
        16. / 9.
    } else {
        4. / 3.
    };
    let widest = crate::tuning::widest_design_aspect();
    if aspect < base_aspect {
        let kept = aspect / base_aspect;
        source_ui::canvas::Rect::new(
            texture.x * (1. - kept) * 0.5,
            0.,
            texture.x * kept,
            texture.y,
        )
    } else if aspect > widest {
        let kept = widest / aspect;
        source_ui::canvas::Rect::new(
            0.,
            texture.y * (1. - kept) * 0.5,
            texture.x,
            texture.y * kept,
        )
    } else {
        source_ui::canvas::Rect::new(0., 0., texture.x, texture.y)
    }
}

/// Draws a pre-blurred background texture over the whole canvas ([`background_source`]).
pub fn draw_background_texture(
    canvas: &source_ui::canvas::Canvas,
    texture: &source_ui::canvas::Texture2D,
) {
    draw_background_texture_alpha(canvas, texture, 1.0);
}

/// Draws a pre-blurred background texture with custom alpha opacity over the canvas.
pub fn draw_background_texture_alpha(
    canvas: &source_ui::canvas::Canvas,
    texture: &source_ui::canvas::Texture2D,
    alpha: f32,
) {
    let size = glam::vec2(canvas.width(), canvas.height());
    let color = source_ui::canvas::Color::new(1., 1., 1., alpha.clamp(0., 1.));
    source_ui::canvas::draw_texture_ex(
        canvas,
        texture,
        0.,
        0.,
        color,
        source_ui::canvas::DrawTextureParams {
            source: Some(background_source(
                size,
                glam::vec2(texture.width(), texture.height()),
            )),
            dest_size: Some(size),
        },
    );
}

impl LoadingDialog {
    /// Creates a new loading dialog for the given map name.
    pub fn new(map: impl Into<String>) -> Self {
        Self {
            map: map.into(),
            kind: LoadingKind::Gameplay,
            stage: None,
            fraction: 0.0,
            status_override: None,
            error: None,
            complete: false,
            transition: BackgroundTransitionState::Idle,
        }
    }

    /// Creates a new loading dialog for background map loading with a non-blocking corner plaque.
    pub fn new_background(map: impl Into<String>) -> Self {
        let map_str = map.into();
        Self {
            map: map_str.clone(),
            kind: LoadingKind::Background,
            stage: None,
            fraction: 0.0,
            status_override: None,
            error: None,
            complete: false,
            transition: BackgroundTransitionState::Starting {
                map: map_str,
                target_category: String::new(),
                displayed_category: String::new(),
            },
        }
    }

    /// Whether this dialog is for a background map load.
    pub fn is_background(&self) -> bool {
        self.kind == LoadingKind::Background
    }

    /// Updates progress from the latest report. Ignored once failed or complete.
    pub fn update(&mut self, progress: Option<LoadProgress>) {
        if self.error.is_some() || self.complete {
            return;
        }
        if self.is_background() {
            self.transition.update(1.0 / 60.0);
        }
        if let Some(p) = progress {
            self.fraction = p.fraction.clamp(0.0, 1.0);
            self.stage = Some(p.stage);
        }
    }

    /// Sets a failure error message. Locks the dialog in the failed state.
    pub fn set_error(&mut self, error: impl Into<String>) {
        let err = error.into();
        if self.is_background() {
            self.transition.fail(&err);
        }
        self.error = Some(err);
    }

    /// Marks the loading operation as complete.
    pub fn set_complete(&mut self) {
        if self.is_background() {
            self.transition.finish();
        }
        self.fraction = 1.0;
        self.complete = true;
    }

    /// Whether the loading operation has failed.
    pub fn is_failed(&self) -> bool {
        self.error.is_some()
    }

    /// Whether the loading operation has completed.
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// Resolves the current status text to display above the progress bar.
    pub fn status_text(&self, loc: &Localization) -> String {
        if let Some(err) = &self.error {
            return err.clone();
        }
        if let Some(custom) = &self.status_override {
            return custom.clone();
        }
        if let Some(stage) = &self.stage {
            loc.text(stage)
        } else {
            let template = loc.text("#GameUI_LoadingFilename");
            if template.contains("%s1") {
                template.replace("%s1", &self.map)
            } else {
                format!("{template} {}", self.map)
            }
        }
    }

    /// Returns the origin and scale for centering a dialog with the given size on the canvas.
    pub fn layout_geometry_sized(canvas_size: Vec2, scale: f32, dialog_size: Vec2) -> (Vec2, f32) {
        let scaled_size = dialog_size * scale;
        let origin = Vec2::new(
            ((canvas_size.x - scaled_size.x) * 0.5).max(0.0).round(),
            ((canvas_size.y - scaled_size.y) * 0.5).max(0.0).round(),
        );
        (origin, scale)
    }

    /// Returns the origin and scale for centering the loading dialog on the canvas.
    pub fn layout_geometry(canvas_size: Vec2, scale: f32) -> (Vec2, f32) {
        Self::layout_geometry_sized(canvas_size, scale, LOADING_DIALOG_SIZE)
    }

    /// Returns the origin and scale for placing the background loading plaque in the bottom-right corner.
    pub fn background_plaque_geometry(canvas_size: Vec2, scale: f32) -> (Vec2, f32) {
        let scaled_size = BACKGROUND_LOADING_SIZE * scale;
        let margin = 24.0 * scale;
        let origin = Vec2::new(
            (canvas_size.x - scaled_size.x - margin).max(0.0).round(),
            (canvas_size.y - scaled_size.y - margin).max(0.0).round(),
        );
        (origin, scale)
    }

    /// Draws the non-blocking background loading plaque in the bottom-right corner.
    fn draw_background_plaque(&self, painter: &Painter, loc: &Localization) {
        let (origin, scale) = Self::background_plaque_geometry(
            Vec2::new(painter.canvas.width(), painter.canvas.height()),
            painter.scale,
        );
        let p = Painter {
            canvas: painter.canvas,
            fonts: painter.fonts,
            theme: painter.theme,
            origin: painter.origin + origin,
            scale,
        };
        // Routed through the shared dialog frame (FrameSpec with FrameStyle::DEFAULT: rounded corners, no outer 3D border)
        // Translucent dark plaque body matching Source VGUI CLoadingDiscPanel (Color(0, 0, 0, 140))
        let plaque_spec = FrameSpec::new(BACKGROUND_LOADING_SIZE, "")
            .without_close()
            .with_bg_color(Color::new(0.0, 0.0, 0.0, 0.65));
        p.frame_spec(plaque_spec, None);

        // Center localized `#GameUI_Loading` ("LOADING...") text within plaque
        let title = loc.text("#GameUI_Loading");
        let text_w = p.text_width(&title, false);
        let font_tall = source_ui::widgets::FONT_TALL;
        let text_x = ((BACKGROUND_LOADING_SIZE.x - text_w) * 0.5)
            .max(4.0)
            .round();
        let text_y = ((BACKGROUND_LOADING_SIZE.y - font_tall) * 0.5)
            .max(2.0)
            .round();
        p.text(&title, Vec2::new(text_x, text_y), p.theme.text, false);
    }

    /// Draws the backdrop overlay for background map loading across the canvas.
    /// In retail Source, this displays the pre-blurred background texture for the target expansion
    /// combined with the authentic `MainMenu.Backdrop` dimming (`rgba(0, 0, 0, 156)` per `sourcescheme.res`).
    /// Never applies to gameplay map loading.
    pub fn draw_background_overlay(
        &self,
        canvas: &source_ui::canvas::Canvas,
        bg_texture: Option<&source_ui::canvas::Texture2D>,
    ) {
        if !self.is_background() || !self.transition.is_active() {
            return;
        }
        let (w, h) = (canvas.width(), canvas.height());
        if let Some(tex) = bg_texture {
            draw_background_texture(canvas, tex);
        }
        canvas.rectangle(0., 0., w, h, BackgroundTransitionState::MAIN_MENU_BACKDROP);
    }

    /// Draws the loading dialog centered within the painter's canvas bounds.
    pub fn draw(&self, painter: &Painter, loc: &Localization) {
        if self.is_background() {
            if self.transition.is_active() {
                self.draw_background_plaque(painter, loc);
            }
            return;
        }
        let label_text = self.status_text(loc);
        let text_color = if self.error.is_some() {
            Color::new(1.0, 0.35, 0.35, 1.0)
        } else {
            painter.theme.text
        };
        // Wrap status/warning text to 340px width (parent dialog width 380 - 20px padding each side)
        let max_label_w = 340.0;
        let lines =
            source_ui::widgets::wrap(&label_text, max_label_w, |s| painter.text_width(s, false));
        let line_count = lines.len().max(1);

        // Dynamically expand dialog height and shift progress bar if text wraps beyond 2 lines
        let font_tall = source_ui::widgets::FONT_TALL;
        let label_h = (line_count as f32 * font_tall).max(24.0);
        let bar_y = 34.0 + label_h + 6.0;
        let dialog_h = (bar_y + 24.0 + 16.0).max(LOADING_DIALOG_SIZE.y);
        let dialog_size = Vec2::new(LOADING_DIALOG_SIZE.x, dialog_h);

        let (origin, scale) = Self::layout_geometry_sized(
            Vec2::new(painter.canvas.width(), painter.canvas.height()),
            painter.scale,
            dialog_size,
        );
        let p = Painter {
            canvas: painter.canvas,
            fonts: painter.fonts,
            theme: painter.theme,
            origin: painter.origin + origin,
            scale,
        };

        let title = loc.text("#GameUI_Loading");
        p.frame_with_close(dialog_size, &title, None);

        // Render each line inside the label area
        let mut y = 34.0;
        for line in &lines {
            p.text(line, Vec2::new(20., y), text_color, false);
            y += font_tall;
        }

        // Progress bar: positioned below the label
        let bar_rect = Rect::new(20., bar_y, 340., 24.);
        let bar_bg = Color::new(0.0, 0.0, 0.0, 0.6);
        let bar_fg = p.theme.text;
        p.progress_bar(bar_rect, self.fraction, bar_fg, bar_bg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_overlay_draws_the_cropped_texture_over_the_whole_screen() {
        use source_ui::canvas::{Canvas, Texture2D};
        let texture = Texture2D::from_rgba8(2, 2, &[255; 16]);
        let canvas = Canvas::default();
        for (w, h, source_w, source_h) in [
            (1080., 1920., 2. * 0.5625 * 0.75, 2.),
            (1280., 1024., 2. * (5. / 4.) / (4. / 3.), 2.),
            (5120., 1440., 2., 1.),
        ] {
            canvas.resize(w, h);
            draw_background_texture(&canvas, &texture);
            let quads = canvas.drain();
            assert_eq!(quads.len(), 1);
            assert_eq!(
                quads[0].destination,
                source_ui::canvas::Rect::new(0., 0., w, h)
            );
            assert!(
                (quads[0].source.w - source_w).abs() < 1e-3
                    && (quads[0].source.h - source_h).abs() < 1e-3,
                "{w}x{h}: {:?}",
                quads[0].source
            );
        }
    }

    #[test]
    fn narrow_and_ultrawide_screens_fill_with_the_centre_of_the_background() {
        use glam::vec2;
        let texture = vec2(512., 512.);
        for canvas in [vec2(1024., 768.), vec2(1920., 1080.)] {
            assert_eq!(
                background_source(canvas, texture),
                source_ui::canvas::Rect::new(0., 0., 512., 512.),
                "{canvas}: whole texture"
            );
        }
        let five_four = background_source(vec2(1280., 1024.), texture);
        assert!((five_four.x - 16.).abs() < 1e-3 && (five_four.w - 480.).abs() < 1e-3);
        assert_eq!(five_four.h, 512.);

        let sixteen_ten = background_source(vec2(1920., 1200.), texture);
        assert!((sixteen_ten.x - 25.6).abs() < 1e-3 && (sixteen_ten.w - 460.8).abs() < 1e-3);
        assert_eq!(sixteen_ten.h, 512.);
        let square = background_source(vec2(1080., 1080.), texture);
        assert_eq!(square.h, 512.);
        assert!(
            (square.w - 384.).abs() < 1e-3,
            "{square:?}: 3/4 of the width"
        );
        assert!((square.x - 64.).abs() < 1e-3, "centred");
        let portrait = background_source(vec2(1080., 1920.), texture);
        assert!(
            (portrait.w - 512. * 0.5625 * 0.75).abs() < 1e-3,
            "{portrait:?}"
        );
        assert!(
            (portrait.x * 2. + portrait.w - 512.).abs() < 1e-3,
            "centred"
        );
        let ultrawide = background_source(vec2(3440., 1440.), texture);
        assert_eq!((ultrawide.x, ultrawide.w), (0., 512.));
        assert!(
            (ultrawide.h - 512. * (16. / 9.) / (3440. / 1440.)).abs() < 1e-3,
            "{ultrawide:?}"
        );
        assert!(
            (ultrawide.y * 2. + ultrawide.h - 512.).abs() < 1e-3,
            "centred"
        );
        let super_ultrawide = background_source(vec2(5120., 1440.), texture);
        assert_eq!(
            super_ultrawide,
            source_ui::canvas::Rect::new(0., 128., 512., 256.),
            "32:9 keeps the middle half of the 16:9 image"
        );
    }

    #[test]
    fn loading_dialog_initial_state_and_progress_updates() {
        let loc = Localization::default();
        let mut dialog = LoadingDialog::new("d1_trainstation_01");
        assert_eq!(dialog.map, "d1_trainstation_01");
        assert_eq!(dialog.fraction, 0.0);
        assert!(!dialog.is_failed());
        assert!(!dialog.is_complete());
        assert_eq!(dialog.status_text(&loc), "Loading d1_trainstation_01 ...");

        dialog.update(Some(LoadProgress {
            fraction: 0.35,
            stage: "LoadingProgress_PrecacheWorld",
        }));
        assert_eq!(dialog.fraction, 0.35);
        assert_eq!(dialog.stage, Some("LoadingProgress_PrecacheWorld"));
        assert_eq!(dialog.status_text(&loc), "Initializing world...");
    }

    #[test]
    fn loading_dialog_stages_fallback_and_localization() {
        let mut loc = Localization::default();
        loc.insert("#GameUI_LoadingFilename", "Chargement de %s1 ...");
        loc.insert("#LoadingProgress_LoadMap", "Chargement du monde...");

        let mut dialog = LoadingDialog::new("test_map");
        assert_eq!(dialog.status_text(&loc), "Chargement de test_map ...");

        dialog.update(Some(LoadProgress {
            fraction: 0.15,
            stage: "LoadingProgress_LoadMap",
        }));
        assert_eq!(dialog.status_text(&loc), "Chargement du monde...");

        dialog.update(Some(LoadProgress {
            fraction: 0.75,
            stage: "LoadingProgress_LoadResources",
        }));
        // Unlocalized token falls back to built-in English string
        assert_eq!(dialog.status_text(&loc), "Loading resources...");
    }

    #[test]
    fn loading_dialog_failure_state_handling() {
        let loc = Localization::default();
        let mut dialog = LoadingDialog::new("missing_map");
        dialog.update(Some(LoadProgress {
            fraction: 0.1,
            stage: "LoadingProgress_LoadMap",
        }));
        assert!(!dialog.is_failed());

        dialog.set_error("owned map missing_map.bsp not found");
        assert!(dialog.is_failed());
        assert_eq!(
            dialog.status_text(&loc),
            "owned map missing_map.bsp not found"
        );

        // Progress updates after failure are ignored
        dialog.update(Some(LoadProgress {
            fraction: 0.8,
            stage: "LoadingProgress_LoadResources",
        }));
        assert_eq!(dialog.fraction, 0.1);
        assert_eq!(
            dialog.status_text(&loc),
            "owned map missing_map.bsp not found"
        );
    }

    #[test]
    fn loading_dialog_completion_state() {
        let mut dialog = LoadingDialog::new("d1_trainstation_02");
        assert!(!dialog.is_complete());
        dialog.set_complete();
        assert!(dialog.is_complete());
        assert_eq!(dialog.fraction, 1.0);

        // Progress updates after completion are ignored
        dialog.update(Some(LoadProgress {
            fraction: 0.2,
            stage: "LoadingProgress_LoadMap",
        }));
        assert_eq!(dialog.fraction, 1.0);
    }

    #[test]
    fn progress_report_is_shared_by_clones_and_reset() {
        let report = ProgressReport::default();
        let loader = report.clone();
        assert_eq!(report.current(), None);

        loader.report(0.45, "LoadingProgress_SignonDataLocal");
        assert_eq!(
            report.current(),
            Some(LoadProgress {
                fraction: 0.45,
                stage: "LoadingProgress_SignonDataLocal",
            })
        );

        // Clamping check
        loader.report(1.5, "LoadingProgress_SignonLocal");
        assert_eq!(
            report.current(),
            Some(LoadProgress {
                fraction: 1.0,
                stage: "LoadingProgress_SignonLocal",
            })
        );

        report.reset();
        assert_eq!(loader.current(), None);
        // Another report is independent.
        assert_eq!(ProgressReport::default().current(), None);
    }

    #[test]
    fn loading_dialog_scaling_and_layout_geometry() {
        // Standard 720p base (scale = 1.0): size is 380x112, centered on 1280x720
        let (origin_720, scale_720) = LoadingDialog::layout_geometry(Vec2::new(1280., 720.), 1.0);
        assert_eq!(scale_720, 1.0);
        assert_eq!(origin_720, Vec2::new(450., 304.));

        // 1280x800 standard reference (scale = 800/720)
        let scale_800 = Painter::scale_for(800.);
        let (origin_800, scale) = LoadingDialog::layout_geometry(Vec2::new(1280., 800.), scale_800);
        assert_eq!(scale, 800. / 720.);
        let scaled_size = LOADING_DIALOG_SIZE * scale;
        assert_eq!(
            origin_800,
            Vec2::new(
                ((1280. - scaled_size.x) * 0.5).round(),
                ((800. - scaled_size.y) * 0.5).round()
            )
        );

        // Native Retina / High-DPI 2560x1600 (scale = 1600/720)
        let scale_retina = Painter::scale_for_window(800., 2.0);
        let (origin_retina, scale_ret) =
            LoadingDialog::layout_geometry(Vec2::new(2560., 1600.), scale_retina);
        assert_eq!(scale_ret, 1600. / 720.);
        let scaled_size_retina = LOADING_DIALOG_SIZE * scale_ret;
        assert_eq!(
            origin_retina,
            Vec2::new(
                ((2560. - scaled_size_retina.x) * 0.5).round(),
                ((1600. - scaled_size_retina.y) * 0.5).round()
            )
        );
    }

    #[test]
    fn loading_dialog_multiline_status_wraps_and_expands_bounds() {
        use source_ui::canvas::Canvas;
        use source_ui::widgets::{Fonts, Theme};

        let loc = Localization::default();
        let mut dialog = LoadingDialog::new("ep1_citadel_00");
        dialog.set_error(
            "Warning: Map transition failed due to missing asset or parsing failure. Please verify game files.",
        );

        let canvas = Canvas::default();
        let fonts = Fonts::default();
        let theme = Theme::default();
        let painter = Painter {
            canvas: &canvas,
            fonts: &fonts,
            theme: &theme,
            origin: Vec2::ZERO,
            scale: 1.0,
        };

        dialog.draw(&painter, &loc);
        let quads = canvas.drain();

        // The background frame quad should have expanded height > LOADING_DIALOG_SIZE.y (112.0)
        let frame_quad = quads
            .iter()
            .find(|q| {
                q.destination.w == LOADING_DIALOG_SIZE.x && q.destination.h >= LOADING_DIALOG_SIZE.y
            })
            .expect("loading dialog frame quad");
        assert!(
            frame_quad.destination.h > LOADING_DIALOG_SIZE.y,
            "Expected dialog height to expand beyond 112px, got {}",
            frame_quad.destination.h
        );
    }

    #[test]
    fn background_loading_plaque_geometry_and_drawing() {
        use source_ui::canvas::Canvas;
        use source_ui::widgets::{Fonts, Theme};

        let mut loc = Localization::default();
        loc.insert("#GameUI_Loading", "LOADING...");
        let dialog = LoadingDialog::new_background("ep2_background01");
        assert!(dialog.is_background());
        assert_eq!(dialog.map, "ep2_background01");

        // 720p base canvas (1280x720, scale = 1.0)
        let (origin_720, scale_720) =
            LoadingDialog::background_plaque_geometry(Vec2::new(1280., 720.), 1.0);
        assert_eq!(scale_720, 1.0);
        let expected_x = 1280. - BACKGROUND_LOADING_SIZE.x - 24.0;
        let expected_y = 720. - BACKGROUND_LOADING_SIZE.y - 24.0;
        assert_eq!(origin_720, Vec2::new(expected_x, expected_y));

        let canvas = Canvas::default();
        let fonts = Fonts::default();
        let theme = Theme::default();
        let painter = Painter {
            canvas: &canvas,
            fonts: &fonts,
            theme: &theme,
            origin: Vec2::ZERO,
            scale: 1.0,
        };

        dialog.draw(&painter, &loc);
        let quads = canvas.drain();

        // Plaque is drawn via the shared dialog frame (rounded corners, no outer 3D border)
        let middle_bar = quads
            .iter()
            .find(|q| {
                q.destination.w == BACKGROUND_LOADING_SIZE.x
                    && q.destination.x == expected_x
                    && q.vertices.is_none()
            })
            .expect("background loading plaque middle bar in bottom-right corner");
        assert_eq!(middle_bar.color, Color::new(0.0, 0.0, 0.0, 0.65));

        // Verify corner polygon quads exist (4 rounded corner arcs from FrameSpec rounded_fill)
        let corner_polygons = quads
            .iter()
            .filter(|q| q.vertices.is_some() && q.destination.x >= expected_x)
            .count();
        assert_eq!(corner_polygons, 4);
    }

    #[test]
    fn background_loading_overlay_emits_backdrop_quad() {
        use source_ui::canvas::Canvas;

        let canvas = Canvas::default();
        canvas.resize(1920., 1080.);
        let dialog = LoadingDialog::new_background("ep2_background01");
        assert!(dialog.is_background());

        dialog.draw_background_overlay(&canvas, None);
        let quads = canvas.drain();
        assert_eq!(quads.len(), 1, "expected exactly 1 overlay quad");
        let overlay = &quads[0];
        assert_eq!(overlay.destination.w, 1920.);
        assert_eq!(overlay.destination.h, 1080.);
        assert_eq!(overlay.color, BackgroundTransitionState::MAIN_MENU_BACKDROP);
    }

    #[test]
    fn gameplay_loading_overlay_does_not_emit_backdrop_quad() {
        use source_ui::canvas::Canvas;
        use source_ui::widgets::{Fonts, Theme};

        let canvas = Canvas::default();
        canvas.resize(1920., 1080.);
        let dialog = LoadingDialog::new("d1_trainstation_01");
        assert!(!dialog.is_background());

        // Background overlay call must be a complete no-op for gameplay loading
        dialog.draw_background_overlay(&canvas, None);
        let overlay_quads = canvas.drain();
        assert!(
            overlay_quads.is_empty(),
            "gameplay loading dialog must not emit any background overlay"
        );

        // Standard draw must only produce normal dialog quads
        let loc = Localization::default();
        let fonts = Fonts::default();
        let theme = Theme::default();
        let painter = Painter {
            canvas: &canvas,
            fonts: &fonts,
            theme: &theme,
            origin: Vec2::ZERO,
            scale: 1.0,
        };

        dialog.draw(&painter, &loc);
        let draw_quads = canvas.drain();
        assert!(!draw_quads.is_empty());
    }

    #[test]
    fn test_background_transition_success_sequence() {
        let mut state = BackgroundTransitionState::default();
        assert_eq!(state, BackgroundTransitionState::Idle);
        assert!(!state.is_active());
        assert!(!state.is_terminal());
        assert_eq!(state.overlay_alpha(), 0.0);

        state.start("ep1_background01", "episodic");
        assert_eq!(
            state,
            BackgroundTransitionState::Starting {
                map: "ep1_background01".into(),
                target_category: "episodic".into(),
                displayed_category: "main".into(),
            }
        );
        assert!(state.is_active());
        assert!(!state.is_terminal());
        assert_eq!(state.target_map(), Some("ep1_background01"));
        assert_eq!(state.target_category(), Some("episodic"));
        assert_eq!(state.displayed_category("episodic"), "main");
        assert_eq!(
            state.overlay_alpha(),
            BackgroundTransitionState::BACKDROP_ALPHA
        );

        state.update(0.1);
        assert_eq!(
            state,
            BackgroundTransitionState::Loading {
                map: "ep1_background01".into(),
                target_category: "episodic".into(),
                displayed_category: "main".into(),
                elapsed: 0.1,
            }
        );
        assert!(state.is_active());
        assert_eq!(state.displayed_category("episodic"), "main");
        assert_eq!(
            state.overlay_alpha(),
            BackgroundTransitionState::BACKDROP_ALPHA
        );

        state.update(0.05);
        if let BackgroundTransitionState::Loading { elapsed, .. } = &state {
            assert!((elapsed - 0.15).abs() < 1e-5);
        } else {
            panic!("expected Loading state");
        }

        state.finish();
        assert_eq!(
            state,
            BackgroundTransitionState::Finished {
                map: "ep1_background01".into(),
                displayed_category: "episodic".into(),
            }
        );
        assert!(!state.is_active());
        assert!(state.is_terminal());
        assert!(state.is_finished());
        assert!(!state.is_failed());
        assert_eq!(state.displayed_category("episodic"), "episodic");
        assert_eq!(state.overlay_alpha(), 0.0);
    }

    #[test]
    fn test_background_transition_failure_sequence() {
        let mut state = BackgroundTransitionState::default();
        state.start("ep2_background01", "ep2");
        assert!(state.is_active());
        assert_eq!(state.displayed_category("ep2"), "main");

        state.update(0.05);
        assert!(matches!(state, BackgroundTransitionState::Loading { .. }));

        state.fail("BSP file missing or corrupted");
        assert_eq!(
            state,
            BackgroundTransitionState::Failed {
                map: "ep2_background01".into(),
                displayed_category: "ep2".into(),
                reason: "BSP file missing or corrupted".into(),
            }
        );
        assert!(!state.is_active());
        assert!(state.is_terminal());
        assert!(state.is_failed());
        assert!(!state.is_finished());
        assert_eq!(state.displayed_category("ep2"), "ep2");
        assert_eq!(state.overlay_alpha(), 0.0);
        assert_eq!(state.target_map(), Some("ep2_background01"));
    }

    #[test]
    fn test_background_transition_stale_completion_ignored_and_latest_wins() {
        let mut state = BackgroundTransitionState::default();
        // 1. Select Episode One
        state.start("ep1_background01", "episodic");
        assert_eq!(state.target_map(), Some("ep1_background01"));
        assert_eq!(state.displayed_category("episodic"), "main");

        // 2. Fast double switch to Episode Two before Episode One finishes
        state.start("ep2_background01", "ep2");
        assert_eq!(state.target_map(), Some("ep2_background01"));
        assert_eq!(state.displayed_category("ep2"), "main");

        // 3. Stale completion arrives for Episode One
        assert!(
            !state.finish_map("episodic", "ep1_background01"),
            "stale finish must return false"
        );
        assert!(state.is_active(), "state must remain active for ep2");
        assert_eq!(state.target_map(), Some("ep2_background01"));
        assert_eq!(state.displayed_category("ep2"), "main");

        // 4. Stale failure for Episode One also ignored
        assert!(!state.fail_map("episodic", "ep1_background01", "stale error"));
        assert!(state.is_active());

        // 5. Episode Two finishes
        assert!(state.finish_map("ep2", "ep2_background01"));
        assert!(state.is_finished());
        assert_eq!(state.displayed_category("ep2"), "ep2");
    }

    #[test]
    fn background_transition_tells_games_with_the_same_map_name_apart() {
        let mut state = BackgroundTransitionState::default();
        state.start("background01", "hl2");
        state.start("background01", "lostcoast");
        assert!(
            !state.finish_map("hl2", "background01"),
            "stale Half-Life 2 load"
        );
        assert!(!state.fail_map("hl2", "background01", "stale"));
        assert!(state.is_active());
        assert!(state.finish_map("lostcoast", "background01"));
        assert_eq!(state.displayed_category("lostcoast"), "lostcoast");
    }

    #[test]
    fn test_background_loading_dialog_failure_ends_transition_and_suppresses_overlay() {
        use source_ui::canvas::Canvas;
        use source_ui::widgets::{Fonts, Theme};

        let canvas = Canvas::default();
        canvas.resize(1920., 1080.);
        let mut dialog = LoadingDialog::new_background("background01");
        assert!(dialog.is_background());
        assert!(dialog.transition.is_active());

        // Overlay is drawn when active
        dialog.draw_background_overlay(&canvas, None);
        let quads = canvas.drain();
        assert_eq!(quads.len(), 1);

        // Map loading fails
        dialog.set_error("IO error loading map");
        assert!(dialog.is_failed());
        assert!(!dialog.transition.is_active());
        assert!(dialog.transition.is_failed());
        assert!(dialog.transition.is_terminal());

        // Once failed, background overlay and plaque must NOT be drawn
        dialog.draw_background_overlay(&canvas, None);
        let overlay_quads = canvas.drain();
        assert!(
            overlay_quads.is_empty(),
            "failed background map load must suppress overlay"
        );

        let loc = Localization::default();
        let fonts = Fonts::default();
        let theme = Theme::default();
        let painter = Painter {
            canvas: &canvas,
            fonts: &fonts,
            theme: &theme,
            origin: Vec2::ZERO,
            scale: 1.0,
        };
        dialog.draw(&painter, &loc);
        let plaque_quads = canvas.drain();
        assert!(
            plaque_quads.is_empty(),
            "failed background map load must suppress corner plaque"
        );
    }

    #[test]
    fn test_background_loading_plaque_styled_with_shared_frame_rounded_corners_no_3d_border() {
        use source_ui::canvas::Canvas;
        use source_ui::widgets::{Fonts, Theme};

        let canvas = Canvas::default();
        canvas.resize(1920., 1080.);
        let dialog = LoadingDialog::new_background("episodic_background01");
        assert!(dialog.is_background());
        assert!(dialog.transition.is_active());

        let mut loc = Localization::default();
        loc.insert("#GameUI_Loading", "LOADING...");
        let fonts = Fonts::default();
        let theme = Theme::default();
        let painter = Painter {
            canvas: &canvas,
            fonts: &fonts,
            theme: &theme,
            origin: Vec2::ZERO,
            scale: 1.0,
        };

        // Draw the background loading plaque
        dialog.draw(&painter, &loc);
        let quads = canvas.drain();
        assert!(!quads.is_empty());

        // 1. Verify corner polygon quads are emitted (4 corner arcs from FrameSpec rounded_fill)
        let polygon_quads: Vec<_> = quads.iter().filter(|q| q.vertices.is_some()).collect();
        assert_eq!(
            polygon_quads.len(),
            4,
            "background loading plaque must use rounded corners (4 corner polygons)"
        );

        // 2. Verify all background fill quads use the translucent plaque color (alpha ~0.65)
        let fill_color = Color::new(0.0, 0.0, 0.0, 0.65);
        for poly in &polygon_quads {
            if let Some(verts) = &poly.vertices {
                for (_, c) in verts {
                    assert_eq!(*c, fill_color);
                }
            }
        }

        // 3. Verify NO 3D raised border quads are emitted
        // (raised border draws 4 rectangles with theme.border_bright and theme.border_dark)
        let has_3d_border = quads.iter().any(|q| {
            q.vertices.is_none()
                && (q.color == theme.border_bright || q.color == theme.border_dark)
                && (q.destination.w <= 2.0 || q.destination.h <= 2.0)
        });
        assert!(
            !has_3d_border,
            "background loading plaque must NOT draw outer 3D raised border"
        );
    }

    #[test]
    fn test_gameplay_loading_dialog_uses_shared_frame_rounded_corners_no_3d_border() {
        use source_ui::canvas::Canvas;
        use source_ui::widgets::{Fonts, Theme};

        let canvas = Canvas::default();
        canvas.resize(1920., 1080.);
        let dialog = LoadingDialog::new("d1_trainstation_01");
        assert!(!dialog.is_background());

        let mut loc = Localization::default();
        loc.insert("#GameUI_Loading", "LOADING...");
        let fonts = Fonts::default();
        let theme = Theme::default();
        let painter = Painter {
            canvas: &canvas,
            fonts: &fonts,
            theme: &theme,
            origin: Vec2::ZERO,
            scale: 1.0,
        };

        dialog.draw(&painter, &loc);
        let quads = canvas.drain();

        // Corner polygon quads emitted by FrameSpec rounded_fill
        let polygon_quads: Vec<_> = quads.iter().filter(|q| q.vertices.is_some()).collect();
        assert_eq!(
            polygon_quads.len(),
            4,
            "gameplay loading dialog must use rounded corners"
        );

        // Frame background color
        for poly in &polygon_quads {
            if let Some(verts) = &poly.vertices {
                for (_, c) in verts {
                    assert_eq!(*c, theme.frame_bg);
                }
            }
        }
    }

    #[test]
    fn test_background_overlay_drawing_with_and_without_texture() {
        let canvas = source_ui::canvas::Canvas::default();
        let mut dialog = LoadingDialog::new_background("background01");
        assert!(dialog.is_background());
        assert!(dialog.transition.is_active());

        // Without background texture: queues only the backdrop dimming rectangle
        dialog.draw_background_overlay(&canvas, None);
        let quads = canvas.drain();
        assert_eq!(quads.len(), 1);
        assert_eq!(
            quads[0].color,
            BackgroundTransitionState::MAIN_MENU_BACKDROP
        );
        assert!(quads[0].texture.is_none());

        // With background texture: queues the textured background quad then the backdrop rectangle
        let bg_tex = source_ui::canvas::Texture2D::from_rgba8(2, 2, &[200; 16]);
        dialog.draw_background_overlay(&canvas, Some(&bg_tex));
        let quads = canvas.drain();
        assert_eq!(quads.len(), 2);
        assert!(quads[0].texture.is_some());
        assert_eq!(quads[0].color, source_ui::canvas::WHITE);
        assert_eq!(
            quads[1].color,
            BackgroundTransitionState::MAIN_MENU_BACKDROP
        );
        assert!(quads[1].texture.is_none());

        // When transition is inactive: nothing drawn
        dialog.transition.finish();
        dialog.draw_background_overlay(&canvas, Some(&bg_tex));
        assert!(canvas.drain().is_empty());
    }
}
