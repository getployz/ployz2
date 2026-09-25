//! systemd socket activation.

use std::{io, os::unix::net::UnixListener, path::Path};

/// Takes the Unix listener systemd socket activation passed, if any.
///
/// # Errors
///
/// Returns an error when systemd passed more than one socket or a socket that
/// is not a Unix stream listener.
pub fn inherited_unix_listener() -> io::Result<Option<UnixListener>> {
    let mut inherited = listenfd::ListenFd::from_env();
    if inherited.len() > 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "expected at most one systemd socket, received {}",
                inherited.len()
            ),
        ));
    }
    inherited.take_unix_listener(0)
}

/// Takes the socket `ployz.socket` passed, or `None` when run without systemd
/// (development, tests, and the local testkit) so the caller binds `path`
/// itself. An inherited socket must be bound to `path`.
pub(crate) fn listen_socket(path: &Path) -> io::Result<Option<tokio::net::UnixListener>> {
    let Some(listener) = inherited_unix_listener()? else {
        return Ok(None);
    };
    require_bound_to(&listener, path)?;
    listener.set_nonblocking(true)?;
    tokio::net::UnixListener::from_std(listener).map(Some)
}

fn require_bound_to(listener: &UnixListener, path: &Path) -> io::Result<()> {
    let address = listener.local_addr()?;
    if address.as_pathname() != Some(path) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "systemd passed a socket bound to {address:?}, expected {}",
                path.display()
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, io, os::unix::net::UnixListener};

    use super::require_bound_to;
    use crate::test_dir::TestDir;

    #[test]
    fn inherited_socket_must_be_bound_to_the_claimed_path() {
        let root = TestDir::new("ployzd-socket-inherited-path");
        fs::create_dir_all(&root.0).unwrap();
        let bound = root.0.join("other.sock");
        let listener = UnixListener::bind(&bound).unwrap();
        require_bound_to(&listener, &bound).unwrap();
        let expected = root.0.join("ployz.sock");
        let error = require_bound_to(&listener, &expected).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        let message = error.to_string();
        assert!(message.contains("other.sock"), "{message}");
        assert!(
            message.contains(&expected.display().to_string()),
            "{message}"
        );
    }
}
