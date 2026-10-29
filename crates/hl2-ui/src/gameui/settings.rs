//! Binds options-page controls (by their `.res` field names) to typed [`Config`] fields.
//!
//! Everything here is pure: given a draft configuration it reads and writes values, lists combo
//! choices and decides dynamic enabled states. The dialog in [`super::options`] only routes input
//! and draws. Retail ranges come from the installed `GameUI.dll` slider constants (see
//! `docs/gameui.md` section 3).

#[cfg(test)]
mod motion_blur_tests;
pub mod support;

use super::{
    display::{self, AspectRatio, DisplayModes, Resolution},
    localize::Localization,
};
use crate::config::{
    AdvancedVideoSettings, AimAssist, Antialiasing, Captions, Config, DisplayMode, Filtering,
    HdrMode, HudAspect, Listed, ModelDetail, ShaderDetail, ShadowDetail, SoundQuality, Speakers,
    TextureDetail, WaterDetail, WeaponSelectionStyle,
};
use source_ui::widgets::SliderRange;

/// The options being edited: the draft configuration plus the Video page's aspect filter, which
/// is page state rather than a stored setting (retail derives it from the resolution), except
/// for the flexible choice, stored as [`crate::config::VideoSettings::flexible_aspect`].
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub config: Config,
    pub aspect: AspectRatio,
}

impl Draft {
    pub fn new(config: Config) -> Self {
        let aspect = if config.video.flexible_aspect {
            AspectRatio::Flexible
        } else {
            AspectRatio::of(config.video.width, config.video.height)
        };
        Self { config, aspect }
    }
}

/// Read-only data the choice lists depend on.
pub struct Env<'a> {
    pub localization: &'a Localization,
    pub languages: &'a [String],
    pub modes: &'a DisplayModes,
    pub recommendation: Option<&'a crate::gameui::recommendation::SystemRecommendation>,
}

impl<'a> Env<'a> {
    pub fn new(
        localization: &'a Localization,
        languages: &'a [String],
        modes: &'a DisplayModes,
    ) -> Self {
        Self {
            localization,
            languages,
            modes,
            recommendation: None,
        }
    }

    pub fn with_recommendation(
        mut self,
        recommendation: Option<&'a crate::gameui::recommendation::SystemRecommendation>,
    ) -> Self {
        self.recommendation = recommendation;
        self
    }

    /// Resolutions offered for the draft's display mode (windowed ones must fit the desktop).
    pub fn resolutions(&self, draft: &Draft) -> Vec<Resolution> {
        offered_resolutions(self.modes, draft)
    }
}

/// Resolutions `modes` offers for the draft's display mode (windowed ones must fit the desktop).
pub fn offered_resolutions(modes: &DisplayModes, draft: &Draft) -> Vec<Resolution> {
    modes.choices(draft.config.video.display_mode.effective() == DisplayMode::Windowed)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    ReverseMouse,
    MouseFilter,
    RawInput,
    Acceleration,
    MuteLoseFocus,
    FastSwitch,
    DeveloperConsole,
}

impl Toggle {
    pub fn get(self, c: &Config) -> bool {
        match self {
            Self::ReverseMouse => c.mouse.invert,
            Self::MouseFilter => c.mouse.filter,
            Self::RawInput => c.mouse.raw_input,
            Self::Acceleration => c.mouse.acceleration,
            Self::MuteLoseFocus => c.audio.mute_losefocus,
            Self::FastSwitch => c.keyboard.fast_switch,
            Self::DeveloperConsole => c.keyboard.developer_console,
        }
    }

