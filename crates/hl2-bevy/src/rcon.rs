//! The remote console (`-usercon`): Valve's RCON protocol on `ip`:`hostport`. Sockets run on
//! their own threads ([`listener`]); each frame the game thread takes what arrived without
//! waiting, runs authenticated commands through the console like typed ones and sends their
//! output back instead of showing it ([`server`]).

mod listener;
mod network;
mod packet;
mod server;

use crate::ui_host::Console;
use bevy::prelude::{
    App, IntoScheduleConfigs, Plugin, Real, Res, ResMut, Resource, Time, Update, info, warn,
};
use listener::{Message, Outgoing};
use network::NetworkPolicy;
use server::{Connection, Limits, Reply, Server};
use source_ui::console::panel::Level;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, SyncSender};

/// The running remote console.
pub(crate) struct RemoteConsole {
    bound: SocketAddr,
    messages: Mutex<Receiver<Message>>,
    writers: HashMap<Connection, SyncSender<Outgoing>>,
    server: Server,
    network: Option<NetworkPolicy>,
}

#[derive(Resource, Default)]
struct StartupListener(Option<StartupPolicy>);

struct StartupPolicy(Option<NetworkPolicy>);

struct Host;

impl Plugin for Host {
    fn build(&self, app: &mut App) {
        app.init_resource::<StartupListener>()
            .add_systems(Update, serve.run_if(active));
    }
}

/// Installs the host for listeners explicitly opened by a runtime contributor.
pub fn install(app: &mut App) {
    if !app.is_plugin_added::<Host>() {
        app.add_plugins(Host);
    }
}

pub(crate) fn install_startup(app: &mut App, options: &crate::Options) -> anyhow::Result<()> {
    let network = NetworkPolicy::load(options)?;
    if options.usercon {
        install(app);
        app.insert_resource(StartupListener(Some(StartupPolicy(network))));
    }
    Ok(())
}

fn active(startup: Res<StartupListener>, ui: Option<Res<Console>>) -> bool {
    startup.0.is_some() || ui.is_some_and(|ui| ui.remote_console.is_some())
}

impl Console {
    /// Actual bound endpoint, including the selected port when bound to port zero.
    pub fn remote_console_address(&self) -> Option<SocketAddr> {
        self.remote_console.as_ref().map(|remote| remote.bound)
    }

    /// Opens an RCON listener on a loopback address. Starting twice leaves the first
    /// listener in place.
    pub fn start_remote_console(&mut self, address: SocketAddr) -> std::io::Result<SocketAddr> {
        if !address.ip().is_loopback() {
            return Err(network::network_bind_denied());
        }
        self.bind_remote_console(address, None)
    }

    fn bind_remote_console(
        &mut self,
        address: SocketAddr,
        network: Option<NetworkPolicy>,
    ) -> std::io::Result<SocketAddr> {
        if let Some(bound) = self.remote_console_address() {
            return Ok(bound);
        }
        let max_packet = self.source.remote.sv_rcon_maxpacketsize as usize;
        let (messages, bound) = listener::start(
            address,
            max_packet,
            network.as_ref().map(|policy| policy.peer),
        )?;
        self.remote_console = Some(RemoteConsole {
            bound,
            messages: Mutex::new(messages),
            writers: HashMap::new(),
            server: Server::default(),
            network,
        });
        info!("Remote console listening on {bound}");
        Ok(bound)
    }
}

impl StartupPolicy {
    fn bind(self, ui: &mut Console) -> std::io::Result<SocketAddr> {
        let address = startup_address(&ui.source.remote)?;
        if address.ip().is_loopback() {
            return ui.start_remote_console(address);
        }
        let network = self.0.ok_or_else(network::network_bind_denied)?;
        if !network::specific_ip(address.ip()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "network RCON requires a specific bind interface",
            ));
        }
        if address.is_ipv4() != network.peer.is_ipv4() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "RCON bind and peer addresses must use the same IP version",
            ));
        }
        ui.bind_remote_console(address, Some(network))
    }
}

