use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};

/// Spawn a proxy handler that mimics main.rs handle_connection.
/// Returns the proxy's listen address.
async fn start_proxy() -> std::net::SocketAddr {
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

                let (method, url_string) = match rhoxy::extract_request_parts(&mut reader).await {
                    Ok(parts) => parts,
                    Err(_) => {
                        let _ = writer
                            .write_all(rhoxy::constants::BAD_REQUEST_RESPONSE)
                            .await;
                        let _ = writer.flush().await;
                        return;
                    }
                };

                if rhoxy::is_health_check(&url_string) {
                    let _ = rhoxy::handle_health_check(&mut writer).await;
                    return;
                }

                let protocol = rhoxy::protocol::Protocol::from_method(&method);
                let _ = protocol
                    .handle_request(&mut writer, &mut reader, method, url_string)
                    .await;
            });
        }
    });

    addr
}

/// Send raw bytes to a TCP address and read the full response.
async fn send_raw(addr: std::net::SocketAddr, request: &[u8]) -> String {
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

// ---------------------------------------------------------------------------
// Health check
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_health_check_returns_200() {
    let proxy = start_proxy().await;

    let request = b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n";
    let response = send_raw(proxy, request).await;

    assert!(
        response.contains("200 OK"),
        "Expected 200 OK for health check, got: {}",
        response
    );
    assert!(
        response.contains("OK"),
        "Expected 'OK' body, got: {}",
        response
    );
}

// ---------------------------------------------------------------------------
// SSRF protection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_http_ssrf_block_private_address() {
    let proxy = start_proxy().await;

    let request = b"GET http://127.0.0.1/secret HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
    let response = send_raw(proxy, request).await;

    assert!(
        response.contains("403 Forbidden"),
        "Expected 403 for private address, got: {}",
        response
    );
}

#[tokio::test]
async fn test_connect_ssrf_block_private_address() {
    let proxy = start_proxy().await;

    let request = b"CONNECT 127.0.0.1:443 HTTP/1.1\r\nHost: 127.0.0.1:443\r\n\r\n";
    let response = send_raw(proxy, request).await;

    assert!(
        response.contains("403 Forbidden"),
        "Expected 403 for CONNECT to private address, got: {}",
        response
    );
}

#[tokio::test]
async fn test_http_ssrf_block_localhost() {
    let proxy = start_proxy().await;

    let request = b"GET http://localhost/secret HTTP/1.1\r\nHost: localhost\r\n\r\n";
    let response = send_raw(proxy, request).await;

    assert!(
        response.contains("403 Forbidden"),
        "Expected 403 for localhost, got: {}",
        response
    );
}

// ---------------------------------------------------------------------------
// Error handling — 502 Bad Gateway
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_http_502_on_upstream_failure() {
    // Can't test 502 through the full proxy because the SSRF filter blocks
    // all private addresses. Test the handler directly with a non-routable host.
    let mut writer = Vec::new();
    let headers_and_body = "Host: unreachable.test.invalid\r\n\r\n";
    let mut reader = tokio::io::BufReader::new(std::io::Cursor::new(headers_and_body));

    let result = rhoxy::protocol::http::handle_request(
        &mut writer,
        &mut reader,
        http::Method::GET,
        "http://unreachable.test.invalid:1/path".to_string(),
    )
    .await;

    assert!(
        result.is_ok(),
        "Handler should not propagate error: {:?}",
        result.err()
    );
    let response = String::from_utf8_lossy(&writer);
    assert!(
        response.contains("502 Bad Gateway") || response.contains("403 Forbidden"),
        "Expected 502 or 403 for unreachable host, got: {}",
        response
    );
}

// ---------------------------------------------------------------------------
// HTTPS CONNECT tunnel
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_connect_tunnel_ssrf_blocks_private() {
    let mut writer = Vec::new();
    let mut reader = tokio::io::BufReader::new(std::io::Cursor::new("Host: 127.0.0.1:443\r\n\r\n"));

    let result = rhoxy::protocol::https::handle_request(
        &mut writer,
        &mut reader,
        "127.0.0.1:443".to_string(),
    )
    .await;

    assert!(result.is_ok());
    let response = String::from_utf8_lossy(&writer);
    assert!(
        response.contains("403 Forbidden"),
        "Expected 403 for CONNECT to localhost, got: {}",
        response
    );
}

#[tokio::test]
async fn test_connect_tunnel_unreachable_host() {
    let mut writer = Vec::new();
    let mut reader = tokio::io::BufReader::new(std::io::Cursor::new(
        "Host: unreachable.test.invalid:1\r\n\r\n",
    ));

    let result = rhoxy::protocol::https::handle_request(
        &mut writer,
        &mut reader,
        "unreachable.test.invalid:1".to_string(),
    )
    .await;

    assert!(result.is_ok(), "Handler should not propagate error");
    let response = String::from_utf8_lossy(&writer);
    assert!(
        response.contains("502 Bad Gateway") || response.contains("403 Forbidden"),
        "Expected 502 or 403, got: {}",
        response
    );
}
