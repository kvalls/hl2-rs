use super::*;
use crate::gameui::localize::Localization;
use crate::gameui::menu::MenuCommand;
use crate::gameui::resources::{MenuItem, Visibility};
use crate::proportional;
use source_ui::canvas::Rect;
use source_ui::console::dialog::{
    DialogLayout, ENTRY_TALL, MIN_SIZE, PROPORTIONAL_BASE, SUBMIT_WIDE,
};
use source_ui::console::panel::{
    HistoryView, Level, Line, CONSOLE_ROW, NOMINAL_ADVANCE, OUTPUT_LINES, WHEEL_ROWS,
};
use source_ui::input::{Input, Key};
use source_ui::placement::CursorHint;
use source_ui::shell;
use source_ui::{widgets::ScrollMode, windows::Layer};
#[test]
fn cell_metrics_rejects_missing_font() {
    assert!(CellMetrics::from_bytes(b"missing font").is_none());
}
#[test]
fn cheats_are_gated_in_order_and_queries_do_not_change_them() {
    let mut console = Console::default();
    assert!(console
        .submit("impulse 101; noclip; setpos 1 2 3; setang 0 0 0")
        .is_empty());
    assert_eq!(
        console.submit("sv_cheats 1; impulse 101; noclip 1"),
        vec![Effect::Loadout, Effect::Noclip(Some(true))]
    );
    assert!(console.submit("sv_cheats").is_empty());
    assert!(console.cheats);
    assert!(console.submit("sv_cheats 0; impulse 101").is_empty());
    assert!(!console.cheats);
}
#[test]
fn npc_create_is_a_cheat_but_its_equipment_cvar_is_not() {
    let mut console = Console::default();
    assert_eq!(
        console.submit("npc_create npc_metropolice; npc_create_equipment weapon_stunstick"),
        vec![Effect::NpcCommand(
            "npc_create_equipment weapon_stunstick".into()
        )]
    );
    assert_eq!(
        console.submit("sv_cheats 1; npc_create_aimed npc_metropolice cop1"),
        vec![Effect::NpcCommand(
            "npc_create_aimed npc_metropolice cop1".into()
        )]
    );
}
#[test]
fn prop_physics_create_and_ent_create_are_cheats_routed_with_npc_create() {
    let mut console = Console::default();
    assert_eq!(
        console.submit("prop_physics_create props_c17/oildrum001"),
        vec![]
    );
    assert_eq!(
            console.submit("sv_cheats 1; prop_physics_create props_c17/oildrum001; ent_create npc_metropolice targetname cop9"),
            vec![
                Effect::NpcCommand("prop_physics_create props_c17/oildrum001".into()),
                Effect::NpcCommand("ent_create npc_metropolice targetname cop9".into()),
            ]
        );
    // Quoted values keep their spaces for the spawn parser.
    assert_eq!(
        console.submit("ent_create npc_citizen angles \"0 180 0\""),
        vec![Effect::NpcCommand(
            "ent_create npc_citizen angles \"0 180 0\"".into()
        )]
    );
}
#[test]
fn ai_disable_is_a_cheat_toggle_without_arguments() {
    let mut console = Console::default();
    assert_eq!(console.submit("ai_disable"), vec![]);
    assert_eq!(
        console.submit("sv_cheats 1; ai_disable; ai_disable 1"),
        vec![Effect::AiDisable]
    );
}
#[test]
fn quoted_separators_are_text_and_bad_quotes_execute_nothing() {
    let mut console = Console::default();
    console.submit("echo \"hello; world\"; echo done");
    assert!(console
        .shell
        .panel
        .output
        .iter()
        .any(|line| line == "hello; world"));
    assert!(console.shell.panel.output.iter().any(|line| line == "done"));
    console.submit("sv_cheats 1; echo \"unfinished");
    assert!(!console.cheats);
}
#[test]
fn invalid_numbers_paths_impulses_and_unknown_commands_do_not_succeed() {
    let mut console = Console::default();
    console.submit("sv_cheats 1");
    for line in [
        "setpos NaN 0 0",
        "setpos inf 0 0",
        "setpos 1",
        "setang 0 0 20",
        "map ../other",
        "map a/b",
        "map C:\\foo",
        "impulse 42",
        "god",
        "sv_cheats 2",
    ] {
        assert!(console.submit(line).is_empty(), "{line}");
    }
    assert_eq!(
        console.submit("map d1_trainstation_02.bsp"),
        vec![Effect::Map("d1_trainstation_02".into())]
    );
    assert_eq!(
        console.submit("setpos -10 20 30"),
        vec![Effect::Setpos {
            x: -10.,
            y: 20.,
            z: Some(30.)
        }]
    );
    assert_eq!(
        console.submit("setpos 1 2"),
        vec![Effect::Setpos {
            x: 1.,
            y: 2.,
            z: None
        }]
    );
}

#[test]
fn entity_inputs_preserve_quoted_values_and_enforce_cheat_and_delay_rules() {
    let mut console = Console::default();
    assert!(console.submit("ent_fire security03 Start").is_empty());
    assert_eq!(
        console.submit("sv_cheats 1; ent_fire security_* Start \"value;kept\" 0.1"),
        vec![Effect::Fire {
            target: "security_*".into(),
            input: "Start".into(),
            parameter: "value;kept".into(),
            delay: 0.,
        }]
    );
    assert_eq!(
        console.submit("ent_fire panel"),
        vec![Effect::Fire {
            target: "panel".into(),
            input: "Use".into(),
            parameter: String::new(),
            delay: 0.,
        }]
    );
    for command in [
        "ent_fire",
        "ent_fire door Open x NaN",
        "ent_fire door Open x -1",
        "ent_fire door Open x inf",
    ] {
        assert!(console.submit(command).is_empty());
    }
}
#[test]
fn console_returns_to_its_previous_pause_state_and_steps_through_history() {
    let mut console = Console::default();
    console.escape();
    assert!(console.paused());
    console.toggle();
    assert_eq!(console.mode(), Mode::Console);
    console.escape();
    assert_eq!(console.mode(), Mode::Pause);
    console.toggle();
    for line in ["echo first", "echo second"] {
        console.input(&Input {
            text: line.into(),
            keys: [Key::Enter].into(),
            ..Default::default()
        });
    }
    // An empty line: Up and Down step through the history, newest first.
    keys(&mut console, &[Key::Up]);
    assert_eq!(console.input_line(), "echo second");
    keys(&mut console, &[Key::Up]);
    assert_eq!(console.input_line(), "echo first");
    keys(&mut console, &[Key::Down]);
    assert_eq!(console.input_line(), "echo second");
    // Escape hides the list, then closes the console.
    console.escape();
    assert_eq!(console.mode(), Mode::Console);
    console.escape();
    assert_eq!(console.mode(), Mode::Pause);
    assert_eq!(
        console.take_changed_history(),
        Some(vec!["echo first".to_owned(), "echo second".to_owned()])
    );
    assert_eq!(console.take_changed_history(), None);
}
#[test]
fn explicit_text_input_edits_unicode_at_boundaries_and_completes_commands() {
    let mut console = Console::default();
    console.toggle();
    console.input(&Input {
        text: "aé🦀".into(),
        ..Default::default()
    });
    keys(&mut console, &[Key::Left]);
    keys(&mut console, &[Key::Backspace]);
    assert_eq!(console.input_line(), "a🦀");
    keys(&mut console, &[Key::Delete]);
    assert_eq!(console.input_line(), "a");
    keys(&mut console, &[Key::Backspace]);
    console.input(&Input {
        text: "noc".into(),
        keys: [Key::Tab].into(),
        ..Default::default()
    });
    assert_eq!(
        console.input_line(),
        "noclip ",
        "one match: taken with a space"
    );
    keys(&mut console, &[Key::Enter]);
    assert!(console.input_line().is_empty());
    assert!(console
        .shell
        .panel
        .output
        .iter()
        .any(|s| s.contains("sv_cheats")));
}
#[test]
fn console_shortcuts_search_paste_filter_and_timestamps() {
    let mut console = Console::default();
    console.toggle();
    console.set_history(vec!["fov 90".into(), "volume 0.5".into()]);
    console.input(&Input {
        text: "x".into(),
        ctrl: true,
        binding_keys: vec!["KeyR".into()],
        ..Default::default()
    });
    console.input(&Input {
        text: "fov".into(),
        ..Default::default()
    });
    assert_eq!(
        console.shell.panel.editor.search(),
        Some(("fov", Some("fov 90")))
    );
    console.escape();
    assert_eq!(
        console.mode(),
        Mode::Console,
        "Escape ends the search first"
    );
    console.input(&Input {
        paste: Some("echo pasted\n".into()),
        ..Default::default()
    });
    assert_eq!(console.input_line(), "echo pasted");
    console.input(&Input {
        ctrl: true,
        binding_keys: vec!["KeyU".into()],
        ..Default::default()
    });
    assert_eq!(console.input_line(), "");
    console.submit("con_filter_enable 1; con_filter_text Loaded; con_timestamp 1");
    assert_eq!(
        console.cvar_value("con_filter_enable").as_deref(),
        Some("1")
    );
    assert_eq!(
        console.cvar_value("con_filter_text").as_deref(),
        Some("Loaded")
    );
    assert!(console.shell.panel.con_timestamp);
    assert!(console.submit("con_filter_enable 3").is_empty());
    assert_eq!(
        console.cvar_value("con_filter_enable").as_deref(),
        Some("1")
    );
    let loaded = Line {
        level: Level::Normal,
        text: "Loaded hl2/background01".into(),
        time: 0.,
    };
    assert!(console.shell.panel.filter_match(&loaded));
    assert_eq!(
        serde_json::to_string(&loaded).unwrap(),
        "\"Loaded hl2/background01\"",
        "lines serialize as their text in the run report"
    );
    console.input(&Input {
        ctrl: true,
        binding_keys: vec!["KeyL".into()],
        ..Default::default()
    });
    assert!(
        console.shell.panel.output.is_empty(),
        "Ctrl+L clears the output"
    );
}
/// Canvas point at the middle of a frame-relative layout rectangle of the console dialog.
fn dialog_point(console: &Console, rect: Rect) -> Vec2 {
    let r = console
        .shell
        .panel
        .dialog_layout(&console.canvas)
        .canvas(rect);
    vec2(r.x + r.w * 0.5, r.y + r.h * 0.5)
}
/// Canvas point on the history row `row` (0 is the top row shown).
fn history_point(console: &Console, row: usize) -> Vec2 {
    let layout = console.shell.panel.dialog_layout(&console.canvas);
    let text = layout.canvas(HistoryView::new(&layout).text);
    vec2(
        text.x + 10.,
        text.y + (row as f32 + 0.5) * CONSOLE_ROW * layout.scale,
    )
}
fn press(console: &mut Console, at: Vec2) -> Vec<Effect> {
    console.input(&Input {
        pointer: Some(at),
        click: true,
        held: true,
        ..Default::default()
    })
}
fn release(console: &mut Console, at: Vec2) -> Vec<Effect> {
    console.input(&Input {
        pointer: Some(at),
        released: true,
        ..Default::default()
    })
}
/// Drags over the history from row `from` to row `to`.
fn drag(console: &mut Console, from: usize, to: usize) {
    press(console, history_point(console, from));
    let to = history_point(console, to);
    console.input(&Input {
        pointer: Some(to),
        held: true,
        ..Default::default()
    });
    release(console, to);
}
fn copy(console: &mut Console) -> Option<String> {
    console.input(&Input {
        copy: true,
        ..Default::default()
    });
    console.take_copied()
}

/// One frame at time `t` with `pressed` going down now and `held` keys down.
fn key_frame(t: f64, pressed: &[Key], held: &[Key]) -> Input {
    Input {
        keys: pressed.iter().copied().collect(),
        held_keys: held.iter().chain(pressed).copied().collect(),
        time: Some(t),
        ..Default::default()
    }
}

#[test]
fn held_keys_repeat_in_the_console_and_the_main_menu() {
    let mut console = Console::default();
    console.toggle();
    console.input(&Input {
        text: "abcdefghij".into(),
        ..Default::default()
    });
    console.input(&key_frame(0., &[Key::Backspace], &[]));
    for i in 1..=10 {
        console.input(&key_frame(i as f64 * 0.1, &[], &[Key::Backspace]));
    }
    // One press, then repeats at 0.4, 0.5, ... 1.0.
    assert_eq!(console.input_line(), "ab");
    console.input(&key_frame(1.1, &[], &[]));
    console.input(&key_frame(2.0, &[], &[Key::Backspace]));
    assert_eq!(
        console.input_line(),
        "ab",
        "released: no repeat until pressed again"
    );

    // Holding Down on the main menu walks it like three separate presses.
    let mut stepped = installed_console();
    stepped.level_loaded(true);
    for _ in 0..3 {
        keys(&mut stepped, &[Key::Down]);
    }
    let mut held = installed_console();
    held.level_loaded(true);
    held.input(&key_frame(0., &[Key::Down], &[]));
    for t in [0.2, 0.4, 0.45] {
        held.input(&key_frame(t, &[], &[Key::Down]));
    }
    assert!(stepped.main_menu.selection.is_some());
    assert_eq!(held.main_menu.selection, stepped.main_menu.selection);
}

/// Press at `from`, move held to `to`, release there.
fn pointer_drag(console: &mut Console, from: Vec2, to: Vec2) {
    press(console, from);
    console.input(&Input {
        pointer: Some(to),
        held: true,
        ..Default::default()
    });
    release(console, to);
}

#[test]
fn the_console_moves_by_its_caption_and_escape_undoes_a_drag() {
    let mut console = Console::default();
    console.canvas.resize(1024., 768.);
    console.toggle();
    let start = console.shell.panel.dialog_layout(&console.canvas);
    let caption = start.origin + vec2(100., 10.) * start.scale;
    pointer_drag(&mut console, caption, caption - vec2(200., 50.));
    let moved = console.shell.panel.dialog_layout(&console.canvas);
    assert_eq!(moved.origin, start.origin - vec2(200., 50.));
    assert_eq!(moved.size, start.size);
    // The caption press is the placement's: no history row selection started.
    assert_eq!(copy(&mut console), None);
    // Escape during a drag restores the frame and keeps the console open.
    let caption = moved.origin + vec2(100., 10.) * moved.scale;
    press(&mut console, caption);
    console.input(&Input {
        pointer: Some(caption + vec2(300., 300.)),
        held: true,
        ..Default::default()
    });
    console.escape();
    assert_eq!(console.mode(), Mode::Console);
    assert_eq!(
        console.shell.panel.dialog_layout(&console.canvas).origin,
        moved.origin
    );
    // Closing and reopening keeps the place.
    console.toggle();
    console.toggle();
    assert_eq!(
        console.shell.panel.dialog_layout(&console.canvas).origin,
        moved.origin
    );
}

#[test]
fn the_console_resizes_by_its_corners_within_limits() {
    let mut console = Console::default();
    console.canvas.resize(1024., 768.);
    console.toggle();
    let start = console.shell.panel.dialog_layout(&console.canvas);
    let corner = start.origin + start.size * start.scale - Vec2::splat(4.);
    console.shell.pointer = Some(corner);
    assert_eq!(console.cursor_hint(), CursorHint::ResizeNwSe);
    pointer_drag(&mut console, corner, corner + vec2(-100., -60.));
    let resized = console.shell.panel.dialog_layout(&console.canvas);
    assert_eq!(resized.origin, start.origin, "the top-left stays");
    assert!((resized.size.x - (start.size.x - 100. / start.scale)).abs() <= 1.);
    assert!(resized.entry.w > 0. && resized.history.h > 0.);
    // Way past the minimum: the layout keeps every control.
    let corner = resized.origin + resized.size * resized.scale - Vec2::splat(4.);
    pointer_drag(&mut console, corner, start.origin);
    assert_eq!(
        console.shell.panel.dialog_layout(&console.canvas).size,
        MIN_SIZE
    );
    // The top-left grip shows the same diagonal; the caption keeps the arrow.
    console.shell.pointer =
        Some(console.shell.panel.dialog_layout(&console.canvas).origin + Vec2::splat(2.));
    assert_eq!(console.cursor_hint(), CursorHint::ResizeNwSe);
    console.shell.pointer = Some(
        console.shell.panel.dialog_layout(&console.canvas).origin + vec2(60., 10.) * start.scale,
    );
    assert_eq!(console.cursor_hint(), CursorHint::Arrow);
    // A smaller window never leaves the console larger than it or off it.
    console.canvas.resize(640., 480.);
    let small = console.shell.panel.dialog_layout(&console.canvas);
    assert!(small.origin.x >= 0. && small.origin.y >= 0.);
    assert!(small.origin.x + small.size.x * small.scale <= 640.);
    assert!(small.origin.y + small.size.y * small.scale <= 480.);
}

#[test]
fn console_placement_survives_a_restart_and_resets_off_screen() {
    let mut console = Console::default();
    console.canvas.resize(1920., 1080.);
    console.toggle();
    let start = console.shell.panel.dialog_layout(&console.canvas);
    let caption = start.origin + vec2(100., 10.) * start.scale;
    pointer_drag(&mut console, caption, caption - vec2(600., 0.));
    let corner = console.shell.panel.dialog_layout(&console.canvas).origin
        + console.shell.panel.dialog_layout(&console.canvas).size * start.scale
        - Vec2::splat(4.);
    pointer_drag(&mut console, corner, corner - vec2(100., 100.));
    console.input(&Input::default());
    let saved = console
        .config
        .dialogs
        .console
        .expect("saved after the gesture");
    let placed = console.shell.panel.dialog_layout(&console.canvas);
    // The next run restores it.
    let mut next = Console::default();
    next.canvas.resize(1920., 1080.);
    next.shell.panel.placement = Placement::from_saved(Some(saved));
    next.toggle();
    assert_eq!(next.shell.panel.dialog_layout(&next.canvas), placed);
    // Opened on a smaller window where it would not fit as saved: the default frame.
    let mut small = Console::default();
    small.canvas.resize(800., 600.);
    small.shell.panel.placement = Placement::from_saved(Some(saved));
    small.toggle();
    assert_eq!(
        small.shell.panel.dialog_layout(&small.canvas),
        DialogLayout::new(vec2(800., 600.), proportional::SCALING)
    );
    small.input(&Input::default());
    assert_eq!(small.config.dialogs.console, None, "the reset is saved");
}

#[test]
fn the_quit_box_moves_alone_and_opens_centered() {
    let mut console = Console::default();
    console.canvas.resize(1280., 720.);
    console.set_mode(Mode::MainMenu);
    let loc = console.gameui.localization.clone();
    console.quit_placement.reset();
    console.confirm_box = Some((crate::gameui::menu::quit_box(&loc), ConfirmKind::Quit));
    let quit = console.confirm_box.as_ref().unwrap().0.clone();
    let (origin, _, _) = console.quit_geometry(&quit);
    pointer_drag(
        &mut console,
        origin + vec2(40., 8.),
        origin + vec2(140., 108.),
    );
    assert!(
        console.confirm_box.is_some(),
        "dragging does not answer the box"
    );
    assert_eq!(console.quit_geometry(&quit).0, origin + vec2(100., 100.));
    console.escape();
    assert!(console.confirm_box.is_none());
    console.quit_placement.reset();
    assert_eq!(console.quit_geometry(&quit).0, origin);
}

#[test]
fn dragging_over_history_rows_selects_them_for_copy() {
    let mut console = Console::default();
    console.canvas.resize(1024., 768.);
    console.toggle();
    console.shell.panel.output.clear();
    for i in 0..5 {
        console.log(format!("line {i}"));
    }
    // Rows 1..=3 by dragging upwards; the selection holds whole lines.
    drag(&mut console, 3, 1);
    assert_eq!(
        copy(&mut console).as_deref(),
        Some("line 1\nline 2\nline 3")
    );
    assert_eq!(console.take_copied(), None, "taken once");

    // New output and lines dropped from the front leave the selection on its lines.
    for i in 5..OUTPUT_LINES + 5 {
        console.log(format!("line {i}"));
    }
    assert_eq!(copy(&mut console), None, "the selected lines scrolled out");
    console.shell.panel.output.clear();
    console.shell.panel.output_start = 0;
    for i in 0..5 {
        console.log(format!("line {i}"));
    }
    drag(&mut console, 2, 2);
    console.log("printed after");
    console.shell.panel.output.pop_front();
    console.shell.panel.output_start += 1;
    assert_eq!(copy(&mut console).as_deref(), Some("line 2"));

    // Copying follows what the history shows: the filter and timestamps.
    drag(&mut console, 0, 3);
    console.submit("con_filter_enable 1; con_filter_text 3; con_timestamp 1");
    let copied = copy(&mut console).unwrap();
    assert!(copied.ends_with("] line 3"), "{copied}");
    assert!(!copied.contains('\n'), "{copied}");

    console.input(&Input {
        ctrl: true,
        binding_keys: vec!["KeyL".into()],
        ..Default::default()
    });
    assert_eq!(copy(&mut console), None, "Ctrl+L clears the selection");
    console.submit("con_filter_enable 0");
    drag(&mut console, 0, 0);
    assert!(copy(&mut console).is_some());
    console.submit("clear");
    console.log("after clear");
    assert_eq!(copy(&mut console), None, "clear drops the selection too");
}

