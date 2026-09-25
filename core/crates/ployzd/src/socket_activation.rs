//! systemd socket activation.

use std::{
    fs, io,
    os::unix::{
        fs::{FileTypeExt, PermissionsExt},
        net::UnixListener,
    },
    path::Path,
};

use crate::filesystem::{MACHINE_API_SOCKET_MODE, set_ployz_group};

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

/// The Machine API listener: the socket `ployz.socket` passed, which must be
/// bound to `path`, or else `path` bound here (development, tests, and the
/// local testkit run without systemd).
///
/// Call it early in startup, before spawning threads or subprocesses: taking
/// the inherited socket clears `LISTEN_FDS` and marks the fd close-on-exec.
/// The caller must already hold the claim on `path`.
///
/// # Errors
///
/// Returns an error when the inherited socket is invalid or bound elsewhere,
/// or when binding `path` fails.
pub(crate) fn machine_api_listener(path: &Path) -> io::Result<UnixListener> {
    let listener = match inherited_unix_listener()? {
        Some(listener) => {
            require_bound_to(&listener, path)?;
            listener
        }
        None => bind_socket(path)?,
    };
    listener.set_nonblocking(true)?;
    Ok(listener)
}

fn bind_socket(path: &Path) -> io::Result<UnixListener> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => fs::remove_file(path)?,
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "refusing to replace a non-socket path",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(MACHINE_API_SOCKET_MODE))?;
    set_ployz_group(path)?;
    Ok(listener)
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
