//! The remote console's sockets: one thread accepts, and each connection gets a reader and a
//! writer thread. The game thread only sees [`Message`]s and sends [`Outgoing`] bytes, so a slow
//! client never blocks a frame.

use super::packet::{self, Packet};
use super::server::{Connection, Event};
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};

/// What the sockets report to the game thread.
pub enum Message {
    Event(Event),
    /// Where replies to a new connection go; sent before its `Connected` event.
    Writer(Connection, Sender<Outgoing>),
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
) -> std::io::Result<(Receiver<Message>, SocketAddr)> {
    let listener = TcpListener::bind(address)?;
    let bound = listener.local_addr()?;
    let (messages, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("rcon-accept".into())
        .spawn(move || accept(&listener, &messages, max_packet))?;
    Ok((receiver, bound))
}

fn accept(listener: &TcpListener, messages: &Sender<Message>, max_packet: usize) {
    for (connection, stream) in (1..).zip(listener.incoming()) {
        let Ok(stream) = stream else {
            continue;
        };
        if spawn_connection(connection, stream, messages, max_packet).is_err() {
            continue;
        }
    }
}

fn spawn_connection(
    connection: Connection,
    stream: TcpStream,
    messages: &Sender<Message>,
    max_packet: usize,
) -> std::io::Result<()> {
    let address = stream.peer_addr()?;
    let writer_stream = stream.try_clone()?;
    let (writer, outgoing) = mpsc::channel();
    std::thread::Builder::new()
        .name("rcon-write".into())
        .spawn(move || write(writer_stream, &outgoing))?;
    let _ = messages.send(Message::Writer(connection, writer));
    let _ = messages.send(Message::Event(Event::Connected(connection, address)));
    let reader_messages = messages.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("rcon-read".into())
        .spawn(move || read(connection, stream, &reader_messages, max_packet))
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

fn read(connection: Connection, mut stream: TcpStream, messages: &Sender<Message>, max: usize) {
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

    fn next_event(messages: &Receiver<Message>) -> (Option<Sender<Outgoing>>, Event) {
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
    fn packets_arrive_and_replies_go_back_over_a_real_socket() {
        let (messages, address) = start(SocketAddr::from(([127, 0, 0, 1], 0)), 1024).expect("bind");
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
