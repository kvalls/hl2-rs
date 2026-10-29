//! **Temporary** English placeholders for UI text that only HL2RS shows.
//!
//! Some HL2RS features have no token in the installed game's localization files (the ultrawide
//! aspect choices, the console's history search prompt). Until HL2RS has its own localization
//! resources, each such string gets an `HL2RS_`-prefixed token constant here and an English
//! placeholder in [`PLACEHOLDER_STRINGS`]. Code always refers to the token constant, never to the
//! text, so nothing changes at the call sites when the placeholders go away.
//!
//! Planned replacement: a resource- and plugin-based localization layer. HL2RS would ship
//! `hl2rs_<language>.txt` files in the same `"lang" { "Tokens" { ... } }` format as the game's
//! `*_english.txt` (loaded with [`super::Localization::add_resource`]), and plugins or users could
//! add or override files for any language. Lookup order would then be: the game's resources, then
//! HL2RS and plugin resources, then the key name. When that layer exists, the entries below move
//! into `hl2rs_english.txt` and this table is deleted; the constants stay as the token names.
//!
//! Today [`super::Localization::lookup`] checks loaded resources first, then this table, then
//! [`super::FALLBACK_STRINGS`], so a loaded resource that defines one of these tokens already wins.

/// Aspect Ratio choice for 21:9 displays (the installed GameUI has no such choice).
pub const ASPECT_ULTRAWIDE_21X9: &str = "#HL2RS_AspectUltrawide21x9";
/// Aspect Ratio choice for 32:9 displays (the installed GameUI has no such choice).
pub const ASPECT_ULTRAWIDE_32X9: &str = "#HL2RS_AspectUltrawide32x9";
/// Aspect Ratio choice for square displays (no installed choice).
pub const ASPECT_SQUARE_1X1: &str = "#HL2RS_AspectSquare1x1";
/// Aspect Ratio choice for 3:4 portrait displays (no installed choice).
pub const ASPECT_PORTRAIT_3X4: &str = "#HL2RS_AspectPortrait3x4";
/// Aspect Ratio choice for 10:16 portrait displays (no installed choice).
pub const ASPECT_PORTRAIT_10X16: &str = "#HL2RS_AspectPortrait10x16";
/// Aspect Ratio choice for 9:16 portrait displays (no installed choice).
pub const ASPECT_PORTRAIT_9X16: &str = "#HL2RS_AspectPortrait9x16";
/// Question of the keep-or-revert box after an aspect ratio change (no installed token; the
/// title, buttons and countdown are installed tokens).
pub const KEEP_VIDEO_SETTINGS: &str = "#HL2RS_KeepVideoSettings";
/// Label of the Video > Advanced vertical field of view slider (no installed token).
pub const VERTICAL_FOV: &str = "#HL2RS_VerticalFov";
/// Label of the Video > Advanced viewmodel field of view slider (no installed token).
pub const VIEWMODEL_FOV: &str = "#HL2RS_ViewmodelFov";
/// Ctrl+R history search prompt in the console (no installed token).
pub const CONSOLE_REVERSE_SEARCH: &str = "#HL2RS_Console_ReverseSearch";

/// Development-only settings reports; no corresponding installed game tokens exist.
pub const STATUS_RAW_INPUT: &str = "#HL2RS_StatusRawInput";
pub const STATUS_FOCUS_AUDIO: &str = "#HL2RS_StatusFocusAudio";
pub const STATUS_AIM_ASSIST: &str = "#HL2RS_StatusAimAssist";
pub const STATUS_SPEAKERS: &str = "#HL2RS_StatusSpeakers";
pub const STATUS_SOUND_QUALITY: &str = "#HL2RS_StatusSoundQuality";
pub const STATUS_LANGUAGE: &str = "#HL2RS_StatusLanguage";
pub const STATUS_GAMEPAD: &str = "#HL2RS_StatusGamepad";
pub const MOTION_BLUR_ULTRA: &str = "#HL2RS_MotionBlurUltra";
pub const STATUS_MOTION_BLUR: &str = "#HL2RS_StatusMotionBlur";
pub const STATUS_SHADERS: &str = "#HL2RS_StatusShaders";
pub const STATUS_WATER: &str = "#HL2RS_StatusWater";
pub const STATUS_MULTICORE: &str = "#HL2RS_StatusMulticore";
pub const STATUS_CLASSIC_EFFECTS: &str = "#HL2RS_StatusClassicEffects";
pub const STATUS_AERO: &str = "#HL2RS_StatusAero";
pub const STATUS_STEAM_INPUT: &str = "#HL2RS_StatusSteamInput";
pub const STATUS_UNIMPLEMENTED: &str = "#HL2RS_StatusUnimplemented";

