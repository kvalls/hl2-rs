//! Key binding data: `scripts/kb_act.lst` rows, `cfg/config_default.cfg` binds and the mapping
//! between Source key names (`w`, `SPACE`, `CTRL`, `MOUSE1`) and the persisted binding names an
//! app saves its bindings under (`KeyW`, `Space`, `ControlLeft`).
//!
//! Source binds keys to commands (`bind "w" "+forward"`); the options page shows one key per
//! command. Keys without a host equivalent (mouse buttons, wheel) keep their Source name so they
//! still display, but hosts cannot press them through the keyboard path.

use std::collections::BTreeMap;

/// Persisted binding name <-> Source key name. Letters and digits are generated separately.
pub const NAMED_KEYS: &[(&str, &str)] = &[
    ("Space", "SPACE"),
    ("ControlLeft", "CTRL"),
    ("ControlRight", "RCTRL"),
    ("ShiftLeft", "SHIFT"),
    ("ShiftRight", "RSHIFT"),
    ("AltLeft", "ALT"),
    ("AltRight", "RALT"),
    ("SuperLeft", "LWIN"),
    ("SuperRight", "RWIN"),
    ("Enter", "ENTER"),
    ("Escape", "ESCAPE"),
    ("Tab", "TAB"),
    ("Backspace", "BACKSPACE"),
    ("CapsLock", "CAPSLOCK"),
    ("ArrowUp", "UPARROW"),
    ("ArrowDown", "DOWNARROW"),
    ("ArrowLeft", "LEFTARROW"),
    ("ArrowRight", "RIGHTARROW"),
    ("Insert", "INS"),
    ("Delete", "DEL"),
    ("Home", "HOME"),
    ("End", "END"),
    ("PageUp", "PGUP"),
    ("PageDown", "PGDN"),
    ("Pause", "PAUSE"),
    ("NumLock", "NUMLOCK"),
    ("Backquote", "`"),
    ("Minus", "-"),
    ("Equal", "="),
    ("BracketLeft", "["),
    ("BracketRight", "]"),
    ("Backslash", "\\"),
    ("Semicolon", "SEMICOLON"),
    ("Quote", "'"),
    ("Comma", ","),
    ("Period", "."),
    ("Slash", "/"),
    ("Numpad0", "KP_INS"),
    ("Numpad1", "KP_END"),
    ("Numpad2", "KP_DOWNARROW"),
    ("Numpad3", "KP_PGDN"),
    ("Numpad4", "KP_LEFTARROW"),
    ("Numpad5", "KP_5"),
    ("Numpad6", "KP_RIGHTARROW"),
    ("Numpad7", "KP_HOME"),
    ("Numpad8", "KP_UPARROW"),
    ("Numpad9", "KP_PGUP"),
    ("NumpadEnter", "KP_ENTER"),
    ("NumpadAdd", "KP_PLUS"),
    ("NumpadSubtract", "KP_MINUS"),
    ("NumpadMultiply", "KP_MULTIPLY"),
    ("NumpadDivide", "KP_SLASH"),
    ("NumpadDecimal", "KP_DEL"),
];

/// Converts a Source key name to the persisted binding name. Unknown names (mouse buttons, wheel,
/// joystick) are kept in upper case so they still round-trip and display.
pub fn binding_from_source(source: &str) -> String {
    let source = source.trim();
    let mut chars = source.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_ascii_alphabetic() {
            return format!("Key{}", c.to_ascii_uppercase());
        }
        if c.is_ascii_digit() {
            return format!("Digit{c}");
        }
    }
    if let Some(n) = function_key(source) {
        return format!("F{n}");
    }
    NAMED_KEYS
        .iter()
        .find(|(_, s)| s.eq_ignore_ascii_case(source))
        .map_or_else(|| source.to_ascii_uppercase(), |(b, _)| (*b).to_owned())
}

