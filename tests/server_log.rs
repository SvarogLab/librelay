//! The server's log subscriber, in a binary of its own: it can be set up once per process.
#![cfg(feature = "server")]

use clap::Parser;
use librelay::server::log::{self, LogArgs};

#[derive(Parser)]
struct Config {
    #[command(flatten)]
    log: LogArgs,
}

#[test]
fn init_logs_by_the_filter_the_client_passed() {
    let config =
        Config::try_parse_from(["relay-server", librelay::LOG_FILTER_FLAG, "warn"]).unwrap();
    log::init(&config.log);
    assert!(tracing::enabled!(tracing::Level::WARN));
    assert!(!tracing::enabled!(tracing::Level::INFO));
}
