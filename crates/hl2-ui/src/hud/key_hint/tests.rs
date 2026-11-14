use super::super::{animations, event_rules, NumericHudEffects, NumericPanel, NumericProperty};
use super::*;
use source_assets::keyvalues;

/// The installed [$WIN32] Valve_Hint_Movement text and a USE hint (hl2/resource/valve_english.txt).
fn localization() -> Localization {
    let mut loc = Localization::default();
    loc.insert(
        "Valve_Hint_Movement",
        "FORWARD %+forward%BACK %+back%LEFT %+moveleft%RIGHT %+moveright%",
    );
    loc.insert("Valve_Hint_USEPickup", "%+use% PICKUP OBJECT");
    loc
}

fn keys() -> BTreeMap<String, String> {
    [
        ("+forward", "KeyW"),
        ("+back", "KeyS"),
        ("+moveleft", "KeyA"),
        ("+moveright", "KeyD"),
        ("+use", "KeyE"),
        ("+jump", ""),
    ]
    .into_iter()
    .map(|(a, k)| (a.to_owned(), k.to_owned()))
    .collect()
}

fn label(text: &str, key: bool) -> HintLabel {
    HintLabel {
        text: text.into(),
        key,
    }
}

#[test]
fn the_movement_hint_alternates_text_and_bound_keys() {
    let labels = labels("Valve_Hint_Movement", &localization(), &keys()).unwrap();
    let expected = [
        ("FORWARD ", false),
        ("W", true),
        ("BACK ", false),
        ("S", true),
        ("LEFT ", false),
        ("A", true),
        ("RIGHT ", false),
        ("D", true),
    ];
    assert_eq!(
        labels,
        expected.map(|(t, k)| label(t, k)).to_vec(),
        "native: FORWARD W / BACK S / LEFT A / RIGHT D"
    );
}

#[test]
fn a_hint_can_start_with_its_key_and_unbound_commands_say_so() {
    let loc = localization();
    let keys = keys();
    assert_eq!(
        labels("#Valve_Hint_USEPickup", &loc, &keys).unwrap(),
        vec![label("E", true), label(" PICKUP OBJECT", false)]
    );
    assert_eq!(
        labels("%+jump% JUMP", &loc, &keys).unwrap()[0],
        label("< NOT BOUND >", true),
        "an explicitly unbound command"
    );
    assert_eq!(
        labels("%+JUMP%", &loc, &BTreeMap::new()).unwrap(),
        vec![label("< NOT BOUND >", true)]
    );
}

#[test]
fn an_empty_message_or_a_missing_hash_token_hides_and_raw_text_stays() {
    let (loc, keys) = (localization(), keys());
    assert_eq!(labels("", &loc, &keys), None);
    assert_eq!(labels("#Valve_Hint_Missing", &loc, &keys), None);
    assert_eq!(
        labels("plain text", &loc, &keys).unwrap(),
        vec![label("plain text", false)]
    );
}

#[test]
fn the_panel_keeps_its_right_and_bottom_edge_and_fits_the_rows() {
    let resource = keyvalues::parse_resource(
        br#"HudHintKeyDisplay { xpos r120 ypos r340 wide 100 tall 200 text_xpos 8 text_ypos 8 text_xgap 8 text_ygap 8 }"#,
    )
    .unwrap();
    let layout = Layout::read(resource.first());
    // 720p: text 16.5 px Verdana labels 74 wide and 17 high, keys 17 x 22 (native frames).
    let sizes = [vec2(74., 17.), vec2(17., 22.)].repeat(4);
    let (panel, origins) = layout.place(&sizes, vec2(1280., 720.));
    // Right edge 1100 + 150, bottom 210 + 300: native's text starts at x 1135, keys at 1221.
    assert_eq!(panel, Rect::new(1123., 362., 127., 148.));
    assert_eq!(origins[0], vec2(12., 14.));
    assert_eq!(origins[1], vec2(98., 12.));
    assert_eq!(origins[7], vec2(98., 12. + 3. * 34.));
}

