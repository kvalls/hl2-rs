//! The GameUI's localization: the toolkit's token table ([`source_ui::localize`]) over the
//! installed `*_english.txt` / `merged_loc_all.txt` files, with HL2RS's in-repo text for tokens
//! they lack ([`InRepoText`]): the temporary [`placeholders`], then [`FALLBACK_STRINGS`].
//!
//! ### Resource Priority & In-Repo Text Rule
//! **Always prioritize game resources/assets over in-repo assets or constants.**
//! Do NOT add values or tokens into `localize.rs` if they can be retrieved straight from game
//! resources/assets (`resource/gameui_english.txt`, `resource/valve_english.txt`, `resource/vgui_english.txt`,
//! `hl2_complete/resource/merged_loc_all.txt`, etc.). In-repo localization mechanisms (such as
//! [`placeholders`]) serve primarily as a source for text that has NO in-game resources, such as
//! modded, reconstructed, or expanded features (e.g. ultra-wide display modes, mod menus).
//!
//! Text that only HL2RS shows (no installed token) uses the temporary [`placeholders`].

pub mod placeholders;

use source_ui::localize::Fallback;

/// The GameUI's localization table: installed resources first, then [`InRepoText`], then the
/// token's key name.
pub type Localization = source_ui::localize::Localization<InRepoText>;

/// HL2RS's in-repo text for tokens no installed resource defines: the temporary
/// [`placeholders`], then the central [`FALLBACK_STRINGS`].
#[derive(Clone, Copy, Debug, Default)]
pub struct InRepoText;

impl Fallback for InRepoText {
    fn lookup(key: &str) -> Option<&'static str> {
        placeholders::lookup(key).or_else(|| fallback_lookup(key))
    }
}

/// Central English fallback table for pre-resource bootstrapping when launching without game resources.
///
/// **Rule**: Fallbacks in this table may ONLY be added for use cases when the game is launched
/// and cannot find or load the in-game resources (such as showing a dialog to browse the game
/// directory in setup, or essential abort/quit confirmation dialogs before game resources are mounted).
/// Never add fallbacks here for text that exists in retail game assets (`resource/gameui_english.txt`, etc.).
/// Always prioritize game resources. Text for modded, reconstructed, or expanded features that have NO
/// in-game resources belongs in [`placeholders`].
pub const FALLBACK_STRINGS: &[(&str, &str)] = &[
    ("GameUI_Console", "Console"),
    ("Console_Title", "Console"),
    ("Console_Submit", "Submit"),
    ("GameUI_ConfirmMainMenu_Title", "RETURN TO MAIN MENU"),
    (
        "GameUI_MainMenuWarning",
        "Are you sure?\nAll unsaved progress will be lost.",
    ),
    ("GameUI_QuitConfirmationTitle", "QUIT GAME"),
    (
        "GameUI_QuitConfirmationText",
        "Do you wish to stop playing now?",
    ),
    (
        "GameUI_Console_QuitWarning",
        "Are you sure you want to quit?\nAll unsaved progress will be lost.",
    ),
    ("GameUI_Quit", "Quit"),
    ("GameUI_Confirm", "CONFIRM"),
    ("Spec_Auto", "Auto"),
    ("vgui_TimeLeftSecondsRemaining", "%s1 seconds remaining"),
    ("vgui_TimeLeftSecondRemaining", "%s1 second remaining"),
    ("GameUI_OK", "OK"),
    ("GameUI_Cancel", "Cancel"),
    ("GameUI_Apply", "Apply"),
    ("GameUI_Close", "Close"),
    ("GameUI_Options", "OPTIONS"),
    ("GameUI_Loading", "LOADING..."),
    ("GameUI_LoadingFilename", "Loading %s1 ..."),
    (
        "LoadingProgress_SpawningServer",
        "Starting local game server...",
    ),
    ("LoadingProgress_LoadMap", "Loading world..."),
    ("LoadingProgress_PrecacheWorld", "Initializing world..."),
    (
        "LoadingProgress_SignonDataLocal",
        "Initializing game data...",
    ),
    ("LoadingProgress_LoadResources", "Loading resources..."),
    ("LoadingProgress_SignonLocal", "Initializing resources..."),
    ("GameUI_HalfLife2", "Half-Life 2"),
    ("GameUI_EpisodeOne", "Episode One"),
    ("GameUI_EpisodeTwo", "Episode Two"),
    ("GameUI_LostCoast", "Lost Coast"),
    ("GameUI_HalfLife2LostCoast", "Half-Life 2: Lost Coast"),
    ("GameUI_Worskhop_MenuName", "Workshop"),
    ("GameUI_Workshop_MenuName", "Workshop"),
    ("GameUI_Gameplay", "Gameplay"),
    ("GameUI_Keyboard", "Keyboard"),
    ("GameUI_Mouse", "Mouse"),
    ("GameUI_Gamepad", "Gamepad"),
    ("GameUI_Audio", "Audio"),
    ("GameUI_Video", "Video"),
    ("GameUI_Action", "Action"),
    ("GameUI_KeyButton", "Key / Button"),
    ("GameUI_AdjustGamma_Title", "BRIGHTNESS"),
    (
        "GameUI_ThirdPartyTechCredits",
        "3RD PARTY TECHNOLOGY CREDITS",
    ),
    ("GameUI_KeyboardSettings", "Keyboard Settings"),
    (
        "GameUI_KeyboardSettingsText",
        "Reset key bindings to default?",
    ),
    (
        "GameUI_Miles_Voice",
        "Uses Miles Sound System. Copyright © 1991-2001 by RAD Game Tools, Inc.",
    ),
    (
        "GameUI_Miles_Audio",
        "MPEG Layer-3 playback supplied with the Miles Sound System from RAD Game Tools, Inc. MPEG Layer-3 audio compression technology licensed by Fraunhofer IIS and THOMSON multimedia.",
    ),
    (
        "GameUI_Bink",
        "Uses Bink Video. Copyright © 1997-2007 by RAD Game Tools, Inc.",
    ),
    ("Valve_Hud_HEALTH", "HEALTH"),
    ("Valve_Hud_SUIT", "SUIT"),
    ("Valve_Hud_AMMO", "AMMO"),
    ("Valve_Hud_AMMO_ALT", "ALT"),
];

