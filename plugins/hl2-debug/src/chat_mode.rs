//! Route local console submissions to player chat before command tokenization.
use hl2_ui::console::Console;
use hl2_ui::extensions::LocalSubmit;

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
    console.say_local(line);
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
        console.submit_local("say hello; world");
        assert_eq!(
            console.shell.panel.output.back().unwrap().text,
            "Player: say hello; world"
        );
        console.execute("sv_cheats 1");
        assert!(console.cheats);
        console.submit_local("chat_mode 0");
        console.submit_local("say hello");
        assert_eq!(
            console.shell.panel.output.back().unwrap().text,
            "Player: hello"
        );
        console.submit_local("echo back to commands");
        assert_eq!(
            console.shell.panel.output.back().unwrap().text,
            "back to commands"
        );
    }
}
