//! Non-retail console editing shortcuts contributed by `hl2-debug` (shell-style line/word
//! editing, history search, clear output, and macOS Option/Command word/line movement & deletion).
use source_ui::console::panel::ConsolePanel;
use source_ui::input::{Input, Key};

/// Reverse history search prompt token registered into localization by `hl2-debug`.
pub const CONSOLE_REVERSE_SEARCH: &str = "#HL2RS_Console_ReverseSearch";

/// Applies non-retail console editing shortcuts and returns `true` if handled instead of retail.
pub fn handle(panel: &mut ConsolePanel, input: &Input) -> bool {
    handle_with_platform(panel, input, cfg!(target_os = "macos"))
}

pub(crate) fn handle_with_platform(panel: &mut ConsolePanel, input: &Input, macos: bool) -> bool {
    let ctrl_only = input.ctrl && !input.alt && !input.command;
    let ctrl_key = |name: &str| ctrl_only && input.binding_keys.iter().any(|k| k == name);
    let mac_word = macos && input.alt && !input.ctrl && !input.command;
    let mac_line = macos && input.command && !input.alt && !input.ctrl;

    let mut consumed = false;
    if ctrl_key("KeyR") {
        panel.editor.search_back();
        consumed = true;
    }
    if ctrl_key("KeyU") {
        panel.editor.kill_to_start();
        consumed = true;
    }
    if ctrl_key("KeyK") {
        panel.editor.kill_to_end();
        consumed = true;
    }
    if ctrl_key("KeyW") {
        panel.editor.delete_word_back();
        consumed = true;
    }
    if ctrl_key("KeyE") {
        panel.editor.end();
        consumed = true;
    }
    if ctrl_key("KeyL") {
        panel.clear_output();
        consumed = true;
    }
    if input.pressed(Key::Backspace) {
        if mac_line {
            panel.editor.kill_to_start();
            consumed = true;
        } else if ctrl_only || mac_word {
            panel.editor.delete_word_back();
            consumed = true;
        }
    }
    if input.pressed(Key::Delete) {
        if mac_line {
            panel.editor.kill_to_end();
            consumed = true;
        } else if ctrl_only || mac_word {
            panel.editor.delete_word_forward();
            consumed = true;
        }
    }
    if mac_line {
        if input.pressed(Key::Left) {
            panel.editor.navigate(Key::Home, false, input.shift);
            consumed = true;
        }
        if input.pressed(Key::Right) {
            panel.editor.navigate(Key::End, false, input.shift);
            consumed = true;
        }
    } else if mac_word {
        for key in [Key::Left, Key::Right] {
            if input.pressed(key) {
                panel.editor.navigate(key, true, input.shift);
                consumed = true;
            }
        }
    }
    consumed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_tests::console;
    use source_ui::canvas::Canvas;
    use source_ui::console::complete::{Completer, Completion};
    use source_ui::console::dialog::Scaling;
    use source_ui::widgets::Painter;

    struct Nothing;
    impl Completer for Nothing {
        fn complete(&self, _: &str) -> Completion {
            Completion::default()
        }
    }

    fn panel() -> (Canvas, ConsolePanel) {
        (
            Canvas::default(),
            ConsolePanel::new(Scaling {
                dialog_scale: |c| Painter::scale_for(c.y),
                design_height: |c| c.y,
            }),
        )
    }

    #[test]
    fn macos_option_and_command_shortcuts_plus_dead_keys_work() {
        let (canvas, mut p) = panel();
        p.input(
            &canvas,
            &Input {
                text: "echo alpha beta".into(),
                ..Default::default()
            },
            &Nothing,
        );
        // Option+Left jumps a word with shift selection.
        let opt_left = Input {
            keys: [Key::Left].into(),
            alt: true,
            shift: true,
            ..Default::default()
        };
        assert!(handle_with_platform(&mut p, &opt_left, true));
        assert_eq!(p.editor.selected_text(), Some("beta"));
        assert!(!handle_with_platform(&mut p, &opt_left, false));

        // Option+Right collapses/jumps right, then Command+Left jumps to line start.
        assert!(handle_with_platform(
            &mut p,
            &Input {
                keys: [Key::Right].into(),
                alt: true,
                ..Default::default()
            },
            true
        ));
        assert!(handle_with_platform(
            &mut p,
            &Input {
                keys: [Key::Left].into(),
                command: true,
                ..Default::default()
            },
            true
        ));
        assert_eq!(p.editor.cursor(), 0);

        // Command+Right moves to end, Command+Backspace kills to start.
        assert!(handle_with_platform(
            &mut p,
            &Input {
                keys: [Key::Right].into(),
                command: true,
                ..Default::default()
            },
            true
        ));
        assert_eq!(p.editor.cursor(), 15);
        assert!(handle_with_platform(
            &mut p,
            &Input {
                keys: [Key::Backspace].into(),
                command: true,
                ..Default::default()
            },
            true
        ));
        assert_eq!(p.editor.text(), "");

        // Option dead-key/special text passes through unconsumed so retail inserts it.
        assert!(!handle_with_platform(
            &mut p,
            &Input {
                text: "é".into(),
                alt: true,
                ..Default::default()
            },
            true
        ));
    }

    #[test]
    fn debug_console_enables_reverse_search_and_shell_shortcuts() {
        let mut c = console();
        c.toggle();
        c.set_history(vec!["fov 90".into(), "volume 0.5".into()]);
        c.input(&Input {
            text: "x".into(),
            ctrl: true,
            binding_keys: vec!["KeyR".into()],
            ..Default::default()
        });
        c.input(&Input {
            text: "fov".into(),
            ..Default::default()
        });
        assert_eq!(c.shell.panel.editor.search(), Some(("fov", Some("fov 90"))));
        assert_eq!(
            c.resources().localization.text(CONSOLE_REVERSE_SEARCH),
            "reverse-i-search"
        );
        c.escape();
        c.input(&Input {
            paste: Some("echo one two".into()),
            ..Default::default()
        });
        c.input(&Input {
            keys: [Key::Backspace].into(),
            ctrl: true,
            ..Default::default()
        });
        assert_eq!(c.input_line(), "echo one ");
        c.input(&Input {
            ctrl: true,
            binding_keys: vec!["KeyU".into()],
            ..Default::default()
        });
        assert_eq!(c.input_line(), "");
        c.log("line");
        c.input(&Input {
            ctrl: true,
            binding_keys: vec!["KeyL".into()],
            ..Default::default()
        });
        assert!(c.shell.panel.output.is_empty());
    }
}
