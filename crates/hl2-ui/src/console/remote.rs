//! Following the console from outside the game: the console log file (`con_logfile`,
//! `-condebug`) and the remote console's cvars (`rcon_password`, `sv_rcon_*`, `hostport`,
//! `ip`). The host opens the file and the socket; nothing here touches either.

use std::path::{Component, Path, PathBuf};

/// `-condebug`'s log file.
pub const CONDEBUG_FILE: &str = "console.log";

/// The name `say` shows for the player (no `name` cvar yet).
pub const PLAYER_SPEAKER: &str = "Player";
/// The name `say` shows for remote console commands.
pub const RCON_SPEAKER: &str = "Console";

/// Valid chat commands print their own speaker-labelled output. Keep the input or remote
/// log entry for malformed lines and batches that also run other commands.
pub fn chat_only(commands: &Result<Vec<Vec<String>>, String>) -> bool {
    commands.as_ref().is_ok_and(|commands| {
        !commands.is_empty() && commands.iter().all(|tokens| {
            matches!(tokens.as_slice(), [name, _, ..] if name.eq_ignore_ascii_case("say"))
        })
    })
}

/// What the host reads to serve outside readers.
#[derive(Debug, Clone, PartialEq)]
pub struct Remote {
    /// `con_logfile`: the file console output is appended to, relative to the settings folder;
    /// empty writes none.
    pub con_logfile: String,
    /// The command running came from the remote console.
    pub from_rcon: bool,
    /// Lines a remote command printed that the local console shows too (`say`).
    pub broadcast: Vec<String>,
    /// Remote console commands run only with a non-empty password; never saved.
    pub rcon_password: String,
    /// `sv_rcon_log`: print each remote command.
    pub sv_rcon_log: bool,
    pub sv_rcon_maxfailures: u32,
    pub sv_rcon_minfailures: u32,
    /// Seconds.
    pub sv_rcon_minfailuretime: u32,
    /// Minutes, 0 for the rest of the run.
    pub sv_rcon_banpenalty: u32,
    /// Bytes; a larger packet bans its address.
    pub sv_rcon_maxpacketsize: u32,
    /// The address and port the remote console listens on (read when it starts).
    pub ip: String,
    pub hostport: u32,
}

impl Default for Remote {
    /// Retail's defaults as remembered from Source servers (**not verified** against the
    /// installed engine); `ip` `localhost` accepts only local connections.
    fn default() -> Self {
        Self {
            con_logfile: String::new(),
            from_rcon: false,
            broadcast: Vec::new(),
            rcon_password: String::new(),
            sv_rcon_log: true,
            sv_rcon_maxfailures: 10,
            sv_rcon_minfailures: 5,
            sv_rcon_minfailuretime: 30,
            sv_rcon_banpenalty: 0,
            sv_rcon_maxpacketsize: 1024,
            ip: "localhost".into(),
            hostport: 27015,
        }
    }
}

/// The file `con_logfile` names, relative to the settings folder: a name without an extension
/// gets `.log`; absolute paths and `..` are refused (the folder is the only place written).
pub fn log_file(name: &str) -> Result<Option<PathBuf>, &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Ok(None);
    }
    let path = Path::new(name);
    if !path
        .components()
        .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err("con_logfile: use a file name relative to the settings folder");
    }
    Ok(Some(if path.extension().is_some() {
        path.to_owned()
    } else {
        path.with_extension("log")
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_files_stay_in_the_settings_folder() {
        assert_eq!(log_file(""), Ok(None));
        assert_eq!(log_file("console.log"), Ok(Some("console.log".into())));
        assert_eq!(log_file("runs/today"), Ok(Some("runs/today.log".into())));
        assert_eq!(log_file("notes.txt"), Ok(Some("notes.txt".into())));
        for refused in ["/tmp/x.log", "../x.log", "a/../../x.log", "./x.log"] {
            assert!(log_file(refused).is_err(), "{refused}");
        }
    }
}
