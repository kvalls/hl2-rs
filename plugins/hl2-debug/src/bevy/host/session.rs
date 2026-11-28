//! Local opt-in and process-scoped discovery for hl2-session.
use super::State;
use hl2_bevy::ui_host::Console;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

pub(super) struct Session(PathBuf);

impl Drop for Session {
    fn drop(&mut self) {
        for name in [
            "pid",
            "rcon_password",
            "port",
            "console_log_path",
            "bootstrap.json",
            ".pending",
        ] {
            let _ = fs::remove_file(self.0.join(name));
        }
        let _ = fs::remove_dir(&self.0);
    }
}

impl Session {
    pub(super) fn create(temp: &Path, pid: u32) -> io::Result<Self> {
        let parent = temp.join("hl2rs-agent-session");
        match private_directory(&parent) {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(&parent)?;
                #[cfg(unix)]
                use std::os::unix::fs::PermissionsExt;
                #[cfg(unix)]
                if metadata.permissions().mode() & 0o077 != 0 {
                    return Err(io::Error::other("session directory must be private"));
                }
                if !metadata.is_dir() {
                    return Err(io::Error::other("invalid session directory"));
                }
            }
            result => result?,
        }
        let path = parent.join(format!("live-{pid}"));
        private_directory(&path)?;
        Ok(Self(path))
    }

    pub(super) fn write(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let pending = self.0.join(".pending");
        let mut options = OpenOptions::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.write(true).create_new(true).open(&pending)?;
        let result = file.write_all(bytes);
        drop(file);
        let result = result.and_then(|()| fs::rename(&pending, self.0.join(name)));
        if result.is_err() {
            let _ = fs::remove_file(pending);
        }
        result
    }
}

fn private_directory(path: &Path) -> io::Result<()> {
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.recursive(false).create(path)
}

pub(super) fn password() -> io::Result<String> {
    let mut bytes = [0_u8; 24];
    getrandom::fill(&mut bytes)
        .map_err(|_| io::Error::other("could not generate session password"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

impl State {
    pub(super) fn agent_rcon(&mut self, ui: &mut Console) {
        if ui.source.remote.from_rcon {
            let text = ui
                .source
                .resources()
                .localization
                .text("#HL2RS_AgentRconLocal");
            ui.source.log(text);
            return;
        }
        let (token, detail) = match self.publish_agent_session(ui) {
            Ok(port) => ("#HL2RS_AgentRconReady", format!("127.0.0.1:{port}")),
            Err(error) => ("#HL2RS_AgentRconFailed", error.to_string()),
        };
        let text = ui.source.resources().localization.text(token);
        ui.source.log(format!("{text} {detail}"));
    }

    fn publish_agent_session(&mut self, ui: &mut Console) -> io::Result<u16> {
        let bound = ui.remote_console_address();
        if bound.is_some_and(|address| address.ip() != Ipv4Addr::LOCALHOST) {
            return Err(io::Error::other(
                "the existing listener must be on 127.0.0.1",
            ));
        }
        let pid = std::process::id();
        let fallback = format!("agent-console-{pid}.log");
        let (log_path, enable_log) = match Console::console_log_path(&ui.source.remote.con_logfile)?
        {
            Some(path) => (path, false),
            None => (
                Console::console_log_path(&fallback)?.expect("nonempty fallback name"),
                true,
            ),
        };
        let log_path = log_path
            .to_str()
            .ok_or_else(|| io::Error::other("console log path is not UTF-8"))?;
        let secret = if bound.is_some() && !ui.source.remote.rcon_password.is_empty() {
            ui.source.remote.rcon_password.clone()
        } else {
            password()?
        };

        let new_session = if self.agent_session.is_none() {
            Some(Session::create(&std::env::temp_dir(), pid)?)
        } else {
            None
        };
        let session = self
            .agent_session
            .as_ref()
            .or(new_session.as_ref())
            .expect("agent session directory");

        let port = ui
            .start_remote_console(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?
            .port();
        // Keep authentication disabled if publishing the endpoint fails after binding.
        if bound.is_none() {
            ui.source.remote.rcon_password.clear();
        }
        session.write("console_log_path", log_path.as_bytes())?;
        session.write("port", port.to_string().as_bytes())?;
        session.write("rcon_password", secret.as_bytes())?;
        session.write("pid", pid.to_string().as_bytes())?;
        ui.source.remote.rcon_password = secret;
        if enable_log {
            ui.source.remote.con_logfile = fallback;
        }
        if new_session.is_some() {
            self.agent_session = new_session;
        }
        Ok(port)
    }
}

#[cfg(test)]
mod tests {
    use super::{Console, Session, State, fs, password, private_directory};

    #[test]
    fn publication_is_private_and_its_owner_cleans_it_up() {
        let temp = std::env::temp_dir().join(format!("hl2rs-session-test-{}", password().unwrap()));
        private_directory(&temp).unwrap();
        let session = Session::create(&temp, 123).unwrap();
        session.write("rcon_password", b"test").unwrap();
        assert!(Session::create(&temp, 123).is_err());
        session.write("rcon_password", b"replacement").unwrap();
        assert_eq!(
            fs::read(session.0.join("rcon_password")).unwrap(),
            b"replacement"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [&session.0, &session.0.join("rcon_password")] {
                assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o077, 0);
            }
        }
        let path = session.0.clone();
        drop(session);
        assert!(!path.exists());
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn an_authenticated_remote_command_cannot_publish_an_agent_session() {
        let mut ui = Console::new(Default::default());
        let mut state = State::default();
        ui.source.remote.from_rcon = true;
        state.agent_rcon(&mut ui);
        assert!(state.agent_session.is_none());
        assert_eq!(ui.remote_console_address(), None);
        assert!(ui.source.remote.rcon_password.is_empty());
    }
}
