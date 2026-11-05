//! The server's log subscriber, in a binary of its own: it can be set up once per process.
#![cfg(feature = "server")]

use librelay::server::log::{self, LogArgs};

/// Built by hand rather than parsed, the filter must still be read as `--log-filter` reads it.
#[test]
fn init_logs_by_the_filter_it_is_given() {
    log::init(&LogArgs {
        filter: "warn, ü=error".to_string(),
    });
    assert!(tracing::enabled!(tracing::Level::WARN));
    assert!(!tracing::enabled!(tracing::Level::INFO));
}
