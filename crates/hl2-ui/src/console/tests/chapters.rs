use super::*;

fn open_and_start(console: &mut Console) -> Vec<Effect> {
    let effects = click_row(console, 0);
    assert_eq!(
        effects,
        vec![Effect::PlaySound("ui/buttonclickrelease.wav".into())]
    );
    assert_eq!(console.mode(), Mode::NewGame);
    assert!(
        !console.is_loading(),
        "opening chapter selection must not load gameplay"
    );
    console.input(&Input {
        keys: [Key::Enter].into(),
        ..Default::default()
    })
}

#[test]
fn background_level_main_menu_navigation_new_game_and_quit_box() {
    let mut console = installed_console();
    console.level_loaded(true);
    assert_eq!(console.mode(), Mode::MainMenu);
    console.escape();
    assert_eq!(console.mode(), Mode::MainMenu, "nothing to resume");
    click_row(&mut console, 0);
    assert_eq!(console.main_menu.category(), "hl2");
    console.escape();
    assert_eq!(console.main_menu.category(), "main");
    assert_eq!(console.mode(), Mode::MainMenu);
    click_row(&mut console, 0);
    assert_eq!(
        open_and_start(&mut console),
        vec![
            Effect::PlaySound("ui/buttonclickrelease.wav".into()),
            Effect::ChapterMap(GameMap::new("hl2", "d1_trainstation_01")),
        ]
    );
    console.level_loaded(false);
    assert_eq!(console.mode(), Mode::Gameplay);
    assert_eq!(console.main_menu.category(), "main");
    console.escape();
    assert_eq!(console.mode(), Mode::Pause);

    console.level_loaded(true);
    click_row(&mut console, 6);
    let (quit, kind) = console.confirm_box.clone().expect("quit confirmation");
    assert_eq!(kind, ConfirmKind::Quit);
    let (origin, scale, layout) = console.quit_geometry(&quit);
    let click_btn = |console: &mut Console, button: Rect| {
        let pointer = Some(origin + vec2(button.x + 3., button.y + 3.) * scale);
        let _ = console.input(&Input {
            pointer,
            click: true,
            held: true,
            ..Default::default()
        });
        console.input(&Input {
            pointer,
            released: true,
            ..Default::default()
        })
    };
    assert_eq!(
        click_btn(&mut console, layout.buttons[1]),
        Vec::<Effect>::new()
    );
    assert!(console.confirm_box.is_none(), "Cancel closes the box");
    assert_eq!(console.mode(), Mode::MainMenu);
    click_row(&mut console, 6);
    assert_eq!(
        click_btn(&mut console, layout.buttons[0]),
        vec![Effect::Quit,]
    );
}

#[test]
fn episode_one_and_two_new_game_navigation_and_missing_content_error_recovery() {
    let mut console = installed_console();
    console.level_loaded(true);
    assert_eq!(console.mode(), Mode::MainMenu);

    // Row 1 on main menu: HL2: EPISODE ONE
    assert_eq!(
        click_row(&mut console, 1),
        vec![
            Effect::PlaySound("ui/buttonclickrelease.wav".into()),
            Effect::MainMenuMap(GameMap::new("episodic", "ep1_background01")),
        ]
    );
    assert_eq!(console.main_menu.category(), "episodic");
    console.level_loaded(true);

    // Row 0 in Episode One submenu: NEW GAME
    let effects = open_and_start(&mut console);
    assert_eq!(
        effects,
        vec![
            Effect::PlaySound("ui/buttonclickrelease.wav".into()),
            Effect::ChapterMap(GameMap::new("episodic", "ep1_citadel_00")),
        ]
    );
    assert!(console.is_loading());
    assert_eq!(
        console.loading_dialog.as_ref().unwrap().map,
        "ep1_citadel_00"
    );

    // Graceful error handling for missing episode content:
    // Loader reports failure, error is latched onto loading dialog.
    console.loading_failed("owned map ep1_citadel_00.bsp not found");
    assert!(!console.is_loading());
    assert!(console.loading_dialog.as_ref().unwrap().is_failed());
    assert_eq!(
        console.loading_dialog.as_ref().unwrap().error.as_deref(),
        Some("owned map ep1_citadel_00.bsp not found")
    );

    // Player dismisses failed dialog via Escape and returns to main menu
    console.escape();
    assert!(console.loading_dialog.is_none());
    assert_eq!(console.mode(), Mode::MainMenu);

    // Row 2 on main menu: HL2: EPISODE TWO
    assert_eq!(
        click_row(&mut console, 2),
        vec![
            Effect::PlaySound("ui/buttonclickrelease.wav".into()),
            Effect::MainMenuMap(GameMap::new("ep2", "ep2_background01")),
        ]
    );
    assert_eq!(console.main_menu.category(), "ep2");
    console.level_loaded(true);

    // Row 0 in Episode Two submenu: NEW GAME
    let effects = open_and_start(&mut console);
    assert_eq!(
        effects,
        vec![
            Effect::PlaySound("ui/buttonclickrelease.wav".into()),
            Effect::ChapterMap(GameMap::new("ep2", "ep2_outland_01")),
        ]
    );
    assert!(console.is_loading());
    assert_eq!(
        console.loading_dialog.as_ref().unwrap().map,
        "ep2_outland_01"
    );

    // Missing episode content error is dismissed via Enter input
    console.loading_failed("owned map ep2_outland_01.bsp not found");
    let enter = Input {
        keys: [Key::Enter].into_iter().collect(),
        ..Default::default()
    };
    console.input(&enter);
    assert!(console.loading_dialog.is_none());
    assert_eq!(console.mode(), Mode::MainMenu);
}