#[test]
fn key_hint_events_fade_the_panel_in_flash_it_and_fade_it_out() {
    // hl2/scripts/hudanimations.txt KeyHintMessageShow / KeyHintMessageHide.
    let source = "event KeyHintMessageShow\n{\nStopPanelAnimations HudHintKeyDisplay 0.0\nAnimate HudHintKeyDisplay Alpha 255 Linear 0.0 0.5\nAnimate HudHintKeyDisplay FgColor \"FgColor\" Linear 0.0 0.01\nAnimate HudHintKeyDisplay FgColor \"255 220 0 255\" Linear 0.5 0.2\nAnimate HudHintKeyDisplay FgColor \"FgColor\" Linear 0.7 0.2\nAnimate HudHintKeyDisplay Alpha 0 Linear 12.0 1.0\n}\nevent KeyHintMessageHide\n{\nAnimate HudHintKeyDisplay Alpha 0 Linear 0.0 0.5\n}";
    let (rules, events) = (animations(source).unwrap(), event_rules(source).unwrap());
    let settings = keyvalues::parse_resource(
        br#"BaseSettings { FgColor "255 220 0 100" BgColor "0 0 0 76" }"#,
    )
    .unwrap()
    .remove(0);
    let mut effects = NumericHudEffects::new(&settings);
    let panel = NumericPanel::HintKeyDisplay as usize;
    let alpha = |e: &NumericHudEffects| e.panels[panel].values[NumericProperty::Alpha as usize][0];
    let fg =
        |e: &NumericHudEffects| e.panels[panel].values[NumericProperty::Foreground as usize][3];
    assert_eq!(alpha(&effects), 0., "hidden until a hint arrives");
    effects.start("KeyHintMessageShow", 10., &rules, &events, &settings);
    effects.advance(10.25);
    assert_eq!(alpha(&effects), 127.5);
    effects.advance(10.7);
    assert_eq!((alpha(&effects), fg(&effects)), (255., 255.), "the flash");
    effects.advance(11.);
    assert_eq!(fg(&effects), 100.);
    effects.advance(22.5);
    assert_eq!(alpha(&effects), 127.5);
    effects.advance(23.);
    assert_eq!(alpha(&effects), 0.);
    effects.start("KeyHintMessageShow", 30., &rules, &events, &settings);
    effects.advance(31.);
    effects.start("KeyHintMessageHide", 31., &rules, &events, &settings);
    effects.advance(31.25);
    assert_eq!(alpha(&effects), 127.5);
}

#[test]
#[ignore = "requires an owned Half-Life 2 installation"]
fn owned_movement_hint_draws_at_the_native_right_edge() {
    use hl2_simulation::gameplay::Inventory;
    let root = source_assets::install::discover().expect("installed game");
    let vfs = Vfs::mount(&root).unwrap();
    let canvas = Canvas::default();
    canvas.resize(1280., 720.);
    let hud = WeaponHud::load(&vfs, canvas.clone()).unwrap();
    let hint = ("Valve_Hint_Movement".to_owned(), 1.);
    hud.set_key_hint(Some(&hint), &keys());
    let inventory = Inventory::default();
    let weapons = BTreeMap::new();
    let selection = Default::default();
    hud.draw_status(&inventory, &weapons, &selection, 1.);
    hud.draw_status(&inventory, &weapons, &selection, 2.);
    assert!(!canvas.drain().is_empty(), "the panel draws without a suit");
    let state = hud.key_hint.state.borrow();
    assert_eq!(state.labels[1], label("W", true));
    let (_, panel, origins) = state.placed.clone().unwrap();
    // Native: right edge 1250, bottom 510, rows 34 px apart (Verdana bold 22 px high).
    assert_eq!((panel.x + panel.w, panel.y + panel.h), (1250., 510.));
    assert_eq!(origins[3].y - origins[1].y, 34.);
}
