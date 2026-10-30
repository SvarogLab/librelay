//! Watching the spawned server: a relay has nothing to do once its server is gone, so it exits
//! and leaves the restart to systemd.

use std::future::Future;
use std::process::ExitStatus;
use std::time::Duration;

use tokio::process::Child;

/// How the link to the server was lost, for the relay to log before it exits.
#[derive(Debug)]
pub enum Died {
    /// The server process exited, with this status if it could be read.
    Exited(std::io::Result<ExitStatus>),
    /// The relay's transport reported the link gone while the server was still running.
    TransportEnded,
}

impl std::fmt::Display for Died {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Died::Exited(Ok(status)) => write!(f, "the server exited ({status})"),
            Died::Exited(Err(e)) => {
                write!(
                    f,
                    "the server is gone; its exit status couldn't be read: {e}"
                )
            }
            Died::TransportEnded => write!(f, "the mux connection to the server ended"),
        }
    }
}

/// Resolves once the link to the server is gone: the server process exits, or `transport_ended`
/// — whatever the relay's own protocol uses to notice a dead link — resolves first.
pub async fn died(child: &mut Child, transport_ended: impl Future) -> Died {
    tokio::select! {
        status = child.wait() => Died::Exited(status),
        _ = transport_ended => Died::TransportEnded,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NotAlive {
    #[error("the server exited within {within:?} of starting ({status})")]
    Exited {
        within: Duration,
        status: ExitStatus,
    },
    #[error("couldn't wait for the server: {0}")]
    Wait(#[from] std::io::Error),
}

/// Resolves on SIGTERM — what `systemctl stop` sends — or SIGINT, for the relay to shut down
/// gracefully instead of being killed mid-request.
pub async fn shutdown_signal() -> std::io::Result<()> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    tokio::select! {
        _ = terminate.recv() => {}
        _ = interrupt.recv() => {}
    }
    Ok(())
}

/// Waits out `within` and fails if the server exits during it. There's no positive health signal
/// to wait for, so this always takes the whole of `within` on success: it catches a server that
/// exits right away on a startup check, and is a tax on every start otherwise.
pub async fn wait_alive(child: &mut Child, within: Duration) -> Result<(), NotAlive> {
    match tokio::time::timeout(within, child.wait()).await {
        Err(_elapsed) => Ok(()),
        Ok(status) => Err(NotAlive::Exited {
            within,
            status: status?,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::process::Command;

    fn spawn(script: &str) -> Child {
        Command::new("sh")
            .args(["-c", script])
            .kill_on_drop(true)
            .spawn()
            .unwrap()
    }

    #[tokio::test]
    async fn died_reports_the_servers_exit_status() {
        let mut child = spawn("exit 3");
        let died = tokio::time::timeout(
            Duration::from_secs(5),
            died(&mut child, std::future::pending::<()>()),
        )
        .await
        .expect("died() did not resolve after the server exited");
        match died {
            Died::Exited(status) => assert_eq!(status.unwrap().code(), Some(3)),
            Died::TransportEnded => panic!("the server exited, the transport didn't end"),
        }
    }

    #[tokio::test]
    async fn died_reports_the_transport_ending() {
        let mut child = spawn("sleep 30");
        let died = tokio::time::timeout(Duration::from_secs(5), died(&mut child, async {}))
            .await
            .expect("died() did not resolve after the transport ended");
        assert!(matches!(died, Died::TransportEnded));
    }

    #[tokio::test]
    async fn died_says_how_the_link_was_lost() {
        let mut child = spawn("exit 3");
        let exited = died(&mut child, std::future::pending::<()>()).await;
        assert_eq!(exited.to_string(), "the server exited (exit status: 3)");
        assert_eq!(
            Died::TransportEnded.to_string(),
            "the mux connection to the server ended"
        );
        let unreadable = Died::Exited(Err(std::io::Error::other("no such process")));
        assert_eq!(
            unreadable.to_string(),
            "the server is gone; its exit status couldn't be read: no such process"
        );
    }

    #[tokio::test]
    async fn a_server_that_stays_up_is_alive() {
        let mut child = spawn("sleep 30");
        wait_alive(&mut child, Duration::from_millis(100))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_server_that_exits_at_once_is_not() {
        let mut child = spawn("exit 3");
        assert!(matches!(
            wait_alive(&mut child, Duration::from_secs(5)).await,
            Err(NotAlive::Exited { .. })
        ));
    }
}
