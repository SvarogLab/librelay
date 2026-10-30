//! Logging both sides set up the same way.

use std::io::IsTerminal;

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt;

/// Logs to stderr. Off a terminal, stderr ends in the journal — the client's directly, the
/// server's through the client — and the journal stamps every line, so no timestamp is added.
///
/// Records of the `log` crate, which yamux logs through, are bridged in too.
pub(crate) fn init_stderr(filter: EnvFilter) {
    subscriber(filter, std::io::stderr().is_terminal(), std::io::stderr).init();
}

fn subscriber<W>(
    filter: EnvFilter,
    on_terminal: bool,
    writer: W,
) -> Box<dyn tracing::Subscriber + Send + Sync>
where
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    let builder = tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(on_terminal)
        .with_env_filter(filter);
    if on_terminal {
        Box::new(builder.finish())
    } else {
        Box::new(builder.without_time().finish())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().write(buf)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn logged(on_terminal: bool) -> String {
        let captured = Captured::default();
        let writer = captured.clone();
        let subscriber = subscriber(EnvFilter::new("info"), on_terminal, move || writer.clone());
        tracing::subscriber::with_default(subscriber, || tracing::info!("started"));
        String::from_utf8(captured.0.lock().unwrap().clone()).unwrap()
    }

    /// yamux logs through the `log` crate.
    #[test]
    fn records_of_the_log_crate_reach_the_subscriber() {
        init_stderr(EnvFilter::new("info"));
        assert_ne!(log::max_level(), log::LevelFilter::Off);
    }

    #[test]
    fn off_a_terminal_lines_carry_neither_timestamp_nor_color() {
        let line = logged(false);
        assert!(
            line.starts_with(" INFO") || line.starts_with("INFO"),
            "{line:?}"
        );
        assert!(!line.contains('\x1b'), "{line:?}");
        assert!(line.trim_end().ends_with("started"), "{line:?}");
    }

    #[test]
    fn on_a_terminal_lines_are_timestamped_and_colored() {
        let line = logged(true);
        let dimmed = line
            .strip_prefix("\x1b[2m")
            .expect("the line starts with a color code");
        assert!(dimmed.starts_with(|c: char| c.is_ascii_digit()), "{line:?}");
    }
}
