//! The Options dialog: a VGUI property sheet whose pages are laid out from the installed
//! `resource/OptionsSub*.res` files and bound to [`Config`] through [`super::settings`].
//!
//! Edits go to a draft. OK returns the draft when it differs from the applied configuration,
//! Apply publishes it and keeps the dialog open, Cancel (or Escape) drops it. Nothing on the
//! Video page is applied before OK/Apply. Dialog geometry follows the SDK `PropertyDialog` /
//! `PropertySheet` rules (see `docs/gameui.md` section 3); the dialog uses the same uniform
//! scale as the other GameUI dialogs.
//!
//! Interaction is modal and layered, innermost first: key capture, open combo list, text entry,
//! confirmation box, sub-dialog (brightness / credits), then the dialog itself. Escape closes the
//! innermost layer; on the dialog it acts as Cancel. While the confirmation box, key capture or an
//! open combo list owns the pointer, the layer below is occluded: it arms, presses, activates and
//! shows tooltips for nothing, and its button tracker only follows the physical button state.
//! An open combo list is a [`widgets::PopupList`]: clicks anywhere close or select through it.
//!
//! Keyboard focus follows the SDK `FocusNavGroup` / `PropertyDialog` rules: Tab and Shift+Tab step
//! through the active tab, the page's controls by their layout `tabPosition` and OK / Cancel /
//! Apply, and wrap; a click focuses the control it presses. Space or Enter presses the focused
//! button, check box or radio button (activation on release), arrows move a radio group, step a
//! slider or a combo box and switch tabs; Alt+Up/Down opens a combo box's list; Enter elsewhere
//! presses the default button (docs/gameui.md section 6c).

mod hints;
mod sub_frame;
use super::settings::support::{self, binding_for_control as control_binding};
pub use hints::{ActiveTooltip, HintReport};
use sub_frame::{drawn_controls, footer_reflow, frame_size};

use super::{
    display::DisplayModes,
    resources::{GameUiResources, Image},
    settings::{self, Binding, Choice, Command, Draft, Env, Panel, Value},
};
use crate::config::Config;
use glam::{vec2, Vec2};
use source_ui::{
    canvas::{Rect, Texture2D},
    input::{Input, Key},
    keys::{self, KeyRow},
    layout::{Align as LayoutAlign, Control, ControlKind},
    message_box::MessageBox,
    placement::{self, Placement},
    widgets::{
        self, Align, ButtonState, FrameSpec, Painter, Theme, CLIENT_INSET_X, CLIENT_TOP, FONT_TALL,
        MENU_ROW, SCROLLBAR_WIDTH, SLIDER_HIT_TALL, SLIDER_TRACK_Y,
    },
};
use std::sync::Arc;

/// Options dialog frame size (retail default `OptionsDialog` size).
pub const DIALOG_SIZE: Vec2 = Vec2::new(512., 406.);
/// `PropertyDialog` reserves this much below the sheet for the OK/Cancel/Apply row.
const SHEET_INSET_BOTTOM: f32 = 32.;
const CLIENT_INSET_BOTTOM: f32 = 8.;
/// `PropertySheet` tab strip height and the text inset on each side of a tab label.
const TAB_HEIGHT: f32 = 28.;
const TAB_TEXT_INSET: f32 = 12.;
const BUTTON_SIZE: Vec2 = Vec2::new(72., 24.);
/// Fallback sub-dialog sizes when the layout carries no frame record.
const GAMMA_SIZE: Vec2 = Vec2::new(290., 396.);
const CREDITS_SIZE: Vec2 = Vec2::new(500., 210.);
/// Key list header and row heights.
const KEY_HEADER: f32 = 20.;
/// Share of the scheme's dim selection alpha (`SectionedListPanel.OutOfFocusSelectedBgColor`,
/// 128) kept on the action cell while a key is captured, so the orange key cell stands out
/// (owner tuning, 2026-10-15; not a retail value). The action text uses the normal row color,
/// since dark selection text would not read on the fainter fill.
const CAPTURE_ACTION_FADE: f32 = 0.5;
/// Rows scrolled per wheel step in lists.
const WHEEL_ROWS: usize = 3;

/// Property sheet tabs, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Gameplay,
    Keyboard,
    Mouse,
    Gamepad,
    Audio,
    Video,
}

impl Tab {
    pub const ALL: [Self; 6] = [
        Self::Gameplay,
        Self::Keyboard,
        Self::Mouse,
        Self::Gamepad,
        Self::Audio,
        Self::Video,
    ];

    pub fn token(self) -> &'static str {
        match self {
            Self::Gameplay => "#GameUI_Gameplay",
            Self::Keyboard => "#GameUI_Keyboard",
            Self::Mouse => "#GameUI_Mouse",
            Self::Gamepad => "#GameUI_Gamepad",
            Self::Audio => "#GameUI_Audio",
            Self::Video => "#GameUI_Video",
        }
    }

    pub fn panel(self) -> Option<Panel> {
        match self {
            Self::Gameplay => Some(Panel::Gameplay),
            Self::Keyboard => Some(Panel::Keyboard),
            Self::Mouse => Some(Panel::Mouse),
            Self::Gamepad => Some(Panel::Gamepad),
            Self::Audio => Some(Panel::Audio),
            Self::Video => Some(Panel::Video),
        }
    }
}

/// [`OptionsDialog::report`].
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct OptionsReport {
    pub tab: String,
    pub sub: Option<String>,
    pub focus: Option<String>,
    pub focus_visible: bool,
    pub changed: bool,
    pub list_open: bool,
    pub editing: bool,
    pub capturing_key: bool,
    /// The Resolution combo's choices for the draft's display mode and aspect ratio.
    pub resolutions: Vec<String>,
    /// The placed dialog on the canvas, `[x, y, width, height]` in pixels (filled by the
    /// console, which knows the canvas).
    pub frame: Option<[f32; 4]>,
    pub controls: Vec<String>,
    pub development_controls: Vec<String>,
    pub tooltip: Option<HintReport>,
}

/// What the owner of the dialog has to do after an input frame.
#[derive(Clone, Debug, PartialEq)]
pub enum OptionsOutcome {
    None,
    /// Apply and persist this configuration; the dialog stays open.
    Apply(Config),
    /// Close the dialog, applying and persisting the configuration if one is given.
    Close(Option<Config>),
}

/// A sub-dialog over the options dialog. Edits in a sub-dialog go to the draft; Cancel restores
/// the draft as it was when the sub-dialog opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sub {
    /// Brightness; its value stays in the draft until OK/Apply on the options dialog.
    Gamma,
    AudioCredits,
    VideoCredits,
    /// Keyboard > Advanced; its OK applies its settings at once, like retail.
    KeyboardAdvanced,
    /// Video > Advanced; likewise.
    VideoAdvanced,
}

impl Sub {
    fn is_credits(self) -> bool {
        matches!(self, Self::AudioCredits | Self::VideoCredits)
    }

    fn panel(self) -> Panel {
        match self {
            Self::Gamma => Panel::Gamma,
            Self::AudioCredits => Panel::AudioCredits,
            Self::VideoCredits => Panel::VideoCredits,
            Self::KeyboardAdvanced => Panel::KeyboardAdvanced,
            Self::VideoAdvanced => Panel::VideoAdvanced,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Gamma => "#GameUI_AdjustGamma_Title",
            Self::AudioCredits | Self::VideoCredits => "#GameUI_ThirdPartyTechCredits",
            Self::KeyboardAdvanced => "#GameUI_KeyboardAdvanced_Title",
            Self::VideoAdvanced => "#GameUI_VideoAdvanced_Title",
        }
    }

    fn fallback_size(self) -> Vec2 {
        match self {
            Self::Gamma => GAMMA_SIZE,
            Self::AudioCredits
            | Self::VideoCredits
            | Self::KeyboardAdvanced
            | Self::VideoAdvanced => CREDITS_SIZE,
        }
    }

    /// Copies the settings this sub-dialog's OK applies at once from `from` into `to`; returns
    /// whether it applies anything (retail advanced dialogs write their cvars on OK).
    fn apply_on_ok(self, from: &Config, to: &mut Config) -> bool {
        match self {
            Self::KeyboardAdvanced => to.keyboard = from.keyboard.clone(),
            Self::VideoAdvanced => {
                to.video.advanced = from.video.advanced.clone();
                to.video.fov = from.video.fov;
                to.video.vertical_fov = from.video.vertical_fov;
                to.video.viewmodel_fov = from.video.viewmodel_fov;
            }
            Self::Gamma | Self::AudioCredits | Self::VideoCredits => return false,
        }
        true
    }
}

/// Something under the pointer that reacts to clicks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Control(usize),
    Tab(usize),
    Ok,
    Cancel,
    Apply,
    Close,
}

impl Target {
    fn widget_kind(self, widgets: &[Widget]) -> widgets::WidgetKind {
        match self {
            Target::Control(i) => widgets
                .get(i)
                .map_or(widgets::WidgetKind::Other, |w| w.control.widget_kind()),
            Target::Tab(_) | Target::Ok | Target::Cancel | Target::Apply | Target::Close => {
                widgets::WidgetKind::DialogButton
            }
        }
    }

    fn has_rollover_sound(self, widgets: &[Widget]) -> bool {
        self.widget_kind(widgets).has_rollover_sound()
    }
}

#[derive(Clone, Debug, PartialEq)]
struct OpenCombo {
    choice: Choice,
    items: Vec<String>,
    popup: widgets::PopupList,
}

#[derive(Clone, Debug, PartialEq)]
struct Editing {
    value: Value,
    text: String,
    rect: Rect,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct KeyList {
    /// Index into [`GameUiResources::key_rows`] of the selected action row.
    selected: Option<usize>,
    first: usize,
    /// Row waiting for a new key.
    capture: Option<usize>,
}

/// A laid-out control of the current page or sub-dialog, in layer coordinates.
struct Widget<'a> {
    control: &'a Control,
    rect: Rect,
    binding: Option<Binding>,
    enabled: bool,
}

