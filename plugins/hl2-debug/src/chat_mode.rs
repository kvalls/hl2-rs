//! Route local console submissions to player chat before command tokenization.
use hl2_ui::console::Console;
use hl2_ui::console::remote::PLAYER_SPEAKER;
use hl2_ui::extensions::LocalSubmit;
use source_ui::console::panel::Level;

const OWNER: &str = "hl2-debug";

pub(super) fn configure(console: &mut Console, enabled: Option<bool>) -> Result<(), String> {
    if let Some(enabled) = enabled {
        let handler = enabled.then_some(route as LocalSubmit);
        console.extensions.set_local_submit(OWNER, handler)?;
    }
    let enabled = u8::from(console.extensions.has_local_submit(OWNER));
    console.log(format!("chat_mode = {enabled}"));
    Ok(())
}

fn route(console: &mut Console, line: &str) -> bool {
    let (first, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let rest = rest.trim();
    if first.eq_ignore_ascii_case("chat_mode") && matches!(rest, "" | "0" | "1") {
        return false;
    }
    console.shell.panel.print_with_log_text(
        &console.canvas,
        Level::ChatLocal,
        format!("{PLAYER_SPEAKER}: {line}"),
        || format!("{PLAYER_SPEAKER} (chat mode): {line}"),
    );
    true
}

#[cfg(test)]
mod tests {
    use crate::ui_tests::console;

    #[test]
    fn chat_mode_routes_only_editor_submissions_and_can_be_disabled_locally() {
        let mut console = console();
        console.submit_local("echo commands work");
        assert_eq!(
            console.shell.panel.output.back().unwrap().text,
            "commands work"
        );
        console.submit_local("chat_mode 1");
        console.submit_local("sv_cheats 1; echo \"unfinished");
        assert!(!console.cheats);
        assert_eq!(
            console.shell.panel.output.back().unwrap().text,
            "Player: sv_cheats 1; echo \"unfinished"
        );
        console.submit_local("chat_mode 0; sv_cheats 1");
        assert!(!console.cheats);
        console.shell.panel.keep_printed(true);
        console.submit_local(r#"say 'hello'; "world""#);
        assert_eq!(
            console.shell.panel.output.back().unwrap().text,
            r#"Player: say 'hello'; "world""#
        );
        assert_eq!(
            console.shell.panel.take_printed()[0].text,
            r#"Player (chat mode): say 'hello'; "world""#
        );
        console.execute("sv_cheats 1");
        assert!(console.cheats);
        console.submit_local("chat_mode 0");
        console.shell.panel.take_printed();
        console.submit_local("say hello");
        assert_eq!(
            console.shell.panel.output.back().unwrap().text,
            "Player: hello"
        );
        assert_eq!(console.shell.panel.take_printed()[0].text, "Player: hello");
        console.submit_local("echo back to commands");
        assert_eq!(
            console.shell.panel.output.back().unwrap().text,
            "back to commands"
        );
    }

    #[test]
    fn chat_mode_lines_stay_in_this_runs_history_but_are_not_saved() {
        use source_ui::input::{Input, Key};
        let mut console = console();
        console.toggle();
        let enter = |console: &mut hl2_ui::console::Console, line: &str| {
            console.input(&Input {
                text: line.into(),
                keys: [Key::Enter].into_iter().collect(),
                ..Default::default()
            });
        };
        enter(&mut console, "chat_mode 1");
        enter(&mut console, "echo \"hello\" there");
        enter(&mut console, "chat_mode 0");
        assert_eq!(
            console.take_changed_history().unwrap(),
            ["chat_mode 1", "chat_mode 0"],
            "a chat-mode message is not saved"
        );
        enter(&mut console, "echo \"hello\" there");
        assert_eq!(
            console.take_changed_history().unwrap(),
            ["chat_mode 1", "chat_mode 0", "echo \"hello\" there"],
            "the same text run as a command is saved"
        );
    }
}
