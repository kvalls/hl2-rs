//! User settings and configuration persistence.
//! Stored in a JSON file alongside the executable, never inside the game install.
//!
//! Every section uses `#[serde(default)]`, so files written by older builds keep loading: missing
//! fields take their defaults, and the legacy `video.borderless` flag migrates to
//! [`VideoSettings::display_mode`] in [`Config::migrate`].

mod motion_blur;
pub use motion_blur::MotionBlurMode;
use serde::{Deserialize, Serialize};
use source_ui::placement::DialogPlacement;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub use crate::hdr::HdrMode;

/// The Video page's Display mode combo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayMode {
    #[default]
    Fullscreen,
    Windowed,
    /// "Windowed (No Border)": a borderless window covering the display (on macOS the native
    /// full-screen Space).
    NoBorder,
}

impl DisplayMode {
    /// The choices offered, in combo order: the same on every platform.
    pub fn choices() -> &'static [Self] {
        &[Self::Fullscreen, Self::Windowed, Self::NoBorder]
    }

    /// The mode actually used: a mode that is not offered falls back to full screen. Every mode
    /// is offered on every platform today.
    pub fn effective(self) -> Self {
        if Self::choices().contains(&self) {
            self
        } else {
            Self::Fullscreen
        }
    }

    pub fn token(self) -> &'static str {
        match self {
            Self::Fullscreen => "#GameUI_Fullscreen",
            Self::Windowed => "#GameUI_Windowed",
            Self::NoBorder => "#GameUI_NoWindowBorder",
        }
    }
}

/// The Video page's HUD Aspect Ratio combo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HudAspect {
    #[default]
    Fit,
    Normal,
    Wide16x9,
    Wide16x10,
}

impl HudAspect {
    pub const ALL: [Self; 4] = [Self::Fit, Self::Normal, Self::Wide16x9, Self::Wide16x10];

    pub fn token(self) -> &'static str {
        match self {
            Self::Fit => "#GameUI_HUDAspectRatio_FitDisplay",
            Self::Normal => "#GameUI_HUDAspectRatio_4x3",
            Self::Wide16x9 => "#GameUI_HUDAspectRatio_16x9",
            Self::Wide16x10 => "#GameUI_HUDAspectRatio_16x10",
        }
    }
}

/// A setting chosen from a combo: its values in combo order and their localization tokens.
pub trait Listed: Copy + PartialEq + 'static {
    const ALL: &'static [Self];
    fn token(self) -> &'static str;
}

/// Combo settings with the items the retail dialogs list, in combo order.
macro_rules! choice_enum {
    ($(#[$doc:meta])* $name:ident { $($variant:ident => $token:literal),+ $(,)? }) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($variant,)+
        }

        impl Listed for $name {
            const ALL: &'static [Self] = &[$(Self::$variant),+];

            fn token(self) -> &'static str {
                match self {
                    $(Self::$variant => $token,)+
                }
            }
        }
    };
}

choice_enum! {
    /// Model detail (`r_rootlod`).
    ModelDetail { Low => "#gameui_low", Medium => "#gameui_medium", High => "#gameui_high" }
}

impl ModelDetail {
    /// The root LOD level for model geometry (`0` for High, `1` for Medium, `2` for Low).
    #[inline]
    pub const fn root_lod(self) -> usize {
        match self {
            Self::High => 0,
            Self::Medium => 1,
            Self::Low => 2,
        }
    }
}

choice_enum! {
    /// Texture detail (`mat_picmip`).
    TextureDetail {
        Low => "#gameui_low", Medium => "#gameui_medium", High => "#gameui_high",
        VeryHigh => "#gameui_ultra",
    }
}

