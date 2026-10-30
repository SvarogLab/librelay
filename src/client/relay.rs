//! Bringing a relay's link to its server up and down: everything between "the client starts" and
//! "streams can be opened", and back.

use std::path::PathBuf;
use std::process::ExitStatus;
use std::time::Duration;

use tokio::process::Child;
use tokio::task::JoinHandle;

use crate::client::lifecycle::{self, Died, NotAlive};
use crate::client::log;
use crate::client::mux::{self, MuxHandle};
use crate::client::spawn::{Server, SpawnError};
use crate::mux::MuxError;

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error(transparent)]
    Spawn(#[from] SpawnError),
    #[error(transparent)]
    NotAlive(#[from] NotAlive),
}

#[derive(Debug, thiserror::Error)]
pub enum CloseError {
    #[error("the server didn't exit within {0:?} of the connection closing")]
    Timeout(Duration),
    #[error("couldn't wait for the server: {0}")]
    Wait(#[from] std::io::Error),
}

/// The running server and the client end of the mux over its stdio. Dropping it kills the server.
pub struct Link {
    child: Child,
    driver: JoinHandle<Result<(), MuxError>>,
}

/// Spawns the server, forwards its stderr to this process's, and runs the client end of the mux
/// over its stdio. With `grace`, also waits that long for the server to prove it stays up (see
/// [`lifecycle::wait_alive`]). Returns the handle that opens streams, and the [`Link`].
pub async fn start(
    server_exe: PathBuf,
    server_args: impl IntoIterator<Item = String>,
    grace: Option<Duration>,
) -> Result<(MuxHandle, Link), StartError> {
    let mut server = Server::spawn(server_exe, server_args).await?;
    tokio::spawn(log::forward_server_stderr(
        server.stderr,
        tokio::io::stderr(),
    ));
    let (handle, driver) = mux::client(tokio::io::join(server.stdout, server.stdin));
    let driver = tokio::spawn(driver);
    if let Some(grace) = grace {
        lifecycle::wait_alive(&mut server.child, grace).await?;
    }
    Ok((
        handle,
        Link {
            child: server.child,
            driver,
        },
    ))
}

impl Link {
    /// Resolves once the link is lost — the server exited or the mux ended — for the relay to log
    /// and exit on, leaving the restart to systemd. Cancel-safe, so it can sit in a `select!`.
    pub async fn died(&mut self) -> Died {
        let died = lifecycle::died(&mut self.child, &mut self.driver).await;
        if let Died::Exited(_) = died {
            // Dropping a `JoinHandle` only detaches its task; the driver has to be stopped
            // explicitly.
            self.driver.abort();
        }
        died
    }

    /// Shuts the link down gracefully. Once `handle` — the last one, after the relay has let its
    /// in-flight work finish — is dropped, the client closes the mux, the server sees the
    /// connection end and exits, and this waits up to `timeout` for its exit status.
    pub async fn close(
        mut self,
        handle: MuxHandle,
        timeout: Duration,
    ) -> Result<ExitStatus, CloseError> {
        drop(handle);
        let exited = async {
            // The driver's end drops the connection, and with it the server's stdin.
            let _ = (&mut self.driver).await;
            self.child.wait().await
        };
        match tokio::time::timeout(timeout, exited).await {
            Ok(status) => Ok(status?),
            Err(_elapsed) => Err(CloseError::Timeout(timeout)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> [String; 2] {
        ["-c".to_string(), script.to_string()]
    }

    #[tokio::test]
    async fn reports_a_server_that_cannot_start() {
        let result = start("/nonexistent/relay-server.exe".into(), [], None).await;
        assert!(matches!(result, Err(StartError::Spawn(_))));
    }

    #[tokio::test]
    async fn reports_a_server_that_exits_within_the_grace_period() {
        let result = start("sh".into(), sh("exit 3"), Some(Duration::from_secs(5))).await;
        assert!(matches!(result, Err(StartError::NotAlive(_))));
    }

    #[tokio::test]
    async fn the_link_is_lost_once_the_server_exits() {
        let (_handle, mut link) =
            start("sh".into(), sh("cat >/dev/null & sleep 0.2; exit 4"), None)
                .await
                .unwrap();
        let died = tokio::time::timeout(Duration::from_secs(5), link.died())
            .await
            .expect("the lost link was not reported");
        assert!(
            matches!(died, Died::Exited(Ok(status)) if status.code() == Some(4)),
            "{died}"
        );
    }

    /// `cat` stands in for a server that exits once its stdin ends, as a mux server does once the
    /// client closes the connection.
    #[tokio::test]
    async fn close_ends_the_connection_and_waits_for_the_server_to_exit() {
        let (handle, link) = start("sh".into(), sh("cat >/dev/null; exit 0"), None)
            .await
            .unwrap();
        let status = link.close(handle, Duration::from_secs(5)).await.unwrap();
        assert!(status.success());
    }

    #[tokio::test]
    async fn close_gives_up_on_a_server_that_doesnt_exit() {
        let (handle, link) = start("sh".into(), sh("exec sleep 30 </dev/null"), None)
            .await
            .unwrap();
        let result = link.close(handle, Duration::from_millis(300)).await;
        assert!(matches!(result, Err(CloseError::Timeout(_))));
    }
}
