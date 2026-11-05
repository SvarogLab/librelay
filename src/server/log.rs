//! The server's logging, filtered by what the client passes (see `client::log::LogFilter`).

/// The server's log filter, flattened into its own `clap` arguments with
/// `#[command(flatten)]`. Required, like every server flag: the client always passes it.
#[derive(clap::Args, Debug, Clone)]
pub struct LogArgs {
    /// `tracing` filter for this process's diagnostics, in `RUST_LOG` syntax. A flag rather than
    /// `RUST_LOG` itself, since environment variables don't cross the WSL interop boundary.
    // A filter the client accepts may start with a hyphen, and must not read as a flag here.
    #[arg(long = "log-filter", value_parser = parse_log_filter, allow_hyphen_values = true)]
    pub filter: String,
}

fn parse_log_filter(value: &str) -> Result<String, String> {
    crate::log::parse_filter(value)
        .map(|(directives, _)| directives)
        .map_err(|e| e.to_string())
}

/// Logs to stderr: colored and timestamped on a terminal, plain lines for the client to forward
/// otherwise.
///
/// # Panics
///
/// If logging is already set up in this process.
pub fn init(args: &LogArgs) {
    let (_, filter) =
        crate::log::parse_filter(&args.filter).expect("--log-filter is validated while parsing");
    crate::log::init_stderr(filter);
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Config {
        #[command(flatten)]
        log: LogArgs,
    }

    #[test]
    fn requires_a_log_filter() {
        assert!(Config::try_parse_from(["relay-server"]).is_err());
    }

    #[test]
    fn takes_the_filter_under_the_flag_the_client_passes() {
        let config =
            Config::try_parse_from(["relay-server", crate::LOG_FILTER_FLAG, "debug,yamux=warn"])
                .unwrap();
        assert_eq!(config.log.filter, "debug,yamux=warn");
    }

    /// The client accepts these as filters, so the server must not take them for flags.
    #[test]
    fn takes_a_filter_that_starts_with_a_hyphen() {
        for filter in ["--", "-x"] {
            let args = ["relay-server", crate::LOG_FILTER_FLAG, filter];
            assert_eq!(Config::try_parse_from(args).unwrap().log.filter, filter);
        }
    }

    #[test]
    fn whitespace_around_a_directive_is_not_part_of_it() {
        let args = ["relay-server", crate::LOG_FILTER_FLAG, "info, ü=warn"];
        assert_eq!(
            Config::try_parse_from(args).unwrap().log.filter,
            "info,ü=warn"
        );
    }

    #[test]
    fn refuses_an_invalid_filter() {
        let args = ["relay-server", crate::LOG_FILTER_FLAG, "relay=loud"];
        assert!(Config::try_parse_from(args).is_err());
    }
}
