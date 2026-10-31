use std::future::{Future, poll_fn};
use std::task::{Context, Poll};

use crate::mux::{self as shared, MuxError};

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

/// Builds the server end of a mux over `io`, the process's joined stdin and stdout.
///
/// Returns the acceptor and the driver, which must be polled to completion (see
/// [`shared::connection`]); the driver's end is how the server learns the client is gone.
pub fn server<T>(io: T) -> (MuxAcceptor, impl Future<Output = Result<(), MuxError>>)
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let connection = shared::connection(io, yamux::Mode::Server);
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

/// Serves the client's streams over `io`, the server's joined stdin and stdout, running `handler`
/// on its own task for each, until the client goes away.
///
/// The server is meant to exit once this returns, which also releases anything handlers still
/// hold open.
///
/// # Errors
///
/// The connection's error if the client spoke the protocol wrongly; a client that closed the
/// connection or vanished ends it with `Ok`.
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
    fn poll_drive(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), MuxError>> {
        // An unbounded sender can't wake this driver when the acceptor goes, so a dropped acceptor
        // is noticed at the next poll; the caller watches the driver's own end, not this.
        if self.closing || self.accept_tx.is_closed() {
            return self.poll_close(cx);
        }
        loop {
            match self.connection.poll_next_inbound(cx) {
                Poll::Ready(Some(Ok(stream))) => {
                    if self.accept_tx.send(stream).is_err() {
                        return self.poll_close(cx);
                    }
                }
                Poll::Ready(Some(Err(e))) if client_is_gone(&e) => return Poll::Ready(Ok(())),
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Err(MuxError::Connection(e))),
                // The client closed the connection: the normal end of a server's life.
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Pending => return Poll::Pending,
            }
        }
    }

    /// With no acceptor left there is no one to hand streams to, so the connection winds down.
    fn poll_close(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), MuxError>> {
        self.closing = true;
        self.connection.poll_close(cx).map_err(MuxError::Connection)
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

    /// A minimal client end, so these tests don't depend on `client::mux`, which only builds on
    /// unix.
    ///
    /// `poll_new_outbound` only books a stream; frames move only while `poll_next_inbound` is
    /// polled. Both need `&mut Connection`, so one `poll_fn` interleaves them, as the client's
    /// driver does.
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
        let mut connection = shared::connection(io, yamux::Mode::Client);
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
                        Poll::Ready(Some(_)) => {}
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
        tokio::spawn(driver);
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
        tokio::spawn(driver);
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
