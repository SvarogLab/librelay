//! Watching the spawned server: a relay has nothing to do once its server is gone, so it exits
//! and leaves the restart to systemd.

use std::future::Future;
use std::process::ExitStatus;
use std::time::Duration;

use tokio::process::Child;

/// Resolves once the link to the server is gone: the server process exits, or `transport_ended`
/// — whatever the relay's own protocol uses to notice a dead link — resolves first.
pub async fn died(child: &mut Child, transport_ended: impl Future) {
    tokio::select! {
        _ = child.wait() => {}
        _ = transport_ended => {}
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
    async fn died_resolves_when_the_server_exits() {
        let mut child = spawn("exit 0");
        tokio::time::timeout(
            Duration::from_secs(5),
            died(&mut child, std::future::pending::<()>()),
        )
        .await
        .expect("died() did not resolve after the server exited");
    }

    #[tokio::test]
    async fn died_resolves_when_the_transport_ends() {
        let mut child = spawn("sleep 30");
        tokio::time::timeout(Duration::from_secs(5), died(&mut child, async {}))
            .await
            .expect("died() did not resolve after the transport ended");
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