    pub fn set(self, c: &mut Config, value: bool) {
        *match self {
            Self::ReverseMouse => &mut c.mouse.invert,
            Self::MouseFilter => &mut c.mouse.filter,
            Self::RawInput => &mut c.mouse.raw_input,
            Self::Acceleration => &mut c.mouse.acceleration,
            Self::MuteLoseFocus => &mut c.audio.mute_losefocus,
            Self::FastSwitch => &mut c.keyboard.fast_switch,
            Self::DeveloperConsole => &mut c.keyboard.developer_console,
        } = value;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value {
    Volume,
    MusicVolume,
    Sensitivity,
    AccelExponent,
    AimAssistScale,
    Gamma,
    /// `fov_desired` (Video > Advanced).
    Fov,
    /// Vertical field of view (Video > Advanced); the slider's low end is automatic.
    VerticalFov,
    /// `viewmodel_fov` (Video > Advanced).
    ViewmodelFov,
    /// `sc_sensitivity_scale` (Gamepad).
    JoystickScale,
    /// `sc_autoaim_scale` (Gamepad).
    GamepadAimAssistScale,
}

impl Value {
    /// Slider range (retail `GameUI.dll` constants). A slider whose layout gives `minvalue` /
    /// `maxvalue` uses those instead ([`Self::slider_range`]); the field of view has no constant
    /// and falls back to the installed `FovSlider` range.
    pub fn range(self) -> SliderRange {
        match self {
            Self::Volume | Self::MusicVolume => SliderRange::new(0., 1.),
            Self::Sensitivity => SliderRange::new(0.1, 6.),
            Self::AccelExponent => SliderRange::new(1., 1.4),
            Self::AimAssistScale | Self::JoystickScale | Self::GamepadAimAssistScale => {
                SliderRange::new(0.5, 2.)
            }
            Self::Gamma => SliderRange::new(1.6, 2.6),
            Self::Fov => SliderRange::new(75., 110.),
            // The low end of the vertical slider is automatic (0), then the explicit range.
            Self::VerticalFov => SliderRange::new(0., crate::tuning::vertical_fov_range().1),
            Self::ViewmodelFov => {
                let (min, max) = crate::tuning::viewmodel_fov_range();
                SliderRange::new(min, max)
            }
        }
    }

    /// The text of a value label: the number, or the installed "Auto" for an automatic vertical
    /// field of view.
    pub fn label(self, c: &Config, loc: &Localization) -> String {
        let value = self.get(c);
        if self == Self::VerticalFov && value == 0. {
            loc.text("#Spec_Auto")
        } else {
            format!("{value:.0}")
        }
    }

    /// The range of a slider bound to this value: the layout's own range when it gives one.
    pub fn slider_range(self, layout: Option<(f32, f32)>) -> SliderRange {
        layout.map_or_else(|| self.range(), |(min, max)| SliderRange::new(min, max))
    }

    pub fn get(self, c: &Config) -> f32 {
        match self {
            Self::Volume => c.audio.volume,
            Self::MusicVolume => c.audio.music_volume,
            Self::Sensitivity => c.mouse.sensitivity,
            Self::AccelExponent => c.mouse.accel_exponent,
            Self::AimAssistScale => c.mouse.aim_assist_scale,
            Self::Gamma => c.video.gamma,
            Self::Fov => c.video.fov,
            Self::VerticalFov => c.video.vertical_fov,
            Self::ViewmodelFov => c.video.viewmodel_fov,
            Self::JoystickScale => c.gamepad.sensitivity_scale,
            Self::GamepadAimAssistScale => c.gamepad.aim_assist_scale,
        }
    }

    /// Stores a value clamped to the slider range. Typed sensitivity entries may exceed the
    /// slider (like the `sensitivity` cvar), up to the console limit of 20.
    pub fn set(self, c: &mut Config, value: f32) {
        if !value.is_finite() {
            return;
        }
        let value = match self {
            Self::Sensitivity => value.clamp(0.1, 20.),
            // Below the smallest explicit field of view the slider means automatic.
            Self::VerticalFov => {
                let (min, max) = crate::tuning::vertical_fov_range();
                if value < min * 0.5 {
                    0.
                } else {
                    value.clamp(min, max)
                }
            }
            _ => self.range().clamp(value),
        };
        *match self {
            Self::Volume => &mut c.audio.volume,
            Self::MusicVolume => &mut c.audio.music_volume,
            Self::Sensitivity => &mut c.mouse.sensitivity,
            Self::AccelExponent => &mut c.mouse.accel_exponent,
            Self::AimAssistScale => &mut c.mouse.aim_assist_scale,
            Self::Gamma => &mut c.video.gamma,
            Self::Fov => &mut c.video.fov,
            Self::VerticalFov => &mut c.video.vertical_fov,
            Self::ViewmodelFov => &mut c.video.viewmodel_fov,
            Self::JoystickScale => &mut c.gamepad.sensitivity_scale,
            Self::GamepadAimAssistScale => &mut c.gamepad.aim_assist_scale,
        } = value;
    }
}

/// The combos of Video > Advanced, over [`AdvancedVideoSettings`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdvancedChoice {
    ModelDetail,
    TextureDetail,
    ShaderDetail,
    WaterDetail,
    ShadowDetail,
    Antialiasing,
    Filtering,
    ColorCorrection,
    VSync,
    MotionBlur,
    Multicore,
    ClassicEffects,
    D3D9Ex,
}

/// Off/on combos list "Disabled" first.
const FLAG_TOKENS: [&str; 2] = ["#gameui_disabled", "#gameui_enabled"];
const CLASSIC_EFFECTS_TOKENS: [&str; 2] =
    ["#GameUI_ClassicEffects_Off", "#GameUI_ClassicEffects_On"];

fn listed_tokens<T: Listed>() -> Vec<&'static str> {
    T::ALL.iter().map(|v| v.token()).collect()
}

fn listed_index<T: Listed>(value: T) -> usize {
    T::ALL.iter().position(|v| *v == value).unwrap_or(0)
}

fn listed_pick<T: Listed>(index: usize, value: &mut T) {
    if let Some(v) = T::ALL.get(index) {
        *value = *v;
    }
}

impl AdvancedChoice {
    fn flag(self, a: &AdvancedVideoSettings) -> Option<bool> {
        Some(match self {
            Self::ColorCorrection => a.color_correction,
            Self::VSync => a.vsync,
            Self::Multicore => a.multicore,
            Self::ClassicEffects => a.classic_effects,
            Self::D3D9Ex => a.d3d9ex,
            _ => return None,
        })
    }

    fn flag_mut(self, a: &mut AdvancedVideoSettings) -> Option<&mut bool> {
        Some(match self {
            Self::ColorCorrection => &mut a.color_correction,
            Self::VSync => &mut a.vsync,
            Self::Multicore => &mut a.multicore,
            Self::ClassicEffects => &mut a.classic_effects,
            Self::D3D9Ex => &mut a.d3d9ex,
            _ => return None,
        })
    }

