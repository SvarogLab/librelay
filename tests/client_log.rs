//! The client's log subscriber, in a binary of its own: it can be set up once per process.
#![cfg(all(feature = "client", unix))]

use librelay::client::log;

#[test]
fn init_logs_by_the_filter_it_is_given() {
    log::init(log::LogFilter::parse(Some("warn".to_string())).unwrap());
    assert!(tracing::enabled!(tracing::Level::WARN));
    assert!(!tracing::enabled!(tracing::Level::INFO));
}