#[test]
fn live_viewmodel_tuning_log_replaces_its_snapshot_and_keeps_other_output() {
    let mut console = Console::default();
    console.shell.panel.clear_output();
    console.log("before snapshot");
    console.log_viewmodel_tuning("{\"fov_offset\": 0.1}");
    console.log("unrelated output");
    console.log_viewmodel_tuning("{\"fov_offset\": 0.2}");

    assert_eq!(console.shell.panel.output.len(), 3);
    assert_eq!(console.shell.panel.output[0].text, "before snapshot");
    assert_eq!(console.shell.panel.output[1].level, Level::Snapshot);
    assert_eq!(console.shell.panel.output[1].text, "{\"fov_offset\": 0.2}");
    assert_eq!(console.shell.panel.output[2].text, "unrelated output");

    console.shell.panel.clear_output();
    console.log_viewmodel_tuning("{\"fov_offset\": 0.3}");
    assert_eq!(console.shell.panel.output.len(), 1);
    assert_eq!(console.shell.panel.output[0].text, "{\"fov_offset\": 0.3}");
}

#[test]
fn scrolled_back_history_stays_put_while_output_arrives() {
    let mut console = Console::default();
    console.canvas.resize(1024., 768.);
    console.toggle();
    for i in 0..200 {
        console.log(format!("line {i}"));
    }
    console.input(&Input {
        wheel: 2.,
        ..Default::default()
    });
    let rows = 2 * ScrollMode::platform().rows_per_step(WHEEL_ROWS);
    assert_eq!(console.shell.panel.scroll, rows);
    console.log("more");
    assert_eq!(console.shell.panel.scroll, rows + 1);
    keys(&mut console, &[Key::Enter]);
    assert_eq!(
        console.shell.panel.scroll, 0,
        "submitting returns to the newest output"
    );
}