#[test]
fn lostcoast_navigation_and_missing_content_error_recovery() {
    let mut console = installed_console();
    console.set_mode(Mode::MainMenu);
    console.background_level = true;
    console.loaded_map = Some(GameMap::new("hl2", "background01"));

    // In main menu: row 4 is EXTRAS
    let effects = click_row(&mut console, 4);
    assert_eq!(console.main_menu.category(), "extras");
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::MainMenuMap(_))),
        "entering extras does not reload background map"
    );

    // In extras: row 1 is disabled Workshop placeholder
    assert!(!console.main_menu.entries()[1].enabled);
    assert_eq!(console.main_menu.entries()[1].label, "WORKSHOP");
    console.activate_menu(1);
    assert!(console
        .shell
        .panel
        .output
        .iter()
        .any(|line| line.contains("WORKSHOP: not implemented")));

    // In extras: row 0 is HALF-LIFE 2: LOST COAST
    let lc_effects = click_row(&mut console, 0);
    assert_eq!(console.main_menu.category(), "lostcoast");
    assert!(
        lc_effects
            .iter()
            .any(|e| e == &Effect::MainMenuMap(GameMap::new("lostcoast", "background01"))),
        "entering lostcoast submenu loads Lost Coast's own background01"
    );
    assert_eq!(
        console.config.gameplay.background_map.as_deref(),
        Some("background01")
    );
    assert_eq!(
        console.config.gameplay.background_game.as_deref(),
        Some("lostcoast")
    );
    console.level_loaded_for_map("lostcoast", "background01", true);

    // In lostcoast: row 0 is NEW GAME
    let new_game_effects = open_and_start(&mut console);
    assert!(
        new_game_effects
            .iter()
            .any(|e| matches!(e, Effect::ChapterMap(target) if target == &GameMap::new("lostcoast", "d2_lostcoast"))),
        "starts loading d2_lostcoast"
    );
    assert!(console.is_loading());
    assert_eq!(console.loading_dialog.as_ref().unwrap().map, "d2_lostcoast");

    // Simulate missing content error
    console.loading_failed("owned map d2_lostcoast.bsp not found");
    assert!(!console.is_loading());
    assert!(console.loading_dialog.as_ref().unwrap().is_failed());
    assert_eq!(
        console.loading_dialog.as_ref().unwrap().error.as_deref(),
        Some("owned map d2_lostcoast.bsp not found")
    );

    // Dismiss error with Escape -> returns to menu without crash
    console.escape();
    assert!(console.loading_dialog.is_none());
    assert_eq!(console.mode(), Mode::MainMenu);
}

