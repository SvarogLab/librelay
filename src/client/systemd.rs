//! Running under systemd: socket activation and readiness.

use std::io;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};

/// Takes the listening sockets systemd passed to this process (none unless socket-activated) and
/// unsets `LISTEN_PID`/`LISTEN_FDS`, so a spawned server doesn't see sockets meant for this
/// process. The fds come back close-on-exec.
///
/// # Safety
///
/// Changes the environment, so it must run while the process has a single thread — before any
/// async runtime starts.
pub unsafe fn take_listen_fds() -> io::Result<Vec<RawFd>> {
    // SAFETY: the caller guarantees a single thread.
    unsafe { sd_notify::listen_fds_and_unset_env() }.map(Iterator::collect)
}

/// Takes over a listening unix socket passed as `fd` (see [`take_listen_fds`]). Fails if `fd`
/// isn't a unix socket — a `.socket` unit with a TCP `ListenStream=`, say. Needs a tokio runtime.
pub fn listener_from_fd(fd: RawFd) -> io::Result<tokio::net::UnixListener> {
    // SAFETY: systemd passes each listening fd to this process alone, and `take_listen_fds`
    // hands it out exactly once.
    let listener = std::os::unix::net::UnixListener::from(unsafe { OwnedFd::from_raw_fd(fd) });
    // Fails with InvalidInput unless the fd is AF_UNIX.
    listener.local_addr()?;
    listener.set_nonblocking(true)?;
    tokio::net::UnixListener::from_std(listener)
}

/// Tells systemd the service is ready. A no-op unless it runs as `Type=notify`, which then counts
/// the service as started only now rather than at exec.
pub fn notify_ready() -> io::Result<()> {
    sd_notify::notify(&[sd_notify::NotifyState::Ready])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::IntoRawFd;
    use std::time::Duration;

    #[tokio::test]
    async fn listener_from_fd_accepts_on_an_already_bound_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.sock");
        let fd = std::os::unix::net::UnixListener::bind(&path)
            .unwrap()
            .into_raw_fd();

        let listener = listener_from_fd(fd).unwrap();
        let accepted = tokio::spawn(async move { listener.accept().await.map(|_| ()) });
        tokio::net::UnixStream::connect(&path).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), accepted)
            .await
            .expect("connection was not accepted")
            .unwrap()
            .unwrap();
    }

    #[test]
    fn listener_from_fd_refuses_a_tcp_socket() {
        let fd = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .into_raw_fd();
        assert!(listener_from_fd(fd).is_err());
    }
}
