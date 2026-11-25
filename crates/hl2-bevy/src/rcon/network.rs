//! Startup policy for an explicitly exposed Source RCON listener.
use anyhow::{Context, ensure};
use std::io::{self, Read};
use std::net::IpAddr;
use std::path::Path;

pub(super) struct NetworkPolicy {
    pub peer: IpAddr,
    pub password: String,
}

impl NetworkPolicy {
    pub(super) fn load(options: &crate::Options) -> anyhow::Result<Option<Self>> {
        let (Some(peer), Some(path)) = (options.rcon_peer, &options.rcon_password_file) else {
            ensure!(
                options.rcon_peer.is_none() && options.rcon_password_file.is_none(),
                "network RCON requires both --rcon-peer and --rcon-password-file"
            );
            return Ok(None);
        };
        ensure!(options.usercon, "network RCON requires -usercon");
        ensure!(
            specific_ip(peer),
            "RCON requires a specific peer IP address"
        );
        let password =
            read_password(path).context("could not load the network RCON password file")?;
        Ok(Some(Self { peer, password }))
    }
}

pub(super) fn network_bind_denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "non-loopback RCON requires --rcon-peer and --rcon-password-file",
    )
}

pub(super) fn specific_ip(ip: IpAddr) -> bool {
    !ip.is_unspecified()
        && !ip.is_multicast()
        && match ip {
            IpAddr::V4(ip) => !ip.is_broadcast(),
            IpAddr::V6(ip) => ip.to_ipv4_mapped().is_none(),
        }
}

fn read_password(path: &Path) -> anyhow::Result<String> {
    ensure!(
        std::fs::symlink_metadata(path)?.is_file(),
        "use a regular password file"
    );
    let file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= 64,
        "use a regular password file of at most 64 bytes"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "password file must be private (chmod 600)"
        );
    }
    let mut contents = String::new();
    file.take(65).read_to_string(&mut contents)?;
    let password = contents.trim_end_matches(['\r', '\n']);
    ensure!(
        contents.len() <= 64
            && password.len() == 48
            && password
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "use a cryptographically generated 48-character lowercase hexadecimal password"
    );
    Ok(password.to_owned())
}

#[cfg(test)]
mod tests {
    use super::specific_ip;

    #[test]
    fn network_addresses_must_be_unambiguous_and_specific() {
        for address in [
            "0.0.0.0",
            "::",
            "224.0.0.1",
            "ff02::1",
            "255.255.255.255",
            "::ffff:192.0.2.1",
        ] {
            assert!(!specific_ip(address.parse().unwrap()));
        }
        assert!(specific_ip("192.0.2.1".parse().unwrap()));
        assert!(specific_ip("2001:db8::1".parse().unwrap()));
    }
}
