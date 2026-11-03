use super::{Console, Effect};
use crate::config::Config;
use source_ui::canvas::Texture2D;

#[test]
fn stored_default_blur_is_backward_compatible_and_independent_of_debug_mode() {
    let legacy = Config::from_json(br#"{"video":{"width":1280,"height":720}}"#).unwrap();
    assert!(!legacy.video.background_force_default_blur);
    let mut console = Console::default();
    assert!(!console.presentation.capture_menu_navigation);
    assert!(!console.config.video.background_uses_widescreen(false));
    assert!(console.config.video.background_uses_widescreen(true));
    let effects = console.submit("background_force_default_blur 1");
    assert!(
        matches!(effects.as_slice(), [Effect::ApplyConfig(config)] if config.video.background_force_default_blur)
    );
    assert!(!console.presentation.capture_menu_navigation);
    assert!(!console.config.video.background_uses_widescreen(true));
    let saved = serde_json::to_vec(&console.config).unwrap();
    assert!(
        Config::from_json(&saved)
            .unwrap()
            .video
            .background_force_default_blur
    );
    console.submit("background_force_default_blur");
    assert_eq!(
        console.shell.panel.output.back().unwrap().text,
        "\"background_force_default_blur\" = \"1\""
    );
    assert!(console.submit("background_force_default_blur 9").is_empty());
    assert!(console.config.video.background_force_default_blur);
    console.base = super::Mode::MainMenu;
    console.presentation.capture_menu_navigation = true;
    console.presentation.capture_menu_navigation = false;
    assert!(
        console.config.video.background_force_default_blur,
        "debug mode lifetime must not reset a saved option"
    );
    console.submit("background_force_default_blur 0");
    assert!(!console.config.video.background_force_default_blur);
}

#[test]
fn override_selects_only_standard_artwork_and_matches_diagnostic_names() {
    let mut console = Console::default();
    let normal = Texture2D::from_rgba8(1, 1, &[255; 4]);
    let wide = Texture2D::from_rgba8(2, 1, &[255; 8]);
    console
        .background_textures
        .insert(("hl2".into(), "background01".into()), normal);
    console
        .background_textures
        .insert(("hl2".into(), "background01_widescreen".into()), wide);
    assert_eq!(
        console
            .background_texture("hl2", "background01", true)
            .unwrap()
            .texture
            .size()
            .x,
        2.
    );
    assert_eq!(
        console
            .background_texture("hl2", "background01", false)
            .unwrap()
            .texture
            .size()
            .x,
        1.
    );
    assert_eq!(
        console.blur_texture_name("background01", true),
        "background01_widescreen"
    );
    console.config.video.background_force_default_blur = true;
    assert_eq!(
        console
            .background_texture("hl2", "background01", true)
            .unwrap()
            .texture
            .size()
            .x,
        1.
    );
    assert_eq!(
        console.blur_texture_name("background01", true),
        "background01"
    );
    console
        .background_textures
        .remove(&("hl2".into(), "background01".into()));
    assert!(
        console
            .background_texture("hl2", "background01", true)
            .is_none(),
        "forced normal must not fall back to wide"
    );
    console.config.video.background_force_default_blur = false;
    assert!(console
        .background_texture("hl2", "background01", true)
        .is_some());
}

#[test]
fn debug_toggle_uses_the_saved_cvar_and_defers_normal_host_application() {
    let mut console = Console::default();
    console.toggle_background_force_default_blur();
    assert!(console.config.video.background_force_default_blur);
    assert!(
        matches!(console.tick(1.).as_slice(), [Effect::ApplyConfig(config)] if config.video.background_force_default_blur)
    );
    console.toggle_background_force_default_blur();
    assert!(!console.config.video.background_force_default_blur);
    assert!(
        matches!(console.tick(2.).as_slice(), [Effect::ApplyConfig(config)] if !config.video.background_force_default_blur)
    );
}

#[test]
fn selected_artwork_retains_its_authored_aspect_including_missing_wide_fallback() {
    let mut console = Console::default();
    console.background_textures.insert(
        ("hl2".into(), "background01".into()),
        Texture2D::from_rgba8(1, 1, &[255; 4]),
    );
    let fallback = console
        .background_texture("hl2", "background01", true)
        .unwrap();
    assert!(!fallback.widescreen);
    console.background_textures.insert(
        ("hl2".into(), "background01_widescreen".into()),
        Texture2D::from_rgba8(1, 1, &[255; 4]),
    );
    assert!(
        console
            .background_texture("hl2", "background01", true)
            .unwrap()
            .widescreen
    );
    console.config.video.background_force_default_blur = true;
    assert!(
        !console
            .background_texture("hl2", "background01", true)
            .unwrap()
            .widescreen
    );
}
