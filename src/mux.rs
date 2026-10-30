//! The yamux connection the client and the server drive over the server's stdio.
//!
//! One stream per unit of work, so any number of them share the one server process.
//! [`crate::client::mux`] opens streams and [`crate::server::mux`] accepts them.

use tokio_util::compat::{Compat, TokioAsyncReadCompatExt};

#[derive(Debug, thiserror::Error)]
pub enum MuxError {
    #[error("yamux connection error: {0}")]
    Connection(#[from] yamux::ConnectionError),
    #[error("mux driver has shut down")]
    Closed,
}

/// yamux 0.14 runs no task of its own: a connection moves frames only while it is polled, so each
/// side wraps it in a driver future that must be polled to completion.
pub(crate) fn connection<T>(io: T, mode: yamux::Mode) -> yamux::Connection<Compat<T>>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    yamux::Connection::new(io.compat(), yamux::Config::default(), mode)
}
