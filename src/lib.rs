//! The pieces a WSL relay shares: a Linux client that spawns one Windows server over WSL interop,
//! once, and talks to it over the server's stdin and stdout.
//!
//! [`client`] is the Linux side and [`server`] the Windows side, each behind the feature of the
//! same name. What the two say to each other — the transport — stays with each relay.

#[cfg(all(feature = "client", unix))]
pub mod client;
#[cfg(feature = "server")]
pub mod server;

/// The server flag carrying the client's log filter: `RUST_LOG` itself doesn't cross the WSL
/// interop boundary into a Windows process, so the client forwards its filter as an argument.
pub const LOG_FILTER_FLAG: &str = "--log-filter";