    /// Item tokens in combo order (the retail dialog's lists).
    pub fn tokens(self) -> Vec<&'static str> {
        self.tokens_with_rec(None)
    }

    /// Item tokens in combo order, gated by hardware capabilities if a system recommendation is present.
    pub fn tokens_with_rec(
        self,
        rec: Option<&crate::gameui::recommendation::SystemRecommendation>,
    ) -> Vec<&'static str> {
        match self {
            Self::ModelDetail => listed_tokens::<ModelDetail>(),
            Self::TextureDetail => listed_tokens::<TextureDetail>(),
            Self::ShaderDetail => listed_tokens::<ShaderDetail>(),
            Self::WaterDetail => listed_tokens::<WaterDetail>(),
            Self::ShadowDetail => listed_tokens::<ShadowDetail>(),
            Self::Antialiasing => {
                if let Some(rec) = rec {
                    rec.gpu
                        .supported_antialiasing()
                        .into_iter()
                        .map(|aa| aa.token())
                        .collect()
                } else {
                    listed_tokens::<Antialiasing>()
                }
            }
            Self::Filtering => listed_tokens::<Filtering>(),
            Self::ClassicEffects => CLASSIC_EFFECTS_TOKENS.to_vec(),
            Self::MotionBlur => vec![
                FLAG_TOKENS[0],
                FLAG_TOKENS[1],
                "#GameUI_SettingAdvanced",
                super::localize::placeholders::MOTION_BLUR_ULTRA,
            ],
            _ => FLAG_TOKENS.to_vec(),
        }
    }

    pub fn selected(self, a: &AdvancedVideoSettings) -> usize {
        match self {
            Self::ModelDetail => listed_index(a.model_detail),
            Self::TextureDetail => listed_index(a.texture_detail),
            Self::ShaderDetail => listed_index(a.shader_detail),
            Self::WaterDetail => listed_index(a.water_detail),
            Self::ShadowDetail => listed_index(a.shadow_detail),
            Self::Antialiasing => listed_index(a.antialiasing),
            Self::Filtering => listed_index(a.filtering),
            Self::MotionBlur => a.motion_blur_mode() as usize,
            flag => usize::from(flag.flag(a).unwrap_or(false)),
        }
    }

    pub fn select(self, a: &mut AdvancedVideoSettings, index: usize) {
        match self {
            Self::ModelDetail => listed_pick(index, &mut a.model_detail),
            Self::TextureDetail => listed_pick(index, &mut a.texture_detail),
            Self::ShaderDetail => listed_pick(index, &mut a.shader_detail),
            Self::WaterDetail => listed_pick(index, &mut a.water_detail),
            Self::ShadowDetail => listed_pick(index, &mut a.shadow_detail),
            Self::Antialiasing => listed_pick(index, &mut a.antialiasing),
            Self::Filtering => listed_pick(index, &mut a.filtering),
            Self::MotionBlur => {
                if index < 4 {
                    a.motion_blur = index != 0;
                    a.motion_blur_viewmodel = index >= 2;
                    a.motion_blur_world = index == 3;
                }
            }
            flag => {
                if let (Some(value), true) = (flag.flag_mut(a), index < 2) {
                    *value = index == 1;
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// A Video > Advanced combo.
    Advanced(AdvancedChoice),
    AimAssist,
    /// Gamepad Aim-Assist (`sc_autoaim_mode`).
    GamepadAimAssist,
    /// Gamepad weapon selection style.
    WeaponSelection,
    Speakers,
    Quality,
    Language,
    Captions,
    Resolution,
    Aspect,
    DisplayMode,
    HudAspect,
    Hdr,
}

impl Choice {
    /// Localized item labels, in combo order.
    pub fn items(self, env: &Env, draft: &Draft) -> Vec<String> {
        let loc = env.localization;
        let tokens = |tokens: &mut dyn Iterator<Item = &'static str>| -> Vec<String> {
            tokens.map(|t| loc.text(t)).collect()
        };
        match self {
            Self::Advanced(advanced) => {
                let rec_idx = env
                    .recommendation
                    .map(|r| r.recommended_advanced_index(advanced));
                advanced
                    .tokens_with_rec(env.recommendation)
                    .into_iter()
                    .enumerate()
                    .map(|(i, t)| {
                        let text = loc.text(t);
                        if rec_idx == Some(i) {
                            format!("{text} *")
                        } else {
                            text
                        }
                    })
                    .collect()
            }
            Self::GamepadAimAssist => tokens(&mut AimAssist::ALL.iter().map(|v| v.token())),
            Self::WeaponSelection => {
                tokens(&mut listed_tokens::<WeaponSelectionStyle>().into_iter())
            }
            Self::AimAssist => tokens(&mut AimAssist::ALL.iter().map(|v| v.token())),
            Self::Speakers => tokens(&mut Speakers::ALL.iter().map(|v| v.token())),
            Self::Quality => tokens(&mut SoundQuality::ALL.iter().map(|v| v.token())),
            Self::Captions => tokens(&mut Captions::ALL.iter().map(|v| v.token())),
            Self::Aspect => tokens(&mut AspectRatio::ALL.iter().map(|v| v.token())),
            Self::DisplayMode => tokens(&mut DisplayMode::choices().iter().map(|v| v.token())),
            Self::HudAspect => tokens(&mut HudAspect::ALL.iter().map(|v| v.token())),
            Self::Hdr => {
                let rec_idx = env.recommendation.map(|r| r.recommended_hdr_index());
                HdrMode::ALL
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let text = loc.text(v.token());
                        if rec_idx == Some(i) {
                            format!("{text} *")
                        } else {
                            text
                        }
                    })
                    .collect()
            }
            Self::Language => env
                .languages
                .iter()
                .map(|l| loc.text(&language_token(l)))
                .collect(),
            Self::Resolution => {
                if draft.aspect == AspectRatio::Flexible {
                    let current = env.modes.current.unwrap_or_else(|| {
                        Resolution::new(draft.config.video.width, draft.config.video.height)
                    });
                    vec![current.without_stripes_for_flexible().label()]
                } else {
                    display::resolution_choices(&env.resolutions(draft), draft.aspect)
                        .into_iter()
                        .map(Resolution::label)
                        .collect()
                }
            }
        }
    }

    pub fn selected(self, env: &Env, draft: &Draft) -> Option<usize> {
        let c = &draft.config;
        fn find<T: PartialEq>(list: &[T], value: T) -> Option<usize> {
            list.iter().position(|v| *v == value)
        }
        match self {
            Self::Advanced(AdvancedChoice::Antialiasing) => {
                if let Some(rec) = env.recommendation {
                    let supported = rec.gpu.supported_antialiasing();
                    let current = c.video.advanced.antialiasing;
                    let clamped = if current.samples() > rec.gpu.max_msaa_samples() {
                        rec.gpu.max_antialiasing()
                    } else {
                        current
                    };
                    supported.iter().position(|&aa| aa == clamped).or(Some(0))
                } else {
                    Some(AdvancedChoice::Antialiasing.selected(&c.video.advanced))
                }
            }
            Self::Advanced(advanced) => Some(advanced.selected(&c.video.advanced)),
            Self::GamepadAimAssist => find(&AimAssist::ALL, c.gamepad.aim_assist),
            Self::WeaponSelection => Some(listed_index(c.gamepad.weapon_selection)),
            Self::AimAssist => find(&AimAssist::ALL, c.mouse.aim_assist),
            Self::Speakers => find(&Speakers::ALL, c.audio.speakers),
            Self::Quality => find(&SoundQuality::ALL, c.audio.quality),
            Self::Captions => find(&Captions::ALL, c.audio.captions),
            Self::Aspect => find(&AspectRatio::ALL, draft.aspect),
            Self::DisplayMode => DisplayMode::choices()
                .iter()
                .position(|m| *m == c.video.display_mode.effective()),
            Self::HudAspect => find(&HudAspect::ALL, c.video.hud_aspect),
            Self::Hdr => find(&HdrMode::ALL, c.video.hdr),
            Self::Language => env
                .languages
                .iter()
                .position(|l| l.eq_ignore_ascii_case(&c.audio.language)),
            Self::Resolution => {
                if draft.aspect == AspectRatio::Flexible {
                    Some(0)
                } else {
                    display::resolution_choices(&env.resolutions(draft), draft.aspect)
                        .iter()
                        .position(|r| *r == Resolution::new(c.video.width, c.video.height))
                }
            }
        }
    }

    /// The text shown in the closed combo: the selected item, or the stored value when it is
    /// not among the choices (an unlisted resolution or an uninstalled language).
    pub fn text(self, env: &Env, draft: &Draft) -> String {
        let items = self.items(env, draft);
        match self.selected(env, draft) {
            Some(i) => items[i].clone(),
            None => match self {
                Self::Resolution => {
                    if draft.aspect == AspectRatio::Flexible {
                        items.first().cloned().unwrap_or_default()
                    } else {
                        Resolution::new(draft.config.video.width, draft.config.video.height).label()
                    }
                }
                Self::Language => env
                    .localization
                    .text(&language_token(&draft.config.audio.language)),
                _ => items.first().cloned().unwrap_or_default(),
            },
        }
    }

    pub fn select(self, env: &Env, draft: &mut Draft, index: usize) {
        let resolutions = env.resolutions(draft);
        let c = &mut draft.config;
        match self {
            Self::Advanced(AdvancedChoice::Antialiasing) => {
                if let Some(rec) = env.recommendation {
                    let supported = rec.gpu.supported_antialiasing();
                    if let Some(&aa) = supported.get(index) {
                        c.video.advanced.antialiasing = aa;
                    }
                } else {
                    AdvancedChoice::Antialiasing.select(&mut c.video.advanced, index);
                }
            }
            Self::Advanced(advanced) => advanced.select(&mut c.video.advanced, index),
            Self::GamepadAimAssist => {
                if let Some(v) = AimAssist::ALL.get(index) {
                    c.gamepad.aim_assist = *v;
                }
            }
            Self::WeaponSelection => listed_pick(index, &mut c.gamepad.weapon_selection),
            Self::AimAssist => {
                if let Some(v) = AimAssist::ALL.get(index) {
                    c.mouse.aim_assist = *v;
                }
            }
            Self::Speakers => {
                if let Some(v) = Speakers::ALL.get(index) {
                    c.audio.speakers = *v;
                }
            }
            Self::Quality => {
                if let Some(v) = SoundQuality::ALL.get(index) {
                    c.audio.quality = *v;
                }
            }
            Self::Captions => {
                if let Some(v) = Captions::ALL.get(index) {
                    c.audio.captions = *v;
                }
            }
            Self::DisplayMode => {
                if let Some(v) = DisplayMode::choices().get(index) {
                    c.video.display_mode = *v;
                }
                // The resolution list depends on the display mode (windowed resolutions must
                // fit the desktop); keep the resolution when it is still offered.
                let current = Resolution::new(c.video.width, c.video.height);
                let offered = env.resolutions(draft);
                if !offered.contains(&current) {
                    if let Some(r) =
                        display::choose_after_aspect_change(&offered, draft.aspect, current)
                    {
                        draft.config.video.width = r.width;
                        draft.config.video.height = r.height;
                    }
                }
            }
            Self::HudAspect => {
                if let Some(v) = HudAspect::ALL.get(index) {
                    c.video.hud_aspect = *v;
                }
            }
            Self::Hdr => {
                if let Some(v) = HdrMode::ALL.get(index) {
                    c.video.hdr = *v;
                }
            }
            Self::Language => {
                if let Some(v) = env.languages.get(index) {
                    c.audio.language = v.clone();
                }
            }
            Self::Resolution => {
                if draft.aspect == AspectRatio::Flexible {
                    return;
                }
                if let Some(r) = display::resolution_choices(&resolutions, draft.aspect).get(index)
                {
                    c.video.width = r.width;
                    c.video.height = r.height;
                }
            }
            Self::Aspect => {
                let Some(aspect) = AspectRatio::ALL.get(index).copied() else {
                    return;
                };
                if aspect == draft.aspect {
                    return;
                }
                draft.aspect = aspect;
                c.video.flexible_aspect = aspect == AspectRatio::Flexible;
                if aspect == AspectRatio::Flexible {
                    let current = env
                        .modes
                        .current
                        .or(env.modes.desktop)
                        .unwrap_or_else(|| Resolution::new(c.video.width, c.video.height))
                        .without_stripes_for_flexible();
                    c.video.width = current.width;
                    c.video.height = current.height;
                } else {
                    let current = Resolution::new(c.video.width, c.video.height);
                    if let Some(r) =
                        display::choose_after_aspect_change(&resolutions, aspect, current)
                    {
                        c.video.width = r.width;
                        c.video.height = r.height;
                    }
                }
            }
        }
    }
}

