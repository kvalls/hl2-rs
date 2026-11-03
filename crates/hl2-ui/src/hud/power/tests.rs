use super::{Bar, PowerState};
use crate::hud::{
    animations, event_rules, NumericHudEffects, NumericPanel, NumericProperty, WeaponHud,
};
use glam::vec2;
use hl2_simulation::{
    flashlight::{Context, Flashlight},
    gameplay::Inventory,
};
use source_assets::{keyvalues, vpk::Vfs};
use source_ui::canvas::Canvas;

fn entry(source: &[u8]) -> keyvalues::Entry {
    keyvalues::parse_resource(source).unwrap().remove(0)
}

#[test]
fn chunk_rounding_drives_low_power_at_chunk_boundaries() {
    let bar = Bar::read(&entry(b"Bar {}"), false);
    assert_eq!(bar.chunks(100.), (10, 10));
    assert_eq!(bar.chunks(24.99), (10, 2));
    assert_eq!(bar.chunks(25.), (10, 3));
    assert_eq!(bar.chunks(-1.), (10, 0));
    assert_eq!(bar.chunks(110.), (10, 10));
    let empty = Bar::read(
        &entry(br#"Bar { BarChunkWidth "0" BarChunkGap "0" }"#),
        false,
    );
    assert_eq!(empty.chunks(100.), (0, 0));
}

#[test]
fn charge_device_and_low_events_fire_once_per_transition() {
    let mut state = PowerState::default();
    assert_eq!(
        state.transitions(100., false, false, false),
        [Some("SuitAuxPowerMax"), None, None]
    );
    assert_eq!(
        state.transitions(99., true, false, false),
        [
            Some("SuitAuxPowerNotMax"),
            Some("SuitAuxPowerOneItemActive"),
            Some("SuitAuxPowerIncreasedAbove25")
        ]
    );
    assert_eq!(state.transitions(90., true, false, false), [None; 3]);
    assert_eq!(
        state.transitions(24., true, true, false)[2],
        Some("SuitAuxPowerDecreasedBelow25")
    );
    assert_eq!(
        state.transitions(23., false, true, false)[1],
        Some("SuitAuxPowerNoItemsActive")
    );
    assert_eq!(
        state.transitions(30., false, false, false)[2],
        Some("SuitAuxPowerIncreasedAbove25")
    );
    assert_eq!(
        state.transitions(100., false, false, false)[0],
        Some("SuitAuxPowerMax")
    );
    assert_eq!(
        state.transitions(100., false, false, true)[1],
        Some("SuitAuxPowerNoItemsActive")
    );
}

#[test]
fn auxiliary_colour_and_geometry_use_the_shared_animation_controller() {
    let settings = entry(br#"Settings { BgColor "0 0 0 76" }"#);
    let source = "event Test\n{\nAnimate HudSuitPower AuxPowerColor \"20 40 60 0\" Linear 0 1\nAnimate HudSuitPower Position \"16 390\" Linear 0 1\nAnimate HudSuitPower Size \"102 36\" Linear 0 1\n}";
    let mut effects = NumericHudEffects::new(&settings);
    effects.viewport = vec2(1280., 960.);
    let panel = NumericPanel::SuitPower as usize;
    effects.start(
        "Test",
        0.,
        &animations(source).unwrap(),
        &event_rules(source).unwrap(),
        &settings,
    );
    effects.advance(0.5);
    let tint = effects.panels[panel].color(NumericProperty::AuxPowerColor);
    assert!((tint.a - 127. / 255.).abs() < 0.001);
    effects.advance(1.);
    let rect = effects.panels[panel].rect().unwrap();
    assert_eq!([rect.x, rect.y, rect.w, rect.h], [32., 780., 204., 72.]);
    assert_eq!(
        effects.panels[panel]
            .color(NumericProperty::AuxPowerColor)
            .a,
        0.
    );
    assert!(effects.animations.is_empty());
}

fn installed(game: &str) -> WeaponHud {
    let root = source_assets::install::discover().unwrap();
    let vfs = Vfs::mount_game(&root, game).unwrap();
    let canvas = Canvas::default();
    canvas.resize(1920., 1080.);
    WeaponHud::load(&vfs, canvas).unwrap()
}
fn draw(hud: &WeaponHud, light: &Flashlight, inv: &Inventory, time: f64) {
    hud.draw_status(inv, &Default::default(), &Default::default(), time);
    hud.canvas.drain();
    hud.draw_power(inv, Some(light), time);
}

#[test]
#[ignore = "requires an owned installed Half-Life 2 copy"]
fn installed_auxiliary_layout_localization_fades_and_visibility() {
    let hud = installed("hl2");
    let mut light = Flashlight::new(true);
    let mut inv = Inventory::default();
    inv.suit = true;
    inv.health = 100.;
    draw(&hud, &light, &inv, 0.);
    draw(&hud, &light, &inv, 1.);
    assert!(hud.canvas.drain().is_empty(), "full power has faded away");
    light.on = true;
    light.suit_power.power = 90.;
    draw(&hud, &light, &inv, 2.);
    draw(&hud, &light, &inv, 3.);
    let report = hud.power_report();
    assert_eq!(report["device"], true);
    assert_eq!(report["rect"], serde_json::json!([36., 877., 229., 81.]));
    assert!(!hud.canvas.drain().is_empty());
    light.suit_power.power = 24.;
    draw(&hud, &light, &inv, 4.);
    draw(&hud, &light, &inv, 5.);
    assert_eq!(hud.power_report()["low"], true);
    assert_eq!(hud.power_report()["color"][1], 0.);
    hud.canvas.drain();
    inv.suit = false;
    draw(&hud, &light, &inv, 6.);
    assert!(hud.canvas.drain().is_empty());
    inv.suit = true;
    inv.health = 0.;
    draw(&hud, &light, &inv, 7.);
    assert!(hud.canvas.drain().is_empty());
    inv.health = 100.;
    hud.canvas.resize(640., 960.);
    draw(&hud, &light, &inv, 8.);
    draw(&hud, &light, &inv, 9.);
    assert_eq!(
        hud.power_report()["rect"],
        serde_json::json!([17., 780., 108., 38.])
    );
    assert!(
        !hud.canvas.drain().is_empty(),
        "prepared text also draws after a portrait resize"
    );
}

#[test]
#[ignore = "requires an owned installed Half-Life 2 copy"]
fn installed_episode_two_battery_is_independent_and_draws_both_states() {
    let hud = installed("ep2");
    let mut inv = Inventory::default();
    inv.suit = true;
    inv.health = 100.;
    let mut light = Flashlight::new(false);
    let context = Context {
        suit: true,
        now: 1.,
        ..Default::default()
    };
    draw(&hud, &light, &inv, 0.);
    let off = hud.canvas.drain();
    assert!(!off.is_empty(), "charged off glyph");
    light.toggle(context);
    light.update(1., context);
    draw(&hud, &light, &inv, 1.);
    let on = hud.canvas.drain();
    assert!(on.len() > off.len(), "on state also draws battery chunks");
    assert_eq!(hud.power_report()["device"], false);
    assert_eq!(light.suit_power.power, 100.);
    assert_eq!(
        hud.power.battery_bar.chunks(100.).0,
        9,
        "owned chunk gap overrides the SDK default"
    );
    // The same charge and device state must not restart animation tracks each frame.
    draw(&hud, &light, &inv, 2.);
    draw(&hud, &light, &inv, 3.);
    assert!(hud.numeric_effects.borrow().animations.is_empty());
}