#[test]
fn chapter_cancel_preserves_category_background_and_escape_waits_for_release() {
    let mut console = installed_console();
    console.level_loaded_for_map("ep2", "ep2_background01", true);
    console.activate_menu(2);
    console.open_chapters("ep2");
    let saved = console.background();
    console.escape_pressed();
    assert!(console.chapter_dialog().unwrap().browser.escape_held);
    assert_eq!(console.mode(), Mode::NewGame);
    console.escape_released();
    assert_eq!(console.mode(), Mode::MainMenu);
    assert_eq!(console.menu_category(), "ep2");
    assert_eq!(console.background(), saved);
    assert!(console.chapters_report().is_none());
    assert!(console.loading_dialog.is_none());
}

#[test]
fn chapter_menu_waits_for_background_success_or_failure() {
    for failure in [false, true] {
        let mut console = installed_console();
        console.level_loaded(true);
        console.activate_menu(1);
        assert!(console.background_transition.is_active());
        assert!(console.activate_menu(0).is_empty());
        console.open_chapters("episodic");
        assert_eq!(console.mode(), Mode::MainMenu);
        for key in [Key::Down, Key::Enter] {
            assert!(console
                .input(&Input {
                    keys: [key].into(),
                    ..Default::default()
                })
                .is_empty());
        }
        console.escape();
        assert_eq!(console.menu_category(), "episodic");
        if failure {
            console.background_load_failed_for_map(
                "episodic",
                "ep1_background01",
                "decode failure",
            );
        } else {
            console.level_loaded_for_map("episodic", "ep1_background01", true);
        }
        assert!(!console.is_background_loading());
        console.activate_menu(0);
        assert_eq!(console.mode(), Mode::NewGame);
    }
}

#[test]
fn returning_to_menu_drops_confirmation_before_reopening_chapters() {
    let mut console = installed_console();
    console.level_loaded(false);
    console.escape();
    click_row(&mut console, 5);
    assert!(console.confirm_box.is_some());
    console.confirm_input(&Input {
        keys: [Key::Enter].into(),
        ..Default::default()
    });
    assert_eq!(console.mode(), Mode::MainMenu);
    assert!(console.closing_confirm_box.is_none());
    assert!(!console.shell.windows.is_open(Window::Confirmation));
    console.level_loaded(true);
    console.open_chapters("hl2");
    assert_eq!(console.mode(), Mode::NewGame);
    console.tick(10.);
    assert!(!console.modal_dialog_open());
}

#[test]
fn starting_chapter_cancels_background_presentation_and_stale_failure() {
    let mut console = installed_console();
    console.start_background_loading("ep2", "ep2_background01");
    console.start_loading("ep2_outland_01");
    assert!(!console.is_background_loading());
    assert!(console.modal_dialog_open());
    console.background_load_failed_for_map("ep2", "ep2_background01", "stale worker failure");
    assert!(console.modal_dialog_open());
    console.level_loaded_for_map("ep2", "ep2_outland_01", false);
    assert_eq!(console.mode(), Mode::Gameplay);
    assert!(!console.is_loading());
    assert!(!console.is_background_loading());
}

#[test]
fn background_wait_blocks_switch_back_until_completion() {
    let mut console = installed_console();
    console.level_loaded_for_map("hl2", "background01", true);
    console.activate_menu(1);
    console.escape();
    assert!(console.activate_menu(0).is_empty());
    assert_eq!(
        console.background_transition.target_category(),
        Some("episodic")
    );
    console.level_loaded_for_map("episodic", "ep1_background01", true);
    console.escape();
    assert!(console
        .activate_menu(0)
        .contains(&Effect::MainMenuMap(GameMap::new("hl2", "background01"))));
}

#[test]
fn loading_draws_no_menu_titles_buttons_or_browser_and_preserves_selection() {
    let mut console = installed_console();
    console.level_loaded(true);
    console.open_chapters("hl2");
    console.input(&Input {
        keys: [Key::Right].into(),
        ..Default::default()
    });
    let selected = console.chapters_report().unwrap()["selected"].clone();
    console.start_background_loading("episodic", "ep1_background01");
    console.draw(0.);
    let hidden = console.canvas.drain();
    // Background composition plus loading plaque must be identical without any windows.
    console.set_mode(Mode::MainMenu);
    console.draw(0.);
    let base = console.canvas.drain();
    assert_eq!(hidden.len(), base.len());
    for (a, b) in hidden.iter().zip(&base) {
        assert_eq!(a.destination, b.destination);
        assert_eq!(a.color, b.color);
    }
    console.set_mode(Mode::NewGame);
    console.level_loaded_for_map("episodic", "ep1_background01", true);
    assert_eq!(console.chapters_report().unwrap()["selected"], selected);
    console.draw(1.);
    let bounds = console
        .chapter_dialog()
        .unwrap()
        .browser
        .bounds(console.canvas_size());
    let inside = |q: &&source_ui::canvas::Quad| {
        q.destination.x >= bounds.x
            && q.destination.y >= bounds.y
            && q.destination.x + q.destination.w <= bounds.x + bounds.w
            && q.destination.y + q.destination.h <= bounds.y + bounds.h
    };
    assert!(
        console.canvas.drain().iter().filter(inside).count() > hidden.iter().filter(inside).count()
    );
}