/// Characters that fit a history row with the nominal advance of a console without fonts.
fn row_chars(console: &Console) -> usize {
    let metrics = console.shell.panel.wrap_metrics(&console.canvas);
    (metrics.width / (metrics.em * NOMINAL_ADVANCE)) as usize
}
/// The top history row shown: its line number and text.
fn top_row(console: &Console) -> (u64, String) {
    let rows = HistoryView::new(&console.shell.panel.dialog_layout(&console.canvas)).rows;
    let history = console.shell.panel.history_rows(&console.canvas);
    let shown = rows.min(history.len());
    let first = history.len() - shown - console.shell.panel.scroll.min(history.len() - shown);
    let row = &history[first];
    (
        row.number,
        console.shell.panel.shown_text(row.line)[row.text.clone()].to_owned(),
    )
}
/// Words long enough to wrap to at least `rows` history rows.
fn long_line(console: &Console, rows: usize) -> String {
    let words = rows * row_chars(console) / 5 + 1;
    (0..words)
        .map(|i| format!("w{i:03}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn long_lines_wrap_to_rows_and_newlines_start_rows() {
    let mut console = Console::default();
    console.canvas.resize(1024., 768.);
    console.toggle();
    console.shell.panel.output.clear();
    console.shell.panel.output_start = 0;
    let long = long_line(&console, 3);
    let warning = "Encountered an error in command `<Enable the debug feature to see the name>`:\n\
                       Entity despawned: The entity with ID 2224v1 is invalid";
    console.log("first");
    console.log(long.clone());
    console.print(Level::Warning, warning);
    console.log("last");
    let metrics = console.shell.panel.wrap_metrics(&console.canvas);
    let history = console.shell.panel.history_rows(&console.canvas);
    let rows: Vec<(u64, String)> = history
        .iter()
        .map(|row| {
            (
                row.number,
                console.shell.panel.shown_text(row.line)[row.text.clone()].to_owned(),
            )
        })
        .collect();
    let long_rows = rows.iter().filter(|(n, _)| *n == 1).count();
    assert!(long_rows >= 3, "{rows:?}");
    // Rows fit the history and break between words; joined they give the line back.
    for (_, text) in &rows {
        let width: f32 = text
            .chars()
            .map(|c| console.shell.panel.advance(c, metrics.em))
            .sum();
        assert!(width <= metrics.width, "{text}");
        assert!(!text.starts_with(' ') && !text.ends_with(' '), "{text:?}");
    }
    let joined: Vec<&str> = rows[1..=long_rows]
        .iter()
        .map(|(_, t)| t.as_str())
        .collect();
    assert_eq!(joined.join(" "), long);
    // The warning wraps, and its newline starts a row.
    let warning_rows: Vec<&str> = rows
        .iter()
        .filter(|(n, _)| *n == 2)
        .map(|(_, t)| t.as_str())
        .collect();
    let count = warning_rows.len();
    assert!(count >= 3, "{warning_rows:?}");
    assert!(warning_rows[count - 2].ends_with("name>`:"));
    assert_eq!(
        warning_rows[count - 1],
        "Entity despawned: The entity with ID 2224v1 is invalid"
    );
    assert_eq!(rows.last().unwrap(), &(3, "last".to_owned()));

    // Selecting any row of a wrapped line copies the whole line as printed.
    drag(&mut console, 2, 2);
    assert_eq!(copy(&mut console).as_deref(), Some(long.as_str()));
    let warning_row = long_rows + count;
    drag(&mut console, warning_row, warning_row);
    assert_eq!(copy(&mut console).as_deref(), Some(warning));
    drag(&mut console, rows.len() - 1, 0);
    assert_eq!(
        copy(&mut console),
        Some(format!("first\n{long}\n{warning}\nlast"))
    );

    // Timestamps wrap with the text and copy with it.
    console.submit("con_timestamp 1");
    assert_eq!(top_row(&console), (0, "[   0.00] first".to_owned()));
    drag(&mut console, 2, 2);
    let copied = copy(&mut console).unwrap();
    assert!(
        copied.starts_with("[   0.00] w000") && copied.ends_with(&long),
        "{copied}"
    );
    // The filter keeps or hides whole lines, all their rows.
    // (Set directly: the echo of a `con_filter_text` command contains its text.)
    console.submit("con_timestamp 0; con_filter_enable 1");
    console.shell.panel.con_filter_text = "despawned".into();
    let shown: Vec<u64> = console
        .shell
        .panel
        .history_rows(&console.canvas)
        .iter()
        .map(|r| r.number)
        .collect();
    assert_eq!(shown, vec![2; count]);
}

#[test]
fn scrolled_back_wrapped_history_stays_put_and_reaches_the_top() {
    let mut console = Console::default();
    console.canvas.resize(1024., 768.);
    console.toggle();
    console.shell.panel.output.clear();
    console.shell.panel.output_start = 0;
    let long = long_line(&console, 2);
    for i in 0..100 {
        console.log(if i % 2 == 0 {
            long.clone()
        } else {
            format!("line {i}")
        });
    }
    console.input(&Input {
        wheel: 2.,
        ..Default::default()
    });
    let top = top_row(&console);
    let scroll = console.shell.panel.scroll;
    assert!(scroll > 0);
    // New output below adds its rows to the scroll, so the shown rows stay.
    console.log(long.clone());
    let added = console
        .shell
        .panel
        .history_rows(&console.canvas)
        .iter()
        .filter(|r| r.number == 100)
        .count();
    assert!(added >= 2);
    assert_eq!(console.shell.panel.scroll, scroll + added);
    console.log("one\ntwo");
    assert_eq!(console.shell.panel.scroll, scroll + added + 2);
    assert_eq!(top_row(&console), top);
    console.input(&Input::default());
    assert_eq!(top_row(&console), top, "the next frame keeps the view");

    // Ctrl+Home goes to the first row of the first line, Page Up stops there too.
    console.input(&Input {
        keys: [Key::Home].into(),
        ctrl: true,
        ..Default::default()
    });
    console.input(&Input::default());
    assert_eq!(top_row(&console).0, 0);
    assert!(long.starts_with(&top_row(&console).1));
    keys(&mut console, &[Key::PageDown]);
    keys(&mut console, &[Key::PageUp]);
    keys(&mut console, &[Key::PageUp]);
    assert_eq!(top_row(&console).0, 0);
    assert!(long.starts_with(&top_row(&console).1));
    console.input(&Input {
        keys: [Key::End].into(),
        ctrl: true,
        ..Default::default()
    });
    assert_eq!(console.shell.panel.scroll, 0);
}

#[test]
fn submit_button_runs_the_line_and_close_button_closes_the_console() {
    let mut console = Console::default();
    console.canvas.resize(1024., 768.);
    console.escape();
    assert_eq!(console.mode(), Mode::Pause);
    console.toggle();
    console.input(&Input {
        text: "echo clicked".into(),
        ..Default::default()
    });
    let submit = dialog_point(
        &console,
        console.shell.panel.dialog_layout(&console.canvas).submit,
    );
    press(&mut console, submit);
    assert_eq!(console.input_line(), "echo clicked", "runs on release");
    assert_eq!(
        release(&mut console, submit),
        vec![Effect::PlaySound(RELEASE_SOUND.into())]
    );
    assert_eq!(console.input_line(), "");
    assert_eq!(
        console.shell.panel.output.back().map(|l| l.text.as_str()),
        Some("clicked")
    );

    let close = dialog_point(
        &console,
        console.shell.panel.dialog_layout(&console.canvas).close,
    );
    press(&mut console, close);
    assert_eq!(console.mode(), Mode::Console);
    release(&mut console, close);
    assert_eq!(
        console.mode(),
        Mode::Pause,
        "back to what it was opened from"
    );
}

/// Pointer position at the middle of menu row `index`.
fn row(console: &Console, index: usize) -> Option<Vec2> {
    let geometry = console.menu_layout.geometry_for(console.canvas_size());
    Some(geometry.menu + vec2(5., geometry.item_tall * (index as f32 + 0.5)))
}
fn click_row(console: &mut Console, index: usize) -> Vec<Effect> {
    let pointer = row(console, index);
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
}
fn keys(console: &mut Console, keys: &[Key]) -> Vec<Effect> {
    console.input(&Input {
        keys: keys.iter().copied().collect(),
        ..Default::default()
    })
}
/// Game -> map test data, standing in for the installed chapter files.
fn test_maps(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(game, map)| ((*game).to_owned(), (*map).to_owned()))
        .collect()
}

/// A console with the built-in menu structure, a known first chapter and a background map.
fn installed_console() -> Console {
    let loc = Localization::default();
    let resources = GameUiResources {
        localization: loc,
        game_menu: fallback_game_menu(),
        games: ["hl2", "episodic", "ep2", "lostcoast"]
            .map(String::from)
            .to_vec(),
        background_map: Some("background01".into()),
        background_maps: test_maps(&[
            ("hl2", "background01"),
            ("episodic", "ep1_background01"),
            ("ep2", "ep2_background01"),
            ("lostcoast", "background01"),
        ]),
        new_game_maps: test_maps(&[
            ("hl2", "d1_trainstation_01"),
            ("episodic", "ep1_citadel_00"),
            ("ep2", "ep2_outland_01"),
            ("lostcoast", "d2_lostcoast"),
        ]),
        new_games: test_maps(&[
            ("hl2", "d1_trainstation_01"),
            ("episodic", "ep1_citadel_00"),
            ("ep2", "ep2_outland_01"),
            ("lostcoast", "d2_lostcoast"),
        ])
        .into_iter()
        .map(|(game, map)| {
            let entries = vec![crate::gameui::new_game::Entry {
                key: "1".into(),
                map,
                heading: "1".into(),
                title: game.clone(),
            }];
            (
                game,
                crate::gameui::new_game::Catalog {
                    entries,
                    ..Default::default()
                },
            )
        })
        .collect(),
        ..Default::default()
    };
    let mut console = Console::default();
    console.set_resources(resources);
    console
}

#[test]
fn pause_menu_entries_activate_by_pointer_and_keyboard() {
    let mut console = installed_console();
    console.escape();
    assert_eq!(console.mode(), Mode::Pause);
    // Resume, Load, Save, Achievements, Options, Return to main menu, Quit.
    assert_eq!(console.pause_menu.entries().len(), 7);
    click_row(&mut console, 4);
    assert_eq!(console.mode(), Mode::Options);
    console.escape();
    assert_eq!(console.mode(), Mode::Pause);
    assert!(
        click_row(&mut console, 1).is_empty(),
        "Load Game is a placeholder"
    );
    assert_eq!(console.mode(), Mode::Pause);
    assert!(console
        .shell
        .panel
        .output
        .back()
        .unwrap()
        .contains("not implemented"));
    assert_eq!(
        click_row(&mut console, 5),
        vec![Effect::PlaySound("ui/buttonclickrelease.wav".into())]
    );
    let (confirm, kind) = console
        .confirm_box
        .clone()
        .expect("return to main menu confirmation");
    assert_eq!(kind, ConfirmKind::ReturnToMainMenu);
    assert_eq!(confirm.title, "RETURN TO MAIN MENU");
    let (origin, scale, layout) = console.quit_geometry(&confirm);
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
    // Cancel dismisses the dialog and remains in pause menu
    assert_eq!(
        click_btn(&mut console, layout.buttons[1]),
        vec![Effect::PlaySound("ui/buttonclickrelease.wav".into())]
    );
    assert!(console.confirm_box.is_none());
    assert_eq!(console.mode(), Mode::Pause);

    // The pointer left Return to main menu selected; Down moves to Quit, and Enter asks for
    // confirmation instead of quitting.
    assert_eq!(
        keys(&mut console, &[Key::Down]),
        vec![Effect::PlaySound("ui/buttonrollover.wav".into())]
    );
    assert_eq!(
        keys(&mut console, &[Key::Enter]),
        vec![Effect::PlaySound("ui/buttonclickrelease.wav".into())]
    );
    assert!(console.confirm_box.is_some());
    console.escape();
    assert!(console.confirm_box.is_none(), "Escape closes only the box");
    assert_eq!(console.mode(), Mode::Pause);
    keys(&mut console, &[Key::Enter]);
    assert_eq!(
        keys(&mut console, &[Key::Enter]),
        vec![
            Effect::PlaySound("ui/buttonclickrelease.wav".into()),
            Effect::Quit,
        ]
    );
    console.confirm_box = None;
    click_row(&mut console, 0);
    assert_eq!(console.mode(), Mode::Gameplay, "Resume");
}

#[test]
fn return_to_main_menu_confirmation_dialog_ok_unloads_to_background() {
    let mut console = installed_console();
    console.escape();
    assert_eq!(console.mode(), Mode::Pause);
    // Click row 5: "Return to main menu"
    assert_eq!(
        click_row(&mut console, 5),
        vec![Effect::PlaySound("ui/buttonclickrelease.wav".into())]
    );
    let (confirm, kind) = console
        .confirm_box
        .clone()
        .expect("return to main menu confirmation");
    assert_eq!(kind, ConfirmKind::ReturnToMainMenu);
    assert_eq!(confirm.title, "RETURN TO MAIN MENU");
    let (origin, scale, layout) = console.quit_geometry(&confirm);
    let pointer = Some(origin + vec2(layout.buttons[0].x + 3., layout.buttons[0].y + 3.) * scale);
    let _ = console.input(&Input {
        pointer,
        click: true,
        held: true,
        ..Default::default()
    });
    let effects = console.input(&Input {
        pointer,
        released: true,
        ..Default::default()
    });
    assert_eq!(
        effects,
        vec![
            Effect::PlaySound("ui/buttonclickrelease.wav".into()),
            Effect::MainMenuMap(GameMap::new("hl2", "background01")),
        ]
    );
    assert!(console.confirm_box.is_none());
    assert_eq!(console.mode(), Mode::MainMenu);
}

#[test]
fn options_open_from_the_main_menu_and_console_commands_update_config() {
    let mut console = installed_console();
    console.level_loaded(true);
    click_row(&mut console, 5);
    assert_eq!(console.mode(), Mode::Options);
    console.escape();
    assert_eq!(console.mode(), Mode::MainMenu);
    click_row(&mut console, 5);
    console.toggle();
    console.toggle();
    assert_eq!(
        console.mode(),
        Mode::Options,
        "console round trip keeps the dialog"
    );
    console.draw(0.);
    console.canvas.drain();
    console.escape();
    assert_eq!(
        console.mode(),
        Mode::MainMenu,
        "Escape returns to the opening menu"
    );
    console.set_mode(Mode::Console);
    let effects = console.submit("volume 0.5; sensitivity 2.5; fov 90; mat_hdr_level 0");
    assert_eq!(console.config.audio.volume, 0.5);
    assert_eq!(console.config.mouse.sensitivity, 2.5);
    assert_eq!(console.config.video.fov, 90.0);
    assert_eq!(console.config.video.hdr, crate::config::HdrMode::None);
    assert_eq!(effects.len(), 4);
}

#[test]
fn loader_thread_progress_reaches_only_its_consoles_dialog() {
    let mut console = Console::default();
    let mut other = Console::default();
    console.start_loading("d1_trainstation_01");
    other.start_loading("d1_trainstation_02");
    let progress = console.loading_progress();
    std::thread::spawn(move || progress.report(0.35, "LoadingProgress_PrecacheWorld"))
        .join()
        .unwrap();
    console.update_loading();
    other.update_loading();
    let dialog = console.loading_dialog.as_ref().unwrap();
    assert_eq!(dialog.fraction, 0.35);
    assert_eq!(dialog.stage, Some("LoadingProgress_PrecacheWorld"));
    assert_eq!(other.loading_dialog.as_ref().unwrap().stage, None);
    // Completing the load clears the report for the next one.
    console.loading_complete();
    assert_eq!(console.loading_progress().current(), None);
}

#[test]
fn loading_dialog_state_transitions_and_dismissal() {
    let mut console = installed_console();
    console.level_loaded(true);
    assert!(!console.is_loading());

    // Start loading
    console.start_loading("d1_trainstation_01");
    assert!(console.is_loading());
    assert!(console.loading_dialog.is_some());
    assert_eq!(
        console.loading_dialog.as_ref().unwrap().map,
        "d1_trainstation_01"
    );

    // Update progress via global reporting
    console
        .loading_progress()
        .report(0.5, "LoadingProgress_SignonDataLocal");
    console.update_loading();
    assert_eq!(console.loading_dialog.as_ref().unwrap().fraction, 0.5);
    assert_eq!(
        console.loading_dialog.as_ref().unwrap().stage,
        Some("LoadingProgress_SignonDataLocal")
    );

    // While actively loading, input is consumed without dismissing
    let input = Input {
        click: true,
        ..Default::default()
    };
    assert!(console.input(&input).is_empty());
    assert!(console.loading_dialog.is_some());

    // Completing loading clears the dialog
    console.loading_complete();
    assert!(!console.is_loading());
    assert!(console.loading_dialog.is_none());

    // Test failure path and dismissal via escape
    console.start_loading("invalid_map");
    console.loading_failed("Map invalid_map not found");
    assert!(!console.is_loading());
    assert!(console.loading_dialog.as_ref().unwrap().is_failed());
    assert_eq!(
        console.loading_dialog.as_ref().unwrap().error.as_deref(),
        Some("Map invalid_map not found")
    );

    console.escape();
    assert!(console.loading_dialog.is_none());
    assert_eq!(console.mode(), Mode::MainMenu);

    // Test failure path and dismissal via Enter or click
    console.start_loading("invalid_map");
    console.loading_failed("Map invalid_map not found");
    let enter_input = Input {
        keys: [Key::Enter].into_iter().collect(),
        ..Default::default()
    };
    console.input(&enter_input);
    assert!(console.loading_dialog.is_none());
    assert_eq!(console.mode(), Mode::MainMenu);

    // Test level_loaded also clears loading dialog
    console.start_loading("d1_trainstation_02");
    console.level_loaded(false);
    assert!(console.loading_dialog.is_none());
    assert_eq!(console.mode(), Mode::Gameplay);
}

#[test]
fn background_map_single_source_of_truth_and_return_to_main_menu() {
    let mut console = installed_console();
    // Default: falls back to GameUI resources background_map
    assert_eq!(console.background_map(), Some("background01"));
    assert_eq!(console.config.gameplay.background_map, None);

    // When overridden in config, background_map() prioritizes config
    console.set_background(Some(GameMap::new("hl2", "background02")));
    assert_eq!(console.background_map(), Some("background02"));

    // Trigger Return to Main Menu confirmation and click OK
    console.escape();
    assert_eq!(console.mode(), Mode::Pause);
    click_row(&mut console, 5); // Return to main menu
    let (confirm, _) = console.confirm_box.clone().expect("confirmation box");
    let (origin, scale, layout) = console.quit_geometry(&confirm);
    let pointer = Some(origin + vec2(layout.buttons[0].x + 3., layout.buttons[0].y + 3.) * scale);
    let _ = console.input(&Input {
        pointer,
        click: true,
        held: true,
        ..Default::default()
    });
    let effects = console.input(&Input {
        pointer,
        released: true,
        ..Default::default()
    });
    // Verifies the overridden background map was emitted
    assert_eq!(
        effects,
        vec![
            Effect::PlaySound("ui/buttonclickrelease.wav".into()),
            Effect::MainMenuMap(GameMap::new("hl2", "background02")),
        ]
    );
    assert_eq!(console.mode(), Mode::MainMenu);
}

#[test]
fn return_to_main_menu_shows_the_running_games_background() {
    let mut console = installed_console();
    // The Episode Two menu is showing, then a Half-Life 2 map is loaded from the console.
    console.set_background(Some(GameMap::new("ep2", "ep2_background01")));
    console.level_loaded_for_map("ep2", "ep2_background01", true);
    assert_eq!(
        console.submit("map d1_trainstation_02"),
        vec![Effect::Map("d1_trainstation_02".into())]
    );
    console.level_loaded_for_map("hl2", "d1_trainstation_02", false);
    assert_eq!(console.mode(), Mode::Gameplay);

    console.escape();
    click_row(&mut console, 5); // Return to main menu
    let (confirm, _) = console.confirm_box.clone().expect("confirmation box");
    let (origin, scale, layout) = console.quit_geometry(&confirm);
    let pointer = Some(origin + vec2(layout.buttons[0].x + 3., layout.buttons[0].y + 3.) * scale);
    let press = Input {
        pointer,
        click: true,
        held: true,
        ..Default::default()
    };
    let _ = console.input(&press);
    let effects = console.input(&Input {
        pointer,
        released: true,
        ..Default::default()
    });
    assert!(
        effects.contains(&Effect::MainMenuMap(GameMap::new("hl2", "background01"))),
        "{effects:?}"
    );
    assert_eq!(
        console.background(),
        Some(GameMap::new("hl2", "background01"))
    );
}

#[test]
fn loading_a_map_saves_its_chapter_background_for_the_menu() {
    use crate::gameui::chapters::{Chapter, ChapterIndex};
    let mut console = installed_console();
    let chapter = |key: &str, start: &str, background: &str| Chapter {
        key: key.into(),
        start: Some(start.into()),
        background: background.into(),
    };
    Arc::make_mut(&mut console.gameui).chapters.insert(
        "hl2".into(),
        vec![
            chapter("1", "d1_trainstation_01", "background01"),
            chapter("7", "d2_coast_01", "background04"),
        ],
    );
    console.chapter_index = ChapterIndex::ready(
        [(
            "hl2".to_owned(),
            [("d2_coast_01".to_owned(), "7".to_owned())].into(),
        )]
        .into(),
    );
    console.set_background(Some(GameMap::new("hl2", "background01")));
    console.level_loaded_for_map("hl2", "background01", true);
    let _ = console.submit("map d2_coast_01");
    console.level_loaded_for_map("hl2", "d2_coast_01", false);
    // Saved on load: quitting the game now still starts over this chapter's background.
    assert_eq!(
        console.background(),
        Some(GameMap::new("hl2", "background04"))
    );

    console.escape();
    click_row(&mut console, 5); // Return to main menu
    let (confirm, _) = console.confirm_box.clone().expect("confirmation box");
    let (origin, scale, layout) = console.quit_geometry(&confirm);
    let pointer = Some(origin + vec2(layout.buttons[0].x + 3., layout.buttons[0].y + 3.) * scale);
    let _ = console.input(&Input {
        pointer,
        click: true,
        held: true,
        ..Default::default()
    });
    let effects = console.input(&Input {
        pointer,
        released: true,
        ..Default::default()
    });
    assert!(
        effects.contains(&Effect::MainMenuMap(GameMap::new("hl2", "background04"))),
        "{effects:?}"
    );
    assert_eq!(
        console.background(),
        Some(GameMap::new("hl2", "background04"))
    );
}

/// A main-menu console at 1280x720 whose Options applied a change to `width` x `height`
/// at UI time 10; returns the applied effects.
fn applied_video_change(width: u32, height: u32) -> (Console, Vec<Effect>) {
    let mut console = installed_console();
    console.set_mode(Mode::MainMenu);
    console.config.video.width = 1280;
    console.config.video.height = 720;
    let _ = console.tick(10.);
    let mut config = console.config.clone();
    config.video.width = width;
    config.video.height = height;
    let effects = console.apply_settings(config);
    (console, effects)
}

fn applied_size(effects: &[Effect]) -> Option<(u32, u32)> {
    effects.iter().find_map(|e| match e {
        Effect::ApplyConfig(c) => Some((c.video.width, c.video.height)),
        _ => None,
    })
}

fn keep_video_open(console: &Console) -> bool {
    matches!(console.confirm_box, Some((_, ConfirmKind::KeepVideo(_))))
}

#[test]
fn an_aspect_change_applies_and_asks_to_keep_it() {
    let (console, effects) = applied_video_change(1024, 768);
    assert_eq!(applied_size(&effects), Some((1024, 768)), "applied at once");
    assert!(keep_video_open(&console));
    assert!(console.modal_dialog_open());
    let (mut loaded, _) = applied_video_change(1024, 768);
    loaded.level_loaded(true);
    assert!(
        keep_video_open(&loaded),
        "a background load does not decide it"
    );
    let (same_aspect, _) = applied_video_change(1920, 1080);
    assert!(
        same_aspect.confirm_box.is_none(),
        "16:9 to 16:9 needs no confirmation"
    );
}

#[test]
fn the_keep_video_box_counts_down_and_reverts_at_the_timeout() {
    let (mut console, _) = applied_video_change(1024, 768);
    let seconds = crate::tuning::video_confirm_seconds();
    let text = |c: &Console| c.confirm_box.as_ref().unwrap().0.text.clone();
    let first = text(&console);
    assert!(
        first.contains(&format!("{}", seconds.ceil() as u32)),
        "{first}"
    );
    assert!(console.tick(10. + seconds - 2.5).is_empty());
    assert!(text(&console).contains('3'), "{}", text(&console));
    let effects = console.tick(10. + seconds);
    assert_eq!(applied_size(&effects), Some((1280, 720)), "reverted");
    assert!(console.confirm_box.is_none());
    assert_eq!(
        (console.config.video.width, console.config.video.height),
        (1280, 720)
    );
    assert_eq!(
        console.options.draft().video.width,
        1280,
        "Options shows it too"
    );
}

#[test]
fn ok_keeps_the_change_and_cancel_escape_revert_it() {
    let enter = |console: &mut Console| {
        let mut effects = console.input(&Input {
            keys: [Key::Enter].into(),
            held_keys: [Key::Enter].into(),
            ..Default::default()
        });
        effects.extend(console.input(&Input::default()));
        effects
    };
    // Cancel has the focus: Enter reverts.
    let (mut console, _) = applied_video_change(1024, 768);
    assert_eq!(applied_size(&enter(&mut console)), Some((1280, 720)));
    assert!(console.confirm_box.is_none());
    // Tab to OK, Enter keeps it: nothing reverts later.
    let (mut console, _) = applied_video_change(1024, 768);
    let _ = console.input(&Input {
        keys: [Key::Tab].into(),
        ..Default::default()
    });
    assert_eq!(applied_size(&enter(&mut console)), None);
    assert!(console.confirm_box.is_none());
    assert!(console.tick(1000.).is_empty(), "no timeout after OK");
    assert_eq!(console.config.video.width, 1024);
    // Escape reverts; the effect comes with the next tick.
    let (mut console, _) = applied_video_change(1080, 1920);
    console.escape();
    assert!(console.confirm_box.is_none());
    assert_eq!(applied_size(&console.tick(11.)), Some((1280, 720)));
}

#[test]
fn a_resting_pointer_does_not_take_the_menu_selection_back_from_the_arrows() {
    let mut console = installed_console();
    console.set_mode(Mode::MainMenu);
    let pointer = row(&console, 0);
    let _ = console.input(&Input {
        pointer,
        ..Default::default()
    });
    assert_eq!(
        console.main_menu.selection,
        Some(0),
        "entering a row selects it"
    );
    let _ = console.input(&Input {
        pointer,
        keys: [Key::Down].into(),
        ..Default::default()
    });
    let after_arrow = console.main_menu.selection;
    assert_ne!(after_arrow, Some(0));
    let _ = console.input(&Input {
        pointer,
        ..Default::default()
    });
    assert_eq!(
        console.main_menu.selection, after_arrow,
        "the arrows keep it"
    );
    let _ = console.input(&Input {
        pointer: row(&console, 1),
        ..Default::default()
    });
    let _ = console.input(&Input {
        pointer,
        ..Default::default()
    });
    assert_eq!(
        console.main_menu.selection,
        Some(0),
        "entering a row again selects it"
    );
}

#[test]
fn a_held_escape_presses_the_close_button_and_the_release_closes() {
    let mut console = installed_console();
    console.set_mode(Mode::MainMenu);
    console.set_mode(Mode::Options);
    console.escape_pressed();
    assert_eq!(console.mode(), Mode::Options, "nothing closes on the press");
    assert!(
        console.options.escape_held,
        "the close button shows pressed"
    );
    console.escape_released();
    assert_eq!(
        console.mode(),
        Mode::MainMenu,
        "the release closes the dialog"
    );
    assert!(!console.options.escape_held);
    // Escape that does not close a dialog acts at once: the main menu has no dialog to close.
    console.escape_pressed();
    assert!(!console.shell.escape_held);
}

#[test]
fn a_held_escape_presses_subdialog_close_button_and_release_closes_it() {
    let mut console = installed_console();
    console.set_mode(Mode::MainMenu);
    console.set_mode(Mode::Options);
    console
        .options
        .open_sub(crate::gameui::options::Sub::VideoAdvanced);
    console.sync_dialog_windows();
    assert_eq!(console.shell.windows.top(), Some(Window::SubDialog));
    assert!(console.options.has_sub());

    // Press Escape while SubDialog is top: subdialog close button shows pressed
    console.escape_pressed();
    assert!(
        console.options.escape_held,
        "subdialog close button shows pressed while Escape is held"
    );
    assert!(
        console.options.has_sub(),
        "subdialog does not close on press"
    );

    // When drawn, verify no panic and quads emit cleanly
    console.canvas.resize(1280., 720.);
    console.draw(0.0);

    // Release Escape: subdialog closes, focus and top return to Options
    console.escape_released();
    assert!(!console.options.escape_held);
    assert!(
        !console.options.has_sub(),
        "subdialog closed on Escape release"
    );
    assert_eq!(
        console.shell.windows.top(),
        Some(Window::Options),
        "Options dialog is now top"
    );

    // Escape on Options now holds Options close button
    console.escape_pressed();
    assert!(console.options.escape_held);
    console.escape_released();
    assert_eq!(console.mode(), Mode::MainMenu);

    // Test cancel drag first: dragging subdialog cancels drag immediately on Escape press
    console.open_options();
    console
        .options
        .open_sub(crate::gameui::options::Sub::VideoAdvanced);
    console.sync_dialog_windows();
    let bounds = console.window_bounds(Window::SubDialog);
    let caption_pt = vec2(bounds.x + 50., bounds.y + 10.);
    pointer_frame(&mut console, caption_pt, true, true, false);
    pointer_frame(
        &mut console,
        caption_pt + vec2(100., 50.),
        false,
        true,
        false,
    );

    // Escape cancels drag immediately without holding close button
    console.escape_pressed();
    assert!(!console.options.escape_held);
    assert!(
        console.options.has_sub(),
        "subdialog still open after drag cancelled"
    );

    // Subsequent Escape press now holds the close button
    console.escape_pressed();
    assert!(console.options.escape_held);
    console.escape_released();
    assert!(!console.options.has_sub());
}

#[test]
fn a_held_escape_presses_confirmation_cancel_button_and_release_cancels_it() {
    let mut console = installed_console();
    let loc = crate::gameui::localize::Localization::default();
    console.set_mode(Mode::MainMenu);
    console.confirm_box = Some((crate::gameui::menu::quit_box(&loc), ConfirmKind::Quit));
    console.sync_dialog_windows();
    assert_eq!(console.shell.windows.top(), Some(Window::Confirmation));
    assert!(console.confirm_box.is_some());

    // Press Escape while confirmation is top: Cancel button shows armed/pressed, box stays open
    console.escape_pressed();
    assert!(
        console.confirm_box.is_some(),
        "confirm box does not close on press"
    );
    assert!(
        console.confirm_box.as_ref().unwrap().0.escape_held,
        "Cancel button on confirm_box shows armed/pressed while Escape is held"
    );

    // Render to verify drawing with escape_held works cleanly
    console.canvas.resize(1280., 720.);
    console.draw(0.0);

    // Release Escape: confirm box closes and cancelled
    console.escape_released();
    assert!(
        console.confirm_box.is_none(),
        "confirm box dismissed on Escape release"
    );
    assert_ne!(
        console.shell.windows.top(),
        Some(Window::Confirmation),
        "Confirmation window closed"
    );

    // Test dragging confirm box: pressing Escape cancels the drag, dialog stays open
    console.confirm_box = Some((crate::gameui::menu::quit_box(&loc), ConfirmKind::Quit));
    console.sync_dialog_windows();
    let bounds = console.window_bounds(Window::Confirmation);
    let caption_pt = vec2(bounds.x + 50., bounds.y + 10.);
    pointer_frame(&mut console, caption_pt, true, true, false);
    pointer_frame(
        &mut console,
        caption_pt + vec2(100., 50.),
        false,
        true,
        false,
    );
    assert!(console.quit_placement.active());

    // Escape cancels drag immediately without holding Escape
    console.escape_pressed();
    assert!(
        !console.confirm_box.as_ref().unwrap().0.escape_held,
        "drag cancelled, escape not held"
    );
    assert!(
        console.confirm_box.is_some(),
        "confirm box still open after drag cancelled"
    );
    assert!(!console.quit_placement.active());

    // Subsequent Escape press now holds Cancel button
    console.escape_pressed();
    assert!(console.confirm_box.as_ref().unwrap().0.escape_held);
    console.escape_released();
    assert!(console.confirm_box.is_none());

    // Test Options Use Defaults confirmation box
    console.open_options();
    console.options.confirm = Some(MessageBox::new(
        "Use Defaults".into(),
        "Reset all keys to default?".into(),
        "OK".into(),
        Some("Cancel".into()),
    ));
    console.sync_dialog_windows();
    assert_eq!(console.shell.windows.top(), Some(Window::Confirmation));
    assert!(console.options.has_confirm());

    // Escape pressed: options confirm has escape_held = true
    console.escape_pressed();
    assert!(console.options.has_confirm());
    assert!(console.shell.escape_held);

    console.draw(0.0);

    // Escape released: options confirm closed
    console.escape_released();
    assert!(!console.options.has_confirm());
    assert_eq!(console.shell.windows.top(), Some(Window::Options));
}

#[test]
fn background_map_and_map_background_console_commands() {
    let mut console = installed_console();
    // Query default background map
    let effects = console.submit("background_map");
    assert!(effects.is_empty());
    assert!(console
        .shell
        .panel
        .output
        .iter()
        .any(|line| line.contains("\"background_map\" = \"background01\"")));

    // Set persistent background map
    let effects = console.submit("background_map background03");
    assert_eq!(effects.len(), 1);
    assert!(matches!(effects[0], Effect::ApplyConfig(_)));
    assert_eq!(
        console.config.gameplay.background_map.as_deref(),
        Some("background03")
    );
    assert_eq!(console.background_map(), Some("background03"));

    // Reset to default
    let effects = console.submit("background_map default");
    assert_eq!(effects.len(), 1);
    assert!(matches!(effects[0], Effect::ApplyConfig(_)));
    assert_eq!(console.config.gameplay.background_map, None);
    assert_eq!(console.background_map(), Some("background01"));

    // map_background loads as background map and persists to config
    let effects = console.submit("map_background background04");
    assert_eq!(
        effects,
        vec![Effect::MainMenuMap(GameMap::new("hl2", "background04"))]
    );
    assert_eq!(
        console.config.gameplay.background_map.as_deref(),
        Some("background04")
    );
    assert_eq!(console.background_map(), Some("background04"));
    assert!(console.is_loading());
    assert_eq!(console.loading_dialog.as_ref().unwrap().map, "background04");

    // Setting background map while gameplay map is loading persists without interrupting load
    console.start_loading("d1_trainstation_01");
    console.background_level = false;
    assert!(console.is_loading());
    let effects = console.submit("map_background background05");
    assert!(effects.is_empty());
    assert_eq!(
        console.loading_dialog.as_ref().unwrap().map,
        "d1_trainstation_01"
    );
    assert_eq!(console.background_map(), Some("background05"));
}

#[test]
fn episode_submenus_show_two_line_titles() {
    let mut console = Console::default();
    let mut loc = Localization::default();
    loc.insert("GameUI_HalfLife2", "Half-Life 2");
    loc.insert("GameUI_EpisodeOne", "Episode One");
    loc.insert("GameUI_EpisodeTwo", "Episode Two");
    console.set_resources(GameUiResources {
        localization: loc.clone(),
        category_titles: resources::fallback_category_titles(&loc),
        ..Default::default()
    });
    console.set_mode(Mode::MainMenu);
    assert_eq!(console.current_title(), "Half-Life 2");

    // Menu with items to navigate between main, episodic, and ep2 categories
    console.main_menu = GameMenu::new(
        vec![
            MenuItem {
                label: "#GameUI_HalfLife2EpisodeOne".into(),
                command: "SwitchExpansion episodic".into(),
                category: Some("main".into()),
                order: 10,
                visibility: Visibility::Always,
                caps: true,
            },
            MenuItem {
                label: "#GameUI_HalfLife2EpisodeTwo".into(),
                command: "SwitchExpansion ep2".into(),
                category: Some("main".into()),
                order: 20,
                visibility: Visibility::Always,
                caps: true,
            },
            MenuItem {
                label: "#GameUI_Back".into(),
                command: "SwitchCategory main".into(),
                category: Some("episodic".into()),
                order: 30,
                visibility: Visibility::Always,
                caps: true,
            },
            MenuItem {
                label: "#GameUI_Back".into(),
                command: "SwitchCategory main".into(),
                category: Some("ep2".into()),
                order: 40,
                visibility: Visibility::Always,
                caps: true,
            },
        ],
        Default::default(),
        &loc,
    );

    // Activate Episode One entry
    console.main_menu.activate(0, &loc);
    assert_eq!(console.main_menu.category(), "episodic");
    assert_eq!(console.current_title(), "Half-Life 2\n== episode one==");
    assert_eq!(console.current_title().lines().count(), 2);

    // Activate Back button to return to main
    console.main_menu.activate(0, &loc);
    assert_eq!(console.main_menu.category(), "main");
    assert_eq!(console.current_title(), "Half-Life 2");
    assert_eq!(console.current_title().lines().count(), 1);

    // Activate Episode Two entry
    console.main_menu.activate(1, &loc);
    assert_eq!(console.main_menu.category(), "ep2");
    assert_eq!(console.current_title(), "Half-Life 2\n== episode two ==");
    assert_eq!(console.current_title().lines().count(), 2);
}

#[test]
fn click_that_dismisses_a_modal_layer_does_not_reach_the_menu_below() {
    let mut console = installed_console();
    console.set_mode(Mode::MainMenu);
    console.canvas.resize(1280., 720.);
    console.start_loading("invalid_map");
    console.loading_failed("Map invalid_map not found");
    let at = row(&console, 0);
    // The press dismisses the failed loading dialog; the held and release frames of the
    // same click land on a menu row and must not activate it.
    let mut effects = Vec::new();
    for (click, held, released) in [
        (true, true, false),
        (false, true, false),
        (false, false, true),
    ] {
        effects.extend(console.input(&Input {
            pointer: at,
            click,
            held,
            released,
            ..Default::default()
        }));
    }
    assert!(console.loading_dialog.is_none());
    assert_eq!(console.mode(), Mode::MainMenu);
    assert!(
        !effects.contains(&Effect::PlaySound("ui/buttonclickrelease.wav".into())),
        "no menu activation from the dismissing click: {effects:?}"
    );
}

#[test]
fn modal_dialog_blocks_underlying_menu_hover_sounds_and_clicks() {
    let mut console = installed_console();
    console.set_mode(Mode::MainMenu);
    console.canvas.resize(1280., 720.);

    // 1. When no modal dialog is open, hovering and clicking menu rows work normally
    assert!(!console.modal_dialog_open());
    assert_eq!(console.modal_bounds(), None);
    assert!(!console.is_occluded(vec2(100., 100.)));

    let hover_effects = console.input(&Input {
        pointer: row(&console, 0),
        ..Default::default()
    });
    assert!(
        hover_effects.contains(&Effect::PlaySound("ui/buttonrollover.wav".into())),
        "hovering menu row should play rollover sound when no dialog is open"
    );
    assert_eq!(console.main_menu.selection, Some(0));

    // 2. Options is a window, not modal: it covers its own frame only (the menu around
    // it keeps working; see the window tests below).
    console.set_mode(Mode::Options);
    assert!(!console.modal_dialog_open());
    let opt_bounds = console.window_bounds(Window::Options);
    assert!(opt_bounds.w > 0. && opt_bounds.h > 0.);
    let center = vec2(
        opt_bounds.x + opt_bounds.w * 0.5,
        opt_bounds.y + opt_bounds.h * 0.5,
    );
    assert!(console.is_occluded(center));
    assert!(!console.is_occluded(vec2(0., 0.)));

    // Closing options via escape preserves keyboard selection and returns to main menu
    console.escape();
    assert_eq!(console.mode(), Mode::MainMenu);
    assert!(!console.modal_dialog_open());
    assert_eq!(console.main_menu.selection, Some(0));

    // 3. Open confirmation dialog: modal check is true, underlying rows inert
    let loc = console.gameui.localization.clone();
    console.confirm_box = Some((crate::gameui::menu::quit_box(&loc), ConfirmKind::Quit));
    assert!(console.modal_dialog_open());
    let confirm_bounds = console.modal_bounds().expect("confirm box bounds");
    assert!(console.is_occluded(vec2(
        confirm_bounds.x + confirm_bounds.w * 0.5,
        confirm_bounds.y + confirm_bounds.h * 0.5
    )));

    // Hover over menu row 0 produces no rollover sound
    let hover_effects = console.input(&Input {
        pointer: row(&console, 0),
        ..Default::default()
    });
    assert!(
        !hover_effects.contains(&Effect::PlaySound("ui/buttonrollover.wav".into())),
        "hover over underlying menu row must not produce rollover sound while ConfirmBox is open"
    );

    // Click over menu row 0 does not activate
    let click_effects = click_row(&mut console, 0);
    assert!(
        click_effects.is_empty(),
        "click over underlying menu row must not pass through while ConfirmBox is open"
    );
    assert!(console.confirm_box.is_some(), "confirm box remains open");

    // Escape dismisses confirm box and preserves menu selection
    console.escape();
    assert!(console.confirm_box.is_none());
    assert!(!console.modal_dialog_open());
    assert_eq!(console.main_menu.selection, Some(0));

    // 4. Loading dialog: modal check is true, underlying rows inert
    console.start_loading("d1_trainstation_01");
    assert!(console.modal_dialog_open());
    assert!(console.is_loading());
    let load_bounds = console.modal_bounds().expect("loading bounds");
    assert!(console.is_occluded(vec2(
        load_bounds.x + load_bounds.w * 0.5,
        load_bounds.y + load_bounds.h * 0.5
    )));

    let hover_effects = console.input(&Input {
        pointer: row(&console, 0),
        ..Default::default()
    });
    assert!(
        !hover_effects.contains(&Effect::PlaySound("ui/buttonrollover.wav".into())),
        "hover over menu row must not produce sound while LoadingDialog is open"
    );

    let click_effects = click_row(&mut console, 0);
    assert!(click_effects.is_empty());
    console.loading_complete();
    assert!(!console.modal_dialog_open());
    assert_eq!(console.main_menu.selection, None);

    // 5. The console is a window too: not modal; a menu row it does not cover still works.
    console.toggle();
    assert_eq!(console.mode(), Mode::Console);
    assert!(!console.modal_dialog_open());
    let row0 = row(&console, 0).unwrap();
    let covered = console.window_bounds(Window::Console).contains(row0);
    let click_effects = click_row(&mut console, 0);
    assert_eq!(click_effects.is_empty(), covered, "{click_effects:?}");
    assert!(console.console_open());
    console.toggle();
    assert_eq!(console.mode(), Mode::MainMenu);
    assert!(!console.modal_dialog_open());
}

#[test]
fn submenu_entry_switches_background_map_and_persists() {
    let mut console = installed_console();
    console.set_mode(Mode::MainMenu);
    console.background_level = true;
    console.loaded_map = Some(GameMap::new("hl2", "background01"));

    // Initial background map state is background01
    assert_eq!(console.background_map(), Some("background01"));

    // 1. Enter Episode One submenu (row 1: "HL2: EPISODE ONE")
    let effects = click_row(&mut console, 1);
    assert_eq!(console.main_menu.category(), "episodic");
    assert_eq!(
        console.config.gameplay.background_map.as_deref(),
        Some("ep1_background01"),
        "single background-map state is updated to episode 1"
    );
    assert!(console.is_loading());
    assert!(console.loading_dialog.as_ref().unwrap().is_background());
    assert!(!console.modal_dialog_open());
    assert!(console.modal_bounds().is_none());
    assert!(
        effects.contains(&Effect::MainMenuMap(GameMap::new(
            "episodic",
            "ep1_background01"
        ))),
        "dispatches Effect::MainMenuMap for ep1_background01"
    );

    // Map loads and level_loaded is called
    console.level_loaded_for_map("episodic", "ep1_background01", true);
    assert_eq!(
        console.main_menu.category(),
        "episodic",
        "category remains in episodic submenu after background map load"
    );
    assert_eq!(
        console.loaded_map,
        Some(GameMap::new("episodic", "ep1_background01"))
    );
    assert!(!console.is_loading());

    // 2. Return to main category via Escape, then enter Episode Two submenu
    console.escape();
    assert_eq!(console.main_menu.category(), "main");
    // Background map remains ep1_background01 until a different expansion is selected
    assert_eq!(
        console.loaded_map,
        Some(GameMap::new("episodic", "ep1_background01"))
    );

    // Click row 2: "HL2: EPISODE TWO"
    let effects = click_row(&mut console, 2);
    assert_eq!(console.main_menu.category(), "ep2");
    assert_eq!(
        console.config.gameplay.background_map.as_deref(),
        Some("ep2_background01"),
        "persisted background map updated to ep2"
    );
    assert!(console.is_loading());
    assert!(console.loading_dialog.as_ref().unwrap().is_background());
    assert!(!console.modal_dialog_open());
    assert!(effects.contains(&Effect::MainMenuMap(GameMap::new(
        "ep2",
        "ep2_background01"
    ))));

    console.level_loaded_for_map("ep2", "ep2_background01", true);
    assert_eq!(console.main_menu.category(), "ep2");
    assert_eq!(
        console.loaded_map,
        Some(GameMap::new("ep2", "ep2_background01"))
    );

    // 3. Return to main, then enter Half-Life 2 submenu (row 0)
    console.escape();
    assert_eq!(console.main_menu.category(), "main");

    let effects = click_row(&mut console, 0);
    assert_eq!(console.main_menu.category(), "hl2");
    assert_eq!(
        console.config.gameplay.background_map.as_deref(),
        Some("background01")
    );
    assert!(effects.contains(&Effect::MainMenuMap(GameMap::new("hl2", "background01"))));

    console.level_loaded_for_map("hl2", "background01", true);
    assert_eq!(console.main_menu.category(), "hl2");
    assert_eq!(
        console.loaded_map,
        Some(GameMap::new("hl2", "background01"))
    );

    // 4. Entering Half-Life 2 submenu when already on background01 does not reload
    console.escape();
    assert_eq!(console.main_menu.category(), "main");
    let effects = click_row(&mut console, 0);
    assert_eq!(console.main_menu.category(), "hl2");
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::MainMenuMap(_))),
        "no redundant reload when target background map is already active"
    );
    assert!(!console.is_loading());
}

