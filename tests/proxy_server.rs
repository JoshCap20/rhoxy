//! Integration tests for production server behaviors: connection timeout
//! and connection limiting.
//!
//! These tests do NOT require the `_test-support` feature because they do not
//! forward to localhost upstreams — they test proxy server infrastructure only.
//!
//!     cargo test --test proxy_server

mod common;

use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

// ---------------------------------------------------------------------------
// Connection timeout
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_connection_timeout_closes_idle_connection() {
    let proxy = common::start_proxy_with_timeout(Duration::from_secs(1)).await;

    let mut stream = TcpStream::connect(proxy).await.unwrap();

    // Send an incomplete request line — never send \r\n to complete it.
    // The proxy blocks on read_line_bounded waiting for the rest.
    stream.write_all(b"GET ").await.unwrap();

    // Wait longer than the timeout
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // The proxy should have timed out and closed the connection.
    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await.unwrap();
    assert_eq!(n, 0, "Expected EOF after timeout, but got {} bytes", n);
}

// ---------------------------------------------------------------------------
// Connection limit — rejection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_connection_limit_rejects_excess() {
    let proxy = common::start_proxy_with_limit(2).await;

    // Hold two connections by connecting without sending.
    // The proxy blocks on read_line_bounded, keeping permits held.
    let _conn1 = TcpStream::connect(proxy).await.unwrap();
    let _conn2 = TcpStream::connect(proxy).await.unwrap();

    // Give the accept loop time to process both connections
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Third connection: accepted at TCP level but immediately closed by proxy.
    let mut conn3 = TcpStream::connect(proxy).await.unwrap();
    let mut buf = vec![0u8; 1024];
    let result = tokio::time::timeout(Duration::from_secs(2), conn3.read(&mut buf)).await;

    match result {
        Ok(Ok(0)) => {}  // EOF — connection rejected, as expected
        Ok(Err(_)) => {} // Connection reset — also acceptable
        other => panic!(
            "Expected connection rejection (EOF or reset), got: {:?}",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Connection limit — recovery
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_connection_limit_recovers_after_completion() {
    let proxy = common::start_proxy_with_limit(1).await;

    // First request: health check completes and releases the permit.
    let response1 =
        common::send_raw(proxy, b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n").await;
    assert!(
        response1.contains("200 OK"),
        "First health check should succeed, got: {}",
        response1
    );

    // Second request: should succeed because the permit was released.
    let response2 =
        common::send_raw(proxy, b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n").await;
    assert!(
        response2.contains("200 OK"),
        "Second health check should succeed after permit release, got: {}",
        response2
    );
}
