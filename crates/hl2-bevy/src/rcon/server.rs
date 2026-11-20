//! What the remote console answers, on the game thread: authentication with failure bans,
//! commands and the end-of-output marker. Sockets live in `listener`; this only sees events.

use super::packet::{self, Packet};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};

/// A connection's number, given by the listener.
pub type Connection = u64;

/// The `sv_rcon_*` limits and the password, read from the console's cvars each event.
#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    pub password: String,
    /// Failures in total before a ban (`sv_rcon_maxfailures`).
    pub max_failures: u32,
    /// Failures within `min_failure_time` before a ban (`sv_rcon_minfailures`).
    pub min_failures: u32,
    /// Seconds failures are counted for `min_failures` (`sv_rcon_minfailuretime`).
    pub min_failure_time: f64,
    /// Minutes a ban lasts, 0 for the rest of the run (`sv_rcon_banpenalty`).
    pub ban_minutes: f64,
}

/// What the listener reports.
#[derive(Debug)]
pub enum Event {
    Connected(Connection, SocketAddr),
    Packet(Connection, Packet),
    /// The client announced a packet larger than `sv_rcon_maxpacketsize` (or malformed).
    Oversized(Connection),
    Closed(Connection),
}

/// What the listener does in reply.
#[derive(Debug, PartialEq)]
pub enum Reply {
    Send(Connection, Vec<Packet>),
    Close(Connection),
}

#[derive(Default)]
pub struct Server {
    addresses: HashMap<Connection, SocketAddr>,
    authenticated: HashSet<Connection>,
    /// Failure times per address (for `min_failures`) and the total.
    failures: HashMap<IpAddr, (Vec<f64>, u32)>,
    /// Banned addresses and when the ban ends (`None`: the rest of the run).
    banned: HashMap<IpAddr, Option<f64>>,
}

impl Server {
    fn is_banned(&mut self, ip: IpAddr, now: f64) -> bool {
        match self.banned.get(&ip) {
            Some(None) => true,
            Some(Some(until)) if now < *until => true,
            Some(Some(_)) => {
                self.banned.remove(&ip);
                false
            }
            None => false,
        }
    }

    /// Counts a failed authentication; true when it bans the address.
    fn fail(&mut self, ip: IpAddr, now: f64, limits: &Limits) -> bool {
        let (times, total) = self.failures.entry(ip).or_default();
        times.retain(|time| now - time < limits.min_failure_time);
        times.push(now);
        *total += 1;
        let ban = times.len() >= limits.min_failures as usize || *total >= limits.max_failures;
        if ban {
            self.failures.remove(&ip);
            let until = (limits.ban_minutes > 0.).then_some(now + limits.ban_minutes * 60.);
            self.banned.insert(ip, until);
        }
        ban
    }

    /// Answers one event; `run` executes a command line and returns its output.
    pub fn handle(
        &mut self,
        event: Event,
        now: f64,
        limits: &Limits,
        run: impl FnOnce(&str, SocketAddr) -> String,
    ) -> Vec<Reply> {
        match event {
            Event::Connected(connection, address) => {
                if self.is_banned(address.ip(), now) {
                    return vec![Reply::Close(connection)];
                }
                self.addresses.insert(connection, address);
                vec![]
            }
            Event::Closed(connection) => {
                self.forget(connection);
                vec![]
            }
            Event::Oversized(connection) => {
                if let Some(address) = self.addresses.get(&connection) {
                    self.banned.insert(address.ip(), None);
                }
                self.forget(connection);
                vec![Reply::Close(connection)]
            }
            Event::Packet(connection, packet) => self.packet(connection, packet, now, limits, run),
        }
    }

    fn forget(&mut self, connection: Connection) {
        self.addresses.remove(&connection);
        self.authenticated.remove(&connection);
    }

