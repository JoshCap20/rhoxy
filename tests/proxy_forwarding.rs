//! Integration tests for HTTP/HTTPS forwarding through the proxy.
//!
//! These tests require the `_test-support` feature because they use localhost
//! upstreams. The SSRF bypass is enabled once for the entire binary.
//!
//!     cargo test --features _test-support --test proxy_forwarding
#![cfg(feature = "_test-support")]

mod common;

use std::sync::Once;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};

static INIT: Once = Once::new();

/// Enable SSRF bypass once for the entire test binary. Every test in this file
/// needs it because they forward to localhost upstreams.
fn setup() {
    INIT.call_once(|| {
        rhoxy::test_support::set_ssrf_bypass(true);
    });
}

// ---------------------------------------------------------------------------
// SSRF bypass verification
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_ssrf_bypass_active() {
    setup();
    assert!(
        !rhoxy::is_private_address("127.0.0.1"),
        "SSRF bypass should be active in this test binary"
    );
}

// ---------------------------------------------------------------------------
// HTTP forwarding
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_http_get_forwarding() {
    setup();

    let upstream =
        common::start_upstream(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello").await;
    let proxy = common::start_proxy().await;

    let request = format!(
        "GET http://{}/path HTTP/1.1\r\nHost: {}\r\n\r\n",
        upstream, upstream
    );
    let response = common::send_raw(proxy, request.as_bytes()).await;

    assert!(
        response.contains("200 OK"),
        "Expected 200 OK, got: {}",
        response
    );
    assert!(
        response.contains("hello"),
        "Expected body 'hello', got: {}",
        response
    );
}

#[tokio::test]
async fn test_http_post_with_body() {
    setup();

    let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = upstream_listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (stream, _) = upstream_listener.accept().await.unwrap();
        let (reader, writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut writer = BufWriter::new(writer);

        let body = common::read_upstream_body(&mut reader).await;

        let resp_body = format!("received {} bytes", body.len());
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
            resp_body.len(),
            resp_body
        );
        writer.write_all(resp.as_bytes()).await.unwrap();
        writer.flush().await.unwrap();
    });

    let proxy = common::start_proxy().await;
    let body = "test body payload";
    let request = format!(
        "POST http://{}/submit HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\n\r\n{}",
        upstream_addr,
        upstream_addr,
        body.len(),
        body
    );
    let response = common::send_raw(proxy, request.as_bytes()).await;

    assert!(
        response.contains("200 OK"),
        "Expected 200 OK, got: {}",
        response
    );
    let expected = format!("received {} bytes", body.len());
    assert!(
        response.contains(&expected),
        "Expected upstream to receive body, got: {}",
        response
    );
}

#[tokio::test]
async fn test_http_response_headers_forwarded() {
    setup();

    let upstream = common::start_upstream(
        b"HTTP/1.1 200 OK\r\nX-Custom: test-value\r\nContent-Length: 2\r\n\r\nOK",
    )
    .await;
    let proxy = common::start_proxy().await;

    let request = format!(
        "GET http://{}/path HTTP/1.1\r\nHost: {}\r\n\r\n",
        upstream, upstream
    );
    let response = common::send_raw(proxy, request.as_bytes()).await;

    assert!(
        response.contains("x-custom: test-value") || response.contains("X-Custom: test-value"),
        "Expected X-Custom header in response, got: {}",
        response
    );
}

#[tokio::test]
async fn test_http_hop_by_hop_headers_stripped_and_others_forwarded() {
    setup();

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
        let has_x_keep = received
            .lines()
            .any(|l| l.to_lowercase().starts_with("x-keep:"));

        let body = match (has_proxy_auth, has_x_keep) {
            (false, true) => "STRIPPED_AND_KEPT",
            (true, _) => "LEAKED",
            (false, false) => "OVER_STRIPPED",
        };

        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(resp.as_bytes()).await.unwrap();
        stream.shutdown().await.unwrap();
    });

    let proxy = common::start_proxy().await;
    let request = format!(
        "GET http://{}/path HTTP/1.1\r\nHost: {}\r\nProxy-Authorization: Basic secret\r\nX-Keep: yes\r\n\r\n",
        upstream_addr, upstream_addr
    );
    let response = common::send_raw(proxy, request.as_bytes()).await;

    assert!(
        response.contains("200 OK"),
        "Expected 200 OK, got: {}",
        response
    );
    assert!(
        response.contains("STRIPPED_AND_KEPT"),
        "Expected hop-by-hop stripped and non-hop-by-hop kept, got: {}",
        response
    );
}

