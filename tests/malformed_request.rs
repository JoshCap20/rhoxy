use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Start the proxy server on a random port and return the address.
async fn start_proxy() -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        // Accept one connection and run it through the same path as main.rs
        let (stream, _) = listener.accept().await.unwrap();
        let (reader, writer) = stream.into_split();
        let mut reader = tokio::io::BufReader::new(reader);
        let mut writer = tokio::io::BufWriter::new(writer);

        match rhoxy::extract_request_parts(&mut reader).await {
            Ok(_) => panic!("Expected malformed request to fail parsing"),
            Err(_) => {
                let _ = writer
                    .write_all(rhoxy::constants::BAD_REQUEST_RESPONSE)
                    .await;
                let _ = writer.flush().await;
            }
        }
    });

    addr
}

#[tokio::test]
async fn test_malformed_request_returns_400() {
    let addr = start_proxy().await;

    let mut stream = TcpStream::connect(addr).await.unwrap();
    // Send garbage that isn't a valid HTTP request line
    stream.write_all(b"NOT-A-VALID-REQUEST\r\n").await.unwrap();
    stream.flush().await.unwrap();

    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut response))
        .await
        .expect("Timed out waiting for response")
        .expect("Failed to read response");

    let response_str = String::from_utf8_lossy(&response);
    assert!(
        response_str.contains("400 Bad Request"),
        "Expected 400 Bad Request, got: {}",
        response_str
    );
}

#[tokio::test]
async fn test_empty_request_returns_400() {
    let addr = start_proxy().await;

    let mut stream = TcpStream::connect(addr).await.unwrap();
    // Send just a bare CRLF — empty request line
    stream.write_all(b"\r\n").await.unwrap();
    stream.flush().await.unwrap();

    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut response))
        .await
        .expect("Timed out waiting for response")
        .expect("Failed to read response");

    let response_str = String::from_utf8_lossy(&response);
    assert!(
        response_str.contains("400 Bad Request"),
        "Expected 400 Bad Request, got: {}",
        response_str
    );
}