#[test]
fn selecting_submenu_keeps_existing_chapter_background_from_same_game() {
    let mut console = installed_console();
    // Simulate having loaded Half-Life 2 chapter 10's background (`hl2/background05`).
    console.set_background(Some(GameMap::new("hl2", "background05")));
    console.level_loaded_for_map("hl2", "background05", true);
    assert_eq!(console.main_menu.category(), "main");

    // Entering the Half-Life 2 submenu must keep `hl2/background05` without reloading `background01`.
    let effects = click_row(&mut console, 0);
    assert_eq!(console.main_menu.category(), "hl2");
    assert_eq!(
        effects,
        vec![Effect::PlaySound("ui/buttonclickrelease.wav".into())],
        "same-game submenu entry must not emit MainMenuMap when another chapter background of that game is active"
    );
    assert!(!console.is_loading());
    assert!(!console.is_background_loading());
    assert_eq!(
        console.loaded_map,
        Some(GameMap::new("hl2", "background05"))
    );
    assert_eq!(
        console.background(),
        Some(GameMap::new("hl2", "background05"))
    );

    // Returning to main and selecting a different game (Episode Two) loads its chapter 1 background.
    console.escape();
    let ep2_effects = click_row(&mut console, 2);
    assert!(ep2_effects.contains(&Effect::MainMenuMap(GameMap::new(
        "ep2",
        "ep2_background01"
    ))));
    console.level_loaded_for_map("ep2", "ep2_background01", true);

    // Simulate having loaded Episode Two chapter 3's background (`ep2/ep2_background02`).
    console.set_background(Some(GameMap::new("ep2", "ep2_background02")));
    console.level_loaded_for_map("ep2", "ep2_background02", true);
    console.escape();
    assert_eq!(console.main_menu.category(), "main");

    // Re-entering Episode Two submenu keeps `ep2/ep2_background02`.
    let ep2_reenter = click_row(&mut console, 2);
    assert_eq!(console.main_menu.category(), "ep2");
    assert_eq!(
        ep2_reenter,
        vec![Effect::PlaySound("ui/buttonclickrelease.wav".into())]
    );
    assert!(!console.is_background_loading());
    assert_eq!(
        console.loaded_map,
        Some(GameMap::new("ep2", "ep2_background02"))
    );
}