/// `#GameUI_Language_<Name>` for an installed voice-pack language (`schinese` ->
/// `Simplified_Chinese`, `latam` -> `Latam_Spanish`, otherwise the capitalized name).
pub fn language_token(language: &str) -> String {
    let name = match language.to_ascii_lowercase().as_str() {
        "schinese" => "Simplified_Chinese".to_owned(),
        "tchinese" => "Traditional_Chinese".to_owned(),
        "latam" => "Latam_Spanish".to_owned(),
        other => {
            let mut chars = other.chars();
            chars
                .next()
                .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        }
    };
    format!("#GameUI_Language_{name}")
}

/// Buttons and links with dialog-level behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    GammaDialog,
    AudioCredits,
    VideoCredits,
    KeyDefaults,
    KeyEdit,
    KeyClear,
    /// "Advanced..." on the Keyboard page.
    KeyAdvanced,
    /// "Advanced..." on the Video page.
    VideoAdvanced,
    /// "Open Steam Input Configuration" on the Gamepad page (placeholder: no Steam overlay).
    SteamInput,
    /// "Test speaker settings" on the Audio page (placeholder in Source: visible 0 in retail layout).
    TestSpeakers,
    /// "HD Content" check button on the Video Advanced page (placeholder in Source: visible 0 in retail layout).
    HDContent,
    /// "Migrate Data From Standalone Episodes" button on the Gameplay page (placeholder in Source: visible 0 in retail layout).
    MigrateData,
    /// OK of a sub-dialog.
    SubOk,
    /// Cancel / Close of a sub-dialog.
    SubCancel,
}

impl Command {
    /// Placeholders are drawn disabled; activating one only logs its label.
    pub fn placeholder(self) -> bool {
        matches!(
            self,
            Self::SteamInput | Self::TestSpeakers | Self::HDContent | Self::MigrateData
        )
    }
}

