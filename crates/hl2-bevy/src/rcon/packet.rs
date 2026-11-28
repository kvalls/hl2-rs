//! Valve's documented remote console packets: a little-endian `i32` size (of what follows),
//! `i32` id, `i32` type, the body and two NUL bytes.

/// `SERVERDATA_AUTH`: the client sends the password.
pub const AUTH: i32 = 3;
/// `SERVERDATA_AUTH_RESPONSE` (server to client) and `SERVERDATA_EXECCOMMAND` (client to server)
/// share the value.
pub const AUTH_RESPONSE: i32 = 2;
pub const EXEC_COMMAND: i32 = 2;
/// `SERVERDATA_RESPONSE_VALUE`: command output; a client sends an empty one after a command
/// and reads until its id comes back, which marks the end of the output.
pub const RESPONSE_VALUE: i32 = 0;
/// The id an authentication response carries when the password was wrong.
pub const AUTH_FAILED_ID: i32 = -1;

/// The id, type and body length plus the two NUL bytes: the smallest `size`.
pub const MIN_SIZE: usize = 10;
/// The most body text one response packet carries; longer output is split.
pub const MAX_BODY: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub id: i32,
    pub kind: i32,
    pub body: String,
}

impl Packet {
    pub fn new(id: i32, kind: i32, body: impl Into<String>) -> Self {
        Self {
            id,
            kind,
            body: body.into(),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let size = (MIN_SIZE + self.body.len()) as i32;
        let mut bytes = Vec::with_capacity(4 + MIN_SIZE + self.body.len());
        bytes.extend(size.to_le_bytes());
        bytes.extend(self.id.to_le_bytes());
        bytes.extend(self.kind.to_le_bytes());
        bytes.extend(self.body.as_bytes());
        bytes.extend([0, 0]);
        bytes
    }

    /// Decodes what follows the size field (`size` bytes).
    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() < MIN_SIZE {
            return None;
        }
        let word = |at: usize| i32::from_le_bytes(data[at..at + 4].try_into().expect("4 bytes"));
        let body = &data[8..data.len() - 2];
        let body = body.split(|&b| b == 0).next().unwrap_or_default();
        Some(Self::new(word(0), word(4), String::from_utf8_lossy(body)))
    }

    /// Command output as response packets with `id`, at most [`MAX_BODY`] bytes each, split on
    /// character boundaries; one empty packet when there is no output.
    pub fn responses(id: i32, text: &str) -> Vec<Self> {
        let mut packets = Vec::new();
        let mut rest = text;
        while !rest.is_empty() {
            let mut end = rest.len().min(MAX_BODY);
            while !rest.is_char_boundary(end) {
                end -= 1;
            }
            packets.push(Self::new(id, RESPONSE_VALUE, &rest[..end]));
            rest = &rest[end..];
        }
        if packets.is_empty() {
            packets.push(Self::new(id, RESPONSE_VALUE, ""));
        }
        packets
    }
}

/// Whether a size field announces a packet this server reads: at least [`MIN_SIZE`] and at
/// most `max` (`sv_rcon_maxpacketsize`).
pub fn acceptable_size(size: i32, max: usize) -> bool {
    usize::try_from(size).is_ok_and(|size| (MIN_SIZE..=max).contains(&size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets_round_trip_through_their_wire_form() {
        let packet = Packet::new(7, EXEC_COMMAND, "getpos");
        let bytes = packet.encode();
        assert_eq!(&bytes[..4], &(10 + 6i32).to_le_bytes());
        assert_eq!(&bytes[bytes.len() - 2..], &[0, 0]);
        assert_eq!(Packet::decode(&bytes[4..]), Some(packet));
        assert_eq!(Packet::decode(&[0; 9]), None);
    }

    #[test]
    fn long_output_is_split_on_character_boundaries() {
        let text = "é".repeat(MAX_BODY);
        let packets = Packet::responses(3, &text);
        assert!(
            packets
                .iter()
                .all(|p| p.id == 3 && p.body.len() <= MAX_BODY)
        );
        assert_eq!(
            packets.iter().map(|p| p.body.as_str()).collect::<String>(),
            text
        );
        assert_eq!(
            Packet::responses(4, ""),
            [Packet::new(4, RESPONSE_VALUE, "")]
        );
    }

    #[test]
    fn sizes_outside_the_limits_are_refused() {
        assert!(acceptable_size(10, 4096) && acceptable_size(4096, 4096));
        assert!(!acceptable_size(9, 4096) && !acceptable_size(4097, 4096));
        assert!(!acceptable_size(-5, 4096));
    }
}