#[test]
fn submenu_back_and_escape_preserves_background_map() {
    let mut console = installed_console();
    console.set_mode(Mode::MainMenu);
    console.background_level = true;
    console.loaded_map = Some(GameMap::new("hl2", "background01"));

    // Switch to Episode One
    let _ = click_row(&mut console, 1);
    console.level_loaded_for_map("episodic", "ep1_background01", true);
    assert_eq!(console.main_menu.category(), "episodic");
    assert_eq!(
        console.config.gameplay.background_map.as_deref(),
        Some("ep1_background01")
    );

    // In episodic submenu, row 5 is "BACK"
    let back_effects = click_row(&mut console, 5);
    assert_eq!(console.main_menu.category(), "main");
    assert!(
        !back_effects
            .iter()
            .any(|e| matches!(e, Effect::MainMenuMap(_))),
        "Back button does not trigger map reload"
    );
    assert_eq!(
        console.config.gameplay.background_map.as_deref(),
        Some("ep1_background01"),
        "background map persistence stays intact on Back"
    );
    assert_eq!(
        console.loaded_map,
        Some(GameMap::new("episodic", "ep1_background01"))
    );
}

#[test]
fn submenu_titles_resolve_for_expansion_aliases() {
    let mut console = Console::default();
    let mut res = GameUiResources::default();
    res.category_titles
        .insert("episodic".into(), "HALF-LIFE'\n== episode one==".into());
    res.category_titles
        .insert("ep2".into(), "HALF-LIFE'\n== episode two ==".into());
    res.category_titles
        .insert("lostcoast".into(), "HALF-LIFE'\nlost coasT".into());
    console.set_resources(res);

    assert_eq!(
        console.category_title("episodic"),
        "HALF-LIFE'\n== episode one=="
    );
    assert_eq!(
        console.category_title("hl2ep1"),
        "HALF-LIFE'\n== episode one=="
    );
    assert_eq!(
        console.category_title("ep2"),
        "HALF-LIFE'\n== episode two =="
    );
    assert_eq!(
        console.category_title("hl2ep2"),
        "HALF-LIFE'\n== episode two =="
    );
    assert_eq!(
        console.category_title("lostcoast"),
        "HALF-LIFE'\nlost coasT"
    );
    assert_eq!(console.category_title("hl2lc"), "HALF-LIFE'\nlost coasT");
}

#[test]
fn background_loading_draw_emits_blur_overlay_behind_menu_and_corner_plaque() {
    let mut console = Console::default();
    console.canvas.resize(1920., 1080.);
    console.set_mode(Mode::MainMenu);

    console.start_background_loading("ep2", "ep2_background01");
    assert!(console.is_loading());
    assert!(console.is_background_loading());
    assert!(!console.modal_dialog_open());

    console.draw(0.0);
    let quads = console.canvas.drain();

    // Must emit the full-canvas background backdrop overlay quad
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
        "expected exactly 1 backdrop overlay quad"
    );
    let overlay = backdrop_quads[0];
    assert_eq!(overlay.destination.w, 1920.);
    assert_eq!(overlay.destination.h, 1080.);

    // The backdrop overlay must be drawn BEFORE the menu items so text is sharp over the backdrop
    let backdrop_index = quads
        .iter()
        .position(|q| {
            q.destination.w == 1920.
                && q.destination.h == 1080.
                && q.color == crate::gameui::loading::BackgroundTransitionState::MAIN_MENU_BACKDROP
        })
        .unwrap();
    assert_eq!(
        backdrop_index, 0,
        "backdrop overlay must be the first quad drawn behind the menu"
    );

    let scale = Painter::scale_for(1080.);
    let expected_plaque_w = (crate::gameui::loading::BACKGROUND_LOADING_SIZE.x * scale).round();
    let margin = (24.0 * scale).round();
    let expected_x = (1920.0 - expected_plaque_w - margin).max(0.0).round();

    // Plaque is drawn via shared rounded dialog frame (no outer 3D border)
    let middle_bar = quads
        .iter()
        .find(|q| {
            (q.destination.w - expected_plaque_w).abs() < 1.0
                && (q.destination.x - expected_x).abs() < 1.0
                && q.vertices.is_none()
        })
        .expect("bottom-right corner plaque middle bar");
    assert_eq!(middle_bar.color, Color::new(0.0, 0.0, 0.0, 0.65));

    let corner_polygons = quads
        .iter()
        .filter(|q| q.vertices.is_some() && q.destination.x >= expected_x - 1.0)
        .count();
    assert_eq!(corner_polygons, 4);
}

#[test]
fn gameplay_loading_draw_does_not_emit_blur_overlay() {
    let mut console = Console::default();
    console.canvas.resize(1920., 1080.);
    console.set_mode(Mode::MainMenu);

    console.start_loading("d1_trainstation_01");
    assert!(console.is_loading());
    assert!(!console.is_background_loading());
    assert!(console.modal_dialog_open());

    console.draw(0.0);
    let quads = console.canvas.drain();

    // Gameplay map loading must NEVER emit a backdrop overlay quad
    let backdrop_quads: Vec<_> = quads
        .iter()
        .filter(|q| {
            q.destination.w == 1920.
                && q.destination.h == 1080.
                && q.color == crate::gameui::loading::BackgroundTransitionState::MAIN_MENU_BACKDROP
        })
        .collect();
    assert!(
        backdrop_quads.is_empty(),
        "gameplay map loading must not emit any backdrop overlay quads"
    );

    // Frame and progress bar quads must be present
    let scale = Painter::scale_for(1080.);
    let expected_frame_w = (crate::gameui::loading::LOADING_DIALOG_SIZE.x * scale).round();
    let _frame_quad = quads
        .iter()
        .find(|q| (q.destination.w - expected_frame_w).abs() < 0.1)
        .expect("centered gameplay loading dialog frame");
}

#[test]
fn background_loading_completion_dismisses_blur_overlay() {
    let mut console = Console::default();
    console.canvas.resize(1920., 1080.);
    console.set_mode(Mode::MainMenu);

    console.start_background_loading("ep2", "ep2_background01");
    assert!(console.is_background_loading());

    // Level loads
    console.level_loaded(true);
    assert!(!console.is_loading());
    assert!(!console.is_background_loading());

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
    assert!(
        backdrop_quads.is_empty(),
        "no backdrop overlay quads once background map loading is complete"
    );
}

#[test]
fn background_loading_state_sequence_success() {
    let mut console = Console {
        mode: Mode::MainMenu,
        base: Mode::MainMenu,
        ..Default::default()
    };
    console.canvas.resize(1920., 1080.);

    assert_eq!(
        console.background_transition_state(),
        &crate::gameui::loading::BackgroundTransitionState::Idle
    );

    console.start_background_loading("ep2", "ep2_background01");
    assert!(console.is_background_loading());
    assert!(matches!(
        console.background_transition_state(),
        crate::gameui::loading::BackgroundTransitionState::Starting { .. }
    ));
    assert_eq!(
        console.background_transition_state().target_map(),
        Some("ep2_background01")
    );

    console.update_loading();
    assert!(matches!(
        console.background_transition_state(),
        crate::gameui::loading::BackgroundTransitionState::Loading { .. }
    ));

    console.level_loaded(true);
    assert!(!console.is_background_loading());
    assert!(matches!(
        console.background_transition_state(),
        crate::gameui::loading::BackgroundTransitionState::Finished { .. }
    ));
    assert!(console.background_transition_state().is_terminal());

    console.draw(0.0);
    let quads = console.canvas.drain();
    assert!(quads.iter().all(|q| !(q.destination.w == 1920.
        && q.destination.h == 1080.
        && q.color == crate::gameui::loading::BackgroundTransitionState::MAIN_MENU_BACKDROP)));
}

#[test]
fn background_load_failed_ends_transition_and_cleans_up() {
    let mut console = Console {
        mode: Mode::MainMenu,
        base: Mode::MainMenu,
        ..Default::default()
    };
    console.canvas.resize(1920., 1080.);

    console.start_background_loading("lostcoast", "background01");
    assert!(console.is_background_loading());

    console.background_load_failed("Failed to open BSP file");
    assert!(!console.is_background_loading());
    assert_eq!(
        console.background_transition_state(),
        &crate::gameui::loading::BackgroundTransitionState::Failed {
            map: "background01".into(),
            displayed_category: "lostcoast".into(),
            reason: "Failed to open BSP file".into(),
        }
    );
    assert!(console.background_transition_state().is_terminal());
    assert!(console.background_transition_state().is_failed());

    console.draw(0.0);
    let quads = console.canvas.drain();
    assert!(
        quads.iter().all(|q| !(q.destination.w == 1920.
            && q.destination.h == 1080.
            && q.color == crate::gameui::loading::BackgroundTransitionState::MAIN_MENU_BACKDROP)),
        "no backdrop overlay quads after background load failure"
    );
}

#[test]
fn generic_loading_failed_on_background_cleans_up() {
    let mut console = Console {
        mode: Mode::MainMenu,
        base: Mode::MainMenu,
        ..Default::default()
    };
    console.canvas.resize(1920., 1080.);

    console.start_background_loading("episodic", "ep1_background01");
    assert!(console.is_background_loading());

    // Generic loading_failed on a background map must dismiss the dialog and mark transition failed
    console.loading_failed("Corrupt lump header");
    assert!(!console.is_background_loading());
    assert!(console.background_transition_state().is_failed());
    assert!(console.loading_dialog.is_none());

    console.draw(0.0);
    let quads = console.canvas.drain();
    assert!(quads.iter().all(|q| !(q.destination.w == 1920.
        && q.destination.h == 1080.
        && q.color == crate::gameui::loading::BackgroundTransitionState::MAIN_MENU_BACKDROP)));
}

#[test]
fn menu_buttons_draw_letters_with_border_along_edge_path_and_no_box_fill() {
    let mut console = Console::default();
    console.canvas.resize(1280., 720.);
    console.set_mode(Mode::MainMenu);
    console.title = String::new();
    let font_bytes = source_ui::PROGGY_CLEAN;
    console.menu_font = FontFace::load(font_bytes, console.canvas.clone()).ok();
    assert!(console.menu_font.is_some());

    // 1. Critical requirement: Verify that NO rectangular background boxes or fills are drawn
    // around the button rows (only textured glyph quads).
    console.draw(0.0);
    let quads = console.canvas.drain();

    let untextured_quads: Vec<_> = quads.iter().filter(|q| q.texture.is_none()).collect();
    assert!(
        untextured_quads.is_empty(),
        "menu button clickable area must not have rectangular box or fill quads: found {:?}",
        untextured_quads.len()
    );

    // 2. Verify outline quads are emitted along the letter paths with semi-transparent black
    let default_border = Color::new(0.0, 0.0, 0.0, 0.45);
    let outline_quads: Vec<_> = quads.iter().filter(|q| q.color == default_border).collect();
    assert!(
        !outline_quads.is_empty(),
        "menu buttons must emit outline quads along letter paths with default border color"
    );
    assert!(
        outline_quads.iter().all(|q| q.texture.is_some()),
        "outline quads must be textured glyphs tracing the letter contour"
    );

    // For each character drawn, there must be 8 outline quads (offsets) per 1 foreground quad
    let fg_quads: Vec<_> = quads
        .iter()
        .filter(|q| q.color == console.style.menu)
        .collect();
    assert!(
        !fg_quads.is_empty(),
        "menu buttons must emit foreground glyph quads"
    );
    assert_eq!(
        outline_quads.len(),
        fg_quads.len() * 8,
        "each letter must emit exactly 8 outline glyph quads (1px in 8 directions) for its contour"
    );

    // 3. Armed state (hovered/selected):
    // When selection is set, the selected item's letters use armed border color (alpha 0.65)
    // and armed foreground color.
    console.main_menu.selection = Some(0);
    console.draw(0.0);
    let quads = console.canvas.drain();

    let armed_border = Color::new(0.0, 0.0, 0.0, 0.65);
    let armed_outline_quads: Vec<_> = quads.iter().filter(|q| q.color == armed_border).collect();
    let armed_fg_quads: Vec<_> = quads
        .iter()
        .filter(|q| q.color == console.style.menu_armed)
        .collect();
    assert!(
        !armed_outline_quads.is_empty(),
        "armed menu button must use armed outline color (alpha 0.65)"
    );
    assert_eq!(
        armed_outline_quads.len(),
        armed_fg_quads.len() * 8,
        "armed item must have 8 outline quads per foreground letter quad"
    );

    // 4. Pressed state offset shift:
    // When pressed, all letter quads (outline and foreground) shift by (1.0, 1.0)
    let unpressed_first_fg_pos = armed_fg_quads[0].destination;
    console.menu_tracker.update(Some(0), true, true, false); // press button 0
    console.draw(0.0);
    let pressed_quads = console.canvas.drain();
    let pressed_fg_quads: Vec<_> = pressed_quads
        .iter()
        .filter(|q| q.color == console.style.menu_armed)
        .collect();
    assert_eq!(
        pressed_fg_quads[0].destination.x,
        unpressed_first_fg_pos.x + 1.0
    );
    assert_eq!(
        pressed_fg_quads[0].destination.y,
        unpressed_first_fg_pos.y + 1.0
    );
}

