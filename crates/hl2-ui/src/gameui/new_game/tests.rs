use super::*;

#[test]
fn discovery_uses_config_keys_not_map_tables_and_sorts_fractional_chapters() {
    let mut loc = Localization::default();
    loc.insert("GameUI_Chapter", "KAPITEL");
    loc.insert("mod_Chapter9a_Title", "Verflechtung");
    let files = [
        "cfg/chapter10.cfg",
        "cfg/chapter9a.cfg",
        "cfg/chapter9.cfg",
        "cfg/chapter1.cfg",
        "cfg/chapter2.cfg",
        "cfg/chapter.cfg",
        "cfg/chapter../3.cfg",
    ];
    let entries = catalog(
        files,
        |f| match f {
            "cfg/chapter1.cfg" => Some("playvideo_exitcommand recap map first".into()),
            "cfg/chapter2.cfg" => Some("// map absent".into()),
            _ => Some("map arbitrary".into()),
        },
        "mod",
        &loc,
    );
    assert_eq!(
        entries.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(),
        ["1", "9", "9a", "10"]
    );
    assert_eq!(entries[0].map, "first");
    assert_eq!(entries[2].title, "Verflechtung");
    assert_eq!(entries[2].heading, "KAPITEL 9a");
}

#[test]
#[ignore = "requires an owned installation"]
fn owned_chapter_catalogs_use_each_games_configs_localization_and_thumbnails() {
    let root = source_assets::install::discover().expect("owned installation");
    let vfs = source_assets::vpk::Vfs::mount(&root).unwrap();
    let resources = super::super::resources::GameUiResources::load(&vfs);
    for (game, count, first, last) in [
        ("hl2", 15, "d1_trainstation_01", "credits"),
        ("episodic", 5, "ep1_citadel_00", "ep1_c17_05"),
        ("ep2", 7, "ep2_outland_01", "ep2_outland_12a"),
        ("lostcoast", 1, "d2_lostcoast", "d2_lostcoast"),
    ] {
        let c = &resources.new_games[game];
        assert_eq!(c.entries.len(), count, "{game}");
        assert_eq!(c.entries.first().unwrap().map, first, "{game}");
        assert_eq!(c.entries.last().unwrap().map, last, "{game}");
        assert_eq!(
            c.thumbnails.iter().filter(|t| t.is_some()).count(),
            count,
            "{game}"
        );
        assert!(
            c.entries
                .iter()
                .all(|e| !e.title.to_ascii_lowercase().contains("_chapter")),
            "{game}"
        );
        assert!(c.dialog.get("Play").is_some(), "{game}");
        assert!(c.dialog.get("Commentary").is_some(), "{game}");
        assert!(c.commentary_icon.is_some(), "{game}");
        let spec = c.browser_spec(&resources.localization);
        assert!(spec.layout.toggle.is_some(), "{game}");
        assert!(spec.toggle_label.is_some(), "{game}");
        assert!(spec.toggle_icon.is_some(), "{game}");
        let card = &spec.layout.card;
        assert!(card.badge.is_some(), "{game}");
        assert_eq!(card.size, glam::vec2(168., 146.), "{game}");
        assert_eq!(
            card.border,
            source_ui::canvas::Rect::new(0., 44., 168., 102.),
            "{game}"
        );
        assert_eq!(
            card.image,
            source_ui::canvas::Rect::new(8., 52., 152., 86.),
            "{game}"
        );
        assert_eq!(card.heading.w, 168., "{game}");
        assert_eq!(card.title.w, 168., "{game}");
    }
}

#[test]
fn toolkit_acceptance_maps_the_selected_index_to_its_hl2_game_and_map() {
    let catalog = Catalog {
        entries: vec![Entry {
            key: "2".into(),
            map: "entrypoint".into(),
            heading: "heading".into(),
            title: "title".into(),
        }],
        ..Default::default()
    };
    let mut dialog = Dialog::new("selected-game", &catalog, &Localization::default());
    assert_eq!(
        dialog.input(
            &source_ui::input::Input {
                keys: [source_ui::input::Key::Enter].into(),
                ..Default::default()
            },
            glam::vec2(1024., 768.)
        ),
        Outcome::Start(crate::console::GameMap::new("selected-game", "entrypoint"))
    );
}
