//! The yamux connection the client and the server drive over the server's stdio: one stream per
//! unit of work, so any number of them share the one server process. [`crate::client::mux`] opens
//! streams and [`crate::server::mux`] accepts them.

#[derive(Debug, thiserror::Error)]
pub enum MuxError {
    #[error("yamux connection error: {0}")]
    Connection(#[from] yamux::ConnectionError),
    #[error("mux driver has shut down")]
    Closed,
}