    fn packet(
        &mut self,
        connection: Connection,
        packet: Packet,
        now: f64,
        limits: &Limits,
        run: impl FnOnce(&str, SocketAddr) -> String,
    ) -> Vec<Reply> {
        let Some(&address) = self.addresses.get(&connection) else {
            return vec![Reply::Close(connection)];
        };
        let authenticated = self.authenticated.contains(&connection);
        match packet.kind {
            packet::AUTH => {
                let accepted = !limits.password.is_empty() && packet.body == limits.password;
                let empty = Packet::new(packet.id, packet::RESPONSE_VALUE, "");
                if accepted {
                    self.authenticated.insert(connection);
                    let response = Packet::new(packet.id, packet::AUTH_RESPONSE, "");
                    return vec![Reply::Send(connection, vec![empty, response])];
                }
                self.authenticated.remove(&connection);
                let failed = Packet::new(packet::AUTH_FAILED_ID, packet::AUTH_RESPONSE, "");
                let mut replies = vec![Reply::Send(connection, vec![empty, failed])];
                if !limits.password.is_empty() && self.fail(address.ip(), now, limits) {
                    self.forget(connection);
                    replies.push(Reply::Close(connection));
                }
                replies
            }
            _ if !authenticated => {
                self.forget(connection);
                vec![Reply::Close(connection)]
            }
            packet::EXEC_COMMAND => {
                let output = run(&packet.body, address);
                vec![Reply::Send(
                    connection,
                    Packet::responses(packet.id, &output),
                )]
            }
            _ => {
                let marker = Packet::new(packet.id, packet::RESPONSE_VALUE, "");
                vec![Reply::Send(connection, vec![marker])]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> Limits {
        Limits {
            password: "secret".into(),
            max_failures: 10,
            min_failures: 5,
            min_failure_time: 30.,
            ban_minutes: 0.,
        }
    }

    fn address(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    fn auth(server: &mut Server, connection: Connection, password: &str, now: f64) -> Vec<Reply> {
        let packet = Packet::new(1, packet::AUTH, password);
        server.handle(
            Event::Packet(connection, packet),
            now,
            &limits(),
            |_, _| unreachable!(),
        )
    }

    fn exec(server: &mut Server, connection: Connection, line: &str) -> Vec<Reply> {
        let packet = Packet::new(2, packet::EXEC_COMMAND, line);
        server.handle(
            Event::Packet(connection, packet),
            0.,
            &limits(),
            |line, _| format!("ran {line}\n"),
        )
    }

    #[test]
    fn commands_run_only_after_the_password() {
        let mut server = Server::default();
        server.handle(
            Event::Connected(1, address(1)),
            0.,
            &limits(),
            |_, _| unreachable!(),
        );
        assert_eq!(exec(&mut server, 1, "getpos"), [Reply::Close(1)]);
        server.handle(
            Event::Connected(2, address(2)),
            0.,
            &limits(),
            |_, _| unreachable!(),
        );
        let ok = auth(&mut server, 2, "secret", 0.);
        let Reply::Send(2, packets) = &ok[0] else {
            panic!("{ok:?}")
        };
        assert_eq!(packets[1], Packet::new(1, packet::AUTH_RESPONSE, ""));
        assert_eq!(
            exec(&mut server, 2, "getpos"),
            [Reply::Send(
                2,
                vec![Packet::new(2, packet::RESPONSE_VALUE, "ran getpos\n")]
            )]
        );
        let marker = Packet::new(3, packet::RESPONSE_VALUE, "");
        let replies = server.handle(
            Event::Packet(2, marker.clone()),
            0.,
            &limits(),
            |_, _| unreachable!(),
        );
        assert_eq!(replies, [Reply::Send(2, vec![marker])]);
    }

    #[test]
    fn repeated_failures_ban_the_address() {
        let mut server = Server::default();
        server.handle(
            Event::Connected(1, address(1)),
            0.,
            &limits(),
            |_, _| unreachable!(),
        );
        for attempt in 0..4 {
            let replies = auth(&mut server, 1, "wrong", f64::from(attempt));
            let Reply::Send(1, packets) = &replies[0] else {
                panic!("{replies:?}")
            };
            assert_eq!(packets[1].id, packet::AUTH_FAILED_ID);
            assert_eq!(replies.len(), 1);
        }
        assert_eq!(
            auth(&mut server, 1, "wrong", 4.).last(),
            Some(&Reply::Close(1))
        );
        let again = server.handle(
            Event::Connected(2, address(2)),
            5.,
            &limits(),
            |_, _| unreachable!(),
        );
        assert_eq!(again, [Reply::Close(2)]);
    }

    #[test]
    fn failures_older_than_the_window_do_not_count_and_bans_expire() {
        let mut server = Server::default();
        let timed = Limits {
            ban_minutes: 1.,
            ..limits()
        };
        server.handle(
            Event::Connected(1, address(1)),
            0.,
            &timed,
            |_, _| unreachable!(),
        );
        for attempt in 0..9 {
            let packet = Packet::new(1, packet::AUTH, "wrong");
            let replies = server.handle(
                Event::Packet(1, packet),
                f64::from(attempt) * 31.,
                &timed,
                |_, _| unreachable!(),
            );
            assert_eq!(replies.len(), 1, "attempt {attempt}");
        }
        let packet = Packet::new(1, packet::AUTH, "wrong");
        let replies = server.handle(
            Event::Packet(1, packet),
            300.,
            &timed,
            |_, _| unreachable!(),
        );
        assert_eq!(replies.last(), Some(&Reply::Close(1)));
        let early = server.handle(
            Event::Connected(2, address(2)),
            330.,
            &timed,
            |_, _| unreachable!(),
        );
        assert_eq!(early, [Reply::Close(2)]);
        let late = server.handle(
            Event::Connected(3, address(3)),
            361.,
            &timed,
            |_, _| unreachable!(),
        );
        assert!(late.is_empty());
    }

    #[test]
    fn an_empty_password_refuses_without_banning() {
        let mut server = Server::default();
        let open = Limits {
            password: String::new(),
            ..limits()
        };
        server.handle(
            Event::Connected(1, address(1)),
            0.,
            &open,
            |_, _| unreachable!(),
        );
        for _ in 0..20 {
            let packet = Packet::new(1, packet::AUTH, "");
            let replies = server.handle(Event::Packet(1, packet), 0., &open, |_, _| unreachable!());
            assert_eq!(replies.len(), 1);
        }
    }

    #[test]
    fn oversized_packets_ban_and_close() {
        let mut server = Server::default();
        server.handle(
            Event::Connected(1, address(1)),
            0.,
            &limits(),
            |_, _| unreachable!(),
        );
        assert_eq!(
            server.handle(Event::Oversized(1), 0., &limits(), |_, _| unreachable!()),
            [Reply::Close(1)]
        );
        let again = server.handle(
            Event::Connected(2, address(2)),
            0.,
            &limits(),
            |_, _| unreachable!(),
        );
        assert_eq!(again, [Reply::Close(2)]);
    }
}
