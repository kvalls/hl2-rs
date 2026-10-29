//! VGUI-style widget drawing and the small pieces of widget logic shared by the menus and dialogs.
//!
//! Colors come from the installed `SourceSchemeBase.res` (`BaseSettings` names resolved through
//! `Colors`); the defaults below are only used when the scheme cannot be read. They mirror the
//! desktop (`[$WIN32]`) values of the installed `SourceScheme.res`, because the 20th-anniversary
//! `hl2_complete/resource/SourceSchemeBase.res` is an empty file and the defaults are what the
//! game currently runs with on that layout (known gap, see `docs/gameui.md` 6a). Geometry is in
//! VGUI layout pixels; a [`Painter`] maps it to canvas pixels with an origin and a uniform scale.
//! The scale is `max(1, canvas_height / 720)`: retail draws these non-proportional dialogs at
//! 1:1 screen pixels, which becomes unreadably small on high-resolution canvases (documented
//! deviation).

mod tooltips;
pub use tooltips::{PreparedTooltip, TooltipSection, TOOLTIP_GAP};
mod frame;
pub use frame::{
    close_button_rect, FrameSpec, FrameStyle, CLIENT_INSET_X, CLIENT_TOP, CLOSE_BUTTON_INSET,
    CLOSE_BUTTON_SIZE, CLOSE_GLYPH_TALL, FRAME_CORNER_RADIUS, TITLE_INSET_X,
};
mod text_input;
pub use text_input::{BlinkingCaret, InputFocus, TextInputModel};

use std::borrow::Cow;

use crate::canvas::{draw_texture_ex, Canvas, Color, DrawTextureParams, Rect, Texture2D, WHITE};
use crate::font::FontFace;
use glam::{vec2, Vec2};
use source_assets::keyvalues::Entry;

/// Character-cell height of the scheme's `Default`/`DefaultBold` fonts at 1:1 scale.
pub const FONT_TALL: f32 = 16.;

/// Classification of interactive UI controls for sound and interaction behaviors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidgetKind {
    /// Main menu and pause/submenu navigation buttons.
    MenuButton,
    /// Standard push button inside dialogs (OK, Cancel, Apply, Advanced, etc.).
    DialogButton,
    /// Radio button choice option.
    RadioButton,
    /// Dropdown / combo box list selector.
    Dropdown,
    /// Single-line or multi-line text input field.
    TextInput,
    /// Check box toggle button.
    CheckBox,
    /// Slider drag control.
    Slider,
    /// Non-interactive or decorative element.
    Other,
}

impl WidgetKind {
    /// Returns whether this control kind is eligible for the hover / rollover sound cue
    /// (`ui/buttonrollover.wav`).
    ///
    /// In authentic Source / Half-Life 2 GameUI, only top-level menu and submenu navigation buttons
    /// play a rollover sound when hovered or stepped. Dialog buttons, radio buttons, dropdowns, and
    /// text input fields inside dialogs do not play rollover sounds.
    #[inline]
    pub fn has_rollover_sound(self) -> bool {
        matches!(self, Self::MenuButton)
    }
}

/// Scheme colors used by the GameUI controls.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub frame_bg: Color,
    /// `Frame.OutOfFocusBgColor`: a frame without the keyboard focus.
    pub frame_bg_out_of_focus: Color,
    pub frame_bg_focused: Color,
    pub title_text: Color,
    /// `FrameTitleBar.DisabledTextColor`: the title of a frame without the focus.
    pub title_text_out_of_focus: Color,
    pub title_text_focused: Color,
    pub text: Color,
    /// Normal label text (`Label.TextColor`, OffWhite): check box and radio button labels at
    /// rest, between `text_dull` and the brighter hover `text`.
    pub label_text: Color,
    pub text_dull: Color,
    pub text_disabled: Color,
    pub text_disabled_shadow: Color,
    pub border_bright: Color,
    pub border_dark: Color,
    pub border_selection: Color,
    pub entry_bg: Color,
    pub selection_bg: Color,
    pub selection_text: Color,
    pub menu_bg: Color,
    pub list_bg: Color,
    pub slider_track: Color,
    pub slider_nob: Color,
    pub slider_text: Color,
    pub check: Color,
    pub arrow: Color,
    pub arrow_armed: Color,
    pub scrollbar_bg: Color,
    pub url_text: Color,
    pub tooltip_bg: Color,
    pub tooltip_text: Color,
    pub tooltip_border: Color,
    /// Push button colors per interaction state (`Button.TextColor` / `Button.BgColor`,
    /// `Button.ArmedTextColor` / `Button.ArmedBgColor`, `Button.DepressedTextColor` /
    /// `Button.DepressedBgColor`).
    pub button_text: Color,
    pub button_bg: Color,
    pub button_armed_text: Color,
    pub button_armed_bg: Color,
    pub button_depressed_text: Color,
    pub button_depressed_bg: Color,
    /// Title bar close (×) glyph colors (`FrameTitleButton.FgColor` /
    /// `FrameTitleButton.DisabledFgColor`); hover and press use `title_text`.
    pub close_fg: Color,
    pub close_disabled: Color,
    /// Line under a sectioned list's section header (`SectionedListPanel.DividerColor`).
    pub list_divider: Color,
    /// Dimmer selection of a list row whose list is busy elsewhere
    /// (`SectionedListPanel.OutOfFocusSelectedBgColor` / `OutOfFocusSelectedTextColor`).
    pub selection_bg_dim: Color,
    pub selection_text_dim: Color,
    /// Scroll bar nob fill (`ScrollBarSlider.FgColor`, Blank: only its border shows) and arrow
    /// glyph color (`ScrollBarButton.FgColor`).
    pub scrollbar_nob: Color,
    pub scrollbar_arrow: Color,
    /// Dashed box inside a push button, check box or radio button with keyboard focus
    /// (`Button.FocusBorderColor`, SDK `Button::DrawFocusBorder`).
    pub focus_border: Color,
    /// Selected text of a focused text entry or combo box (`TextEntry.SelectedBgColor` /
    /// `TextEntry.SelectedTextColor`) and the focused entry's inner edge
    /// (`TextEntry.FocusEdgeColor`).
    pub entry_selected_bg: Color,
    pub entry_selected_text: Color,
    pub entry_focus_edge: Color,
}

impl Default for Theme {
    fn default() -> Self {
        let rgba = Color::from_rgba;
        Self {
            frame_bg: rgba(132, 132, 132, 212),
            frame_bg_out_of_focus: rgba(132, 132, 132, 62),
            frame_bg_focused: rgba(132, 132, 132, 212),
            title_text: rgba(255, 255, 255, 255),
            title_text_out_of_focus: rgba(255, 255, 255, 192),
            title_text_focused: rgba(255, 255, 255, 255),
            text: rgba(255, 255, 255, 255),
            label_text: rgba(221, 221, 221, 255),
            text_dull: rgba(190, 190, 190, 255),
            text_disabled: rgba(117, 117, 117, 255),
            text_disabled_shadow: rgba(30, 30, 30, 255),
            border_bright: rgba(200, 200, 200, 196),
            border_dark: rgba(40, 40, 40, 196),
            border_selection: rgba(0, 0, 0, 196),
            entry_bg: rgba(0, 0, 0, 128),
            selection_bg: rgba(255, 155, 0, 255),
            selection_text: rgba(0, 0, 0, 255),
            menu_bg: rgba(160, 160, 160, 64),
            list_bg: rgba(0, 0, 0, 128),
            slider_track: rgba(31, 31, 31, 255),
            slider_nob: rgba(108, 108, 108, 255),
            slider_text: rgba(180, 180, 180, 255),
            check: rgba(255, 255, 255, 255),
            arrow: rgba(190, 190, 190, 255),
            arrow_armed: rgba(255, 255, 255, 255),
            scrollbar_bg: rgba(255, 255, 255, 64),
            url_text: rgba(255, 155, 0, 255),
            tooltip_bg: rgba(255, 155, 0, 240),
            tooltip_text: rgba(0, 0, 0, 255),
            tooltip_border: rgba(40, 40, 40, 255),
            // The desktop scheme (`[$WIN32]`) keeps armed and depressed identical to the
            // normal colors: White text over Blank.
            button_text: rgba(255, 255, 255, 255),
            button_bg: rgba(0, 0, 0, 0),
            button_armed_text: rgba(255, 255, 255, 255),
            button_armed_bg: rgba(0, 0, 0, 0),
            button_depressed_text: rgba(255, 255, 255, 255),
            button_depressed_bg: rgba(0, 0, 0, 0),
            close_fg: rgba(200, 200, 200, 196),
            close_disabled: rgba(255, 255, 255, 192),
            list_divider: rgba(0, 0, 0, 255),
            selection_bg_dim: rgba(255, 155, 0, 128),
            selection_text_dim: rgba(0, 0, 0, 255),
            scrollbar_nob: rgba(0, 0, 0, 0),
            scrollbar_arrow: rgba(255, 255, 255, 255),
            focus_border: rgba(0, 0, 0, 255),
            entry_selected_bg: rgba(255, 155, 0, 255),
            entry_selected_text: rgba(0, 0, 0, 255),
            entry_focus_edge: rgba(0, 0, 0, 196),
        }
    }
}

impl Theme {
    /// The theme of a dialog drawn over another dialog. Stacked dialogs keep the scheme's
    /// translucent `Frame.BgColor` / `Frame.OutOfFocusBgColor` now that underlying content
    /// behind the focused dialog is clipped.
    pub fn stacked(&self) -> Self {
        self.clone()
    }

    /// A frame's focus look (SDK `Frame::OnFrameFocusChanged`): the background `fade` of the
    /// way (0..1) from `Frame.OutOfFocusBgColor` to `Frame.BgColor` (it fades over
    /// `Frame.FocusTransitionEffectTime`), the title in `FrameTitleBar.TextColor` when
    /// `focused`, else `FrameTitleBar.DisabledTextColor` (switched at once).
    pub fn focus_faded(&self, fade: f32, focused: bool) -> Self {
        let mix = |from: Color, to: Color| {
            let t = fade.clamp(0., 1.);
            Color::new(
                from.r + (to.r - from.r) * t,
                from.g + (to.g - from.g) * t,
                from.b + (to.b - from.b) * t,
                from.a + (to.a - from.a) * t,
            )
        };
        let mut theme = self.clone();
        theme.frame_bg = mix(self.frame_bg_out_of_focus, self.frame_bg_focused);
        theme.title_text = if focused {
            self.title_text_focused
        } else {
            self.title_text_out_of_focus
        };
        theme
    }

    /// Reads the scheme colors, keeping defaults for anything missing.
    pub fn from_scheme(root: &Entry) -> Self {
        let mut theme = Self::default();
        let fields: [(&str, &mut Color); 43] = [
            ("Frame.BgColor", &mut theme.frame_bg),
            ("Frame.OutOfFocusBgColor", &mut theme.frame_bg_out_of_focus),
            ("FrameTitleBar.TextColor", &mut theme.title_text),
            (
                "FrameTitleBar.DisabledTextColor",
                &mut theme.title_text_out_of_focus,
            ),
            ("Label.TextBrightColor", &mut theme.text),
            ("Label.TextColor", &mut theme.label_text),
            ("Label.TextDullColor", &mut theme.text_dull),
            ("Label.DisabledFgColor1", &mut theme.text_disabled),
            ("Label.DisabledFgColor2", &mut theme.text_disabled_shadow),
            ("Border.Bright", &mut theme.border_bright),
            ("Border.Dark", &mut theme.border_dark),
            ("Border.Selection", &mut theme.border_selection),
            ("TextEntry.BgColor", &mut theme.entry_bg),
            ("ListPanel.SelectedBgColor", &mut theme.selection_bg),
            ("ListPanel.SelectedTextColor", &mut theme.selection_text),
            ("Menu.BgColor", &mut theme.menu_bg),
            ("SectionedListPanel.BgColor", &mut theme.list_bg),
            ("Slider.TrackColor", &mut theme.slider_track),
            ("Slider.NobColor", &mut theme.slider_nob),
            ("Slider.TextColor", &mut theme.slider_text),
            ("CheckButton.Check", &mut theme.check),
            ("ComboBoxButton.ArrowColor", &mut theme.arrow),
            ("ComboBoxButton.ArmedArrowColor", &mut theme.arrow_armed),
            ("ScrollBarSlider.BgColor", &mut theme.scrollbar_bg),
            ("ScrollBarSlider.FgColor", &mut theme.scrollbar_nob),
            ("ScrollBarButton.FgColor", &mut theme.scrollbar_arrow),
            ("Tooltip.BgColor", &mut theme.tooltip_bg),
            ("Tooltip.TextColor", &mut theme.tooltip_text),
            ("Button.TextColor", &mut theme.button_text),
            ("Button.BgColor", &mut theme.button_bg),
            ("Button.ArmedTextColor", &mut theme.button_armed_text),
            ("Button.ArmedBgColor", &mut theme.button_armed_bg),
            (
                "Button.DepressedTextColor",
                &mut theme.button_depressed_text,
            ),
            ("Button.DepressedBgColor", &mut theme.button_depressed_bg),
            ("FrameTitleButton.FgColor", &mut theme.close_fg),
            (
                "FrameTitleButton.DisabledFgColor",
                &mut theme.close_disabled,
            ),
            ("SectionedListPanel.DividerColor", &mut theme.list_divider),
            (
                "SectionedListPanel.OutOfFocusSelectedBgColor",
                &mut theme.selection_bg_dim,
            ),
            (
                "SectionedListPanel.OutOfFocusSelectedTextColor",
                &mut theme.selection_text_dim,
            ),
            ("Button.FocusBorderColor", &mut theme.focus_border),
            ("TextEntry.SelectedBgColor", &mut theme.entry_selected_bg),
            (
                "TextEntry.SelectedTextColor",
                &mut theme.entry_selected_text,
            ),
            ("TextEntry.FocusEdgeColor", &mut theme.entry_focus_edge),
        ];
        for (name, slot) in fields {
            if let Some(color) = scheme_color(root, name) {
                *slot = color;
            }
        }
        theme.frame_bg_focused = theme.frame_bg;
        theme.title_text_focused = theme.title_text;
        theme
    }

    /// Resolves the look of a bordered push button for an interaction state.
    ///
    /// Follows the SDK `Button` precedence: colors are depressed, then armed, then normal;
    /// the border is depressed, then key focus (the default button), then raised. Bordered
    /// buttons have no armed border, so hover only shows through the armed colors.
    /// Disabled buttons are never armed or depressed.
    ///
    /// Deliberate deviation (owner decision): the desktop scheme's armed background equals the
    /// normal one, so retail shows no hover. When that is the case the hover background is the
    /// scheme's `Menu.BgColor`, the translucent fill of menus and drop-down lists, which lightens
    /// the button slightly. A scheme with its own armed background keeps it.
    pub fn button_style(&self, state: ButtonState) -> ButtonStyle {
        let (text, bg) = if state.depressed() {
            (self.button_depressed_text, self.button_depressed_bg)
        } else if state.hovered() {
            let bg = if self.button_armed_bg == self.button_bg {
                self.menu_bg
            } else {
                self.button_armed_bg
            };
            (self.button_armed_text, bg)
        } else {
            (self.button_text, self.button_bg)
        };
        let border = if state.depressed() {
            ButtonBorder::Depressed
        } else if state.default && state.enabled {
            ButtonBorder::KeyFocus
        } else {
            ButtonBorder::Raised
        };
        ButtonStyle {
            border,
            text,
            bg,
            text_offset: if state.depressed() { 1. } else { 0. },
        }
    }

    /// Color of the title bar close (×) glyph. The button has no border or fill; hovering or
    /// pressing lightens the glyph to the title text color (owner decision, not retail).
    pub fn close_style(&self, state: ButtonState) -> Color {
        if !state.enabled {
            self.close_disabled
        } else if state.armed || state.pressed {
            self.title_text
        } else {
            self.close_fg
        }
    }

    /// Resolves the look of a property-sheet tab.
    ///
    /// Tabs (SDK `PageTab`) never enter the armed state and pick their border and text color
    /// from the active flag alone. As an owner-approved deviation, a hovered inactive, enabled
    /// tab gets the same slight `Menu.BgColor` fill as a hovered push button; the active tab
    /// never changes, and pressing adds nothing.
    /// Label color of a check box or radio button: the normal label color at rest, the bright
    /// text while hovered or pressed, the disabled color when disabled.
    pub fn toggle_text(&self, state: ButtonState) -> Color {
        if !state.enabled {
            self.text_disabled
        } else if state.armed || state.pressed {
            self.text
        } else {
            self.label_text
        }
    }

    pub fn tab_style(&self, active: bool, state: ButtonState) -> TabStyle {
        TabStyle {
            text: if active { self.text } else { self.text_dull },
            open_bottom: active,
            fill: !active,
            hover_bg: (!active && state.armed && state.enabled).then_some(self.menu_bg),
        }
    }
}

/// Border of a bordered push button (`ButtonBorder`, `ButtonDepressedBorder`,
/// `ButtonKeyFocusBorder`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonBorder {
    Raised,
    Depressed,
    KeyFocus,
}

/// Resolved push button look; see [`Theme::button_style`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonStyle {
    pub border: ButtonBorder,
    pub text: Color,
    pub bg: Color,
    /// Label shift while depressed, in layout pixels.
    pub text_offset: f32,
}

