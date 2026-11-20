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

#[test]
fn ai_enable_comes_from_the_debug_plugin() {
    let mut console = console();
    assert_eq!(
        console.submit("ai_enable 0"),
        vec![effect(Request::AiEnable(Some(false)))]
    );
    assert_eq!(
        console.submit("ai_enable"),
        vec![effect(Request::AiEnable(None))]
    );
    assert!(console.submit("ai_enable 2").is_empty());
    assert!(
        Console::default().submit("ai_enable 0").is_empty(),
        "core has no ai_enable"
    );
}

#[test]
fn npc_dump_parses_default_radius_and_entity_id() {
    use crate::commands::{DEFAULT_NPC_DUMP_RADIUS, NpcDumpQuery};
    let mut console = console();
    assert_eq!(
        console.submit("npc_dump"),
        vec![effect(Request::NpcDump(NpcDumpQuery::Nearby(
            DEFAULT_NPC_DUMP_RADIUS
        )))]
    );
    assert_eq!(
        console.submit("npc_dump 250"),
        vec![effect(Request::NpcDump(NpcDumpQuery::Nearby(250.0)))]
    );
    assert_eq!(
        console.submit("npc_dump id 1297"),
        vec![effect(Request::NpcDump(NpcDumpQuery::Entity(1297)))]
    );
    assert!(
        console
            .submit("npc_dump 0; npc_dump -5; npc_dump foo; npc_dump id abc")
            .is_empty()
    );
    assert!(
        Console::default().submit("npc_dump").is_empty(),
        "core has no npc_dump"
    );
}

#[test]
fn remote_reply_clears_status_without_printing_it() {
    let mut c = console();
    let before = c.shell.panel.output.len();
    c.execute("agent_status thinking");
    assert_eq!(c.shell.panel.status.as_deref(), Some("Thinking..."));
    c.execute("agent_status waiting");
    assert_eq!(
        c.shell.panel.status.as_deref(),
        Some("Still waiting for a reply...")
    );
    assert_eq!(c.shell.panel.output.len(), before);
    c.remote.from_rcon = true;
    c.execute("say hello");
    assert!(c.shell.panel.status.is_none());
    assert_eq!(c.remote.broadcast, ["Console: hello"]);
}
