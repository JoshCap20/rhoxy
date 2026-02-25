use anyhow::Result;
use http::Method;
use reqwest::Url;
use std::{collections::HashMap, sync::LazyLock, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tracing::{debug, error};

use crate::constants;

static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
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
        .build()
        .expect("Failed to build HTTP client")
});

#[derive(Debug)]
struct HttpRequest {
    method: Method,
    url: Url,
    headers: HashMap<String, String>,
    body: Option<Vec<u8>>,
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

    let request = HttpRequest {
        method,
        url: Url::parse(&url_string)?,
        headers,
        body,
    };

    debug!("Received HTTP request: {:?}", request);

    let client_to_target = match send_request(&request).await {
        Ok(response) => {
            debug!("Forwarding response for request: {:?}", request);
            response
        }
        Err(e) => {
            let error_message = format!("Failed to send request to {}: {}", request.url, e);
            error!(
                "HTTP request failed: {} (error kind: {:?})",
                error_message,
                e.source()
            );
            writer
                .write_all(constants::BAD_GATEWAY_RESPONSE_HEADER)
                .await?;
            writer.flush().await?;
            return Err(e);
        }
    };

    match forward_response(writer, client_to_target).await {
        Ok(_) => {
            debug!("Forwarded response for request: {:?}", request);
        }
        Err(e) => {
            error!("Failed to forward response: {}", e);
            writer
                .write_all(constants::BAD_GATEWAY_RESPONSE_HEADER)
                .await?;
            writer.flush().await?;
            return Err(e);
        }
    };

    Ok(())
}

async fn extract_request_body<R>(
    reader: &mut R,
    headers: &HashMap<String, String>,
) -> Result<Option<Vec<u8>>, anyhow::Error>
where
    R: AsyncReadExt + Unpin,
{
    let content_length = headers
        .get("Content-Length")
        .and_then(|s| s.parse::<usize>().ok());
    let body = parse_request_body(reader, content_length).await?;
    Ok(body)
}

async fn send_request(request: &HttpRequest) -> Result<reqwest::Response> {
    let mut req = HTTP_CLIENT.request(request.method.clone(), request.url.clone());

    for (key, value) in &request.headers {
        let key_lower = key.to_lowercase();
        if !is_hop_by_hop_header(&key_lower) {
            req = req.header(key, value);
        }
    }

    if let Some(body) = &request.body {
        req = req.body(body.clone());
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
        let header_line = format!("{}: {}\r\n", key, value.to_str().unwrap_or(""));
        writer.write_all(header_line.as_bytes()).await?;
    }
    writer.write_all(b"\r\n").await?;

    let body = response.bytes().await?;
    writer.write_all(&body).await?;
    writer.flush().await?;

    Ok(())
}

async fn parse_request_headers<R>(reader: &mut R) -> Result<HashMap<String, String>>
where
    R: AsyncBufReadExt + Unpin,
{
    let mut headers = HashMap::new();
    let mut line = String::new();

    loop {
        line.clear();
        reader.read_line(&mut line).await?;
        let line = line.trim();

        if line.is_empty() {
            break;
        }

        if let Some((key, value)) = line.split_once(':') {
            headers.insert(key.trim().to_string(), value.trim().to_string());
        } else {
            return Err(anyhow::anyhow!("Invalid header line: {}", line));
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
        let mut body = vec![0; length];
        reader.read_exact(&mut body).await?;
        Ok(Some(body))
    } else {
        Ok(None)
    }
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
    use std::collections::HashMap;
    use std::io::Cursor;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};

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
        assert_eq!(result.get("Host").unwrap(), "example.com");
        assert_eq!(result.get("Content-Type").unwrap(), "application/json");
        assert_eq!(result.get("Content-Length").unwrap(), "100");
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
        assert_eq!(result.get("Host").unwrap(), "example.com");
        assert_eq!(result.get("Content-Type").unwrap(), "application/json");
    }

    #[tokio::test]
    async fn test_parse_request_headers_invalid_format() {
        let headers_data = "Invalid header line without colon\r\n\r\n";
        let mut reader = BufReader::new(Cursor::new(headers_data));

        let result = parse_request_headers(&mut reader).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid header line")
        );
    }

    #[tokio::test]
    async fn test_parse_request_headers_colon_in_value() {
        let headers_data = "Authorization: Bearer token:with:colons\r\n\r\n";
        let mut reader = BufReader::new(Cursor::new(headers_data));

        let result = parse_request_headers(&mut reader).await.unwrap();
        assert_eq!(
            result.get("Authorization").unwrap(),
            "Bearer token:with:colons"
        );
    }

    #[tokio::test]
    async fn test_parse_request_headers_empty_value() {
        let headers_data = "Empty-Header:\r\n\r\n";
        let mut reader = BufReader::new(Cursor::new(headers_data));

        let result = parse_request_headers(&mut reader).await.unwrap();
        assert_eq!(result.get("Empty-Header").unwrap(), "");
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
            let response =
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/nowhere\r\nContent-Length: 0\r\n\r\n";
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let request = HttpRequest {
            method: Method::GET,
            url: Url::parse(&format!("http://{}/test", addr)).unwrap(),
            headers: HashMap::new(),
            body: None,
        };

        let response = send_request(&request)
            .await
            .expect("Proxy should return redirect response directly, not follow it");
        assert_eq!(response.status().as_u16(), 302);
    }
}