/// What a control does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binding {
    Toggle(Toggle),
    /// One radio button of the skill group.
    Skill(u8),
    Slider(Value),
    /// The text entry showing (and accepting) a slider's value.
    Entry(Value),
    /// A label showing a slider's value (Video > Advanced `FOVValueLabel`).
    ValueLabel(Value),
    Choice(Choice),
    Command(Command),
    /// The keyboard page's binding list.
    KeyList,
}

/// The panels whose controls come from `.res` layouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Gameplay,
    Keyboard,
    Mouse,
    Audio,
    Video,
    Gamma,
    AudioCredits,
    VideoCredits,
    Gamepad,
    KeyboardAdvanced,
    VideoAdvanced,
}

impl Panel {
    /// Layout file stem in [`super::resources::LAYOUT_FILES`].
    pub fn layout(self) -> &'static str {
        match self {
            Self::Gameplay => "optionssubdifficulty",
            Self::Keyboard => "optionssubkeyboard",
            Self::Mouse => "optionssubmouse",
            Self::Gamepad => "optionssubgamepad",
            Self::Audio => "optionssubaudio",
            Self::Video => "optionssubvideo",
            Self::Gamma => "optionssubvideogammadlg",
            Self::AudioCredits => "optionssubaudiothirdpartydlg",
            Self::VideoCredits => "optionssubvideothirdpartydlg",
            Self::KeyboardAdvanced => "optionssubkeyboardadvanceddlg",
            Self::VideoAdvanced => "optionssubvideoadvanceddlg",
        }
    }
}

/// The binding of a control, by panel and `.res` field name.
pub fn binding(panel: Panel, name: &str) -> Option<Binding> {
    use Binding::*;
    // Every registered identifier fits here; unknown longer names cannot be bindings.
    let mut key = [0u8; 64];
    let normalized = key.get_mut(..name.len())?;
    normalized.copy_from_slice(name.as_bytes());
    normalized.make_ascii_lowercase();
    let name = std::str::from_utf8(normalized).ok()?;
    Some(match (panel, name) {
        (Panel::Gameplay, "skill1radio") => Skill(1),
        (Panel::Gameplay, "skill2radio") => Skill(2),
        (Panel::Gameplay, "skill3radio") => Skill(3),
        (Panel::Gameplay, "migratebutton") => Command(self::Command::MigrateData),
        (Panel::Mouse, "reversemouse") => Toggle(self::Toggle::ReverseMouse),
        (Panel::Mouse, "mousefilter") => Toggle(self::Toggle::MouseFilter),
        (Panel::Mouse, "mouseraw") => Toggle(self::Toggle::RawInput),
        (Panel::Mouse, "mouseaccelerationcheckbox") => Toggle(self::Toggle::Acceleration),
        (Panel::Mouse, "slider") => Slider(Value::Sensitivity),
        (Panel::Mouse, "sensitivitylabel") => Entry(Value::Sensitivity),
        (Panel::Mouse, "mouseaccelerationslider") => Slider(Value::AccelExponent),
        (Panel::Mouse, "mouseaccelerationlabel") => Entry(Value::AccelExponent),
        (Panel::Mouse, "autoaim") => Choice(self::Choice::AimAssist),
        (Panel::Mouse, "autoaimscale") => Slider(Value::AimAssistScale),
        (Panel::Gamepad, "joyscale") => Slider(Value::JoystickScale),
        (Panel::Gamepad, "autoaim") => Choice(self::Choice::GamepadAimAssist),
        (Panel::Gamepad, "autoaimscale") => Slider(Value::GamepadAimAssistScale),
        (Panel::Gamepad, "weaponselection") => Choice(self::Choice::WeaponSelection),
        (Panel::Gamepad, "steaminput") => Command(self::Command::SteamInput),
        (Panel::Audio, "sfxslider") => Slider(Value::Volume),
        (Panel::Audio, "musicslider") => Slider(Value::MusicVolume),
        (Panel::Audio, "speakersetup") => Choice(self::Choice::Speakers),
        (Panel::Audio, "soundquality") => Choice(self::Choice::Quality),
        (Panel::Audio, "audiospokenlanguage") => Choice(self::Choice::Language),
        (Panel::Audio, "closecaptioncheck") => Choice(self::Choice::Captions),
        (Panel::Audio, "snd_mute_losefocus") => Toggle(self::Toggle::MuteLoseFocus),
        (Panel::Audio, "thirdpartysoundcredits") => Command(self::Command::AudioCredits),
        (Panel::Audio, "testspeakers") => Command(self::Command::TestSpeakers),
        (Panel::Video, "resolution") => Choice(self::Choice::Resolution),
        (Panel::Video, "aspectratio") => Choice(self::Choice::Aspect),
        (Panel::Video, "displaymodecombo") => Choice(self::Choice::DisplayMode),
        (Panel::Video, "hudaspectratio") => Choice(self::Choice::HudAspect),
        (Panel::Video, "hdr") => Choice(self::Choice::Hdr),
        (Panel::Video, "gammabutton") => Command(self::Command::GammaDialog),
        (Panel::Video, "advancedbutton") => Command(self::Command::VideoAdvanced),
        (Panel::Video, "thirdpartyvideocredits") => Command(self::Command::VideoCredits),
        (Panel::Keyboard, "listpanel_keybindlist") => KeyList,
        (Panel::Keyboard, "defaults") => Command(self::Command::KeyDefaults),
        (Panel::Keyboard, "keyadvancedbutton") => Command(self::Command::KeyAdvanced),
        (Panel::Keyboard, "changekeybutton") => Command(self::Command::KeyEdit),
        (Panel::Keyboard, "clearkeybutton") => Command(self::Command::KeyClear),
        (Panel::Gamma, "gamma") => Slider(Value::Gamma),
        (Panel::Gamma, "gammaentry") => Entry(Value::Gamma),
        (Panel::KeyboardAdvanced, "fastswitchcheck") => Toggle(self::Toggle::FastSwitch),
        (Panel::KeyboardAdvanced, "consolecheck") => Toggle(self::Toggle::DeveloperConsole),
        (Panel::VideoAdvanced, "fovslider") => Slider(Value::Fov),
        (Panel::VideoAdvanced, "fovvaluelabel") => ValueLabel(Value::Fov),
        (Panel::VideoAdvanced, "verticalfovslider") => Slider(Value::VerticalFov),
        (Panel::VideoAdvanced, "verticalfovvaluelabel") => ValueLabel(Value::VerticalFov),
        (Panel::VideoAdvanced, "viewmodelfovslider") => Slider(Value::ViewmodelFov),
        (Panel::VideoAdvanced, "viewmodelfovvaluelabel") => ValueLabel(Value::ViewmodelFov),
        (Panel::VideoAdvanced, "hdcontentbutton") => Command(self::Command::HDContent),
        (Panel::VideoAdvanced, name) => Choice(self::Choice::Advanced(advanced_choice(name)?)),
        (Panel::Gamma, "okbutton") => Command(self::Command::SubOk),
        (Panel::Gamma, "button1") => Command(self::Command::SubCancel),
        (Panel::AudioCredits | Panel::VideoCredits, "button2") => Command(self::Command::SubCancel),
        _ => return None,
    })
}