/// Resolved tab look; see [`Theme::tab_style`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabStyle {
    pub text: Color,
    /// The active tab has no bottom edge, so it opens into its page.
    pub open_bottom: bool,
    /// Inactive tabs fill their own background; the active tab is the page's surface (the
    /// frame shows through both), so the two read as one piece.
    pub fill: bool,
    /// Hover fill drawn over the tab background, if any.
    pub hover_bg: Option<Color>,
}

/// Resolves a scheme color: a literal `"r g b a"` or a name in `BaseSettings`/`Colors`.
pub fn scheme_color(root: &Entry, name: &str) -> Option<Color> {
    let mut value = name;
    for _ in 0..8 {
        let values: Vec<u8> = value
            .split_whitespace()
            .map(str::parse)
            .collect::<Result<_, _>>()
            .unwrap_or_default();
        if let [r, g, b, a] = values.as_slice() {
            return Some(Color::from_rgba(*r, *g, *b, *a));
        }
        value = root
            .get("BaseSettings")
            .and_then(|e| e.get(value))
            .or_else(|| root.get("Colors").and_then(|e| e.get(value)))?
            .text()?;
    }
    None
}

/// Fonts used by the controls. Missing fonts skip text drawing but keep layout and input working.
#[derive(Clone, Default)]
pub struct Fonts {
    pub text: Option<FontFace>,
    pub bold: Option<FontFace>,
    pub marlett: Option<FontFace>,
}

/// Text alignment inside a rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Maps layout rectangles to canvas pixels and draws primitives.
pub struct Painter<'a> {
    pub canvas: &'a Canvas,
    pub fonts: &'a Fonts,
    pub theme: &'a Theme,
    pub origin: Vec2,
    pub scale: f32,
}

impl Painter<'_> {
    /// The dialog scale for a canvas height (see module documentation).
    pub fn scale_for(canvas_height: f32) -> f32 {
        (canvas_height / 720.).max(1.)
    }

    /// Computes the dialog scale for a given window logical height and display scale factor.
    pub fn scale_for_window(logical_height: f32, scale_factor: f32) -> f32 {
        Self::scale_for(logical_height * scale_factor.max(1.0))
    }

    pub fn at(&self, offset: Vec2) -> Painter<'_> {
        Painter {
            canvas: self.canvas,
            fonts: self.fonts,
            theme: self.theme,
            origin: self.origin + offset * self.scale,
            scale: self.scale,
        }
    }

    pub fn map(&self, rect: Rect) -> Rect {
        Rect::new(
            (self.origin.x + rect.x * self.scale).round(),
            (self.origin.y + rect.y * self.scale).round(),
            (rect.w * self.scale).round(),
            (rect.h * self.scale).round(),
        )
    }

    pub fn fill(&self, rect: Rect, color: Color) {
        let r = self.map(rect);
        self.canvas.rectangle(r.x, r.y, r.w, r.h, color);
    }

    /// One-pixel (scaled) outline with separate top-left and bottom-right colors.
    pub fn border(&self, rect: Rect, top_left: Color, bottom_right: Color) {
        let r = self.map(rect);
        let t = self.scale.round().max(1.);
        self.canvas.rectangle(r.x, r.y, r.w, t, top_left);
        self.canvas.rectangle(r.x, r.y, t, r.h, top_left);
        self.canvas
            .rectangle(r.x, r.y + r.h - t, r.w, t, bottom_right);
        self.canvas
            .rectangle(r.x + r.w - t, r.y, t, r.h, bottom_right);
    }

    pub fn raised(&self, rect: Rect) {
        self.border(rect, self.theme.border_bright, self.theme.border_dark);
    }

    /// A raised border whose top edge is open between the side edges of `tab`, where the active
    /// tab joins the page below it.
    pub fn raised_open_top(&self, rect: Rect, tab: Rect) {
        let r = self.map(rect);
        let tab = self.map(tab);
        let t = self.scale.round().max(1.);
        let (bright, dark) = (self.theme.border_bright, self.theme.border_dark);
        let gap_start = (tab.x + t).clamp(r.x, r.x + r.w);
        let gap_end = (tab.x + tab.w - t).clamp(gap_start, r.x + r.w);
        self.canvas.rectangle(r.x, r.y, gap_start - r.x, t, bright);
        self.canvas
            .rectangle(gap_end, r.y, r.x + r.w - gap_end, t, bright);
        self.canvas.rectangle(r.x, r.y, t, r.h, bright);
        self.canvas.rectangle(r.x, r.y + r.h - t, r.w, t, dark);
        self.canvas.rectangle(r.x + r.w - t, r.y, t, r.h, dark);
    }

    pub fn sunken(&self, rect: Rect) {
        self.border(rect, self.theme.border_dark, self.theme.border_bright);
    }

    pub fn font(&self, bold: bool) -> Option<&FontFace> {
        if bold {
            self.fonts.bold.as_ref().or(self.fonts.text.as_ref())
        } else {
            self.fonts.text.as_ref()
        }
    }

    /// Text width in layout units (max line width for multi-line text).
    pub fn text_width(&self, text: &str, bold: bool) -> f32 {
        self.font(bold).map_or(
            text.lines()
                .map(|l| l.chars().count() as f32 * 7.)
                .fold(0.0, f32::max),
            |f| {
                text.lines()
                    .map(|l| f.width_cell(l, FONT_TALL))
                    .fold(0.0, f32::max)
            },
        )
    }

    /// Truncates `text` with an ellipsis so its width fits within `max_w` using the current font.
    pub fn truncate_ellipsis<'a>(&self, text: &'a str, max_w: f32, bold: bool) -> Cow<'a, str> {
        truncate_ellipsis(text, max_w, |s| self.text_width(s, bold))
    }

    /// Draws text with its cell top-left at a layout position. Multi-line text is split on newlines.
    pub fn text(&self, text: &str, at: Vec2, color: Color, bold: bool) {
        if let Some(font) = self.font(bold) {
            for (i, line) in text.lines().enumerate() {
                let p = self.origin + (at + vec2(0., i as f32 * FONT_TALL)) * self.scale;
                font.draw_cell(line, p.round(), FONT_TALL * self.scale, color);
            }
        }
    }

    /// Draws one line of text vertically centered in `rect`.
    pub fn text_in(&self, text: &str, rect: Rect, align: Align, color: Color, bold: bool) {
        let width = self.text_width(text, bold);
        let x = match align {
            Align::Left => rect.x,
            Align::Center => rect.x + (rect.w - width) * 0.5,
            Align::Right => rect.x + rect.w - width,
        };
        self.text(
            text,
            vec2(x, rect.y + (rect.h - FONT_TALL) * 0.5),
            color,
            bold,
        );
    }

    /// Label text: bright when enabled, VGUI's embossed two-tone gray when disabled.
    pub fn label(&self, text: &str, rect: Rect, align: Align, enabled: bool) {
        if enabled {
            self.text_in(text, rect, align, self.theme.text, false);
        } else {
            let shadow = Rect::new(rect.x + 1., rect.y + 1., rect.w, rect.h);
            self.text_in(text, shadow, align, self.theme.text_disabled_shadow, false);
            self.text_in(text, rect, align, self.theme.text_disabled, false);
        }
    }

    /// Draws multi-line text with word wrapping, respecting `rect` bounds and alignment.
    /// Lines exceeding `rect.h` are truncated so text never overflows its parent bounds.
    pub fn multiline_label(&self, text: &str, rect: Rect, align: Align, color: Color) {
        let max_w = rect.w.max(1.);
        let lines = wrap(text, max_w, |s| self.text_width(s, false));
        let mut y = rect.y;
        for line in lines {
            if y + FONT_TALL > rect.y + rect.h + FONT_TALL * 0.5 {
                break;
            }
            let line_w = self.text_width(&line, false);
            let x = match align {
                Align::Left => rect.x,
                Align::Center => rect.x + ((rect.w - line_w) * 0.5).max(0.),
                Align::Right => rect.x + (rect.w - line_w).max(0.),
            };
            self.text(&line, vec2(x, y), color, false);
            y += FONT_TALL;
        }
    }

    /// Word-wrapped text from the top-left of `rect` (north-west labels).
    pub fn wrapped(&self, text: &str, rect: Rect, color: Color) {
        self.multiline_label(text, rect, Align::Left, color);
    }

    /// A filled disc approximated by one rectangle per scaled pixel row.
    pub fn disc(&self, center: Vec2, radius: f32, color: Color) {
        let c = self.origin + center * self.scale;
        let r = radius * self.scale;
        let rows = (2. * r).ceil() as i32;
        for row in 0..rows {
            let dy = row as f32 + 0.5 - r;
            let half = (r * r - dy * dy).max(0.).sqrt();
            self.canvas.rectangle(
                (c.x - half).round(),
                (c.y - r + row as f32).round(),
                (2. * half).round(),
                1.,
                color,
            );
        }
    }

    /// A downward triangle (combo box arrow) built from row rectangles.
    pub fn arrow_down(&self, center: Vec2, half_width: f32, color: Color) {
        let c = self.origin + center * self.scale;
        let w = half_width * self.scale;
        let rows = w.ceil() as i32;
        for row in 0..rows {
            let half = w - row as f32;
            self.canvas.rectangle(
                (c.x - half).round(),
                (c.y - w * 0.5 + row as f32).round(),
                (2. * half).round().max(1.),
                1.,
                color,
            );
        }
    }

    /// An upward triangle, the mirror of [`Painter::arrow_down`].
    pub fn arrow_up(&self, center: Vec2, half_width: f32, color: Color) {
        let c = self.origin + center * self.scale;
        let w = half_width * self.scale;
        let rows = w.ceil() as i32;
        for row in 0..rows {
            let half = w - row as f32;
            self.canvas.rectangle(
                (c.x - half).round(),
                (c.y + w * 0.5 - 1. - row as f32).round(),
                (2. * half).round().max(1.),
                1.,
                color,
            );
        }
    }

    /// Draws a checkmark inside a checkbox's box rectangle (`box_rect`).
    ///
    /// Matches the Source VGUI checkmark appearance: a 2-pixel-thick tick mark
    /// with a short descending left stroke and longer ascending right stroke.
    pub fn check_mark(&self, box_rect: Rect, color: Color, offset: f32) {
        let x0 = box_rect.x + offset;
        let y0 = box_rect.y + offset;
        // The checkmark is composed of 7 vertical columns (each 2 units tall) centered inside the 12x12 box:
        // Left arm descends from (3, 5) to apex at (5, 8); right arm ascends from (5, 8) to (9, 3).
        const COLUMNS: [(f32, f32); 7] = [
            (3., 5.),
            (4., 6.),
            (5., 7.),
            (6., 6.),
            (7., 5.),
            (8., 4.),
            (9., 3.),
        ];
        for (dx, dy) in COLUMNS {
            self.fill(Rect::new(x0 + dx, y0 + dy, 1., 2.), color);
        }
    }
}

/// Button interaction state for drawing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ButtonState {
    pub armed: bool,
    pub pressed: bool,
    pub enabled: bool,
    /// The dialog's default button (drawn with the selection border).
    pub default: bool,
}

impl ButtonState {
    pub const fn new(enabled: bool) -> Self {
        Self {
            armed: false,
            pressed: false,
            enabled,
            default: false,
        }
    }

    pub const fn active(enabled: bool, armed: bool, pressed: bool) -> Self {
        Self {
            armed,
            pressed,
            enabled,
            default: false,
        }
    }

    /// Drawn pushed in: pressed and enabled. Pressing overrides hover.
    pub const fn depressed(self) -> bool {
        self.pressed && self.enabled
    }

    /// Drawn with the hover (armed) look: armed, enabled and not depressed.
    pub const fn hovered(self) -> bool {
        self.armed && self.enabled && !self.depressed()
    }
}

/// Keyboard focus moves of a dialog layer (SDK `FocusNavGroup::RequestFocusNext/Prev`): Tab and
/// Shift+Tab step through `order` (the layer's controls by `tabPosition`) and wrap at both ends.
/// With nothing focused, Tab starts at the first stop and Shift+Tab at the last.
pub fn focus_step<T: Copy + PartialEq>(
    current: Option<T>,
    order: &[T],
    forward: bool,
) -> Option<T> {
    let len = order.len();
    if len == 0 {
        return None;
    }
    let index = match current.and_then(|c| order.iter().position(|t| *t == c)) {
        Some(i) if forward => (i + 1) % len,
        Some(i) => (i + len - 1) % len,
        None if forward => 0,
        None => len - 1,
    };
    Some(order[index])
}

/// Space/Enter on a focused button (SDK `Button::OnKeyCodePressed/Released`): the press shows
/// the button depressed and the release activates it. A press that is not held (a scripted
/// one-frame key) activates at once; focus moving away drops the press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyPress<T: Copy + PartialEq> {
    down: Option<(crate::input::Key, T)>,
}

impl<T: Copy + PartialEq> Default for KeyPress<T> {
    fn default() -> Self {
        Self { down: None }
    }
}

impl<T: Copy + PartialEq> KeyPress<T> {
    /// The control shown depressed by a held Space or Enter.
    pub fn pressed(&self) -> Option<T> {
        self.down.map(|(_, target)| target)
    }

    pub fn reset(&mut self) {
        self.down = None;
    }

    /// Feeds one frame. `pressed` is a Space or Enter press on `focused` this frame, `held`
    /// whether a key is still down. Returns the control to activate.
    pub fn update(
        &mut self,
        focused: Option<T>,
        pressed: Option<crate::input::Key>,
        held: impl Fn(crate::input::Key) -> bool,
    ) -> Option<T> {
        if let Some((key, target)) = self.down {
            if focused != Some(target) {
                self.down = None;
            } else if !held(key) {
                self.down = None;
                return Some(target);
            }
            return None;
        }
        let (key, target) = (pressed?, focused?);
        if held(key) {
            self.down = Some((key, target));
            None
        } else {
            Some(target)
        }
    }
}

/// Tracks pointer button press, hold, cancellation and release activation for interactive controls.
///
/// Two drag-off policies exist. The default ([`ButtonTracker::new`]) cancels the press when the
/// pointer leaves the control while held, as for menu items, which do not capture the mouse.
/// [`ButtonTracker::capturing`] follows the SDK `Button` default for dialog controls: a press
/// captures the mouse, the control stays pressed while the pointer is dragged off and back,
/// and the release activates it only while the pointer is over it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ButtonTracker<T: Copy + PartialEq> {
    pressed: Option<T>,
    was_down: bool,
    capture: bool,
}

impl<T: Copy + PartialEq> ButtonTracker<T> {
    pub const fn new() -> Self {
        Self {
            pressed: None,
            was_down: false,
            capture: false,
        }
    }

    /// A tracker whose presses capture the mouse until release (dialog controls).
    pub const fn capturing() -> Self {
        Self {
            pressed: None,
            was_down: false,
            capture: true,
        }
    }

    /// Current pressed control, if any.
    pub fn pressed(&self) -> Option<T> {
        self.pressed
    }

    /// Whether the given control is currently pressed.
    pub fn is_pressed(&self, target: T) -> bool {
        self.pressed == Some(target)
    }

    /// The control that may show its hover look: while a capturing press is held only the
    /// pressed control can be armed, since the captured mouse reaches no other control.
    pub fn armable(&self, hovered: Option<T>) -> Option<T> {
        match self.pressed {
            Some(target) if self.capture => hovered.filter(|h| *h == target),
            _ => hovered,
        }
    }

    /// Drops any press. The button edge state is kept: it describes the physical button, and
    /// clearing it would make a button still held from another layer's click read as a new
    /// press on the next frame.
    pub fn reset(&mut self) {
        self.pressed = None;
    }

    /// Advances the tracker on a frame when a layer above owns the pointer (an open popup
    /// list or a modal dialog): nothing is pressed or activated here, but the button edge
    /// state stays current, so the rest of a gesture the upper layer consumed is never read
    /// as a new press when this layer gets the pointer back.
    pub fn occlude(&mut self, down: bool) {
        self.pressed = None;
        self.was_down = down;
    }

    /// Updates the state machine for the current frame.
    ///
    /// - `hovered`: The interactive control under the cursor, if any.
    /// - `down`: Whether the mouse button is down (`click` or `held`).
    /// - `click`: Whether the mouse button went down this frame.
    /// - `released`: Whether the mouse button was released this frame.
    ///
    /// Returns `Some(T)` only on release while the cursor is still over the control.
    /// If the cursor moves away while the mouse button is held, a non-capturing tracker
    /// cancels the press, so a later release does not trigger the action; a capturing
    /// tracker keeps it, so releasing after moving back over the control still does.
    pub fn update(
        &mut self,
        hovered: Option<T>,
        down: bool,
        click: bool,
        released: bool,
    ) -> Option<T> {
        let is_release = released || (self.was_down && !down);
        let mut activated = None;

        if is_release {
            if let Some(target) = self.pressed {
                if hovered == Some(target) {
                    activated = Some(target);
                }
            }
            self.pressed = None;
        } else if click || (down && !self.was_down) {
            // Mouse button went down this frame.
            self.pressed = hovered;
        } else if down {
            // Mouse button is being held down.
            if let Some(target) = self.pressed.filter(|_| !self.capture) {
                if hovered != Some(target) {
                    // Cursor moved away while mouse button is held -> cancel pressed state.
                    self.pressed = None;
                }
            }
        } else {
            self.pressed = None;
        }

        self.was_down = down;
        activated
    }
}