// ---------------------------------------------------------------------------
// Health check non-interception for absolute URLs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_health_check_not_intercepted_for_absolute_url() {
    setup();

    let upstream =
        common::start_upstream(b"HTTP/1.1 418 I'm a Teapot\r\nContent-Length: 6\r\n\r\nteapot")
            .await;
    let proxy = common::start_proxy().await;

    let request = format!(
        "GET http://{}/health HTTP/1.1\r\nHost: {}\r\n\r\n",
        upstream, upstream
    );
    let response = common::send_raw(proxy, request.as_bytes()).await;

    assert!(
        response.contains("418"),
        "Absolute /health should be forwarded to upstream, got: {}",
        response
    );
}

// ---------------------------------------------------------------------------
// 502 Bad Gateway — deterministic (closed port)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_http_502_on_closed_port() {
    setup();

    // Bind and immediately drop to get a port guaranteed not listening.
    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_addr = dead.local_addr().unwrap();
    drop(dead);

    let proxy = common::start_proxy().await;
    let request = format!(
        "GET http://{}/path HTTP/1.1\r\nHost: {}\r\n\r\n",
        dead_addr, dead_addr
    );
    let response = common::send_raw(proxy, request.as_bytes()).await;

    assert!(
        response.contains("502 Bad Gateway"),
        "Expected 502 for closed port, got: {}",
        response
    );
}

#[tokio::test]
async fn test_connect_502_on_closed_port() {
    setup();

    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_addr = dead.local_addr().unwrap();
    drop(dead);

    let proxy = common::start_proxy().await;
    let request = format!(
        "CONNECT {} HTTP/1.1\r\nHost: {}\r\n\r\n",
        dead_addr, dead_addr
    );
    let response = common::send_raw(proxy, request.as_bytes()).await;

    assert!(
        response.contains("502 Bad Gateway"),
        "Expected 502 for CONNECT to closed port, got: {}",
        response
    );
}

// ---------------------------------------------------------------------------
// CONNECT tunnel — bidirectional data
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_connect_tunnel_bidirectional() {
    setup();

    // Start a TCP echo server: reads data, sends it back prefixed with "echo:".
    let echo_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let echo_addr = echo_listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (mut stream, _) = echo_listener.accept().await.unwrap();
        let mut buf = vec![0u8; 1024];
        let n = stream.read(&mut buf).await.unwrap();
        let mut response = b"echo:".to_vec();
        response.extend_from_slice(&buf[..n]);
        stream.write_all(&response).await.unwrap();
        stream.shutdown().await.unwrap();
    });

    let proxy = common::start_proxy().await;
    let mut stream = TcpStream::connect(proxy).await.unwrap();

    // Phase 1: Send CONNECT request
    let connect_req = format!(
        "CONNECT {} HTTP/1.1\r\nHost: {}\r\n\r\n",
        echo_addr, echo_addr
    );
    stream.write_all(connect_req.as_bytes()).await.unwrap();

    // Phase 2: Read "200 Connection Established\r\n\r\n"
    let mut buf = vec![0u8; 256];
    let mut total = 0;
    loop {
        let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf[total..]))
            .await
            .expect("Timed out waiting for CONNECT response")
            .expect("Failed to read CONNECT response");
        assert!(n > 0, "Connection closed before tunnel established");
        total += n;
        if buf[..total].windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let establishment = String::from_utf8_lossy(&buf[..total]);
    assert!(
        establishment.contains("200 Connection Established"),
        "Expected tunnel established, got: {}",
        establishment
    );

    // Phase 3: Send data through the tunnel
    stream.write_all(b"hello tunnel").await.unwrap();
    stream.shutdown().await.unwrap();

    // Phase 4: Read echo response through the tunnel
    let mut tunnel_response = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(5),
        stream.read_to_end(&mut tunnel_response),
    )
    .await
    .expect("Timed out reading tunnel data")
    .expect("Failed to read tunnel data");

    let tunnel_str = String::from_utf8_lossy(&tunnel_response);
    assert!(
        tunnel_str.contains("echo:hello tunnel"),
        "Expected echo response through tunnel, got: {}",
        tunnel_str
    );
}