/// The Video > Advanced combo with this (lower-case) field name.
fn advanced_choice(name: &str) -> Option<AdvancedChoice> {
    use AdvancedChoice::*;
    Some(match name {
        "modeldetail" => ModelDetail,
        "texturedetail" => TextureDetail,
        "shaderdetail" => ShaderDetail,
        "waterdetail" => WaterDetail,
        "shadowdetail" => ShadowDetail,
        "antialiasingmode" => Antialiasing,
        "filteringmode" => Filtering,
        "colorcorrection" => ColorCorrection,
        "vsync" => VSync,
        "motionblur" => MotionBlur,
        "multicore" => Multicore,
        "classiceffects" => ClassicEffects,
        "d3d9ex" => D3D9Ex,
        _ => return None,
    })
}

/// The binding of a button by its layout `Command`: dialog layouts close themselves with `OK` and
/// `Close` (VGUI `Frame` commands).
pub fn command_binding(command: &str) -> Option<Binding> {
    if command.eq_ignore_ascii_case("ok") || command.eq_ignore_ascii_case("apply") {
        Some(Binding::Command(Command::SubOk))
    } else if command.eq_ignore_ascii_case("close") || command.eq_ignore_ascii_case("cancel") {
        Some(Binding::Command(Command::SubCancel))
    } else {
        None
    }
}

/// Controls the retail layout hides on macOS (`[$OSX]`) that this rewrite still shows, because
/// the renderer supports the feature on every platform: the HDR label and combo.
pub fn force_visible(panel: Panel, name: &str) -> bool {
    panel == Panel::Video
        && (name.eq_ignore_ascii_case("HDR") || name.eq_ignore_ascii_case("HDR Label"))
}

