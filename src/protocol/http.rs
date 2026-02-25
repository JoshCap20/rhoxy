use anyhow::Result;
use http::Method;
use reqwest::Url;
use std::{sync::LazyLock, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tracing::{debug, error};

use crate::constants;

/// Shared client configuration applied to both the static pool and per-host
/// pinned clients. Centralised here to prevent timeout/policy drift between
/// the two paths.
fn base_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .pool_max_idle_per_host(20)
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_keepalive(Duration::from_secs(60))
        .http2_keep_alive_interval(Duration::from_secs(30))
        .http2_keep_alive_timeout(Duration::from_secs(10))
        .http2_keep_alive_while_idle(true)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
}

static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    base_client_builder()
        .build()
        .expect("Failed to build HTTP client")
});

#[derive(Debug)]
struct HttpRequest {
    method: Method,
    url: Url,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    resolved_addrs: Vec<std::net::SocketAddr>,
}

pub async fn handle_request<W, R>(
    writer: &mut W,
    reader: &mut R,
    method: Method,
    url_string: String,
) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
    R: AsyncBufReadExt + Unpin,
{
    let headers = parse_request_headers(reader).await?;

    let body = extract_request_body(reader, &headers).await?;

    let url = Url::parse(&url_string)?;

    let mut resolved_addrs = Vec::new();
    if let Some(host) = url.host_str() {
        if crate::is_private_address(host) {
            tracing::warn!("Blocked HTTP request to private address: {}", url_string);
            writer.write_all(constants::FORBIDDEN_RESPONSE).await?;
            writer.flush().await?;
            return Ok(());
        }

        // Resolve DNS and verify resolved IPs are not private (prevents DNS rebinding)
        let port = url.port().unwrap_or(80);
        match crate::resolve_and_verify_non_private(host, port).await {
            Ok(addrs) => resolved_addrs = addrs,
            Err(e) => {
                tracing::warn!("Blocked HTTP request to {}: {}", url_string, e);
                writer.write_all(constants::FORBIDDEN_RESPONSE).await?;
                writer.flush().await?;
                return Ok(());
            }
        }
    }

    let request = HttpRequest {
        method,
        url,
        headers,
        body,
        resolved_addrs,
    };

    debug!("Received HTTP request: {:?}", request);

    let request_url = request.url.to_string();
    let client_to_target = match send_request(request).await {
        Ok(response) => {
            debug!("Forwarding response for {}", request_url);
            response
        }
        Err(e) => {
            error!(
                "HTTP request failed for {}: {} (source: {:?})",
                request_url,
                e,
                e.source()
            );
            writer
                .write_all(constants::BAD_GATEWAY_RESPONSE_HEADER)
                .await?;
            writer.flush().await?;
            return Ok(());
        }
    };

    match forward_response(writer, client_to_target).await {
        Ok(_) => {
            debug!("Forwarded response for {}", request_url);
        }
        Err(e) => {
            error!("Failed to forward response: {}", e);
            writer
                .write_all(constants::BAD_GATEWAY_RESPONSE_HEADER)
                .await?;
            writer.flush().await?;
            return Ok(());
        }
    };

    Ok(())
}

async fn extract_request_body<R>(
    reader: &mut R,
    headers: &[(String, String)],
) -> Result<Option<Vec<u8>>, anyhow::Error>
where
    R: AsyncBufReadExt + Unpin,
{
    let is_chunked = headers.iter().any(|(k, v)| {
        k == "transfer-encoding"
            && v.as_bytes()
                .windows(7)
                .any(|w| w.eq_ignore_ascii_case(b"chunked"))
    });

    if is_chunked {
        let body = parse_chunked_body(reader).await?;
        return Ok(Some(body));
    }

    let content_length = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse::<usize>().ok());
    let body = parse_request_body(reader, content_length).await?;
    Ok(body)
}