#[test]
fn menu_titles_and_subtitles_draw_letters_with_border_along_edge_path() {
    let mut console = Console::default();
    console.canvas.resize(1280., 720.);
    console.set_mode(Mode::MainMenu);

    let font_bytes = source_ui::PROGGY_CLEAN;
    console.menu_font = FontFace::load(font_bytes, console.canvas.clone()).ok();
    console.menu_metrics = CellMetrics::from_bytes(font_bytes).unwrap_or_default();
    assert!(console.menu_font.is_some());

    let mut loc = Localization::default();
    loc.insert("GameUI_HalfLife2", "Half-Life 2");
    loc.insert("GameUI_EpisodeOne", "Episode One");
    loc.insert("GameUI_EpisodeTwo", "Episode Two");
    loc.insert("GameUI_LostCoast", "Lost Coast");
    console.set_resources(GameUiResources {
        localization: loc.clone(),
        category_titles: resources::fallback_category_titles(&loc),
        ..Default::default()
    });

    let default_border = Color::new(0.0, 0.0, 0.0, 0.45);
    let geometry = console.menu_layout.geometry_for(console.canvas_size());

    for title_font_enabled in [false, true] {
        if title_font_enabled {
            console.title_metrics = CellMetrics::from_bytes(font_bytes).unwrap_or_default();
            console.title_font = FontFace::load(font_bytes, console.canvas.clone()).ok();
            console.title_face = console.title_font.as_ref().map(FontFace::face);
        } else {
            console.title_font = None;
            console.title_face = None;
        }

        // Test all expansion categories: hl2, hl2ep1, hl2ep2, hl2lc
        for (category, has_subtitle) in [
            ("hl2", false),
            ("hl2ep1", true),
            ("hl2ep2", true),
            ("hl2lc", true),
        ] {
            console.main_menu = GameMenu::new(
                vec![
                    MenuItem {
                        label: category.into(),
                        command: format!("SwitchExpansion {category}"),
                        category: Some("main".into()),
                        order: 10,
                        visibility: Visibility::Always,
                        caps: true,
                    },
                    MenuItem {
                        label: "Back".into(),
                        command: "SwitchCategory main".into(),
                        category: Some(category.into()),
                        order: 20,
                        visibility: Visibility::Always,
                        caps: true,
                    },
                ],
                Default::default(),
                &loc,
            );
            console.main_menu.activate(0, &loc);
            assert_eq!(console.main_menu.category(), category);
            console.draw(0.0);
            let quads = console.canvas.drain();

            let title_quads: Vec<_> = quads
                .iter()
                .filter(|q| q.destination.y < geometry.menu.y)
                .collect();

            // 1. No untextured background box or fill quads in title/subtitle area
            let untextured: Vec<_> = title_quads.iter().filter(|q| q.texture.is_none()).collect();
            assert!(
                    untextured.is_empty(),
                    "title/subtitle area for category {category} (title_font={title_font_enabled}) must not have box/fill quads: found {:?}",
                    untextured.len()
                );

            // 2. Outline quads emitted along letter paths with default border color
            let outline_quads: Vec<_> = title_quads
                .iter()
                .filter(|q| q.color == default_border)
                .collect();
            assert!(
                    !outline_quads.is_empty(),
                    "title/subtitle for category {category} (title_font={title_font_enabled}) must emit outline quads"
                );
            assert!(
                outline_quads.iter().all(|q| q.texture.is_some()),
                "outline quads must be textured glyphs tracing the letter contour"
            );

            // 3. Foreground quads: line 0 is WHITE, subtitle (if present) is rgba(255, 255, 255, 200)
            let fg_line0: Vec<_> = title_quads.iter().filter(|q| q.color == WHITE).collect();
            assert!(
                !fg_line0.is_empty(),
                "title line 0 must emit white foreground glyph quads for category {category}"
            );

            let fg_sub: Vec<_> = title_quads
                .iter()
                .filter(|q| q.color == Color::from_rgba(255, 255, 255, 200))
                .collect();
            if has_subtitle {
                assert!(
                        !fg_sub.is_empty(),
                        "subtitle line must emit subtitle foreground glyph quads for category {category}"
                    );
            } else {
                assert!(
                    fg_sub.is_empty(),
                    "category {category} should not emit subtitle foreground quads"
                );
            }

            let total_fg = fg_line0.len() + fg_sub.len();
            assert_eq!(
                    outline_quads.len(),
                    total_fg * 8,
                    "each title/subtitle letter in category {category} (title_font={title_font_enabled}) must emit 8 outline quads"
                );

            // 4. In title_font mode, verify two-pass layering for line 0:
            // All line 0 outline quads must precede all line 0 foreground quads.
            if title_font_enabled {
                let first_fg_line0_pos = title_quads
                    .iter()
                    .position(|q| q.color == WHITE)
                    .expect("foreground quad for line 0");
                let outline_count_before_first_fg = title_quads[..first_fg_line0_pos]
                    .iter()
                    .filter(|q| q.color == default_border)
                    .count();
                assert_eq!(
                    outline_count_before_first_fg,
                    fg_line0.len() * 8,
                    "all line 0 outline quads must be drawn before any line 0 foreground quads"
                );
            }
        }
    }

    // 5. Verify Pause menu also renders title outline
    console.set_mode(Mode::Pause);
    console.pause_menu.set_in_game(true, &loc);
    console.draw(0.0);
    let pause_quads = console.canvas.drain();
    let pause_title_quads: Vec<_> = pause_quads
        .iter()
        .filter(|q| q.destination.y < geometry.menu.y)
        .collect();
    let pause_outline: Vec<_> = pause_title_quads
        .iter()
        .filter(|q| q.color == default_border)
        .collect();
    let pause_fg: Vec<_> = pause_title_quads
        .iter()
        .filter(|q| q.color == WHITE)
        .collect();
    assert!(
        !pause_outline.is_empty(),
        "pause menu must emit title outline quads"
    );
    assert_eq!(
        pause_outline.len(),
        pause_fg.len() * 8,
        "pause menu title letters must each have 8 outline quads"
    );
}

#[test]
fn test_subtitle_font_selection_and_glyph_decoration() {
    let mut console = Console {
        mode: Mode::MainMenu,
        base: Mode::MainMenu,
        ..Default::default()
    };
    console.canvas.resize(1280., 720.);

    let font_bytes = source_ui::PROGGY_CLEAN;
    console.menu_font = FontFace::load(font_bytes, console.canvas.clone()).ok();
    console.menu_metrics = CellMetrics::from_bytes(font_bytes).unwrap_or_default();
    console.title_font = FontFace::load(font_bytes, console.canvas.clone()).ok();
    console.title_face = console.title_font.as_ref().map(FontFace::face);
    console.title_metrics = CellMetrics::from_bytes(font_bytes).unwrap_or_default();

    // 1. Verify title_font_can_render correctly detects glyph coverage
    assert!(console.title_font_can_render("== episode one=="));
    assert!(console.title_font_can_render("== episode two =="));
    assert!(!console.title_font_can_render(""));
    // Character not in standard ASCII font
    assert!(!console.title_font_can_render("λ episode"));

    // 2. Set up category titles
    let mut loc = Localization::default();
    loc.insert("GameUI_HalfLife2", "Half-Life 2");
    let titles = resources::fallback_category_titles(&loc);
    console.set_resources(GameUiResources {
        localization: loc.clone(),
        category_titles: titles,
        ..Default::default()
    });

    // 3. Test episode subtitle drawing with title_font
    console.main_menu = GameMenu::new(
        vec![
            MenuItem {
                label: "Episode One".into(),
                command: "SwitchExpansion episodic".into(),
                category: Some("main".into()),
                order: 10,
                visibility: Visibility::Always,
                caps: true,
            },
            MenuItem {
                label: "Back".into(),
                command: "SwitchCategory main".into(),
                category: Some("episodic".into()),
                order: 20,
                visibility: Visibility::Always,
                caps: true,
            },
        ],
        Default::default(),
        &loc,
    );
    console.main_menu.activate(0, &loc);
    assert_eq!(console.main_menu.category(), "episodic");
    assert_eq!(console.current_title(), "Half-Life 2\n== episode one==");

    console.draw(0.0);
    let quads = console.canvas.drain();
    let geometry = console.menu_layout.geometry_for(console.canvas_size());
    let subtitle_quads: Vec<_> = quads
        .iter()
        .filter(|q| q.destination.y >= geometry.title2.y - 5. && q.destination.y < geometry.menu.y)
        .collect();
    assert!(
        !subtitle_quads.is_empty(),
        "subtitle must emit quads for '== episode one=='"
    );
    let fg_subtitle: Vec<_> = subtitle_quads
        .iter()
        .filter(|q| q.color == Color::from_rgba(255, 255, 255, 200))
        .collect();
    assert!(
        !fg_subtitle.is_empty(),
        "subtitle must have foreground quads with rgba(255, 255, 255, 200)"
    );

    // 4. Test lostcoast subtitle drawing with dedicated lostcoast_title_font
    console.lostcoast_title_font = FontFace::load(font_bytes, console.canvas.clone()).ok();
    console.lostcoast_title_face = console.lostcoast_title_font.as_ref().map(FontFace::face);
    console.lostcoast_title_metrics = CellMetrics::from_bytes(font_bytes).unwrap_or_default();

    console.main_menu = GameMenu::new(
        vec![
            MenuItem {
                label: "Lost Coast".into(),
                command: "SwitchExpansion lostcoast".into(),
                category: Some("main".into()),
                order: 10,
                visibility: Visibility::Always,
                caps: true,
            },
            MenuItem {
                label: "Back".into(),
                command: "SwitchCategory main".into(),
                category: Some("lostcoast".into()),
                order: 20,
                visibility: Visibility::Always,
                caps: true,
            },
        ],
        Default::default(),
        &loc,
    );
    console.main_menu.activate(0, &loc);
    assert_eq!(console.main_menu.category(), "lostcoast");
    assert_eq!(console.current_title(), "Half-Life 2\nlost coasT");

    console.draw(0.0);
    let lc_quads = console.canvas.drain();
    let lc_subtitle_quads: Vec<_> = lc_quads
        .iter()
        .filter(|q| q.destination.y >= geometry.title2.y - 5. && q.destination.y < geometry.menu.y)
        .collect();
    assert!(
        !lc_subtitle_quads.is_empty(),
        "lost coast subtitle must emit quads for 'lost coasT'"
    );
}

#[test]
fn test_subtitle_applied_after_background_loading_finishes_or_fails() {
    let mut console = installed_console();
    let mut loc = Localization::default();
    loc.insert("GameUI_HalfLife2", "Half-Life 2");
    loc.insert("GameUI_EpisodeOne", "Episode One");
    loc.insert("GameUI_EpisodeTwo", "Episode Two");
    loc.insert("GameUI_LostCoast", "Lost Coast");
    console.category_titles = resources::fallback_category_titles(&loc);
    console.set_mode(Mode::MainMenu);
    console.canvas.resize(1920., 1080.);

    // Initial default title state
    assert_eq!(console.current_title(), "Half-Life 2");
    assert_eq!(
        console.background_transition_state(),
        &crate::gameui::loading::BackgroundTransitionState::Idle
    );

    // --- Sequence 1: Select Episode One -> loading (old title shown) -> finished (new title shown) ---
    // Row 1 in fallback_game_menu() is Episode One ("SwitchExpansion episodic")
    click_row(&mut console, 1);
    assert_eq!(console.main_menu.category(), "episodic");
    assert!(console.is_background_loading());
    assert!(console.background_transition_state().is_active());
    assert_eq!(
        console.background_transition_state().target_map(),
        Some("ep1_background01")
    );
    // While in Starting / Loading, the old title MUST still be shown
    assert_eq!(
        console.current_title(),
        "Half-Life 2",
        "old title must remain visible while background map is starting"
    );

    console.update_loading();
    assert!(console.is_background_loading());
    assert_eq!(
        console.current_title(),
        "Half-Life 2",
        "old title must remain visible while background map is loading"
    );

    // Finish background loading for Episode One
    console.level_loaded_for_map("episodic", "ep1_background01", true);
    assert!(!console.is_background_loading());
    assert!(console.background_transition_state().is_finished());
    // Now new title is applied!
    assert_eq!(
        console.current_title(),
        "Half-Life 2\n== episode one==",
        "new subtitle must be applied after background loading finishes"
    );

    // --- Sequence 2: Select Episode Two -> load failure (new title still applied) ---
    // Enter Episode Two ("SwitchExpansion ep2")
    console.start_background_loading("ep2", "ep2_background01");
    assert!(console.is_background_loading());
    assert!(console.background_transition_state().is_active());
    // While loading Episode Two, previously displayed title (Episode One) remains visible
    assert_eq!(
        console.current_title(),
        "Half-Life 2\n== episode one==",
        "previous subtitle must remain visible while new background map is loading"
    );

    // Fail Episode Two load
    console.background_load_failed("BSP corrupted or missing");
    assert!(!console.is_background_loading());
    assert!(console.background_transition_state().is_failed());
    // Failure must still apply the new title so it does not remain stuck on the old title
    assert_eq!(
        console.current_title(),
        "Half-Life 2\n== episode two ==",
        "new title must still be applied when background load fails"
    );

    // --- Sequence 3: Fast double switch (latest wins, stale completion ignored) ---
    // Switch to Episode One first
    console.start_background_loading("episodic", "ep1_background01");
    assert_eq!(
        console.background_transition_state().target_map(),
        Some("ep1_background01")
    );
    assert_eq!(
        console.current_title(),
        "Half-Life 2\n== episode two ==",
        "Episode Two title remains visible while Episode One starts loading"
    );

    // User fast double switches to Lost Coast before Episode One finishes
    console.start_background_loading("lostcoast", "background01");
    assert_eq!(
        console.background_transition_state().target_map(),
        Some("background01")
    );
    assert_eq!(
        console.current_title(),
        "Half-Life 2\n== episode two ==",
        "Episode Two title still visible while Lost Coast is loading"
    );

    // Stale completion arrives for Episode One
    console.level_loaded_for_map("episodic", "ep1_background01", true);
    // Stale completion must be ignored; transition must still be active for Lost Coast
    assert!(
        console.is_background_loading(),
        "stale Episode One completion must not end active Lost Coast transition"
    );
    assert_eq!(
        console.background_transition_state().target_category(),
        Some("lostcoast")
    );
    // A stale Half-Life 2 `background01` completion is not Lost Coast's `background01`.
    console.level_loaded_for_map("hl2", "background01", true);
    assert!(console.is_background_loading());
    assert_eq!(
        console.current_title(),
        "Half-Life 2\n== episode two ==",
        "stale completion must not apply Episode One title"
    );

    // Lost Coast finishes
    console.level_loaded_for_map("lostcoast", "background01", true);
    assert!(!console.is_background_loading());
    assert!(console.background_transition_state().is_finished());
    assert_eq!(
        console.current_title(),
        "Half-Life 2\nlost coasT",
        "Lost Coast subtitle must be applied (latest wins)"
    );
}

#[test]
fn test_console_background_texture_resolution() {
    let mut console = Console::default();
    let texture =
        |width: u32| Texture2D::from_rgba8(width as u16, 2, &vec![255; width as usize * 2 * 4]);
    for (game, image, width) in [
        ("hl2", "background01", 2),
        ("hl2", "background01_widescreen", 4),
        ("lostcoast", "background01", 20),
        ("ep2", "ep2_background01", 2),
    ] {
        console
            .background_textures
            .insert((game.into(), image.into()), texture(width));
    }
    let width = |game, map, wide| {
        console
            .background_texture(game, map, wide)
            .map(|t| t.width())
    };
    assert_eq!(width("hl2", "background01", false), Some(2.0));
    assert_eq!(width("hl2", "background01.bsp", true), Some(4.0));
    // Lost Coast's `background01` is its own image, with no widescreen variant.
    assert_eq!(width("lostcoast", "background01", false), Some(20.0));
    assert_eq!(width("lostcoast", "background01", true), Some(20.0));
    assert_eq!(width("ep2", "ep2_background01", true), Some(2.0));
    assert_eq!(width("episodic", "ep2_background01", false), None);
}

#[test]
fn con_enable_off_keeps_the_console_closed_but_lets_it_close() {
    let mut console = Console::default();
    console.set_mode(Mode::MainMenu);
    console.config.keyboard.developer_console = false;
    console.toggle();
    assert_eq!(console.mode(), Mode::MainMenu, "con_enable 0");
    console.config.keyboard.developer_console = true;
    console.toggle();
    assert_eq!(console.mode(), Mode::Console);
    console.config.keyboard.developer_console = false;
    console.toggle();
    assert_eq!(
        console.mode(),
        Mode::MainMenu,
        "an open console still closes"
    );
}

// ---- non-modal windows (Options, console) over the menu ----

fn menu_with_options(height: f32) -> Console {
    let mut console = installed_console();
    console.set_mode(Mode::MainMenu);
    console.canvas.resize(height * 16. / 9., height);
    console.open_options();
    console
}

fn row_of(console: &Console, command: MenuCommand) -> usize {
    console
        .main_menu
        .entries()
        .iter()
        .position(|e| e.command == command)
        .expect("menu row")
}

/// A point of the Options dialog, from layout units inside its frame.
fn in_options(console: &Console, local: Vec2) -> Vec2 {
    let bounds = console.window_bounds(Window::Options);
    let scale = crate::proportional::dialog_scale(console.canvas_size());
    vec2(bounds.x, bounds.y) + local * scale
}

fn pointer_frame(
    console: &mut Console,
    at: Vec2,
    click: bool,
    held: bool,
    released: bool,
) -> Vec<Effect> {
    console.input(&Input {
        pointer: Some(at),
        click,
        held,
        released,
        ..Default::default()
    })
}

/// Press at `from`, drag over `via`, release at `to`; every effect of the gesture.
fn gesture(console: &mut Console, from: Vec2, via: Vec2, to: Vec2) -> Vec<Effect> {
    let mut effects = pointer_frame(console, from, true, true, false);
    effects.extend(pointer_frame(console, via, false, true, false));
    effects.extend(pointer_frame(console, to, false, true, false));
    effects.extend(pointer_frame(console, to, false, false, true));
    effects
}

fn menu_sounds(effects: &[Effect]) -> bool {
    effects.iter().any(|e| {
            matches!(e, Effect::PlaySound(s) if s == "ui/buttonrollover.wav" || s == "ui/buttonclickrelease.wav")
        })
}

#[test]
fn options_is_not_modal_and_the_menu_beside_it_takes_clicks() {
    let mut console = menu_with_options(720.);
    assert!(
        !console.modal_dialog_open(),
        "Options is a window, not a modal dialog"
    );
    let episode = row_of(&console, MenuCommand::SwitchExpansion("episodic".into()));
    let at = row(&console, episode).unwrap();
    assert!(
        !console.window_bounds(Window::Options).contains(at),
        "the row is beside the dialog"
    );
    let hover = pointer_frame(&mut console, at, false, false, false);
    assert!(hover.contains(&Effect::PlaySound("ui/buttonrollover.wav".into())));
    let effects = gesture(&mut console, at, at, at);
    assert!(effects.contains(&Effect::PlaySound("ui/buttonclickrelease.wav".into())));
    assert_eq!(
        console.main_menu.category(),
        "episodic",
        "the menu item ran"
    );
    assert!(console.options_open(), "the dialog stays open");
    assert_eq!(console.mode(), Mode::MainMenu, "the click focused the menu");
}

#[test]
fn a_press_on_options_released_over_the_menu_never_reaches_the_menu() {
    let mut console = menu_with_options(720.);
    let row0 = row(&console, 0).unwrap();
    let cancel = button_center(&console, 1);
    let effects = gesture(&mut console, cancel, row0, row0);
    assert!(!menu_sounds(&effects), "{effects:?}");
    assert_eq!(console.last_menu_hover, None, "the menu never armed");
    assert!(
        console.options_open(),
        "Cancel released off the button does nothing"
    );
    assert_eq!(console.mode(), Mode::Options);
}

fn button_center(console: &Console, index: usize) -> Vec2 {
    let rect = crate::gameui::options::button_rects()[index];
    in_options(console, vec2(rect.x + rect.w * 0.5, rect.y + rect.h * 0.5))
}