impl TextureDetail {
    /// The mip level offset for VTF textures (`-1` for Very High, `0` for High, `1` for Medium, `2` for Low).
    #[inline]
    pub const fn picmip(self) -> i32 {
        match self {
            Self::VeryHigh => -1,
            Self::High => 0,
            Self::Medium => 1,
            Self::Low => 2,
        }
    }
}
choice_enum! {
    /// Shader detail (`mat_reducefillrate`).
    ShaderDetail { Low => "#gameui_low", High => "#gameui_high", VeryHigh => "#gameui_ultra" }
}
choice_enum! {
    /// Water detail (`r_waterforceexpensive` / `r_waterforcereflectentities`).
    WaterDetail {
        Simple => "#gameui_noreflections", World => "#gameui_reflectonlyworld",
        All => "#gameui_reflectall",
    }
}
choice_enum! {
    /// Shadow detail (`r_shadowrendertotexture` / `r_flashlightdepthtexture`).
    ShadowDetail { Low => "#gameui_low", Medium => "#gameui_medium", High => "#gameui_high" }
}
choice_enum! {
    /// Antialiasing mode (`mat_antialias`): the multisample counts offered here.
    Antialiasing {
        None => "#GameUI_None", X2 => "#GameUI_2X", X4 => "#GameUI_4X", X8 => "#GameUI_8X",
    }
}

impl Antialiasing {
    /// The multisample sample count (`1` for None/off, `2` for 2X, `4` for 4X, `8` for 8X).
    #[inline]
    pub const fn samples(self) -> u32 {
        match self {
            Self::None => 1,
            Self::X2 => 2,
            Self::X4 => 4,
            Self::X8 => 8,
        }
    }
}

choice_enum! {
    /// Filtering mode (`mat_trilinear` / `mat_forceaniso`).
    Filtering {
        Bilinear => "#GameUI_Bilinear", Trilinear => "#GameUI_Trilinear",
        Anisotropic2 => "#GameUI_Anisotropic2X", Anisotropic4 => "#GameUI_Anisotropic4X",
        Anisotropic8 => "#GameUI_Anisotropic8X", Anisotropic16 => "#GameUI_Anisotropic16X",
    }
}

impl Filtering {
    /// The anisotropic filtering level (`1` for bilinear and trilinear, `2`, `4`, `8`, `16` for anisotropic).
    #[inline]
    pub const fn aniso(self) -> u16 {
        match self {
            Self::Bilinear | Self::Trilinear => 1,
            Self::Anisotropic2 => 2,
            Self::Anisotropic4 => 4,
            Self::Anisotropic8 => 8,
            Self::Anisotropic16 => 16,
        }
    }

    /// Whether mip levels are blended linearly (`true` for trilinear and anisotropic, `false` for bilinear nearest mip).
    #[inline]
    pub const fn is_trilinear(self) -> bool {
        !matches!(self, Self::Bilinear)
    }
}

/// Video > Advanced. Detail levels (`model_detail`, `texture_detail`), `antialiasing`, `filtering`, and `vsync` affect rendering.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AdvancedVideoSettings {
    pub model_detail: ModelDetail,
    pub texture_detail: TextureDetail,
    pub shader_detail: ShaderDetail,
    pub water_detail: WaterDetail,
    pub shadow_detail: ShadowDetail,
    /// `mat_colorcorrection`.
    pub color_correction: bool,
    pub antialiasing: Antialiasing,
    pub filtering: Filtering,
    /// `mat_vsync`: present frames in step with the display.
    pub vsync: bool,
    /// `mat_motion_blur_enabled`.
    pub motion_blur: bool,
    /// Opt-in animation blur; false preserves retail camera-only blur and old saves.
    pub motion_blur_viewmodel: bool,
    /// Ultra-only camera/world and independent projectile reconstruction.
    pub motion_blur_world: bool,
    /// `mat_queue_mode` (-1 multicore, 0 single).
    pub multicore: bool,
    /// Classic Effects (`r_classic_fire` / `r_classic_blood`).
    pub classic_effects: bool,
    /// Windows Aero extensions (`mat_disable_d3d9ex` 0); the layout shows it on Windows only.
    pub d3d9ex: bool,
}

