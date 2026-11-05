//! The client and the server agree on every `RUST_LOG`: a filter the client accepts reaches the
//! server unchanged, and one it refuses the server would refuse too — so a typo is reported by
//! the client, never by a server exiting at startup.
#![no_main]

use clap::Parser;
use libfuzzer_sys::fuzz_target;
use librelay::client::log::LogFilter;
use librelay::server::log::LogArgs;

#[derive(Parser)]
struct ServerConfig {
    #[command(flatten)]
    log: LogArgs,
}

fuzz_target!(|rust_log: &str| {
    match LogFilter::parse(Some(rust_log.to_string())) {
        Ok(filter) => {
            let args = std::iter::once("relay-server".to_string()).chain(filter.server_args());
            let config = ServerConfig::try_parse_from(args)
                .unwrap_or_else(|e| panic!("the server refuses {:?}: {e}", filter.as_str()));
            assert_eq!(config.log.filter, filter.as_str());
        }
        Err(_) => {
            let args = ["relay-server", librelay::LOG_FILTER_FLAG, rust_log];
            assert!(ServerConfig::try_parse_from(args).is_err(), "{rust_log:?}");
        }
    }
});