/// Dynamic enabled state on top of the layout's own `enabled` flag.
///
/// * Resolution is disabled when the aspect ratio is [`AspectRatio::Flexible`];
/// * acceleration amount follows its check box;
/// * Aim-Assist Sensitivity is enabled only while aim assist is on (the layout disables it);
/// * the brightness button is enabled in every display mode: retail enables it in full screen
///   only (its hardware gamma ramp needs full screen; the layout ships it disabled), but the
///   ramp is applied by the presentation shader here (owner decision, 2026-10-15);
/// * placeholders are always disabled.
pub fn enabled(
    binding: Option<Binding>,
    layout_enabled: bool,
    draft: &Draft,
    _applied: &Config,
) -> bool {
    match binding {
        Some(Binding::Choice(Choice::Resolution)) => {
            layout_enabled && draft.aspect != AspectRatio::Flexible
        }
        Some(Binding::Slider(Value::AccelExponent) | Binding::Entry(Value::AccelExponent)) => {
            layout_enabled && draft.config.mouse.acceleration
        }
        Some(Binding::Slider(Value::AimAssistScale)) => {
            draft.config.mouse.aim_assist != AimAssist::Off
        }
        Some(Binding::Slider(Value::GamepadAimAssistScale)) => {
            draft.config.gamepad.aim_assist != AimAssist::Off
        }
        Some(Binding::Command(Command::GammaDialog)) => true,
        Some(Binding::Command(command)) if command.placeholder() => false,
        _ => layout_enabled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(loc: &'a Localization, languages: &'a [String], modes: &'a DisplayModes) -> Env<'a> {
        Env {
            localization: loc,
            languages,
            modes,
            recommendation: None,
        }
    }

    #[test]
    fn values_clamp_to_retail_ranges_and_reject_non_finite() {
        let mut c = Config::default();
        Value::AccelExponent.set(&mut c, 3.);
        assert_eq!(c.mouse.accel_exponent, 1.4);
        Value::Gamma.set(&mut c, 0.);
        assert_eq!(c.video.gamma, 1.6);
        Value::Sensitivity.set(&mut c, 12.);
        assert_eq!(
            c.mouse.sensitivity, 12.,
            "typed sensitivity may exceed the slider"
        );
        Value::Volume.set(&mut c, f32::NAN);
        assert_eq!(c.audio.volume, 1.);
        Toggle::MuteLoseFocus.set(&mut c, false);
        assert!(!Toggle::MuteLoseFocus.get(&c));
    }

    #[test]
    fn aspect_change_resets_resolution_without_touching_applied_config() {
        let loc = Localization::default();
        let modes = DisplayModes::new(vec![
            Resolution::new(1920, 1080),
            Resolution::new(1280, 720),
            Resolution::new(1024, 768),
            Resolution::new(1680, 1050),
        ]);
        let langs = ["english".to_owned()];
        let env = env(&loc, &langs, &modes);
        let applied = Config::default();
        let mut draft = Draft::new(applied.clone());
        assert_eq!(draft.aspect, AspectRatio::Wide16x9);
        assert_eq!(
            Choice::Resolution.items(&env, &draft),
            ["1920 x 1080", "1280 x 720"]
        );
        Choice::Aspect.select(&env, &mut draft, 1);
        assert_eq!(draft.aspect, AspectRatio::Normal);
        assert_eq!(
            (draft.config.video.width, draft.config.video.height),
            (1024, 768)
        );
        assert_eq!(Choice::Resolution.selected(&env, &draft), Some(0));
        assert_eq!(applied.video.width, 1280, "nothing applied before OK/Apply");
        Choice::Aspect.select(&env, &mut draft, 3);
        assert_eq!(
            (draft.config.video.width, draft.config.video.height),
            (1680, 1050)
        );
    }

    #[test]
    fn windowed_choices_fit_the_desktop_and_switching_keeps_a_valid_resolution() {
        let loc = Localization::default();
        let langs = ["english".to_owned()];
        let modes = DisplayModes {
            fullscreen: vec![
                Resolution::new(3456, 2234),
                Resolution::new(1920, 1080),
                Resolution::new(1600, 900),
                Resolution::new(1280, 720),
                Resolution::new(1024, 768),
            ],
            desktop: Some(Resolution::new(1728, 1117)),
            current: None,
        };
        let env = env(&loc, &langs, &modes);
        let mut config = Config::default();
        config.video.display_mode = DisplayMode::Fullscreen;
        config.video.width = 1920;
        config.video.height = 1080;
        let mut draft = Draft::new(config);
        assert_eq!(
            Choice::Resolution.items(&env, &draft),
            ["3456 x 2234", "1920 x 1080", "1600 x 900", "1280 x 720"]
        );
        let windowed = DisplayMode::choices()
            .iter()
            .position(|m| *m == DisplayMode::Windowed)
            .unwrap();
        Choice::DisplayMode.select(&env, &mut draft, windowed);
        assert_eq!(
            Choice::Resolution.items(&env, &draft),
            ["1600 x 900", "1280 x 720"]
        );
        assert_eq!(
            (draft.config.video.width, draft.config.video.height),
            (1600, 900),
            "a resolution larger than the desktop is replaced by the largest that fits"
        );
        draft.config.video.width = 1280;
        draft.config.video.height = 720;
        let fullscreen = DisplayMode::choices()
            .iter()
            .position(|m| *m == DisplayMode::Fullscreen)
            .unwrap();
        Choice::DisplayMode.select(&env, &mut draft, fullscreen);
        assert_eq!(
            (draft.config.video.width, draft.config.video.height),
            (1280, 720),
            "a still-offered resolution is kept"
        );
    }

    #[test]
    fn choices_use_tokens_with_key_name_fallback_and_platform_display_modes() {
        let mut loc = Localization::default();
        loc.insert("GameUI_Language_English", "English");
        let langs = ["english".to_owned(), "schinese".to_owned()];
        let no_modes = DisplayModes::default();
        let env = env(&loc, &langs, &no_modes);
        let mut draft = Draft::new(Config::default());
        assert_eq!(
            Choice::Language.items(&env, &draft),
            ["English", "GameUI_Language_Simplified_Chinese"]
        );
        assert_eq!(
            Choice::Captions.items(&env, &draft)[0],
            "GameUI_NoClosedCaptions"
        );
        let modes = Choice::DisplayMode.items(&env, &draft);
        assert_eq!(modes.len(), DisplayMode::choices().len());
        assert!(modes.contains(&"GameUI_NoWindowBorder".to_owned()));
        Choice::Hdr.select(&env, &mut draft, 1);
        assert_eq!(draft.config.video.hdr, HdrMode::Bloom);
        draft.config.audio.language = "french".into();
        assert_eq!(Choice::Language.selected(&env, &draft), None);
        assert_eq!(
            Choice::Language.text(&env, &draft),
            "GameUI_Language_French"
        );
    }

    #[test]
    fn advanced_video_combos_list_retail_items_and_edit_their_settings() {
        let mut a = AdvancedVideoSettings::default();
        assert_eq!(
            AdvancedChoice::TextureDetail.tokens(),
            [
                "#gameui_low",
                "#gameui_medium",
                "#gameui_high",
                "#gameui_ultra"
            ]
        );
        assert_eq!(
            AdvancedChoice::ShaderDetail.tokens(),
            ["#gameui_low", "#gameui_high", "#gameui_ultra"]
        );
        assert_eq!(AdvancedChoice::VSync.tokens(), FLAG_TOKENS);
        assert_eq!(
            AdvancedChoice::ClassicEffects.tokens(),
            CLASSIC_EFFECTS_TOKENS
        );
        assert_eq!(AdvancedChoice::VSync.selected(&a), 1, "enabled");
        AdvancedChoice::VSync.select(&mut a, 0);
        assert!(!a.vsync);
        AdvancedChoice::VSync.select(&mut a, 7);
        assert!(!a.vsync, "out-of-range items are ignored");
        AdvancedChoice::Filtering.select(&mut a, 5);
        assert_eq!(a.filtering, Filtering::Anisotropic16);
        assert_eq!(AdvancedChoice::Filtering.selected(&a), 5);
        assert_eq!(
            binding(Panel::VideoAdvanced, "FilteringMode"),
            Some(Binding::Choice(Choice::Advanced(AdvancedChoice::Filtering)))
        );
        assert_eq!(binding(Panel::VideoAdvanced, "dxlabel"), None);
        assert_eq!(
            Value::Fov.slider_range(Some((75., 110.))),
            SliderRange::new(75., 110.)
        );
        let mut c = Config::default();
        Value::Fov.set(&mut c, 130.);
        // The vertical field of view: automatic at the slider's low end, else an explicit
        // value in the tuned range.
        let (vertical_min, vertical_max) = crate::tuning::vertical_fov_range();
        let mut v = Config::default();
        assert_eq!(Value::VerticalFov.get(&v), 0., "automatic by default");
        Value::VerticalFov.set(&mut v, vertical_min - 1.);
        assert_eq!(
            v.video.vertical_fov, vertical_min,
            "just below the range clamps up"
        );
        Value::VerticalFov.set(&mut v, vertical_min * 0.4);
        assert_eq!(v.video.vertical_fov, 0., "the low end is automatic");
        Value::VerticalFov.set(&mut v, 999.);
        assert_eq!(v.video.vertical_fov, vertical_max);
        let mut loc = Localization::default();
        loc.insert("#Spec_Auto", "Auto");
        Value::VerticalFov.set(&mut v, 0.);
        assert_eq!(Value::VerticalFov.label(&v, &loc), "Auto");
        Value::VerticalFov.set(&mut v, 90.);
        assert_eq!(Value::VerticalFov.label(&v, &loc), "90");
        assert_eq!(Value::Fov.label(&v, &loc), "75");
        // The viewmodel field of view keeps to its tuned range.
        let (view_min, view_max) = crate::tuning::viewmodel_fov_range();
        assert_eq!(Value::ViewmodelFov.get(&v), 54., "retail default");
        Value::ViewmodelFov.set(&mut v, 1.);
        assert_eq!(v.video.viewmodel_fov, view_min);
        Value::ViewmodelFov.set(&mut v, 500.);
        assert_eq!(v.video.viewmodel_fov, view_max);
        assert_eq!(c.video.fov, 110.);
    }

    #[test]
    fn dynamic_enabled_states() {
        let applied = Config::default();
        let mut draft = Draft::new(applied.clone());
        let scale = Some(Binding::Slider(Value::AimAssistScale));
        assert!(!enabled(scale, false, &draft, &applied));
        draft.config.mouse.aim_assist = AimAssist::Basic;
        assert!(enabled(scale, false, &draft, &applied));
        let accel = Some(Binding::Slider(Value::AccelExponent));
        assert!(!enabled(accel, true, &draft, &applied));
        draft.config.mouse.acceleration = true;
        assert!(enabled(accel, true, &draft, &applied));
        assert!(
            !enabled(accel, false, &draft, &applied),
            "layout [$OSX] disable wins"
        );
        let gamma = Some(Binding::Command(Command::GammaDialog));
        assert!(enabled(gamma, false, &draft, &applied));
        let mut windowed = applied.clone();
        windowed.video.display_mode = DisplayMode::Windowed;
        assert!(
            enabled(gamma, false, &draft, &windowed),
            "unlocked in windowed mode"
        );
        for advanced in [Command::VideoAdvanced, Command::KeyAdvanced] {
            assert!(enabled(
                Some(Binding::Command(advanced)),
                true,
                &draft,
                &applied
            ));
        }
        assert_eq!(
            command_binding("OK"),
            Some(Binding::Command(Command::SubOk))
        );
        assert_eq!(
            command_binding("Close"),
            Some(Binding::Command(Command::SubCancel))
        );
        assert_eq!(command_binding("OpenSteamInput"), None);
        assert_eq!(
            binding(Panel::Mouse, "ReverseMouse"),
            Some(Binding::Toggle(Toggle::ReverseMouse))
        );
        assert!(force_visible(Panel::Video, "HDR Label"));
        let resolution = Some(Binding::Choice(Choice::Resolution));
        assert!(enabled(resolution, true, &draft, &applied));
        draft.aspect = AspectRatio::Flexible;
        assert!(!enabled(resolution, true, &draft, &applied));
    }

    #[test]
    fn flexible_aspect_disables_resolution_and_shows_fit_display() {
        let mut loc = Localization::default();
        loc.insert("#GameUI_HUDAspectRatio_FitDisplay", "Fit Display");
        let modes = DisplayModes {
            fullscreen: vec![Resolution::new(1920, 1080), Resolution::new(1280, 720)],
            desktop: Some(Resolution::new(1920, 1080)),
            current: Some(Resolution::new(1600, 900)),
        };
        let langs = ["english".to_owned()];
        let env = env(&loc, &langs, &modes);
        let mut draft = Draft::new(Config::default());
        let res_binding = Some(Binding::Choice(Choice::Resolution));
        assert!(enabled(res_binding, true, &draft, &draft.config));
        assert_eq!(
            Choice::Resolution.items(&env, &draft),
            ["1920 x 1080", "1280 x 720"]
        );

        // Switch to Flexible ("Fit Display")
        let flexible_idx = AspectRatio::ALL
            .iter()
            .position(|a| *a == AspectRatio::Flexible)
            .unwrap();
        assert_eq!(flexible_idx, 0, "Fit Display is in the first place");
        Choice::Aspect.select(&env, &mut draft, flexible_idx);
        assert_eq!(draft.aspect, AspectRatio::Flexible);
        assert!(draft.config.video.flexible_aspect);
        assert_eq!(draft.config.video.width, 1600);
        assert_eq!(draft.config.video.height, 900);

        // Resolution is now disabled and shows "1600 x 900"
        assert!(!enabled(res_binding, true, &draft, &draft.config));
        assert_eq!(Choice::Resolution.items(&env, &draft), ["1600 x 900"]);
        assert_eq!(Choice::Resolution.selected(&env, &draft), Some(0));
        assert_eq!(Choice::Resolution.text(&env, &draft), "1600 x 900");

        // Selecting resolution while flexible is a no-op
        Choice::Resolution.select(&env, &mut draft, 0);

        // Window exceeding 32:9 has pillarbox black stripes; reading excludes them (8000x1000 -> 3556x1000)
        let pillarbox_modes = DisplayModes {
            fullscreen: vec![],
            desktop: None,
            current: Some(Resolution::new(8000, 1000)),
        };
        let env_pillarbox = Env {
            localization: &loc,
            languages: &langs,
            modes: &pillarbox_modes,
            recommendation: None,
        };
        assert_eq!(
            Choice::Resolution.items(&env_pillarbox, &draft),
            ["3556 x 1000"]
        );
        assert_eq!(
            Choice::Resolution.text(&env_pillarbox, &draft),
            "3556 x 1000"
        );

        // Window taller than 9:16 has letterbox black stripes; reading excludes them (500x2000 -> 500x889)
        let letterbox_modes = DisplayModes {
            fullscreen: vec![],
            desktop: None,
            current: Some(Resolution::new(500, 2000)),
        };
        let env_letterbox = Env {
            localization: &loc,
            languages: &langs,
            modes: &letterbox_modes,
            recommendation: None,
        };
        assert_eq!(
            Choice::Resolution.items(&env_letterbox, &draft),
            ["500 x 889"]
        );
        assert_eq!(Choice::Resolution.text(&env_letterbox, &draft), "500 x 889");

        // Switch back to 16:9
        let wide_idx = AspectRatio::ALL
            .iter()
            .position(|a| *a == AspectRatio::Wide16x9)
            .unwrap();
        Choice::Aspect.select(&env, &mut draft, wide_idx);
        assert_eq!(draft.aspect, AspectRatio::Wide16x9);
        assert!(!draft.config.video.flexible_aspect);
        assert!(enabled(res_binding, true, &draft, &draft.config));
        assert_eq!(
            Choice::Resolution.items(&env, &draft),
            ["1920 x 1080", "1280 x 720"]
        );
    }
}
