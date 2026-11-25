//! The remote console's sockets: one thread accepts, and each connection gets a reader and a
//! writer thread. The game thread only sees [`Message`]s and sends [`Outgoing`] bytes, so a slow
//! client never blocks a frame.

use super::packet::{self, Packet};
use super::server::{Connection, Event};
use std::io::{Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::{
    Arc,
    mpsc::{self, Receiver, SyncSender},
};
use std::time::Duration;

/// What the sockets report to the game thread.
pub enum Message {
    Event(Event),
    /// Where replies to a new connection go; sent before its `Connected` event.
    Writer(Connection, SyncSender<Outgoing>),
}

/// What the game thread asks a connection's writer to do.
pub enum Outgoing {
    Bytes(Vec<u8>),
    Close,
}

/// Binds `address` and accepts connections on a thread; packets announcing more than
/// `max_packet` bytes end the connection.
pub fn start(
    address: SocketAddr,
    max_packet: usize,
    peer: Option<IpAddr>,
) -> std::io::Result<(Receiver<Message>, SocketAddr)> {
    let listener = TcpListener::bind(address)?;
    let bound = listener.local_addr()?;
    let (messages, receiver) = mpsc::sync_channel(256);
    std::thread::Builder::new()
        .name("rcon-accept".into())
        .spawn(move || accept(&listener, &messages, max_packet, peer))?;
    Ok((receiver, bound))
}

fn accept(
    listener: &TcpListener,
    messages: &SyncSender<Message>,
    max_packet: usize,
    peer: Option<IpAddr>,
) {
    let workers = Arc::new(());
    for (connection, stream) in (1..).zip(listener.incoming()) {
        let Ok(stream) = stream else {
            continue;
        };
        if let Some(peer) = peer {
            // Only the configured peer can consume worker slots or enter the game queue.
            if stream.peer_addr().map(|address| address.ip()).ok() != Some(peer)
                || Arc::strong_count(&workers) > 7
                || stream
                    .set_read_timeout(Some(Duration::from_secs(30)))
                    .is_err()
                || stream
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .is_err()
            {
                continue;
            }
        }
        let max_packet = if peer.is_some() {
            max_packet.min(4096)
        } else {
            max_packet
        };
        let worker_slot = peer.map(|_| Arc::clone(&workers));
        if spawn_connection(connection, stream, messages, max_packet, worker_slot).is_err() {
            continue;
        }
    }
}

fn spawn_connection(
    connection: Connection,
    stream: TcpStream,
    messages: &SyncSender<Message>,
    max_packet: usize,
    worker_slot: Option<Arc<()>>,
) -> std::io::Result<()> {
    let address = stream.peer_addr()?;
    let writer_stream = stream.try_clone()?;
    let (writer, outgoing) = mpsc::sync_channel(32);
    let writer_slot = worker_slot.clone();
    std::thread::Builder::new()
        .name("rcon-write".into())
        .spawn(move || {
            let _slot = writer_slot;
            write(writer_stream, &outgoing);
        })?;
    let _ = messages.send(Message::Writer(connection, writer));
    let _ = messages.send(Message::Event(Event::Connected(connection, address)));
    let reader_messages = messages.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("rcon-read".into())
        .spawn(move || {
            let _slot = worker_slot;
            read(connection, stream, &reader_messages, max_packet);
        })
    {
        let _ = messages.send(Message::Event(Event::Closed(connection)));
        return Err(error);
    }
    Ok(())
}

fn write(mut stream: TcpStream, outgoing: &Receiver<Outgoing>) {
    while let Ok(Outgoing::Bytes(bytes)) = outgoing.recv() {
        if stream.write_all(&bytes).is_err() {
            break;
        }
    }
    let _ = stream.shutdown(Shutdown::Both);
}

fn read(connection: Connection, mut stream: TcpStream, messages: &SyncSender<Message>, max: usize) {
    let event = loop {
        let mut size = [0; 4];
        if stream.read_exact(&mut size).is_err() {
            break Event::Closed(connection);
        }
        let size = i32::from_le_bytes(size);
        if !packet::acceptable_size(size, max) {
            break Event::Oversized(connection);
        }
        let mut data = vec![0; size as usize];
        if stream.read_exact(&mut data).is_err() {
            break Event::Closed(connection);
        }
        let Some(packet) = Packet::decode(&data) else {
            break Event::Oversized(connection);
        };
        if messages
            .send(Message::Event(Event::Packet(connection, packet)))
            .is_err()
        {
            return;
        }
    };
    let _ = messages.send(Message::Event(event));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn next_event(messages: &Receiver<Message>) -> (Option<SyncSender<Outgoing>>, Event) {
        let mut writer = None;
        loop {
            match messages
                .recv_timeout(Duration::from_secs(5))
                .expect("message")
            {
                Message::Writer(_, sender) => writer = Some(sender),
                Message::Event(event) => return (writer, event),
            }
        }
    }

    #[test]
    fn out_of_policy_peers_never_reach_the_game_queue() {
        let local = SocketAddr::from(([127, 0, 0, 1], 0));
        let (messages, address) = start(local, 1024, Some("192.0.2.1".parse().unwrap())).unwrap();
        let mut client = TcpStream::connect(address).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        // The listener can close before or after the handshake reaches the client.
        match client.read(&mut [0]) {
            Ok(0) => {}
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
            result => panic!("unexpected read result: {result:?}"),
        }
        assert!(messages.try_recv().is_err());
    }

    #[test]
    fn packets_arrive_and_replies_go_back_over_a_real_socket() {
        let (messages, address) =
            start(SocketAddr::from(([127, 0, 0, 1], 0)), 1024, None).expect("bind");
        let mut client = TcpStream::connect(address).expect("connect");
        let (writer, connected) = next_event(&messages);
        assert!(matches!(connected, Event::Connected(..)));
        client
            .write_all(&Packet::new(5, packet::AUTH, "pw").encode())
            .expect("send");
        let (_, Event::Packet(_, packet)) = next_event(&messages) else {
            panic!("expected a packet");
        };
        assert_eq!(packet, Packet::new(5, packet::AUTH, "pw"));
        let reply = Packet::new(5, packet::AUTH_RESPONSE, "");
        let writer = writer.expect("writer");
        writer.send(Outgoing::Bytes(reply.encode())).expect("reply");
        let mut bytes = vec![0; reply.encode().len()];
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        client.read_exact(&mut bytes).expect("read");
        assert_eq!(Packet::decode(&bytes[4..]), Some(reply));
        client.write_all(&[0xff, 0xff, 0, 0]).expect("oversized");
        assert!(matches!(next_event(&messages).1, Event::Oversized(_)));
    }
}
