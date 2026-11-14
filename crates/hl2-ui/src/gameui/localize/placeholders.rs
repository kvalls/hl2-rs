//! **Temporary** English placeholders for UI text that only HL2RS shows.
//!
//! Some HL2RS features have no token in the installed game's localization files (the console's
//! history search prompt). Until HL2RS has its own localization
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
pub const STATUS_SHADERS: &str = "#HL2RS_StatusShaders";
pub const STATUS_WATER: &str = "#HL2RS_StatusWater";
pub const STATUS_MULTICORE: &str = "#HL2RS_StatusMulticore";
pub const STATUS_CLASSIC_EFFECTS: &str = "#HL2RS_StatusClassicEffects";
pub const STATUS_AERO: &str = "#HL2RS_StatusAero";
pub const STATUS_STEAM_INPUT: &str = "#HL2RS_StatusSteamInput";
pub const STATUS_UNIMPLEMENTED: &str = "#HL2RS_StatusUnimplemented";
/// A key hint's command with no key bound (the retail client's own text; no installed token).
pub const KEY_NOT_BOUND: &str = "#HL2RS_KeyNotBound";

/// Temporary English text of each placeholder token (keys without the `#`).
pub const PLACEHOLDER_STRINGS: &[(&str, &str)] = &[
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
    ("HL2RS_Console_ReverseSearch", "reverse-i-search"),
    ("HL2RS_KeyNotBound", "< not bound >"),
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
        assert!(CONSOLE_REVERSE_SEARCH.starts_with("#HL2RS_"));
        assert!(lookup(CONSOLE_REVERSE_SEARCH).is_some_and(|t| !t.is_empty()));
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
        assert_eq!(loc.text(CONSOLE_REVERSE_SEARCH), "reverse-i-search");
        loc.insert(CONSOLE_REVERSE_SEARCH, "history search");
        assert_eq!(loc.text(CONSOLE_REVERSE_SEARCH), "history search");
    }
}