async fn send_request(request: HttpRequest) -> Result<reqwest::Response> {
    // Pin DNS to the pre-verified addresses to close the TOCTOU gap: without
    // pinning, reqwest re-resolves independently and an attacker with a short-TTL
    // record could return a private IP on the second resolution.
    //
    // We build a per-host client from base_client_builder() so all timeout,
    // pooling, and keepalive settings stay in sync with HTTP_CLIENT.
    let client = match (request.resolved_addrs.is_empty(), request.url.host_str()) {
        (false, Some(host)) => {
            let mut builder = base_client_builder();
            for addr in &request.resolved_addrs {
                builder = builder.resolve(host, *addr);
            }
            builder.build()?
        }
        // Unreachable in normal proxy operation: handle_request always populates
        // resolved_addrs when a host is present. Defensive fallback only.
        _ => HTTP_CLIENT.clone(),
    };

    let mut req = client.request(request.method, request.url);

    for (key, value) in &request.headers {
        if !is_hop_by_hop_header(key) {
            req = req.header(key, value);
        }
    }

    if let Some(body) = request.body {
        req = req.body(body);
    }

    let response = req.send().await?;
    Ok(response)
}

async fn forward_response<W>(writer: &mut W, response: reqwest::Response) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let status_line = build_proxy_status_line(
        response.status().as_u16(),
        response.status().canonical_reason().unwrap_or(""),
    );
    writer.write_all(status_line.as_bytes()).await?;

    for (key, value) in response.headers().iter() {
        writer.write_all(key.as_str().as_bytes()).await?;
        writer.write_all(b": ").await?;
        writer.write_all(value.as_bytes()).await?;
        writer.write_all(b"\r\n").await?;
    }
    writer.write_all(b"\r\n").await?;

    let mut response = response;
    while let Some(chunk) = response.chunk().await? {
        writer.write_all(&chunk).await?;
    }
    writer.flush().await?;

    Ok(())
}

async fn parse_request_headers<R>(reader: &mut R) -> Result<Vec<(String, String)>>
where
    R: AsyncBufReadExt + Unpin,
{
    let mut headers = Vec::new();
    let mut line = String::new();

    loop {
        line.clear();
        crate::read_line_bounded(&mut *reader, &mut line, constants::MAX_HEADER_LINE_LEN).await?;

        let trimmed = line.trim();

        if trimmed.is_empty() {
            break;
        }

        if headers.len() >= constants::MAX_HEADER_COUNT {
            return Err(anyhow::anyhow!(
                "Too many headers: exceeds limit of {}",
                constants::MAX_HEADER_COUNT
            ));
        }

        if let Some((key, value)) = trimmed.split_once(':') {
            headers.push((key.trim().to_lowercase(), value.trim().to_string()));
        } else {
            return Err(anyhow::anyhow!("Invalid header line: {}", trimmed));
        }
    }
    Ok(headers)
}

async fn parse_request_body<R>(
    reader: &mut R,
    content_length: Option<usize>,
) -> Result<Option<Vec<u8>>>
where
    R: AsyncReadExt + Unpin,
{
    if let Some(length) = content_length {
        if length > constants::MAX_BODY_SIZE {
            return Err(anyhow::anyhow!(
                "Content-Length {} exceeds maximum body size of {} bytes",
                length,
                constants::MAX_BODY_SIZE
            ));
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).await?;
        Ok(Some(body))
    } else {
        Ok(None)
    }
}

async fn parse_chunked_body<R>(reader: &mut R) -> Result<Vec<u8>>
where
    R: AsyncBufReadExt + Unpin,
{
    let mut body = Vec::new();
    let mut line = String::new();

    loop {
        line.clear();
        crate::read_line_bounded(&mut *reader, &mut line, constants::MAX_HEADER_LINE_LEN).await?;
        // Strip chunk extensions (RFC 7230: chunk-size *( ";" chunk-ext ) CRLF)
        let size_str = line.trim().split(';').next().unwrap_or("");
        let size = usize::from_str_radix(size_str, 16)
            .map_err(|_| anyhow::anyhow!("Invalid chunk size: {}", size_str))?;

        if size == 0 {
            // Read trailing \r\n after final chunk
            line.clear();
            crate::read_line_bounded(&mut *reader, &mut line, constants::MAX_HEADER_LINE_LEN)
                .await?;
            break;
        }

        if body.len() + size > constants::MAX_BODY_SIZE {
            return Err(anyhow::anyhow!(
                "Chunked body exceeds maximum size of {} bytes",
                constants::MAX_BODY_SIZE
            ));
        }

        let mut chunk = vec![0u8; size];
        reader.read_exact(&mut chunk).await?;
        body.extend_from_slice(&chunk);

        // Read trailing \r\n after chunk data
        line.clear();
        crate::read_line_bounded(&mut *reader, &mut line, constants::MAX_HEADER_LINE_LEN).await?;
    }

    Ok(body)
}

