mod common;

#[tokio::test]
async fn test_malformed_request_returns_400() {
    let proxy = common::start_proxy().await;
    let response = common::send_raw(proxy, b"NOT-A-VALID-REQUEST\r\n").await;

    assert!(
        response.contains("400 Bad Request"),
        "Expected 400 Bad Request, got: {}",
        response
    );
}

#[tokio::test]
async fn test_empty_request_returns_400() {
    let proxy = common::start_proxy().await;
    let response = common::send_raw(proxy, b"\r\n").await;

    assert!(
        response.contains("400 Bad Request"),
        "Expected 400 Bad Request, got: {}",
        response
    );
}
