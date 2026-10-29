//! Declarative specifications, layout metrics, and drawing for VGUI dialog frames.

use glam::{vec2, Vec2};

use super::{Align, ButtonState, Painter, FONT_TALL};
use crate::canvas::{Color, Rect};

/// Frame client area offsets (`Frame.ClientInsetX`, title height) at 1:1 scale.
pub const CLIENT_INSET_X: f32 = 8.;
/// Top of a frame's client area: inset 6 + caption 16 + border 7 + 1.
pub const CLIENT_TOP: f32 = 30.;
/// `Frame.TitleTextInsetX`.
pub const TITLE_INSET_X: f32 = 16.;
/// Close button dimensions at 1:1 scale (Source VGUI FrameButton: 18x16 unscaled).
pub const CLOSE_BUTTON_SIZE: Vec2 = vec2(18., 16.);
/// Inset of the close button from the top-right corner of the frame.
pub const CLOSE_BUTTON_INSET: Vec2 = vec2(24., 6.);
/// Target cell height for the Marlett close glyph 'r' in VGUI dialog frames.
///
/// In Source VGUI (`vgui_controls::Frame`), caption buttons use Marlett at positive
/// cell height 14 in Windows GDI, where internal leading yields an effective 10px EM square
/// and an 8x8 cross centered in the 18x16 button rect (80% of fontdue's raw 14px EM raster).
pub const CLOSE_GLYPH_TALL: f32 = 10.;

/// Authentic corner radius for VGUI rounded dialog frames (`FrameBorder` `backgroundtype 2`).
///
/// In the Source SDK / retail GameUI, rounded frame borders (`FrameBorder`) use 8px corner
/// textures at base resolution, scaling to ~5.0 layout points.
pub const FRAME_CORNER_RADIUS: f32 = 5.0;

/// Style configuration for VGUI dialog frames.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameStyle {
    /// Corner radius in unscaled layout points.
    pub corner_radius: f32,
    /// Whether corners are rendered with rounded arcs.
    pub rounded: bool,
    /// Whether content, title bar, and close buttons are clipped to the rounded shape.
    pub clip_overflow: bool,
    /// Whether an outer 3D raised border is drawn.
    ///
    /// Per Source GameUI specifications (`sourcescheme.res` `FrameBorder`), dialog frames
    /// use `backgroundtype 2` with rounded corners and do NOT draw outer 3D raised borders.
    pub outer_3d_border: bool,
}

impl Default for FrameStyle {
    fn default() -> Self {
        Self {
            corner_radius: FRAME_CORNER_RADIUS,
            rounded: true,
            clip_overflow: true,
            outer_3d_border: false,
        }
    }
}

impl FrameStyle {
    pub const DEFAULT: Self = Self {
        corner_radius: FRAME_CORNER_RADIUS,
        rounded: true,
        clip_overflow: true,
        outer_3d_border: false,
    };

    /// Returns whether `point` (in frame layout coordinates) falls inside the styled frame bounds.
    pub fn contains_point(&self, point: Vec2, frame_size: Vec2) -> bool {
        if point.x < 0. || point.y < 0. || point.x > frame_size.x || point.y > frame_size.y {
            return false;
        }
        if !self.rounded || self.corner_radius <= 0. {
            return true;
        }
        let r = self
            .corner_radius
            .min(frame_size.x * 0.5)
            .min(frame_size.y * 0.5);
        // Top-left corner
        if point.x < r && point.y < r {
            let dx = r - point.x;
            let dy = r - point.y;
            return dx * dx + dy * dy <= r * r;
        }
        // Top-right corner
        if point.x > frame_size.x - r && point.y < r {
            let dx = point.x - (frame_size.x - r);
            let dy = r - point.y;
            return dx * dx + dy * dy <= r * r;
        }
        // Bottom-left corner
        if point.x < r && point.y > frame_size.y - r {
            let dx = r - point.x;
            let dy = point.y - (frame_size.y - r);
            return dx * dx + dy * dy <= r * r;
        }
        // Bottom-right corner
        if point.x > frame_size.x - r && point.y > frame_size.y - r {
            let dx = point.x - (frame_size.x - r);
            let dy = point.y - (frame_size.y - r);
            return dx * dx + dy * dy <= r * r;
        }
        true
    }

    /// Clips a child layout rectangle to stay strictly within the frame bounds,
    /// insetting from corners when `clip_overflow` is enabled.
    pub fn clip_rect(&self, rect: Rect, frame_size: Vec2) -> Option<Rect> {
        let mut x0 = rect.x.max(0.);
        let mut y0 = rect.y.max(0.);
        let mut x1 = (rect.x + rect.w).min(frame_size.x);
        let mut y1 = (rect.y + rect.h).min(frame_size.y);

        if x1 <= x0 || y1 <= y0 {
            return None;
        }

        if self.clip_overflow && self.rounded && self.corner_radius > 0. {
            let r = self
                .corner_radius
                .min(frame_size.x * 0.5)
                .min(frame_size.y * 0.5);
            let inset = r * 0.2929;
            if x0 < r && y0 < r {
                x0 = x0.max(inset);
                y0 = y0.max(inset);
            }
            if x1 > frame_size.x - r && y0 < r {
                x1 = x1.min(frame_size.x - inset);
                y0 = y0.max(inset);
            }
            if x0 < r && y1 > frame_size.y - r {
                x0 = x0.max(inset);
                y1 = y1.min(frame_size.y - inset);
            }
            if x1 > frame_size.x - r && y1 > frame_size.y - r {
                x1 = x1.min(frame_size.x - inset);
                y1 = y1.min(frame_size.y - inset);
            }
        }

        if x1 <= x0 || y1 <= y0 {
            None
        } else {
            Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
        }
    }
}

