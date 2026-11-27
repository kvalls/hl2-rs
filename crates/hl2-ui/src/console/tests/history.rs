//! The console command history: browsing it and which lines are saved.
use super::*;

#[test]
fn console_hide_preserves_pause_and_escape_dismisses_history_first() {
    let mut console = Console::default();
    console.escape();
    assert!(console.paused());
    console.toggle();
    assert_eq!(console.mode(), Mode::Console);
    console.execute("hideconsole");
    assert_eq!(console.mode(), Mode::Pause);
    console.toggle();
    let send = |c: &mut Console, t: &str, k: &[Key]| {
        c.input(&Input {
            text: t.into(),
            keys: k.iter().copied().collect(),
            ..Default::default()
        });
    };
    send(&mut console, "echo first", &[Key::Enter]);
    send(&mut console, "echo second", &[Key::Enter]);
    keys(&mut console, &[Key::Up]);
    assert!(console.input_line() == "echo second" && console.completion_bounds().is_none());
    keys(&mut console, &[Key::Up]);
    assert_eq!(console.input_line(), "echo first");
    keys(&mut console, &[Key::Down]);
    assert_eq!(console.input_line(), "echo second");
    keys(&mut console, &[Key::Down]);
    assert_eq!(console.input_line(), "");
    send(&mut console, "ec", &[]);
    assert!(console.completion_bounds().is_some());
    console.escape();
    assert!(console.mode() == Mode::Console && console.completion_bounds().is_none());
    console.escape();
    assert!(!console.paused() && console.mode() == Mode::Gameplay);
    let history = console.take_changed_history().unwrap();
    assert!(history == ["echo first", "echo second"] && console.take_changed_history().is_none());
}

#[test]
fn chat_lines_stay_in_this_runs_history_but_are_not_saved() {
    let mut console = Console::default();
    console.toggle();
    for line in [
        "echo kept",
        "say \"hello; there\"",
        "SAY bye",
        "say hi; echo mixed",
        "echo kept",
    ] {
        console.input(&Input {
            text: line.into(),
            keys: [Key::Enter].into_iter().collect(),
            ..Default::default()
        });
    }
    for _ in 0..3 {
        keys(&mut console, &[Key::Up]);
    }
    assert_eq!(
        console.input_line(),
        "SAY bye",
        "chat stays reachable this run"
    );
    assert_eq!(
        console.take_changed_history().unwrap(),
        ["say hi; echo mixed", "echo kept"],
        "only lines that run a command are saved"
    );
}
