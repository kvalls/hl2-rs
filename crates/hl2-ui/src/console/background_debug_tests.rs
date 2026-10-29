//! Tests for background visual debug console commands and opacity/FOV controls.

use super::{Console, Effect, Mode};

#[test]
fn background_debug_mode_command_behavior() {
    let mut console = Console {
        base: Mode::MainMenu,
        ..Default::default()
    };
    console.set_mode(Mode::Console);
    assert_eq!(console.mode(), Mode::Console);

    // In MainMenu, enabling is allowed and focuses the menu base
    assert_eq!(
        console.submit("background_debug_mode 1"),
        vec![Effect::BackgroundDebugMode(true)]
    );
    assert!(console.background_debug_mode);
    assert_eq!(console.mode(), Mode::MainMenu);

    // Disabling
    assert_eq!(
        console.submit("background_debug_mode 0"),
        vec![Effect::BackgroundDebugMode(false)]
    );
    assert!(!console.background_debug_mode);

    // In gameplay mode (not background, not loading), enabling is rejected
    console.base = Mode::Gameplay;
    assert!(console.submit("background_debug_mode 1").is_empty());
    assert!(!console.background_debug_mode);
}

#[test]
fn background_blur_opacity_requires_debug_mode() {
    let mut console = Console::default();
    // Default opacity is 50
    assert_eq!(console.background_blur_opacity, 50);

    // Fails when background_debug_mode is 0
    assert!(console.submit("background_blur_opacity 75").is_empty());
    assert_eq!(console.background_blur_opacity, 50);

    // Works when background_debug_mode is 1
    console.background_debug_mode = true;
    assert_eq!(
        console.submit("background_blur_opacity 75"),
        vec![Effect::BackgroundBlurOpacity(75)]
    );
    assert_eq!(console.background_blur_opacity, 75);

    // Query prints current value without effect
    assert!(console.submit("background_blur_opacity").is_empty());

    // Out-of-bounds (> 100) is rejected
    assert!(console.submit("background_blur_opacity 101").is_empty());
    assert_eq!(console.background_blur_opacity, 75);
}

#[test]
fn background_map_fov_commands() {
    let mut console = Console::default();
    assert_eq!(
        console.submit("background_map_fov"),
        vec![Effect::BackgroundMapFov(None)]
    );
    assert_eq!(
        console.submit("background_map_fov 60"),
        vec![Effect::BackgroundMapFov(Some(60.0))]
    );

    // Invalid FOV values rejected
    assert!(console.submit("background_map_fov 0").is_empty());
    assert!(console.submit("background_map_fov 200").is_empty());
}

#[test]
fn background_debug_mode_suppresses_menu_arrow_navigation() {
    let mut console = Console {
        base: Mode::MainMenu,
        ..Default::default()
    };
    console.main_menu.selection = Some(0);
    let mut input = source_ui::input::Input::default();
    input.keys.insert(source_ui::input::Key::Down);

    // When background debug mode is false, Down navigates
    console.background_debug_mode = false;
    console.menu_input(&input, false);
    assert_eq!(console.main_menu.selection, Some(1));

    // When background debug mode is true, Down does not navigate
    console.background_debug_mode = true;
    console.menu_input(&input, false);
    assert_eq!(console.main_menu.selection, Some(1));
}

#[test]
fn all_chapter_backgrounds_and_swap_background() {
    let mut console = Console::default();
    let list = console.all_chapter_backgrounds();
    assert_eq!(
        list,
        vec![
            ("hl2".into(), "background01".into()),
            ("episodic".into(), "ep1_background01".into()),
            ("ep2".into(), "ep2_background01".into()),
            ("lostcoast".into(), "background01".into()),
        ]
    );

    let (slot1_game, slot1_bg) = console.game_and_initial_background_for_slot(1).unwrap();
    assert_eq!(
        (slot1_game.as_str(), slot1_bg.as_str()),
        ("hl2", "background01")
    );
    let (slot2_game, slot2_bg) = console.game_and_initial_background_for_slot(2).unwrap();
    assert_eq!(
        (slot2_game.as_str(), slot2_bg.as_str()),
        ("episodic", "ep1_background01")
    );
    let (slot3_game, slot3_bg) = console.game_and_initial_background_for_slot(3).unwrap();
    assert_eq!(
        (slot3_game.as_str(), slot3_bg.as_str()),
        ("ep2", "ep2_background01")
    );
    let (slot4_game, slot4_bg) = console.game_and_initial_background_for_slot(4).unwrap();
    assert_eq!(
        (slot4_game.as_str(), slot4_bg.as_str()),
        ("lostcoast", "background01")
    );

    let effect = console.swap_background("episodic", "ep1_background01");
    assert_eq!(
        effect,
        Effect::MainMenuMap(crate::console::GameMap::new("episodic", "ep1_background01"))
    );
    assert_eq!(
        console.current_background_target(),
        ("episodic".into(), "ep1_background01".into())
    );
}
