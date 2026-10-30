# librelay

`librelay` holds what WSL relays share. A relay of this kind is a Linux client that spawns one
Windows server through WSL interop, exactly once, and talks to it over the server's stdin and
stdout, so a Linux application can reach something that only exists on Windows: a named pipe, the
Credential Manager. [`sockrelay`](https://github.com/SvarogLab/sockrelay) and
[`credrelay`](https://github.com/SvarogLab/credrelay) are built on it.

Spawning the server once matters: starting a Windows process per request from WSL runs into a WSL
interop bug under load ([microsoft/WSL #7371](https://github.com/microsoft/WSL/issues/7371)).

## Architecture

One crate, two sides, each behind the feature of the same name (both on by default):

- **`client`** (Linux; compiles to nothing elsewhere):
  - `spawn::Server::spawn` starts the server with all three standard streams piped, on the blocking
    pool, since the spawn itself can hang under the interop bug above. The server is killed when
    the `Server` is dropped.
  - `mux::client` runs the client end of the yamux connection over the server's stdio; its
    `MuxHandle` opens one stream per unit of work.
  - `lifecycle::died` resolves once the server exits or the mux driver ends, and says which; the
    relay then exits and systemd restarts it. `lifecycle::wait_alive` is an optional startup check
    that fails if the server exits within a grace period.
  - `log::LogFilter` reads `RUST_LOG`; `log::init` sets up the client's logging and
    `log::forward_server_stderr` forwards the server's (see [Logging](#logging)).
  - `systemd::take_listen_fds` and `systemd::listener_from_fd` take a socket-activated socket;
    `systemd::notify_ready` reports readiness to a `Type=notify` service.
- **`server`** (Windows):
  - `mux::server` runs the server end of the yamux connection over its own stdin and stdout; its
    `MuxAcceptor` hands out each stream the client opens. The driver ends once the client is gone,
    which is the server's cue to exit.
  - `log::LogArgs`, the server's required `--log-filter` flag, flattened into its own `clap`
    arguments, and `log::init`, its logging.

What goes over a stream stays with each relay: `sockrelay` relays one connection's bytes to the
ssh-agent pipe, `credrelay` sends one request line and reads one answer line.

## Usage

Add it as a git dependency pinned to a release tag, with the side each binary needs:

```toml
[dependencies]
librelay = { git = "https://github.com/SvarogLab/librelay", tag = "v0.1.0", default-features = false, features = ["client"] }
```

A client, in outline:

```rust
use librelay::client::{lifecycle, log, mux, spawn::Server, systemd};

// Before the runtime starts: the environment may only change while there is one thread.
let fds = unsafe { systemd::take_listen_fds() }?;
let filter = log::LogFilter::from_env()?;
let server_args = filter.server_args();
log::init(filter);

// Inside the runtime:
let mut server = Server::spawn("relay-server.exe".into(), server_args).await?;
tokio::spawn(log::forward_server_stderr(server.stderr, tokio::io::stderr()));
lifecycle::wait_alive(&mut server.child, Duration::from_millis(500)).await?;
systemd::notify_ready()?;
let (handle, driver) = mux::client(tokio::io::join(server.stdout, server.stdin));
let mut driver = tokio::spawn(driver);
// ... open a stream with handle.open_stream() per unit of work, and exit once
// lifecycle::died(&mut server.child, &mut driver) resolves.
```

A server flattens `LogArgs` into its own arguments, logs from it, and serves streams until the
client goes away:

```rust
use librelay::server::{log, mux};

#[derive(clap::Parser)]
struct Config {
    #[command(flatten)]
    log: log::LogArgs,
}

let config = Config::parse();
log::init(&config.log);

let (mut acceptor, driver) = mux::server(tokio::io::join(tokio::io::stdin(), tokio::io::stdout()));
tokio::spawn(async move {
    while let Some(stream) = acceptor.accept().await {
        tokio::spawn(serve(stream));
    }
});
let _ = driver.await;
// The client is gone: exit, which also closes whatever the server held open.
```

## Logging

Both processes log to the client's stderr, which systemd sends to the journal:

- The filter is `RUST_LOG`, `info` when it's unset or empty. The client validates it, and an
  invalid one is an error naming `RUST_LOG` before anything starts.
- Environment variables don't cross the WSL interop boundary, so the client passes the same filter
  to the server as `--log-filter`, a required server flag.
- Every line the server writes to its stderr is forwarded with a `server:` prefix followed by a
  space, and otherwise as the server wrote it, apart from a trailing carriage return.
- The server leaves out its own timestamp when its stderr isn't a terminal — which it never is when
  the client spawns it — since the journal timestamps every line.
- Output is colored only when stderr is a terminal.

## Testing

```sh
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

The server side also builds and tests for Windows, cross-compiled with
[`cargo-xwin`](https://github.com/rust-cross/cargo-xwin) for the host's own architecture; WSL runs
the test binary natively:

```sh
eval "$(cargo xwin env --target aarch64-pc-windows-msvc)"
unset CARGO_TARGET_AARCH64_PC_WINDOWS_MSVC_RUNNER
cargo test --target aarch64-pc-windows-msvc --no-default-features --features server
cargo clippy --all-targets --all-features --target aarch64-pc-windows-msvc -- -D warnings
```

(`cargo xwin env` sets the target's runner to `wine`, which isn't needed here, so it is unset.)

### Releasing

Run the **Release PR** workflow from the Actions tab. It opens `chore: release vX.Y.Z` with the
version git-cliff computes from the commits since the last tag and a regenerated `CHANGELOG.md`;
merging it tags the commit and publishes the release, which the relays then pin.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT)
at your option.