// ---------------------------------------------------------------------------
// Multiple requests through same proxy
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_proxy_handles_multiple_sequential_requests() {
    setup();

    let proxy = common::start_proxy().await;

    for i in 0..3 {
        let upstream =
            common::start_upstream(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK").await;

        let request = format!(
            "GET http://{}/path/{} HTTP/1.1\r\nHost: {}\r\n\r\n",
            upstream, i, upstream
        );
        let response = common::send_raw(proxy, request.as_bytes()).await;
        assert!(
            response.contains("200 OK"),
            "Request {} failed, got: {}",
            i,
            response
        );
    }
}

// ---------------------------------------------------------------------------
// Concurrent requests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_proxy_handles_concurrent_requests() {
    setup();

    let upstream =
        common::start_looping_upstream(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK").await;
    let proxy = common::start_proxy().await;

    let mut handles = Vec::new();
    for i in 0..10 {
        let request = format!(
            "GET http://{}/path/{} HTTP/1.1\r\nHost: {}\r\n\r\n",
            upstream, i, upstream
        );
        let request_bytes = request.into_bytes();
        handles.push(tokio::spawn(async move {
            common::send_raw(proxy, &request_bytes).await
        }));
    }

    for (i, handle) in handles.into_iter().enumerate() {
        let response = handle.await.expect("Task panicked");
        assert!(
            response.contains("200 OK"),
            "Concurrent request {} failed, got: {}",
            i,
            response
        );
    }
}

// ---------------------------------------------------------------------------
// Large body forwarding
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_http_post_large_body() {
    setup();

    let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = upstream_listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (stream, _) = upstream_listener.accept().await.unwrap();
        let (reader, writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut writer = BufWriter::new(writer);

        let body = common::read_upstream_body(&mut reader).await;

        let resp_body = format!("received {} bytes", body.len());
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
            resp_body.len(),
            resp_body
        );
        writer.write_all(resp.as_bytes()).await.unwrap();
        writer.flush().await.unwrap();
    });

    let proxy = common::start_proxy().await;
    let body_size = 128 * 1024; // 128 KiB
    let body = "X".repeat(body_size);
    let request = format!(
        "POST http://{}/submit HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\n\r\n{}",
        upstream_addr,
        upstream_addr,
        body.len(),
        body
    );
    let response = common::send_raw(proxy, request.as_bytes()).await;

    assert!(
        response.contains("200 OK"),
        "Expected 200 OK, got: {}",
        response
    );
    let expected = format!("received {} bytes", body_size);
    assert!(
        response.contains(&expected),
        "Expected upstream to receive full body, got: {}",
        response
    );
}

// ---------------------------------------------------------------------------
// Chunked transfer encoding
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_http_post_chunked_transfer_encoding() {
    setup();

    let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = upstream_listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (stream, _) = upstream_listener.accept().await.unwrap();
        let (reader, writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut writer = BufWriter::new(writer);

        let body = common::read_upstream_body(&mut reader).await;

        let body_str = String::from_utf8_lossy(&body);
        let resp_body = format!("body={}", body_str);
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
            resp_body.len(),
            resp_body
        );
        writer.write_all(resp.as_bytes()).await.unwrap();
        writer.flush().await.unwrap();
    });

    let proxy = common::start_proxy().await;
    // Chunked: "Hello" (5 bytes) + " World!" (7 bytes) = "Hello World!" (12 bytes)
    let request = format!(
        "POST http://{}/submit HTTP/1.1\r\nHost: {}\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nHello\r\n7\r\n World!\r\n0\r\n\r\n",
        upstream_addr, upstream_addr
    );
    let response = common::send_raw(proxy, request.as_bytes()).await;

    assert!(
        response.contains("200 OK"),
        "Expected 200 OK, got: {}",
        response
    );
    assert!(
        response.contains("body=Hello World!"),
        "Expected chunked body reassembled as 'Hello World!', got: {}",
        response
    );
}