/// Temporary English text of each placeholder token (keys without the `#`).
pub const PLACEHOLDER_STRINGS: &[(&str, &str)] = &[
    ("HL2RS_MotionBlurUltra", "Ultra"),
    ("HL2RS_StatusMotionBlur", "Advanced adds viewmodel animation blur. Ultra also reconstructs world/camera and projectile motion; GPU performance tuning is in progress."),
    ("HL2RS_StatusRawInput", "In development: raw input is always used. Turning it off has no effect yet."),
    ("HL2RS_StatusFocusAudio", "Partially implemented: muting works, but disabling it cannot keep audio playing while the game pauses on focus loss."),
    ("HL2RS_StatusAimAssist", "Not implemented: aim assistance and its strength setting have no effect yet."),
    ("HL2RS_StatusSpeakers", "Not implemented: audio output is stereo. This setting does not change the speaker mix yet."),
    ("HL2RS_StatusSoundQuality", "Not implemented: changing sound quality has no effect yet."),
    ("HL2RS_StatusLanguage", "Not implemented: changing the spoken language does not switch voice audio yet."),
    ("HL2RS_StatusGamepad", "Not implemented: controller input and these controller settings are not connected yet."),
    ("HL2RS_StatusShaders", "Not implemented: shader quality does not change with this setting yet."),
    ("HL2RS_StatusWater", "Not implemented: water reflections and refraction are not rendered yet. This setting has no effect."),
    ("HL2RS_StatusMulticore", "Not implemented: this setting does not change how rendering work is shared across CPU cores."),
    ("HL2RS_StatusClassicEffects", "Not implemented: this setting does not switch between classic and updated effects yet."),
    ("HL2RS_StatusAero", "Not implemented: this setting does not change Windows desktop effects."),
    ("HL2RS_StatusSteamInput", "Not implemented: opening Steam Input configuration from this button is not connected yet."),
    ("HL2RS_StatusUnimplemented", "Not implemented: this control has no effect yet."),
    // Worded like the installed "Widescreen 16:9" / "Widescreen 16:10" choices.
    ("HL2RS_AspectUltrawide21x9", "Ultrawide 21:9"),
    ("HL2RS_AspectUltrawide32x9", "Super Ultrawide 32:9"),
    ("HL2RS_AspectSquare1x1", "Square 1:1"),
    ("HL2RS_AspectPortrait3x4", "Portrait 3:4"),
    ("HL2RS_AspectPortrait10x16", "Portrait 10:16"),
    ("HL2RS_AspectPortrait9x16", "Portrait 9:16"),
    ("HL2RS_KeepVideoSettings", "Keep these display settings?"),
    ("HL2RS_VerticalFov", "Vertical FOV"),
    ("HL2RS_ViewmodelFov", "Viewmodel FOV"),
    ("HL2RS_Console_ReverseSearch", "reverse-i-search"),
];

/// The placeholder text of `token` (with or without `#`, any case).
pub fn lookup(token: &str) -> Option<&'static str> {
    let key = token.trim().trim_start_matches('#');
    PLACEHOLDER_STRINGS
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| *v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_constant_has_placeholder_text_and_an_hl2rs_token() {
        for token in [
            ASPECT_ULTRAWIDE_21X9,
            ASPECT_ULTRAWIDE_32X9,
            ASPECT_SQUARE_1X1,
            ASPECT_PORTRAIT_3X4,
            ASPECT_PORTRAIT_10X16,
            ASPECT_PORTRAIT_9X16,
            KEEP_VIDEO_SETTINGS,
            VERTICAL_FOV,
            VIEWMODEL_FOV,
            CONSOLE_REVERSE_SEARCH,
        ] {
            assert!(token.starts_with("#HL2RS_"), "{token}");
            assert!(lookup(token).is_some_and(|t| !t.is_empty()), "{token}");
        }
        for (key, _) in PLACEHOLDER_STRINGS {
            assert!(key.starts_with("HL2RS_"), "{key}");
            assert!(
                super::super::FALLBACK_STRINGS
                    .iter()
                    .all(|(k, _)| !k.eq_ignore_ascii_case(key)),
                "{key} is not also a game-token fallback"
            );
        }
    }

    #[test]
    fn loaded_resources_override_placeholders() {
        let mut loc = super::super::Localization::default();
        assert_eq!(loc.text(ASPECT_ULTRAWIDE_21X9), "Ultrawide 21:9");
        loc.insert(ASPECT_ULTRAWIDE_21X9, "Ultralarge 21:9");
        assert_eq!(loc.text(ASPECT_ULTRAWIDE_21X9), "Ultralarge 21:9");
    }
}