impl Widget<'_> {
    fn interactive(&self) -> bool {
        self.binding
            .is_some_and(|b| !matches!(b, Binding::ValueLabel(_)))
            || matches!(
                self.control.kind,
                ControlKind::Button | ControlKind::UrlButton
            )
    }

    /// Pressed by Space or Enter while focused (SDK `Button` and its check / radio subclasses).
    fn pushable(&self) -> bool {
        matches!(
            self.binding,
            Some(Binding::Toggle(_) | Binding::Skill(_) | Binding::Command(_))
        ) || matches!(
            self.control.kind,
            ControlKind::Button | ControlKind::UrlButton
        )
    }

    /// The area that reacts to the pointer: sliders only along their track band.
    fn hit_rect(&self) -> Rect {
        match self.control.kind {
            ControlKind::Slider => Rect::new(
                self.rect.x,
                self.rect.y + SLIDER_TRACK_Y - SLIDER_HIT_TALL * 0.5,
                self.rect.w,
                SLIDER_HIT_TALL,
            ),
            _ => self.rect,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum CreditItem {
    Label(String),
    Image { path: String, size: Vec2 },
}

pub struct OptionsDialog {
    res: Arc<GameUiResources>,
    modes: DisplayModes,
    pub recommendation: crate::gameui::recommendation::SystemRecommendation,
    tab: Tab,
    applied: Config,
    pub(crate) draft: Draft,
    pub(crate) armed: Option<Target>,
    pub(crate) pressed: Option<Target>,
    last_armed: Option<Target>,
    sounds: Vec<String>,
    tracker: widgets::ButtonTracker<Target>,
    drag: Option<(Value, Rect, widgets::SliderRange)>,
    combo: Option<OpenCombo>,
    editing: Option<Editing>,
    keys: KeyList,
    pub(crate) confirm: Option<MessageBox>,
    sub: Option<Sub>,
    /// Currently fading-out sub-dialog being dismissed.
    pub(crate) closing_sub: Option<Sub>,
    /// Currently fading-out confirmation box being dismissed.
    pub(crate) closing_confirm: Option<MessageBox>,
    /// Sub-dialog composite alpha multiplier (for opening/closing and focus transitions).
    pub sub_alpha: f32,
    /// Confirmation dialog composite alpha multiplier.
    pub confirm_alpha: f32,
    /// Active sub-dialog theme (focus-faded, from Console window management).
    pub sub_theme: Option<Theme>,
    /// Active confirmation dialog theme (focus-faded, from Console window management).
    pub confirm_theme: Option<Theme>,
    /// The draft configuration when the open sub-dialog opened, restored by its Cancel.
    sub_saved: Option<Config>,
    gamma_texture: Option<Texture2D>,
    bink_texture: Option<Texture2D>,
    log: Vec<String>,
    pub(crate) active_tooltip: Option<ActiveTooltip>,
    /// Time of the current input frame, for double clicks and caret blinking.
    pub now: Option<f64>,
    /// Whether the parent Options dialog currently holds active window focus.
    pub parent_focused: bool,
    key_clicks: widgets::DoubleClick<usize>,
    /// Wheel handling for lists: notched on Windows/Linux, accumulated on macOS.
    scroll_mode: widgets::ScrollMode,
    key_wheel: widgets::ScrollAccumulator,
    key_bar: widgets::ScrollBarState,
    /// Where the dialog was dragged (kept while the game runs).
    placement: Placement,
    /// Where the open sub-dialog and the confirmation box were dragged (centered on opening).
    sub_placement: Placement,
    confirm_placement: Placement,
    /// Keyboard focus of the top layer (page or sub-dialog); none until Tab or a click.
    focus: Option<Target>,
    /// The page's focus while a sub-dialog is open, given back when it closes.
    sub_return_focus: Option<Target>,
    key_press: widgets::KeyPress<Target>,
    /// Escape is held and will close the top dialog on release: its close button shows pressed.
    pub(crate) escape_held: bool,
    /// Focus is drawn only after Tab / Shift+Tab; a click focuses without drawing it
    /// (owner request, 2026-10-17).
    focus_visible: bool,
}

impl Default for OptionsDialog {
    fn default() -> Self {
        let config = Config::default();
        Self {
            res: Arc::default(),
            modes: DisplayModes::default(),
            recommendation: crate::gameui::recommendation::SystemRecommendation::detect(),
            tab: Tab::Gameplay,
            draft: Draft::new(config.clone()),
            applied: config,
            armed: None,
            pressed: None,
            last_armed: None,
            sounds: Vec::new(),
            tracker: widgets::ButtonTracker::capturing(),
            drag: None,
            combo: None,
            editing: None,
            keys: KeyList::default(),
            confirm: None,
            sub: None,
            closing_sub: None,
            closing_confirm: None,
            sub_alpha: 1.0,
            confirm_alpha: 1.0,
            sub_theme: None,
            confirm_theme: None,
            sub_saved: None,
            gamma_texture: None,
            bink_texture: None,
            log: Vec::new(),
            active_tooltip: None,
            now: None,
            parent_focused: true,
            key_clicks: widgets::DoubleClick::default(),
            scroll_mode: widgets::ScrollMode::platform(),
            key_wheel: widgets::ScrollAccumulator::default(),
            key_bar: widgets::ScrollBarState::default(),
            placement: Placement::default(),
            sub_placement: Placement::default(),
            confirm_placement: Placement::default(),
            focus: None,
            sub_return_focus: None,
            key_press: widgets::KeyPress::default(),
            escape_held: false,
            focus_visible: false,
        }
    }
}

fn texture(image: &Option<Image>) -> Option<Texture2D> {
    image
        .as_ref()
        .filter(|i| i.rgba.len() == usize::from(i.width) * usize::from(i.height) * 4)
        .map(|i| Texture2D::from_rgba8(i.width, i.height, &i.rgba))
}

fn centered(canvas: Vec2, size: Vec2, scale: f32) -> Vec2 {
    ((canvas - size * scale) * 0.5).round()
}

/// A dialog layer of `size` (layout units) centered on `canvas`, moveable by its caption.
fn layer_frame(canvas: Vec2, size: Vec2, close: fn(Vec2) -> Rect) -> placement::Frame {
    let scale = crate::proportional::dialog_scale(canvas);
    placement::Frame {
        origin: centered(canvas, size, scale),
        size,
        scale,
        canvas,
        min_size: None,
        close,
    }
}

fn main_frame(canvas: Vec2) -> placement::Frame {
    layer_frame(canvas, DIALOG_SIZE, widgets::close_button_rect)
}

fn sub_frame(canvas: Vec2, size: Vec2) -> placement::Frame {
    layer_frame(canvas, size, widgets::close_button_rect)
}

/// The confirmation box has no title-bar close button.
fn confirm_frame(canvas: Vec2, size: Vec2) -> placement::Frame {
    layer_frame(canvas, size, |_| Rect::new(0., 0., 0., 0.))
}

/// The page area below the tabs, in dialog coordinates.
pub fn page_rect() -> Rect {
    let top = CLIENT_TOP + TAB_HEIGHT;
    Rect::new(
        CLIENT_INSET_X,
        top,
        DIALOG_SIZE.x - 2. * CLIENT_INSET_X,
        DIALOG_SIZE.y - top - CLIENT_INSET_BOTTOM - SHEET_INSET_BOTTOM,
    )
}

/// OK, Cancel and Apply, right-aligned 80 apart on the bottom row (SDK `PropertyDialog`).
pub fn button_rects() -> [Rect; 3] {
    let y = DIALOG_SIZE.y - CLIENT_INSET_BOTTOM - 28.;
    let apply = DIALOG_SIZE.x - CLIENT_INSET_X - 80.;
    let rect = |x: f32| Rect::new(x, y, BUTTON_SIZE.x, BUTTON_SIZE.y);
    [rect(apply - 160.), rect(apply - 80.), rect(apply)]
}

fn layout_widgets<'a>(
    res: &'a GameUiResources,
    panel: Panel,
    area: Rect,
    draft: &Draft,
    applied: &Config,
) -> Vec<Widget<'a>> {
    let Some(layout) = res.layout(panel.layout()) else {
        return Vec::new();
    };
    let controls = drawn_controls(panel, layout);
    let (shift, footer) = footer_reflow(panel, &controls, (area.w, area.h));
    let mut widgets: Vec<Widget> = controls
        .into_iter()
        .zip(footer)
        .map(|(control, in_footer)| {
            let (x, y, w, h) = control.rect((area.w, area.h));
            let y = if in_footer { y + shift } else { y };
            let binding = control_binding(panel, control);
            Widget {
                control,
                rect: Rect::new(area.x + x, area.y + y, w, h),
                binding,
                enabled: settings::enabled(binding, control.enabled, draft, applied),
            }
        })
        .collect();
    // SDK `Label`: "If the associate is disabled, so are we".
    let disabled: Vec<&str> = widgets
        .iter()
        .filter(|w| !w.enabled)
        .map(|w| w.control.name.as_str())
        .collect();
    for widget in &mut widgets {
        let associate = &widget.control.associate;
        if !associate.is_empty() && disabled.iter().any(|d| d.eq_ignore_ascii_case(associate)) {
            widget.enabled = false;
        }
    }
    widgets
}

/// Controls whose tops are this close (layout units) share a row of the Tab order: half a
/// standard 24-unit control row.
const FOCUS_ROW_TOLERANCE: f32 = 12.;

/// The Tab order of a layer's controls: enabled interactive controls as they read, left to right
/// in rows from top to bottom (owner decision, 2026-10-17; the installed layouts' `tabPosition`
/// values jump around the pages and leave controls out, so they are not used). A radio group is
/// one stop, its selected button (SDK `RadioButton`).
fn control_order(list: &[Widget], config: &Config) -> Vec<Target> {
    let mut stops: Vec<usize> = list
        .iter()
        .enumerate()
        .filter(|(_, w)| w.enabled && w.interactive())
        .filter(|(_, w)| match w.binding {
            Some(Binding::Skill(skill)) => skill == config.gameplay.skill,
            _ => true,
        })
        .map(|(i, _)| i)
        .collect();
    stops.sort_by(|a, b| list[*a].rect.y.total_cmp(&list[*b].rect.y));
    let mut rows: Vec<Vec<usize>> = Vec::new();
    for i in stops {
        match rows.last_mut() {
            Some(row) if list[i].rect.y - list[row[0]].rect.y <= FOCUS_ROW_TOLERANCE => row.push(i),
            _ => rows.push(vec![i]),
        }
    }
    rows.into_iter()
        .flat_map(|mut row| {
            row.sort_by(|a, b| list[*a].rect.x.total_cmp(&list[*b].rect.x));
            row
        })
        .map(Target::Control)
        .collect()
}

/// Visible key-list rows below the header.
/// The key list's scroll bar: along the right edge, below the header.
fn key_bar_rect(list: Rect) -> Rect {
    Rect::new(
        list.x + list.w - 1. - SCROLLBAR_WIDTH,
        list.y + KEY_HEADER,
        SCROLLBAR_WIDTH,
        list.h - KEY_HEADER - 1.,
    )
}

fn key_rows_visible(list: Rect) -> usize {
    (((list.h - KEY_HEADER) / MENU_ROW).floor() as usize).max(1)
}

fn key_list_scrollable(list: Rect, total: usize) -> bool {
    total > key_rows_visible(list)
}

impl OptionsDialog {
    /// Installs the GameUI resources; textures are created here, never per frame.
    pub fn set_resources(&mut self, res: Arc<GameUiResources>) {
        self.gamma_texture = texture(&res.gamma_image);
        self.bink_texture = texture(&res.bink_image);
        self.res = res;
        self.keys = KeyList::default();
        self.active_tooltip = None;
    }

    /// Resolutions offered on the Video page (from the host's monitor query).
    pub fn set_display_modes(&mut self, modes: DisplayModes) {
        self.modes = modes;
        if self.draft.aspect == crate::gameui::display::AspectRatio::Flexible {
            if let Some(current) = self.modes.current.or(self.modes.desktop) {
                let res = current.without_stripes_for_flexible();
                self.draft.config.video.width = res.width;
                self.draft.config.video.height = res.height;
                if self.applied.video.flexible_aspect {
                    self.applied.video.width = res.width;
                    self.applied.video.height = res.height;
                }
            }
        }
    }

