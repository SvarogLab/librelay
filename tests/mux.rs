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
async fn the_server_driver_ends_when_the_client_goes_away() {
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
        .unwrap_err();
}
