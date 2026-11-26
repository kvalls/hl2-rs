//! Retail Source (`vgui::TextEntry`) keyboard editing for the console input line.
use super::editor::LineEditor;
use crate::input::{Input, Key};

/// Retail Source shortcut modifier (`Ctrl` on all platforms, or `Command` on macOS, without `Alt`).
pub fn retail_shortcut_mod(input: &Input, macos: bool) -> bool {
    (input.ctrl || (macos && input.command)) && !input.alt
}

pub(super) fn edit_input(editor: &mut LineEditor, input: &Input) -> Option<String> {
    retail_edit_with_platform(editor, input, cfg!(target_os = "macos"))
}

/// Applies retail `TextEntry` editing shortcuts and returns any cut text for the clipboard.
pub(super) fn retail_edit_with_platform(
    editor: &mut LineEditor,
    input: &Input,
    macos: bool,
) -> Option<String> {
    if !input.ctrl && !input.command {
        let typed: String = input
            .text
            .chars()
            .filter(|c| *c != '`' && *c != '~')
            .collect();
        editor.insert(&typed);
    }
    if let Some(paste) = &input.paste {
        editor.paste(paste);
    }
    if input.select_all {
        editor.select_all();
    }
    if input.undo {
        editor.undo();
    }
    let mut cut = None;
    if input.cut || (input.shift && input.pressed(Key::Delete)) {
        cut = editor.cut();
    }
    if input.pressed(Key::Backspace) && !input.ctrl && !input.command && !input.alt {
        editor.backspace();
    }
    if input.pressed(Key::Delete) && !input.shift && !input.ctrl && !input.command && !input.alt {
        editor.delete();
    }
    let word_mod = retail_shortcut_mod(input, macos);
    if !input.alt {
        for key in [Key::Left, Key::Right] {
            if input.pressed(key) {
                editor.navigate(key, word_mod, input.shift);
            }
        }
    }
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retail_shortcuts_support_undo_cut_and_word_navigation() {
        let mut editor = LineEditor::default();
        editor.insert("map d1_trainstation_01");

        // On macOS retail, Cmd+Left jumps a word; on non-macOS, Command does not.
        retail_edit_with_platform(
            &mut editor,
            &Input {
                keys: [Key::Left].into(),
                command: true,
                ..Default::default()
            },
            true,
        );
        assert_eq!(editor.cursor(), 4);

        let mut win_editor = LineEditor::default();
        win_editor.insert("map d1_trainstation_01");
        retail_edit_with_platform(
            &mut win_editor,
            &Input {
                keys: [Key::Left].into(),
                command: true,
                ..Default::default()
            },
            false,
        );
        assert_eq!(win_editor.cursor(), 21);

        // Select word right + Cut (or Shift+Delete) + Undo restores text.
        retail_edit_with_platform(
            &mut editor,
            &Input {
                keys: [Key::Right].into(),
                ctrl: true,
                shift: true,
                ..Default::default()
            },
            false,
        );
        assert_eq!(editor.selected_text(), Some("d1_trainstation_01"));
        let cut = retail_edit_with_platform(
            &mut editor,
            &Input {
                cut: true,
                ..Default::default()
            },
            true,
        );
        assert_eq!(cut.as_deref(), Some("d1_trainstation_01"));
        assert_eq!(editor.text(), "map ");
        retail_edit_with_platform(
            &mut editor,
            &Input {
                undo: true,
                ..Default::default()
            },
            true,
        );
        assert_eq!(editor.text(), "map d1_trainstation_01");

        // Alt+Left falls through in retail TextEntry (no movement).
        let before = editor.cursor();
        retail_edit_with_platform(
            &mut editor,
            &Input {
                keys: [Key::Left].into(),
                alt: true,
                ..Default::default()
            },
            true,
        );
        assert_eq!(editor.cursor(), before);
    }
}
