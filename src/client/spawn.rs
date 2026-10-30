use std::io;
use std::path::PathBuf;
use std::process::Stdio;

use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};

#[derive(Debug, thiserror::Error)]
#[error("failed to start {path:?}: {source}")]
pub struct SpawnError {
    path: PathBuf,
    #[source]
    source: io::Error,
}

/// The one Windows server a client talks to, killed when dropped.
pub struct Server {
    pub child: Child,
    pub stdin: ChildStdin,
    pub stdout: ChildStdout,
    pub stderr: ChildStderr,
}

impl Server {
    /// Starts `path` with `args` and all three standard streams piped.
    ///
    /// The spawn runs on the blocking pool: starting a Windows process from WSL can hang inside
    /// the spawn call itself (microsoft/WSL#7371), and there it stalls one blocking thread rather
    /// than an async worker.
    pub async fn spawn(path: PathBuf, args: Vec<String>) -> Result<Self, SpawnError> {
        tokio::task::spawn_blocking(move || {
            let mut child = Command::new(&path)
                .args(&args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|source| SpawnError { path, source })?;
            Ok(Self {
                stdin: child.stdin.take().expect("stdin is piped"),
                stdout: child.stdout.take().expect("stdout is piped"),
                stderr: child.stderr.take().expect("stderr is piped"),
                child,
            })
        })
        .await
        .expect("the spawn task doesn't panic")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// `cat` stands in for the server: the same pipes, on any host.
    #[tokio::test]
    async fn the_child_reads_stdin_and_writes_stdout() {
        let mut server = Server::spawn("cat".into(), Vec::new()).await.unwrap();
        server.stdin.write_all(b"ping").await.unwrap();
        drop(server.stdin);
        let mut echoed = Vec::new();
        server.stdout.read_to_end(&mut echoed).await.unwrap();
        assert_eq!(echoed, b"ping");
        assert!(server.child.wait().await.unwrap().success());
    }

    #[tokio::test]
    async fn the_child_gets_its_arguments() {
        let args = vec![
            "-c".into(),
            "printf %s \"$0\"".into(),
            "--log-filter".into(),
        ];
        let mut server = Server::spawn("sh".into(), args).await.unwrap();
        let mut printed = Vec::new();
        server.stdout.read_to_end(&mut printed).await.unwrap();
        assert_eq!(printed, b"--log-filter");
    }

    #[tokio::test]
    async fn the_childs_stderr_is_separate_from_its_stdout() {
        let args = vec!["-c".into(), "echo out; echo err >&2".into()];
        let mut server = Server::spawn("sh".into(), args).await.unwrap();
        let mut err = String::new();
        server.stderr.read_to_string(&mut err).await.unwrap();
        assert_eq!(err, "err\n");
    }

    #[tokio::test]
    async fn a_missing_binary_fails_to_start() {
        let error = Server::spawn("/nonexistent/relay-server.exe".into(), Vec::new())
            .await
            .err()
            .expect("spawning a missing binary must fail");
        assert!(error.to_string().contains("relay-server.exe"), "{error}");
    }
}
