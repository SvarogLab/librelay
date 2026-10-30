use std::future::{Future, poll_fn};
use std::task::{Context, Poll};

use tokio_util::compat::TokioAsyncReadCompatExt;

use crate::mux::MuxError;

/// Hands out inbound yamux streams as the client opens them, one per unit of work the relay
/// carries — a relayed connection, a request.
pub struct MuxAcceptor {
    accept_rx: tokio::sync::mpsc::UnboundedReceiver<yamux::Stream>,
}

impl MuxAcceptor {
    /// Waits for the next inbound stream. Returns `None` once the connection is gone and no more
    /// streams will ever arrive.
    pub async fn accept(&mut self) -> Option<yamux::Stream> {
        self.accept_rx.recv().await
    }
}

/// Builds a server-mode mux over `io` (the process's own joined stdin/stdout, or any other duplex
/// transport — see the tests below, which use `tokio::io::duplex`). Returns an acceptor plus the
/// driver future, which the caller must poll to completion (e.g. via `tokio::spawn`) for the
/// connection to make any progress — this is what actually reads/writes the underlying transport,
/// and its completion is how the server learns the client is gone.
pub fn server<T>(io: T) -> (MuxAcceptor, impl Future<Output = Result<(), MuxError>>)
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let connection =
        yamux::Connection::new(io.compat(), yamux::Config::default(), yamux::Mode::Server);
    // Unbounded: yamux itself already caps the number of live streams per connection (default
    // 512, see `yamux::Config`), so there's a natural upper bound on outstanding items without
    // needing bounded-channel backpressure logic here too.
    let (accept_tx, accept_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut driver = Driver {
        connection,
        accept_tx,
        closing: false,
    };
    (
        MuxAcceptor { accept_rx },
        poll_fn(move |cx| driver.poll_drive(cx)),
    )
}

/// Serves the client's streams over `io` — in a server, its joined stdin and stdout — running
/// `handler` on its own task for each, until the client goes away. Returns the driver's outcome,
/// for the server's exit status; the server then exits, which also releases anything handlers
/// still hold open.
pub async fn serve<T, F, Fut>(io: T, handler: F) -> Result<(), MuxError>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    F: Fn(yamux::Stream) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (mut acceptor, driver) = server(io);
    // Handlers aren't awaited: once the client is gone there is no one to answer, and the server
    // exiting is what releases whatever they still hold.
    tokio::spawn(async move {
        while let Some(stream) = acceptor.accept().await {
            tokio::spawn(handler(stream));
        }
    });
    driver.await
}

struct Driver<T> {
    connection: yamux::Connection<T>,
    accept_tx: tokio::sync::mpsc::UnboundedSender<yamux::Stream>,
    closing: bool,
}

