//! The client's shutdown signals, in a binary of its own: their handlers stay installed for the
//! rest of the process.
#![cfg(all(feature = "client", unix))]

use std::time::Duration;

use librelay::client::lifecycle;

#[tokio::test]
async fn shutdown_signal_waits_for_sigterm() {
    let mut signal = std::pin::pin!(lifecycle::shutdown_signal());
    tokio::time::timeout(Duration::from_millis(100), &mut signal)
        .await
        .expect_err("resolved with no signal sent");

    let killed = std::process::Command::new("kill")
        .args(["-TERM", &std::process::id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    tokio::time::timeout(Duration::from_secs(5), signal)
        .await
        .expect("SIGTERM did not resolve it")
        .unwrap();
}
