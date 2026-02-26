mod common;

// ---------------------------------------------------------------------------
// Health check
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_health_check_returns_200() {
    let proxy = common::start_proxy().await;

    let response =
        common::send_raw(proxy, b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n").await;

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
// SSRF protection — full proxy
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_http_ssrf_block_private_address() {
    let proxy = common::start_proxy().await;

    let response = common::send_raw(
        proxy,
        b"GET http://127.0.0.1/secret HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
    )
    .await;

    assert!(
        response.contains("403 Forbidden"),
        "Expected 403 for private address, got: {}",
        response
    );
}

#[tokio::test]
async fn test_connect_ssrf_block_private_address() {
    let proxy = common::start_proxy().await;

    let response = common::send_raw(
        proxy,
        b"CONNECT 127.0.0.1:443 HTTP/1.1\r\nHost: 127.0.0.1:443\r\n\r\n",
    )
    .await;

    assert!(
        response.contains("403 Forbidden"),
        "Expected 403 for CONNECT to private address, got: {}",
        response
    );
}

#[tokio::test]
async fn test_http_ssrf_block_localhost() {
    let proxy = common::start_proxy().await;

    let response = common::send_raw(
        proxy,
        b"GET http://localhost/secret HTTP/1.1\r\nHost: localhost\r\n\r\n",
    )
    .await;

    assert!(
        response.contains("403 Forbidden"),
        "Expected 403 for localhost, got: {}",
        response
    );
}

// ---------------------------------------------------------------------------
// SSRF protection — handler-level
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_connect_handler_ssrf_blocks_private() {
    let mut writer = Vec::new();
    let mut reader = tokio::io::BufReader::new(std::io::Cursor::new("Host: 127.0.0.1:443\r\n\r\n"));

    let result =
        rhoxy::protocol::https::handle_request(&mut writer, &mut reader, "127.0.0.1:443".into())
            .await;

    assert!(result.is_ok());
    let response = String::from_utf8_lossy(&writer);
    assert!(
        response.contains("403 Forbidden"),
        "Expected 403 for CONNECT to localhost, got: {}",
        response
    );
}