impl Painter<'_> {
    /// Draws a filled rectangle with rounded corners using the specified layout radius.
    pub fn rounded_fill(&self, rect: Rect, color: Color, radius: f32) {
        let r = self.map(rect);
        let cr = (radius * self.scale).round().min(r.w * 0.5).min(r.h * 0.5);
        if cr <= 0. {
            self.canvas.rectangle(r.x, r.y, r.w, r.h, color);
            return;
        }

        // Top bar
        self.canvas
            .rectangle(r.x + cr, r.y, r.w - 2. * cr, cr, color);
        // Middle bar
        self.canvas
            .rectangle(r.x, r.y + cr, r.w, r.h - 2. * cr, color);
        // Bottom bar
        self.canvas
            .rectangle(r.x + cr, r.y + r.h - cr, r.w - 2. * cr, cr, color);

        let k = 0.2929 * cr;

        // Top-Left corner: center (r.x + cr, r.y + cr)
        let tl_center = vec2(r.x + cr, r.y + cr);
        self.canvas.polygon([
            (tl_center, color),
            (vec2(r.x + cr, r.y), color),
            (vec2(r.x + k, r.y + k), color),
            (vec2(r.x, r.y + cr), color),
        ]);

        // Top-Right corner: center (r.x + r.w - cr, r.y + cr)
        let tr_center = vec2(r.x + r.w - cr, r.y + cr);
        self.canvas.polygon([
            (tr_center, color),
            (vec2(r.x + r.w, r.y + cr), color),
            (vec2(r.x + r.w - k, r.y + k), color),
            (vec2(r.x + r.w - cr, r.y), color),
        ]);

        // Bottom-Right corner: center (r.x + r.w - cr, r.y + r.h - cr)
        let br_center = vec2(r.x + r.w - cr, r.y + r.h - cr);
        self.canvas.polygon([
            (br_center, color),
            (vec2(r.x + r.w - cr, r.y + r.h), color),
            (vec2(r.x + r.w - k, r.y + r.h - k), color),
            (vec2(r.x + r.w, r.y + r.h - cr), color),
        ]);

        // Bottom-Left corner: center (r.x + cr, r.y + r.h - cr)
        let bl_center = vec2(r.x + cr, r.y + r.h - cr);
        self.canvas.polygon([
            (bl_center, color),
            (vec2(r.x, r.y + r.h - cr), color),
            (vec2(r.x + k, r.y + r.h - k), color),
            (vec2(r.x + cr, r.y + r.h), color),
        ]);
    }

    /// A push button drawn from [`Theme::button_style`]: raised border, sunken while pressed,
    /// selection border when default, and the scheme's armed colors while hovered.
    pub fn button(&self, rect: Rect, label: &str, state: ButtonState) {
        let style = self.theme.button_style(state);
        if style.bg.a > 0. {
            self.fill(rect, style.bg);
        }
        match style.border {
            ButtonBorder::Depressed => self.sunken(rect),
            ButtonBorder::Raised => self.raised(rect),
            ButtonBorder::KeyFocus => {
                self.raised(rect);
                let outer = Rect::new(rect.x - 1., rect.y - 1., rect.w + 2., rect.h + 2.);
                self.border(
                    outer,
                    self.theme.border_selection,
                    self.theme.border_selection,
                );
            }
        }
        let offset = style.text_offset;
        let text_rect = Rect::new(rect.x + 6. + offset, rect.y + offset, rect.w - 12., rect.h);
        if state.enabled {
            self.text_in(label, text_rect, Align::Left, style.text, false);
        } else {
            self.label(label, text_rect, Align::Left, false);
        }
    }

    /// `CheckButton`: a sunken box with a check mark, then the label.
    pub fn check_box(&self, rect: Rect, label: &str, checked: bool, state: ButtonState) {
        let cy = rect.y + rect.h * 0.5;
        let bx = Rect::new(
            rect.x + 2.,
            (cy - CHECK_SIZE * 0.5).round(),
            CHECK_SIZE,
            CHECK_SIZE,
        );
        let bg_color = if state.pressed && state.enabled {
            self.theme.border_dark
        } else {
            self.theme.entry_bg
        };
        self.fill(bx, bg_color);
        self.sunken(bx);
        if checked {
            let color = if state.enabled {
                self.theme.check
            } else {
                self.theme.text_disabled
            };
            let offset = if state.pressed && state.enabled {
                1.
            } else {
                0.
            };
            self.check_mark(bx, color, offset);
        }
        let text_rect = Rect::new(rect.x + LABEL_INDENT, rect.y, rect.w - LABEL_INDENT, rect.h);
        let text_color = self.theme.toggle_text(state);
        if state.enabled {
            self.text_in(label, text_rect, Align::Left, text_color, false);
        } else {
            self.label(label, text_rect, Align::Left, false);
        }
    }

    /// `RadioButton`: a sunken disc with a dot when selected, then the label.
    pub fn radio(&self, rect: Rect, label: &str, selected: bool, state: ButtonState) {
        let center = vec2(rect.x + 2. + CHECK_SIZE * 0.5, rect.y + rect.h * 0.5);
        self.disc(center, CHECK_SIZE * 0.5, self.theme.border_dark);
        let bg_color = if state.pressed && state.enabled {
            self.theme.border_dark
        } else {
            self.theme.entry_bg
        };
        self.disc(center, CHECK_SIZE * 0.5 - 1., bg_color);
        if selected {
            let color = if state.enabled {
                self.theme.check
            } else {
                self.theme.text_disabled
            };
            let offset = if state.pressed && state.enabled {
                vec2(0.5, 0.5)
            } else {
                Vec2::ZERO
            };
            self.disc(center + offset, 2.5, color);
        }
        let text_rect = Rect::new(rect.x + LABEL_INDENT, rect.y, rect.w - LABEL_INDENT, rect.h);
        let text_color = self.theme.toggle_text(state);
        if state.enabled {
            self.text_in(label, text_rect, Align::Left, text_color, false);
        } else {
            self.label(label, text_rect, Align::Left, false);
        }
    }

    /// `Slider` with tick marks and its left/right captions below the track.
    pub fn slider(&self, rect: Rect, fraction: f32, left: &str, right: &str, enabled: bool) {
        let (start, end) = slider_track(rect);
        let track_y = rect.y + SLIDER_TRACK_Y;
        let track = Rect::new(start, track_y - 2., end - start, 4.);
        self.fill(track, self.theme.slider_track);
        self.sunken(track);
        for i in 0..=SLIDER_TICKS {
            let x = start + (end - start) * i as f32 / SLIDER_TICKS as f32;
            self.fill(
                Rect::new(x.round(), track_y + 8., 1., 4.),
                self.theme.slider_text,
            );
        }
        let nob_x = start + (end - start) * fraction.clamp(0., 1.) - NOB_WIDTH * 0.5;
        let nob = Rect::new(nob_x, track_y - 8., NOB_WIDTH, 16.);
        self.fill(nob, self.theme.slider_nob);
        self.raised(nob);
        if rect.h >= 32. {
            // The captions span what the slider draws, so the right one ends with the slider.
            let drawn = slider_content(rect);
            let caption = Rect::new(drawn.x, track_y + 12., drawn.w, FONT_TALL);
            let color = if enabled {
                self.theme.slider_text
            } else {
                self.theme.text_disabled
            };
            self.text_in(left, caption, Align::Left, color, false);
            self.text_in(right, caption, Align::Right, color, false);
        }
    }

    /// `TextEntry`: sunken field; `caret` draws the insertion bar after the text.
    /// SDK `Button::DrawFocusBorder`: a 1-pixel dashed box from (3, 3) to (wide - 4, tall - 2)
    /// inside a control with keyboard focus.
    pub fn focus_box(&self, rect: Rect) {
        let (x0, y0) = (rect.x + 3., rect.y + 3.);
        let (x1, y1) = (rect.x + rect.w - 4., rect.y + rect.h - 2.);
        let color = self.theme.focus_border;
        let mut x = x0;
        while x < x1 {
            self.fill(Rect::new(x, y0, 1., 1.), color);
            self.fill(Rect::new(x, y1 - 1., 1., 1.), color);
            x += 2.;
        }
        let mut y = y0;
        while y < y1 {
            self.fill(Rect::new(x0, y, 1., 1.), color);
            self.fill(Rect::new(x1 - 1., y, 1., 1.), color);
            y += 2.;
        }
    }

    /// A dashed 1-pixel ring on `rect`'s edges: the focus mark around controls whose SDK focus
    /// look is not a push button's inner box (check boxes, radio buttons, sliders, entries,
    /// lists; owner request, 2026-10-17).
    pub fn focus_ring(&self, rect: Rect) {
        self.focus_box(Rect::new(
            rect.x - 3.,
            rect.y - 3.,
            rect.w + 7.,
            rect.h + 5.,
        ));
    }

    /// Draws the background, sunken 3D border, and optional focus edge of a `TextEntry`.
    pub fn text_entry_frame(&self, rect: Rect, focused: bool) {
        self.fill(rect, self.theme.entry_bg);
        self.sunken(rect);
        if focused {
            // SDK `TextEntry`: a focused entry draws its focus edge inside the border.
            let edge = self.theme.entry_focus_edge;
            self.border(
                Rect::new(rect.x + 1., rect.y + 1., rect.w - 2., rect.h - 2.),
                edge,
                edge,
            );
        }
    }

    /// Draws the vertical insertion bar (caret) inside a text entry at dialog x coordinate `x`.
    pub fn caret(&self, rect: Rect, x: f32) {
        let r = Rect::new(x, rect.y + 4., 1., (rect.h - 8.).max(1.));
        let mut mapped = self.map(r);
        mapped.w = mapped.w.max(1.);
        self.canvas
            .rectangle(mapped.x, mapped.y, mapped.w, mapped.h, self.theme.text);
    }

    /// `TextEntry`: sunken field; `focused` draws the focus edge border and keeps text untruncated;
    /// `caret_visible` draws the 1-pixel insertion bar after the text.
    pub fn text_entry(
        &self,
        rect: Rect,
        text: &str,
        enabled: bool,
        focused: bool,
        caret_visible: bool,
    ) {
        self.text_entry_frame(rect, focused);
        let inner = Rect::new(rect.x + 3., rect.y, (rect.w - 6.).max(0.), rect.h);
        let color = if enabled {
            self.theme.text
        } else {
            self.theme.text_disabled
        };
        if !focused {
            let display = self.truncate_ellipsis(text, inner.w, false);
            self.text_in(&display, inner, Align::Left, color, false);
        } else {
            self.text_in(text, inner, Align::Left, color, false);
            if caret_visible {
                let x = inner.x + self.text_width(text, false) + 1.;
                self.caret(rect, x);
            }
        }
    }

    /// Convenience wrapper for [`Self::text_entry`] driven by a [`TextInputModel`].
    pub fn text_entry_model(
        &self,
        rect: Rect,
        text: &str,
        enabled: bool,
        model: &TextInputModel,
        time: f64,
    ) {
        self.text_entry(
            rect,
            text,
            enabled,
            model.should_draw_focus_edge(),
            model.should_draw_caret(time),
        );
    }

    /// `ComboBox`: a text entry with a drop-down arrow button on its right edge. A focused combo
    /// shows its text selected (SDK `ComboBox` selects all its text on focus).
    ///
    /// The text field is strictly bounded before the arrow button (`SetDrawWidth(wide - bwide)`
    /// per Source SDK `ComboBox`), truncating overflowing strings with an ellipsis (`...`) so text
    /// and selection highlights never collide with the arrow or exceed the control bounds.
    pub fn combo(&self, rect: Rect, text: &str, enabled: bool, armed: bool, focused: bool) {
        let button_w = rect.h;
        let button = Rect::new(
            rect.x + rect.w - button_w,
            rect.y + 1.,
            button_w - 1.,
            rect.h - 2.,
        );
        let text_inset = 3.;
        let text_padding_right = 2.;
        let max_text_w = (button.x - text_padding_right - (rect.x + text_inset)).max(0.);
        let inner = Rect::new(rect.x + text_inset, rect.y, max_text_w, rect.h);

        let display_text = self.truncate_ellipsis(text, max_text_w, false);
        let display_w = self.text_width(&display_text, false);

        self.fill(rect, self.theme.entry_bg);
        self.sunken(rect);

        if focused && enabled {
            let sel_w = (display_w + 2.).min(max_text_w + 1.);
            self.fill(
                Rect::new(inner.x - 1., rect.y + 3., sel_w, rect.h - 6.),
                self.theme.entry_selected_bg,
            );
            self.text_in(
                &display_text,
                inner,
                Align::Left,
                self.theme.entry_selected_text,
                false,
            );
        } else {
            let color = if enabled {
                self.theme.text
            } else {
                self.theme.text_disabled
            };
            self.text_in(&display_text, inner, Align::Left, color, false);
        }

        let color = if !enabled {
            self.theme.text_disabled
        } else if armed {
            self.theme.arrow_armed
        } else {
            self.theme.arrow
        };
        self.arrow_down(
            vec2(button.x + button.w * 0.5, button.y + button.h * 0.5),
            4.,
            color,
        );
    }

    /// The open drop-down list of a combo box (`rows` visible items starting at `first`).
    ///
    /// The list is fully opaque (owner decision): the frame color is laid down at full alpha
    /// before the translucent `Menu.BgColor` tint, so nothing behind the list shows through.
    pub fn menu_list(
        &self,
        rect: Rect,
        items: &[String],
        first: usize,
        highlight: Option<usize>,
        bar: Option<ScrollPart>,
    ) {
        self.fill(
            rect,
            Color {
                a: 1.,
                ..self.theme.frame_bg
            },
        );
        self.fill(rect, self.theme.menu_bg);
        self.raised(rect);
        let rows = (rect.h / MENU_ROW).floor() as usize;
        let has_scroll = items.len() > rows;
        let max_item_w = if has_scroll {
            (rect.w - 2. - 12. - SCROLLBAR_WIDTH).max(0.)
        } else {
            (rect.w - 2. - 12.).max(0.)
        };
        for (row, (index, item)) in items.iter().enumerate().skip(first).take(rows).enumerate() {
            let line = Rect::new(
                rect.x + 1.,
                rect.y + row as f32 * MENU_ROW,
                rect.w - 2.,
                MENU_ROW,
            );
            let color = if highlight == Some(index) {
                self.fill(line, self.theme.selection_bg);
                self.theme.selection_text
            } else {
                self.theme.text
            };
            let text_rect = Rect::new(line.x + 6., line.y, max_item_w, line.h);
            let display = self.truncate_ellipsis(item, max_item_w, false);
            self.text_in(&display, text_rect, Align::Left, color, false);
        }
        if has_scroll {
            self.scroll_bar(
                Rect::new(
                    rect.x + rect.w - SCROLLBAR_WIDTH,
                    rect.y,
                    SCROLLBAR_WIDTH,
                    rect.h,
                ),
                first,
                rows,
                items.len(),
                bar,
            );
        }
    }

    /// A vertical scroll bar showing `visible` of `total` rows starting at `first`.
    ///
    /// SDK `ScrollBar` layout ([`ScrollBarLayout`]): square arrow buttons at both ends (raised,
    /// sunken while `pressed`), the track (`ScrollBarSlider.BgColor`) between them and the nob
    /// (`ScrollBarSlider.FgColor` fill with a raised border).
    pub fn scroll_bar(
        &self,
        rect: Rect,
        first: usize,
        visible: usize,
        total: usize,
        pressed: Option<ScrollPart>,
    ) {
        let layout = ScrollBarLayout::new(rect, first, visible, total);
        self.fill(layout.track, self.theme.scrollbar_bg);
        for (button, part) in [(layout.up, ScrollPart::Up), (layout.down, ScrollPart::Down)] {
            let down = pressed == Some(part);
            if down {
                self.sunken(button);
            } else {
                self.raised(button);
            }
            let offset = if down { 1. } else { 0. };
            let center = vec2(
                button.x + button.w * 0.5 + offset,
                button.y + button.h * 0.5 + offset,
            );
            let half = (button.w * 0.25).round().max(2.);
            if part == ScrollPart::Up {
                self.arrow_up(center, half, self.theme.scrollbar_arrow);
            } else {
                self.arrow_down(center, half, self.theme.scrollbar_arrow);
            }
        }
        if layout.max_first == 0 {
            return;
        }
        if self.theme.scrollbar_nob.a > 0. {
            self.fill(layout.nob, self.theme.scrollbar_nob);
        }
        self.raised(layout.nob);
    }

    /// `Divider`: a sunken two-pixel line.
    pub fn divider(&self, rect: Rect) {
        self.sunken(Rect::new(rect.x, rect.y, rect.w, rect.h.max(2.)));
    }

    /// `URLButton`: link-colored, underlined text with armed and pressed states.
    pub fn url(&self, rect: Rect, label: &str, state: ButtonState) {
        // The text starts inside the focus box (3 units in), `FOCUS_PAD` clear of it.
        let rect = Rect::new(
            rect.x + LINK_TEXT_INSET,
            rect.y,
            rect.w - LINK_TEXT_INSET,
            rect.h,
        );
        if !state.enabled {
            self.label(label, rect, Align::Left, false);
            return;
        }
        let (color, offset) = if state.pressed {
            (self.theme.text, vec2(1., 1.))
        } else if state.armed {
            (self.theme.text, vec2(0., 0.))
        } else {
            (self.theme.url_text, vec2(0., 0.))
        };
        let target_rect = Rect::new(rect.x + offset.x, rect.y + offset.y, rect.w, rect.h);
        self.text_in(label, target_rect, Align::Left, color, false);
        let width = self.text_width(label, false);
        let y = target_rect.y + (target_rect.h + FONT_TALL) * 0.5 - 1.;
        self.fill(Rect::new(target_rect.x, y.round(), width, 1.), color);
    }

    /// A property-sheet tab drawn from [`Theme::tab_style`]; the active tab is open towards
    /// its page.
    pub fn tab(&self, rect: Rect, label: &str, active: bool, state: ButtonState) {
        let style = self.theme.tab_style(active, state);
        let color = style.text;
        if style.fill {
            self.fill(rect, self.theme.frame_bg);
        }
        if let Some(bg) = style.hover_bg {
            self.fill(rect, bg);
        }
        let r = self.map(rect);
        let t = self.scale.round().max(1.);
        self.canvas
            .rectangle(r.x, r.y, r.w, t, self.theme.border_bright);
        self.canvas
            .rectangle(r.x, r.y, t, r.h, self.theme.border_bright);
        self.canvas
            .rectangle(r.x + r.w - t, r.y, t, r.h, self.theme.border_dark);
        if !style.open_bottom {
            self.canvas
                .rectangle(r.x, r.y + r.h - t, r.w, t, self.theme.border_bright);
        }
        self.text_in(label, rect, Align::Center, color, false);
    }

    /// Draws a texture stretched over a layout rectangle.
    pub fn image(&self, rect: Rect, texture: &Texture2D) {
        let r = self.map(rect);
        draw_texture_ex(
            self.canvas,
            texture,
            r.x,
            r.y,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(r.w, r.h)),
                ..Default::default()
            },
        );
    }

    /// A Source VGUI segmented progress bar: sunken cavity border, transparent background,
    /// and segmented fill blocks (segment wide 8, gap 4, inset 4 per Source SDK `ProgressBar.cpp`).
    pub fn progress_bar(&self, rect: Rect, fraction: f32, fg: Color, bg: Color) {
        self.fill(rect, bg);
        self.sunken(rect);

        let inset = 4.;
        let seg_w = 8.;
        let seg_gap = 4.;
        let inner_x = rect.x + inset;
        let inner_y = rect.y + inset;
        let inner_w = (rect.w - 2. * inset).max(0.);
        let inner_h = (rect.h - 2. * inset).max(0.);
        if inner_w <= 0. || inner_h <= 0. {
            return;
        }

        let total_segs = ((inner_w + seg_gap) / (seg_w + seg_gap)).floor() as usize;
        let drawn_segs = ((total_segs as f32) * fraction.clamp(0.0, 1.0)).floor() as usize;
        for i in 0..drawn_segs {
            let seg_x = inner_x + (i as f32) * (seg_w + seg_gap);
            self.fill(Rect::new(seg_x, inner_y, seg_w, inner_h), fg);
        }
    }
}

