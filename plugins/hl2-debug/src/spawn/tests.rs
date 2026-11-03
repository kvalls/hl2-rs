use super::*;
#[test]
fn props_have_their_own_tab_with_folders() {
    let mut overlay = Overlay::default();
    overlay.key(OverlayKey::Toggle);
    // The NPC tab wraps among NPC classes only.
    overlay.key(OverlayKey::Up);
    assert_eq!(SPAWN_CLASSES[overlay.class], "npc_combinedropship");
    overlay.key(OverlayKey::Down);
    // Tab switches to the props: folders on the left, the folder's models on the right.
    let mut folders = std::collections::BTreeMap::new();
    folders.insert(
        "props_c17".to_string(),
        vec!["props_c17/oildrum001.mdl".to_string()],
    );
    folders.insert(
        "props_junk".to_string(),
        (0..30)
            .map(|i| format!("props_junk/item{i:02}.mdl"))
            .collect(),
    );
    overlay.set_props(folders);
    overlay.key(OverlayKey::Tab);
    assert_eq!(overlay.tab, Tab::Props);
    let panel = overlay.panel(true, false);
    assert!(panel.title.starts_with("F4 Props (31 models)"));
    assert_eq!(panel.headings[0], "folder");
    // The installed common props come first (with their folders), then each folder.
    assert_eq!(panel.classes[0].text, "* common/ (1)");
    assert_eq!(panel.classes[1].text, "props_c17/ (1)");
    assert!(panel.classes[0].cursor);
    assert_eq!(panel.options[0].text, "props_c17/oildrum001  (oil drum)");
    // Enter on a folder opens it; Enter on a model spawns it.
    overlay.key(OverlayKey::Down);
    overlay.key(OverlayKey::Down);
    assert_eq!(overlay.folder, 2);
    assert_eq!(overlay.key(OverlayKey::Enter), None);
    assert_eq!(overlay.column, Column::Options);
    overlay.key(OverlayKey::PageDown);
    assert_eq!(overlay.model, VISIBLE_CLASSES);
    overlay.key(OverlayKey::PageDown);
    overlay.key(OverlayKey::PageDown);
    // Pages clamp at the end; single steps wrap.
    assert_eq!(overlay.model, 29);
    overlay.key(OverlayKey::Down);
    assert_eq!(overlay.model, 0);
    overlay.key(OverlayKey::Up);
    let panel = overlay.panel(true, false);
    assert!(panel.options.iter().any(|r| r.cursor && r.text == "item29"));
    assert!(panel.options[0].text.contains("more above"));
    assert_eq!(
        overlay.key(OverlayKey::Enter),
        Some(OverlayCommand::SpawnProp("props_junk/item29.mdl".into()))
    );
    // Changing folders restarts at its first model; Tab returns to the NPCs.
    overlay.key(OverlayKey::Left);
    overlay.key(OverlayKey::Down);
    assert_eq!((overlay.folder, overlay.model), (0, 0));
    overlay.key(OverlayKey::Tab);
    assert_eq!(overlay.class, 0);
    assert!(matches!(
        overlay.key(OverlayKey::Enter),
        Some(OverlayCommand::Spawn(_))
    ));
    // Without an installation listing the Props tab falls back to SPAWN_PROPS.
    let mut fallback = Overlay::default();
    fallback.key(OverlayKey::Toggle);
    fallback.key(OverlayKey::Tab);
    fallback.key(OverlayKey::Enter);
    assert_eq!(
        fallback.key(OverlayKey::Enter),
        Some(OverlayCommand::SpawnProp("props_c17/oildrum001.mdl".into()))
    );
}
#[test]
fn the_mouse_and_the_filter_drive_the_overlay() {
    let mut o = Overlay::default();
    o.set_usable(usable());
    assert_eq!(o.click(PanelTarget::Tab(Tab::Props)), None);
    assert_eq!(o.tab, Tab::Npcs, "closed: clicks do nothing");
    o.key(OverlayKey::Toggle);
    // NPC tab: a click selects a class, a second click on it spawns it.
    let cop = SPAWN_CLASSES
        .iter()
        .position(|c| *c == "npc_metropolice")
        .unwrap();
    assert_eq!(o.click(PanelTarget::Row(Column::Classes, cop + 1)), None);
    assert_eq!(o.click(PanelTarget::Row(Column::Classes, cop)), None);
    assert_eq!(o.class, cop);
    let Some(OverlayCommand::Spawn(r)) = o.click(PanelTarget::Row(Column::Classes, cop)) else {
        panic!()
    };
    assert_eq!(r.classname, "npc_metropolice");
    // Options: a weapon click chooses it; toggles act on every click.
    o.click(PanelTarget::Row(Column::Options, 2));
    assert_eq!(o.weapon, "weapon_pistol");
    let allegiance = o
        .rows()
        .iter()
        .position(|r| *r == OptionRow::Allegiance)
        .unwrap();
    o.click(PanelTarget::Row(Column::Options, allegiance));
    o.click(PanelTarget::Row(Column::Options, allegiance));
    assert_eq!(o.allegiance, Allegiance::Foe.next());
    // The wheel moves the hovered column's selection, clamped.
    o.scroll(Some(Column::Classes), -1000);
    assert_eq!((o.class, o.column), (0, Column::Classes));
    o.scroll(Some(Column::Classes), 3);
    assert_eq!(o.class, 3);
    // The drawn rows carry their click targets; notes page the list.
    o.rows_visible = 5;
    let panel = o.panel(true, false);
    assert_eq!(panel.classes.len(), 7);
    assert_eq!(
        panel.classes[0].target,
        Some(PanelTarget::Page(Column::Classes, -1))
    );
    assert_eq!(
        panel.classes[1].target,
        Some(PanelTarget::Row(Column::Classes, 1))
    );
    o.click(PanelTarget::Page(Column::Classes, 1));
    assert_eq!(o.class, 8);
    assert_eq!(panel.tabs.len(), 2);
    // Props: the filter box takes typed text; matches come first across folders.
    let mut folders = std::collections::BTreeMap::new();
    folders.insert(
        "props_c17".to_string(),
        vec![
            "props_c17/oildrum001.mdl".to_string(),
            "props_c17/oildrum001_explosive.mdl".to_string(),
        ],
    );
    folders.insert(
        "props_junk".to_string(),
        vec![
            "props_junk/wood_crate001a.mdl".to_string(),
            "props_junk/wood_crate002a.mdl".to_string(),
            "props_junk/popcan01a.mdl".to_string(),
        ],
    );
    o.set_props(folders);
    o.click(PanelTarget::Filter);
    assert!(o.typing && o.tab == Tab::Props);
    o.type_text("Crate!");
    assert_eq!(o.filter, "crate");
    let panel = o.panel(true, false);
    assert_eq!(panel.classes[0].text, "* matches \"crate\"/ (2)");
    assert_eq!(panel.classes[1].text, "props_junk/ (2)");
    assert_eq!(panel.tabs[2].text, "filter: crate_");
    o.key(OverlayKey::Backspace);
    o.type_text("e");
    assert_eq!(o.filter, "crate");
    // Enter ends typing in the models column; click a model, again to spawn.
    o.key(OverlayKey::Enter);
    assert!(!o.typing);
    assert_eq!(o.click(PanelTarget::Row(Column::Options, 1)), None);
    assert_eq!(
        o.click(PanelTarget::Row(Column::Options, 1)),
        Some(OverlayCommand::SpawnProp(
            "props_junk/wood_crate002a.mdl".into()
        ))
    );
    // Without a filter the common folder lists the installed common props.
    o.click(PanelTarget::Filter);
    for _ in 0..5 {
        o.key(OverlayKey::Backspace);
    }
    o.key(OverlayKey::Filter);
    let panel = o.panel(true, false);
    assert_eq!(panel.classes[0].text, "* common/ (5)");
    // Common props are found by their plain names too.
    o.key(OverlayKey::Filter);
    o.type_text("explosive barrel");
    let panel = o.panel(true, false);
    assert_eq!(
        panel.options[0].text,
        "props_c17/oildrum001_explosive  (explosive barrel)"
    );
    o.key(OverlayKey::Enter);
    // Escape stops typing first, then closes the overlay.
    o.key(OverlayKey::Filter);
    assert!(o.escape() && o.open && !o.typing);
    assert!(o.escape() && !o.open);
    assert!(!o.escape());
}
fn usable() -> std::collections::BTreeMap<String, Vec<String>> {
    [
        (
            "npc_metropolice",
            &["weapon_stunstick", "weapon_pistol", "weapon_smg1"][..],
        ),
        ("npc_combine_s", &["weapon_smg1", "weapon_ar2"][..]),
    ]
    .into_iter()
    .map(|(c, w)| (c.to_owned(), w.iter().map(|w| (*w).to_owned()).collect()))
    .collect()
}
#[test]
fn overlay_columns_offer_only_usable_weapons_and_keep_the_selection_visible() {
    let mut o = Overlay::default();
    o.set_usable(usable());
    assert_eq!(o.key(OverlayKey::Up), None, "closed: keys do nothing");
    o.key(OverlayKey::Toggle);
    // The cop starts with its default weapon; the column lists none + usable + rows.
    assert_eq!(o.request().equipment, "weapon_stunstick");
    let panel = o.panel(false, false);
    let texts: Vec<&str> = panel.options.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "no weapon",
            "weapon_stunstick (HL2 maps)",
            "weapon_pistol (HL2 maps)",
            "weapon_smg1 (HL2 maps)",
            "allegiance: class default",
            "notarget: off",
            "AI: OFF (idle)",
            "player hurts friendlies: off (debug)"
        ]
    );
    assert!(
        panel
            .classes
            .iter()
            .any(|r| r.cursor && r.text == "npc_metropolice")
    );
    // Options column: Down moves onto the pistol and chooses it.
    o.key(OverlayKey::Right);
    o.key(OverlayKey::Down);
    assert_eq!(o.request().equipment, "weapon_pistol");
    // Enter on allegiance cycles it; on notarget/AI it asks the host.
    o.key(OverlayKey::Down);
    o.key(OverlayKey::Down);
    assert_eq!(o.key(OverlayKey::Enter), None);
    assert_eq!(o.allegiance, Allegiance::Foe);
    o.key(OverlayKey::Down);
    assert_eq!(
        o.key(OverlayKey::Enter),
        Some(OverlayCommand::ToggleNotarget)
    );
    o.key(OverlayKey::Down);
    assert_eq!(o.key(OverlayKey::Enter), Some(OverlayCommand::ToggleAi));
    o.key(OverlayKey::Down);
    assert_eq!(
        o.key(OverlayKey::Enter),
        Some(OverlayCommand::ToggleFriendlyFire)
    );
    // On a weapon row (wrapping down to "no weapon"), Enter spawns with the choices.
    o.key(OverlayKey::Down);
    let Some(OverlayCommand::Spawn(r)) = o.key(OverlayKey::Enter) else {
        panic!()
    };
    assert_eq!(
        (
            r.classname.as_str(),
            r.equipment.as_str(),
            r.allegiance,
            r.aimed
        ),
        ("npc_metropolice", "", Allegiance::Foe, true)
    );
    // A zombie: no weapon column, natural attacks only.
    o.key(OverlayKey::Left);
    let zombie = SPAWN_CLASSES
        .iter()
        .position(|c| *c == "npc_zombie")
        .unwrap();
    for _ in 0..zombie {
        o.key(OverlayKey::Down);
    }
    let panel = o.panel(true, true);
    assert_eq!(panel.options[0].text, "no weapons (natural attacks)");
    assert_eq!(panel.options.len(), 5);
    assert_eq!(o.request().equipment, "");
    // The class list scrolls: the selection is always one of the drawn rows.
    o.key(OverlayKey::Up);
    for _ in 0..entry_count() {
        o.key(OverlayKey::Down);
        let panel = o.panel(true, false);
        assert!(panel.classes.len() <= VISIBLE_CLASSES + 2);
        let text = SPAWN_CLASSES[o.class].to_owned();
        assert!(
            panel.classes.iter().any(|r| r.selected && r.text == text),
            "{panel:?}"
        );
    }
    // Wrapping up from the first class reaches the last NPC class, with "more above".
    while o.class != 0 {
        o.key(OverlayKey::Down);
    }
    o.key(OverlayKey::Up);
    let panel = o.panel(false, false);
    assert!(panel.classes[0].text.contains("more above"));
    assert_eq!(o.class, entry_count() - 1);
    assert_eq!(o.request().classname, "npc_combinedropship");
    // Combine soldiers have no stunstick: their default SMG1 is preselected.
    while SPAWN_CLASSES[o.class] != "npc_combine_s" {
        o.key(OverlayKey::Down);
    }
    assert_eq!(o.request().equipment, "weapon_smg1");
    assert_eq!(o.request().grenades, Some(0));
    // Their options add NumGrenades after the allegiance; Enter cycles the count.
    let texts: Vec<String> = o
        .panel(true, false)
        .options
        .into_iter()
        .map(|r| r.text)
        .collect();
    assert_eq!(texts[3], "allegiance: foe (hates the player)");
    assert_eq!(texts[4], "grenades: 0 (NumGrenades)");
    o.key(OverlayKey::Right);
    for _ in 0..3 {
        o.key(OverlayKey::Down);
    }
    for expected in [1, 2, 3, 5, 10, 99, 0, 1] {
        assert_eq!(o.key(OverlayKey::Enter), None);
        assert_eq!(o.grenades, expected);
    }
    assert_eq!(o.request().grenades, Some(1));
    // Classes that never read NumGrenades neither show nor send it.
    o.key(OverlayKey::Left);
    o.key(OverlayKey::Up);
    assert!(
        o.panel(true, false)
            .options
            .iter()
            .all(|r| !r.text.starts_with("grenades"))
    );
    assert_eq!(o.request().grenades, None);
}