fn build_proxy_status_line(status_code: u16, reason: &str) -> String {
    format!("HTTP/1.1 {} {}\r\n", status_code, reason)
}

fn is_hop_by_hop_header(header: &str) -> bool {
    matches!(
        header,
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailers"
            | "transfer-encoding"
            | "upgrade"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::Method;
    use reqwest::Url;
    use std::io::Cursor;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};

    fn get_header<'a>(headers: &'a [(String, String)], key: &str) -> Option<&'a str> {
        headers
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    #[tokio::test]
    async fn test_parse_request_body_with_content_length() {
        let body_data = b"test body content";
        let mut reader = BufReader::new(Cursor::new(body_data));

        let result = parse_request_body(&mut reader, Some(17)).await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap(), body_data);
    }

    #[tokio::test]
    async fn test_parse_request_body_no_content_length() {
        let body_data = b"test body content";
        let mut reader = BufReader::new(Cursor::new(body_data));

        let result = parse_request_body(&mut reader, None).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_parse_request_body_zero_length() {
        let body_data = b"";
        let mut reader = BufReader::new(Cursor::new(body_data));

        let result = parse_request_body(&mut reader, Some(0)).await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap(), Vec::<u8>::new());
    }

    #[tokio::test]
    async fn test_parse_request_headers_valid() {
        let headers_data =
            "Host: example.com\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n";
        let mut reader = BufReader::new(Cursor::new(headers_data));

        let result = parse_request_headers(&mut reader).await.unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(get_header(&result, "host").unwrap(), "example.com");
        assert_eq!(
            get_header(&result, "content-type").unwrap(),
            "application/json"
        );
        assert_eq!(get_header(&result, "content-length").unwrap(), "100");
    }

    #[tokio::test]
    async fn test_parse_request_headers_empty() {
        let headers_data = "\r\n";
        let mut reader = BufReader::new(Cursor::new(headers_data));

        let result = parse_request_headers(&mut reader).await.unwrap();
        assert_eq!(result.len(), 0);
    }

    #[tokio::test]
    async fn test_parse_request_headers_whitespace_handling() {
        let headers_data =
            "  Host  :  example.com  \r\n  Content-Type  :  application/json  \r\n\r\n";
        let mut reader = BufReader::new(Cursor::new(headers_data));

        let result = parse_request_headers(&mut reader).await.unwrap();
        assert_eq!(get_header(&result, "host").unwrap(), "example.com");
        assert_eq!(
            get_header(&result, "content-type").unwrap(),
            "application/json"
        );
    }

    #[tokio::test]
    async fn test_parse_request_headers_invalid_format() {
        let headers_data = "Invalid header line without colon\r\n\r\n";
        let mut reader = BufReader::new(Cursor::new(headers_data));

        let result = parse_request_headers(&mut reader).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid header line"));
    }

    #[tokio::test]
    async fn test_parse_request_headers_colon_in_value() {
        let headers_data = "Authorization: Bearer token:with:colons\r\n\r\n";
        let mut reader = BufReader::new(Cursor::new(headers_data));

        let result = parse_request_headers(&mut reader).await.unwrap();
        assert_eq!(
            get_header(&result, "authorization").unwrap(),
            "Bearer token:with:colons"
        );
    }

    #[tokio::test]
    async fn test_parse_request_headers_empty_value() {
        let headers_data = "Empty-Header:\r\n\r\n";
        let mut reader = BufReader::new(Cursor::new(headers_data));

        let result = parse_request_headers(&mut reader).await.unwrap();
        assert_eq!(get_header(&result, "empty-header").unwrap(), "");
    }

    #[tokio::test]
    async fn test_parse_request_headers_rejects_too_many() {
        let mut headers_data = String::new();
        for i in 0..=constants::MAX_HEADER_COUNT {
            headers_data.push_str(&format!("X-Header-{}: value\r\n", i));
        }
        headers_data.push_str("\r\n");
        let mut reader = BufReader::new(Cursor::new(headers_data));

        let result = parse_request_headers(&mut reader).await;
        assert!(
            result.is_err(),
            "Should reject when header count exceeds limit"
        );
    }

    #[tokio::test]
    async fn test_parse_request_headers_rejects_oversized_line() {
        let long_value = "X".repeat(constants::MAX_HEADER_LINE_LEN + 1);
        let headers_data = format!("X-Big: {}\r\n\r\n", long_value);
        let mut reader = BufReader::new(Cursor::new(headers_data));

        let result = parse_request_headers(&mut reader).await;
        assert!(
            result.is_err(),
            "Should reject header lines exceeding size limit"
        );
    }

    #[tokio::test]
    async fn test_parse_request_headers_preserves_duplicates() {
        let headers_data = "Set-Cookie: a=1\r\nSet-Cookie: b=2\r\nHost: example.com\r\n\r\n";
        let mut reader = BufReader::new(Cursor::new(headers_data));

        let result = parse_request_headers(&mut reader).await.unwrap();
        let cookie_values: Vec<&str> = result
            .iter()
            .filter(|(k, _)| k.as_str() == "set-cookie")
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(
            cookie_values.len(),
            2,
            "Both Set-Cookie headers should be preserved"
        );
        assert!(cookie_values.contains(&"a=1"));
        assert!(cookie_values.contains(&"b=2"));
    }

    #[tokio::test]
    async fn test_extract_request_body_case_insensitive_content_length() {
        let body_data = b"hello";
        let mut reader = BufReader::new(Cursor::new(body_data));
        let headers = vec![("content-length".to_string(), "5".to_string())];

        let result = extract_request_body(&mut reader, &headers).await.unwrap();
        assert!(
            result.is_some(),
            "Body should be read regardless of Content-Length casing"
        );
        assert_eq!(result.unwrap(), b"hello");
    }

    #[tokio::test]
    async fn test_parse_chunked_body() {
        let chunked_data = "5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        let mut reader = BufReader::new(Cursor::new(chunked_data));

        let result = parse_chunked_body(&mut reader).await.unwrap();
        assert_eq!(result, b"hello world");
    }

    #[tokio::test]
    async fn test_parse_chunked_body_single_chunk() {
        let chunked_data = "d\r\nhello, world!\r\n0\r\n\r\n";
        let mut reader = BufReader::new(Cursor::new(chunked_data));

        let result = parse_chunked_body(&mut reader).await.unwrap();
        assert_eq!(result, b"hello, world!");
    }

    #[tokio::test]
    async fn test_parse_chunked_body_empty() {
        let chunked_data = "0\r\n\r\n";
        let mut reader = BufReader::new(Cursor::new(chunked_data));

        let result = parse_chunked_body(&mut reader).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_extract_request_body_chunked_transfer_encoding() {
        let chunked_data = "5\r\nhello\r\n0\r\n\r\n";
        let mut reader = BufReader::new(Cursor::new(chunked_data));
        let headers = vec![("transfer-encoding".to_string(), "chunked".to_string())];

        let result = extract_request_body(&mut reader, &headers).await.unwrap();
        assert!(result.is_some(), "Chunked body should be read");
        assert_eq!(result.unwrap(), b"hello");
    }

    #[tokio::test]
    async fn test_extract_request_body_chunked_case_insensitive() {
        let chunked_data = "5\r\nhello\r\n0\r\n\r\n";
        let mut reader = BufReader::new(Cursor::new(chunked_data));
        let headers = vec![("transfer-encoding".to_string(), "Chunked".to_string())];

        let result = extract_request_body(&mut reader, &headers).await.unwrap();
        assert!(
            result.is_some(),
            "Chunked detection should be case-insensitive"
        );
        assert_eq!(result.unwrap(), b"hello");
    }

    #[test]
    fn test_build_proxy_status_line_always_http_1_1() {
        // The proxy-to-client connection is always HTTP/1.1, regardless of
        // what protocol the upstream server used. HTTP/2 responses must be
        // downgraded when serialized back to the client.
        let line = build_proxy_status_line(200, "OK");
        assert_eq!(line, "HTTP/1.1 200 OK\r\n");

        let line = build_proxy_status_line(404, "Not Found");
        assert_eq!(line, "HTTP/1.1 404 Not Found\r\n");

        let line = build_proxy_status_line(302, "Found");
        assert_eq!(line, "HTTP/1.1 302 Found\r\n");
    }

    #[tokio::test]
    async fn test_send_request_does_not_follow_redirects() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = stream.read(&mut buf).await;
            let response = "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/nowhere\r\nContent-Length: 0\r\n\r\n";
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let request = HttpRequest {
            method: Method::GET,
            url: Url::parse(&format!("http://{}/test", addr)).unwrap(),
            headers: Vec::new(),
            body: None,
            resolved_addrs: Vec::new(),
        };

        let response = send_request(request)
            .await
            .expect("Proxy should return redirect response directly, not follow it");
        assert_eq!(response.status().as_u16(), 302);
    }

    #[tokio::test]
    async fn test_parse_chunked_body_rejects_oversized() {
        // Create chunks that together exceed MAX_BODY_SIZE
        let chunk_size = constants::MAX_BODY_SIZE / 2 + 1;
        let chunk_data = "A".repeat(chunk_size);
        let chunked = format!(
            "{:x}\r\n{}\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            chunk_size, chunk_data, chunk_size, chunk_data
        );
        let mut reader = BufReader::new(Cursor::new(chunked));

        let result = parse_chunked_body(&mut reader).await;
        assert!(
            result.is_err(),
            "Should reject chunked body exceeding MAX_BODY_SIZE"
        );
    }

    #[tokio::test]
    async fn test_parse_request_body_rejects_oversized_content_length() {
        let body = vec![0u8; constants::MAX_BODY_SIZE + 1];
        let mut reader = BufReader::new(Cursor::new(body));

        let result = parse_request_body(&mut reader, Some(constants::MAX_BODY_SIZE + 1)).await;
        assert!(
            result.is_err(),
            "Should reject body exceeding MAX_BODY_SIZE"
        );
    }

    #[tokio::test]
    async fn test_handle_request_ssrf_block_returns_ok() {
        // Request to a private address should send 403 and return Ok, not Err
        let request_data = "Host: 127.0.0.1\r\n\r\n";
        let mut reader = BufReader::new(Cursor::new(request_data));
        let mut writer = Vec::new();

        let result = handle_request(
            &mut writer,
            &mut reader,
            Method::GET,
            "http://127.0.0.1/secret".to_string(),
        )
        .await;

        assert!(
            result.is_ok(),
            "SSRF block should return Ok after sending 403"
        );
        let response = String::from_utf8_lossy(&writer);
        assert!(response.contains("403 Forbidden"));
    }

    #[tokio::test]
    async fn test_send_request_uses_resolved_addrs() {
        // Start a local HTTP server
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = stream.read(&mut buf).await;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK")
                .await
                .unwrap();
        });

        // Use a hostname that does NOT resolve in DNS.
        // With DNS pinning (resolved_addrs), reqwest connects to the local server.
        // Without it, reqwest would fail to resolve the hostname.
        let request = HttpRequest {
            method: Method::GET,
            url: Url::parse(&format!(
                "http://nonexistent.test.invalid:{}/test",
                addr.port()
            ))
            .unwrap(),
            headers: Vec::new(),
            body: None,
            resolved_addrs: vec![addr],
        };

        let result = send_request(request).await;
        assert!(
            result.is_ok(),
            "Should connect using pre-resolved addrs, not re-resolving DNS: {:?}",
            result.err()
        );
        assert_eq!(result.unwrap().status().as_u16(), 200);
    }

    #[tokio::test]
    async fn test_parse_chunked_body_with_extensions() {
        // RFC 7230: chunk-size can be followed by ;ext=value
        let chunked_data = "5;ext=val\r\nhello\r\n6;name=\"foo\"\r\n world\r\n0\r\n\r\n";
        let mut reader = BufReader::new(Cursor::new(chunked_data));

        let result = parse_chunked_body(&mut reader).await.unwrap();
        assert_eq!(result, b"hello world");
    }
}
