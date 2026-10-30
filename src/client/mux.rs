use std::future::{Future, poll_fn};
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::mux::{self as shared, MuxError};

/// A request to open a new outbound yamux stream, answered by the driver loop.
struct OpenRequest {
    reply: tokio::sync::oneshot::Sender<Result<yamux::Stream, MuxError>>,
}

/// Opens outbound streams over the one connection; clone it for every task that needs one. The
/// connection itself belongs to the driver [`client`] returns with it.
#[derive(Clone)]
pub struct MuxHandle {
    open_tx: tokio::sync::mpsc::Sender<OpenRequest>,
}

impl MuxHandle {
    /// Opens a new stream once the driver has created it, which yamux holds back while too many
    /// streams wait for the server's acknowledgement.
    ///
    /// # Errors
    ///
    /// [`MuxError::Closed`] once the driver has ended, or the connection's own error.
    pub async fn open_stream(&self) -> Result<Stream, MuxError> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.open_tx
            .send(OpenRequest { reply: reply_tx })
            .await
            .map_err(|_| MuxError::Closed)?;
        let inner = reply_rx.await.map_err(|_| MuxError::Closed)??;
        Ok(Stream {
            inner,
            sent: false,
            closed_unsent: false,
        })
    }
}

/// One outbound stream: a `yamux::Stream`, except that one closed before anything was written to it
/// ends at once on this side.
///
/// yamux 0.14 opens a stream on the wire with its first data frame, and closing a stream that never
/// sent one doesn't open it either: the server never hears of it, and a read waiting for the
/// server's answer would wait forever. Such a stream has nothing to answer, so it reads as ended.
pub struct Stream {
    inner: yamux::Stream,
    sent: bool,
    closed_unsent: bool,
}

impl futures::AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        if self.closed_unsent {
            return Poll::Ready(Ok(0));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl futures::AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let written = Pin::new(&mut self.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = written
            && n > 0
        {
            self.sent = true;
        }
        written
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if !self.sent {
            self.closed_unsent = true;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_close(cx)
    }
}

/// Builds the client end of a mux over `io`, the server's joined stdout and stdin.
///
/// Returns the handle that opens streams and the driver, which must be polled to completion (see
/// [`shared::connection`]); every handle dropped closes the connection.
pub fn client<T>(io: T) -> (MuxHandle, impl Future<Output = Result<(), MuxError>>)
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let connection = shared::connection(io, yamux::Mode::Client);
    let (open_tx, open_rx) = tokio::sync::mpsc::channel(32);
    let mut driver = Driver {
        connection,
        open_rx,
        pending_reply: None,
    };
    (
        MuxHandle { open_tx },
        poll_fn(move |cx| driver.poll_drive(cx)),
    )
}

struct Driver<T> {
    connection: yamux::Connection<T>,
    open_rx: tokio::sync::mpsc::Receiver<OpenRequest>,
    pending_reply: Option<tokio::sync::oneshot::Sender<Result<yamux::Stream, MuxError>>>,
}

impl<T> Driver<T>
where
    T: futures::AsyncRead + futures::AsyncWrite + Unpin,
{
    fn poll_drive(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), MuxError>> {
        if self.poll_open_requests(cx).is_ready() {
            return self.connection.poll_close(cx).map_err(MuxError::Connection);
        }
        self.poll_inbound(cx)
    }

    /// Opens a stream for every request as far as yamux allows; ready once every handle is gone
    /// and nothing can ask for another.
    fn poll_open_requests(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        loop {
            if let Some(reply) = self.pending_reply.take() {
                match self.connection.poll_new_outbound(cx) {
                    Poll::Ready(stream) => {
                        let _ = reply.send(stream.map_err(MuxError::Connection));
                    }
                    Poll::Pending => {
                        self.pending_reply = Some(reply);
                        return Poll::Pending;
                    }
                }
            } else {
                match self.open_rx.poll_recv(cx) {
                    Poll::Ready(Some(request)) => self.pending_reply = Some(request.reply),
                    Poll::Ready(None) => return Poll::Ready(()),
                    Poll::Pending => return Poll::Pending,
                }
            }
        }
    }

    /// Moves frames both ways until the connection ends; that is also what sends the frames
    /// queued for outbound streams.
    fn poll_inbound(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), MuxError>> {
        loop {
            match self.connection.poll_next_inbound(cx) {
                Poll::Ready(Some(Ok(_stream))) => {
                    tracing::warn!("dropping a stream the server opened; servers don't open any");
                }
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Err(MuxError::Connection(e))),
                Poll::Ready(None) => return Poll::Ready(Err(MuxError::Closed)),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::io::{AsyncReadExt, AsyncWriteExt};

    /// Drives a bare `Mode::Server` yamux connection in the background, echoing back whatever
    /// bytes it reads on each inbound stream, so this test exercises a real two-sided exchange
    /// without depending on `server::mux`.
    fn spawn_echo_server<T>(io: T) -> tokio::task::JoinHandle<()>
    where
        T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        tokio::spawn(async move {
            let mut connection = shared::connection(io, yamux::Mode::Server);
            loop {
                let next = std::future::poll_fn(|cx| connection.poll_next_inbound(cx)).await;
                match next {
                    Some(Ok(mut stream)) => {
                        tokio::spawn(async move {
                            let mut buf = Vec::new();
                            if stream.read_to_end(&mut buf).await.is_ok() {
                                let _ = stream.write_all(&buf).await;
                                let _ = stream.close().await;
                            }
                        });
                    }
                    Some(Err(_)) | None => break,
                }
            }
        })
    }

    #[tokio::test]
    async fn open_stream_relays_data_end_to_end() {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let (handle, driver) = client(client_io);
        tokio::spawn(driver);
        spawn_echo_server(server_io);

        let mut stream = handle.open_stream().await.unwrap();
        stream.write_all(b"hello, agent").await.unwrap();
        stream.close().await.unwrap();

        let mut echoed = Vec::new();
        stream.read_to_end(&mut echoed).await.unwrap();
        assert_eq!(echoed, b"hello, agent");
    }

    #[tokio::test]
    async fn multiple_concurrent_streams_are_independent() {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let (handle, driver) = client(client_io);
        tokio::spawn(driver);
        spawn_echo_server(server_io);

        let mut tasks = Vec::new();
        for i in 0..8u32 {
            let handle = handle.clone();
            tasks.push(tokio::spawn(async move {
                let payload = format!("stream-{i}").into_bytes();
                let mut stream = handle.open_stream().await.unwrap();
                stream.write_all(&payload).await.unwrap();
                stream.close().await.unwrap();

                let mut echoed = Vec::new();
                stream.read_to_end(&mut echoed).await.unwrap();
                assert_eq!(echoed, payload);
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn driver_shuts_down_once_all_handles_are_dropped() {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let (handle, driver) = client(client_io);
        spawn_echo_server(server_io);

        drop(handle);

        tokio::time::timeout(std::time::Duration::from_secs(5), driver)
            .await
            .expect("driver did not shut down after handles were dropped")
            .unwrap();
    }
}