fn limits(remote: &hl2_ui::console::remote::Remote, network: Option<&NetworkPolicy>) -> Limits {
    if let Some(network) = network {
        return Limits {
            password: network.password.clone(),
            max_failures: 5,
            min_failures: 3,
            min_failure_time: 60.,
            ban_minutes: 5.,
        };
    }
    Limits {
        password: remote.rcon_password.clone(),
        max_failures: remote.sv_rcon_maxfailures,
        min_failures: remote.sv_rcon_minfailures,
        min_failure_time: f64::from(remote.sv_rcon_minfailuretime),
        ban_minutes: f64::from(remote.sv_rcon_banpenalty),
    }
}

/// Starts listening on `ip`:`hostport` as the console's cvars stand.
fn startup_address(remote: &hl2_ui::console::remote::Remote) -> std::io::Result<SocketAddr> {
    let port = u16::try_from(remote.hostport).map_err(std::io::Error::other)?;
    let ip = if remote.ip.eq_ignore_ascii_case("localhost") {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    } else {
        remote.ip.parse().map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "RCON requires a numeric IP address or localhost",
            )
        })?
    };
    Ok(SocketAddr::new(ip, port))
}

/// Runs a remote command through the console with its output redirected; `say` lines are
/// shown locally too.
fn run(
    ui: &mut crate::ui_host::Console,
    sim: &mut crate::simulation::Simulation,
    game: Option<&mut crate::gameplay::Gameplay>,
    line: &str,
    from: SocketAddr,
) -> String {
    let commands = source_ui::console::command::tokenize(line);
    if ui.source.remote.sv_rcon_log && !ui.source.remote_log_skips(&commands) {
        ui.source.print(
            Level::Normal,
            format!("rcon from \"{from}\": command \"{line}\""),
        );
    }
    ui.source.remote.from_rcon = true;
    let redirect = ui.source.shell.panel.start_redirect();
    let effects = ui.source.execute_parsed(commands);
    if let Some(game) = game {
        sim.console_effects(ui, game, effects);
    }
    let lines = ui.source.shell.panel.finish_redirect(redirect);
    ui.source.remote.from_rcon = false;
    match ui.source.remote.snapshot.take() {
        Some(Some((level, text))) => ui.source.log_snapshot_with_level(level, text),
        Some(None) => ui.source.clear_snapshot(),
        None => {}
    }
    let console_open = ui.source.console_open();
    for line in std::mem::take(&mut ui.source.remote.broadcast) {
        if !console_open {
            sim.with_extensions(|extension, _| extension.remote_console_chat(&line));
        }
        ui.source.print(Level::ChatRemote, line);
    }
    lines
        .iter()
        .map(|line| format!("{}\n", line.text))
        .collect()
}

/// Takes what the sockets reported this frame and answers it.
fn serve(
    mut startup: ResMut<StartupListener>,
    mut ui: Option<ResMut<crate::ui_host::Console>>,
    mut sim: ResMut<crate::simulation::Simulation>,
    mut game: Option<ResMut<crate::gameplay::Gameplay>>,
    time: Res<Time<Real>>,
) {
    let Some(ui) = ui.as_deref_mut() else {
        return;
    };
    // `+rcon_password`, `+hostport` and `+ip` run once the first level loaded.
    if ui.source.startup_pending() {
        return;
    }
    if let Some(startup) = startup.0.take()
        && ui.remote_console.is_none()
        && let Err(error) = startup.bind(ui)
    {
        warn!("Could not start the remote console: {error}");
    }
    let Some(mut remote) = ui.remote_console.take() else {
        return;
    };
    let now = time.elapsed_secs_f64();
    let budget = if remote.network.is_some() {
        64
    } else {
        usize::MAX
    };
    for message in remote
        .messages
        .get_mut()
        .expect("game owns receiver")
        .try_iter()
        .take(budget)
    {
        let event = match message {
            Message::Writer(connection, writer) => {
                remote.writers.insert(connection, writer);
                continue;
            }
            Message::Event(event) => event,
        };
        if let server::Event::Closed(connection) = &event {
            remote.writers.remove(connection);
        }
        let limits = limits(&ui.source.remote, remote.network.as_ref());
        let replies = remote.server.handle(event, now, &limits, |line, from| {
            run(ui, &mut sim, game.as_deref_mut(), line, from)
        });
        for reply in replies {
            send(&mut remote.writers, reply);
        }
    }
    ui.remote_console = Some(remote);
}

