//! The client's logging: one filter, read from `RUST_LOG`, for the client and the server alike,
//! and the server's stderr forwarded into the client's.

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tracing_subscriber::EnvFilter;

/// The `tracing` filter both processes log with: `RUST_LOG` when set and non-empty, `info`
/// otherwise.
///
/// Validated once, here, so a typo is reported by the process that read it rather than making the
/// server exit at startup.
#[derive(Debug)]
pub struct LogFilter {
    directives: String,
    env_filter: EnvFilter,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid RUST_LOG: {0}")]
pub struct LogFilterError(#[from] tracing_subscriber::filter::ParseError);

impl LogFilter {
    /// Reads `RUST_LOG`.
    ///
    /// # Errors
    ///
    /// [`LogFilterError`] if `RUST_LOG` isn't a valid filter.
    pub fn from_env() -> Result<Self, LogFilterError> {
        Self::parse(std::env::var("RUST_LOG").ok())
    }

    /// `rust_log` is `RUST_LOG`'s value, if it has one.
    ///
    /// # Errors
    ///
    /// [`LogFilterError`] if `rust_log` isn't a valid filter.
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

/// Logs to stderr: colored and timestamped on a terminal, plain lines for the journal otherwise.
///
/// Consumes the filter, so take [`LogFilter::server_args`] first.
///
/// # Panics
///
/// If logging is already set up in this process.
pub fn init(filter: LogFilter) {
    crate::log::init_stderr(filter.env_filter);
}

/// Marks every line the server writes to its stderr as the server's, so both processes'
/// diagnostics can share the journal and still be told apart.
pub const SERVER_LINE_PREFIX: &str = "server: ";

/// Windows processes may end lines with `\r\n`; the journal wants `\n` alone.
fn server_line(raw: &[u8]) -> String {
    let line = String::from_utf8_lossy(raw);
    format!(
        "{SERVER_LINE_PREFIX}{}\n",
        line.trim_end_matches(['\r', '\n'])
    )
}

/// Copies the server's stderr into `out` line by line until the server closes it.
///
/// Each line gets [`SERVER_LINE_PREFIX`] and is otherwise passed through as the server wrote it:
/// its own `tracing` setup already formatted it.
pub async fn forward_server_stderr(
    server_stderr: impl AsyncRead + Unpin,
    mut out: impl AsyncWrite + Unpin,
) {
    let mut server_stderr = BufReader::new(server_stderr);
    let mut raw = Vec::new();
    loop {
        raw.clear();
        match server_stderr.read_until(b'\n', &mut raw).await {
            Ok(0) => return,
            Err(e) => {
                tracing::warn!(error = %e, "stopped forwarding the server's stderr");
                return;
            }
            Ok(_) => {
                // One write per line, flushed, so a line reaches the journal whole and can't
                // interleave with the client's own output. A failed write goes unreported: it
                // would be reported to the very stderr that failed.
                let _ = out.write_all(server_line(&raw).as_bytes()).await;
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

    #[test]
    fn a_server_line_gets_the_prefix_and_loses_its_line_ending() {
        assert_eq!(server_line(b"started\r\n"), "server: started\n");
        assert_eq!(server_line(b"started\n"), "server: started\n");
        assert_eq!(
            server_line(b"last, unterminated"),
            "server: last, unterminated\n"
        );
        assert_eq!(server_line(b"\xffbyte"), "server: \u{fffd}byte\n");
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
