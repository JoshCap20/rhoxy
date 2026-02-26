//! Integration tests for HTTP forwarding through the proxy.
//!
//! These tests require the `_test-support` feature because they use localhost
//! upstreams. The SSRF bypass is enabled so the proxy can forward to local
//! addresses. Run with:
//!
//!     cargo test --features _test-support --test proxy_forwarding
#![cfg(feature = "_test-support")]

use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};

/// Spawn a proxy handler that mimics main.rs handle_connection.
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

/// Spawn a simple upstream HTTP server that reads one request and sends a
/// canned response.
async fn start_upstream(response: &'static [u8]) -> std::net::SocketAddr {
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

fn enable_ssrf_bypass() {
    rhoxy::test_support::set_ssrf_bypass(true);
}

// ---------------------------------------------------------------------------
// HTTP forwarding
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_http_get_forwarding() {
    enable_ssrf_bypass();

    let upstream = start_upstream(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello").await;
    let proxy = start_proxy().await;

    let request = format!(
        "GET http://{}/path HTTP/1.1\r\nHost: {}\r\n\r\n",
        upstream, upstream
    );
    let response = send_raw(proxy, request.as_bytes()).await;

    assert!(response.contains("200"), "Expected 200, got: {}", response);
    assert!(
        response.contains("hello"),
        "Expected body 'hello', got: {}",
        response
    );
}

#[tokio::test]
async fn test_http_post_with_body() {
    enable_ssrf_bypass();

    let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = upstream_listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (stream, _) = upstream_listener.accept().await.unwrap();
        let (reader, writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut writer = BufWriter::new(writer);

        let mut content_length = 0usize;
        let mut line = String::new();
        loop {
            line.clear();
            reader.read_line(&mut line).await.unwrap();
            if line.trim().is_empty() {
                break;
            }
            if let Some(val) = line.strip_prefix("content-length: ") {
                content_length = val.trim().parse().unwrap_or(0);
            }
        }

        let mut body = vec![0u8; content_length];
        if content_length > 0 {
            reader.read_exact(&mut body).await.unwrap();
        }

        let resp_body = format!("received {} bytes", body.len());
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
            resp_body.len(),
            resp_body
        );
        writer.write_all(resp.as_bytes()).await.unwrap();
        writer.flush().await.unwrap();
    });

    let proxy = start_proxy().await;
    let body = "test body payload";
    let request = format!(
        "POST http://{}/submit HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\n\r\n{}",
        upstream_addr,
        upstream_addr,
        body.len(),
        body
    );
    let response = send_raw(proxy, request.as_bytes()).await;

    assert!(response.contains("200"), "Expected 200, got: {}", response);
    assert!(
        response.contains("received 17 bytes"),
        "Expected upstream to receive body, got: {}",
        response
    );
}

#[tokio::test]
async fn test_http_response_headers_forwarded() {
    enable_ssrf_bypass();

    let upstream =
        start_upstream(b"HTTP/1.1 200 OK\r\nX-Custom: test-value\r\nContent-Length: 2\r\n\r\nOK")
            .await;
    let proxy = start_proxy().await;

    let request = format!(
        "GET http://{}/path HTTP/1.1\r\nHost: {}\r\n\r\n",
        upstream, upstream
    );
    let response = send_raw(proxy, request.as_bytes()).await;

    assert!(
        response.contains("x-custom: test-value") || response.contains("X-Custom: test-value"),
        "Expected X-Custom header in response, got: {}",
        response
    );
}

#[tokio::test]
async fn test_http_hop_by_hop_headers_stripped() {
    enable_ssrf_bypass();

    let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = upstream_listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (mut stream, _) = upstream_listener.accept().await.unwrap();
        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).await.unwrap();
        let received = String::from_utf8_lossy(&buf[..n]);

        let has_proxy_auth = received
            .lines()
            .any(|l| l.to_lowercase().starts_with("proxy-authorization:"));

        let body = if has_proxy_auth { "LEAKED" } else { "CLEAN" };

        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(resp.as_bytes()).await.unwrap();
        stream.shutdown().await.unwrap();
    });

    let proxy = start_proxy().await;
    let request = format!(
        "GET http://{}/path HTTP/1.1\r\nHost: {}\r\nProxy-Authorization: Basic secret\r\nX-Keep: yes\r\n\r\n",
        upstream_addr, upstream_addr
    );
    let response = send_raw(proxy, request.as_bytes()).await;

    assert!(response.contains("200"), "Expected 200, got: {}", response);
    assert!(
        response.contains("CLEAN"),
        "Proxy-Authorization should be stripped before forwarding, got: {}",
        response
    );
}

// ---------------------------------------------------------------------------
// Health check non-interception for absolute URLs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_health_check_not_intercepted_for_absolute_url() {
    enable_ssrf_bypass();

    let upstream =
        start_upstream(b"HTTP/1.1 418 I'm a Teapot\r\nContent-Length: 6\r\n\r\nteapot").await;
    let proxy = start_proxy().await;

    let request = format!(
        "GET http://{}/health HTTP/1.1\r\nHost: {}\r\n\r\n",
        upstream, upstream
    );
    let response = send_raw(proxy, request.as_bytes()).await;

    assert!(
        response.contains("418"),
        "Absolute /health should be forwarded to upstream, got: {}",
        response
    );
}

// ---------------------------------------------------------------------------
// Multiple requests through same proxy
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_proxy_handles_multiple_sequential_requests() {
    enable_ssrf_bypass();

    let proxy = start_proxy().await;

    for i in 0..3 {
        let upstream = start_upstream(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK").await;

        let request = format!(
            "GET http://{}/path/{} HTTP/1.1\r\nHost: {}\r\n\r\n",
            upstream, i, upstream
        );
        let response = send_raw(proxy, request.as_bytes()).await;
        assert!(
            response.contains("200"),
            "Request {} failed, got: {}",
            i,
            response
        );
    }
}