    /// Sets system hardware recommendation profile for video settings.
    pub fn set_system_recommendation(
        &mut self,
        recommendation: crate::gameui::recommendation::SystemRecommendation,
    ) {
        self.recommendation = recommendation;
    }

    /// The detected or configured system hardware recommendation.
    pub fn system_recommendation(&self) -> &crate::gameui::recommendation::SystemRecommendation {
        &self.recommendation
    }

    /// Resets draft video advanced settings and HDR to system recommendations.
    pub fn apply_recommended_video(&mut self) {
        self.draft.config.video.advanced = self.recommendation.video.clone();
        self.draft.config.video.hdr = self.recommendation.hdr;
    }

    /// Constructs an evaluation environment from individual resource references without borrowing
    /// the full OptionsDialog, allowing disjoint mutable access to the draft.
    pub fn make_env<'a>(
        res: &'a GameUiResources,
        modes: &'a DisplayModes,
        rec: &'a crate::gameui::recommendation::SystemRecommendation,
    ) -> Env<'a> {
        Env {
            localization: &res.localization,
            languages: &res.spoken_languages,
            modes,
            recommendation: Some(rec),
        }
    }

    /// Constructs the current evaluation environment with localization, languages, display modes,
    /// and active hardware recommendations.
    pub fn env(&self) -> Env<'_> {
        Self::make_env(&self.res, &self.modes, &self.recommendation)
    }

    /// Restores a placement saved in the settings.
    pub fn set_placement(&mut self, placement: Placement) {
        self.placement = placement;
    }

    /// When the dialog opens on `canvas`: a saved placement that no longer lies fully on it (the
    /// resolution changed) goes back to the centered default.
    pub fn reset_placement_if_off_screen(&mut self, canvas: Vec2) {
        self.placement.reset_if_off_screen(&main_frame(canvas));
    }

    /// The dialog's placement to save, once after a drag or reset changed it.
    pub fn take_placement_changed(
        &mut self,
    ) -> Option<Option<source_ui::placement::DialogPlacement>> {
        self.placement
            .take_changed()
            .then(|| self.placement.saved())
    }

    /// Opens the dialog over `config`, keeping the last selected tab.
    pub fn open(&mut self, config: Config) {
        self.draft = Draft::new(config.clone());
        self.applied = config;
        self.armed = None;
        self.pressed = None;
        self.last_armed = None;
        self.drag = None;
        self.combo = None;
        self.editing = None;
        self.keys.capture = None;
        self.confirm = None;
        self.sub = None;
        self.closing_sub = None;
        self.closing_confirm = None;
        self.sub_theme = None;
        self.confirm_theme = None;
        self.sub_saved = None;
        self.active_tooltip = None;
        self.placement.end();
        self.sub_placement.reset();
        self.confirm_placement.reset();
        self.focus = None;
        self.sub_return_focus = None;
        self.key_press.reset();
        self.parent_focused = true;
    }

    /// The settings were changed outside the dialog (an unconfirmed video change reverted): the
    /// applied state and the draft follow them.
    pub fn reset_applied(&mut self, config: Config) {
        self.draft = Draft::new(config.clone());
        self.applied = config;
        self.combo = None;
    }

    pub fn tab(&self) -> Tab {
        self.tab
    }

    /// The configuration being edited.
    pub fn draft(&self) -> &Config {
        &self.draft.config
    }

    /// The brightness being previewed: the draft's gamma while the brightness sub-dialog is
    /// open (retail applies the slider immediately; Cancel restores the saved value).
    pub fn preview_gamma(&self) -> Option<f32> {
        matches!(self.sub, Some(Sub::Gamma)).then_some(self.draft.config.video.gamma)
    }

    /// The window's display mode changed outside the dialog (for example macOS native full
    /// screen entered with the green button): the applied state follows it, and so does the
    /// draft unless the user already picked a different mode there.
    pub fn sync_display_mode(&mut self, mode: crate::config::DisplayMode) {
        if self.draft.config.video.display_mode == self.applied.video.display_mode {
            self.draft.config.video.display_mode = mode;
        }
        self.applied.video.display_mode = mode;
    }

    /// Whether Apply would change anything.
    pub fn changed(&self) -> bool {
        self.draft.config != self.applied
    }

    /// Messages for the console (placeholder activations).
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }

    /// Drains any queued UI sound cues (e.g. rollover or release activation).
    pub fn take_sounds(&mut self) -> Vec<String> {
        std::mem::take(&mut self.sounds)
    }

    /// Closes the innermost layer. Returns `false` when the dialog itself closes (Cancel).
    /// Whether Escape would close a dialog with a close button (the sub-dialog, else the Options
    /// dialog) or a confirmation dialog with a Cancel button rather than cancel a drag, key
    /// capture, dropdown list, or text edit.
    pub fn escape_closes(&self) -> bool {
        !(self.placement.active()
            || self.sub_placement.active()
            || self.confirm_placement.active()
            || self.keys.capture.is_some()
            || self.combo.is_some()
            || self.editing.is_some())
    }

    pub fn escape(&mut self) -> bool {
        // A drag or resize in progress is undone first.
        if self.confirm_placement.cancel() || self.sub_placement.cancel() || self.placement.cancel()
        {
            return true;
        }
        self.drag = None;
        self.last_armed = None;
        self.active_tooltip = None;
        if self.keys.capture.take().is_some() {
            self.focus = self.key_list_target();
            return true;
        }
        if self.combo.take().is_some()
            || self.editing.take().is_some()
            || self.confirm.take().is_some()
        {
            return true;
        }
        if self.sub.is_some() {
            self.cancel_sub();
            return true;
        }
        false
    }

    fn tab_label(&self, tab: Tab) -> String {
        self.res.localization.text(tab.token())
    }

    /// Tab rectangles as drawn: the active tab is taller and touches the page.
    fn tab_rects(&self, measure: &dyn Fn(&str) -> f32) -> Vec<Rect> {
        let mut x = CLIENT_INSET_X;
        Tab::ALL
            .iter()
            .map(|tab| {
                let w = (measure(&self.tab_label(*tab)) + 2. * TAB_TEXT_INSET).ceil();
                let rect = if *tab == self.tab {
                    Rect::new(x, CLIENT_TOP + 2., w, TAB_HEIGHT - 1.)
                } else {
                    Rect::new(x, CLIENT_TOP + 4., w, TAB_HEIGHT - 3.)
                };
                x += w + 1.;
                rect
            })
            .collect()
    }

    fn placeholder(&mut self, label: &str) {
        let label = label.trim().trim_end_matches("...").trim_end_matches('…');
        self.log
            .push(format!("{label}: not implemented in HL2-RS yet."));
    }

    pub(crate) fn select_tab(&mut self, tab: Tab) {
        self.commit_entry();
        self.combo = None;
        self.active_tooltip = None;
        self.tracker.reset();
        self.key_press.reset();
        if matches!(self.focus, Some(Target::Control(_))) {
            self.focus = None;
        }
        self.tab = tab;
    }

    fn commit_entry(&mut self) {
        if let Some(editing) = self.editing.take() {
            if let Some(value) = widgets::parse_value(&editing.text) {
                editing.value.set(&mut self.draft.config, value);
            }
        }
    }

    /// One input frame. `measure` supplies the canvas size and text metrics.
    pub fn input(&mut self, input: &Input, measure: &Painter) -> OptionsOutcome {
        self.now = input.time;
        let canvas = vec2(measure.canvas.width(), measure.canvas.height());
        let scale = crate::proportional::dialog_scale(canvas);
        let local = |origin: Vec2| input.pointer.map(|p| (p - origin) / scale);
        if let Some(mut confirm) = self.confirm.take() {
            self.occlude(input);
            let layout = confirm.layout(|t| measure.text_width(t, false));
            let frame = confirm_frame(canvas, layout.size);
            if self.confirm_placement.input(
                &frame,
                input.pointer,
                input.click,
                input.held,
                input.time,
            ) {
                self.confirm = Some(confirm);
                return OptionsOutcome::None;
            }
            let pointer = local(self.confirm_placement.place(&frame).origin);
            let down = input.click || input.held;
            let outcome = confirm
                .input(&layout, pointer, down, input.click, input.released, false)
                .or_else(|| confirm.keys(input));
            self.sounds.extend(confirm.take_sounds());
            match outcome {
                Some(true) => {
                    self.use_default_keys();
                    self.closing_confirm = Some(confirm);
                }
                Some(false) => {
                    self.closing_confirm = Some(confirm);
                }
                None => {
                    self.confirm = Some(confirm);
                }
            }
            return OptionsOutcome::None;
        }
        if let Some(sub) = self.sub {
            let size = self.sub_frame_size(sub, |s| measure.text_width(s, false));
            if sub.is_credits()
                && !self.credits_fits_window(sub, canvas, |s| measure.text_width(s, false))
            {
                let scaled_size = size * scale;
                let msg = format!(
                    "Credits dialog content ({}x{}px) cannot fit window bounds ({}x{}px) at current resolution",
                    scaled_size.x.round(),
                    scaled_size.y.round(),
                    canvas.x.round(),
                    canvas.y.round(),
                );
                if !self.log.iter().any(|l| l == &msg) {
                    self.log.push(msg);
                }
            }
            let frame = sub_frame(canvas, size);
            let pointer = local(self.sub_placement.place(&frame).origin);
            // A combo list opened in the sub-dialog is the topmost layer there.
            if let Some(combo) = self.combo.take() {
                self.occlude(input);
                self.combo_input(combo, input, pointer);
                return OptionsOutcome::None;
            }
            if self
                .sub_placement
                .input(&frame, input.pointer, input.click, input.held, input.time)
            {
                self.occlude(input);
                return OptionsOutcome::None;
            }
            return self.sub_input(sub, size, input, pointer, measure);
        }
        let main = main_frame(canvas);
        let pointer = local(self.placement.place(&main).origin);
        if let Some(row) = self.keys.capture {
            self.occlude(input);
            if let Some(key) = input.binding_keys.iter().find(|k| *k != "Escape") {
                if let Some(KeyRow::Action { command, .. }) = self.res.key_rows.get(row) {
                    keys::assign(&mut self.draft.config.keys, command, key);
                }
                self.keys.capture = None;
                // The key list leaves its edit mode with the keyboard (arrows and Enter go to it).
                self.focus = self.key_list_target();
            }
            return OptionsOutcome::None;
        }
        if let Some(combo) = self.combo.take() {
            self.occlude(input);
            self.combo_input(combo, input, pointer);
            return OptionsOutcome::None;
        }
        if self
            .placement
            .input(&main, input.pointer, input.click, input.held, input.time)
        {
            self.occlude(input);
            return OptionsOutcome::None;
        }
        self.page_input(input, pointer, measure)
    }

    /// The dialog's state for run reports: the open tab and sub-dialog, the keyboard focus (a
    /// control's layout name, `tab`, `ok`, `cancel` or `apply`), whether focus is drawn, and
    /// whether Apply would change anything.
    pub fn report(&self) -> OptionsReport {
        let list = match self.sub {
            Some(sub) => {
                let size = self.sub_frame_size(sub, |s| s.chars().count() as f32 * 7.);
                layout_widgets(
                    &self.res,
                    sub.panel(),
                    Rect::new(0., 0., size.x, size.y),
                    &self.draft,
                    &self.applied,
                )
            }
            None => self.tab.panel().map_or_else(Vec::new, |panel| {
                layout_widgets(&self.res, panel, page_rect(), &self.draft, &self.applied)
            }),
        };
        let focus = self.focus.map(|target| match target {
            Target::Control(i) => list
                .get(i)
                .map_or_else(String::new, |w| w.control.name.clone()),
            Target::Tab(_) => "tab".into(),
            Target::Ok => "ok".into(),
            Target::Cancel => "cancel".into(),
            Target::Apply => "apply".into(),
            Target::Close => "close".into(),
        });
        OptionsReport {
            tab: format!("{:?}", self.tab),
            sub: self.sub.map(|s| format!("{s:?}")),
            focus,
            focus_visible: self.focus_visible,
            changed: self.changed(),
            list_open: self.combo.is_some(),
            editing: self.editing.is_some(),
            capturing_key: self.keys.capture.is_some(),
            resolutions: self.resolution_labels(),
            frame: None,
            controls: list.iter().map(|w| w.control.name.clone()).collect(),
            development_controls: list
                .iter()
                .filter(|w| w.binding.and_then(support::development).is_some())
                .map(|w| w.control.name.clone())
                .collect(),
            tooltip: self.active_tooltip.as_ref().map(HintReport::from),
        }
    }

    /// The Resolution combo's choices as shown (for the report).
    fn resolution_labels(&self) -> Vec<String> {
        settings::Choice::Resolution.items(&self.env(), &self.draft)
    }

    /// The Keyboard page's key list as a focus target.
    fn key_list_target(&self) -> Option<Target> {
        let list = layout_widgets(
            &self.res,
            Panel::Keyboard,
            page_rect(),
            &self.draft,
            &self.applied,
        );
        list.iter()
            .position(|w| w.binding == Some(Binding::KeyList))
            .map(Target::Control)
    }

    /// The Keyboard page is waiting for a key to bind (held keys must not repeat into it).
    pub fn capturing_key(&self) -> bool {
        self.keys.capture.is_some()
    }

    /// A sub-dialog is open over this options dialog.
    pub fn has_sub(&self) -> bool {
        self.sub.is_some()
    }

    /// The options reset defaults confirmation box is open.
    pub fn has_confirm(&self) -> bool {
        self.confirm.is_some()
    }

    pub(crate) fn set_confirm_escape_held(&mut self, held: bool) {
        if let Some(confirm) = &mut self.confirm {
            confirm.escape_held = held;
        }
    }

    /// Dismisses the confirmation box.
    pub fn dismiss_confirm(&mut self) {
        if let Some(mut confirm) = self.confirm.take() {
            confirm.escape_held = false;
            self.closing_confirm = Some(confirm);
        }
    }

    /// The dialog's frame on `canvas`, in canvas pixels, where it is placed.
    pub fn frame_bounds(&self, canvas: Vec2) -> Rect {
        let frame = main_frame(canvas);
        let placed = self.placement.place(&frame);
        Rect::new(
            placed.origin.x,
            placed.origin.y,
            placed.size.x * frame.scale,
            placed.size.y * frame.scale,
        )
    }

    /// A sub-dialog's frame on `canvas`, in canvas pixels, if one is open.
    pub fn sub_bounds(&self, canvas: Vec2) -> Rect {
        if let Some(sub) = self.sub {
            let size = self.sub_frame_size(sub, |s| s.chars().count() as f32 * 7.);
            let frame = sub_frame(canvas, size);
            let placed = self.sub_placement.place(&frame);
            Rect::new(
                placed.origin.x,
                placed.origin.y,
                placed.size.x * frame.scale,
                placed.size.y * frame.scale,
            )
        } else {
            Rect::default()
        }
    }

    /// The options reset confirmation frame on `canvas`, in canvas pixels, if one is open.
    pub fn confirm_bounds(&self, canvas: Vec2) -> Rect {
        if let Some(confirm) = &self.confirm {
            let size = confirm.layout(|s| s.chars().count() as f32 * 7.).size;
            let frame = confirm_frame(canvas, size);
            let placed = self.confirm_placement.place(&frame);
            Rect::new(
                placed.origin.x,
                placed.origin.y,
                placed.size.x * frame.scale,
                placed.size.y * frame.scale,
            )
        } else {
            Rect::default()
        }
    }

    /// A layer that takes every click while it is open, wherever the pointer is: a sub-dialog
    /// (retail opens them modal), the confirmation box, an open drop-down list (an outside
    /// click closes it) or a key capture.
    pub fn modal_layer_open(&self) -> bool {
        self.sub.is_some()
            || self.confirm.is_some()
            || self.combo.is_some()
            || self.keys.capture.is_some()
    }

    /// A layer above the dialog page owns the pointer this frame.
    pub(crate) fn occlude(&mut self, input: &Input) {
        self.active_tooltip = None;
        self.armed = None;
        self.pressed = None;
        self.last_armed = None;
        self.tracker.occlude(input.held);
    }

    fn combo_input(&mut self, mut combo: OpenCombo, input: &Input, pointer: Option<Vec2>) {
        let popup = &mut combo.popup;
        popup.wheel(input.wheel, self.scroll_mode);
        if input.pressed(Key::Up) || input.pressed(Key::Down) {
            popup.step(if input.pressed(Key::Up) { -1 } else { 1 });
        }
        let event = popup.pointer(pointer, input.held, input.click, input.released, self.now);
        let pick = match event {
            Some(widgets::PopupEvent::Select(index)) => Some(Some(index)),
            Some(widgets::PopupEvent::Close) => Some(None),
            None if input.pressed(Key::Enter) => Some(popup.highlight),
            None => None,
        };
        match pick {
            Some(Some(index)) => {
                self.sounds.push("ui/buttonclickrelease.wav".to_string());
                let env = Self::make_env(&self.res, &self.modes, &self.recommendation);
                combo.choice.select(&env, &mut self.draft, index);
            }
            Some(None) => {}
            None => self.combo = Some(combo),
        }
    }

    /// Slider drags, text editing and hover for a layer. Returns the clicked target.
    fn track(
        &mut self,
        widgets: &[Widget],
        extra: &[(Target, Rect)],
        pointer: Option<Vec2>,
        input: &Input,
    ) -> Option<Target> {
        if let Some((value, rect, range)) = self.drag {
            if input.held {
                if let Some(p) = pointer {
                    let fraction = widgets::slider_fraction_at(rect, p.x);
                    value.set(&mut self.draft.config, range.value(fraction));
                }
                return None;
            }
            self.drag = None;
        }
        if let Some(editing) = self.editing.as_mut() {
            for c in input.text.chars() {
                if (c.is_ascii_digit() || c == '.' || c == '-') && editing.text.len() < 12 {
                    editing.text.push(c);
                }
            }
            if input.pressed(Key::Backspace) {
                editing.text.pop();
            }
            if input.pressed(Key::Enter) {
                self.commit_entry();
            }
        }
        let hit = pointer.and_then(|p| {
            extra
                .iter()
                .find(|(_, rect)| rect.contains(p))
                .map(|(target, _)| *target)
                .or_else(|| {
                    widgets
                        .iter()
                        .enumerate()
                        .rev()
                        .find(|(_, w)| w.interactive() && w.hit_rect().contains(p))
                        .map(|(i, _)| Target::Control(i))
                })
        });
        self.armed = self.parent_focused.then_some(hit).flatten();
        let armed_target = self.armed.filter(|target| match target {
            Target::Control(i) => widgets.get(*i).is_some_and(|w| w.enabled),
            _ => true,
        });
        if let Some(target) = armed_target {
            if armed_target != self.last_armed && target.has_rollover_sound(widgets) {
                self.sounds.push("ui/buttonrollover.wav".to_string());
            }
        }
        self.last_armed = armed_target;
        if input.click {
            let keep_editing = matches!(
                (&self.editing, pointer),
                (Some(editing), Some(p)) if editing.rect.contains(p)
            );
            if !keep_editing {
                self.commit_entry();
            }
            // SDK `Button::OnMousePressed` (and the other controls) request focus on a press;
            // focus taken by the mouse is not drawn.
            if let Some(target) = armed_target.filter(|t| *t != Target::Close) {
                self.focus = Some(target);
                self.focus_visible = false;
                self.key_press.reset();
            }
        }
        let is_slider = matches!(
            hit,
            Some(Target::Control(i)) if matches!(widgets.get(i), Some(w) if matches!(w.binding, Some(Binding::Slider(_))))
        );
        // A press captured by another control keeps the mouse while it crosses a slider.
        if is_slider && self.tracker.pressed().is_none() {
            self.tracker.reset();
            self.pressed = hit.filter(|_| input.held || input.click);
            if input.click {
                return hit;
            }
            return None;
        }
        let activated = self
            .tracker
            .update(hit, input.held, input.click, input.released);
        self.pressed = self.tracker.pressed();
        self.armed = self
            .parent_focused
            .then(|| self.tracker.armable(hit))
            .flatten();
        activated
    }

    fn page_input(
        &mut self,
        input: &Input,
        pointer: Option<Vec2>,
        measure: &Painter,
    ) -> OptionsOutcome {
        let res = self.res.clone();
        let page = page_rect();
        let list = self.tab.panel().map_or_else(Vec::new, |panel| {
            layout_widgets(&res, panel, page, &self.draft, &self.applied)
        });
        let tabs: Vec<(Target, Rect)> = self
            .tab_rects(&|t| measure.text_width(t, false))
            .into_iter()
            .enumerate()
            .map(|(i, r)| (Target::Tab(i), Rect::new(r.x, CLIENT_TOP, r.w, TAB_HEIGHT)))
            .collect();
        let [ok, cancel, apply] = button_rects();
        let mut extra = tabs;
        extra.extend([
            (Target::Ok, ok),
            (Target::Cancel, cancel),
            (Target::Apply, apply),
            (Target::Close, widgets::close_button_rect(DIALOG_SIZE)),
        ]);
        let active = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
        let mut order = vec![Target::Tab(active)];
        order.extend(control_order(&list, &self.draft.config));
        order.extend([Target::Ok, Target::Cancel]);
        if self.changed() {
            order.push(Target::Apply);
        }
        if let Some(outcome) = self.focus_keys(&list, &order, input) {
            return outcome;
        }
        let key_list_focus = matches!(
            self.focus,
            Some(Target::Control(i)) if matches!(list.get(i).and_then(|w| w.binding), Some(Binding::KeyList))
        );
        if self.tab == Tab::Keyboard {
            self.key_list_keys(input, &list, key_list_focus);
            if self.key_list_bar(input, pointer, &list) {
                return OptionsOutcome::None;
            }
        }
        let clicked = self.track(&list, &extra, pointer, input);
        if let Some(panel) = self.tab.panel() {
            self.update_tooltip(
                panel,
                &list,
                pointer.filter(|p| page.contains(*p)),
                input,
                measure,
            );
        }
        match clicked {
            Some(Target::Tab(i)) => {
                self.sounds.push("ui/buttonclickrelease.wav".to_string());
                self.select_tab(Tab::ALL[i]);
                self.focus = Some(Target::Tab(i));
            }
            Some(Target::Ok) => {
                self.sounds.push("ui/buttonclickrelease.wav".to_string());
                return self.ok();
            }
            Some(Target::Cancel) | Some(Target::Close) => {
                self.sounds.push("ui/buttonclickrelease.wav".to_string());
                return OptionsOutcome::Close(None);
            }
            Some(Target::Apply) => {
                self.sounds.push("ui/buttonclickrelease.wav".to_string());
                return self.apply();
            }
            Some(Target::Control(i)) => {
                if let (Some(widget), Some(p)) = (list.get(i), pointer) {
                    if !matches!(widget.binding, Some(Binding::Slider(_))) {
                        self.sounds.push("ui/buttonclickrelease.wav".to_string());
                    }
                    return self.activate(widget, p);
                }
            }
            None => {}
        }
        OptionsOutcome::None
    }

    fn ok(&mut self) -> OptionsOutcome {
        self.commit_entry();
        OptionsOutcome::Close(self.changed().then(|| self.draft.config.clone()))
    }

    fn apply(&mut self) -> OptionsOutcome {
        self.commit_entry();
        if !self.changed() {
            return OptionsOutcome::None;
        }
        self.applied = self.draft.config.clone();
        OptionsOutcome::Apply(self.applied.clone())
    }

    fn activate(&mut self, widget: &Widget, pointer: Vec2) -> OptionsOutcome {
        if !widget.enabled {
            if let Some(Binding::Command(command)) = widget.binding {
                if command.placeholder() {
                    let label = self.res.localization.localize(&widget.control.label);
                    self.placeholder(&label);
                }
            }
            return OptionsOutcome::None;
        }
        match widget.binding {
            Some(Binding::Toggle(toggle)) => {
                let value = toggle.get(&self.draft.config);
                toggle.set(&mut self.draft.config, !value);
            }
            Some(Binding::Skill(skill)) => self.draft.config.gameplay.skill = skill,
            Some(Binding::Slider(value)) => {
                let fraction = widgets::slider_fraction_at(widget.rect, pointer.x);
                let range = value.slider_range(widget.control.range);
                value.set(&mut self.draft.config, range.value(fraction));
                self.drag = Some((value, widget.rect, range));
            }
            Some(Binding::Entry(value)) => {
                if self.editing.as_ref().is_none_or(|e| e.value != value) {
                    self.editing = Some(Editing {
                        value,
                        text: widgets::format_value(value.get(&self.draft.config)),
                        rect: widget.rect,
                    });
                }
            }
            Some(Binding::Choice(choice)) => self.open_combo(choice, widget.rect),
            Some(Binding::Command(command)) => return self.command(command),
            Some(Binding::KeyList) => self.key_list_click(widget.rect, pointer),
            Some(Binding::ValueLabel(_)) | None => {}
        }
        OptionsOutcome::None
    }

    fn open_combo(&mut self, choice: Choice, rect: Rect) {
        let env = self.env();
        let items = choice.items(&env, &self.draft);
        let selected = choice.selected(&env, &self.draft);
        let popup = widgets::PopupList::below(rect, items.len(), selected);
        self.combo = Some(OpenCombo {
            choice,
            items,
            popup,
        });
    }

    /// Keyboard focus keys of the top layer, before the pointer: Tab / Shift+Tab, the focused
    /// control's own keys and Space / Enter on a focused button. Returns an outcome when the
    /// frame's keys were consumed. Text being edited keeps every key but Tab.
    fn focus_keys(
        &mut self,
        list: &[Widget],
        order: &[Target],
        input: &Input,
    ) -> Option<OptionsOutcome> {
        if self.focus.is_some_and(|f| !order.contains(&f)) {
            self.focus = None;
            self.key_press.reset();
        }
        if input.pressed(Key::Tab) {
            self.commit_entry();
            self.key_press.reset();
            self.focus = widgets::focus_step(self.focus, order, !input.shift);
            self.focus_visible = true;
            if let Some(Target::Control(i)) = self.focus {
                if let Some(
                    widget @ Widget {
                        binding: Some(Binding::Entry(_)),
                        ..
                    },
                ) = list.get(i)
                {
                    // A focused text entry edits at once (SDK `TextEntry` takes the caret).
                    let _ = self.activate(widget, widget.rect.center());
                }
            }
            return Some(OptionsOutcome::None);
        }
        if self.editing.is_some() {
            return None;
        }
        let focus = self.focus?;
        let widget = match focus {
            Target::Control(i) => list.get(i),
            _ => None,
        };
        // SDK `RadioButton`: arrows select the previous / next button of the group (wrapping).
        if let Some(Binding::Skill(skill)) = widget.and_then(|w| w.binding) {
            let step = if input.pressed(Key::Up) || input.pressed(Key::Left) {
                -1
            } else if input.pressed(Key::Down) || input.pressed(Key::Right) {
                1
            } else {
                0
            };
            if step != 0 {
                let mut group: Vec<(u32, usize, u8)> = list
                    .iter()
                    .enumerate()
                    .filter_map(|(i, w)| match w.binding {
                        Some(Binding::Skill(s)) if w.enabled => {
                            Some((w.control.sub_tab_position, i, s))
                        }
                        _ => None,
                    })
                    .collect();
                group.sort_unstable();
                if let Some(at) = group.iter().position(|(_, _, s)| *s == skill) {
                    let next = (at as i32 + step).rem_euclid(group.len() as i32) as usize;
                    let (_, index, next_skill) = group[next];
                    self.draft.config.gameplay.skill = next_skill;
                    self.focus = Some(Target::Control(index));
                }
                return Some(OptionsOutcome::None);
            }
        }
        let pushable = match focus {
            Target::Ok | Target::Cancel | Target::Apply => true,
            Target::Control(_) => widget.is_some_and(|w| w.pushable()),
            Target::Tab(_) | Target::Close => false,
        };
        if pushable || self.key_press.pressed().is_some() {
            let key = [Key::Space, Key::Enter]
                .into_iter()
                .find(|k| pushable && input.pressed(*k));
            let pressed = self.key_press.pressed();
            if let Some(target) = self
                .key_press
                .update(Some(focus), key, |k| input.held_keys.contains(&k))
            {
                self.sounds.push("ui/buttonclickrelease.wav".to_string());
                return Some(match target {
                    Target::Ok => self.ok(),
                    // A sub-dialog's own Close (credits) closes the sub-dialog only.
                    Target::Cancel if self.sub.is_some() => self.command(Command::SubCancel),
                    Target::Cancel => OptionsOutcome::Close(None),
                    Target::Apply => self.apply(),
                    Target::Control(i) => match list.get(i) {
                        Some(widget) => self.activate(widget, widget.rect.center()),
                        None => OptionsOutcome::None,
                    },
                    Target::Tab(_) | Target::Close => OptionsOutcome::None,
                });
            }
            if key.is_some() || pressed.is_some() {
                return Some(OptionsOutcome::None);
            }
        }
        match (focus, widget.and_then(|w| w.binding)) {
            // SDK `PropertySheet`: Left / Right on the focused tab switch pages.
            (Target::Tab(i), _) => {
                let next = if input.pressed(Key::Left) {
                    i.checked_sub(1)
                } else if input.pressed(Key::Right) {
                    Some(i + 1).filter(|n| *n < Tab::ALL.len())
                } else {
                    return None;
                };
                if let Some(next) = next {
                    self.sounds.push("ui/buttonclickrelease.wav".to_string());
                    self.select_tab(Tab::ALL[next]);
                    self.focus = Some(Target::Tab(next));
                }
                Some(OptionsOutcome::None)
            }
            // SDK `Slider`: Left / Down and Right / Up step by one unit, Page Down / Up by a tick.
            // The retail slider classes are not in the SDK; a unit is a hundredth of the range
            // and a tick a tenth (unverified).
            (Target::Control(_), Some(Binding::Slider(value))) => {
                let step = if input.pressed(Key::Left) || input.pressed(Key::Down) {
                    -0.01
                } else if input.pressed(Key::Right) || input.pressed(Key::Up) {
                    0.01
                } else if input.pressed(Key::PageDown) {
                    -0.1
                } else if input.pressed(Key::PageUp) {
                    0.1
                } else {
                    return None;
                };
                let range = value.slider_range(widget?.control.range);
                let fraction = range.fraction(value.get(&self.draft.config));
                value.set(
                    &mut self.draft.config,
                    range.value((fraction + step).clamp(0., 1.)),
                );
                Some(OptionsOutcome::None)
            }
            // SDK `ComboBox`: Up / Down (and Home / End) select without opening the list;
            // Alt+Up / Down opens it; Enter is taken by the combo box.
            (Target::Control(_), Some(Binding::Choice(choice))) => {
                let vertical = input.pressed(Key::Up) || input.pressed(Key::Down);
                if input.alt && vertical {
                    self.open_combo(choice, widget?.rect);
                    return Some(OptionsOutcome::None);
                }
                let env = Self::make_env(&self.res, &self.modes, &self.recommendation);
                let count = choice.items(&env, &self.draft).len();
                let selected = choice.selected(&env, &self.draft);
                let last = count.checked_sub(1)?;
                let next = if input.pressed(Key::Up) {
                    selected.map_or(0, |s| s.saturating_sub(1))
                } else if input.pressed(Key::Down) {
                    selected.map_or(0, |s| (s + 1).min(last))
                } else if input.pressed(Key::Home) {
                    0
                } else if input.pressed(Key::End) {
                    last
                } else if input.pressed(Key::Enter) {
                    return Some(OptionsOutcome::None);
                } else {
                    return None;
                };
                if Some(next) != selected {
                    choice.select(&env, &mut self.draft, next);
                }
                Some(OptionsOutcome::None)
            }
            _ => None,
        }
    }

    pub(crate) fn open_sub(&mut self, sub: Sub) {
        self.sub_placement.reset();
        self.sub_saved = Some(self.draft.config.clone());
        self.closing_sub = None;
        self.sub = Some(sub);
        self.sub_return_focus = self.focus.take();
        // The credits' Close button is their only control: it holds the focus (drawn after Tab).
        if sub.is_credits() {
            self.focus = Some(Target::Cancel);
        }
        self.key_press.reset();
    }

    /// Focus goes back to the page control that opened the closed sub-dialog.
    fn close_sub(&mut self) {
        self.closing_sub = self.sub.take();
        self.focus = self.sub_return_focus.take();
        self.key_press.reset();
    }

    /// Cancel, Escape or the title-bar close of a sub-dialog: its edits are dropped.
    pub fn cancel_sub(&mut self) {
        if let Some(saved) = self.sub_saved.take() {
            self.draft.config = saved;
        }
        self.close_sub();
    }

    /// OK of a sub-dialog: its edits stay in the draft, and an advanced dialog applies its own
    /// settings at once (other unapplied edits stay unapplied).
    fn ok_sub(&mut self) -> OptionsOutcome {
        self.commit_entry();
        self.sub_saved = None;
        let Some(sub) = self.sub else {
            return OptionsOutcome::None;
        };
        self.close_sub();
        let mut applied = self.applied.clone();
        if sub.apply_on_ok(&self.draft.config, &mut applied) && applied != self.applied {
            self.applied = applied;
            return OptionsOutcome::Apply(self.applied.clone());
        }
        OptionsOutcome::None
    }

    pub(crate) fn command(&mut self, command: Command) -> OptionsOutcome {
        self.tracker.reset();
        let loc = &self.res.localization;
        match command {
            Command::GammaDialog => self.open_sub(Sub::Gamma),
            Command::AudioCredits => self.open_sub(Sub::AudioCredits),
            Command::VideoCredits => self.open_sub(Sub::VideoCredits),
            Command::KeyAdvanced => self.open_sub(Sub::KeyboardAdvanced),
            Command::KeyDefaults => {
                self.confirm_placement.reset();
                self.closing_confirm = None;
                self.confirm = Some(MessageBox::new(
                    loc.text("#GameUI_KeyboardSettings"),
                    loc.text("#GameUI_KeyboardSettingsText"),
                    loc.text("#GameUI_OK"),
                    Some(loc.text("#GameUI_Cancel")),
                ))
            }
            Command::KeyEdit => self.keys.capture = self.keys.selected,
            Command::KeyClear => {
                if let Some(KeyRow::Action { command, .. }) =
                    self.keys.selected.and_then(|i| self.res.key_rows.get(i))
                {
                    self.draft
                        .config
                        .keys
                        .insert(command.clone(), keys::UNBOUND.into());
                }
            }
            Command::VideoAdvanced => self.open_sub(Sub::VideoAdvanced),
            // Placeholder: disabled, so activation only logs its label (see `activate`).
            Command::SteamInput
            | Command::TestSpeakers
            | Command::HDContent
            | Command::MigrateData => {}
            Command::SubOk => return self.ok_sub(),
            Command::SubCancel => self.cancel_sub(),
        }
        OptionsOutcome::None
    }

    /// Resets the `kb_act.lst` actions to the installed defaults (actions without one become
    /// unbound, as retail `unbindall` + default config does); other bindings are kept unless a
    /// default takes their key.
    fn use_default_keys(&mut self) {
        let res = self.res.clone();
        let keys = &mut self.draft.config.keys;
        for row in &res.key_rows {
            if let KeyRow::Action { command, .. } = row {
                keys.insert(command.clone(), keys::UNBOUND.into());
            }
        }
        for row in &res.key_rows {
            if let KeyRow::Action { command, .. } = row {
                if let Some(key) = res.default_bindings.get(command) {
                    keys::assign(keys, command, key);
                }
            }
        }
    }

    /// The key list's scroll bar while it has more rows than fit.
    fn key_bar_layout(&self, rect: Rect) -> Option<widgets::ScrollBarLayout> {
        let total = self.res.key_rows.len();
        key_list_scrollable(rect, total).then(|| {
            widgets::ScrollBarLayout::new(
                key_bar_rect(rect),
                self.keys.first,
                key_rows_visible(rect),
                total,
            )
        })
    }

    /// A gesture on the key list's scroll bar owns the pointer from press to release and
    /// scrolls live; the rest of the page is occluded meanwhile. Returns whether it took the
    /// frame.
    fn key_list_bar(&mut self, input: &Input, pointer: Option<Vec2>, list: &[Widget]) -> bool {
        let Some(rect) = Self::key_list_rect(list) else {
            return false;
        };
        let Some(layout) = self.key_bar_layout(rect) else {
            return false;
        };
        if !self.key_bar.wants(&layout, pointer, input.click) {
            return false;
        }
        let page = key_rows_visible(rect);
        if let Some(first) = self.key_bar.pointer(
            &layout,
            pointer,
            input.held,
            input.click,
            self.now,
            self.keys.first,
            page,
        ) {
            self.keys.first = first;
        }
        self.occlude(input);
        true
    }

    fn key_list_rect(list: &[Widget]) -> Option<Rect> {
        list.iter()
            .find(|w| w.binding == Some(Binding::KeyList))
            .map(|w| w.rect)
    }

    /// Wheel scrolling and, while the list has keyboard focus (or nothing has), arrow navigation
    /// and Enter-to-edit on the Keyboard page.
    fn key_list_keys(&mut self, input: &Input, list: &[Widget], focused: bool) {
        let Some(rect) = Self::key_list_rect(list) else {
            return;
        };
        let rows = &self.res.key_rows;
        let visible = key_rows_visible(rect);
        let max_first = rows.len().saturating_sub(visible);
        if self.combo.is_none() {
            let rows = self.scroll_mode.rows_per_step(WHEEL_ROWS) as i32;
            let steps = self.key_wheel.steps(input.wheel, self.scroll_mode) * rows;
            self.keys.first = (self.keys.first as i32 - steps).clamp(0, max_first as i32) as usize;
        }
        if !focused {
            return;
        }
        let step = if input.pressed(Key::Up) {
            -1
        } else if input.pressed(Key::Down) {
            1
        } else {
            0
        };
        if step != 0 {
            let actions: Vec<usize> = rows
                .iter()
                .enumerate()
                .filter(|(_, r)| matches!(r, KeyRow::Action { .. }))
                .map(|(i, _)| i)
                .collect();
            let position = self
                .keys
                .selected
                .and_then(|s| actions.iter().position(|a| *a == s));
            let next = match position {
                None if step > 0 => actions.first(),
                None => actions.last(),
                Some(p) => {
                    actions.get((p as i32 + step).clamp(0, actions.len() as i32 - 1) as usize)
                }
            };
            if let Some(&next) = next {
                self.keys.selected = Some(next);
                if next < self.keys.first {
                    self.keys.first = next;
                } else if next >= self.keys.first + visible {
                    self.keys.first = (next + 1 - visible).min(max_first);
                }
            }
        }
        if input.pressed(Key::Enter) && self.editing.is_none() && self.keys.selected.is_some() {
            self.keys.capture = self.keys.selected;
        }
    }

    fn key_list_click(&mut self, rect: Rect, p: Vec2) {
        let total = self.res.key_rows.len();
        let body = Rect::new(rect.x, rect.y + KEY_HEADER, rect.w, rect.h - KEY_HEADER);
        if !body.contains(p) {
            return;
        }
        // The scroll bar handles its own presses (`key_list_bar`).
        if key_list_scrollable(rect, total) && key_bar_rect(rect).contains(p) {
            return;
        }
        let index = self.keys.first + ((p.y - body.y) / MENU_ROW).floor() as usize;
        if matches!(self.res.key_rows.get(index), Some(KeyRow::Action { .. })) {
            self.keys.selected = Some(index);
            // A double click on an action row edits its key, like the Edit Key button.
            if self.key_clicks.register(index, self.now) {
                self.keys.capture = Some(index);
            }
        }
    }

    fn sub_input(
        &mut self,
        sub: Sub,
        size: Vec2,
        input: &Input,
        pointer: Option<Vec2>,
        measure: &Painter,
    ) -> OptionsOutcome {
        if sub.is_credits() {
            self.active_tooltip = None;
            self.credits_input(size, input, pointer);
            return OptionsOutcome::None;
        }
        let res = self.res.clone();
        let area = Rect::new(0., 0., size.x, size.y);
        let list = layout_widgets(&res, sub.panel(), area, &self.draft, &self.applied);
        let order = control_order(&list, &self.draft.config);
        if let Some(outcome) = self.focus_keys(&list, &order, input) {
            return outcome;
        }
        let close_rect = widgets::close_button_rect(size);
        let extra = [(Target::Close, close_rect)];
        let clicked = self.track(&list, &extra, pointer, input);
        self.update_tooltip(sub.panel(), &list, pointer, input, measure);
        match clicked {
            Some(Target::Close) => {
                self.sounds.push("ui/buttonclickrelease.wav".to_string());
                self.command(Command::SubCancel)
            }
            Some(Target::Control(i)) => match (list.get(i), pointer) {
                (Some(widget), Some(p)) => {
                    if !matches!(widget.binding, Some(Binding::Slider(_))) {
                        self.sounds.push("ui/buttonclickrelease.wav".to_string());
                    }
                    self.activate(widget, p)
                }
                _ => OptionsOutcome::None,
            },
            _ => OptionsOutcome::None,
        }
    }

    fn credits_items(&self, sub: Sub) -> Vec<CreditItem> {
        let mut items = Vec::new();
        if let Some(layout) = self.res.layout(sub.panel().layout()) {
            let mut controls: Vec<&Control> = layout
                .controls
                .iter()
                .filter(|c| {
                    c.visible
                        && !c.name.eq_ignore_ascii_case("Button2")
                        && !matches!(c.kind, ControlKind::Button | ControlKind::Other(_))
                })
                .collect();
            controls.sort_by(|a, b| {
                a.y.resolve(CREDITS_SIZE.y)
                    .partial_cmp(&b.y.resolve(CREDITS_SIZE.y))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for c in controls {
                match c.kind {
                    ControlKind::Label => {
                        items.push(CreditItem::Label(c.label.clone()));
                    }
                    ControlKind::ImagePanel => {
                        items.push(CreditItem::Image {
                            path: c.image.clone(),
                            size: Vec2::new(c.wide, c.tall),
                        });
                    }
                    _ => {}
                }
            }
        }
        if items.is_empty() {
            match sub {
                Sub::AudioCredits => {
                    items.push(CreditItem::Label("#GameUI_Miles_Audio".into()));
                    items.push(CreditItem::Label("#GameUI_Miles_Voice".into()));
                    items.push(CreditItem::Label("#GameUI_Bink".into()));
                    items.push(CreditItem::Image {
                        path: "vgui/techcredits/bink".into(),
                        size: Vec2::new(100., 40.),
                    });
                }
                Sub::VideoCredits => {
                    items.push(CreditItem::Label("#GameUI_Bink".into()));
                    items.push(CreditItem::Image {
                        path: "vgui/techcredits/bink".into(),
                        size: Vec2::new(100., 40.),
                    });
                }
                _ => {}
            }
        }
        items
    }

    fn measure_credits_height(
        &self,
        items: &[CreditItem],
        content_w: f32,
        measure_w: impl Fn(&str) -> f32,
    ) -> f32 {
        let mut total_h = 0.0;
        let loc = &self.res.localization;
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                total_h += 10.0;
            }
            match item {
                CreditItem::Label(token) => {
                    let text = loc.localize(token);
                    let lines = widgets::wrap(&text, content_w, &measure_w);
                    total_h += lines.len() as f32 * widgets::FONT_TALL;
                }
                CreditItem::Image { size, .. } => {
                    total_h += size.y;
                }
            }
        }
        total_h
    }

    /// Frame size of a sub-dialog: sized flexibly to wrapped content for credits dialogs,
    /// or layout / fallback size for other sub-dialogs.
    pub(crate) fn sub_frame_size(&self, sub: Sub, measure_w: impl Fn(&str) -> f32) -> Vec2 {
        if sub.is_credits() {
            let items = self.credits_items(sub);
            let content_w = CREDITS_SIZE.x - 32.;
            let content_h = self.measure_credits_height(&items, content_w, measure_w);
            // 34. top inset (title bar) + content_h + 12. gap + 24. close button + 10. bottom margin = 46. bottom inset
            vec2(CREDITS_SIZE.x, 34. + content_h + 46.)
        } else {
            frame_size(&self.res, sub)
        }
    }

    /// Returns whether the credits subdialog content fits completely within the window at `canvas` resolution.
    fn credits_fits_window(&self, sub: Sub, canvas: Vec2, measure_w: impl Fn(&str) -> f32) -> bool {
        let scale = crate::proportional::dialog_scale(canvas);
        let size = self.sub_frame_size(sub, measure_w);
        size.x * scale <= canvas.x && size.y * scale <= canvas.y
    }

    fn draw_credits_subdialog(&self, sp: &Painter, sub: Sub, size: Vec2, modal: bool) {
        let loc = &self.res.localization;
        let items = self.credits_items(sub);
        let content_w = size.x - 32.;
        let mut y_cursor = 34.;

        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                y_cursor += 10.0;
            }
            match item {
                CreditItem::Label(token) => {
                    let text = loc.localize(token);
                    let lines = widgets::wrap(&text, content_w, |s| sp.text_width(s, false));
                    for line in lines {
                        sp.text(&line, Vec2::new(16., y_cursor), sp.theme.text, false);
                        y_cursor += widgets::FONT_TALL;
                    }
                }
                CreditItem::Image {
                    path,
                    size: img_size,
                } => {
                    let texture = if path.to_ascii_lowercase().contains("bink") {
                        self.bink_texture.as_ref()
                    } else {
                        None
                    };
                    if let Some(tex) = texture {
                        sp.image(Rect::new(16., y_cursor, img_size.x, img_size.y), tex);
                    }
                    y_cursor += img_size.y;
                }
            }
        }

        let close_rect = Rect::new(size.x - 96., size.y - 34., 80., 24.);
        let close_armed = !modal && self.armed == Some(Target::Cancel);
        let close_pressed = !modal
            && (self.pressed == Some(Target::Cancel)
                || self.key_press.pressed() == Some(Target::Cancel));
        sp.button(
            close_rect,
            &loc.text("#GameUI_Close"),
            ButtonState {
                armed: close_armed,
                pressed: close_pressed,
                enabled: !modal,
                default: false,
            },
        );
        if !modal && self.focus_visible && self.focus == Some(Target::Cancel) {
            sp.focus_box(close_rect);
        }
    }

    fn credits_input(&mut self, size: Vec2, input: &Input, pointer: Option<Vec2>) {
        let close_rect = Rect::new(size.x - 96., size.y - 34., 80., 24.);
        let title_close_rect = widgets::close_button_rect(size);
        let extra = [
            (Target::Close, title_close_rect),
            (Target::Cancel, close_rect),
        ];

        // Tab keeps the focus on Close and shows it; Space / Enter press and release it.
        if self.focus_keys(&[], &[Target::Cancel], input).is_some() {
            return;
        }
        if let Some(Target::Close | Target::Cancel) = self.track(&[], &extra, pointer, input) {
            self.sounds.push("ui/buttonclickrelease.wav".to_string());
            self.command(Command::SubCancel);
        }
    }

    // ---- drawing -------------------------------------------------------------------------

    pub fn draw(&self, base: &Painter) {
        self.draw_with_occluders(base, &[], self.now, None);
    }

    pub fn draw_with_occluders(
        &self,
        base: &Painter,
        external_occluders: &[Rect],
        time: Option<f64>,
        parent_focused: Option<bool>,
    ) {
        let time_val = time.or(self.now).unwrap_or(0.0);
        let canvas = vec2(base.canvas.width(), base.canvas.height());
        let scale = crate::proportional::dialog_scale(canvas);
        let modal = self.sub.is_some() || self.confirm.is_some();
        let options_theme =
            if parent_focused.is_some() || self.sub_theme.is_some() || self.confirm_theme.is_some()
            {
                base.theme.clone()
            } else {
                base.theme
                    .focus_faded(if modal { 0.0 } else { 1.0 }, !modal)
            };
        let parent_focused = parent_focused.unwrap_or(self.parent_focused) && !modal;
        let p = Painter {
            canvas: base.canvas,
            fonts: base.fonts,
            theme: &options_theme,
            origin: self.placement.place(&main_frame(canvas)).origin,
            scale,
        };
        // Sub-dialogs and confirmation boxes adapt their focus theme based on which is topmost.
        let sub_theme = self.sub_theme.clone().unwrap_or_else(|| {
            base.theme.focus_faded(
                if self.confirm.is_some() { 0.0 } else { 1.0 },
                self.confirm.is_none(),
            )
        });
        let confirm_theme = self
            .confirm_theme
            .clone()
            .unwrap_or_else(|| base.theme.focus_faded(1.0, true));
        let loc = &self.res.localization;
        let env = self.env();
        let state = |target: Target| {
            (
                parent_focused && self.armed == Some(target),
                !modal
                    && (self.pressed == Some(target) || self.key_press.pressed() == Some(target)),
            )
        };
        let focused = |target: Target| !modal && self.focus_visible && self.focus == Some(target);
        let (close_armed, close_pressed) = state(Target::Close);
        let close_pressed = close_pressed || (self.escape_held && self.sub.is_none());
        let (sub_bounds, confirm_bounds) = self.overlay_bounds(base, canvas, scale);

        // 1. Draw Options frame background (unclipped).
        let options_title = loc.text("#GameUI_Options");
        let spec = FrameSpec::new(DIALOG_SIZE, &options_title);
        p.frame_background(&spec);

        // 2. Push occluders for Options content: any subdialog, confirmation, or external window on top.
        let occluders = self.dialog_occluders(sub_bounds, confirm_bounds, external_occluders);
        for &b in &occluders {
            base.canvas.push_occluder(b);
        }

        // 3. Draw Options foreground (title, close button) and content.
        p.frame_foreground(
            &spec,
            Some(ButtonState {
                armed: close_armed,
                pressed: close_pressed,
                enabled: !modal,
                default: false,
            }),
        );
        let tabs = self.tab_rects(&|t| p.text_width(t, false));
        let page = page_rect();
        let active = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
        p.raised_open_top(page, tabs[active]);
        for (i, rect) in tabs.iter().enumerate() {
            if i != active {
                let (armed, pressed) = state(Target::Tab(i));
                p.tab(
                    *rect,
                    &self.tab_label(Tab::ALL[i]),
                    false,
                    ButtonState::active(!modal, armed, pressed),
                );
            }
        }
        let (armed, pressed) = state(Target::Tab(active));
        p.tab(
            tabs[active],
            &self.tab_label(self.tab),
            true,
            ButtonState::active(!modal, armed, pressed),
        );
        if focused(Target::Tab(active)) {
            p.focus_box(tabs[active]);
        }
        if let Some(panel) = self.tab.panel() {
            let list = layout_widgets(&self.res, panel, page, &self.draft, &self.applied);
            for (i, widget) in list.iter().enumerate() {
                let (armed, pressed) = state(Target::Control(i));
                let focus = focused(Target::Control(i));
                let logical = !modal && self.focus == Some(Target::Control(i));
                self.draw_widget(
                    &p,
                    widget,
                    &env,
                    ((armed, pressed), (focus, logical)),
                    (parent_focused, time_val),
                );
            }
        }
        let labels = ["#GameUI_OK", "#GameUI_Cancel", "#GameUI_Apply"];
        let targets = [Target::Ok, Target::Cancel, Target::Apply];
        for ((rect, label), target) in button_rects().into_iter().zip(labels).zip(targets) {
            let (armed, pressed) = state(target);
            let enabled = target != Target::Apply || self.changed();
            p.button(
                rect,
                &loc.text(label),
                ButtonState {
                    armed,
                    pressed,
                    enabled,
                    // No default-button border (owner request, 2026-10-17): focus is the
                    // dashed box alone.
                    default: false,
                },
            );
            if enabled && focused(target) {
                p.focus_box(rect);
            }
        }
        if self.sub.is_none() {
            self.draw_combo(&p);
        }

        // Pop Options content occluders before drawing subdialog on top.
        for _ in 0..occluders.len() {
            base.canvas.pop_occluder();
        }

        // 4. Draw subdialog on top (if any).
        if let Some(sub) = self.sub.or(self.closing_sub) {
            let size = self.sub_frame_size(sub, |s| base.text_width(s, false));
            let sp = Painter {
                canvas: base.canvas,
                fonts: base.fonts,
                theme: &sub_theme,
                origin: self.sub_placement.place(&sub_frame(canvas, size)).origin,
                scale,
            };
            let sub_modal = self.confirm.is_some() || self.closing_confirm.is_some();
            let is_closing = self.sub.is_none();
            let sub_parent_focused = self.parent_focused && self.confirm.is_none() && !is_closing;
            let sub_close_armed =
                sub_parent_focused && !sub_modal && self.armed == Some(Target::Close);
            let sub_close_pressed = !sub_modal
                && !is_closing
                && (self.pressed == Some(Target::Close) || self.escape_held);
            let old_alpha = base.canvas.set_alpha_multiplier(self.sub_alpha);

            // Subdialog frame background (unclipped).
            let sub_title = loc.text(sub.title());
            let sub_spec = FrameSpec::new(size, &sub_title);
            sp.frame_background(&sub_spec);

            // Push occluders for subdialog content (if confirm box or external window is open).
            let sub_occluders = self.dialog_occluders(None, confirm_bounds, external_occluders);
            for &bounds in &sub_occluders {
                base.canvas.push_occluder(bounds);
            }

            sp.frame_foreground(
                &sub_spec,
                Some(ButtonState {
                    armed: sub_close_armed,
                    pressed: sub_close_pressed,
                    enabled: !sub_modal && !is_closing,
                    default: false,
                }),
            );

            if sub.is_credits() {
                self.draw_credits_subdialog(&sp, sub, size, sub_modal || is_closing);
            } else {
                let area = Rect::new(0., 0., size.x, size.y);
                let list = layout_widgets(&self.res, sub.panel(), area, &self.draft, &self.applied);
                for (i, widget) in list.iter().enumerate() {
                    let target = Some(Target::Control(i));
                    let armed = sub_parent_focused && !sub_modal && self.armed == target;
                    let pressed = !sub_modal
                        && !is_closing
                        && (self.pressed == target || self.key_press.pressed() == target);
                    let logical = !sub_modal && !is_closing && self.focus == target;
                    let focus = logical && self.focus_visible;
                    self.draw_widget(
                        &sp,
                        widget,
                        &env,
                        ((armed, pressed), (focus, logical)),
                        (sub_parent_focused, time_val),
                    );
                }
                self.draw_combo(&sp);
            }

            for _ in 0..sub_occluders.len() {
                base.canvas.pop_occluder();
            }

            base.canvas.set_alpha_multiplier(old_alpha);
        }

        // 5. Draw confirmation box on top (if any).
        let confirm_to_draw = self.confirm.as_ref().or(self.closing_confirm.as_ref());
        if let Some(confirm) = confirm_to_draw {
            let layout = confirm.layout(|t| p.text_width(t, false));
            let cp = Painter {
                canvas: base.canvas,
                fonts: base.fonts,
                theme: &confirm_theme,
                origin: self
                    .confirm_placement
                    .place(&confirm_frame(canvas, layout.size))
                    .origin,
                scale,
            };
            let old_alpha = base.canvas.set_alpha_multiplier(self.confirm_alpha);
            confirm.draw(&cp, &layout);
            base.canvas.set_alpha_multiplier(old_alpha);
        }
        self.draw_tooltip(base, canvas, scale);
    }

    /// The open combo list, over the layer (page or sub-dialog) that opened it.
    fn draw_combo(&self, p: &Painter) {
        if let Some(OpenCombo { items, popup, .. }) = &self.combo {
            p.menu_list(
                popup.list,
                items,
                popup.first,
                popup.highlight,
                popup.bar_pressed(),
            );
        }
    }

    /// Draws a control; `focus` is (focus drawn, focus held): the focus look appears only after
    /// keyboard navigation, the key list dims its selection without focus.
    fn draw_widget(
        &self,
        p: &Painter,
        w: &Widget,
        env: &Env,
        ((armed, pressed), (focused, logical)): ((bool, bool), (bool, bool)),
        (parent_focused, time): (bool, f64),
    ) {
        self.draw_control(
            p,
            w,
            env,
            ((armed, pressed), (focused, logical)),
            (parent_focused, time),
        );
        self.mark_development(p, w);
        if !focused || !w.enabled {
            return;
        }
        let pad = widgets::FOCUS_PAD;
        match w.control.kind {
            ControlKind::Button | ControlKind::UrlButton => p.focus_box(w.rect),
            // The whole control, box or circle and label, evenly padded.
            ControlKind::CheckButton | ControlKind::RadioButton => {
                let label = self.res.localization.localize(&w.control.label);
                p.focus_ring(widgets::toggle_content(w.rect, p.text_width(&label, false)).pad(pad));
            }
            ControlKind::Slider => p.focus_ring(widgets::slider_content(w.rect).pad(pad)),
            ControlKind::TextEntry | ControlKind::ListPanel => p.focus_ring(w.rect.pad(pad)),
            _ => {}
        }
    }

    fn draw_control(
        &self,
        p: &Painter,
        w: &Widget,
        env: &Env,
        ((armed, pressed), (focused, logical)): ((bool, bool), (bool, bool)),
        (parent_focused, time): (bool, f64),
    ) {
        let loc = &self.res.localization;
        let c = w.control;
        let label = loc.localize(&c.label);
        let config = &self.draft.config;
        match (&c.kind, w.binding) {
            (ControlKind::Label, Some(Binding::ValueLabel(value))) => {
                let align = match c.align {
                    LayoutAlign::Center => Align::Center,
                    LayoutAlign::East => Align::Right,
                    _ => Align::Left,
                };
                let text = value.label(config, loc);
                p.label(&text, w.rect, align, w.enabled);
            }
            (ControlKind::Label, _) => {
                let multiline = c.align == LayoutAlign::NorthWest || c.tall >= FONT_TALL * 2.;
                if multiline {
                    let color = if w.enabled {
                        p.theme.text
                    } else {
                        p.theme.text_disabled
                    };
                    p.wrapped(&label, w.rect, color);
                } else {
                    let align = match c.align {
                        LayoutAlign::Center => Align::Center,
                        LayoutAlign::East => Align::Right,
                        _ => Align::Left,
                    };
                    p.label(&label, w.rect, align, w.enabled);
                }
            }
            (ControlKind::CheckButton, Some(Binding::Toggle(toggle))) => p.check_box(
                w.rect,
                &label,
                toggle.get(config),
                ButtonState::active(w.enabled, armed, pressed),
            ),
            (ControlKind::CheckButton, _) => {
                p.check_box(w.rect, &label, false, ButtonState::new(false))
            }
            (ControlKind::RadioButton, binding) => {
                let selected = binding == Some(Binding::Skill(config.gameplay.skill));
                p.radio(
                    w.rect,
                    &label,
                    selected,
                    ButtonState::active(w.enabled, armed, pressed),
                )
            }
            (ControlKind::ComboBox, Some(Binding::Choice(choice))) => {
                let open = self
                    .combo
                    .as_ref()
                    .is_some_and(|o| o.popup.anchor == w.rect);
                p.combo(
                    w.rect,
                    &choice.text(env, &self.draft),
                    w.enabled,
                    armed || open,
                    focused,
                )
            }
            (ControlKind::ComboBox, _) => p.combo(w.rect, "", false, false, false),
            (ControlKind::Slider, binding) => {
                let fraction = match binding {
                    Some(Binding::Slider(value)) => {
                        value.slider_range(c.range).fraction(value.get(config))
                    }
                    _ => 0.,
                };
                p.slider(
                    w.rect,
                    fraction,
                    &loc.localize(&c.left_text),
                    &loc.localize(&c.right_text),
                    w.enabled,
                )
            }
            (ControlKind::TextEntry, binding) => {
                let (text, child_focused) = match (&self.editing, binding) {
                    (Some(editing), Some(Binding::Entry(value))) if editing.value == value => {
                        (editing.text.clone(), true)
                    }
                    (_, Some(Binding::Entry(value))) => {
                        (widgets::format_value(value.get(config)), false)
                    }
                    _ => (label.clone(), false),
                };
                let model = widgets::TextInputModel::new(parent_focused, child_focused);
                p.text_entry_model(w.rect, &text, w.enabled, &model, time);
            }
            (ControlKind::Button, _) => p.button(
                w.rect,
                &label,
                ButtonState {
                    armed,
                    pressed,
                    enabled: w.enabled,
                    default: false,
                },
            ),
            (ControlKind::UrlButton, _) => p.url(
                w.rect,
                &label,
                ButtonState::active(w.enabled, armed, pressed),
            ),
            (ControlKind::Divider, _) => p.divider(w.rect),
            (ControlKind::ImagePanel, _) => {
                let image = c.image.to_ascii_lowercase();
                let texture = if image.contains("bink") {
                    self.bink_texture.as_ref()
                } else if image.contains("gamma") {
                    self.gamma_texture.as_ref()
                } else {
                    None
                };
                if let Some(texture) = texture {
                    p.image(w.rect, texture);
                }
            }
            (ControlKind::ListPanel, Some(Binding::KeyList)) => {
                self.draw_key_list(p, w.rect, logical)
            }
            _ => {}
        }
    }

    /// `focused`: the list holds keyboard focus; without it the selected row takes the scheme's
    /// out-of-focus selection (SDK `SectionedListPanel`).
    fn draw_key_list(&self, p: &Painter, rect: Rect, focused: bool) {
        let loc = &self.res.localization;
        let theme = p.theme;
        p.fill(rect, theme.list_bg);
        p.sunken(rect);
        let rows = &self.res.key_rows;
        let scroll = key_list_scrollable(rect, rows.len());
        let inner_w = rect.w - 2. - if scroll { SCROLLBAR_WIDTH } else { 0. };
        let action_w = (inner_w * 0.6).round();
        let header = Rect::new(rect.x + 1., rect.y + 1., inner_w, KEY_HEADER - 1.);
        p.raised(Rect::new(header.x, header.y, action_w, header.h));
        p.raised(Rect::new(
            header.x + action_w,
            header.y,
            inner_w - action_w,
            header.h,
        ));
        let cell = |x: f32, y: f32, w: f32| Rect::new(x + 6., y, w - 12., MENU_ROW);
        p.text_in(
            &loc.text("#GameUI_Action"),
            cell(header.x, header.y, action_w),
            Align::Left,
            theme.text,
            false,
        );
        p.text_in(
            &loc.text("#GameUI_KeyButton"),
            cell(header.x + action_w, header.y, inner_w - action_w),
            Align::Left,
            theme.text,
            false,
        );
        let visible = key_rows_visible(rect);
        let top = rect.y + KEY_HEADER;
        for (line, (index, row)) in rows
            .iter()
            .enumerate()
            .skip(self.keys.first)
            .take(visible)
            .enumerate()
        {
            let y = top + line as f32 * MENU_ROW;
            match row {
                KeyRow::Section(title) => {
                    p.text_in(
                        &loc.localize(title),
                        cell(header.x, y, inner_w),
                        Align::Left,
                        theme.text,
                        true,
                    );
                    // SDK `SectionedListPanelHeader`: a one-pixel divider two pixels above the
                    // bottom of the header row, inset by one pixel on the left and two on the
                    // right, separating action groups.
                    p.fill(
                        Rect::new(header.x + 1., y + MENU_ROW - 2., inner_w - 3., 1.),
                        theme.list_divider,
                    );
                }
                KeyRow::Action { command, label } => {
                    // While capturing, only the key cell is highlighted (the retail list edits
                    // the key column in place); the action name keeps the normal row look.
                    let capturing = self.keys.capture == Some(index);
                    let selected = self.keys.selected == Some(index) && !capturing;
                    if capturing {
                        p.fill(
                            Rect::new(header.x + action_w, y, inner_w - action_w, MENU_ROW),
                            theme.selection_bg,
                        );
                    }
                    let color = if capturing {
                        // The action stays identifiable with the scheme's dimmer selection.
                        let fill = theme.selection_bg_dim;
                        p.fill(
                            Rect::new(header.x, y, action_w, MENU_ROW),
                            source_ui::canvas::Color {
                                a: fill.a * CAPTURE_ACTION_FADE,
                                ..fill
                            },
                        );
                        theme.text
                    } else if selected {
                        let (fill, text) = if focused {
                            (theme.selection_bg, theme.selection_text)
                        } else {
                            (theme.selection_bg_dim, theme.selection_text_dim)
                        };
                        p.fill(Rect::new(header.x, y, inner_w, MENU_ROW), fill);
                        text
                    } else {
                        theme.text
                    };
                    p.text_in(
                        &loc.localize(label),
                        cell(header.x, y, action_w),
                        Align::Left,
                        color,
                        false,
                    );
                    let key = if capturing {
                        String::new()
                    } else {
                        self.draft
                            .config
                            .keys
                            .get(command)
                            .map(|k| keys::display_name(k))
                            .unwrap_or_default()
                    };
                    p.text_in(
                        &key,
                        cell(header.x + action_w, y, inner_w - action_w),
                        Align::Left,
                        color,
                        false,
                    );
                }
            }
        }
        if scroll {
            p.scroll_bar(
                key_bar_rect(rect),
                self.keys.first,
                visible,
                rows.len(),
                self.key_bar.pressed(),
            );
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
