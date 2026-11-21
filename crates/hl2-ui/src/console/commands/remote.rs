//! The console for readers outside the game: the log file, chat lines, the remote console's
//! cvars and screenshots.

use super::{Outcome, Spec};
use crate::console::{remote, Console, Effect};
use source_ui::console::panel::Level;

pub(super) fn con_logfile(console: &mut Console, spec: &Spec, args: &[String]) -> Outcome {
    match args {
        [] => console.log(format!(
            "\"{}\" = \"{}\"",
            spec.name, console.remote.con_logfile
        )),
        [name] => {
            remote::log_file(name)?;
            console.remote.con_logfile = name.trim().to_owned();
        }
        _ => return Err(spec.usage()),
    }
    Ok(None)
}

pub(super) fn con_logfile_value(console: &Console) -> Option<String> {
    Some(console.remote.con_logfile.clone())
}

/// A remote console cvar stored in [`remote::Remote`]: its handler (prints without an argument,
/// parses one) and its value.
macro_rules! remote_cvar {
    ($name:ident, $value:ident) => {
        pub(super) fn $name(console: &mut Console, spec: &Spec, args: &[String]) -> Outcome {
            match args {
                [] => console.log(format!(
                    "\"{}\" = \"{}\"",
                    spec.name,
                    $value(console).unwrap_or_default()
                )),
                [value] => {
                    console.remote.$name = parse(value).ok_or_else(|| spec.usage())?;
                }
                _ => return Err(spec.usage()),
            }
            Ok(None)
        }

        pub(super) fn $value(console: &Console) -> Option<String> {
            Some(show(&console.remote.$name))
        }
    };
}

remote_cvar!(rcon_password, rcon_password_value);
remote_cvar!(sv_rcon_log, sv_rcon_log_value);
remote_cvar!(sv_rcon_maxfailures, sv_rcon_maxfailures_value);
remote_cvar!(sv_rcon_minfailures, sv_rcon_minfailures_value);
remote_cvar!(sv_rcon_minfailuretime, sv_rcon_minfailuretime_value);
remote_cvar!(sv_rcon_banpenalty, sv_rcon_banpenalty_value);
remote_cvar!(sv_rcon_maxpacketsize, sv_rcon_maxpacketsize_value);
remote_cvar!(ip, ip_value);
remote_cvar!(hostport, hostport_value);

/// A cvar value as typed: booleans as 0 or 1.
trait CvarValue: Sized {
    fn parse(text: &str) -> Option<Self>;
    fn show(&self) -> String;
}

impl CvarValue for String {
    fn parse(text: &str) -> Option<Self> {
        Some(text.to_owned())
    }
    fn show(&self) -> String {
        self.clone()
    }
}

impl CvarValue for u32 {
    fn parse(text: &str) -> Option<Self> {
        text.parse().ok()
    }
    fn show(&self) -> String {
        self.to_string()
    }
}

impl CvarValue for bool {
    fn parse(text: &str) -> Option<Self> {
        source_ui::console::command::boolean(text)
    }
    fn show(&self) -> String {
        u8::from(*self).to_string()
    }
}

fn parse<T: CvarValue>(text: &str) -> Option<T> {
    T::parse(text)
}

fn show<T: CvarValue>(value: &T) -> String {
    value.show()
}

/// `screenshot`: the host saves the next game frame.
pub(super) fn screenshot(_console: &mut Console, spec: &Spec, args: &[String]) -> Outcome {
    if !args.is_empty() {
        return Err(spec.usage());
    }
    Ok(Some(Effect::Screenshot))
}

/// `say`: a chat line under the speaker's name, `Console` for the remote console. Its line
/// reaches the local console even while a remote command's output is redirected.
pub(super) fn say(console: &mut Console, spec: &Spec, args: &[String]) -> Outcome {
    if args.is_empty() {
        return Err(spec.usage());
    }
    let (speaker, level) = if console.remote.from_rcon {
        (remote::RCON_SPEAKER, Level::ChatRemote)
    } else {
        (remote::PLAYER_SPEAKER, Level::ChatLocal)
    };
    let line = format!("{speaker}: {}", args.join(" "));
    if console.remote.from_rcon {
        console.clear_snapshot();
        console.remote.broadcast.push(line.clone());
    }
    console.print(level, line);
    Ok(None)
}

#[cfg(test)]
mod tests {
    use crate::console::Console;

    fn last(console: &Console) -> Option<&str> {
        console
            .shell
            .panel
            .output
            .back()
            .map(|line| line.text.as_str())
    }

    #[test]
    fn only_chat_skips_the_command_echo() {
        use crate::console::remote::chat_only;
        use source_ui::console::command::tokenize;
        for line in [r#"SAY "a;b""#, "say hello; say goodbye"] {
            assert!(chat_only(&tokenize(line)));
        }
        for line in ["echo say", "say hello; echo ok", "say", "say \"open", ";"] {
            assert!(!chat_only(&tokenize(line)));
        }
    }

    #[test]
    fn say_names_the_player_or_the_remote_console() {
        let mut console = Console::default();
        let before = console.shell.panel.output.len();
        console.submit("say hello there");
        assert_eq!(console.shell.panel.output.len(), before + 1);
        assert_eq!(last(&console), Some("Player: hello there"));
        assert_eq!(
            console.shell.panel.output.back().map(|line| line.level),
            Some(super::Level::ChatLocal)
        );
        assert!(console.remote.broadcast.is_empty());
        console.remote.from_rcon = true;
        console.submit("say hi");
        assert_eq!(last(&console), Some("Console: hi"));
        assert_eq!(
            console.shell.panel.output.back().map(|line| line.level),
            Some(super::Level::ChatRemote)
        );
        assert_eq!(console.remote.broadcast, ["Console: hi"]);
    }
}
