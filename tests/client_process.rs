//! What the client sets up for its whole process: the shutdown signals and the log subscriber.
//! A binary of its own, since neither can be undone within a process.
#![cfg(all(feature = "client", unix))]

use std::time::Duration;

use librelay::client::{lifecycle, log};

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

#[test]
fn init_logs_by_the_filter_it_is_given() {
    log::init(log::LogFilter::parse(Some("warn".to_string())).unwrap());
    assert!(tracing::enabled!(tracing::Level::WARN));
    assert!(!tracing::enabled!(tracing::Level::INFO));
}
