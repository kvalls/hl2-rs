//! Tests for background visual debug console commands and opacity/FOV controls.

use crate::commands::Request;
use hl2_ui::console::{Console, Effect, Mode};
fn console() -> Console {
    let mut c = Console::default();
    let mut e = hl2_ui::extensions::Extensions::default();
    crate::register(&mut e).unwrap();
    e.install(&mut c);
    c
}
fn effect(r: Request) -> Effect {
    Effect::Extension(serde_json::to_value(r).unwrap())
}

#[test]
fn background_debug_mode_command_behavior() {
    let mut console = console();
    console.set_mode(Mode::MainMenu);
    console.set_mode(Mode::Console);
    assert_eq!(console.mode(), Mode::Console);

    // In MainMenu, enabling is allowed and focuses the menu base
    assert_eq!(
        console.submit("background_debug_mode 1"),
        vec![effect(Request::BackgroundDebugMode(true))]
    );
    assert!(console.presentation.capture_menu_navigation);
    assert_eq!(console.mode(), Mode::MainMenu);

    // Disabling
    assert_eq!(
        console.submit("background_debug_mode 0"),
        vec![effect(Request::BackgroundDebugMode(false))]
    );
    assert!(!console.presentation.capture_menu_navigation);

    // In gameplay mode (not background, not loading), enabling is rejected
    console.set_mode(Mode::Gameplay);
    assert!(console.submit("background_debug_mode 1").is_empty());
    assert!(!console.presentation.capture_menu_navigation);
}

#[test]
fn background_blur_opacity_requires_debug_mode() {
    let mut console = console();
    // Default opacity is 50
    assert_eq!(console.presentation.background_opacity.unwrap_or(50), 50);

    // Fails when background_debug_mode is 0
    assert!(console.submit("background_blur_opacity 75").is_empty());
    assert_eq!(console.presentation.background_opacity.unwrap_or(50), 50);

    // Works when background_debug_mode is 1
    console.presentation.capture_menu_navigation = true;
    assert_eq!(
        console.submit("background_blur_opacity 75"),
        vec![effect(Request::BackgroundBlurOpacity(75))]
    );
    assert_eq!(console.presentation.background_opacity.unwrap_or(50), 75);

    // Query prints current value without effect
    assert!(console.submit("background_blur_opacity").is_empty());

    // Out-of-bounds (> 100) is rejected
    assert!(console.submit("background_blur_opacity 101").is_empty());
    assert_eq!(console.presentation.background_opacity.unwrap_or(50), 75);
}

#[test]
fn background_map_fov_commands() {
    let mut console = console();
    assert_eq!(
        console.submit("background_map_fov"),
        vec![effect(Request::BackgroundMapFov(None))]
    );
    assert_eq!(
        console.submit("background_map_fov 60"),
        vec![effect(Request::BackgroundMapFov(Some(60.0)))]
    );

    // Invalid FOV values rejected
    assert!(console.submit("background_map_fov 0").is_empty());
    assert!(console.submit("background_map_fov 200").is_empty());
}

#[test]
fn viewmodel_debug_and_reset_commands_reach_the_host() {
    let mut console = console();
    assert_eq!(
        console.submit("viewmodel_debug_mode 1; viewmodel_tweak_reset; viewmodel_debug_mode 0"),
        vec![
            effect(Request::ViewmodelDebugMode(true)),
            effect(Request::ViewmodelTweakReset),
            effect(Request::ViewmodelDebugMode(false))
        ]
    );
    assert!(
        console
            .submit("viewmodel_debug_mode; viewmodel_debug_mode 2; viewmodel_tweak_reset 1")
            .is_empty()
    );
}

#[test]
fn viewmodel_debug_name_is_registered_and_old_mode_name_is_not() {
    let names: Vec<_> = crate::commands::specs::<Console>()
        .iter()
        .map(|s| s.name)
        .collect();
    assert!(names.contains(&"viewmodel_debug_mode"));
    assert!(!names.contains(&"viewmodel_tweak_mode"));
}
