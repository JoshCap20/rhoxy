use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

/// Spawn a proxy using the same `handle_connection` as production.
/// Accepts connections in a loop until the listener is dropped.
#[allow(dead_code)]
pub async fn start_proxy() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let (reader, writer) = stream.into_split();
                let mut reader = BufReader::new(reader);
                let mut writer = BufWriter::new(writer);

                let _ = rhoxy::handle_connection(&mut writer, &mut reader, None).await;
            });
        }
    });

    addr
}

/// Like `start_proxy` but wraps each connection in a timeout.
/// Mirrors `main.rs` timeout behavior.
#[allow(dead_code)]
pub async fn start_proxy_with_timeout(timeout: Duration) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let (reader, writer) = stream.into_split();
                let mut reader = BufReader::new(reader);
                let mut writer = BufWriter::new(writer);

                let _ = tokio::time::timeout(
                    timeout,
                    rhoxy::handle_connection(&mut writer, &mut reader, None),
                )
                .await;
            });
        }
    });

    addr
}

/// Like `start_proxy` but limits concurrent connections via a semaphore.
/// Mirrors `main.rs` connection limiting behavior.
#[allow(dead_code)]
pub async fn start_proxy_with_limit(max_connections: usize) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let semaphore = Arc::new(Semaphore::new(max_connections));

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };

            let permit = match semaphore.clone().try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    drop(stream);
                    continue;
                }
            };

            tokio::spawn(async move {
                let _permit = permit;
                let (reader, writer) = stream.into_split();
                let mut reader = BufReader::new(reader);
                let mut writer = BufWriter::new(writer);

                let _ = rhoxy::handle_connection(&mut writer, &mut reader, None).await;
            });
        }
    });

    addr
}

/// Like `start_upstream` but accepts connections in a loop (not just one).
/// Used by concurrent-requests tests.
#[allow(dead_code)]
pub async fn start_looping_upstream(response: &'static [u8]) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let _ = stream.read(&mut buf).await;
                let _ = stream.write_all(response).await;
                let _ = stream.shutdown().await;
            });
        }
    });

    addr
}

/// Spawn a simple upstream HTTP server that accepts one connection,
/// reads whatever is sent, and responds with the given canned response.
#[allow(dead_code)]
pub async fn start_upstream(response: &'static [u8]) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 8192];
        let _ = stream.read(&mut buf).await;
        stream.write_all(response).await.unwrap();
        stream.shutdown().await.unwrap();
    });

    addr
}

/// Send raw bytes to a TCP address, shut down the write half, and read
/// the full response.
pub async fn send_raw(addr: std::net::SocketAddr, request: &[u8]) -> String {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(request).await.unwrap();
    stream.shutdown().await.unwrap();

    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut response))
        .await
        .expect("Timed out reading response")
        .expect("Failed to read response");

    String::from_utf8_lossy(&response).into_owned()
}