fn send(writers: &mut HashMap<Connection, SyncSender<Outgoing>>, reply: Reply) {
    match reply {
        Reply::Send(connection, packets) => {
            if let Some(writer) = writers.get(&connection) {
                let bytes = packets.iter().flat_map(packet::Packet::encode).collect();
                if writer.try_send(Outgoing::Bytes(bytes)).is_err() {
                    writers.remove(&connection);
                }
            }
        }
        Reply::Close(connection) => {
            if let Some(writer) = writers.remove(&connection) {
                let _ = writer.try_send(Outgoing::Close);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Level, run};
    use crate::{extensions::RuntimeExtension, simulation::Simulation, ui_host::Console};
    use std::sync::{Arc, Mutex};

    struct Chat(Arc<Mutex<Vec<String>>>);

    impl RuntimeExtension for Chat {
        fn remote_console_chat(&mut self, line: &str) {
            self.0.lock().unwrap().push(line.to_owned());
        }
    }

    #[test]
    fn the_host_refuses_non_loopback_addresses_at_the_bind_boundary() {
        use std::io::ErrorKind;

        let mut console = Console::new(Default::default());
        for address in ["0.0.0.0:0", "[::]:0", "192.0.2.1:0", "[2001:db8::1]:0"] {
            let error = console
                .start_remote_console(address.parse().unwrap())
                .unwrap_err();
            assert_eq!(error.kind(), ErrorKind::PermissionDenied);
            assert!(console.remote_console_address().is_none());
        }
    }

    #[test]
    fn startup_resolves_localhost_without_dns_and_accepts_numeric_ipv6() {
        use hl2_ui::console::remote::Remote;

        let mut remote = Remote::default();
        assert_eq!(
            super::startup_address(&remote).unwrap(),
            "127.0.0.1:27015".parse().unwrap()
        );
        remote.ip = "::1".into();
        assert_eq!(
            super::startup_address(&remote).unwrap(),
            "[::1]:27015".parse().unwrap()
        );
        remote.ip = "game.example".into();
        assert!(super::startup_address(&remote).is_err());
    }

    #[test]
    fn an_explicit_listener_reports_its_actual_port_and_is_reused() {
        use std::net::{SocketAddr, TcpStream};

        let mut console = Console::new(Default::default());
        assert!(console.remote_console_address().is_none());
        let request = SocketAddr::from(([127, 0, 0, 1], 0));
        let bound = console.start_remote_console(request).unwrap();
        assert_eq!(bound.ip(), request.ip());
        assert_ne!(bound.port(), 0);
        assert_eq!(console.remote_console_address(), Some(bound));
        assert_eq!(console.start_remote_console(request).unwrap(), bound);
        assert!(TcpStream::connect(bound).is_ok());
    }

    #[test]
    fn only_remote_chat_broadcasts_with_the_console_closed_reach_runtime_extensions() {
        let mut console = Console::new(Default::default());
        let mut sim = Simulation::new(
            &modkit_core::World::default(),
            glam::Vec3::ZERO,
            0.,
            0.,
            false,
            vec![],
        );
        let lines = Arc::new(Mutex::new(Vec::new()));
        sim.extensions.push(Box::new(Chat(Arc::clone(&lines))));
        let from = "127.0.0.1:27015".parse().unwrap();
        console.source.submit("say player line");
        console
            .source
            .log_snapshot_with_level(Level::ChatRemote, "status line");
        let reply = run(
            &mut console,
            &mut sim,
            None,
            "echo command output; say reply",
            from,
        );
        assert!(reply.contains("command output"));
        assert!(reply.contains("Console: reply"));
        console.source.set_mode(hl2_ui::console::Mode::Console);
        console.source.focus_base();
        assert!(console.source.console_open());
        let open_reply = run(&mut console, &mut sim, None, "say visible in console", from);
        assert!(open_reply.contains("Console: visible in console"));
        assert!(
            console
                .source
                .shell
                .panel
                .output
                .iter()
                .any(|line| line.text == "Console: visible in console")
        );
        console.source.set_mode(hl2_ui::console::Mode::Gameplay);
        assert!(!console.source.console_open());
        run(&mut console, &mut sim, None, "say after close", from);
        assert_eq!(
            *lines.lock().unwrap(),
            ["Console: reply", "Console: after close"]
        );
        assert!(
            console
                .source
                .shell
                .panel
                .output
                .iter()
                .any(|line| line.text == "Console: reply")
        );
    }
}