impl Default for AdvancedVideoSettings {
    fn default() -> Self {
        Self {
            model_detail: ModelDetail::High,
            texture_detail: TextureDetail::High,
            shader_detail: ShaderDetail::High,
            water_detail: WaterDetail::World,
            shadow_detail: ShadowDetail::High,
            color_correction: true,
            antialiasing: Antialiasing::None,
            filtering: Filtering::Trilinear,
            vsync: true,
            motion_blur: true,
            motion_blur_viewmodel: false,
            motion_blur_world: false,
            multicore: true,
            classic_effects: false,
            d3d9ex: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoSettings {
    pub width: u32,
    pub height: u32,
    pub display_mode: DisplayMode,
    /// Video > Aspect Ratio "Fit Display": the game frame takes the window's or display's
    /// aspect ratio (within `9:16..=32:9`) and `width`/`height` only set its detail and the
    /// requested windowed size.
    pub flexible_aspect: bool,
    pub fov: f32,
    /// Vertical field of view in degrees (Video > Advanced); 0 is automatic: derived from
    /// `fov` and the screen's aspect ratio. An explicit value replaces the derived one and scales
    /// with weapon zoom.
    pub vertical_fov: f32,
    /// `viewmodel_fov`: the viewmodel's horizontal 4:3 field of view in degrees.
    pub viewmodel_fov: f32,
    pub hdr: HdrMode,
    /// Set once the old default `hdr: none` has been replaced (see [`crate::hdr::migrate_default`]).
    pub hdr_migrated: Option<bool>,
    pub hud_aspect: HudAspect,
    /// `mat_monitorgamma` (1.6 bright to 2.6 dark).
    pub gamma: f32,
    /// Video > Advanced.
    pub advanced: AdvancedVideoSettings,
    /// Legacy flag from older files; read once by [`Config::migrate`], never written.
    #[serde(skip_serializing)]
    pub borderless: Option<bool>,
}

/// What a loaded map is built from: a change of any reloads the map.
pub type LoadDetail = (ModelDetail, TextureDetail, bool);

impl VideoSettings {
    pub fn load_detail(&self) -> LoadDetail {
        (
            self.advanced.model_detail,
            self.advanced.texture_detail,
            self.hdr.is_hdr(),
        )
    }
}

impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 720,
            display_mode: DisplayMode::Fullscreen,
            flexible_aspect: false,
            fov: 75.0,
            vertical_fov: 0.0,
            viewmodel_fov: hl2_simulation::viewmodel_effects::VIEWMODEL_FOV,
            hdr: HdrMode::Full,
            hdr_migrated: None,
            hud_aspect: HudAspect::Fit,
            gamma: 2.2,
            advanced: AdvancedVideoSettings::default(),
            borderless: None,
        }
    }
}

/// Speaker configuration (`snd_surround_speakers`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Speakers {
    Headphones,
    #[default]
    Two,
    Four,
    FiveOne,
    SevenOne,
}

impl Speakers {
    pub const ALL: [Self; 5] = [
        Self::Headphones,
        Self::Two,
        Self::Four,
        Self::FiveOne,
        Self::SevenOne,
    ];

    pub fn token(self) -> &'static str {
        match self {
            Self::Headphones => "#GameUI_Headphones",
            Self::Two => "#GameUI_2Speakers",
            Self::Four => "#GameUI_4Speakers",
            Self::FiveOne => "#GameUI_5Speakers",
            Self::SevenOne => "#GameUI_7Speakers",
        }
    }

    pub fn channels(self) -> u8 {
        match self {
            Self::Headphones => 0,
            Self::Two => 2,
            Self::Four => 4,
            Self::FiveOne => 5,
            Self::SevenOne => 7,
        }
    }

    pub fn from_channels(channels: u8) -> Option<Self> {
        match channels {
            0 => Some(Self::Headphones),
            2 => Some(Self::Two),
            4 => Some(Self::Four),
            5 => Some(Self::FiveOne),
            7 => Some(Self::SevenOne),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundQuality {
    #[default]
    High,
    Medium,
    Low,
}

impl SoundQuality {
    pub const ALL: [Self; 3] = [Self::High, Self::Medium, Self::Low];

    pub fn token(self) -> &'static str {
        match self {
            Self::High => "#GameUI_High",
            Self::Medium => "#GameUI_Medium",
            Self::Low => "#GameUI_Low",
        }
    }
}

/// Captioning (`closecaption` / `cc_subtitles`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Captions {
    #[default]
    None,
    ClosedCaptions,
    Subtitles,
}