/// The Source spelling shown in the key column (`W`, `SPACE`, `CTRL`, `MOUSE1`).
pub fn display_name(binding: &str) -> String {
    if let Some(letter) = binding.strip_prefix("Key").filter(|l| l.len() == 1) {
        return letter.to_owned();
    }
    if let Some(digit) = binding.strip_prefix("Digit").filter(|d| d.len() == 1) {
        return digit.to_owned();
    }
    if function_key(binding).is_some() {
        return binding.to_ascii_uppercase();
    }
    NAMED_KEYS
        .iter()
        .find(|(b, _)| *b == binding)
        .map_or_else(|| binding.to_ascii_uppercase(), |(_, s)| (*s).to_owned())
}

fn function_key(name: &str) -> Option<u8> {
    let n: u8 = name
        .strip_prefix('F')
        .or_else(|| name.strip_prefix('f'))?
        .parse()
        .ok()?;
    (1..=12).contains(&n).then_some(n)
}

/// One row of the key binding list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyRow {
    /// A section title (`"blank" "#Valve_Movement_Title"`).
    Section(String),
    /// A bindable command and its description token.
    Action { command: String, label: String },
}

/// Parses `scripts/kb_act.lst`: `"<command>" "<#token>"` pairs, where `blank` rows are section
/// titles and `=====` rule rows are dropped.
pub fn parse_actions(text: &str) -> Vec<KeyRow> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let fields = quoted_fields(line);
        let [command, label, ..] = fields.as_slice() else {
            continue;
        };
        if command.eq_ignore_ascii_case("blank") {
            if !label.starts_with('=') && !label.is_empty() {
                rows.push(KeyRow::Section(label.clone()));
            }
        } else if !command.is_empty() {
            rows.push(KeyRow::Action {
                command: command.clone(),
                label: label.clone(),
            });
        }
    }
    rows
}

/// Applies `bind`, `unbind` and (on macOS) `bind_mac`/`unbind_mac` lines of a config file in order
/// and returns command -> binding name, choosing each command's first bound key in file order.
pub fn parse_default_binds(text: &str, mac: bool) -> BTreeMap<String, String> {
    // key (upper-case Source name) -> (first appearance, command)
    let mut keys: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or_default();
        let mut words = line.split_whitespace();
        let Some(verb) = words.next().map(str::to_ascii_lowercase) else {
            continue;
        };
        let fields = quoted_or_words(line);
        let (bind, unbind) = match verb.as_str() {
            "bind" => (true, false),
            "unbind" => (false, true),
            "bind_mac" if mac => (true, false),
            "unbind_mac" if mac => (false, true),
            "unbindall" => {
                keys.clear();
                continue;
            }
            _ => continue,
        };
        let Some(key) = fields.get(1).map(|k| k.to_ascii_uppercase()) else {
            continue;
        };
        let existing = keys.iter().position(|(k, _)| *k == key);
        if unbind {
            if let Some(index) = existing {
                keys.remove(index);
            }
        } else if bind {
            let Some(command) = fields.get(2) else {
                continue;
            };
            match existing {
                Some(index) => keys[index].1 = command.clone(),
                None => keys.push((key, command.clone())),
            }
        }
    }
    let mut commands = BTreeMap::new();
    for (key, command) in keys {
        commands
            .entry(command)
            .or_insert_with(|| binding_from_source(&key));
    }
    commands
}

/// Persisted value of an explicitly unbound action. A missing entry means "use the default", so
/// older config files pick up actions added to the defaults later.
pub const UNBOUND: &str = "";

/// Binds `key` to `action`; any other action bound to `key` becomes [`UNBOUND`] (a key runs one
/// command).
pub fn assign(keys: &mut BTreeMap<String, String>, action: &str, key: &str) {
    for (other, bound) in keys.iter_mut() {
        if other != action && bound == key {
            bound.clear();
        }
    }
    keys.insert(action.to_owned(), key.to_owned());
}

