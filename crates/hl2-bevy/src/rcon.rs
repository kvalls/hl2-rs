//! The remote console (`-usercon`): Valve's RCON protocol on `ip`:`hostport`. Sockets run on
//! their own threads ([`listener`]); each frame the game thread takes what arrived without
//! waiting, runs authenticated commands through the console like typed ones and sends their
//! output back instead of showing it ([`server`]). Without `-usercon` none of this exists.

mod listener;
mod packet;
mod server;

use bevy::prelude::{App, Local, Real, Res, ResMut, Time, Update, info, warn};
use listener::{Message, Outgoing};
use server::{Connection, Limits, Reply, Server};
use source_ui::console::panel::Level;
use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::mpsc::{Receiver, Sender};

/// The running remote console.
struct RemoteConsole {
    messages: Receiver<Message>,
    writers: HashMap<Connection, Sender<Outgoing>>,
    server: Server,
}

/// Adds the remote console's system (`-usercon`).
pub fn install(app: &mut App) {
    app.add_systems(Update, serve);
}

fn limits(remote: &hl2_ui::console::remote::Remote) -> Limits {
    Limits {
        password: remote.rcon_password.clone(),
        max_failures: remote.sv_rcon_maxfailures,
        min_failures: remote.sv_rcon_minfailures,
        min_failure_time: f64::from(remote.sv_rcon_minfailuretime),
        ban_minutes: f64::from(remote.sv_rcon_banpenalty),
    }
}

/// Starts listening on `ip`:`hostport` as the console's cvars stand.
fn start(remote: &hl2_ui::console::remote::Remote) -> std::io::Result<RemoteConsole> {
    let port = u16::try_from(remote.hostport).map_err(std::io::Error::other)?;
    let address = (remote.ip.as_str(), port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| std::io::Error::other("the address does not resolve"))?;
    let max_packet = remote.sv_rcon_maxpacketsize as usize;
    let (messages, bound) = listener::start(address, max_packet)?;
    info!("Remote console listening on {bound}");
    Ok(RemoteConsole {
        messages,
        writers: HashMap::new(),
        server: Server::default(),
    })
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
    if ui.source.remote.sv_rcon_log && !hl2_ui::console::remote::chat_only(&commands) {
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
        Some(Some(text)) => ui.source.log_snapshot(text),
        Some(None) => ui.source.clear_snapshot(),
        None => {}
    }
    for line in std::mem::take(&mut ui.source.remote.broadcast) {
        ui.source.print(Level::ChatRemote, line);
    }
    lines
        .iter()
        .map(|line| format!("{}\n", line.text))
        .collect()
}

/// Takes what the sockets reported this frame and answers it.
fn serve(
    mut remote: Local<Option<RemoteConsole>>,
    mut failed: Local<bool>,
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
    if remote.is_none() && !*failed {
        match start(&ui.source.remote) {
            Ok(started) => *remote = Some(started),
            Err(error) => {
                warn!("Could not start the remote console: {error}");
                *failed = true;
            }
        }
    }
    let Some(remote) = remote.as_mut() else {
        return;
    };
    let now = time.elapsed_secs_f64();
    while let Ok(message) = remote.messages.try_recv() {
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
        let limits = limits(&ui.source.remote);
        let replies = remote.server.handle(event, now, &limits, |line, from| {
            run(ui, &mut sim, game.as_deref_mut(), line, from)
        });
        for reply in replies {
            send(&mut remote.writers, reply);
        }
    }
}

fn send(writers: &mut HashMap<Connection, Sender<Outgoing>>, reply: Reply) {
    match reply {
        Reply::Send(connection, packets) => {
            if let Some(writer) = writers.get(&connection) {
                let bytes = packets.iter().flat_map(packet::Packet::encode).collect();
                let _ = writer.send(Outgoing::Bytes(bytes));
            }
        }
        Reply::Close(connection) => {
            if let Some(writer) = writers.remove(&connection) {
                let _ = writer.send(Outgoing::Close);
            }
        }
    }
}