impl Captions {
    pub const ALL: [Self; 3] = [Self::None, Self::ClosedCaptions, Self::Subtitles];

    pub fn token(self) -> &'static str {
        match self {
            Self::None => "#GameUI_NoClosedCaptions",
            Self::ClosedCaptions => "#GameUI_SubtitlesAndSoundEffects",
            Self::Subtitles => "#GameUI_Subtitles",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub volume: f32,
    pub music_volume: f32,
    pub speakers: Speakers,
    pub quality: SoundQuality,
    /// Spoken language (installed voice pack name, lower case). Applied after a restart.
    pub language: String,
    pub captions: Captions,
    /// `snd_mute_losefocus`.
    pub mute_losefocus: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            volume: 1.0,
            music_volume: 1.0,
            speakers: Speakers::Two,
            quality: SoundQuality::High,
            language: "english".into(),
            captions: Captions::None,
            mute_losefocus: true,
        }
    }
}

/// Aim-Assist combo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AimAssist {
    #[default]
    Off,
    Basic,
    Enhanced,
}

impl AimAssist {
    pub const ALL: [Self; 3] = [Self::Off, Self::Basic, Self::Enhanced];

    pub fn token(self) -> &'static str {
        match self {
            Self::Off => "#GameUI_SettingOff",
            Self::Basic => "#GameUI_SettingBasic",
            Self::Enhanced => "#GameUI_SettingEnhanced",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MouseSettings {
    pub sensitivity: f32,
    pub invert: bool,
    /// `m_filter`: average the last two frames of mouse movement.
    pub filter: bool,
    /// `m_rawinput`.
    pub raw_input: bool,
    /// `m_customaccel` enabled.
    pub acceleration: bool,
    /// `m_customaccel_exponent` (1.0 unaccelerated).
    pub accel_exponent: f32,
    pub aim_assist: AimAssist,
    /// `m_autoaim_scale`.
    pub aim_assist_scale: f32,
}

impl Default for MouseSettings {
    fn default() -> Self {
        Self {
            sensitivity: 3.0,
            invert: false,
            filter: false,
            raw_input: false,
            acceleration: false,
            accel_exponent: 1.05,
            aim_assist: AimAssist::Off,
            aim_assist_scale: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameplaySettings {
    /// `skill`: 1 Easy, 2 Normal, 3 Hard.
    pub skill: u8,
    /// `commentary`: developer commentary track enabled.
    pub commentary: bool,
    /// Overridden or unlocked main-menu background map (falls back to `scripts/chapterbackgrounds.txt`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_map: Option<String>,
    /// Installation folder of the game `background_map` belongs to (`hl2`, `lostcoast`, ...);
    /// unset means the first game of the menu.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_game: Option<String>,
}

impl Default for GameplaySettings {
    fn default() -> Self {
        Self {
            skill: 2,
            commentary: false,
            background_map: None,
            background_game: None,
        }
    }
}

choice_enum! {
    /// Gamepad weapon selection style (`hud_weaponui_gamepad` 2 cross, 3 carousel).
    WeaponSelectionStyle {
        Cross => "#GameUI_Gamepad_WeaponSelection_Crossbar",
        Carousel => "#GameUI_Gamepad_WeaponSelection_Carousel",
    }
}

/// The Gamepad page. Stored only: HL2-RS has no gamepad input yet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GamepadSettings {
    /// `sc_sensitivity_scale`.
    pub sensitivity_scale: f32,
    /// `sc_autoaim_mode`.
    pub aim_assist: AimAssist,
    /// `sc_autoaim_scale`.
    pub aim_assist_scale: f32,
    /// `hud_weaponui_gamepad`; the retail page shows Cross for an unset value.
    pub weapon_selection: WeaponSelectionStyle,
}

impl Default for GamepadSettings {
    fn default() -> Self {
        Self {
            sensitivity_scale: 1.0,
            aim_assist: AimAssist::Off,
            aim_assist_scale: 1.0,
            weapon_selection: WeaponSelectionStyle::Cross,
        }
    }
}

/// Keyboard > Advanced.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyboardSettings {
    /// `hud_fastswitch`: number keys switch weapons at once instead of opening the menu.
    pub fast_switch: bool,
    /// `con_enable`: the console key opens the developer console.
    pub developer_console: bool,
}

impl Default for KeyboardSettings {
    fn default() -> Self {
        Self {
            fast_switch: false,
            developer_console: true,
        }
    }
}

/// Saved dialog placements; unset dialogs open at their default place.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DialogPlacements {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<DialogPlacement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub console: Option<DialogPlacement>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub gameplay: GameplaySettings,
    pub video: VideoSettings,
    pub audio: AudioSettings,
    pub mouse: MouseSettings,
    pub keyboard: KeyboardSettings,
    pub gamepad: GamepadSettings,
    pub dialogs: DialogPlacements,
    pub keys: BTreeMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        let mut keys = BTreeMap::new();
        keys.insert("+attack".into(), "MOUSE1".into());
        keys.insert("+attack2".into(), "MOUSE2".into());
        keys.insert("+forward".into(), "KeyW".into());
        keys.insert("+back".into(), "KeyS".into());
        keys.insert("+moveleft".into(), "KeyA".into());
        keys.insert("+moveright".into(), "KeyD".into());
        keys.insert("+jump".into(), "Space".into());
        // config_default.cfg: `bind_mac "ALT" "+duck"`, `bind_mac "LWIN" "+walk"`,
        // `unbind_mac "CTRL"`.
        if cfg!(target_os = "macos") {
            keys.insert("+duck".into(), "AltLeft".into());
            keys.insert("+walk".into(), "SuperLeft".into());
        } else {
            keys.insert("+duck".into(), "ControlLeft".into());
            keys.insert("+walk".into(), "AltLeft".into());
        }
        keys.insert("+speed".into(), "ShiftLeft".into());
        keys.insert("+use".into(), "KeyE".into());
        keys.insert("+reload".into(), "KeyR".into());
        keys.insert("lastinv".into(), "KeyQ".into());
        keys.insert("invprev".into(), "MWHEELUP".into());
        keys.insert("invnext".into(), "MWHEELDOWN".into());
        keys.insert("impulse".into(), "KeyG".into());
        // config_default.cfg: `bind "f" "impulse 100"`.
        keys.insert("impulse 100".into(), "KeyF".into());
        keys.insert("loadout".into(), "F3".into());
        keys.insert("slot1".into(), "Digit1".into());
        keys.insert("slot2".into(), "Digit2".into());
        keys.insert("slot3".into(), "Digit3".into());
        keys.insert("slot4".into(), "Digit4".into());
        keys.insert("slot5".into(), "Digit5".into());
        keys.insert("slot6".into(), "Digit6".into());
        Self {
            gameplay: GameplaySettings::default(),
            video: VideoSettings::default(),
            audio: AudioSettings::default(),
            mouse: MouseSettings::default(),
            keyboard: KeyboardSettings::default(),
            gamepad: GamepadSettings::default(),
            dialogs: DialogPlacements::default(),
            keys,
        }
    }
}

impl Config {
    pub fn config_path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("hl2-config.json")))
            .unwrap_or_else(|| PathBuf::from("hl2-config.json"))
    }

    pub fn load_or_default() -> Self {
        let path = Self::config_path();
        std::fs::read(&path)
            .ok()
            .and_then(|bytes| Self::from_json(&bytes))
            .unwrap_or_default()
    }

    /// Parses a config file and applies [`Config::migrate`]. Returns `None` for unreadable JSON.
    pub fn from_json(bytes: &[u8]) -> Option<Self> {
        let mut config: Self = serde_json::from_slice(bytes).ok()?;
        config.migrate();
        crate::hdr::migrate_default(&mut config.video);
        Some(config)
    }

    /// Upgrades legacy fields and brings out-of-range values back into their valid ranges.
    pub fn migrate(&mut self) {
        if let Some(borderless) = self.video.borderless.take() {
            self.video.display_mode = if borderless {
                DisplayMode::NoBorder
            } else {
                DisplayMode::Windowed
            };
        }
        let clamp = |value: &mut f32, min: f32, max: f32, default: f32| {
            *value = if value.is_finite() {
                value.clamp(min, max)
            } else {
                default
            };
        };
        let defaults = Self::default();
        clamp(&mut self.audio.volume, 0., 1., defaults.audio.volume);
        clamp(
            &mut self.audio.music_volume,
            0.,
            1.,
            defaults.audio.music_volume,
        );
        clamp(
            &mut self.mouse.sensitivity,
            0.1,
            20.,
            defaults.mouse.sensitivity,
        );
        clamp(
            &mut self.mouse.accel_exponent,
            1.,
            1.4,
            defaults.mouse.accel_exponent,
        );
        clamp(
            &mut self.mouse.aim_assist_scale,
            0.5,
            2.,
            defaults.mouse.aim_assist_scale,
        );
        clamp(&mut self.video.gamma, 1.6, 2.6, defaults.video.gamma);
        clamp(
            &mut self.gamepad.sensitivity_scale,
            0.5,
            2.,
            defaults.gamepad.sensitivity_scale,
        );
        clamp(
            &mut self.gamepad.aim_assist_scale,
            0.5,
            2.,
            defaults.gamepad.aim_assist_scale,
        );
        clamp(&mut self.video.fov, 50., 120., defaults.video.fov);
        let (vertical_min, vertical_max) = crate::tuning::vertical_fov_range();
        if self.video.vertical_fov != 0. {
            clamp(
                &mut self.video.vertical_fov,
                vertical_min,
                vertical_max,
                defaults.video.vertical_fov,
            );
        }
        let (viewmodel_min, viewmodel_max) = crate::tuning::viewmodel_fov_range();
        clamp(
            &mut self.video.viewmodel_fov,
            viewmodel_min,
            viewmodel_max,
            defaults.video.viewmodel_fov,
        );
        self.gameplay.skill = self.gameplay.skill.clamp(1, 3);
        if let Some(map) = self.gameplay.background_map.take() {
            let clean = map
                .trim()
                .strip_suffix(".bsp")
                .unwrap_or(map.trim())
                .to_owned();
            if !clean.is_empty()
                && clean.len() <= 128
                && clean
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            {
                self.gameplay.background_map = Some(clean);
            } else {
                self.gameplay.background_map = None;
            }
        }
        if let Some(game) = self.gameplay.background_game.take() {
            let game = game.trim();
            if self.gameplay.background_map.is_some()
                && !game.is_empty()
                && game.len() <= 64
                && game
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                self.gameplay.background_game = Some(game.to_owned());
            }
        }
        // A hand-edited or corrupt placement opens the dialog at its default place instead.
        for placement in [&mut self.dialogs.options, &mut self.dialogs.console] {
            let valid = placement.is_some_and(|p| {
                p.offset.iter().all(|v| v.is_finite())
                    && p.size
                        .is_none_or(|s| s.iter().all(|v| v.is_finite() && *v > 0.))
            });
            if !valid {
                *placement = None;
            }
        }
        if self.audio.language.trim().is_empty() {
            self.audio.language = defaults.audio.language;
        }
        // Missing actions take their default unless that key already runs another command;
        // explicitly unbound actions are stored as "" and stay unbound.
        for (action, key) in defaults.keys {
            if !self.keys.contains_key(&action) && !self.keys.values().any(|bound| *bound == key) {
                self.keys.insert(action, key);
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            let _ = std::fs::create_dir_all(parent);
        }
        let data = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_and_serialization() {
        let default_cfg = Config::default();
        let serialized = serde_json::to_string_pretty(&default_cfg).unwrap();
        assert!(!serialized.contains("borderless"));
        let deserialized = Config::from_json(serialized.as_bytes()).unwrap();
        let mut expected = default_cfg.clone();
        expected.video.hdr_migrated = Some(true);
        assert_eq!(expected, deserialized);
        assert_eq!(deserialized.video.hdr, HdrMode::Full);
        assert_eq!(deserialized.video.display_mode, DisplayMode::Fullscreen);
        assert_eq!(deserialized.audio.volume, 1.0);
        assert_eq!(deserialized.mouse.sensitivity, 3.0);
        assert_eq!(deserialized.gameplay.skill, 2);
        assert_eq!(deserialized.mouse.aim_assist, AimAssist::Off);
        assert_eq!(deserialized.audio.speakers, Speakers::Two);
        assert_eq!(deserialized.audio.captions, Captions::None);
        assert!(!deserialized.mouse.invert);
    }

    #[test]
    fn legacy_files_keep_loading_and_migrate_borderless() {
        let legacy = br#"{
            "video": { "width": 1920, "height": 1080, "borderless": false, "fov": 90.0, "hdr": "full" },
            "audio": { "volume": 0.5 },
            "mouse": { "sensitivity": 2.5, "invert": true },
            "keys": { "+jump": "KeyJ" }
        }"#;
        let config = Config::from_json(legacy).unwrap();
        assert_eq!(config.video.display_mode, DisplayMode::Windowed);
        assert_eq!(config.video.borderless, None);
        assert_eq!((config.video.width, config.video.height), (1920, 1080));
        assert_eq!(config.video.hdr, HdrMode::Full);
        assert_eq!(config.video.gamma, 2.2);
        assert_eq!(config.audio.volume, 0.5);
        assert_eq!(config.audio.music_volume, 1.0);
        assert!(config.mouse.invert);
        assert_eq!(config.mouse.accel_exponent, 1.05);
        assert_eq!(config.gameplay.skill, 2);
        assert_eq!(config.keys.get("+jump").map(String::as_str), Some("KeyJ"));
        assert!(
            config.keyboard.developer_console && !config.keyboard.fast_switch,
            "files without a keyboard section keep the console and the weapon menu"
        );

        let borderless = Config::from_json(br#"{ "video": { "borderless": true } }"#).unwrap();
        assert_eq!(borderless.video.display_mode, DisplayMode::NoBorder);
        assert_eq!(
            borderless.video.display_mode.effective(),
            DisplayMode::NoBorder
        );
        assert_eq!(
            DisplayMode::choices(),
            [
                DisplayMode::Fullscreen,
                DisplayMode::Windowed,
                DisplayMode::NoBorder
            ],
            "every platform offers all three"
        );
    }

    #[test]
    fn migrate_clamps_out_of_range_values() {
        let config = Config::from_json(
            br#"{ "gameplay": { "skill": 9 }, "video": { "gamma": 9.0 }, "mouse": { "accel_exponent": 0.2 }, "audio": { "language": " " }, "gamepad": { "sensitivity_scale": 7.0 } }"#,
        )
        .unwrap();
        assert_eq!(config.gameplay.skill, 3);
        assert_eq!(config.video.gamma, 2.6);
        assert_eq!(config.mouse.accel_exponent, 1.0);
        assert_eq!(config.audio.language, "english");
        assert_eq!(config.gamepad.sensitivity_scale, 2.0);
        let corrupt = Config::from_json(
            br#"{ "dialogs": { "options": { "offset": [1.0, 2.0] }, "console": { "offset": [0.0, 0.0], "size": [-5.0, 100.0] } } }"#,
        )
        .unwrap();
        assert_eq!(corrupt.dialogs.options.map(|p| p.offset), Some([1., 2.]));
        assert_eq!(
            corrupt.dialogs.console, None,
            "a non-positive size is dropped"
        );
        assert_eq!(config.gamepad.weapon_selection, WeaponSelectionStyle::Cross);
        assert!(Config::from_json(b"not json").is_none());
        assert_eq!(HdrMode::from_level(1), Some(HdrMode::Bloom));
        assert_eq!(HdrMode::from_level(3), None);
    }

    #[test]
    fn migrate_fills_missing_default_bindings_but_keeps_unbinds_and_taken_keys() {
        let config = Config::from_json(
            br#"{ "keys": { "+attack": "", "+jump": "MOUSE2", "+forward": "KeyZ" } }"#,
        )
        .unwrap();
        let key = |action: &str| config.keys.get(action).map(String::as_str);
        assert_eq!(key("+attack"), Some(""), "explicit unbind kept");
        assert_eq!(key("+attack2"), None, "MOUSE2 already runs +jump");
        assert_eq!(key("+forward"), Some("KeyZ"));
        assert_eq!(key("invnext"), Some("MWHEELDOWN"), "new default filled in");
        assert_eq!(key("+back"), Some("KeyS"));
    }

    #[test]
    fn background_map_persistence_roundtrip_and_migration() {
        let mut cfg = Config::default();
        assert_eq!(cfg.gameplay.background_map, None);
        let serialized = serde_json::to_string(&cfg).unwrap();
        assert!(!serialized.contains("background_map"));

        cfg.gameplay.background_map = Some("background02".into());
        let serialized = serde_json::to_string(&cfg).unwrap();
        assert!(serialized.contains("\"background_map\":\"background02\""));
        let deserialized = Config::from_json(serialized.as_bytes()).unwrap();
        assert_eq!(
            deserialized.gameplay.background_map.as_deref(),
            Some("background02")
        );

        // Migration strips .bsp extensions
        let with_bsp =
            Config::from_json(br#"{"gameplay":{"background_map":"background03.bsp"}}"#).unwrap();
        assert_eq!(
            with_bsp.gameplay.background_map.as_deref(),
            Some("background03")
        );

        // Migration discards invalid or blank map names
        let blank = Config::from_json(br#"{"gameplay":{"background_map":"   "}}"#).unwrap();
        assert_eq!(blank.gameplay.background_map, None);

        let invalid =
            Config::from_json(br#"{"gameplay":{"background_map":"bad map/name"}}"#).unwrap();
        assert_eq!(invalid.gameplay.background_map, None);
    }

    #[test]
    fn background_game_persists_with_its_map_and_rejects_unsafe_folders() {
        let mut cfg = Config::default();
        cfg.gameplay.background_map = Some("background01".into());
        cfg.gameplay.background_game = Some("lostcoast".into());
        let serialized = serde_json::to_string(&cfg).unwrap();
        let deserialized = Config::from_json(serialized.as_bytes()).unwrap();
        assert_eq!(
            deserialized.gameplay.background_game.as_deref(),
            Some("lostcoast")
        );
        assert_eq!(
            deserialized.gameplay.background_map.as_deref(),
            Some("background01")
        );

        // A game without a (valid) map, or an unsafe folder name, is dropped.
        for json in [
            &br#"{"gameplay":{"background_game":"lostcoast"}}"#[..],
            br#"{"gameplay":{"background_map":"bad map","background_game":"lostcoast"}}"#,
            br#"{"gameplay":{"background_map":"background01","background_game":"../x"}}"#,
            br#"{"gameplay":{"background_map":"background01","background_game":" "}}"#,
        ] {
            let cfg = Config::from_json(json).unwrap();
            assert_eq!(
                cfg.gameplay.background_game,
                None,
                "{}",
                String::from_utf8_lossy(json)
            );
        }
    }

    #[test]
    fn antialiasing_and_filtering_mappings() {
        assert_eq!(Antialiasing::None.samples(), 1);
        assert_eq!(Antialiasing::X2.samples(), 2);
        assert_eq!(Antialiasing::X4.samples(), 4);
        assert_eq!(Antialiasing::X8.samples(), 8);

        assert_eq!(Filtering::Bilinear.aniso(), 1);
        assert!(!Filtering::Bilinear.is_trilinear());

        assert_eq!(Filtering::Trilinear.aniso(), 1);
        assert!(Filtering::Trilinear.is_trilinear());

        assert_eq!(Filtering::Anisotropic2.aniso(), 2);
        assert!(Filtering::Anisotropic2.is_trilinear());

        assert_eq!(Filtering::Anisotropic4.aniso(), 4);
        assert!(Filtering::Anisotropic4.is_trilinear());

        assert_eq!(Filtering::Anisotropic8.aniso(), 8);
        assert!(Filtering::Anisotropic8.is_trilinear());

        assert_eq!(Filtering::Anisotropic16.aniso(), 16);
        assert!(Filtering::Anisotropic16.is_trilinear());
    }
}