#[test]
fn gameplay_map_loading_draws_target_chapter_blurred_background_and_hides_menu() {
    let mut console = installed_console();
    console.canvas.resize(1280., 720.);
    console.level_loaded_for_map("hl2", "background01", true);
    std::sync::Arc::make_mut(&mut console.gameui)
        .chapters
        .insert(
            "hl2".into(),
            vec![
                crate::gameui::chapters::Chapter {
                    key: "1".into(),
                    background: "background01".into(),
                    start: Some("d1_trainstation_01".into()),
                },
                crate::gameui::chapters::Chapter {
                    key: "6".into(),
                    background: "background03".into(),
                    start: Some("d1_town_01".into()),
                },
            ],
        );

    // Distinct 2x2 textures for background01 vs background03 (chapter 6: d1_town_01).
    let tex_bg1 = Texture2D::from_rgba8(2, 2, &[10; 16]);
    let tex_bg3 = Texture2D::from_rgba8(2, 2, &[220; 16]);
    console.background_textures.insert(
        ("hl2".into(), "background01_widescreen".into()),
        tex_bg1.clone(),
    );
    console.background_textures.insert(
        ("hl2".into(), "background03_widescreen".into()),
        tex_bg3.clone(),
    );

    // Start loading chapter 6 (`d1_town_01`, whose chapter background is `background03`).
    console.start_loading_for_game("hl2", "d1_town_01");
    assert_eq!(
        console.loading_background,
        Some(GameMap::new("hl2", "background03"))
    );

    console.draw(0.);
    let quads = console.canvas.drain();
    let full_screen_bg = quads
        .iter()
        .find(|q| q.destination == Rect::new(0., 0., 1280., 720.) && q.texture.is_some())
        .expect("full-screen blurred chapter background quad");
    assert_eq!(
        full_screen_bg.texture.as_ref().unwrap().id(),
        tex_bg3.id(),
        "must draw the target map's chapter background texture (background03)"
    );

    // Also draws the blurred chapter background when loading in Mode::Gameplay (mid-campaign changelevel).
    console.set_mode(Mode::Gameplay);
    console.draw(0.);
    let gameplay_quads = console.canvas.drain();
    assert!(
        gameplay_quads
            .iter()
            .any(|q| q.destination == Rect::new(0., 0., 1280., 720.)
                && q.texture.as_ref().map(Texture2D::id) == Some(tex_bg3.id())),
        "Mode::Gameplay loading screen must also draw the blurred chapter background"
    );
}

fn press(console: &mut Console, key: Key) -> Vec<Effect> {
    console.confirm_input(&Input {
        keys: [key].into(),
        ..Default::default()
    })
}

#[test]
fn confirming_the_commentary_explanation_starts_the_chapter_with_commentary_and_cheats() {
    let mut console = installed_console();
    let target = GameMap::new("lostcoast", "d2_lostcoast");
    let message =
        crate::gameui::menu::commentary_explanation_box(&console.gameui.localization, |text| {
            text.len() as f32 * 7.0
        });
    console.confirm_box = Some((message, ConfirmKind::StartCommentary(target.clone())));
    let effects = press(&mut console, Key::Enter);
    assert!(effects.contains(&Effect::ChapterMap(target)));
    assert!(console.config.gameplay.commentary);
    assert!(console.cheats);
    assert!(console.is_loading());
    assert!(console.confirm_box.is_none());
}

#[test]
fn cancelling_the_commentary_explanation_does_not_start_or_enable_commentary() {
    let mut console = installed_console();
    console.config.gameplay.commentary = false;
    let target = GameMap::new("lostcoast", "d2_lostcoast");
    let message =
        crate::gameui::menu::commentary_explanation_box(&console.gameui.localization, |text| {
            text.len() as f32 * 7.0
        });
    console.confirm_box = Some((message, ConfirmKind::StartCommentary(target)));
    press(&mut console, Key::Tab);
    let effects = press(&mut console, Key::Enter);
    assert!(!effects.iter().any(|e| matches!(e, Effect::ChapterMap(_))));
    assert!(!console.config.gameplay.commentary);
    assert!(!console.cheats);
    assert!(!console.is_loading());
}

#[test]
fn a_normal_chapter_start_clears_commentary_and_cheats() {
    let mut console = installed_console();
    console.config.gameplay.commentary = true;
    console.cheats = true;
    let effects = console.start_chapter(GameMap::new("hl2", "d1_trainstation_01"), false);
    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::ChapterMap(m) if m.map == "d1_trainstation_01")));
    assert!(!console.config.gameplay.commentary);
    assert!(!console.cheats);
}

#[test]
fn start_background_loading_sets_main_menu_mode_and_draws_blurred_background() {
    let mut console = Console::default();
    console.canvas.resize(1920., 1080.);
    assert_eq!(console.mode(), Mode::Gameplay);

    console.start_background_loading("hl2", "background01");
    assert_eq!(console.mode(), Mode::MainMenu);
    assert!(console.is_background_loading());

    console.draw(0.0);
    let quads = console.canvas.drain();
    let backdrop_quads: Vec<_> = quads
        .iter()
        .filter(|q| {
            q.destination.w == 1920.
                && q.destination.h == 1080.
                && q.color == crate::gameui::loading::BackgroundTransitionState::MAIN_MENU_BACKDROP
        })
        .collect();
    assert_eq!(
        backdrop_quads.len(),
        1,
        "backdrop overlay quad must be drawn during background loading from default gameplay mode"
    );
}