/// Calculates the placed tooltip rectangle per Source SDK positioning and screen clamping rules.
pub fn tooltip_rect(pointer: Vec2, tip_size: Vec2, bounds: Rect) -> Rect {
    let mut x = pointer.x;
    let mut y = pointer.y + 20.;
    if x + tip_size.x > bounds.x + bounds.w {
        x = pointer.x - tip_size.x;
    }
    if y + tip_size.y > bounds.y + bounds.h {
        y = pointer.y - tip_size.y - 20.;
    }
    let x = x.clamp(bounds.x, (bounds.x + bounds.w - tip_size.x).max(bounds.x));
    let y = y.clamp(bounds.y, (bounds.y + bounds.h - tip_size.y).max(bounds.y));
    Rect::new(x, y, tip_size.x, tip_size.y)
}

/// Check box / radio indicator size and the label indent after it.
pub const CHECK_SIZE: f32 = 12.;
pub const LABEL_INDENT: f32 = 20.;
/// Track centre line inside a slider control and its tick count.
pub const SLIDER_TRACK_Y: f32 = 10.;
pub const SLIDER_TICKS: usize = 10;
/// Height of a slider's interactive band (track and nob).
pub const SLIDER_HIT_TALL: f32 = 22.;
/// Drop-down menu row height and the maximum rows shown before scrolling.
pub const MENU_ROW: f32 = 20.;
pub const MENU_MAX_ROWS: usize = 12;
/// Scroll bar width (`ScrollBar.Wide` in the installed scheme); the arrow buttons are squares of
/// this size.
pub const SCROLLBAR_WIDTH: f32 = 17.;
/// Press-and-hold repeat timing (SDK `ScrollBar` arrow buttons), shared by every held control
/// and key: first repeat after this delay, then one per interval (seconds).
pub const HOLD_REPEAT_DELAY: f64 = 0.4;
pub const HOLD_REPEAT_INTERVAL: f64 = 0.05;

/// Repeats a held control: the press acts once, then [`HOLD_REPEAT_DELAY`] later it fires again
/// and every [`HOLD_REPEAT_INTERVAL`] after that, at most once a frame. Without a frame time
/// (scripted input) nothing repeats.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HoldRepeat {
    next: Option<f64>,
}

impl HoldRepeat {
    /// The control was pressed at `now`: arm the first repeat.
    pub fn press(&mut self, now: Option<f64>) {
        self.next = now.map(|t| t + HOLD_REPEAT_DELAY);
    }

    /// The control is still held at `now`: whether it fires again this frame.
    pub fn held(&mut self, now: Option<f64>) -> bool {
        match (now, self.next) {
            (Some(t), Some(at)) if t >= at => {
                self.next = Some(at + HOLD_REPEAT_INTERVAL);
                true
            }
            _ => false,
        }
    }

    /// The control was released (or the repeat is cancelled).
    pub fn release(&mut self) {
        self.next = None;
    }
}

/// A part of a vertical scroll bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollPart {
    Up,
    Down,
    Nob,
    /// The track above the nob.
    PageUp,
    /// The track below the nob.
    PageDown,
}

/// Geometry of a vertical scroll bar (SDK `ScrollBar::PerformLayout` / `ScrollBarSlider`):
/// square arrow buttons at the ends, the track between them, and the nob sized to the visible
/// share of the rows but never shorter than the bar is wide.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollBarLayout {
    pub up: Rect,
    pub down: Rect,
    pub track: Rect,
    pub nob: Rect,
    pub max_first: usize,
}

impl ScrollBarLayout {
    pub fn new(rect: Rect, first: usize, visible: usize, total: usize) -> Self {
        let button = rect.w.min(rect.h * 0.5);
        let up = Rect::new(rect.x, rect.y, rect.w, button);
        let down = Rect::new(rect.x, rect.y + rect.h - button, rect.w, button);
        let track = Rect::new(
            rect.x,
            rect.y + button,
            rect.w,
            (rect.h - 2. * button).max(0.),
        );
        let max_first = total.saturating_sub(visible);
        let nob_h = if total == 0 {
            track.h
        } else {
            (track.h * visible as f32 / total as f32).clamp(rect.w.min(track.h), track.h)
        };
        let fraction = first.min(max_first) as f32 / max_first.max(1) as f32;
        let nob = Rect::new(
            track.x,
            track.y + (track.h - nob_h) * fraction,
            track.w,
            nob_h,
        );
        Self {
            up,
            down,
            track,
            nob,
            max_first,
        }
    }

    /// The part under `point`, if it is on the bar.
    pub fn part_at(&self, point: Vec2) -> Option<ScrollPart> {
        if self.up.contains(point) {
            Some(ScrollPart::Up)
        } else if self.down.contains(point) {
            Some(ScrollPart::Down)
        } else if self.nob.contains(point) {
            Some(ScrollPart::Nob)
        } else if self.track.contains(point) {
            Some(if point.y < self.nob.y {
                ScrollPart::PageUp
            } else {
                ScrollPart::PageDown
            })
        } else {
            None
        }
    }

    /// The first row for a nob whose top edge is at `top`.
    pub fn first_for_nob_top(&self, top: f32) -> usize {
        let travel = self.track.h - self.nob.h;
        if travel <= 0. {
            return 0;
        }
        let fraction = ((top - self.track.y) / travel).clamp(0., 1.);
        (fraction * self.max_first as f32).round() as usize
    }
}

/// Pointer gesture on a scroll bar, following SDK `ScrollBar`: an arrow button steps one row on
/// press and repeats while held ([`HoldRepeat`]); the nob follows the pointer while dragged; a press on the track
/// pages once by the visible rows. The bar owns the pointer until the release.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollBarState {
    pressed: Option<ScrollPart>,
    /// Pointer offset from the nob's top edge when the drag started.
    grab: f32,
    /// The held arrow button's repeat.
    repeat: HoldRepeat,
}

impl ScrollBarState {
    /// Whether a gesture that started on the bar is in progress.
    pub fn active(&self) -> bool {
        self.pressed.is_some()
    }

    /// The part being pressed, for drawing.
    pub fn pressed(&self) -> Option<ScrollPart> {
        self.pressed
    }

    /// Whether this frame belongs to the bar: a gesture is in progress, or a press lands on it.
    pub fn wants(&self, layout: &ScrollBarLayout, point: Option<Vec2>, click: bool) -> bool {
        self.active() || (click && point.and_then(|p| layout.part_at(p)).is_some())
    }

    /// Feeds one frame of pointer state; returns the new first row when it changes.
    /// Without a frame time (`now`) a held arrow button does not repeat.
    #[allow(clippy::too_many_arguments)]
    pub fn pointer(
        &mut self,
        layout: &ScrollBarLayout,
        point: Option<Vec2>,
        down: bool,
        click: bool,
        now: Option<f64>,
        first: usize,
        page: usize,
    ) -> Option<usize> {
        let max = layout.max_first;
        let step = |part: ScrollPart| match part {
            ScrollPart::Up => first.saturating_sub(1),
            ScrollPart::Down => (first + 1).min(max),
            ScrollPart::PageUp => first.saturating_sub(page.max(1)),
            ScrollPart::PageDown => (first + page.max(1)).min(max),
            ScrollPart::Nob => first,
        };
        let target = if click {
            let part = point.and_then(|p| layout.part_at(p));
            self.pressed = part;
            self.repeat.release();
            match (part, point) {
                (Some(ScrollPart::Nob), Some(p)) => {
                    self.grab = p.y - layout.nob.y;
                    None
                }
                (Some(part @ (ScrollPart::Up | ScrollPart::Down)), _) => {
                    self.repeat.press(now);
                    Some(step(part))
                }
                (Some(part), _) => Some(step(part)),
                (None, _) => None,
            }
        } else if down {
            match (self.pressed, point) {
                (Some(ScrollPart::Nob), Some(p)) => Some(layout.first_for_nob_top(p.y - self.grab)),
                (Some(part @ (ScrollPart::Up | ScrollPart::Down)), _) => {
                    self.repeat.held(now).then(|| step(part))
                }
                _ => None,
            }
        } else {
            None
        };
        if !down {
            self.pressed = None;
            self.repeat.release();
        }
        target.filter(|t| *t != first)
    }
}

/// Rectangle of a combo box's open list below the combo.
pub fn menu_list_rect(combo: Rect, items: usize) -> Rect {
    let rows = items.clamp(1, MENU_MAX_ROWS);
    Rect::new(combo.x, combo.y + combo.h, combo.w, rows as f32 * MENU_ROW)
}

/// Longest gap between two clicks on the same target that counts as a double click. This is the
/// common desktop default; the retail interval comes from the operating system and is not
/// reproduced exactly.
pub const DOUBLE_CLICK_SECONDS: f64 = 0.5;

/// Detects double clicks: two clicks on the same target within [`DOUBLE_CLICK_SECONDS`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DoubleClick<T: Copy + PartialEq> {
    last: Option<(T, f64)>,
}

impl<T: Copy + PartialEq> DoubleClick<T> {
    /// Records a click on `target` at `time` and returns whether it completes a double click.
    /// A double click is consumed, so a third quick click starts a new pair. Without a time
    /// nothing is a double click.
    pub fn register(&mut self, target: T, time: Option<f64>) -> bool {
        let Some(time) = time else {
            self.last = None;
            return false;
        };
        let double = matches!(
            self.last,
            Some((previous, at)) if previous == target && (0. ..=DOUBLE_CLICK_SECONDS).contains(&(time - at))
        );
        self.last = (!double).then_some((target, time));
        double
    }
}

/// How wheel input scrolls lists on this platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollMode {
    /// Mouse wheel notches (Windows, Linux): each frame's wheel value is rounded to whole
    /// notches, and each notch scrolls a fixed number of rows.
    Notched,
    /// Trackpad (macOS): small deltas accumulate across frames and scroll one row per whole
    /// step, so slow two-finger motion is not lost.
    Smooth,
}

impl ScrollMode {
    /// The scroll mode of the platform this build targets.
    pub const fn platform() -> Self {
        if cfg!(target_os = "macos") {
            Self::Smooth
        } else {
            Self::Notched
        }
    }

    /// Rows scrolled per step: `notch_rows` per wheel notch, one row per trackpad step.
    pub const fn rows_per_step(self, notch_rows: usize) -> usize {
        match self {
            Self::Notched => notch_rows,
            Self::Smooth => 1,
        }
    }
}

/// Turns per-frame wheel values (positive up) into whole scroll steps for a [`ScrollMode`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollAccumulator {
    carry: f32,
}

impl ScrollAccumulator {
    /// Whole steps for this frame. Smooth mode keeps the remainder for later frames and drops
    /// it when the direction changes; notched mode rounds each frame on its own.
    pub fn steps(&mut self, wheel: f32, mode: ScrollMode) -> i32 {
        match mode {
            ScrollMode::Notched => {
                self.carry = 0.;
                wheel.round() as i32
            }
            ScrollMode::Smooth => {
                if wheel == 0. {
                    return 0;
                }
                if self.carry != 0. && self.carry.signum() != wheel.signum() {
                    self.carry = 0.;
                }
                self.carry += wheel;
                let steps = self.carry.trunc();
                self.carry -= steps;
                steps as i32
            }
        }
    }
}

/// What the pointer is over while a popup list is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupHit {
    /// A list entry.
    Item(usize),
    /// Inside the list but not on an entry (scroll bar, empty rows).
    List,
    /// The control that opened the list (a combo box header).
    Anchor,
    /// Anywhere else.
    Outside,
}

/// Result of a pointer gesture on an open popup list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupEvent {
    /// An entry was chosen; the list closes.
    Select(usize),
    /// The list closes without a selection (anchor or outside click).
    Close,
}

/// An open popup list (a combo box drop-down, or any future popup menu) anchored to a control.
///
/// While open it is the topmost input layer: its owner routes all pointer input here and
/// occludes the layers below ([`ButtonTracker::occlude`]). Every close path acts on the release
/// of a gesture that started on the same kind of target, so one click never closes the list
/// and lets its remainder reach, or reopen from, the control underneath: releasing on an entry
/// after pressing in the list selects it (SDK `MenuItem` activates on release), releasing on
/// the anchor after pressing it closes the list, and releasing outside after pressing outside
/// closes it without activating anything.
#[derive(Clone, Debug, PartialEq)]
pub struct PopupList {
    pub anchor: Rect,
    pub list: Rect,
    pub items: usize,
    pub first: usize,
    pub highlight: Option<usize>,
    pressed: Option<PopupHit>,
    was_down: bool,
    wheel: ScrollAccumulator,
    bar: ScrollBarState,
}

impl PopupList {
    /// Opens below `anchor` with the `selected` entry highlighted and scrolled into view.
    pub fn below(anchor: Rect, items: usize, selected: Option<usize>) -> Self {
        let mut popup = Self {
            anchor,
            list: menu_list_rect(anchor, items),
            items,
            first: 0,
            highlight: selected,
            pressed: None,
            was_down: false,
            wheel: ScrollAccumulator::default(),
            bar: ScrollBarState::default(),
        };
        if let Some(index) = selected {
            popup.reveal(index);
        }
        popup
    }

    /// Visible rows.
    pub fn rows(&self) -> usize {
        ((self.list.h / MENU_ROW).floor() as usize).max(1)
    }

    /// Scrolls so that `index` is visible.
    pub fn reveal(&mut self, index: usize) {
        let rows = self.rows();
        if index < self.first {
            self.first = index;
        } else if index >= self.first + rows {
            self.first = index + 1 - rows;
        }
    }

    /// Scrolls by one frame of wheel input (positive up), one row per step.
    pub fn wheel(&mut self, wheel: f32, mode: ScrollMode) {
        let steps = self.wheel.steps(wheel, mode);
        self.scroll(steps);
    }

    /// Scrolls by whole steps; positive scrolls up.
    pub fn scroll(&mut self, steps: i32) {
        let max = self.items.saturating_sub(self.rows()) as i32;
        self.first = (self.first as i32 - steps).clamp(0, max) as usize;
    }

    /// Moves the highlight by `delta` entries (arrow keys).
    pub fn step(&mut self, delta: i32) {
        if self.items == 0 {
            return;
        }
        let current = self.highlight.map_or(-1, |i| i as i32);
        let next = (current + delta).clamp(0, self.items as i32 - 1) as usize;
        self.highlight = Some(next);
        self.reveal(next);
    }

