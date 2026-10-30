//! The server's logging, filtered by what the client passes (see `client::log::LogFilter`).

/// The server's log filter, flattened into its own `clap` arguments with
/// `#[command(flatten)]`. Required, like every server flag: the client always passes it.
#[derive(clap::Args, Debug, Clone)]
pub struct LogArgs {
    /// `tracing` filter for this process's diagnostics, in `RUST_LOG` syntax. A flag rather than
    /// `RUST_LOG` itself, since environment variables don't cross the WSL interop boundary.
    #[arg(long = "log-filter", value_parser = parse_log_filter)]
    pub log_filter: String,
}

fn parse_log_filter(value: &str) -> Result<String, String> {
    tracing_subscriber::EnvFilter::try_new(value)
        .map(|_| value.to_string())
        .map_err(|e| e.to_string())
}

/// Logs to stderr, colored only on a terminal. Off a terminal, stderr is the client's pipe and
/// the journal stamps every line, so the server's own timestamp is left out.
pub fn init(args: &LogArgs) {
    use std::io::IsTerminal;
    let filter = tracing_subscriber::EnvFilter::try_new(&args.log_filter)
        .expect("--log-filter is validated while parsing");
    let on_terminal = std::io::stderr().is_terminal();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(on_terminal)
        .with_env_filter(filter);
    if on_terminal {
        subscriber.init();
    } else {
        subscriber.without_time().init();
    }
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
        assert_eq!(config.log.log_filter, "debug,yamux=warn");
    }

    #[test]
    fn refuses_an_invalid_filter() {
        let args = ["relay-server", crate::LOG_FILTER_FLAG, "relay=loud"];
        assert!(Config::try_parse_from(args).is_err());
    }
}
