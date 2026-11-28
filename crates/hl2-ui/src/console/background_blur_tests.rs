use super::Console;
use crate::config::VideoSettings;
use crate::extensions::{Extensions, StandardBackground};
use source_ui::canvas::Texture2D;

fn always(_: &VideoSettings) -> bool {
    true
}

fn by_flag(video: &VideoSettings) -> bool {
    video.extensions.bool("standard_flag").unwrap_or(false)
}

const ALWAYS: StandardBackground = StandardBackground(Some(always));

#[test]
fn a_contributed_rule_decides_the_standard_background_and_core_alone_keeps_retail() {
    let mut console = Console::default();
    assert!(!console.config.video.background_uses_widescreen(false));
    assert!(console.config.video.background_uses_widescreen(true));
    let mut extensions = Extensions::default();
    extensions.select_standard_background(by_flag).unwrap();
    assert!(
        extensions.select_standard_background(always).is_err(),
        "one rule only"
    );
    extensions.install(&mut console);
    assert!(console.config.video.background_uses_widescreen(true));
    console
        .config
        .video
        .extensions
        .set_bool("standard_flag", true);
    assert!(!console.config.video.background_uses_widescreen(true));
    assert!(
        !console.presentation.capture_menu_navigation,
        "independent of background debug mode"
    );
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
    console.config.video.standard_background = ALWAYS;
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
    console.config.video.standard_background = StandardBackground::default();
    assert!(console
        .background_texture("hl2", "background01", true)
        .is_some());
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
    console.config.video.standard_background = ALWAYS;
    assert!(
        !console
            .background_texture("hl2", "background01", true)
            .unwrap()
            .widescreen
    );
}
