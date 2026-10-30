//! The client's logging: one filter, read from `RUST_LOG`, for the client and the server alike,
//! and the server's stderr forwarded into the client's.

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tracing_subscriber::EnvFilter;

/// The `tracing` filter both processes log with: `RUST_LOG` when set and non-empty, `info`
/// otherwise. Validated once, here, so a typo is reported by the process that read it rather
/// than making the server exit at startup.
#[derive(Debug)]
pub struct LogFilter {
    directives: String,
    env_filter: EnvFilter,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid RUST_LOG: {0}")]
pub struct LogFilterError(#[from] tracing_subscriber::filter::ParseError);

impl LogFilter {
    pub fn from_env() -> Result<Self, LogFilterError> {
        Self::parse(std::env::var("RUST_LOG").ok())
    }

    /// `rust_log` is `RUST_LOG`'s value, if it has one.
    pub fn parse(rust_log: Option<String>) -> Result<Self, LogFilterError> {
        let directives = rust_log
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "info".to_string());
        let env_filter = EnvFilter::try_new(&directives)?;
        Ok(Self {
            directives,
            env_filter,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.directives
    }

    /// The arguments that pass this filter to the server, for its `server::log::LogArgs`.
    pub fn server_args(&self) -> [String; 2] {
        [crate::LOG_FILTER_FLAG.to_string(), self.directives.clone()]
    }
}

/// Logs to stderr — the journal, under systemd — colored only on a terminal.
pub fn init(filter: LogFilter) {
    use std::io::IsTerminal;
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_env_filter(filter.env_filter)
        .init();
}

/// Marks every line the server writes to its stderr as the server's, so both processes'
/// diagnostics can share the journal and still be told apart.
pub const SERVER_LINE_PREFIX: &str = "server: ";

/// Copies the server's stderr into `out` line by line, each line prefixed with
/// [`SERVER_LINE_PREFIX`], until the server closes it. The line itself is passed through as the
/// server wrote it — its own `tracing` setup already formatted it — apart from a trailing `\r`,
/// which Windows processes may end lines with.
pub async fn forward_server_stderr(
    server_stderr: impl AsyncRead + Unpin,
    mut out: impl AsyncWrite + Unpin,
) {
    let mut server_stderr = BufReader::new(server_stderr);
    let mut raw = Vec::new();
    loop {
        raw.clear();
        match server_stderr.read_until(b'\n', &mut raw).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                let line = String::from_utf8_lossy(&raw);
                let line = line.strip_suffix('\n').unwrap_or(&line);
                let line = line.strip_suffix('\r').unwrap_or(line);
                // One write per line, flushed, so a line reaches the journal whole and can't
                // interleave with the client's own output.
                let _ = out
                    .write_all(format!("{SERVER_LINE_PREFIX}{line}\n").as_bytes())
                    .await;
                let _ = out.flush().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_rust_log_as_it_is() {
        let filter = LogFilter::parse(Some("debug,yamux=warn".to_string())).unwrap();
        assert_eq!(filter.as_str(), "debug,yamux=warn");
    }

    #[test]
    fn falls_back_to_info_when_rust_log_is_unset_or_empty() {
        assert_eq!(LogFilter::parse(None).unwrap().as_str(), "info");
        assert_eq!(
            LogFilter::parse(Some(String::new())).unwrap().as_str(),
            "info"
        );
    }

    #[test]
    fn refuses_an_invalid_rust_log() {
        let error = LogFilter::parse(Some("relay=loud".to_string())).unwrap_err();
        assert!(
            error.to_string().starts_with("invalid RUST_LOG: "),
            "{error}"
        );
    }

    #[test]
    fn passes_the_filter_to_the_server_under_its_flag() {
        let filter = LogFilter::parse(Some("debug".to_string())).unwrap();
        assert_eq!(filter.server_args(), [crate::LOG_FILTER_FLAG, "debug"]);
    }

    #[tokio::test]
    async fn prefixes_each_server_line() {
        let mut out = Vec::new();
        forward_server_stderr(&b"first\r\nsecond\n\n\xffthird"[..], &mut out).await;
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "server: first\nserver: second\nserver: \nserver: \u{fffd}third\n"
        );
    }
}
