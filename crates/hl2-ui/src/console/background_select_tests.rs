use super::{Console, Effect, Mode};

#[test]
fn presentation_capture_suppresses_menu_arrow_navigation() {
    let mut console = Console {
        base: Mode::MainMenu,
        ..Default::default()
    };
    console.main_menu.selection = Some(0);
    let mut input = source_ui::input::Input::default();
    input.keys.insert(source_ui::input::Key::Down);

    // When background debug mode is false, Down navigates
    console.presentation.capture_menu_navigation = false;
    console.menu_input(&input, false);
    assert_eq!(console.main_menu.selection, Some(1));

    // When background debug mode is true, Down does not navigate
    console.presentation.capture_menu_navigation = true;
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