#[test]
fn a_menu_press_dragged_onto_options_activates_nothing() {
    let mut console = menu_with_options(720.);
    let quit = row(&console, row_of(&console, MenuCommand::Quit)).unwrap();
    let cancel = button_center(&console, 1);
    let effects = gesture(&mut console, quit, cancel, cancel);
    assert!(
        !effects.contains(&Effect::PlaySound("ui/buttonclickrelease.wav".into())),
        "{effects:?}"
    );
    assert!(
        console.confirm_box.is_none(),
        "the menu press was cancelled off its row"
    );
    assert!(console.options_open(), "and Options' Cancel never saw it");
}

#[test]
fn dragging_the_options_caption_over_the_menu_moves_it_at_any_scale() {
    for height in [720., 1440.] {
        let mut console = menu_with_options(height);
        let before = console.window_bounds(Window::Options);
        let caption = in_options(&console, vec2(150., 12.));
        let row0 = row(&console, 0).unwrap();
        let effects = gesture(&mut console, caption, row0, row0);
        assert!(!menu_sounds(&effects), "{height}: {effects:?}");
        let after = console.window_bounds(Window::Options);
        assert_ne!(
            (after.x, after.y),
            (before.x, before.y),
            "{height}: it moved"
        );
        assert!(
            after.contains(row0) || after.x < before.x,
            "{height}: towards the menu"
        );
        // The pointer now hits the dialog where it was dragged.
        let inside = vec2(after.x + 20., after.y + after.h * 0.5);
        assert!(console.is_occluded(inside));
        let effects = pointer_frame(&mut console, inside, false, false, false);
        assert!(
            !menu_sounds(&effects),
            "{height}: hover over the moved dialog"
        );
    }
}

#[test]
fn a_modal_layer_of_options_takes_every_click() {
    let mut console = menu_with_options(720.);
    console
        .options
        .command(crate::gameui::settings::Command::VideoCredits);
    assert!(console.options.modal_layer_open());
    let episode = row_of(&console, MenuCommand::SwitchExpansion("episodic".into()));
    let at = row(&console, episode).unwrap();
    let effects = gesture(&mut console, at, at, at);
    assert!(!menu_sounds(&effects), "{effects:?}");
    assert_eq!(
        console.main_menu.category(),
        "main",
        "the menu stayed covered"
    );
}

#[test]
fn quit_from_the_menu_is_modal_over_options_and_cancel_returns_to_it() {
    let mut console = menu_with_options(720.);
    let quit = row(&console, row_of(&console, MenuCommand::Quit)).unwrap();
    gesture(&mut console, quit, quit, quit);
    assert!(console.confirm_box.is_some());
    assert!(console.modal_dialog_open());
    // A click on Options outside the box reaches nothing.
    let quit_box = console.modal_bounds().expect("the quit box");
    let cancel = button_center(&console, 1);
    assert!(!quit_box.contains(cancel));
    gesture(&mut console, cancel, cancel, cancel);
    assert!(console.confirm_box.is_some(), "the box stays");
    assert!(console.options_open(), "Options' Cancel did not run");
    console.escape();
    assert!(console.confirm_box.is_none());
    assert!(console.options_open(), "Options is still there");
}

#[test]
fn a_submenu_background_loading_behind_options_keeps_it_open() {
    let mut console = menu_with_options(720.);
    console.shell.windows.focus_base();
    console.sync_mode();
    console.level_loaded(true);
    assert!(console.options_open());
    assert_eq!(console.mode(), Mode::MainMenu);
    console.level_loaded(false);
    assert!(!console.options_open(), "a gameplay map closes the windows");
    assert_eq!(console.mode(), Mode::Gameplay);
}

#[test]
fn options_again_from_the_menu_keeps_the_open_dialog_as_it_is() {
    let mut console = menu_with_options(720.);
    keys(&mut console, &[Key::Tab]);
    assert_eq!(console.options.report().focus.as_deref(), Some("tab"));
    let options = row(&console, row_of(&console, MenuCommand::OpenOptions)).unwrap();
    gesture(&mut console, options, options, options);
    assert_eq!(console.mode(), Mode::Options, "raised and focused");
    assert_eq!(
        console.options.report().focus.as_deref(),
        Some("tab"),
        "not reopened"
    );
}

#[test]
fn resume_from_the_pause_menu_closes_options() {
    let mut console = installed_console();
    console.canvas.resize(1280., 720.);
    console.set_mode(Mode::Pause);
    console.open_options();
    let resume = console
        .pause_menu
        .entries()
        .iter()
        .position(|e| e.command == MenuCommand::Resume)
        .expect("resume row");
    console.set_mode(Mode::Pause);
    console.open_options();
    console.activate_menu(resume);
    assert!(!console.options_open());
    assert_eq!(console.mode(), Mode::Gameplay);
}

#[test]
fn the_topmost_window_takes_the_overlap_and_a_press_raises_the_other() {
    let mut console = menu_with_options(720.);
    console.toggle();
    assert_eq!(console.mode(), Mode::Console);
    let options = console.window_bounds(Window::Options);
    let console_rect = console.window_bounds(Window::Console);
    let overlap = (0..40)
        .flat_map(|i| (0..40).map(move |j| vec2(i as f32 * 32., j as f32 * 18.)))
        .find(|p| options.contains(*p) && console_rect.contains(*p))
        .expect("the windows overlap");
    let options_only = (0..40)
        .flat_map(|i| (0..40).map(move |j| vec2(i as f32 * 32., j as f32 * 18.)))
        .find(|p| options.contains(*p) && !console_rect.contains(*p))
        .expect("Options shows beside the console");
    pointer_frame(&mut console, overlap, true, true, false);
    pointer_frame(&mut console, overlap, false, false, true);
    assert_eq!(console.mode(), Mode::Console, "the console is on top there");
    pointer_frame(&mut console, options_only, true, true, false);
    pointer_frame(&mut console, options_only, false, false, true);
    assert_eq!(
        console.mode(),
        Mode::Options,
        "the press raised and focused Options"
    );
    assert_eq!(console.shell.windows.top(), Some(Window::Options));
    assert!(console.console_open(), "the console stays open below");
    keys(&mut console, &[Key::Tab]);
    assert_eq!(
        console.options.report().focus.as_deref(),
        Some("tab"),
        "the keyboard follows the focus"
    );
}

#[test]
fn the_console_completion_popup_past_its_frame_is_part_of_the_console() {
    let mut console = installed_console();
    console.set_mode(Mode::MainMenu);
    console.canvas.resize(1280., 720.);
    console.toggle();
    // At the top of the canvas, so the list opens below the entry line, past the frame.
    console.shell.panel.placement =
        Placement::from_saved(Some(source_ui::placement::DialogPlacement {
            offset: [0., -2000.],
            size: None,
        }));
    console.input(&Input {
        text: "s".into(),
        ..Default::default()
    });
    let popup = console
        .completion_bounds()
        .expect("the completion popup shows");
    let frame = console.window_bounds(Window::Console);
    let outside = (0..64)
        .flat_map(|i| (0..64).map(move |j| (i, j)))
        .map(|(i, j)| {
            vec2(
                popup.x + popup.w * i as f32 / 64.,
                popup.y + popup.h * j as f32 / 64.,
            )
        })
        .find(|p| popup.contains(*p) && !frame.contains(*p));
    // It opens below the entry line, at the frame's bottom: past the frame.
    let at = outside.expect("the popup reaches past the frame");
    assert!(console.is_occluded(at));
    let effects = gesture(&mut console, at, at, at);
    assert!(!menu_sounds(&effects), "{effects:?}");
    assert_eq!(console.last_menu_hover, None);
    assert_eq!(
        console.mode(),
        Mode::Console,
        "the press stayed with the console"
    );
}

#[test]
fn without_a_pointer_nothing_on_the_menu_arms() {
    let mut console = menu_with_options(720.);
    let effects = console.input(&Input {
        click: true,
        held: true,
        ..Default::default()
    });
    assert!(!menu_sounds(&effects));
    assert_eq!(console.last_menu_hover, None);
}

#[test]
fn an_unfocused_window_fades_to_its_out_of_focus_look() {
    let mut console = menu_with_options(720.);
    let focused = console.window_theme(Window::Options, 0.).frame_bg;
    assert!((focused.a - console.theme.frame_bg.a).abs() < 1e-4);
    let episode = row_of(&console, MenuCommand::SwitchExpansion("episodic".into()));
    let at = row(&console, episode).unwrap();
    console.input(&Input {
        pointer: Some(at),
        click: true,
        held: true,
        time: Some(10.),
        ..Default::default()
    });
    assert_eq!(console.mode(), Mode::MainMenu);
    let half = console.window_theme(Window::Options, 10.15).frame_bg.a;
    let out = console.window_theme(Window::Options, 10.5);
    assert!(
        half < focused.a && half > out.frame_bg.a,
        "fades over 0.3 s"
    );
    assert!((out.frame_bg.a - console.theme.frame_bg_out_of_focus.a).abs() < 1e-4);
    assert_eq!(out.title_text, console.theme.title_text_out_of_focus);
}

#[test]
fn escape_closes_the_top_window_even_while_the_menu_has_the_focus() {
    let mut console = menu_with_options(720.);
    console.toggle();
    console.shell.windows.focus_base();
    console.sync_mode();
    console.escape();
    assert!(!console.console_open(), "the console was on top");
    assert!(console.options_open());
    console.escape();
    assert!(!console.options_open());
    assert_eq!(console.mode(), Mode::MainMenu);
}

#[test]
fn main_menu_loses_focus_and_becomes_translucent_when_dialog_opens_and_restores_on_close() {
    let mut console = installed_console();
    console.set_mode(Mode::MainMenu);
    console.canvas.resize(1280., 720.);

    // Initially main menu has focus
    assert_eq!(console.shell.windows.focus(), Layer::Base);
    assert_eq!(console.shell.windows.base_focus_amount(0.0), 1.0);

    // Open Options dialog: main menu loses focus and becomes translucent
    console.open_options();
    console.shell.windows.set_time(0.0);
    assert_eq!(
        console.shell.windows.focus(),
        Layer::Window(Window::Options)
    );
    assert_eq!(
        console.shell.windows.focus_amount(Window::Options, 0.0),
        1.0
    );
    // Base fades to 0.0
    assert!((console.shell.windows.base_focus_amount(0.3) - 0.0).abs() < 1e-4);

    // Open SubDialog inside Options
    console
        .options
        .open_sub(crate::gameui::options::Sub::VideoAdvanced);
    console.sync_dialog_windows();
    console.shell.windows.set_time(1.0);
    assert_eq!(
        console.shell.windows.focus(),
        Layer::Window(Window::SubDialog)
    );
    // Parent options is now unfocused
    assert!((console.shell.windows.focus_amount(Window::Options, 1.3) - 0.0).abs() < 1e-4);

    // Close SubDialog: restores focus to Options
    console.options.cancel_sub();
    console.sync_dialog_windows();
    console.shell.windows.set_time(2.0);
    assert_eq!(
        console.shell.windows.focus(),
        Layer::Window(Window::Options)
    );
    assert!((console.shell.windows.focus_amount(Window::Options, 2.3) - 1.0).abs() < 1e-4);

    // Close Options: restores focus to MainMenu (Base)
    shell::close_window(&mut console, Window::Options);
    console.sync_dialog_windows();
    console.shell.windows.set_time(3.0);
    assert_eq!(console.shell.windows.focus(), Layer::Base);
    assert!((console.shell.windows.base_focus_amount(3.3) - 1.0).abs() < 1e-4);
}

#[test]
fn subdialog_in_focus_renders_focused_frame_while_parent_options_is_unfocused() {
    let mut console = Console::default();
    console.open_options();
    console.tick(0.0);

    // 1. Options alone holds focus:
    let options_theme = console.window_theme(Window::Options, 0.0);
    assert!((options_theme.frame_bg.a - console.theme.frame_bg.a).abs() < 1e-4);

    // 2. Open SubDialog through input (matching user click on Advanced button):
    console
        .options
        .open_sub(crate::gameui::options::Sub::VideoAdvanced);
    let input = source_ui::input::Input {
        time: Some(1.0),
        ..Default::default()
    };
    console.input(&input);
    // Frame 0: SubDialog focused, parent Options theme starts smooth fade at 1.0 (no flicker to 0.0):
    assert_eq!(
        console.shell.windows.focus(),
        Layer::Window(Window::SubDialog)
    );
    let f0_theme = console.window_theme(Window::Options, 1.0);
    assert!((f0_theme.frame_bg.a - console.theme.frame_bg.a).abs() < 1e-4);
    let sub_theme = console.options.sub_theme.as_ref().expect("sub theme");
    assert!((sub_theme.frame_bg.a - console.theme.frame_bg.a).abs() < 1e-4);
    assert_eq!(sub_theme.title_text, console.theme.title_text_focused);

    // After 0.3s fade-in transition:
    console.tick(1.3);

    // Parent Options is out of focus:
    let options_theme = console.window_theme(Window::Options, 1.3);
    assert!((options_theme.frame_bg.a - console.theme.frame_bg_out_of_focus.a).abs() < 1e-4);
    assert_eq!(
        options_theme.title_text,
        console.theme.title_text_out_of_focus
    );

    // SubDialog composite alpha is 1.0:
    assert!((console.options.sub_alpha - 1.0).abs() < 1e-4);

    // 3. Open confirmation box over SubDialog:
    console
        .options
        .command(crate::gameui::settings::Command::KeyDefaults);
    console.tick(2.0);
    assert_eq!(
        console.shell.windows.focus(),
        Layer::Window(Window::Confirmation)
    );

    // After 0.3s fade-in transition:
    console.tick(2.3);

    // Confirmation box is in focus:
    let confirm_theme = console
        .options
        .confirm_theme
        .as_ref()
        .expect("confirm theme");
    assert!(
        (confirm_theme.frame_bg.a - console.theme.frame_bg.a).abs() < 1e-4,
        "confirmation box has focused Frame.BgColor (alpha={})",
        confirm_theme.frame_bg.a
    );
    assert_eq!(confirm_theme.title_text, console.theme.title_text_focused);

    // SubDialog transitions to out of focus:
    let sub_theme_unfocused = console.options.sub_theme.as_ref().expect("sub theme");
    assert!(
        (sub_theme_unfocused.frame_bg.a - console.theme.frame_bg_out_of_focus.a).abs() < 1e-4,
        "subdialog under confirmation box has out-of-focus Frame.OutOfFocusBgColor (alpha={})",
        sub_theme_unfocused.frame_bg.a
    );
    assert_eq!(
        sub_theme_unfocused.title_text,
        console.theme.title_text_out_of_focus
    );
}

#[test]
fn context_aware_background_dim_overlay_across_all_four_cases() {
    let backdrop = crate::tuning::backdrop_alpha();
    let gameplay_dim = crate::tuning::gameplay_dim_alpha();

    // 1. Menu background focused: no dim (opacity 0.0)
    assert_eq!(Console::dim_overlay_opacity(false, true, 1.0), 0.0);

    let mut console = installed_console();
    console.background_level = true;
    console.set_mode(Mode::MainMenu);
    console.shell.windows.set_time(0.0);
    assert_eq!(console.dim_overlay_alpha(0.0), 0.0);

    // 2. Menu background unfocused with dialog open: dim active (backdrop opacity ~0.6118)
    assert!((Console::dim_overlay_opacity(false, true, 0.0) - backdrop).abs() < 1e-4);

    console.open_options();
    console.shell.windows.set_time(0.0);
    // At t=0.0: just opened, base_focus is still 1.0, dim is 0.0
    assert_eq!(console.dim_overlay_alpha(0.0), 0.0);
    // At t=0.15 (halfway): base_focus is 0.5, dim is halfway
    assert!((console.dim_overlay_alpha(0.15) - backdrop * 0.5).abs() < 1e-4);
    // At t=0.30 (complete): base_focus is 0.0, dim is full backdrop
    assert!((console.dim_overlay_alpha(0.30) - backdrop).abs() < 1e-4);

    // Close options at t=1.0: restores focus to menu, fading dim out
    console.shell.windows.set_time(1.0);
    shell::close_window(&mut console, Window::Options);
    console.sync_dialog_windows();
    assert!((console.dim_overlay_alpha(1.0) - backdrop).abs() < 1e-4);
    assert!((console.dim_overlay_alpha(1.15) - backdrop * 0.5).abs() < 1e-4);
    assert!((console.dim_overlay_alpha(1.30) - 0.0).abs() < 1e-4);

    // 3. Mid-gameplay main menu open and focused: gameplay dim active (0.3)
    assert!((Console::dim_overlay_opacity(true, true, 1.0) - gameplay_dim).abs() < 1e-4);

    let mut play_console = installed_console();
    play_console.background_level = false;
    play_console.set_mode(Mode::Pause);
    play_console.shell.windows.set_time(2.0);
    assert!((play_console.dim_overlay_alpha(2.0) - gameplay_dim).abs() < 1e-4);

    // 4. Mid-gameplay with a dialog open: coordinated transition to backdrop dim (~0.6118)
    assert!((Console::dim_overlay_opacity(true, true, 0.0) - backdrop).abs() < 1e-4);

    play_console.open_options();
    play_console.shell.windows.set_time(2.0);
    // At t=2.0: options just opened, dim starts at gameplay_dim
    assert!((play_console.dim_overlay_alpha(2.0) - gameplay_dim).abs() < 1e-4);
    // At t=2.15: halfway between gameplay_dim and backdrop
    let mid_dim = gameplay_dim + (backdrop - gameplay_dim) * 0.5;
    assert!((play_console.dim_overlay_alpha(2.15) - mid_dim).abs() < 1e-4);
    // At t=2.30: fully faded to backdrop
    assert!((play_console.dim_overlay_alpha(2.30) - backdrop).abs() < 1e-4);

    // Close options at t=3.0: fades back down to gameplay_dim
    play_console.shell.windows.set_time(3.0);
    shell::close_window(&mut play_console, Window::Options);
    play_console.sync_dialog_windows();
    assert!((play_console.dim_overlay_alpha(3.0) - backdrop).abs() < 1e-4);
    assert!((play_console.dim_overlay_alpha(3.15) - mid_dim).abs() < 1e-4);
    assert!((play_console.dim_overlay_alpha(3.30) - gameplay_dim).abs() < 1e-4);

    // 5. Active gameplay without menu: no dim
    assert_eq!(Console::dim_overlay_opacity(false, false, 1.0), 0.0);
    assert_eq!(Console::dim_overlay_opacity(true, false, 1.0), 0.0);
}

#[test]
fn mid_gameplay_tilde_dev_console_transition_and_pause_menu_focus() {
    let mut console = installed_console();
    // 1. Start in active gameplay (not background level)
    console.level_loaded(false);
    assert_eq!(console.mode(), Mode::Gameplay);
    assert_eq!(console.base, Mode::Gameplay);
    assert!(!console.paused());
    assert_eq!(console.shell.windows.focus(), Layer::Base);

    // 2. Press tilde mid-gameplay: opens main menu as active base (paused) with dev console focused
    console.toggle();
    assert_eq!(console.mode(), Mode::Console);
    assert_eq!(console.base, Mode::Pause);
    assert!(console.paused());
    assert_eq!(
        console.shell.windows.focus(),
        Layer::Window(Window::Console)
    );

    // 3. Close the dev console via toggle (pressing tilde again):
    // Leaves the main/pause menu open, active, and focused
    console.toggle();
    assert_eq!(console.mode(), Mode::Pause);
    assert_eq!(console.base, Mode::Pause);
    assert!(console.paused());
    assert_eq!(console.shell.windows.focus(), Layer::Base);

    // 4. Pressing tilde again from the pause menu opens dev console over pause menu
    console.toggle();
    assert_eq!(console.mode(), Mode::Console);
    assert_eq!(console.base, Mode::Pause);
    assert_eq!(
        console.shell.windows.focus(),
        Layer::Window(Window::Console)
    );

    // 5. Closing dev console via Escape also leaves main/pause menu active and focused
    console.escape_pressed();
    console.escape_released();
    assert_eq!(console.mode(), Mode::Pause);
    assert_eq!(console.base, Mode::Pause);
    assert!(console.paused());
    assert_eq!(console.shell.windows.focus(), Layer::Base);

    // 6. Resuming from the main/pause menu returns to active gameplay
    console.escape_pressed();
    console.escape_released();
    assert_eq!(console.mode(), Mode::Gameplay);
    assert_eq!(console.base, Mode::Gameplay);
    assert!(!console.paused());
    assert_eq!(console.shell.windows.focus(), Layer::Base);
}

