//! Readiness reaches the socket systemd names in `NOTIFY_SOCKET`. A binary of its own, since it
//! sets that variable for the whole process.
#![cfg(all(feature = "client", unix))]

use std::os::unix::net::UnixDatagram;
use std::time::Duration;

#[test]
fn notify_ready_tells_systemd_the_service_is_ready() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notify.sock");
    let systemd = UnixDatagram::bind(&path).unwrap();
    systemd
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    // SAFETY: the only test in this binary, so no other thread reads the environment.
    unsafe { std::env::set_var("NOTIFY_SOCKET", &path) };

    librelay::client::systemd::notify_ready().unwrap();

    let mut message = [0; 64];
    let len = systemd.recv(&mut message).unwrap();
    let message = String::from_utf8_lossy(&message[..len]);
    assert!(message.lines().any(|line| line == "READY=1"), "{message:?}");
}