    /// What lies under `point`; `None` when the pointer is not over the canvas.
    pub fn hit(&self, point: Option<Vec2>) -> Option<PopupHit> {
        let p = point?;
        if self.list.contains(p) {
            let on_scroll_bar =
                self.items > self.rows() && p.x >= self.list.x + self.list.w - SCROLLBAR_WIDTH;
            let item = (!on_scroll_bar)
                .then(|| menu_list_hit(self.list, self.first, self.items, p))
                .flatten();
            return Some(item.map_or(PopupHit::List, PopupHit::Item));
        }
        Some(if self.anchor.contains(p) {
            PopupHit::Anchor
        } else {
            PopupHit::Outside
        })
    }

    /// The scroll bar along the right edge when the entries do not all fit.
    pub fn bar_layout(&self) -> Option<ScrollBarLayout> {
        (self.items > self.rows()).then(|| {
            ScrollBarLayout::new(
                Rect::new(
                    self.list.x + self.list.w - SCROLLBAR_WIDTH,
                    self.list.y,
                    SCROLLBAR_WIDTH,
                    self.list.h,
                ),
                self.first,
                self.rows(),
                self.items,
            )
        })
    }

    /// The scroll bar part being pressed, for drawing.
    pub fn bar_pressed(&self) -> Option<ScrollPart> {
        self.bar.pressed()
    }

    /// Feeds one frame of pointer state. Hovering an entry highlights it; see the type docs
    /// for when a gesture selects or closes. A gesture that starts on the scroll bar scrolls
    /// (live, with arrow repeat at `now`) and never selects or closes.
    pub fn pointer(
        &mut self,
        point: Option<Vec2>,
        down: bool,
        click: bool,
        released: bool,
        now: Option<f64>,
    ) -> Option<PopupEvent> {
        let bar = self
            .bar_layout()
            .filter(|layout| self.bar.wants(layout, point, click));
        if let Some(layout) = bar {
            let rows = self.rows();
            let held = down && !released;
            if let Some(first) = self
                .bar
                .pointer(&layout, point, held, click, now, self.first, rows)
            {
                self.first = first;
            }
            self.pressed = None;
            self.was_down = held;
            return None;
        }
        let hit = self.hit(point);
        if let Some(PopupHit::Item(index)) = hit {
            self.highlight = Some(index);
        }
        let mut event = None;
        if released || (self.was_down && !down) {
            event = match (self.pressed.take(), hit) {
                (Some(PopupHit::Item(_) | PopupHit::List), Some(PopupHit::Item(index))) => {
                    Some(PopupEvent::Select(index))
                }
                (Some(PopupHit::Anchor), Some(PopupHit::Anchor))
                | (Some(PopupHit::Outside), Some(PopupHit::Outside)) => Some(PopupEvent::Close),
                _ => None,
            };
        } else if click || (down && !self.was_down) {
            self.pressed = hit;
        }
        self.was_down = down;
        event
    }
}

/// Item index under a point in an open list, if any.
pub fn menu_list_hit(list: Rect, first: usize, items: usize, point: Vec2) -> Option<usize> {
    if !list.contains(point) {
        return None;
    }
    let index = first + ((point.y - list.y) / MENU_ROW).floor() as usize;
    (index < items).then_some(index)
}

/// Greedy word wrap by measured width; words longer than `width` are broken by character
/// so lines never exceed `width`.
pub fn wrap(text: &str, width: f32, measure: impl Fn(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && measure(&candidate) > width {
                lines.push(std::mem::take(&mut line));
            }
            if line.is_empty() {
                if measure(word) <= width || width <= 0. {
                    line = word.to_owned();
                } else {
                    // Very long word exceeds max width: break by character
                    let mut chunk = String::new();
                    for ch in word.chars() {
                        let mut next = chunk.clone();
                        next.push(ch);
                        if !chunk.is_empty() && measure(&next) > width {
                            lines.push(std::mem::take(&mut chunk));
                        }
                        chunk.push(ch);
                    }
                    if !chunk.is_empty() {
                        line = chunk;
                    }
                }
            } else {
                line = candidate;
            }
        }
        lines.push(line);
    }
    lines
}

/// Truncates `text` with an ellipsis (`"..."`) so that its measured width does not exceed `max_w`
/// (Source SDK `TextEntry::NeedsEllipses`).
///
/// If `text` fits within `max_w`, it is returned unchanged without allocation (`Cow::Borrowed`).
/// If `max_w` cannot even fit the ellipsis, as many characters as can fit without an ellipsis are returned.
pub fn truncate_ellipsis<'a>(
    text: &'a str,
    max_w: f32,
    measure: impl Fn(&str) -> f32,
) -> Cow<'a, str> {
    if max_w <= 0. || text.is_empty() {
        return if max_w <= 0. && !text.is_empty() {
            Cow::Borrowed("")
        } else {
            Cow::Borrowed(text)
        };
    }
    if measure(text) <= max_w {
        return Cow::Borrowed(text);
    }

    const ELLIPSIS: &str = "...";
    let dots_w = measure(ELLIPSIS);

    if max_w >= dots_w {
        let budget = max_w - dots_w;
        let mut last_valid_end = 0;
        for (i, ch) in text.char_indices() {
            let next_end = i + ch.len_utf8();
            if measure(&text[..next_end]) <= budget {
                last_valid_end = next_end;
            } else {
                break;
            }
        }
        if last_valid_end == 0 {
            Cow::Borrowed(ELLIPSIS)
        } else {
            Cow::Owned(format!("{}{}", &text[..last_valid_end], ELLIPSIS))
        }
    } else {
        // Not even the ellipsis fits. Fit as many characters as possible without it.
        let mut last_valid_end = 0;
        for (i, ch) in text.char_indices() {
            let next_end = i + ch.len_utf8();
            if measure(&text[..next_end]) <= max_w {
                last_valid_end = next_end;
            } else {
                break;
            }
        }
        Cow::Borrowed(&text[..last_valid_end])
    }
}

/// Continuous slider model: maps a value range to the track and back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderRange {
    pub min: f32,
    pub max: f32,
}

impl SliderRange {
    pub const fn new(min: f32, max: f32) -> Self {
        Self { min, max }
    }

    pub fn fraction(self, value: f32) -> f32 {
        if self.max == self.min {
            return 0.;
        }
        ((value - self.min) / (self.max - self.min)).clamp(0., 1.)
    }

    pub fn value(self, fraction: f32) -> f32 {
        self.min + (self.max - self.min) * fraction.clamp(0., 1.)
    }

    pub fn clamp(self, value: f32) -> f32 {
        value.clamp(self.min.min(self.max), self.min.max(self.max))
    }
}

/// Slider geometry inside its control rectangle: the track spans the width minus the nob.
pub const NOB_WIDTH: f32 = 8.;

pub fn slider_track(rect: Rect) -> (f32, f32) {
    // SDK `Slider::GetTrackRect` leaves the nob's width free at the right end, which keeps the
    // slider clear of a control placed right after it (the Mouse sensitivity text field).
    (rect.x + NOB_WIDTH * 0.5, rect.x + rect.w - NOB_WIDTH * 1.5)
}

/// Space kept between a control's content and its focus ring.
pub const FOCUS_PAD: f32 = 2.;
/// Where a link's text starts in its control: past the left edge of the focus box
/// (`Painter::focus_box`, 3 units in) by `FOCUS_PAD`.
pub const LINK_TEXT_INSET: f32 = 3. + FOCUS_PAD;

/// What a check box or radio button draws: the box or circle and a label `label_width` wide,
/// with the same margin beside the box and after the label as above and below the box, so a
/// ring around it sits evenly.
pub fn toggle_content(rect: Rect, label_width: f32) -> Rect {
    let height = FONT_TALL.max(CHECK_SIZE);
    let margin = (height - CHECK_SIZE) * 0.5;
    let left = rect.x + 2. - margin;
    let right = (rect.x + LABEL_INDENT + label_width).min(rect.x + rect.w) + margin;
    Rect::new(left, rect.y + (rect.h - height) * 0.5, right - left, height)
}

/// What a slider draws: its track with the nob at both ends, from the nob's top to the bottom
/// of the captions (of the tick marks when it has no captions).
pub fn slider_content(rect: Rect) -> Rect {
    let (start, end) = slider_track(rect);
    let top = rect.y + SLIDER_TRACK_Y - 8.;
    let bottom = if rect.h >= 32. {
        rect.y + SLIDER_TRACK_Y + 12. + FONT_TALL
    } else {
        rect.y + SLIDER_TRACK_Y + 12.
    };
    Rect::new(
        start - NOB_WIDTH * 0.5,
        top,
        end - start + NOB_WIDTH,
        bottom - top,
    )
}

/// Fraction of a pointer x position on a slider track.
pub fn slider_fraction_at(rect: Rect, x: f32) -> f32 {
    let (start, end) = slider_track(rect);
    if end <= start {
        return 0.;
    }
    ((x - start) / (end - start)).clamp(0., 1.)
}

/// Formats a slider entry the way the retail pages do (`" %.2f"`, shown trimmed).
pub fn format_value(value: f32) -> String {
    format!("{value:.2}")
}