#[test]
fn subdialog_buttons_can_be_activated_by_mouse_click() {
    let mut console = menu_with_options(720.);
    console.set_resources((*crate::gameui::options::tests::resources()).clone());
    console.open_options();

    let scale = crate::proportional::dialog_scale(console.canvas_size());

    // Test 1: Sub::VideoAdvanced OK button click
    console
        .options
        .open_sub(crate::gameui::options::Sub::VideoAdvanced);
    console.sync_dialog_windows();
    assert!(console.options.has_sub());
    assert!(console.shell.windows.is_active(Window::SubDialog));
    let bounds = console.window_bounds(Window::SubDialog);
    assert!(bounds.w > 0. && bounds.h > 0.);
    // OK button in test layout VIDEO_ADVANCED is at (290, 316, 80, 24)
    let ok_pt = vec2(bounds.x, bounds.y) + (vec2(290. + 40., 316. + 12.) * scale);

    pointer_frame(&mut console, ok_pt, true, true, false);
    assert!(console.options.has_sub(), "still open while pressed");
    pointer_frame(&mut console, ok_pt, false, false, true);
    assert!(!console.options.has_sub(), "closed on OK click release");
    assert!(console.options.closing_sub.is_some());

    // Test 2: Sub::VideoAdvanced Cancel button click
    console
        .options
        .open_sub(crate::gameui::options::Sub::VideoAdvanced);
    console.sync_dialog_windows();
    assert!(console.options.has_sub());
    let bounds = console.window_bounds(Window::SubDialog);
    // Cancel button in test layout VIDEO_ADVANCED is at (382, 316, 80, 24)
    let cancel_pt = vec2(bounds.x, bounds.y) + (vec2(382. + 40., 316. + 12.) * scale);

    pointer_frame(&mut console, cancel_pt, true, true, false);
    pointer_frame(&mut console, cancel_pt, false, false, true);
    assert!(!console.options.has_sub(), "closed on Cancel click release");

    // Test 3: Sub::AudioCredits title bar close button click
    console
        .options
        .open_sub(crate::gameui::options::Sub::AudioCredits);
    console.sync_dialog_windows();
    assert!(console.options.has_sub());
    let bounds = console.window_bounds(Window::SubDialog);
    let size = console
        .options
        .sub_frame_size(crate::gameui::options::Sub::AudioCredits, |_| 0.);
    let title_close_pt =
        vec2(bounds.x, bounds.y) + (source_ui::widgets::close_button_rect(size).center() * scale);

    pointer_frame(&mut console, title_close_pt, true, true, false);
    pointer_frame(&mut console, title_close_pt, false, false, true);
    assert!(
        !console.options.has_sub(),
        "credits closed on close button release"
    );

    // Test 4: Sub::AudioCredits bottom Close button click
    console
        .options
        .open_sub(crate::gameui::options::Sub::AudioCredits);
    console.sync_dialog_windows();
    assert!(console.options.has_sub());
    let bounds = console.window_bounds(Window::SubDialog);
    let size = console
        .options
        .sub_frame_size(crate::gameui::options::Sub::AudioCredits, |s| {
            s.chars().count() as f32 * 7.
        });
    let bottom_close_rect = Rect::new(size.x - 96., size.y - 34., 80., 24.);
    let bottom_close_pt = vec2(bounds.x, bounds.y) + (bottom_close_rect.center() * scale);

    pointer_frame(&mut console, bottom_close_pt, true, true, false);
    pointer_frame(&mut console, bottom_close_pt, false, false, true);
    assert!(
        !console.options.has_sub(),
        "credits closed on bottom close button release"
    );

    // Test 5: Sub::Gamma (Brightness) OK and Cancel buttons
    console.options.open_sub(crate::gameui::options::Sub::Gamma);
    console.sync_dialog_windows();
    assert!(console.options.has_sub());
    let bounds = console.window_bounds(Window::SubDialog);
    // Gamma OKButton is at (104, 356, 72, 24)
    let gamma_ok_pt = vec2(bounds.x, bounds.y) + (vec2(104. + 36., 356. + 12.) * scale);

    pointer_frame(&mut console, gamma_ok_pt, true, true, false);
    pointer_frame(&mut console, gamma_ok_pt, false, false, true);
    assert!(
        !console.options.has_sub(),
        "brightness closed on OK click release"
    );
}

#[test]
fn keyboard_use_defaults_confirmation_works_with_click_and_keys() {
    let mut console = menu_with_options(720.);
    let res = GameUiResources {
        key_rows: vec![source_ui::keys::KeyRow::Action {
            command: "+forward".into(),
            label: "Move forward".into(),
        }],
        default_bindings: std::collections::BTreeMap::from([("+forward".into(), "w".into())]),
        ..Default::default()
    };
    console.set_resources(res);
    console.open_options();
    console
        .options
        .select_tab(crate::gameui::options::Tab::Keyboard);

    // Modify a key binding
    console
        .options
        .draft
        .config
        .keys
        .insert("+forward".into(), "x".into());
    assert_eq!(
        console
            .options
            .draft
            .config
            .keys
            .get("+forward")
            .map(|s| s.as_str()),
        Some("x")
    );

    // Trigger KeyDefaults
    console
        .options
        .command(crate::gameui::settings::Command::KeyDefaults);
    console.sync_dialog_windows();
    assert!(console.options.has_confirm());
    assert!(console.shell.windows.is_active(Window::Confirmation));

    // Idle frame: confirmation dialog persists across frames
    console.input(&Input::default());
    assert!(
        console.options.has_confirm(),
        "confirmation dialog persists across idle frames"
    );

    // Cancel via Escape or dismissal
    console.options.dismiss_confirm();
    assert!(!console.options.has_confirm());
    assert!(console.options.closing_confirm.is_some());
    assert_eq!(
        console
            .options
            .draft
            .config
            .keys
            .get("+forward")
            .map(|s| s.as_str()),
        Some("x"),
        "binding unchanged after cancel"
    );

    // Re-open KeyDefaults
    console
        .options
        .command(crate::gameui::settings::Command::KeyDefaults);
    console.sync_dialog_windows();
    assert!(console.options.has_confirm());

    // Press Enter to confirm defaults
    console.input(&Input {
        keys: [Key::Enter].into_iter().collect(),
        held_keys: [Key::Enter].into_iter().collect(),
        ..Default::default()
    });
    console.input(&Input {
        keys: Default::default(),
        held_keys: Default::default(),
        ..Default::default()
    });
    assert!(!console.options.has_confirm(), "confirm closed after Enter");
    assert!(console.options.closing_confirm.is_some());
    assert_eq!(
        console
            .options
            .draft
            .config
            .keys
            .get("+forward")
            .map(|s| s.as_str()),
        Some("w"),
        "default bindings restored after Enter confirmation"
    );

    // Re-open KeyDefaults and test mouse click on OK
    console
        .options
        .draft
        .config
        .keys
        .insert("+forward".into(), "x".into());
    console
        .options
        .command(crate::gameui::settings::Command::KeyDefaults);
    console.sync_dialog_windows();
    assert!(console.options.has_confirm());

    let bounds = console.window_bounds(Window::Confirmation);
    let ok_pt = vec2(bounds.x + bounds.w * 0.25, bounds.y + bounds.h - 25.);
    pointer_frame(&mut console, ok_pt, true, true, false);
    pointer_frame(&mut console, ok_pt, false, false, true);
    assert!(
        !console.options.has_confirm(),
        "confirm closed after OK click"
    );
    assert_eq!(
        console
            .options
            .draft
            .config
            .keys
            .get("+forward")
            .map(|s| s.as_str()),
        Some("w"),
        "default bindings restored after OK click"
    );
}

#[test]
fn out_of_focus_menu_title_and_subtitle_are_translucent_without_black_outline() {
    let mut console = installed_console();
    console.set_mode(Mode::MainMenu);
    console.canvas.resize(1280., 720.);

    // When menu is focused (t=0.0):
    console.shell.windows.set_time(0.0);
    assert_eq!(console.shell.windows.base_focus_amount(0.0), 1.0);

    // Open options: base menu focus fades out to 0.0 at t=0.3
    console.open_options();
    console.shell.windows.set_time(0.0);
    let base_focus = console.shell.windows.base_focus_amount(0.3);
    assert!(base_focus < 1e-4);

    // Draw menu when fully out of focus (t=0.3)
    console.canvas.drain();
    console.draw(0.3);
    let quads = console.canvas.drain();

    // Outline quads (emitted by draw_cell_text_outlined) have color (0,0,0, 0.45 * base_focus)
    // and texture.is_some() (the glyph cache texture).
    // When base_focus is 0.0, outline is TextOutline::NONE, so NO text outline quads are emitted.
    // Verify no text glyph outline quads with alpha >= 0.2 are emitted under the menu.
    let black_outline_quads: Vec<_> = quads
        .iter()
        .filter(|q| {
            q.texture.is_some()
                && q.color.r == 0.0
                && q.color.g == 0.0
                && q.color.b == 0.0
                && q.color.a >= 0.2
        })
        .collect();
    assert!(
        black_outline_quads.is_empty(),
        "no opaque outline quads should be emitted when out of focus, found: {}",
        black_outline_quads.len()
    );
}

#[test]
fn console_invocation_is_blocked_when_blocking_subdialog_or_modal_is_open() {
    let mut console = menu_with_options(720.);
    console.set_resources((*crate::gameui::options::tests::resources()).clone());
    console.open_options();
    console.sync_dialog_windows();

    // 1. With just Options open (non-modal), toggle() successfully opens the console:
    assert!(!console.console_open());
    console.toggle();
    assert!(console.console_open(), "console opens over options dialog");
    console.toggle();
    assert!(!console.console_open(), "console closes on toggle");

    // 2. Open blocking subdialog (Video Advanced): toggle() is blocked!
    console
        .options
        .open_sub(crate::gameui::options::Sub::VideoAdvanced);
    console.sync_dialog_windows();
    assert!(console.options.has_sub());
    assert!(console.shell.windows.has_modal());

    console.toggle();
    assert!(
        !console.console_open(),
        "console invocation is blocked when subdialog is open"
    );

    // 3. Close subdialog: console toggle is unblocked
    console.options.cancel_sub();
    console.sync_dialog_windows();
    assert!(!console.options.has_sub());
    assert!(!console.shell.windows.has_modal());

    console.toggle();
    assert!(
        console.console_open(),
        "console opens after subdialog is closed"
    );
    console.toggle();
    assert!(!console.console_open());

    // 4. Open options confirmation dialog (Reset Defaults): toggle() is blocked!
    console
        .options
        .command(crate::gameui::settings::Command::KeyDefaults);
    console.sync_dialog_windows();
    assert!(console.options.has_confirm());
    assert!(console.shell.windows.has_modal());

    console.toggle();
    assert!(
        !console.console_open(),
        "console invocation is blocked when options confirm dialog is open"
    );

    // 5. Dismiss options confirmation dialog: toggle is unblocked
    console.options.dismiss_confirm();
    console.sync_dialog_windows();
    assert!(!console.options.has_confirm());
    assert!(!console.shell.windows.has_modal());

    console.toggle();
    assert!(
        console.console_open(),
        "console opens after options confirm is dismissed"
    );
    console.toggle();
    assert!(!console.console_open());

    // 6. When Quit confirmation box is open: toggle() is blocked!
    let loc = console.gameui.localization.clone();
    console.confirm_box = Some((crate::gameui::menu::quit_box(&loc), ConfirmKind::Quit));
    assert!(console.modal_dialog_open());

    console.toggle();
    assert!(
        !console.console_open(),
        "console invocation is blocked when quit confirmation is open"
    );
}

#[test]
fn console_entry_cursor_blinks_when_focused_and_hides_when_unfocused() {
    let mut console = Console::default();
    console.shell.windows.set_time(-1.0);
    console.shell.windows.open(Window::Console);
    console.shell.windows.set_time(0.0);
    console.sync_dialog_windows();
    assert_eq!(
        console.shell.windows.focus(),
        Layer::Window(Window::Console)
    );

    let layout = console.shell.panel.dialog_layout(&console.canvas);
    let entry_canvas = layout.canvas(layout.entry);
    let caret_h = (layout.entry.h - 8.) * layout.scale;

    let has_console_caret = |quads: &[source_ui::canvas::Quad]| {
        quads.iter().any(|q| {
            (q.destination.w - layout.scale).abs() < 1e-3
                && (q.destination.h - caret_h).abs() < 1e-3
                && q.destination.y >= entry_canvas.y
                && q.destination.y <= entry_canvas.y + entry_canvas.h
        })
    };

    // 1. Console is in focus: caret blinks at 2.0 Hz (on at 0.0s, off at 0.5s, on at 1.0s)
    console.draw(0.0);
    let quads_0 = console.canvas.drain();
    assert!(
        has_console_caret(&quads_0),
        "focused console entry draws blinking caret at t=0.0"
    );

    console.draw(0.5);
    let quads_half = console.canvas.drain();
    assert!(
        !has_console_caret(&quads_half),
        "focused console entry hides caret at t=0.5 (blink off)"
    );

    console.draw(1.0);
    let quads_1 = console.canvas.drain();
    assert!(
        has_console_caret(&quads_1),
        "focused console entry draws caret at t=1.0 (blink on)"
    );

    // 2. Options window is opened and raised over Console:
    console.open_options();
    console.sync_dialog_windows();
    assert_eq!(
        console.shell.windows.focus(),
        Layer::Window(Window::Options)
    );

    // When Console is out of focus, its entry caret must be strictly hidden at all times!
    console.draw(0.0);
    let quads_unfocused_0 = console.canvas.drain();
    assert!(
        !has_console_caret(&quads_unfocused_0),
        "unfocused console entry must not draw caret at t=0.0"
    );

    console.draw(1.0);
    let quads_unfocused_1 = console.canvas.drain();
    assert!(
        !has_console_caret(&quads_unfocused_1),
        "unfocused console entry must not draw caret at t=1.0"
    );

    // 3. When Quit confirmation box appears over console:
    shell::close_window(&mut console, Window::Options);
    console.sync_dialog_windows();
    assert_eq!(
        console.shell.windows.focus(),
        Layer::Window(Window::Console)
    );

    let loc = console.gameui.localization.clone();
    console.confirm_box = Some((crate::gameui::menu::quit_box(&loc), ConfirmKind::Quit));
    console.sync_dialog_windows();
    assert_eq!(
        console.shell.windows.focus(),
        Layer::Window(Window::Confirmation)
    );

    console.draw(0.0);
    let quads_confirm = console.canvas.drain();
    assert!(
        !has_console_caret(&quads_confirm),
        "console entry must not draw caret when confirmation dialog is in focus"
    );
}

#[test]
fn mouse_cvars_query_set_clamp_and_cvar_value() {
    let mut console = installed_console();
    console.set_mode(Mode::Console);

    // m_pitch (invert)
    console.submit("m_pitch -0.022");
    assert!(console.config.mouse.invert);
    assert_eq!(console.cvar_value("m_pitch").as_deref(), Some("-0.022"));
    console.submit("m_pitch 0.022");
    assert!(!console.config.mouse.invert);
    assert_eq!(console.cvar_value("m_pitch").as_deref(), Some("0.022"));

    // m_filter
    console.submit("m_filter 1");
    assert!(console.config.mouse.filter);
    assert_eq!(console.cvar_value("m_filter").as_deref(), Some("1"));
    console.submit("m_filter 0");
    assert!(!console.config.mouse.filter);
    assert_eq!(console.cvar_value("m_filter").as_deref(), Some("0"));

    // m_rawinput
    console.submit("m_rawinput 1");
    assert!(console.config.mouse.raw_input);
    assert_eq!(console.cvar_value("m_rawinput").as_deref(), Some("1"));
    console.submit("m_rawinput 0");
    assert!(!console.config.mouse.raw_input);
    assert_eq!(console.cvar_value("m_rawinput").as_deref(), Some("0"));

    // m_customaccel
    console.submit("m_customaccel 3");
    assert!(console.config.mouse.acceleration);
    assert_eq!(console.cvar_value("m_customaccel").as_deref(), Some("1"));
    console.submit("m_customaccel 0");
    assert!(!console.config.mouse.acceleration);
    assert_eq!(console.cvar_value("m_customaccel").as_deref(), Some("0"));

    // m_customaccel_exponent
    console.submit("m_customaccel_exponent 1.25");
    assert_eq!(console.config.mouse.accel_exponent, 1.25);
    assert_eq!(
        console.cvar_value("m_customaccel_exponent").as_deref(),
        Some("1.25")
    );
    console.submit("m_customaccel_exponent 5.0");
    assert_eq!(console.config.mouse.accel_exponent, 1.4);

    // sk_autoaim_mode and alias m_autoaim_mode
    console.submit("sk_autoaim_mode 1");
    assert_eq!(
        console.config.mouse.aim_assist,
        crate::config::AimAssist::Basic
    );
    assert_eq!(console.cvar_value("sk_autoaim_mode").as_deref(), Some("1"));
    assert_eq!(console.cvar_value("m_autoaim_mode").as_deref(), Some("1"));

    console.submit("m_autoaim_mode 2");
    assert_eq!(
        console.config.mouse.aim_assist,
        crate::config::AimAssist::Enhanced
    );
    assert_eq!(console.cvar_value("sk_autoaim_mode").as_deref(), Some("2"));

    console.submit("sk_autoaim_mode 0");
    assert_eq!(
        console.config.mouse.aim_assist,
        crate::config::AimAssist::Off
    );
    assert_eq!(console.cvar_value("sk_autoaim_mode").as_deref(), Some("0"));

    // sk_autoaim_scale1 and alias m_autoaim_scale
    console.submit("sk_autoaim_scale1 1.5");
    assert_eq!(console.config.mouse.aim_assist_scale, 1.5);
    assert_eq!(
        console.cvar_value("sk_autoaim_scale1").as_deref(),
        Some("1.50")
    );
    assert_eq!(
        console.cvar_value("m_autoaim_scale").as_deref(),
        Some("1.50")
    );

    console.submit("m_autoaim_scale 1.8");
    assert_eq!(console.config.mouse.aim_assist_scale, 1.8);

    console.submit("sk_autoaim_scale1 10.0");
    assert_eq!(console.config.mouse.aim_assist_scale, 2.0);

    console.submit("sk_autoaim_scale1 0.1");
    assert_eq!(console.config.mouse.aim_assist_scale, 0.5);
}

#[test]
fn audio_cvars_query_set_clamp_and_cvar_value() {
    let mut console = installed_console();
    console.set_mode(Mode::Console);

    // snd_musicvolume
    console.submit("snd_musicvolume 0.75");
    assert_eq!(console.config.audio.music_volume, 0.75);
    assert_eq!(
        console.cvar_value("snd_musicvolume").as_deref(),
        Some("0.75")
    );
    console.submit("snd_musicvolume 1.5");
    assert_eq!(console.config.audio.music_volume, 1.0);
    console.submit("snd_musicvolume -0.5");
    assert_eq!(console.config.audio.music_volume, 0.0);

    // snd_mute_losefocus
    console.submit("snd_mute_losefocus 0");
    assert!(!console.config.audio.mute_losefocus);
    assert_eq!(
        console.cvar_value("snd_mute_losefocus").as_deref(),
        Some("0")
    );
    console.submit("snd_mute_losefocus 1");
    assert!(console.config.audio.mute_losefocus);
    assert_eq!(
        console.cvar_value("snd_mute_losefocus").as_deref(),
        Some("1")
    );

    // closecaption & cc_subtitles
    console.submit("closecaption 0");
    assert_eq!(console.config.audio.captions, crate::config::Captions::None);
    assert_eq!(console.cvar_value("closecaption").as_deref(), Some("0"));
    assert_eq!(console.cvar_value("cc_subtitles").as_deref(), Some("0"));

    console.submit("closecaption 1");
    assert_eq!(
        console.config.audio.captions,
        crate::config::Captions::ClosedCaptions
    );
    assert_eq!(console.cvar_value("closecaption").as_deref(), Some("1"));
    assert_eq!(console.cvar_value("cc_subtitles").as_deref(), Some("0"));

    console.submit("cc_subtitles 1");
    assert_eq!(
        console.config.audio.captions,
        crate::config::Captions::Subtitles
    );
    assert_eq!(console.cvar_value("closecaption").as_deref(), Some("1"));
    assert_eq!(console.cvar_value("cc_subtitles").as_deref(), Some("1"));

    console.submit("cc_subtitles 0");
    assert_eq!(
        console.config.audio.captions,
        crate::config::Captions::ClosedCaptions
    );
    assert_eq!(console.cvar_value("cc_subtitles").as_deref(), Some("0"));

    // snd_surround_speakers
    console.submit("snd_surround_speakers 0");
    assert_eq!(
        console.config.audio.speakers,
        crate::config::Speakers::Headphones
    );
    assert_eq!(
        console.cvar_value("snd_surround_speakers").as_deref(),
        Some("0")
    );

    console.submit("snd_surround_speakers 5");
    assert_eq!(
        console.config.audio.speakers,
        crate::config::Speakers::FiveOne
    );
    assert_eq!(
        console.cvar_value("snd_surround_speakers").as_deref(),
        Some("5")
    );

    console.submit("snd_surround_speakers 2");
    assert_eq!(console.config.audio.speakers, crate::config::Speakers::Two);

    // snd_pitchquality & dsp_slow_cpu
    console.submit("snd_pitchquality 1");
    assert_eq!(
        console.config.audio.quality,
        crate::config::SoundQuality::High
    );
    assert_eq!(console.cvar_value("snd_pitchquality").as_deref(), Some("1"));

    console.submit("dsp_slow_cpu 1");
    assert_eq!(
        console.config.audio.quality,
        crate::config::SoundQuality::Low
    );
    assert_eq!(console.cvar_value("dsp_slow_cpu").as_deref(), Some("1"));

    console.submit("dsp_slow_cpu 0");
    assert_eq!(
        console.config.audio.quality,
        crate::config::SoundQuality::High
    );
    assert_eq!(console.cvar_value("dsp_slow_cpu").as_deref(), Some("0"));
}