impl<T> Driver<T>
where
    T: futures::AsyncRead + futures::AsyncWrite + Unpin,
{
    /// Drives the connection by continuously polling `poll_next_inbound` — as with the client
    /// side, this is not optional bookkeeping, it's the only thing that flushes/receives frames at
    /// all (yamux 0.14 has no separate background task or `Control`/`next_stream()` API — driving
    /// the connection *is* what makes progress). Every accepted stream is forwarded to the
    /// `MuxAcceptor`; once that's dropped, there's no one left to hand streams to, so the
    /// connection is wound down gracefully instead of accepting forever.
    fn poll_drive(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), MuxError>> {
        if self.closing {
            return self.connection.poll_close(cx).map_err(MuxError::Connection);
        }

        // Checked unconditionally, not just when a send fails: unlike `UnboundedReceiver`,
        // `UnboundedSender` has no waker-registering "wait until closed" primitive, so this only
        // detects a dropped acceptor at the next moment this future happens to get polled anyway
        // (e.g. an idle connection with no traffic at all won't notice a dropped acceptor until
        // *something* wakes it — acceptable here since this is best-effort local cleanup, not the
        // liveness detection that matters: that's the caller watching this driver's completion).
        if self.accept_tx.is_closed() {
            self.closing = true;
            return self.connection.poll_close(cx).map_err(MuxError::Connection);
        }

        loop {
            match self.connection.poll_next_inbound(cx) {
                Poll::Ready(Some(Ok(stream))) => {
                    if self.accept_tx.send(stream).is_err() {
                        self.closing = true;
                        return self.connection.poll_close(cx).map_err(MuxError::Connection);
                    }
                    continue;
                }
                Poll::Ready(Some(Err(e))) if client_is_gone(&e) => return Poll::Ready(Ok(())),
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Err(MuxError::Connection(e))),
                // The client closed the connection: the normal end of a server's life.
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// Whether `e` only says the transport is gone — the client closed it, or its whole side went
/// away — rather than that the client spoke wrongly. Either way the server's job is over, and an
/// answer to a closed connection fails with exactly these.
fn client_is_gone(e: &yamux::ConnectionError) -> bool {
    use std::io::ErrorKind::{BrokenPipe, ConnectionReset, UnexpectedEof};
    match e {
        yamux::ConnectionError::Closed => true,
        yamux::ConnectionError::Io(e) => {
            matches!(e.kind(), BrokenPipe | ConnectionReset | UnexpectedEof)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::io::{AsyncReadExt, AsyncWriteExt};
    use std::future::Future as StdFuture;
    use std::pin::Pin;

    fn spawn_driver(
        driver: impl StdFuture<Output = Result<(), MuxError>> + Send + 'static,
    ) -> tokio::task::JoinHandle<Result<(), MuxError>> {
        tokio::spawn(Box::pin(driver) as Pin<Box<dyn StdFuture<Output = _> + Send>>)
    }

    /// A minimal `Mode::Client` counterpart, so these tests don't depend on `client::mux`, which
    /// only builds on unix.
    ///
    /// Mirrors the shape of the real `Driver` in this file: `poll_new_outbound` alone only
    /// registers local bookkeeping for a new stream, it does **not** flush anything to the wire.
    /// Only `poll_next_inbound` actually sends/receives frames, so without a driving loop every
    /// write on the resulting stream hangs. Both `poll_new_outbound` and `poll_next_inbound` need `&mut Connection`, so — same as the real `Driver` — they're
    /// interleaved from a single task via one `poll_fn`, not two competing futures.
    struct BareClient {
        open_tx: tokio::sync::mpsc::UnboundedSender<tokio::sync::oneshot::Sender<yamux::Stream>>,
    }

    impl BareClient {
        async fn open_stream(&self) -> yamux::Stream {
            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            self.open_tx.send(reply_tx).unwrap();
            reply_rx.await.unwrap()
        }
    }

    fn spawn_bare_client<T>(io: T) -> BareClient
    where
        T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let mut connection =
            yamux::Connection::new(io.compat(), yamux::Config::default(), yamux::Mode::Client);
        let (open_tx, mut open_rx) =
            tokio::sync::mpsc::unbounded_channel::<tokio::sync::oneshot::Sender<yamux::Stream>>();

        tokio::spawn(async move {
            let mut pending: Option<tokio::sync::oneshot::Sender<yamux::Stream>> = None;
            poll_fn(|cx| {
                loop {
                    if let Some(reply) = pending.take() {
                        match connection.poll_new_outbound(cx) {
                            Poll::Ready(Ok(stream)) => {
                                let _ = reply.send(stream);
                                continue;
                            }
                            Poll::Ready(Err(_)) => return Poll::Ready(()),
                            Poll::Pending => pending = Some(reply),
                        }
                    } else {
                        match open_rx.poll_recv(cx) {
                            Poll::Ready(Some(reply)) => {
                                pending = Some(reply);
                                continue;
                            }
                            Poll::Ready(None) | Poll::Pending => {}
                        }
                    }
                    match connection.poll_next_inbound(cx) {
                        Poll::Ready(Some(_)) => continue,
                        Poll::Ready(None) => return Poll::Ready(()),
                        Poll::Pending => return Poll::Pending,
                    }
                }
            })
            .await;
        });

        BareClient { open_tx }
    }

    #[tokio::test]
    async fn accept_relays_a_stream_end_to_end_in_both_directions() {
        let (server_io, client_io) = tokio::io::duplex(64 * 1024);
        let (mut acceptor, driver) = server(server_io);
        spawn_driver(driver);
        let bare_client = spawn_bare_client(client_io);

        let mut client_stream = bare_client.open_stream().await;
        client_stream.write_all(b"open sesame").await.unwrap();

        let mut accepted = acceptor.accept().await.expect("stream was not accepted");

        let mut buf = vec![0u8; b"open sesame".len()];
        accepted.read_exact(&mut buf).await.unwrap();
        assert_eq!(buf, b"open sesame");

        accepted.write_all(b"granted").await.unwrap();
        let mut reply = vec![0u8; b"granted".len()];
        client_stream.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply, b"granted");
    }

    #[tokio::test]
    async fn accept_hands_out_independent_streams_one_at_a_time() {
        let (server_io, client_io) = tokio::io::duplex(64 * 1024);
        let (mut acceptor, driver) = server(server_io);
        spawn_driver(driver);
        let bare_client = spawn_bare_client(client_io);

        for i in 0..4u32 {
            let payload = format!("stream-{i}").into_bytes();
            let mut client_stream = bare_client.open_stream().await;
            client_stream.write_all(&payload).await.unwrap();

            let mut accepted = acceptor.accept().await.expect("stream was not accepted");
            let mut buf = vec![0u8; payload.len()];
            accepted.read_exact(&mut buf).await.unwrap();
            assert_eq!(buf, payload);
        }
    }

    #[tokio::test]
    async fn driver_shuts_down_once_the_acceptor_is_dropped() {
        let (server_io, _client_io) = tokio::io::duplex(64 * 1024);
        let (acceptor, driver) = server(server_io);

        drop(acceptor);

        tokio::time::timeout(std::time::Duration::from_secs(5), driver)
            .await
            .expect("driver did not shut down after the acceptor was dropped")
            .unwrap();
    }
}