/// Parses a typed entry value, rejecting non-finite input.
pub fn parse_value(text: &str) -> Option<f32> {
    text.trim().parse::<f32>().ok().filter(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_repeat_fires_after_the_delay_then_every_interval() {
        let mut repeat = HoldRepeat::default();
        repeat.press(Some(10.));
        assert!(!repeat.held(Some(10.39)), "not before the delay");
        assert!(repeat.held(Some(10.401)));
        assert!(!repeat.held(Some(10.42)));
        assert!(repeat.held(Some(10.451)));
        // A slow frame fires once; missed repeats follow one per frame.
        assert!(repeat.held(Some(11.)));
        assert!(repeat.held(Some(11.)));
        repeat.release();
        assert!(!repeat.held(Some(20.)), "released");
        repeat.press(None);
        assert!(!repeat.held(None), "no frame time, no repeat");
        assert!(!repeat.held(Some(20.)));
    }

    #[test]
    fn slider_value_and_fraction_round_trip_and_clamp() {
        let range = SliderRange::new(0.1, 6.0);
        assert!((range.value(range.fraction(3.0)) - 3.0).abs() < 1e-5);
        assert_eq!(range.fraction(-5.), 0.);
        assert_eq!(range.fraction(50.), 1.);
        assert_eq!(SliderRange::new(1., 1.).fraction(1.), 0.);
        // Track 14..106: half a nob in from the left, a nob and a half from the right.
        let rect = Rect::new(10., 0., 108., 20.);
        assert_eq!(slider_track(rect), (14., 106.));
        assert_eq!(slider_fraction_at(rect, 0.), 0.);
        assert_eq!(slider_fraction_at(rect, 60.), 0.5);
        assert_eq!(slider_fraction_at(rect, 500.), 1.);
    }

    #[test]
    fn entry_values_format_with_two_decimals_and_reject_garbage() {
        assert_eq!(format_value(3.0), "3.00");
        assert_eq!(format_value(1.054), "1.05");
        assert_eq!(parse_value(" 2.5 "), Some(2.5));
        assert_eq!(parse_value("NaN"), None);
        assert_eq!(parse_value("abc"), None);
    }

    #[test]
    fn wrap_breaks_on_measured_width() {
        let lines = wrap("Adjust the slider until all three", 60., |s| {
            s.len() as f32 * 5.
        });
        assert_eq!(lines, ["Adjust the", "slider until", "all three"]);
        assert_eq!(wrap("", 10., |_| 0.), [""]);

        // Very long word exceeding width is broken character-by-character
        let long_word_lines = wrap("Supercalifragilistic", 30., |s| s.len() as f32 * 5.);
        assert_eq!(long_word_lines, ["Superc", "alifra", "gilist", "ic"]);
        for line in &long_word_lines {
            assert!(line.len() as f32 * 5. <= 30.);
        }

        // Localized multi-byte UTF-8 string with umlauts and symbols
        let loc_lines = wrap(
            "Urheberrecht © 1991-2001 RAD Game Tools. Überragend!",
            80.,
            |s| s.chars().count() as f32 * 5.,
        );
        for line in &loc_lines {
            assert!(line.chars().count() as f32 * 5. <= 80.);
        }

        // Explicit newlines preserved
        let multiline = wrap("First line\nSecond line is longer", 50., |s| {
            s.len() as f32 * 5.
        });
        assert_eq!(multiline, ["First line", "Second", "line is", "longer"]);

        // Zero width falls back safely
        let zero_lines = wrap("hello world", 0., |_| 10.);
        assert_eq!(zero_lines, ["hello", "world"]);
    }

    #[test]
    fn scheme_colors_resolve_names_and_keep_defaults() {
        let roots = source_assets::keyvalues::parse_resource(
            br#"Scheme { Colors { Orange "255 155 0 255" } BaseSettings { Frame.BgColor "1 2 3 4" ListPanel.SelectedBgColor Orange } }"#,
        )
        .unwrap();
        let theme = Theme::from_scheme(&roots[0]);
        assert_eq!(theme.frame_bg, Color::from_rgba(1, 2, 3, 4));
        assert_eq!(theme.frame_bg_focused, Color::from_rgba(1, 2, 3, 4));
        assert_eq!(theme.selection_bg, Color::from_rgba(255, 155, 0, 255));
        assert_eq!(theme.slider_nob, Theme::default().slider_nob);
    }

    #[test]
    fn focus_faded_transitions_are_reversible_and_preserve_focused_colors() {
        let theme = Theme::default();
        let focused = theme.focus_faded(1.0, true);
        assert_eq!(focused.frame_bg, theme.frame_bg_focused);
        assert_eq!(focused.title_text, theme.title_text_focused);

        let unfocused = focused.focus_faded(0.0, false);
        assert_eq!(unfocused.frame_bg, theme.frame_bg_out_of_focus);
        assert_eq!(unfocused.title_text, theme.title_text_out_of_focus);

        // Crucial test: fading an already-unfocused theme back to focused restores the focused colors:
        let restored = unfocused.focus_faded(1.0, true);
        assert_eq!(restored.frame_bg, theme.frame_bg_focused);
        assert_eq!(restored.title_text, theme.title_text_focused);
    }

    #[test]
    fn toggle_labels_rest_between_dull_and_hover_text() {
        let theme = Theme::default();
        let rest = theme.toggle_text(ButtonState::new(true));
        let hover = theme.toggle_text(ButtonState::active(true, true, false));
        let pressed = theme.toggle_text(ButtonState::active(true, false, true));
        assert!(rest.r > theme.text_dull.r && rest.r < hover.r, "{rest:?}");
        assert_eq!((hover, pressed), (theme.text, theme.text));
        assert_eq!(
            theme.toggle_text(ButtonState::new(false)),
            theme.text_disabled
        );
        let roots = source_assets::keyvalues::parse_resource(
            br#"Scheme { Colors { OffWhite "221 221 221 255" } BaseSettings { Label.TextColor OffWhite } }"#,
        )
        .unwrap();
        assert_eq!(
            Theme::from_scheme(&roots[0]).label_text,
            Color::from_rgba(221, 221, 221, 255)
        );
    }

    /// The desktop scheme's push button entries, with the console-only armed color.
    const BUTTON_SCHEME: &[u8] = br#"Scheme { Colors { White "255 255 255 255" Blank "0 0 0 0" }
        BaseSettings {
            Button.TextColor "White"
            Button.BgColor "Blank"
            Button.ArmedTextColor "White"
            Button.ArmedBgColor "Blank" [$WIN32]
            Button.ArmedBgColor "190 115 0 255" [$X360]
            Button.DepressedTextColor "White"
            Button.DepressedBgColor "Blank"
        } }"#;

    fn button_states() -> [(ButtonState, &'static str); 6] {
        [
            (ButtonState::active(true, false, false), "normal"),
            (ButtonState::active(true, true, false), "hover"),
            (ButtonState::active(true, true, true), "pressed over"),
            (
                ButtonState::active(true, false, true),
                "pressed, dragged off",
            ),
            (ButtonState::active(false, true, false), "disabled hover"),
            (ButtonState::active(false, true, true), "disabled pressed"),
        ]
    }

    #[test]
    fn desktop_scheme_push_buttons_hover_with_the_menu_fill() {
        let roots = source_assets::keyvalues::parse_resource(BUTTON_SCHEME).unwrap();
        let theme = Theme::from_scheme(&roots[0]);
        assert_eq!(
            theme,
            Theme::default(),
            "defaults mirror the desktop scheme"
        );
        assert_eq!(
            theme.button_armed_bg, theme.button_bg,
            "retail: no armed background"
        );
        let normal = theme.button_style(ButtonState::new(true));
        let hover = theme.button_style(ButtonState::active(true, true, false));
        assert_eq!(
            hover.bg, theme.menu_bg,
            "slight hover fill from Menu.BgColor"
        );
        assert_eq!((hover.border, hover.text), (normal.border, normal.text));
        assert_ne!(hover, normal);
        // Pressed and disabled keep their own look.
        let pressed = theme.button_style(ButtonState::active(true, true, true));
        assert_eq!(pressed.bg, theme.button_depressed_bg);
        let disabled = theme.button_style(ButtonState::active(false, true, false));
        assert_eq!(disabled, theme.button_style(ButtonState::new(false)));
    }

    #[test]
    fn button_style_resolves_hover_pressed_and_disabled_from_scheme_colors() {
        let rgba = Color::from_rgba;
        let theme = Theme {
            button_text: rgba(1, 1, 1, 255),
            button_bg: rgba(2, 2, 2, 0),
            button_armed_text: rgba(3, 3, 3, 255),
            button_armed_bg: rgba(4, 4, 4, 255),
            button_depressed_text: rgba(5, 5, 5, 255),
            button_depressed_bg: rgba(6, 6, 6, 255),
            ..Theme::default()
        };
        let normal = (ButtonBorder::Raised, theme.button_text, theme.button_bg, 0.);
        let hover = (
            ButtonBorder::Raised,
            theme.button_armed_text,
            theme.button_armed_bg,
            0.,
        );
        let pressed = (
            ButtonBorder::Depressed,
            theme.button_depressed_text,
            theme.button_depressed_bg,
            1.,
        );
        let expected = [normal, hover, pressed, pressed, normal, normal];
        for ((state, name), want) in button_states().into_iter().zip(expected) {
            let style = theme.button_style(state);
            assert_eq!(
                (style.border, style.text, style.bg, style.text_offset),
                want,
                "{name}"
            );
        }
        // The default button keeps its selection border while hovered; pressing replaces it
        // with the depressed border, and a disabled default button shows neither.
        let mut default = ButtonState::active(true, true, false);
        default.default = true;
        assert_eq!(theme.button_style(default).border, ButtonBorder::KeyFocus);
        assert_eq!(theme.button_style(default).text, theme.button_armed_text);
        default.pressed = true;
        assert_eq!(theme.button_style(default).border, ButtonBorder::Depressed);
        default.enabled = false;
        assert_eq!(theme.button_style(default).border, ButtonBorder::Raised);
    }

    #[test]
    fn only_inactive_enabled_tabs_show_a_hover_fill() {
        let theme = Theme::default();
        // Active tab: hover, press and disabled never change it.
        let active = theme.tab_style(true, ButtonState::new(true));
        assert_eq!(active.hover_bg, None);
        for (state, name) in button_states() {
            assert_eq!(theme.tab_style(true, state), active, "active, {name}");
        }
        // Inactive tab: the hover fill appears while the pointer is over it and enabled.
        let expected = [
            None,
            Some(theme.menu_bg),
            Some(theme.menu_bg),
            None,
            None,
            None,
        ];
        for ((state, name), want) in button_states().into_iter().zip(expected) {
            let style = theme.tab_style(false, state);
            assert_eq!(style.hover_bg, want, "inactive, {name}");
            assert_eq!(style.text, theme.text_dull, "inactive, {name}");
        }
        assert_eq!(
            theme.tab_style(true, ButtonState::new(true)).text,
            theme.text
        );
        assert_eq!(
            theme.tab_style(false, ButtonState::new(true)).text,
            theme.text_dull
        );
    }

    #[test]
    fn hover_look_draws_through_the_shared_button_path() {
        let canvas = Canvas::default();
        let fonts = Fonts::default();
        let theme = Theme {
            button_armed_bg: Color::from_rgba(4, 4, 4, 255),
            ..Theme::default()
        };
        let painter = Painter {
            canvas: &canvas,
            fonts: &fonts,
            theme: &theme,
            origin: Vec2::ZERO,
            scale: 1.,
        };
        let rect = Rect::new(10., 10., 80., 24.);
        painter.button(rect, "", ButtonState::new(true));
        let normal = canvas.drain();
        painter.button(rect, "", ButtonState::active(true, true, false));
        let hover = canvas.drain();
        assert_eq!(hover.len(), normal.len() + 1, "armed background fill");
        painter.button(rect, "", ButtonState::active(false, true, false));
        assert_eq!(canvas.drain().len(), normal.len(), "disabled: no hover");
    }

    #[test]
    fn capturing_tracker_keeps_press_through_drag_off_and_activates_on_return() {
        let mut tracker = ButtonTracker::<usize>::capturing();
        tracker.update(Some(0), true, true, false);
        // Dragged off (nothing or another control under the pointer): still pressed, and
        // the other control cannot arm while the mouse is captured.
        assert_eq!(tracker.update(None, true, false, false), None);
        assert_eq!(tracker.pressed(), Some(0));
        assert_eq!(tracker.update(Some(1), true, false, false), None);
        assert_eq!(tracker.pressed(), Some(0));
        assert_eq!(tracker.armable(Some(1)), None);
        assert_eq!(tracker.armable(Some(0)), Some(0));
        // Back on and released over it: activates.
        tracker.update(Some(0), true, false, false);
        assert_eq!(tracker.update(Some(0), false, false, true), Some(0));
        assert_eq!(tracker.pressed(), None);
        assert_eq!(tracker.armable(Some(1)), Some(1), "capture ends on release");
        // Released while off: no activation.
        tracker.update(Some(0), true, true, false);
        tracker.update(Some(1), true, false, false);
        assert_eq!(tracker.update(Some(1), false, false, true), None);
        assert_eq!(tracker.pressed(), None);
        // A press that starts on nothing captures nothing.
        tracker.update(None, true, true, false);
        tracker.update(Some(2), true, false, false);
        assert_eq!(tracker.update(Some(2), false, false, true), None);
    }

    #[test]
    fn popup_list_closes_and_selects_only_on_release_of_its_own_gesture() {
        let anchor = Rect::new(0., 0., 100., 24.);
        let mut popup = PopupList::below(anchor, 3, Some(1));
        assert_eq!(popup.list, Rect::new(0., 24., 100., 60.));
        assert_eq!(popup.highlight, Some(1));
        let entry = |i: usize| Some(vec2(10., 24. + MENU_ROW * (i as f32 + 0.5)));
        let header = Some(vec2(10., 10.));
        let outside = Some(vec2(300., 300.));
        let gesture = |popup: &mut PopupList, press, release| {
            let mut events = vec![popup.pointer(press, true, true, false, None)];
            events.push(popup.pointer(press, true, false, false, None));
            events.push(popup.pointer(release, false, false, true, None));
            events
        };
        assert_eq!(popup.hit(entry(2)), Some(PopupHit::Item(2)));
        assert_eq!(popup.hit(header), Some(PopupHit::Anchor));
        assert_eq!(popup.hit(outside), Some(PopupHit::Outside));
        assert_eq!(popup.hit(None), None);
        // Entry: nothing on press or hold, selection on release; hover moves the highlight.
        let events = gesture(&mut popup, entry(2), entry(2));
        assert_eq!(events, vec![None, None, Some(PopupEvent::Select(2))]);
        assert_eq!(popup.highlight, Some(2));
        // Press on one entry, release on another: the released entry is chosen.
        let events = gesture(&mut popup, entry(0), entry(1));
        assert_eq!(events[2], Some(PopupEvent::Select(1)));
        // Header and outside: close on release only.
        assert_eq!(
            gesture(&mut popup, header, header),
            vec![None, None, Some(PopupEvent::Close)]
        );
        assert_eq!(
            gesture(&mut popup, outside, outside),
            vec![None, None, Some(PopupEvent::Close)]
        );
        // Mixed gestures do nothing: the list stays open.
        assert_eq!(gesture(&mut popup, outside, entry(0))[2], None);
        assert_eq!(gesture(&mut popup, header, outside)[2], None);
        assert_eq!(gesture(&mut popup, entry(0), header)[2], None);
        // A release without an observed press (the opening click) does nothing.
        let mut fresh = PopupList::below(anchor, 3, None);
        assert_eq!(fresh.pointer(header, false, false, true, None), None);
        // Keyboard stepping clamps and scrolls into view.
        fresh.step(-1);
        assert_eq!(fresh.highlight, Some(0));
        fresh.step(5);
        assert_eq!(fresh.highlight, Some(2));
    }

    #[test]
    fn occluded_tracker_never_reads_a_consumed_click_as_a_new_press() {
        let mut tracker = ButtonTracker::<usize>::capturing();
        // A layer above consumes the press and closes on it; the button is still held.
        tracker.occlude(true);
        assert_eq!(tracker.update(Some(0), true, false, false), None);
        assert_eq!(
            tracker.pressed(),
            None,
            "held after an occluded press is no press"
        );
        assert_eq!(tracker.update(Some(0), false, false, true), None);
        // reset() keeps the physical button state for the same reason.
        tracker.update(Some(0), true, true, false);
        tracker.reset();
        assert_eq!(tracker.update(Some(0), true, false, false), None);
        assert_eq!(tracker.update(Some(0), false, false, true), None);
        // The next real click works.
        tracker.update(Some(0), true, true, false);
        assert_eq!(tracker.update(Some(0), false, false, true), Some(0));
    }

    #[test]
    fn button_tracker_press_cancel_activate_state_machine() {
        let mut tracker = ButtonTracker::<usize>::new();
        assert_eq!(tracker.pressed(), None);
        assert!(!tracker.is_pressed(0));

        // 1. Mouse down on control 0 -> pressed state active, no activation yet
        let act = tracker.update(Some(0), true, true, false);
        assert_eq!(act, None);
        assert_eq!(tracker.pressed(), Some(0));
        assert!(tracker.is_pressed(0));

        // 2. Mouse held while still over control 0 -> stays pressed, no activation
        let act = tracker.update(Some(0), true, false, false);
        assert_eq!(act, None);
        assert_eq!(tracker.pressed(), Some(0));

        // 3. Mouse released over control 0 -> triggers activation!
        let act = tracker.update(Some(0), false, false, true);
        assert_eq!(act, Some(0));
        assert_eq!(tracker.pressed(), None);
        assert!(!tracker.is_pressed(0));

        // 4. Test cancel on move away:
        // Mouse down on control 1
        tracker.update(Some(1), true, true, false);
        assert_eq!(tracker.pressed(), Some(1));

        // Cursor moves away while mouse is held (hovered = None) -> pressed cancelled!
        let act = tracker.update(None, true, false, false);
        assert_eq!(act, None);
        assert_eq!(tracker.pressed(), None);
        assert!(!tracker.is_pressed(1));

        // Cursor moves back over control 1 while mouse is still held -> stays cancelled!
        let act = tracker.update(Some(1), true, false, false);
        assert_eq!(act, None);
        assert_eq!(tracker.pressed(), None);

        // Mouse released over control 1 after being cancelled -> does NOT trigger activation!
        let act = tracker.update(Some(1), false, false, true);
        assert_eq!(act, None);
        assert_eq!(tracker.pressed(), None);

        // 5. Test release outside:
        tracker.update(Some(2), true, true, false);
        assert_eq!(tracker.pressed(), Some(2));
        // Cursor moves away and releases outside
        let act = tracker.update(None, false, false, true);
        assert_eq!(act, None);
        assert_eq!(tracker.pressed(), None);

        // 6. Click started outside control, moving into control while held
        let act = tracker.update(None, true, true, false);
        assert_eq!(act, None);
        assert_eq!(tracker.pressed(), None);
        let act = tracker.update(Some(3), true, false, false);
        assert_eq!(act, None);
        assert_eq!(tracker.pressed(), None);
        let act = tracker.update(Some(3), false, false, true);
        assert_eq!(act, None);
    }

    #[test]
    fn widget_pressed_states_drawing() {
        let canvas = Canvas::default();
        let fonts = Fonts::default();
        let theme = Theme::default();
        let painter = Painter {
            canvas: &canvas,
            fonts: &fonts,
            theme: &theme,
            origin: Vec2::ZERO,
            scale: 1.,
        };

        let rect = Rect::new(10., 10., 80., 24.);

        // Button normal vs pressed
        painter.button(rect, "Test", ButtonState::new(true));
        let quads_normal = canvas.drain();
        assert!(!quads_normal.is_empty());

        painter.button(rect, "Test", ButtonState::active(true, true, true));
        let quads_pressed = canvas.drain();
        assert!(!quads_pressed.is_empty());

        // CheckBox normal vs pressed
        painter.check_box(rect, "Check", false, ButtonState::new(true));
        let quads_chk_normal = canvas.drain();
        assert!(!quads_chk_normal.is_empty());

        painter.check_box(rect, "Check", true, ButtonState::active(true, true, true));
        let quads_chk_pressed = canvas.drain();
        assert!(!quads_chk_pressed.is_empty());

        // Radio normal vs pressed
        painter.radio(rect, "Radio", false, ButtonState::new(true));
        let quads_radio_normal = canvas.drain();
        assert!(!quads_radio_normal.is_empty());

        painter.radio(rect, "Radio", true, ButtonState::active(true, true, true));
        let quads_radio_pressed = canvas.drain();
        assert!(!quads_radio_pressed.is_empty());

        // URLButton normal vs pressed
        painter.url(rect, "Link", ButtonState::new(true));
        let quads_url_normal = canvas.drain();
        assert!(!quads_url_normal.is_empty());

        painter.url(rect, "Link", ButtonState::active(true, true, true));
        let quads_url_pressed = canvas.drain();
        assert!(!quads_url_pressed.is_empty());

        // The link's text and underline start inside its focus box, FOCUS_PAD clear of it.
        painter.url(rect, "Link", ButtonState::new(true));
        let left = canvas
            .drain()
            .iter()
            .map(|q| q.destination.x)
            .fold(f32::INFINITY, f32::min);
        painter.focus_box(rect);
        let box_left = canvas
            .drain()
            .iter()
            .map(|q| q.destination.x)
            .fold(f32::INFINITY, f32::min);
        assert!(left >= box_left + FOCUS_PAD, "{left} vs {box_left}");
    }

    #[test]
    fn frame_spec_and_close_button_drawing() {
        let size = vec2(300., 200.);
        let spec_with_close = FrameSpec::new(size, "Test Frame");
        assert!(spec_with_close.has_close);
        assert_eq!(
            spec_with_close.close_rect(),
            Some(Rect::new(size.x - 24., 6., 18., 16.))
        );

        let spec_no_close = spec_with_close.without_close();
        assert!(!spec_no_close.has_close);
        assert_eq!(spec_no_close.close_rect(), None);

        let canvas = Canvas::default();
        let fonts = Fonts::default();
        let theme = Theme::default();
        let painter = Painter {
            canvas: &canvas,
            fonts: &fonts,
            theme: &theme,
            origin: Vec2::ZERO,
            scale: 1.,
        };

        // Frame without close button
        painter.frame_with_close(size, "No Close", None);
        let quads_no_close = canvas.drain();
        assert!(!quads_no_close.is_empty());

        // Frame with close button: the cross is a glyph only (no border quads; no font here)
        painter.frame_with_close(size, "With Close", Some(ButtonState::new(true)));
        let quads_with_close = canvas.drain();
        assert_eq!(quads_with_close.len(), quads_no_close.len());

        // Frame with close button pressed
        painter.frame_with_close(
            size,
            "With Close Pressed",
            Some(ButtonState::active(true, true, true)),
        );
        let quads_pressed = canvas.drain();
        assert_eq!(quads_pressed.len(), quads_with_close.len());

        // With Marlett font loaded, close button emits glyphs centered in the close rect
        let proggy = FontFace::load(crate::PROGGY_CLEAN, canvas.clone()).ok();
        let fonts_with_marlett = Fonts {
            marlett: proggy,
            ..Fonts::default()
        };
        let painter_marlett = Painter {
            canvas: &canvas,
            fonts: &fonts_with_marlett,
            theme: &theme,
            origin: Vec2::ZERO,
            scale: 1.,
        };
        painter_marlett.frame_with_close(size, "With Marlett", Some(ButtonState::new(true)));
        let quads_marlett = canvas.drain();
        assert!(quads_marlett.len() > quads_no_close.len());

        painter_marlett.frame_with_close(
            size,
            "With Marlett Pressed",
            Some(ButtonState::active(true, true, true)),
        );
        let quads_marlett_pressed = canvas.drain();
        assert_eq!(quads_marlett_pressed.len(), quads_marlett.len());
        let last_unpressed = quads_marlett.last().unwrap();
        let last_pressed = quads_marlett_pressed.last().unwrap();
        assert_eq!(
            last_pressed.destination.x,
            last_unpressed.destination.x + 1.
        );
        assert_eq!(
            last_pressed.destination.y,
            last_unpressed.destination.y + 1.
        );
    }

    #[test]
    fn dialog_frame_style_rounded_corners_and_overflow_clipping() {
        // 1. Verify default FrameStyle enables rounding, enables overflow clipping, disables outer 3D border
        let default_style = FrameStyle::DEFAULT;
        assert!(default_style.rounded);
        assert!(default_style.clip_overflow);
        assert!(!default_style.outer_3d_border);
        assert_eq!(default_style.corner_radius, FRAME_CORNER_RADIUS);
        assert_eq!(FrameStyle::default(), default_style);

        // 2. Corner radius and bounds hit testing
        let size = vec2(200., 150.);
        // Interior point is contained
        assert!(default_style.contains_point(vec2(100., 75.), size));
        // Outside points
        assert!(!default_style.contains_point(vec2(-1., 50.), size));
        assert!(!default_style.contains_point(vec2(201., 50.), size));
        assert!(!default_style.contains_point(vec2(50., -1.), size));
        assert!(!default_style.contains_point(vec2(50., 151.), size));

        // Corner hit testing: radius is 5.0.
        // Extreme corner point (0.5, 0.5) is outside the circle (5-0.5)^2 + (5-0.5)^2 = 40.5 > 25
        assert!(!default_style.contains_point(vec2(0.5, 0.5), size));
        assert!(!default_style.contains_point(vec2(199.5, 0.5), size));
        assert!(!default_style.contains_point(vec2(0.5, 149.5), size));
        assert!(!default_style.contains_point(vec2(199.5, 149.5), size));

        // Point inside the corner arc (3.0, 3.0): (5-3)^2 + (5-3)^2 = 8 <= 25
        assert!(default_style.contains_point(vec2(3.0, 3.0), size));

        // When rounding is disabled, extreme corner (0.5, 0.5) is contained
        let square_style = FrameStyle {
            rounded: false,
            ..default_style
        };
        assert!(square_style.contains_point(vec2(0.5, 0.5), size));

        // 3. Overflow clipping
        let raw_corner = Rect::new(0., 0., 10., 10.);
        let clipped = default_style
            .clip_rect(raw_corner, size)
            .expect("clipped rect");
        assert!(clipped.x > 0.0);
        assert!(clipped.y > 0.0);

        // Completely outside rect returns None
        assert_eq!(
            default_style.clip_rect(Rect::new(-20., -20., 10., 10.), size),
            None
        );

        // 4. FrameSpec integration with FrameStyle
        let spec = FrameSpec::new(size, "Rounded Dialog");
        assert_eq!(spec.style, default_style);
        let close_rect = spec.close_rect().expect("close button rect");
        // Close button must be within the frame boundaries
        assert!(close_rect.x >= 0. && close_rect.x + close_rect.w <= size.x);
        assert!(close_rect.y >= 0. && close_rect.y + close_rect.h <= size.y);

        // 5. Canvas drawing verification:
        let canvas = Canvas::default();
        let fonts = Fonts::default();
        let theme = Theme::default();
        let painter = Painter {
            canvas: &canvas,
            fonts: &fonts,
            theme: &theme,
            origin: Vec2::ZERO,
            scale: 1.,
        };

        // Drawing with default FrameStyle emits rounded fill (includes corner polygon quads) and NO outer 3D border
        painter.frame_spec(spec.without_close(), None);
        let quads_rounded = canvas.drain();
        // 3 rectangular strips + 4 corner polygon quads = 7 quads
        assert_eq!(quads_rounded.len(), 7);
        let polygon_count = quads_rounded
            .iter()
            .filter(|q| q.vertices.is_some())
            .count();
        assert_eq!(polygon_count, 4);

        // In contrast, a frame with outer_3d_border: true and rounded: false emits 1 fill + 4 3D border quads = 5 quads
        let legacy_spec = spec.without_close().with_style(FrameStyle {
            rounded: false,
            outer_3d_border: true,
            ..default_style
        });
        painter.frame_spec(legacy_spec, None);
        let quads_legacy = canvas.drain();
        assert_eq!(quads_legacy.len(), 5);
        let legacy_polygons = quads_legacy.iter().filter(|q| q.vertices.is_some()).count();
        assert_eq!(legacy_polygons, 0);

        // 6. Push buttons keep their 3D border; the close button is a plain glyph
        let btn_rect = Rect::new(10., 10., 80., 24.);
        painter.button(btn_rect, "OK", ButtonState::new(true));
        let btn_quads = canvas.drain();
        // A button draws 4 border quads (raised 3D border)
        assert_eq!(btn_quads.len(), 4);

        let close_btn_rect = Rect::new(50., 10., 18., 16.);
        painter.close_button(close_btn_rect, ButtonState::new(true));
        let close_quads = canvas.drain();
        assert!(close_quads.is_empty(), "no border around the cross");
    }

    #[test]
    fn drop_down_list_is_fully_opaque() {
        let canvas = Canvas::default();
        let fonts = Fonts::default();
        let theme = Theme::default();
        assert!(theme.frame_bg.a < 1., "scheme frame color is translucent");
        let painter = Painter {
            canvas: &canvas,
            fonts: &fonts,
            theme: &theme,
            origin: Vec2::ZERO,
            scale: 1.,
        };
        painter.menu_list(Rect::new(0., 0., 100., 60.), &[], 0, None, None);
        let quads = canvas.drain();
        assert_eq!(quads[0].destination.w, 100.);
        assert_eq!(quads[0].color.a, 1., "base layer is opaque");
    }

    #[test]
    fn double_click_needs_the_same_target_within_the_interval() {
        let mut clicks = DoubleClick::default();
        assert!(!clicks.register(1, Some(10.)));
        assert!(clicks.register(1, Some(10.3)), "second click in time");
        assert!(
            !clicks.register(1, Some(10.4)),
            "a third click starts a new pair"
        );
        assert!(clicks.register(1, Some(10.6)));
        assert!(!clicks.register(1, Some(20.)));
        assert!(!clicks.register(1, Some(20.6)), "too slow");
        assert!(!clicks.register(2, Some(20.7)), "different target");
        assert!(!clicks.register(2, None), "no time, no double click");
        assert!(!clicks.register(2, None));
    }

    #[test]
    fn scroll_steps_follow_the_platform_mode() {
        let mut notched = ScrollAccumulator::default();
        let mut smooth = ScrollAccumulator::default();
        // Small trackpad deltas: lost when notched, accumulated when smooth.
        let small: Vec<i32> = (0..5)
            .map(|_| notched.steps(0.3, ScrollMode::Notched))
            .collect();
        assert_eq!(small, vec![0; 5]);
        let small: i32 = (0..5).map(|_| smooth.steps(0.3, ScrollMode::Smooth)).sum();
        assert_eq!(small, 1, "1.5 accumulated: one step, half kept");
        assert_eq!(
            smooth.steps(0.6, ScrollMode::Smooth),
            1,
            "0.5 + 0.6 crosses a step"
        );
        // A direction change drops the remainder.
        assert_eq!(smooth.steps(-0.5, ScrollMode::Smooth), 0);
        assert_eq!(smooth.steps(-0.6, ScrollMode::Smooth), -1);
        // Whole notches behave the same in both modes.
        assert_eq!(notched.steps(2., ScrollMode::Notched), 2);
        assert_eq!(
            ScrollAccumulator::default().steps(2., ScrollMode::Smooth),
            2
        );
        assert_eq!(ScrollMode::Notched.rows_per_step(3), 3);
        assert_eq!(ScrollMode::Smooth.rows_per_step(3), 1);
        assert_eq!(
            ScrollMode::platform(),
            if cfg!(target_os = "macos") {
                ScrollMode::Smooth
            } else {
                ScrollMode::Notched
            }
        );
    }

    #[test]
    fn popup_list_wheel_accumulates_trackpad_deltas_only_on_smooth() {
        let anchor = Rect::new(0., 0., 100., 24.);
        let mut popup = PopupList::below(anchor, 30, Some(29));
        assert_eq!(popup.first, 30 - MENU_MAX_ROWS);
        let start = popup.first;
        for _ in 0..4 {
            popup.wheel(0.25, ScrollMode::Notched);
        }
        assert_eq!(popup.first, start, "sub-notch deltas are dropped");
        for _ in 0..4 {
            popup.wheel(0.25, ScrollMode::Smooth);
        }
        assert_eq!(
            popup.first,
            start - 1,
            "four quarter steps scroll one row up"
        );
        popup.wheel(-1., ScrollMode::Notched);
        assert_eq!(popup.first, start);
    }

    #[test]
    fn scroll_bar_layout_has_square_end_buttons_and_a_minimum_nob() {
        let rect = Rect::new(0., 0., SCROLLBAR_WIDTH, 200.);
        let top = ScrollBarLayout::new(rect, 0, 10, 100);
        assert_eq!(top.up, Rect::new(0., 0., 17., 17.));
        assert_eq!(top.down, Rect::new(0., 183., 17., 17.));
        assert_eq!(top.track, Rect::new(0., 17., 17., 166.));
        assert_eq!(top.nob.h, 17., "10% of 166 is below the bar width");
        assert_eq!(top.nob.y, top.track.y);
        let bottom = ScrollBarLayout::new(rect, 90, 10, 100);
        assert_eq!(bottom.nob.y + bottom.nob.h, bottom.track.y + bottom.track.h);
        assert_eq!(bottom.first_for_nob_top(bottom.nob.y), 90);
        assert_eq!(bottom.part_at(vec2(5., 5.)), Some(ScrollPart::Up));
        assert_eq!(bottom.part_at(vec2(5., 190.)), Some(ScrollPart::Down));
        assert_eq!(bottom.part_at(vec2(5., 40.)), Some(ScrollPart::PageUp));
        assert_eq!(bottom.part_at(vec2(5., 175.)), Some(ScrollPart::Nob));
    }

    #[test]
    fn scroll_bar_arrows_step_on_press_and_repeat_while_held() {
        let rect = Rect::new(0., 0., SCROLLBAR_WIDTH, 200.);
        let mut first = 5;
        let mut bar = ScrollBarState::default();
        let up = Some(vec2(5., 5.));
        let frame = |bar: &mut ScrollBarState, first: &mut usize, down, click, now| {
            let layout = ScrollBarLayout::new(rect, *first, 10, 100);
            if let Some(f) = bar.pointer(&layout, up, down, click, now, *first, 10) {
                *first = f;
            }
        };
        frame(&mut bar, &mut first, true, true, Some(1.0));
        assert_eq!(first, 4, "steps on press");
        assert_eq!(bar.pressed(), Some(ScrollPart::Up));
        frame(&mut bar, &mut first, true, false, Some(1.3));
        assert_eq!(first, 4, "no repeat before the delay");
        frame(&mut bar, &mut first, true, false, Some(1.4));
        assert_eq!(first, 3, "first repeat after 400 ms");
        frame(&mut bar, &mut first, true, false, Some(1.42));
        assert_eq!(first, 3);
        frame(&mut bar, &mut first, true, false, Some(1.45));
        assert_eq!(first, 2, "then every 50 ms");
        frame(&mut bar, &mut first, false, false, Some(1.5));
        assert!(!bar.active(), "release ends the gesture");
        // Without frame times there is no repeat.
        frame(&mut bar, &mut first, true, true, None);
        frame(&mut bar, &mut first, true, false, None);
        assert_eq!(first, 1);
    }

    #[test]
    fn scroll_bar_nob_drags_live_and_track_pages_once() {
        let rect = Rect::new(0., 0., SCROLLBAR_WIDTH, 200.);
        let mut bar = ScrollBarState::default();
        let layout = ScrollBarLayout::new(rect, 0, 10, 100);
        let grab = vec2(5., layout.nob.y + 5.);
        assert_eq!(
            bar.pointer(&layout, Some(grab), true, true, None, 0, 10),
            None
        );
        // Moving while held scrolls before any release.
        let moved = bar.pointer(
            &layout,
            Some(grab + vec2(0., 74.5)),
            true,
            false,
            None,
            0,
            10,
        );
        assert_eq!(moved, Some(45), "half the travel of 149 px");
        bar.pointer(&layout, Some(grab), false, false, None, 45, 10);
        assert!(!bar.active());
        // A press on the track below the nob pages once, holding adds nothing.
        let below = Some(vec2(5., 150.));
        assert_eq!(
            bar.pointer(&layout, below, true, true, None, 0, 10),
            Some(10)
        );
        assert_eq!(bar.pointer(&layout, below, true, false, None, 10, 10), None);
    }

    #[test]
    fn popup_scroll_bar_scrolls_without_selecting_or_closing() {
        let anchor = Rect::new(0., 0., 100., 24.);
        let mut popup = PopupList::below(anchor, 30, None);
        let layout = popup.bar_layout().expect("30 entries do not fit");
        let down_button = Some(vec2(layout.down.x + 5., layout.down.y + 5.));
        assert_eq!(popup.pointer(down_button, true, true, false, None), None);
        assert_eq!(popup.first, 1, "the arrow scrolls on press");
        assert_eq!(popup.bar_pressed(), Some(ScrollPart::Down));
        assert_eq!(popup.pointer(down_button, false, false, true, None), None);
        assert_eq!(popup.bar_pressed(), None);
        assert_eq!(popup.highlight, None, "nothing selected");
        // An entry click still selects afterwards.
        let entry = Some(vec2(10., popup.list.y + MENU_ROW * 0.5));
        popup.pointer(entry, true, true, false, None);
        assert_eq!(
            popup.pointer(entry, false, false, true, None),
            Some(PopupEvent::Select(1))
        );
    }

    #[test]
    fn close_cross_lightens_on_hover_and_press() {
        let theme = Theme::default();
        assert_eq!(theme.close_style(ButtonState::new(true)), theme.close_fg);
        assert_eq!(
            theme.close_style(ButtonState::active(true, true, false)),
            theme.title_text
        );
        assert_eq!(
            theme.close_style(ButtonState::active(true, false, true)),
            theme.title_text
        );
        assert_eq!(
            theme.close_style(ButtonState::active(false, true, true)),
            theme.close_disabled
        );
        assert_ne!(theme.close_fg, theme.title_text, "hover must be visible");
    }

    #[test]
    fn tooltip_geometry_and_screen_clamping() {
        let bounds = Rect::new(0., 0., 1280., 720.);
        let tip_size = vec2(200., 40.);

        // 1. Normal interior point: hangs right and down
        let normal = tooltip_rect(vec2(100., 100.), tip_size, bounds);
        assert_eq!(normal, Rect::new(100., 120., 200., 40.));

        // 2. Near right edge: flips to hang left
        let right = tooltip_rect(vec2(1200., 100.), tip_size, bounds);
        assert_eq!(right, Rect::new(1000., 120., 200., 40.));
        assert!(right.x + right.w <= bounds.w);

        // 3. Near bottom edge: flips to hang up
        let bottom = tooltip_rect(vec2(100., 700.), tip_size, bounds);
        assert_eq!(bottom, Rect::new(100., 640., 200., 40.));
        assert!(bottom.y + bottom.h <= bounds.h);

        // 4. Bottom-right corner: flips both left and up
        let corner = tooltip_rect(vec2(1250., 710.), tip_size, bounds);
        assert_eq!(corner, Rect::new(1050., 650., 200., 40.));
        assert!(corner.x >= bounds.x && corner.x + corner.w <= bounds.w);
        assert!(corner.y >= bounds.y && corner.y + corner.h <= bounds.h);

        // 5. Painter drawing tooltip produces canvas quads and empty text is ignored
        let canvas = Canvas::default();
        let fonts = Fonts::default();
        let theme = Theme::default();
        let painter = Painter {
            canvas: &canvas,
            fonts: &fonts,
            theme: &theme,
            origin: Vec2::ZERO,
            scale: 1.,
        };
        painter.tooltip("", vec2(100., 100.), bounds);
        assert!(canvas.drain().is_empty(), "Empty tooltip draws nothing");

        painter.tooltip("Hover tip text", vec2(100., 100.), bounds);
        let quads = canvas.drain();
        assert!(!quads.is_empty(), "Tooltip draws background and border");
    }

    #[test]
    fn progress_bar_drawing_and_segments() {
        let canvas = Canvas::default();
        let fonts = Fonts::default();
        let theme = Theme::default();
        let painter = Painter {
            canvas: &canvas,
            fonts: &fonts,
            theme: &theme,
            origin: Vec2::ZERO,
            scale: 1.,
        };

        // 1. Zero fraction draws background and sunken border but no segments
        let rect = Rect::new(20., 64., 340., 24.);
        let fg = Color::new(1., 1., 1., 1.);
        let bg = Color::new(0., 0., 0., 1.);
        painter.progress_bar(rect, 0.0, fg, bg);
        let quads_0 = canvas.drain();
        assert!(!quads_0.is_empty(), "Draws cavity and border");

        // 2. 50% fraction draws about half the segments
        painter.progress_bar(rect, 0.5, fg, bg);
        let quads_half = canvas.drain();
        assert!(quads_half.len() > quads_0.len(), "50% draws segment blocks");

        // 3. 100% fraction draws all segments
        painter.progress_bar(rect, 1.0, fg, bg);
        let quads_full = canvas.drain();
        assert!(
            quads_full.len() > quads_half.len(),
            "100% draws all segment blocks"
        );
    }

    #[test]
    fn check_box_checked_draws_white_checkmark_not_plain_square() {
        let canvas = Canvas::default();
        let fonts = Fonts::default();
        let theme = Theme::default();
        let painter = Painter {
            canvas: &canvas,
            fonts: &fonts,
            theme: &theme,
            origin: Vec2::ZERO,
            scale: 1.,
        };

        let rect = Rect::new(10., 10., 120., 24.);

        // Unchecked: emits bg + 4 border quads
        painter.check_box(rect, "Test", false, ButtonState::new(true));
        let quads_unchecked = canvas.drain();
        assert_eq!(quads_unchecked.len(), 5);

        // Checked: emits bg + 4 border quads + 7 checkmark quads
        painter.check_box(rect, "Test", true, ButtonState::new(true));
        let quads_checked = canvas.drain();
        assert_eq!(quads_checked.len(), 12);

        // Checkmark quads should have theme.check color (white)
        let check_quads = &quads_checked[5..12];
        for quad in check_quads {
            assert_eq!(quad.color, theme.check);
            assert_eq!(quad.destination.w, 1.);
            assert_eq!(quad.destination.h, 2.);
        }

        // Verify shape: X coordinates span 15..=21 (bx.x is 12., columns are dx=3..=9)
        // Y coordinates descend to apex at dx=5 (bx.y is 16., dy=7 -> y=23) and ascend to dy=3 (y=19)
        let xs: Vec<f32> = check_quads.iter().map(|q| q.destination.x).collect();
        let ys: Vec<f32> = check_quads.iter().map(|q| q.destination.y).collect();
        assert_eq!(xs, vec![15., 16., 17., 18., 19., 20., 21.]);
        assert_eq!(ys, vec![21., 22., 23., 22., 21., 20., 19.]);

        // When pressed, checkmark should be shifted by 1px offset
        painter.check_box(rect, "Test", true, ButtonState::active(true, true, true));
        let quads_pressed = canvas.drain();
        let check_pressed = &quads_pressed[5..12];
        let xs_pressed: Vec<f32> = check_pressed.iter().map(|q| q.destination.x).collect();
        let ys_pressed: Vec<f32> = check_pressed.iter().map(|q| q.destination.y).collect();
        assert_eq!(xs_pressed, vec![16., 17., 18., 19., 20., 21., 22.]);
        assert_eq!(ys_pressed, vec![22., 23., 24., 23., 22., 21., 20.]);
    }

    #[test]
    fn scale_computation_for_scale_factors_and_window_sizes() {
        // Scale factor 1.0 (standard DPI)
        assert_eq!(Painter::scale_for_window(480., 1.0), 1.0);
        assert_eq!(Painter::scale_for_window(720., 1.0), 1.0);
        assert_eq!(Painter::scale_for_window(800., 1.0), 800. / 720.);
        assert_eq!(Painter::scale_for_window(1080., 1.0), 1080. / 720.);
        assert_eq!(Painter::scale_for_window(1440., 1.0), 1440. / 720.);
        assert_eq!(Painter::scale_for_window(2160., 1.0), 2160. / 720.);

        // Scale factor 2.0 (High-DPI / Retina)
        assert_eq!(Painter::scale_for_window(480., 2.0), 960. / 720.);
        assert_eq!(Painter::scale_for_window(720., 2.0), 1440. / 720.);
        assert_eq!(Painter::scale_for_window(800., 2.0), 1600. / 720.);
        assert_eq!(Painter::scale_for_window(1080., 2.0), 2160. / 720.);
        assert_eq!(Painter::scale_for_window(1440., 2.0), 2880. / 720.);
        assert_eq!(Painter::scale_for_window(2160., 2.0), 4320. / 720.);

        // Regression assertion: 1280x800 layout scale is unchanged
        let scale_800_1x = Painter::scale_for_window(800., 1.0);
        assert_eq!(scale_800_1x, Painter::scale_for(800.));
        assert_eq!(scale_800_1x, 800. / 720.);
    }

    #[test]
    fn truncate_ellipsis_truncates_long_strings_with_ellipsis() {
        let char_measure = |s: &str| s.chars().count() as f32 * 10.;

        // 1. String fitting within bounds is returned unchanged without allocation (Cow::Borrowed)
        let fits = truncate_ellipsis("1920 x 1080", 150., char_measure);
        assert_eq!(fits, "1920 x 1080");
        assert!(matches!(fits, Cow::Borrowed(_)));

        // 2. Exact fit is returned unchanged
        let exact = truncate_ellipsis("12345", 50., char_measure);
        assert_eq!(exact, "12345");
        assert!(matches!(exact, Cow::Borrowed(_)));

        // 3. Overflowing string is truncated with ellipsis, total width <= max_w
        let truncated = truncate_ellipsis("Fit Display (1920 x 1080)", 100., char_measure);
        assert_eq!(truncated, "Fit Dis...");
        assert_eq!(char_measure(&truncated), 100.);
        assert!(matches!(truncated, Cow::Owned(_)));

        // 4. Multi-byte UTF-8 string with umlauts and symbols truncates on valid character boundaries
        let utf8_str = "Überragende Auflösung © 2026";
        let utf8_trunc = truncate_ellipsis(utf8_str, 80., char_measure);
        assert!(utf8_trunc.ends_with("..."));
        assert!(char_measure(&utf8_trunc) <= 80.);

        // 5. Width only allows the ellipsis itself
        let only_dots = truncate_ellipsis("Hello World", 30., char_measure);
        assert_eq!(only_dots, "...");
        assert!(matches!(only_dots, Cow::Borrowed(_)));

        // 6. Width smaller than ellipsis width: fits as many characters as possible without ellipsis
        let small = truncate_ellipsis("Hello World", 20., char_measure);
        assert_eq!(small, "He");
        assert_eq!(char_measure(&small), 20.);
        assert!(matches!(small, Cow::Borrowed(_)));

        // 7. Zero width returns empty string
        let zero = truncate_ellipsis("Hello", 0., char_measure);
        assert_eq!(zero, "");

        // 8. Negative width returns empty string
        let neg = truncate_ellipsis("Hello", -10., char_measure);
        assert_eq!(neg, "");

        // 9. Empty string returns empty string
        let empty = truncate_ellipsis("", 50., char_measure);
        assert_eq!(empty, "");
    }

    #[test]
    fn combobox_text_and_selection_never_overlap_arrow_or_overflow_bounds() {
        let canvas = Canvas::default();
        let font_bytes = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/fonts/ProggyClean.ttf"
        ));
        let font = FontFace::load(font_bytes, canvas.clone()).expect("load font");
        let fonts = Fonts {
            text: Some(font),
            bold: None,
            marlett: None,
        };
        let theme = Theme::default();
        let painter = Painter {
            canvas: &canvas,
            fonts: &fonts,
            theme: &theme,
            origin: Vec2::ZERO,
            scale: 1.,
        };

        let rect = Rect::new(36., 48., 160., 24.);
        let button_w = rect.h;
        let button_x = rect.x + rect.w - button_w; // 36 + 160 - 24 = 172.

        let long_text = "Fit Display (1920 x 1080)";
        assert!(
            painter.text_width(long_text, false) > button_x - rect.x - 5.,
            "long_text width must exceed available combobox text width"
        );

        // 1. Focused combobox: selection rectangle and text glyphs must stay strictly before button
        painter.combo(rect, long_text, true, false, true);
        let quads_focused = canvas.drain();
        assert!(!quads_focused.is_empty());

        let sel_quads: Vec<_> = quads_focused
            .iter()
            .filter(|q| q.color == theme.entry_selected_bg)
            .collect();
        assert_eq!(sel_quads.len(), 1, "exactly one selection highlight quad");
        let sel = sel_quads[0];
        assert!(
            sel.destination.x >= rect.x,
            "selection starts inside combobox"
        );
        assert!(
            sel.destination.x + sel.destination.w <= button_x,
            "selection right edge ({}) must not overlap button.x ({})",
            sel.destination.x + sel.destination.w,
            button_x
        );
        assert!(
            sel.destination.x + sel.destination.w <= rect.x + rect.w,
            "selection right edge must not exceed combobox bounds"
        );

        let glyph_quads_focused: Vec<_> = quads_focused
            .iter()
            .filter(|q| q.texture.is_some())
            .collect();
        assert!(
            !glyph_quads_focused.is_empty(),
            "focused combobox emits glyph quads"
        );
        for quad in &glyph_quads_focused {
            assert!(quad.destination.x >= rect.x, "glyph starts inside combobox");
            assert!(
                quad.destination.x + quad.destination.w <= button_x,
                "glyph right edge ({}) must not overlap button.x ({})",
                quad.destination.x + quad.destination.w,
                button_x
            );
            assert!(
                quad.destination.x + quad.destination.w <= rect.x + rect.w,
                "glyph right edge must not exceed combobox bounds"
            );
        }

        // 2. Unfocused combobox: text glyphs must stay strictly before button
        painter.combo(rect, long_text, true, false, false);
        let quads_unfocused = canvas.drain();
        let glyph_quads_unfocused: Vec<_> = quads_unfocused
            .iter()
            .filter(|q| q.texture.is_some())
            .collect();
        assert!(
            !glyph_quads_unfocused.is_empty(),
            "unfocused combobox emits glyph quads"
        );
        for quad in &glyph_quads_unfocused {
            assert!(
                quad.destination.x + quad.destination.w <= button_x,
                "unfocused glyph right edge ({}) must not overlap button.x ({})",
                quad.destination.x + quad.destination.w,
                button_x
            );
            assert!(
                quad.destination.x + quad.destination.w <= rect.x + rect.w,
                "unfocused glyph right edge must not exceed combobox bounds"
            );
        }

        // 3. Disabled combobox: text glyphs stay strictly before button and use disabled color
        painter.combo(rect, long_text, false, false, false);
        let quads_disabled = canvas.drain();
        let glyph_quads_disabled: Vec<_> = quads_disabled
            .iter()
            .filter(|q| q.texture.is_some())
            .collect();
        assert!(
            !glyph_quads_disabled.is_empty(),
            "disabled combobox emits glyph quads"
        );
        for quad in &glyph_quads_disabled {
            assert_eq!(quad.color, theme.text_disabled);
            assert!(
                quad.destination.x + quad.destination.w <= button_x,
                "disabled glyph right edge ({}) must not overlap button.x ({})",
                quad.destination.x + quad.destination.w,
                button_x
            );
        }
    }

    #[test]
    fn menu_list_items_never_overlap_scrollbar_or_overflow_bounds() {
        let canvas = Canvas::default();
        let font_bytes = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/fonts/ProggyClean.ttf"
        ));
        let font = FontFace::load(font_bytes, canvas.clone()).expect("load font");
        let fonts = Fonts {
            text: Some(font),
            bold: None,
            marlett: None,
        };
        let theme = Theme::default();
        let painter = Painter {
            canvas: &canvas,
            fonts: &fonts,
            theme: &theme,
            origin: Vec2::ZERO,
            scale: 1.,
        };

        let rect = Rect::new(36., 72., 160., 80.);
        let long_item = "3840 x 2160 (16:9 Widescreen Ultra High Definition 4K)".to_string();
        let items = vec![
            long_item.clone(),
            "1920 x 1080".to_string(),
            "1280 x 720".to_string(),
            "1024 x 768".to_string(),
            "800 x 600".to_string(),
            "640 x 480".to_string(),
        ];

        // 1. List with scrollbar (items.len() > rows): glyphs must not overlap scrollbar
        let rows = (rect.h / MENU_ROW).floor() as usize;
        assert!(items.len() > rows);
        let scrollbar_x = rect.x + rect.w - SCROLLBAR_WIDTH;

        painter.menu_list(rect, &items, 0, Some(0), None);
        let quads_scroll = canvas.drain();
        let glyphs_scroll: Vec<_> = quads_scroll
            .iter()
            .filter(|q| q.texture.is_some())
            .collect();
        assert!(!glyphs_scroll.is_empty());
        for quad in &glyphs_scroll {
            assert!(
                quad.destination.x + quad.destination.w <= scrollbar_x,
                "menu list item glyph ({}) must not overlap scrollbar_x ({})",
                quad.destination.x + quad.destination.w,
                scrollbar_x
            );
        }

        // 2. List without scrollbar: glyphs must not exceed rect bounds
        let short_items = vec![long_item, "1920 x 1080".to_string()];
        painter.menu_list(rect, &short_items, 0, None, None);
        let quads_no_scroll = canvas.drain();
        let glyphs_no_scroll: Vec<_> = quads_no_scroll
            .iter()
            .filter(|q| q.texture.is_some())
            .collect();
        assert!(!glyphs_no_scroll.is_empty());
        for quad in &glyphs_no_scroll {
            assert!(
                quad.destination.x + quad.destination.w <= rect.x + rect.w,
                "menu list item glyph ({}) must not exceed rect.x + rect.w ({})",
                quad.destination.x + quad.destination.w,
                rect.x + rect.w
            );
        }
    }

    #[test]
    fn input_focus_parent_child_relations() {
        let unfocused = InputFocus::unfocused();
        assert!(!unfocused.parent_focused);
        assert!(!unfocused.child_focused);
        assert!(!unfocused.is_active(), "both unfocused is inactive");

        let parent_only = InputFocus::parent_only();
        assert!(parent_only.parent_focused);
        assert!(!parent_only.child_focused);
        assert!(
            !parent_only.is_active(),
            "parent focused alone does not give child active focus"
        );

        let child_only = InputFocus::new(false, true);
        assert!(!child_only.parent_focused);
        assert!(child_only.child_focused);
        assert!(
            !child_only.is_active(),
            "child focused with unfocused parent cannot receive input"
        );

        let active = InputFocus::new(true, true);
        assert!(active.parent_focused);
        assert!(active.child_focused);
        assert!(
            active.is_active(),
            "both parent and child focused yields active input"
        );

        // Builder and setter methods
        let modified = unfocused.with_parent(true).with_child(true);
        assert!(modified.is_active());

        let mut mutable = InputFocus::default();
        assert!(!mutable.is_active());
        mutable.set_parent(true);
        assert!(!mutable.is_active());
        mutable.set_child(true);
        assert!(mutable.is_active());
        mutable.set_parent(false);
        assert!(!mutable.is_active());
    }

    #[test]
    fn blinking_caret_phase_timing_and_focus_gating() {
        let caret = BlinkingCaret::default();
        assert_eq!(caret.rate, 2.0);

        // Phase test at 2.0 Hz (500 ms per phase)
        assert!(caret.is_phase_on(0.0), "0.0s is phase on");
        assert!(caret.is_phase_on(0.25), "0.25s is phase on");
        assert!(caret.is_phase_on(0.499), "0.499s is phase on");
        assert!(!caret.is_phase_on(0.50), "0.50s is phase off");
        assert!(!caret.is_phase_on(0.75), "0.75s is phase off");
        assert!(!caret.is_phase_on(0.999), "0.999s is phase off");
        assert!(caret.is_phase_on(1.00), "1.00s is phase on (new cycle)");
        assert!(!caret.is_phase_on(1.50), "1.50s is phase off");

        // Focus gating: caret is ONLY visible when focus.is_active() is true
        let active = InputFocus::new(true, true);
        let parent_only = InputFocus::new(true, false);
        let child_only = InputFocus::new(false, true);
        let neither = InputFocus::new(false, false);

        assert!(
            caret.is_visible(active, 0.0),
            "active in phase on is visible"
        );
        assert!(
            !caret.is_visible(active, 0.5),
            "active in phase off is hidden"
        );

        // Crucial bugfix check: when out of focus, NEVER visible, regardless of timestamp
        for t in [0.0, 0.25, 0.49, 1.0, 1.25, 2.0] {
            assert!(
                !caret.is_visible(parent_only, t),
                "parent-only focus must never show caret at t={t}"
            );
            assert!(
                !caret.is_visible(child_only, t),
                "child-only focus (unfocused parent) must never show caret at t={t}"
            );
            assert!(
                !caret.is_visible(neither, t),
                "unfocused must never show caret at t={t}"
            );
        }

        // Negative or non-finite timestamps
        assert!(caret.is_phase_on(-1.0));
        assert!(caret.is_phase_on(f64::NAN));
    }

    #[test]
    fn text_input_model_and_interaction_restart() {
        let mut model = TextInputModel::new(true, true);
        assert!(model.is_focused());
        assert!(model.should_draw_focus_edge());
        assert!(model.should_draw_caret(0.2));
        assert!(!model.should_draw_caret(0.6));

        // Interaction restart (SDK ResetCursorBlink): user types at t=0.6
        model.record_interaction(0.6);
        // At t=0.6, elapsed = 0.0, so caret is immediately visible
        assert!(model.should_draw_caret(0.6));
        assert!(model.should_draw_caret(0.8));
        assert!(!model.should_draw_caret(1.15));

        // When parent dialog loses focus, both focus edge and caret disappear
        model.set_parent_focused(false);
        assert!(!model.is_focused());
        assert!(!model.should_draw_focus_edge());
        assert!(!model.should_draw_caret(0.6));
    }

    #[test]
    fn painter_text_entry_focus_edge_and_caret_drawing() {
        let canvas = Canvas::default();
        let fonts = Fonts::default();
        let theme = Theme::default();
        let painter = Painter {
            canvas: &canvas,
            fonts: &fonts,
            theme: &theme,
            origin: Vec2::ZERO,
            scale: 1.,
        };
        let rect = Rect::new(10., 10., 100., 20.);

        // 1. Unfocused text entry
        painter.text_entry(rect, "3.00", true, false, false);
        let quads_unfocused = canvas.drain();
        // Check that no 1px vertical line caret quad was drawn
        let caret_quads: Vec<_> = quads_unfocused
            .iter()
            .filter(|q| q.destination.w == 1. && q.destination.h == rect.h - 8.)
            .collect();
        assert!(
            caret_quads.is_empty(),
            "unfocused entry must not draw vertical caret"
        );

        // 2. Focused text entry with caret_visible = true
        painter.text_entry(rect, "3.00", true, true, true);
        let quads_caret_on = canvas.drain();
        let caret_quads_on: Vec<_> = quads_caret_on
            .iter()
            .filter(|q| q.destination.w == 1. && q.destination.h == rect.h - 8.)
            .collect();
        assert_eq!(
            caret_quads_on.len(),
            1,
            "focused entry with caret_visible=true must draw 1px caret"
        );

        // 3. Focused text entry with caret_visible = false (blinking off phase)
        painter.text_entry(rect, "3.00", true, true, false);
        let quads_caret_off = canvas.drain();
        let caret_quads_off: Vec<_> = quads_caret_off
            .iter()
            .filter(|q| q.destination.w == 1. && q.destination.h == rect.h - 8.)
            .collect();
        assert!(
            caret_quads_off.is_empty(),
            "focused entry with caret_visible=false must not draw 1px caret"
        );

        // 4. Model-driven drawing helper
        let model_on = TextInputModel::new(true, true);
        painter.text_entry_model(rect, "3.00", true, &model_on, 0.0);
        let quads_model = canvas.drain();
        assert_eq!(
            quads_model
                .iter()
                .filter(|q| q.destination.w == 1. && q.destination.h == rect.h - 8.)
                .count(),
            1
        );
    }
}
