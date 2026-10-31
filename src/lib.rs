//! The pieces a WSL relay shares: a Linux client that spawns one Windows server over WSL interop,
//! once, and talks to it over the server's stdin and stdout.
//!
//! [`client`] is the Linux side and [`server`] the Windows side, each behind the feature of the
//! same name. They talk over a yamux connection ([`mux`]), one stream per unit of work; what goes
//! over a stream stays with each relay.

#[cfg(all(feature = "client", unix))]
pub mod client;
#[cfg(any(all(feature = "client", unix), feature = "server"))]
mod log;
#[cfg(feature = "mux")]
pub mod mux;
#[cfg(feature = "server")]
pub mod server;

/// The server flag carrying the client's log filter: `RUST_LOG` itself doesn't cross the WSL
/// interop boundary into a Windows process, so the client forwards its filter as an argument.
pub const LOG_FILTER_FLAG: &str = "--log-filter";
