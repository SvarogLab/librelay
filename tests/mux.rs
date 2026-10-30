//! The client's and the server's halves of the mux, paired as a relay pairs them.
#![cfg(all(feature = "client", feature = "server", unix))]

use futures::io::{AsyncReadExt, AsyncWriteExt};
use librelay::{client, server};

#[tokio::test]
async fn a_stream_the_client_opens_reaches_the_server_and_back() {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let (handle, client_driver) = client::mux::client(client_io);
    let (mut acceptor, server_driver) = server::mux::server(server_io);
    tokio::spawn(client_driver);
    tokio::spawn(server_driver);

    tokio::spawn(async move {
        while let Some(mut stream) = acceptor.accept().await {
            tokio::spawn(async move {
                let mut request = Vec::new();
                stream.read_to_end(&mut request).await.unwrap();
                request.reverse();
                stream.write_all(&request).await.unwrap();
                stream.close().await.unwrap();
            });
        }
    });

    let mut stream = handle.open_stream().await.unwrap();
    stream.write_all(b"request").await.unwrap();
    stream.close().await.unwrap();
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await.unwrap();
    assert_eq!(answer, b"tseuqer");
}

#[tokio::test]
async fn the_server_driver_ends_cleanly_when_the_client_closes_the_connection() {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let (handle, client_driver) = client::mux::client(client_io);
    let (_acceptor, server_driver) = server::mux::server(server_io);
    let server_driver = tokio::spawn(server_driver);

    drop(handle);
    client_driver.await.unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(5), server_driver)
        .await
        .expect("the server driver kept running after the client closed the connection")
        .unwrap()
        .expect("a connection the client closed is a normal end, not an error");
}

#[tokio::test]
async fn serve_runs_the_handler_for_each_stream_and_returns_once_the_client_is_gone() {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let (handle, client_driver) = client::mux::client(client_io);
    let client_driver = tokio::spawn(client_driver);
    let served = tokio::spawn(server::mux::serve(server_io, |mut stream| async move {
        let mut request = Vec::new();
        stream.read_to_end(&mut request).await.unwrap();
        request.make_ascii_uppercase();
        stream.write_all(&request).await.unwrap();
        stream.close().await.unwrap();
    }));

    for word in ["one", "two"] {
        let mut stream = handle.open_stream().await.unwrap();
        stream.write_all(word.as_bytes()).await.unwrap();
        stream.close().await.unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).await.unwrap();
        assert_eq!(answer, word.to_uppercase());
    }

    drop(handle);
    client_driver.await.unwrap().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), served)
        .await
        .expect("serve() kept running after the client went away")
        .unwrap()
        .unwrap();
}

/// A client that dies without closing the connection — killed, or its whole side gone — leaves the
/// server's input at end of file mid-connection. The server has to notice that and stop too, or
/// it lingers on Windows as an orphan.
#[tokio::test]
async fn serve_returns_when_the_client_vanishes_without_closing() {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let (handle, client_driver) = client::mux::client(client_io);
    let client_driver = tokio::spawn(client_driver);
    let (stalled_tx, stalled_rx) = tokio::sync::oneshot::channel::<()>();
    let stalled_rx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(stalled_rx)));
    let served = tokio::spawn(server::mux::serve(server_io, move |_stream| {
        let stalled_rx = stalled_rx.clone();
        async move {
            // A handler still busy when the client vanishes must not keep the server up.
            let rx = stalled_rx.lock().await.take().unwrap();
            let _ = rx.await;
        }
    }));

    let mut stream = handle.open_stream().await.unwrap();
    stream.write_all(b"work").await.unwrap();
    tokio::task::yield_now().await;
    client_driver.abort();

    tokio::time::timeout(std::time::Duration::from_secs(5), served)
        .await
        .expect("serve() kept running after the client vanished")
        .unwrap()
        .ok();
    drop(stalled_tx);
}

/// A connection that closes without sending anything — a health check's connect-and-close, say —
/// still has to end on both sides, or each one leaves a task and a stream behind for good.
#[tokio::test]
async fn a_stream_closed_before_any_data_still_ends() {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let (handle, client_driver) = client::mux::client(client_io);
    tokio::spawn(client_driver);
    tokio::spawn(server::mux::serve(server_io, |mut stream| async move {
        let mut request = Vec::new();
        let _ = stream.read_to_end(&mut request).await;
        let _ = stream.close().await;
    }));

    let mut stream = handle.open_stream().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        stream.close().await.unwrap();
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer).await.unwrap();
    })
    .await
    .expect("a stream closed before sending anything never ended");
}