#[test]
fn video_hdr_cvar_query_set_and_cvar_value() {
    let mut console = installed_console();
    console.set_mode(Mode::Console);

    assert_eq!(console.config.video.hdr, crate::config::HdrMode::Full);
    assert_eq!(console.cvar_value("mat_hdr_level").as_deref(), Some("2"));

    console.submit("mat_hdr_level 1");
    assert_eq!(console.config.video.hdr, crate::config::HdrMode::Bloom);
    assert_eq!(console.cvar_value("mat_hdr_level").as_deref(), Some("1"));

    console.submit("mat_hdr_level 2");
    assert_eq!(console.config.video.hdr, crate::config::HdrMode::Full);
    assert_eq!(console.cvar_value("mat_hdr_level").as_deref(), Some("2"));

    console.submit("mat_hdr_level 0");
    assert_eq!(console.config.video.hdr, crate::config::HdrMode::None);
    assert_eq!(console.cvar_value("mat_hdr_level").as_deref(), Some("0"));

    // Invalid level reports error and does not mutate config
    console.submit("mat_hdr_level 3");
    assert_eq!(console.config.video.hdr, crate::config::HdrMode::None);
    assert_eq!(console.cvar_value("mat_hdr_level").as_deref(), Some("0"));
}

#[test]
fn video_fov_cvars_query_set_clamp_and_cvar_value() {
    let mut console = installed_console();
    console.set_mode(Mode::Console);

    // fov & fov_desired
    assert_eq!(console.cvar_value("fov").as_deref(), Some("75"));
    assert_eq!(console.cvar_value("fov_desired").as_deref(), Some("75"));

    console.submit("fov_desired 90");
    assert_eq!(console.config.video.fov, 90.0);
    assert_eq!(console.cvar_value("fov").as_deref(), Some("90"));
    assert_eq!(console.cvar_value("fov_desired").as_deref(), Some("90"));

    console.submit("fov 105");
    assert_eq!(console.config.video.fov, 105.0);

    // vertical_fov
    assert_eq!(console.cvar_value("vertical_fov").as_deref(), Some("0"));
    console.submit("vertical_fov 60");
    assert_eq!(console.config.video.vertical_fov, 60.0);
    assert_eq!(console.cvar_value("vertical_fov").as_deref(), Some("60"));

    console.submit("vertical_fov 0");
    assert_eq!(console.config.video.vertical_fov, 0.0);

    console.submit("vertical_fov 150");
    assert_eq!(console.config.video.vertical_fov, 100.0); // clamped to 100

    // viewmodel_fov
    assert_eq!(console.cvar_value("viewmodel_fov").as_deref(), Some("54"));
    console.submit("viewmodel_fov 65");
    assert_eq!(console.config.video.viewmodel_fov, 65.0);
    assert_eq!(console.cvar_value("viewmodel_fov").as_deref(), Some("65"));

    console.submit("viewmodel_fov 20");
    assert_eq!(console.config.video.viewmodel_fov, 40.0); // clamped to 40
    console.submit("viewmodel_fov 120");
    assert_eq!(console.config.video.viewmodel_fov, 80.0); // clamped to 80
}

#[test]
fn video_advanced_presentation_cvars_query_set_and_cvar_value() {
    let mut console = installed_console();
    console.set_mode(Mode::Console);

    // mat_vsync
    assert_eq!(console.cvar_value("mat_vsync").as_deref(), Some("1"));
    console.submit("mat_vsync 0");
    assert!(!console.config.video.advanced.vsync);
    assert_eq!(console.cvar_value("mat_vsync").as_deref(), Some("0"));
    console.submit("mat_vsync 1");
    assert!(console.config.video.advanced.vsync);
    assert_eq!(console.cvar_value("mat_vsync").as_deref(), Some("1"));

    // mat_colorcorrection
    assert_eq!(
        console.cvar_value("mat_colorcorrection").as_deref(),
        Some("1")
    );
    console.submit("mat_colorcorrection 0");
    assert!(!console.config.video.advanced.color_correction);
    assert_eq!(
        console.cvar_value("mat_colorcorrection").as_deref(),
        Some("0")
    );

    // mat_motion_blur_enabled
    assert_eq!(
        console.cvar_value("mat_motion_blur_enabled").as_deref(),
        Some("1")
    );
    console.submit("mat_motion_blur_enabled 0");
    assert!(!console.config.video.advanced.motion_blur);
    assert_eq!(
        console.cvar_value("mat_motion_blur_enabled").as_deref(),
        Some("0")
    );

    // mat_queue_mode
    assert_eq!(console.cvar_value("mat_queue_mode").as_deref(), Some("-1"));
    console.submit("mat_queue_mode 0");
    assert!(!console.config.video.advanced.multicore);
    assert_eq!(console.cvar_value("mat_queue_mode").as_deref(), Some("0"));
    console.submit("mat_queue_mode -1");
    assert!(console.config.video.advanced.multicore);
    assert_eq!(console.cvar_value("mat_queue_mode").as_deref(), Some("-1"));
    console.submit("mat_queue_mode 2");
    assert!(console.config.video.advanced.multicore);
}

#[test]
fn video_advanced_detail_cvars_query_set_and_cvar_value() {
    let mut console = installed_console();
    console.set_mode(Mode::Console);

    // r_rootlod: 0=High, 1=Medium, 2=Low
    assert_eq!(console.cvar_value("r_rootlod").as_deref(), Some("0"));
    console.submit("r_rootlod 1");
    assert_eq!(
        console.config.video.advanced.model_detail,
        crate::config::ModelDetail::Medium
    );
    assert_eq!(console.cvar_value("r_rootlod").as_deref(), Some("1"));
    console.submit("r_rootlod 2");
    assert_eq!(
        console.config.video.advanced.model_detail,
        crate::config::ModelDetail::Low
    );
    assert_eq!(console.cvar_value("r_rootlod").as_deref(), Some("2"));
    console.submit("r_rootlod 0");
    assert_eq!(
        console.config.video.advanced.model_detail,
        crate::config::ModelDetail::High
    );

    // mat_picmip: -1=VeryHigh, 0=High, 1=Medium, 2=Low
    assert_eq!(console.cvar_value("mat_picmip").as_deref(), Some("0"));
    console.submit("mat_picmip -1");
    assert_eq!(
        console.config.video.advanced.texture_detail,
        crate::config::TextureDetail::VeryHigh
    );
    assert_eq!(console.cvar_value("mat_picmip").as_deref(), Some("-1"));
    console.submit("mat_picmip 1");
    assert_eq!(
        console.config.video.advanced.texture_detail,
        crate::config::TextureDetail::Medium
    );
    assert_eq!(console.cvar_value("mat_picmip").as_deref(), Some("1"));
    console.submit("mat_picmip 2");
    assert_eq!(
        console.config.video.advanced.texture_detail,
        crate::config::TextureDetail::Low
    );
    assert_eq!(console.cvar_value("mat_picmip").as_deref(), Some("2"));

    // mat_reducefillrate: 0=High, 1=Low
    assert_eq!(
        console.cvar_value("mat_reducefillrate").as_deref(),
        Some("0")
    );
    console.submit("mat_reducefillrate 1");
    assert_eq!(
        console.config.video.advanced.shader_detail,
        crate::config::ShaderDetail::Low
    );
    assert_eq!(
        console.cvar_value("mat_reducefillrate").as_deref(),
        Some("1")
    );
    console.submit("mat_reducefillrate 0");
    assert_eq!(
        console.config.video.advanced.shader_detail,
        crate::config::ShaderDetail::High
    );

    // r_waterforceexpensive & r_waterforcereflectentities
    // WaterDetail: Simple (0, 0), World (1, 0), All (1, 1)
    assert_eq!(
        console.cvar_value("r_waterforceexpensive").as_deref(),
        Some("1")
    );
    assert_eq!(
        console.cvar_value("r_waterforcereflectentities").as_deref(),
        Some("0")
    );
    console.submit("r_waterforcereflectentities 1");
    assert_eq!(
        console.config.video.advanced.water_detail,
        crate::config::WaterDetail::All
    );
    assert_eq!(
        console.cvar_value("r_waterforcereflectentities").as_deref(),
        Some("1")
    );
    console.submit("r_waterforceexpensive 0");
    assert_eq!(
        console.config.video.advanced.water_detail,
        crate::config::WaterDetail::Simple
    );
    assert_eq!(
        console.cvar_value("r_waterforceexpensive").as_deref(),
        Some("0")
    );
    assert_eq!(
        console.cvar_value("r_waterforcereflectentities").as_deref(),
        Some("0")
    );

    // r_shadowrendertotexture & r_flashlightdepthtexture
    // ShadowDetail: Low (0, 0), Medium (1, 0), High (1, 1)
    assert_eq!(
        console.cvar_value("r_shadowrendertotexture").as_deref(),
        Some("1")
    );
    assert_eq!(
        console.cvar_value("r_flashlightdepthtexture").as_deref(),
        Some("1")
    );
    console.submit("r_flashlightdepthtexture 0");
    assert_eq!(
        console.config.video.advanced.shadow_detail,
        crate::config::ShadowDetail::Medium
    );
    assert_eq!(
        console.cvar_value("r_shadowrendertotexture").as_deref(),
        Some("1")
    );
    assert_eq!(
        console.cvar_value("r_flashlightdepthtexture").as_deref(),
        Some("0")
    );
    console.submit("r_shadowrendertotexture 0");
    assert_eq!(
        console.config.video.advanced.shadow_detail,
        crate::config::ShadowDetail::Low
    );
    assert_eq!(
        console.cvar_value("r_shadowrendertotexture").as_deref(),
        Some("0")
    );
    assert_eq!(
        console.cvar_value("r_flashlightdepthtexture").as_deref(),
        Some("0")
    );
}

#[test]
fn video_advanced_filtering_and_antialiasing_cvars_query_set_and_cvar_value() {
    let mut console = installed_console();
    console.set_mode(Mode::Console);

    // mat_forceaniso: 1=Bilinear/Trilinear, 2=Anisotropic2X, 4=Anisotropic4X, 8=Anisotropic8X, 16=Anisotropic16X
    // mat_trilinear: 0=Bilinear, 1=Trilinear/Anisotropic
    assert_eq!(console.cvar_value("mat_forceaniso").as_deref(), Some("1"));
    assert_eq!(console.cvar_value("mat_trilinear").as_deref(), Some("1"));
    assert_eq!(
        console.config.video.advanced.filtering,
        crate::config::Filtering::Trilinear
    );

    console.submit("mat_forceaniso 4");
    assert_eq!(
        console.config.video.advanced.filtering,
        crate::config::Filtering::Anisotropic4
    );
    assert_eq!(console.cvar_value("mat_forceaniso").as_deref(), Some("4"));
    assert_eq!(console.cvar_value("mat_trilinear").as_deref(), Some("1"));

    console.submit("mat_forceaniso 16");
    assert_eq!(
        console.config.video.advanced.filtering,
        crate::config::Filtering::Anisotropic16
    );
    assert_eq!(console.cvar_value("mat_forceaniso").as_deref(), Some("16"));

    console.submit("mat_trilinear 0");
    assert_eq!(
        console.config.video.advanced.filtering,
        crate::config::Filtering::Bilinear
    );
    assert_eq!(console.cvar_value("mat_forceaniso").as_deref(), Some("1"));
    assert_eq!(console.cvar_value("mat_trilinear").as_deref(), Some("0"));

    console.submit("mat_trilinear 1");
    assert_eq!(
        console.config.video.advanced.filtering,
        crate::config::Filtering::Trilinear
    );
    assert_eq!(console.cvar_value("mat_trilinear").as_deref(), Some("1"));

    // mat_antialias: 0/1=None, 2=X2, 4=X4, 8=X8
    assert_eq!(console.cvar_value("mat_antialias").as_deref(), Some("0"));
    assert_eq!(console.cvar_value("mat_aaquality").as_deref(), Some("0"));
    assert_eq!(
        console.config.video.advanced.antialiasing,
        crate::config::Antialiasing::None
    );

    console.submit("mat_antialias 4");
    assert_eq!(
        console.config.video.advanced.antialiasing,
        crate::config::Antialiasing::X4
    );
    assert_eq!(console.cvar_value("mat_antialias").as_deref(), Some("4"));

    console.submit("mat_antialias 8");
    assert_eq!(
        console.config.video.advanced.antialiasing,
        crate::config::Antialiasing::X8
    );
    assert_eq!(console.cvar_value("mat_antialias").as_deref(), Some("8"));

    console.submit("mat_antialias 0");
    assert_eq!(
        console.config.video.advanced.antialiasing,
        crate::config::Antialiasing::None
    );
    assert_eq!(console.cvar_value("mat_antialias").as_deref(), Some("0"));
}

#[test]
fn video_advanced_effects_and_d3d9ex_cvars_query_set_and_cvar_value() {
    let mut console = installed_console();
    console.set_mode(Mode::Console);

    // r_classic_fire & r_classic_blood
    assert_eq!(console.cvar_value("r_classic_fire").as_deref(), Some("0"));
    assert_eq!(console.cvar_value("r_classic_blood").as_deref(), Some("0"));
    assert!(!console.config.video.advanced.classic_effects);

    console.submit("r_classic_fire 1");
    assert!(console.config.video.advanced.classic_effects);
    assert_eq!(console.cvar_value("r_classic_fire").as_deref(), Some("1"));
    assert_eq!(console.cvar_value("r_classic_blood").as_deref(), Some("1"));

    console.submit("r_classic_blood 0");
    assert!(!console.config.video.advanced.classic_effects);
    assert_eq!(console.cvar_value("r_classic_fire").as_deref(), Some("0"));
    assert_eq!(console.cvar_value("r_classic_blood").as_deref(), Some("0"));

    // mat_disable_d3d9ex (0=enabled, 1=disabled)
    assert_eq!(
        console.cvar_value("mat_disable_d3d9ex").as_deref(),
        Some("0")
    );
    assert!(console.config.video.advanced.d3d9ex);

    console.submit("mat_disable_d3d9ex 1");
    assert!(!console.config.video.advanced.d3d9ex);
    assert_eq!(
        console.cvar_value("mat_disable_d3d9ex").as_deref(),
        Some("1")
    );

    console.submit("mat_disable_d3d9ex 0");
    assert!(console.config.video.advanced.d3d9ex);
    assert_eq!(
        console.cvar_value("mat_disable_d3d9ex").as_deref(),
        Some("0")
    );
}

#[test]
fn gameplay_skill_cvar_and_migrate_command() {
    let mut console = installed_console();
    console.set_mode(Mode::Console);

    // Default skill is 2 (Normal)
    assert_eq!(console.cvar_value("skill").as_deref(), Some("2"));
    assert_eq!(console.config.gameplay.skill, 2);

    // Query skill
    console.submit("skill");
    assert_eq!(
        console.shell.panel.output.back().unwrap().text,
        "\"skill\" = \"2\""
    );

    // Set skill 1 (Easy)
    console.submit("skill 1");
    assert_eq!(console.config.gameplay.skill, 1);
    assert_eq!(console.cvar_value("skill").as_deref(), Some("1"));

    // Set skill 3 (Hard)
    console.submit("skill 3");
    assert_eq!(console.config.gameplay.skill, 3);
    assert_eq!(console.cvar_value("skill").as_deref(), Some("3"));

    // Invalid skill values rejected
    console.submit("skill 0");
    assert_eq!(console.config.gameplay.skill, 3);
    console.submit("skill 4");
    assert_eq!(console.config.gameplay.skill, 3);

    // Placeholder migrate_episodic_data command
    console.submit("migrate_episodic_data");
    assert_eq!(
        console.shell.panel.output.back().unwrap().text,
        "migrate_episodic_data: save data migration is not implemented in this build"
    );
}

#[test]
fn layout_follows_the_console_panel_insets_at_1024x768() {
    let layout = DialogLayout::new(vec2(1024., 768.), proportional::SCALING);
    // Proportional 16 and 64 at 768 lines are 25 and 102 pixels.
    assert_eq!(layout.origin, vec2(487., 102.));
    let scale = 768. / 720.;
    assert_eq!(layout.scale, scale);
    assert_eq!(layout.size, (vec2(512., 564.) / scale).round());
    let (w, h) = (layout.size.x, layout.size.y);
    // Client area: x 8, y 30, w - 16 wide, down to 6 above the frame edge.
    assert_eq!(layout.history, Rect::new(16., 42., w - 32., h - 36. - 52.));
    let bottom = h - 6. - 8. - 24.;
    assert_eq!(layout.submit, Rect::new(w - 8. - 79., bottom, 64., 24.));
    assert_eq!(
        layout.entry,
        Rect::new(16., bottom, w - 16. - 79. - 4. - 16., 24.)
    );
    // History, entry and button stay inside the frame, entry left of the button.
    assert!(layout.entry.x + layout.entry.w < layout.submit.x);
    assert!(layout.history.y + layout.history.h < layout.entry.y);
    assert!(layout.submit.x + layout.submit.w < w);
    // Layout and canvas coordinates round-trip.
    let submit = layout.canvas(layout.submit);
    let centre = vec2(submit.x + submit.w * 0.5, submit.y + submit.h * 0.5);
    assert!(layout.submit.contains(layout.local(centre)));
}

#[test]
fn landscape_frames_are_the_retail_ones_and_narrow_screens_get_a_bounded_frame() {
    for canvas in [vec2(1280., 1024.), vec2(1920., 1080.), vec2(5120., 1440.)] {
        let (origin, size) = DialogLayout::default_frame(canvas, proportional::SCALING);
        let scale = proportional::dialog_scale(canvas);
        let p = |v: f32| (v * canvas.y / PROPORTIONAL_BASE).floor();
        assert_eq!(
            origin,
            vec2((canvas.x * 0.5).floor() - p(16.), p(64.)),
            "{canvas}"
        );
        assert_eq!(
            size,
            (vec2((canvas.x * 0.5).floor(), canvas.y - 2. * p(64.)) / scale).round(),
            "{canvas}"
        );
    }
    for canvas in [vec2(1080., 1080.), vec2(1080., 1920.), vec2(720., 1280.)] {
        let (origin, size) = DialogLayout::default_frame(canvas, proportional::SCALING);
        let scale = proportional::dialog_scale(canvas);
        let (right, bottom) = (origin.x + size.x * scale, origin.y + size.y * scale);
        let design = proportional::design_height(canvas);
        assert!(
            origin.x > 0. && right < canvas.x,
            "{canvas}: spans the width"
        );
        assert!(right > canvas.x * 0.9, "{canvas}: {right}");
        assert!(
            bottom <= design,
            "{canvas}: ends in the design area, not at the bottom"
        );
        assert!(
            bottom < canvas.y * 0.8 || canvas.y == canvas.x,
            "{canvas}: {bottom}"
        );
    }
}

#[test]
fn small_canvases_and_resizes_keep_the_minimum_size() {
    let layout = DialogLayout::new(vec2(120., 100.), proportional::SCALING);
    assert_eq!(layout.size, MIN_SIZE);
    let smallest = DialogLayout::at(1., Vec2::ZERO, vec2(10., 10.));
    assert_eq!(smallest.size, MIN_SIZE);
    assert!(smallest.entry.w >= SUBMIT_WIDE, "{:?}", smallest.entry);
    assert!(
        smallest.history.h >= 3. * ENTRY_TALL,
        "{:?}",
        smallest.history
    );
    assert!(smallest.entry.x + smallest.entry.w < smallest.submit.x);
    assert!(smallest.submit.x + smallest.submit.w < smallest.size.x);
}

mod chapters;