fn quoted_fields(line: &str) -> Vec<String> {
    line.split('"')
        .skip(1)
        .step_by(2)
        .map(|s| s.trim().to_owned())
        .collect()
}

/// `bind "w" "+forward"` and `bind w +forward` both yield `[bind, w, +forward]`.
fn quoted_or_words(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut rest = line.trim();
    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix('"') {
            let end = stripped.find('"').unwrap_or(stripped.len());
            fields.push(stripped[..end].to_owned());
            rest = stripped.get(end + 1..).unwrap_or_default().trim_start();
        } else {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            fields.push(rest[..end].to_owned());
            rest = rest[end..].trim_start();
        }
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_key_names_round_trip_to_binding_names() {
        for (source, binding, shown) in [
            ("w", "KeyW", "W"),
            ("1", "Digit1", "1"),
            ("SPACE", "Space", "SPACE"),
            ("ctrl", "ControlLeft", "CTRL"),
            ("LWIN", "SuperLeft", "LWIN"),
            ("F12", "F12", "F12"),
            ("`", "Backquote", "`"),
            ("KP_END", "Numpad1", "KP_END"),
            ("MOUSE1", "MOUSE1", "MOUSE1"),
            ("mwheelup", "MWHEELUP", "MWHEELUP"),
        ] {
            assert_eq!(binding_from_source(source), binding, "{source}");
            assert_eq!(display_name(binding), shown, "{binding}");
        }
        assert_eq!(display_name("F13"), "F13", "unknown names stay visible");
    }

    #[test]
    fn kb_act_rows_keep_sections_and_drop_rules() {
        let rows = parse_actions(
            "\"blank\" \"====\"\n\"blank\" \"#Valve_Movement_Title\"\n\"blank\" \"====\"\n\"+forward\"\t\"#Valve_Move_Forward\"\n\n\"impulse 100\" \"#Valve_Flashlight\"\n",
        );
        assert_eq!(
            rows,
            [
                KeyRow::Section("#Valve_Movement_Title".into()),
                KeyRow::Action {
                    command: "+forward".into(),
                    label: "#Valve_Move_Forward".into()
                },
                KeyRow::Action {
                    command: "impulse 100".into(),
                    label: "#Valve_Flashlight".into()
                },
            ]
        );
    }

    const DEFAULTS: &str = "unbindall\nbind \"w\" \"+forward\"\nbind \"UPARROW\" \"+forward\"\nbind \"CTRL\" \"+duck\"\nbind \"ALT\" \"+walk\"\nbind \"f\" \"impulse 100\" // flashlight\nbind_mac \"ALT\" \"+duck\"\nbind_mac \"LWIN\" \"+walk\"\nunbind_mac \"CTRL\"\n";

    #[test]
    fn default_binds_follow_file_order_and_platform_overrides() {
        let windows = parse_default_binds(DEFAULTS, false);
        assert_eq!(windows["+forward"], "KeyW", "first key in file order");
        assert_eq!(windows["+duck"], "ControlLeft");
        assert_eq!(windows["+walk"], "AltLeft");
        assert_eq!(windows["impulse 100"], "KeyF");
        let mac = parse_default_binds(DEFAULTS, true);
        assert_eq!(
            mac["+duck"], "AltLeft",
            "bind_mac rebinds ALT, CTRL is unbound"
        );
        assert_eq!(mac["+walk"], "SuperLeft");
    }

    #[test]
    fn assigning_a_key_moves_it_away_from_other_actions() {
        let mut keys = BTreeMap::from([
            ("+forward".to_owned(), "KeyW".to_owned()),
            ("+jump".to_owned(), "Space".to_owned()),
        ]);
        assign(&mut keys, "+jump", "KeyW");
        assert_eq!(keys.get("+jump").map(String::as_str), Some("KeyW"));
        assert_eq!(
            keys.get("+forward").map(String::as_str),
            Some(UNBOUND),
            "the previous owner stays present but unbound, so defaults are not reapplied"
        );
    }
}