/// Looks up an authentic English fallback string for the given token name if defined in the central table.
pub fn fallback_lookup(token: &str) -> Option<&'static str> {
    let key = token.trim().trim_start_matches('#');
    FALLBACK_STRINGS
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| *v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn central_fallback_table_resolves_standard_tokens_when_resources_absent() {
        let loc = Localization::default();
        assert_eq!(
            loc.text("#GameUI_ConfirmMainMenu_Title"),
            "RETURN TO MAIN MENU"
        );
        assert_eq!(
            loc.text("#GameUI_MainMenuWarning"),
            "Are you sure?\nAll unsaved progress will be lost."
        );
        assert_eq!(loc.text("#GameUI_Console"), "Console");
        assert_eq!(loc.text("#GameUI_Loading"), "LOADING...");
        assert_eq!(loc.text("#LoadingProgress_LoadMap"), "Loading world...");
        assert_eq!(loc.text("#Valve_Hud_HEALTH"), "HEALTH");
        assert_eq!(loc.text("#Valve_Hud_SUIT"), "SUIT");
        assert_eq!(loc.text("#Valve_Hud_AMMO"), "AMMO");
        assert_eq!(loc.text("#Valve_Hud_AMMO_ALT"), "ALT");
    }

    #[test]
    fn loaded_game_resources_take_precedence_over_central_fallbacks() {
        let mut loc = Localization::default();
        loc.insert("GameUI_Console", "Custom Console");
        assert_eq!(loc.text("#GameUI_Console"), "Custom Console");
    }

    #[test]
    fn audit_no_hardcoded_user_visible_strings_in_ui_code_outside_fallback_constants() {
        // The workspace's crates folder: the UI files are in hl2-ui and in source-ui, which holds
        // code that came from the console (input.rs, console/command.rs, console/panel.rs), the
        // menu (message_box.rs) and the HUD (font.rs).
        let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates folder");
        let console_files = [
            "hl2-ui/src/console.rs",
            "hl2-ui/src/console/focus.rs",
            "hl2-ui/src/console/loading.rs",
            "hl2-ui/src/console/menu_flow.rs",
            "hl2-ui/src/console/options_dialog.rs",
            "hl2-ui/src/console/panel.rs",
            "source-ui/src/input.rs",
            "source-ui/src/console/command.rs",
            "source-ui/src/console/panel.rs",
        ];
        let ui_files = console_files.into_iter().chain([
            "hl2-ui/src/gameui/loading.rs",
            "hl2-ui/src/gameui/menu.rs",
            "hl2-ui/src/gameui/options.rs",
            "hl2-ui/src/gameui/resources.rs",
            "hl2-ui/src/hud.rs",
            "source-ui/src/message_box.rs",
            "source-ui/src/font.rs",
        ]);
        let banned = [
            "Are you sure?\nAll unsaved progress will be lost.",
            "Are you sure you want to quit?",
            "RETURN TO MAIN MENU",
            "QUIT GAME",
            "Starting local game server...",
            "Loading world...",
            "Initializing world...",
            "Initializing game data...",
            "Loading resources...",
            "Initializing resources...",
            "3RD PARTY TECHNOLOGY CREDITS",
            "Reset key bindings to default?",
        ];
        for rel in ui_files {
            let path = crates.join(rel);
            let content = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
            // Check non-test code
            let non_test_content = content.split("#[cfg(test)]").next().unwrap_or(&content);
            for b in banned {
                assert!(
                    !non_test_content.contains(b),
                    "Found banned user-visible literal {:?} in {}",
                    b,
                    path.display()
                );
            }
        }

        // Specifically check that the console's non-test code does not hardcode "Console" title
        for rel in console_files {
            let console_src = std::fs::read_to_string(crates.join(rel)).unwrap();
            let console_non_test = console_src.split("#[cfg(test)]").next().unwrap();
            assert!(
                !console_non_test.contains("text(\n                \"Console\""),
                "{rel} should not hardcode Console title string"
            );
        }

        // Check that hud.rs non-test code does not hardcode numeric panel labels
        let hud_src = std::fs::read_to_string(crates.join("hl2-ui/src/hud.rs")).unwrap();
        let hud_non_test = hud_src.split("#[cfg(test)]").next().unwrap();
        for label in ["\"HEALTH\"", "\"SUIT\"", "\"AMMO\"", "\"ALT\""] {
            assert!(
                !hud_non_test.contains(&format!(
                    "numeric_panel(\n            &self.health_panel,\n            {label}"
                )),
                "hud.rs should not pass literal {label} to numeric_panel"
            );
        }
    }
}
