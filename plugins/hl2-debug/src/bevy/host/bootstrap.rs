//! Private loopback request for the existing local RCON activation handler.
use super::{State, session};
use hl2_bevy::ui_host::Console;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering::Relaxed};

const IDLE: u8 = 0;
const REQUESTED: u8 = 1;
const STOPPED: u8 = 2;

pub(super) struct Bootstrap {
    state: Arc<AtomicU8>,
    socket: UdpSocket,
    address: SocketAddr,
    startup_served: bool,
}

impl Bootstrap {
    fn start(token: String) -> io::Result<Self> {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))?;
        let address = socket.local_addr()?;
        let receiving_socket = socket.try_clone()?;
        let state = Arc::new(AtomicU8::new(IDLE));
        let worker_state = Arc::clone(&state);
        std::thread::Builder::new()
            .name("agent-rcon-bootstrap".into())
            .spawn(move || receive(receiving_socket, &worker_state, token.as_bytes()))?;
        Ok(Self {
            state,
            socket,
            address,
            startup_served: false,
        })
    }

    fn take_request(&mut self, ui: &Console) -> bool {
        self.state.load(Relaxed) == REQUESTED
            && !ui.source.startup_pending()
            // Give the startup RCON system its Update before the first runtime request.
            && std::mem::replace(&mut self.startup_served, true)
            && self
                .state
                .compare_exchange(REQUESTED, IDLE, Relaxed, Relaxed)
                .is_ok()
    }
}

impl Drop for Bootstrap {
    fn drop(&mut self) {
        self.state.store(STOPPED, Relaxed);
        // Wake the blocking receive so the worker can release its copy of the socket.
        let _ = self.socket.send_to(&[0], self.address);
    }
}

fn receive(socket: UdpSocket, state: &AtomicU8, token: &[u8]) {
    // A full UDP buffer lets a malformed long datagram be discarded on every platform.
    let mut data = [0; 65_536];
    while let Ok((size, from)) = socket.recv_from(&mut data) {
        if state.load(Relaxed) == STOPPED {
            return;
        }
        let valid = from.ip().is_loopback()
            && size == token.len()
            && data[..size]
                .iter()
                .zip(token)
                .fold(0, |difference, (a, b)| difference | (a ^ b))
                == 0;
        if valid {
            let _ = state.compare_exchange(IDLE, REQUESTED, Relaxed, Relaxed);
        }
    }
}

impl State {
    pub(super) fn start_agent_bootstrap(&mut self) -> io::Result<()> {
        let pid = std::process::id();
        let session = session::Session::create(&std::env::temp_dir(), pid)?;
        let token = session::password()?;
        let bootstrap = Bootstrap::start(token.clone())?;
        let record =
            serde_json::json!({"pid": pid, "port": bootstrap.address.port(), "token": token});
        session.write("bootstrap.json", &serde_json::to_vec(&record)?)?;
        self.agent_session = Some(session);
        self.agent_bootstrap = Some(bootstrap);
        Ok(())
    }

    pub(super) fn process_agent_bootstrap(&mut self, ui: &mut Console) {
        if self
            .agent_bootstrap
            .as_mut()
            .is_some_and(|bootstrap| bootstrap.take_request(ui))
        {
            self.agent_rcon(ui);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Bootstrap, Console, IDLE, Ipv4Addr, Relaxed, UdpSocket};
    use std::time::{Duration, Instant};

    #[test]
    fn only_the_exact_token_requests_activation() {
        let token = "ab".repeat(24);
        let mut bootstrap = Bootstrap::start(token.clone()).unwrap();
        let client = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let ui = Console::new(Default::default());
        for invalid in ["cd".repeat(24), format!("{token} extra")] {
            client
                .send_to(invalid.as_bytes(), bootstrap.address)
                .unwrap();
        }
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(bootstrap.state.load(Relaxed), IDLE);

        client.send_to(token.as_bytes(), bootstrap.address).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !bootstrap.take_request(&ui) {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!bootstrap.take_request(&ui));
    }
}