/// Calculates the layout rectangle of the close button for a frame of the given `size`.
pub fn close_button_rect(frame_size: Vec2) -> Rect {
    Rect::new(
        frame_size.x - CLOSE_BUTTON_INSET.x,
        CLOSE_BUTTON_INSET.y,
        CLOSE_BUTTON_SIZE.x,
        CLOSE_BUTTON_SIZE.y,
    )
}

/// Declarative specification for a VGUI frame window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameSpec<'a> {
    pub size: Vec2,
    pub title: &'a str,
    /// Whether the frame displays a close (X) button in the top-right corner.
    pub has_close: bool,
    /// Style configuration for this frame (corners, borders, clipping).
    pub style: FrameStyle,
    /// Optional background fill color override. When `None`, defaults to `theme.frame_bg`.
    pub bg_color: Option<Color>,
}

impl<'a> FrameSpec<'a> {
    pub const fn new(size: Vec2, title: &'a str) -> Self {
        Self {
            size,
            title,
            has_close: true,
            style: FrameStyle::DEFAULT,
            bg_color: None,
        }
    }

    pub const fn without_close(mut self) -> Self {
        self.has_close = false;
        self
    }

    pub const fn with_style(mut self, style: FrameStyle) -> Self {
        self.style = style;
        self
    }

    pub const fn with_bg_color(mut self, color: Color) -> Self {
        self.bg_color = Some(color);
        self
    }

    pub fn close_rect(&self) -> Option<Rect> {
        if !self.has_close {
            return None;
        }
        let raw_rect = close_button_rect(self.size);
        if self.style.clip_overflow {
            self.style.clip_rect(raw_rect, self.size)
        } else {
            Some(raw_rect)
        }
    }
}

impl Painter<'_> {
    /// A titled VGUI frame of `size` at the painter origin.
    pub fn frame(&self, size: Vec2, title: &str) {
        self.frame_with_close(size, title, None);
    }

    /// A titled VGUI frame of `size` with an optional close (X) button.
    pub fn frame_with_close(&self, size: Vec2, title: &str, close: Option<ButtonState>) {
        self.frame_spec(FrameSpec::new(size, title), close);
    }

    /// Draws the frame background (rounded or flat fill) and optional outer 3D border.
    pub fn frame_background(&self, spec: &FrameSpec) {
        let rect = Rect::new(0., 0., spec.size.x, spec.size.y);
        let bg = spec.bg_color.unwrap_or(self.theme.frame_bg);
        if spec.style.rounded {
            self.rounded_fill(rect, bg, spec.style.corner_radius);
        } else {
            self.fill(rect, bg);
        }
        if spec.style.outer_3d_border {
            self.raised(rect);
        }
    }

    /// Draws the frame title text and optional close button.
    pub fn frame_foreground(&self, spec: &FrameSpec, close: Option<ButtonState>) {
        if !spec.title.is_empty() {
            self.text(
                spec.title,
                vec2(TITLE_INSET_X, 8.),
                self.theme.title_text,
                true,
            );
        }
        if let Some(state) = close {
            if spec.has_close {
                let close_rect = spec
                    .close_rect()
                    .unwrap_or_else(|| close_button_rect(spec.size));
                self.close_button(close_rect, state);
            }
        }
    }

    /// Renders a titled VGUI frame according to the declarative [`FrameSpec`].
    pub fn frame_spec(&self, spec: FrameSpec, close: Option<ButtonState>) {
        self.frame_background(&spec);
        self.frame_foreground(&spec, close);
    }

    /// The close (X) button in a frame title bar.
    /// Drawn using Marlett's 'r' glyph (matching Source SDK Frame.cpp / SourceScheme.res),
    /// or a centered fallback cross, shifted by a pixel while pressed.
    pub fn close_button(&self, rect: Rect, state: ButtonState) {
        let offset = if state.depressed() { 1. } else { 0. };
        let color = self.theme.close_style(state);
        if let Some(marlett) = &self.fonts.marlett {
            // Source SDK Frame.cpp: _closeButton = new FrameButton(this, "frame_close", "r");
            // Marlett's 'r' is the standard titlebar close glyph (rendered at 8x8 px, 80% of tall 14).
            let tall = CLOSE_GLYPH_TALL;
            let width = marlett.width_cell("r", tall);
            let x = rect.x + (rect.w - width) * 0.5 + offset;
            let y = rect.y + (rect.h - tall) * 0.5 + offset;
            let p = self.origin + vec2(x, y) * self.scale;
            marlett.draw_cell("r", p.round(), tall * self.scale, color);
        } else if let Some(font) = self.font(false) {
            let symbol = if font.width_cell("×", FONT_TALL) < 8. {
                "X"
            } else {
                "×"
            };
            let text_rect = Rect::new(rect.x + offset, rect.y - 1. + offset, rect.w, rect.h);
            self.text_in(symbol, text_rect, Align::Center, color, false);
        }
    }
}
