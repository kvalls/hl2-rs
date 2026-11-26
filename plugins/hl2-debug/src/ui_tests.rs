//! Tests for background visual debug console commands and opacity/FOV controls.

use crate::commands::Request;
use hl2_ui::console::{Console, Effect, Mode};
pub(super) fn console() -> Console {
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
    use source_ui::console::panel::Level;

    let mut c = console();
    let before = c.shell.panel.output.len();
    assert!(c.execute("agent_status thinking").is_empty());
    assert_eq!(c.shell.panel.output.len(), before + 1);
    assert_eq!(
        c.shell
            .panel
            .output
            .back()
            .map(|l| (l.level, l.text.as_str())),
        Some((Level::ChatRemote, "Console: Thinking..."))
    );
    c.log("rcon from 127.0.0.1");
    assert_eq!(
        c.shell.panel.output.back().map(|l| l.text.as_str()),
        Some("Console: Thinking..."),
        "status stays pinned below ordinary log output"
    );
    c.execute("agent_status waiting");
    assert_eq!(c.shell.panel.output.len(), before + 2);
    assert_eq!(
        c.shell
            .panel
            .output
            .back()
            .map(|l| (l.level, l.text.as_str())),
        Some((Level::ChatRemote, "Console: Still waiting for a reply..."))
    );
    c.execute("agent_status Working...");
    assert_eq!(
        c.shell
            .panel
            .output
            .back()
            .map(|l| (l.level, l.text.as_str())),
        Some((Level::ChatRemote, "Console: Working..."))
    );
    c.execute("agent_status \"Building release...\"");
    assert_eq!(
        c.shell
            .panel
            .output
            .back()
            .map(|l| (l.level, l.text.as_str())),
        Some((Level::ChatRemote, "Console: Building release..."))
    );
    c.execute("agent_status Spawning NPCs...");
    assert_eq!(
        c.shell
            .panel
            .output
            .back()
            .map(|l| (l.level, l.text.as_str())),
        Some((Level::ChatRemote, "Console: Spawning NPCs..."))
    );
    assert!(c.execute("agent_status clear").is_empty());
    assert_eq!(c.shell.panel.output.len(), before + 1);
    c.execute("agent_status; agent_status \"\"; agent_status \"   \"");
    assert_eq!(
        c.shell.panel.output.back().map(|l| l.level),
        Some(Level::Normal)
    );
    c.remote.from_rcon = true;
    assert_eq!(
        c.execute("agent_status thinking"),
        vec![effect(Request::AgentCaption("Console: Thinking...".into()))]
    );
    assert_eq!(
        c.remote.snapshot,
        Some(Some((Level::ChatRemote, "Console: Thinking...".into())))
    );
    c.execute("agent_status \"Running tests...\"");
    assert_eq!(
        c.remote.snapshot,
        Some(Some((
            Level::ChatRemote,
            "Console: Running tests...".into()
        )))
    );
    c.execute("say hello");
    assert_eq!(c.remote.snapshot, Some(None));
    assert_eq!(c.remote.broadcast, ["Console: hello"]);
}

#[test]
fn agent_status_is_not_logged_from_the_remote_console() {
    let console = console();
    for line in [
        "agent_status thinking",
        "agent_status \"Working...\"",
        "agent_status Spawning NPCs...",
    ] {
        let commands = source_ui::console::command::tokenize(line);
        assert!(console.remote_log_skips(&commands), "{line}");
    }
}

#[test]
fn map_dump_accepts_coordinates_and_pages_only_with_debug_tools() {
    use crate::commands::MapDumpQuery;
    let mut c = console();
    for (command, query) in [
        ("map_dump", MapDumpQuery::Page(0)),
        ("map_dump page 12", MapDumpQuery::Page(12)),
        (
            "map_dump near 158 -1258 -738 200",
            MapDumpQuery::Near {
                point: [158., -1258., -738.],
                radius: 200.,
            },
        ),
    ] {
        assert_eq!(c.submit(command), vec![effect(Request::MapDump(query))]);
        assert!(Console::default().submit(command).is_empty());
    }
    for bad in [
        "map_dump page -1",
        "map_dump near 1 2 3 0",
        "map_dump near NaN 0 0 1",
        "map_dump near 1 2",
    ] {
        assert!(c.submit(bad).is_empty());
    }
}

#[test]
fn spawn_at_preserves_coordinates_and_only_accepts_known_options() {
    let mut console = console();
    assert_eq!(
        console.submit("spawn_at npc_citizen_rebel pos -4310 84 -57 targetname scout additionalequipment weapon_smg1"),
        vec![effect(Request::SpawnAt(crate::commands::SpawnAt {
            class: "npc_citizen_rebel".into(),
            origin: [-4310., 84., -57.],
            name: Some("scout".into()),
            model: None,
            equipment: Some("weapon_smg1".into()),
        }))]
    );
    for command in [
        "spawn_at item_battery pos -4300 nan -57",
        "spawn_at item_battery pos 1 2",
        "spawn_at item_battery pos 1 2 3 bogus 4",
        "spawn_at item_battery pos 1 2 3 targetname one targetname two",
    ] {
        assert!(console.submit(command).is_empty(), "{command}");
    }
}
