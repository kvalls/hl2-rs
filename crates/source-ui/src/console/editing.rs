//! Keyboard input for the shared console line editor.
use super::editor::LineEditor;
use crate::input::{Input, Key};
pub(super) fn edit_input(editor: &mut LineEditor, input: &Input) {
    let ctrl_key = |name: &str| input.ctrl && input.binding_keys.iter().any(|k| k == name);

    if !input.ctrl {
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
    if ctrl_key("KeyR") {
        editor.search_back();
    }
    if ctrl_key("KeyU") {
        editor.kill_to_start();
    }
    if ctrl_key("KeyK") {
        editor.kill_to_end();
    }
    if ctrl_key("KeyW") {
        editor.delete_word_back();
    }
    if input.select_all {
        editor.select_all();
    }
    if ctrl_key("KeyE") {
        editor.end();
    }
    if input.pressed(Key::Backspace) {
        if input.ctrl {
            editor.delete_word_back();
        } else {
            editor.backspace();
        }
    }
    if input.pressed(Key::Delete) {
        if input.ctrl {
            editor.delete_word_forward();
        } else {
            editor.delete();
        }
    }
    for key in [Key::Left, Key::Right] {
        if input.pressed(key) {
            editor.navigate(key, input.ctrl, input.shift);
        }
    }
}
