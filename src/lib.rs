pub mod constants;
pub mod protocol;

use ::http::Method;
use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

pub async fn extract_request_parts<R>(reader: &mut R) -> Result<(Method, String)>
where
    R: AsyncBufReadExt + Unpin,
{
    let mut first_line = String::new();
    reader.read_line(&mut first_line).await?;
    if first_line.len() > constants::MAX_REQUEST_LINE_LEN {
        return Err(anyhow::anyhow!(
            "Request line too long: {} bytes (max {})",
            first_line.len(),
            constants::MAX_REQUEST_LINE_LEN
        ));
    }
    let first_line = first_line.trim();

    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() != 3 {
        return Err(anyhow::anyhow!("Invalid request line: {}", first_line));
    }

    let method = Method::from_bytes(parts[0].as_bytes())?;
    let url_string = parts[1].to_string();

    Ok((method, url_string))
}

pub fn is_health_check(url: &str) -> bool {
    if url == constants::HEALTH_ENDPOINT_PATH {
        return true;
    }
    // For absolute URLs like http://host:port/health, extract the path
    if let Some(scheme_end) = url.find("://") {
        let after_authority = &url[scheme_end + 3..];
        if let Some(path_start) = after_authority.find('/') {
            return &after_authority[path_start..] == constants::HEALTH_ENDPOINT_PATH;
        }
    }
    false
}

pub async fn handle_health_check<W>(writer: &mut W) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    writer.write_all(constants::HEALTH_CHECK_RESPONSE).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[tokio::test]
    async fn test_extract_request_parts_valid_get() {
        let request = "GET /path HTTP/1.1\r\n";
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader).await.unwrap();
        assert_eq!(result.0, Method::GET);
        assert_eq!(result.1, "/path");
    }

    #[tokio::test]
    async fn test_extract_request_parts_valid_post() {
        let request = "POST /api/users HTTP/1.1\r\n";
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader).await.unwrap();
        assert_eq!(result.0, Method::POST);
        assert_eq!(result.1, "/api/users");
    }

    #[tokio::test]
    async fn test_extract_request_parts_valid_connect() {
        let request = "CONNECT example.com:443 HTTP/1.1\r\n";
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader).await.unwrap();
        assert_eq!(result.0, Method::CONNECT);
        assert_eq!(result.1, "example.com:443");
    }

    #[tokio::test]
    async fn test_extract_request_parts_full_url() {
        let request = "GET https://example.com/path HTTP/1.1\r\n";
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader).await.unwrap();
        assert_eq!(result.0, Method::GET);
        assert_eq!(result.1, "https://example.com/path");
    }

    #[tokio::test]
    async fn test_extract_request_parts_allows_unstandard_methods() {
        let request = "INVALID /path HTTP/1.1\r\n";
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader).await.unwrap();
        assert_eq!(result.0.as_str(), "INVALID");
        assert_eq!(result.1, "/path");
    }

    #[tokio::test]
    async fn test_extract_request_parts_too_few_parts() {
        let request = "GET /path\r\n";
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid request line")
        );
    }

    #[tokio::test]
    async fn test_extract_request_parts_too_many_parts() {
        let request = "GET /path HTTP/1.1 extra\r\n";
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid request line")
        );
    }

    #[tokio::test]
    async fn test_extract_request_parts_empty_line() {
        let request = "\r\n";
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid request line")
        );
    }

    #[test]
    fn test_is_health_check_matches_relative_path() {
        assert!(is_health_check("/health"));
    }

    #[test]
    fn test_is_health_check_matches_absolute_url() {
        assert!(is_health_check("http://localhost:8080/health"));
        assert!(is_health_check("http://127.0.0.1:8081/health"));
        assert!(is_health_check("http://proxy.example.com/health"));
    }

    #[test]
    fn test_is_health_check_rejects_non_health() {
        assert!(!is_health_check("/other"));
        assert!(!is_health_check("http://example.com/api"));
        assert!(!is_health_check("/healthcheck"));
    }

    #[tokio::test]
    async fn test_extract_request_parts_rejects_oversized_line() {
        let long_path = "X".repeat(constants::MAX_REQUEST_LINE_LEN + 1);
        let request = format!("GET /{} HTTP/1.1\r\n", long_path);
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader).await;
        assert!(result.is_err(), "Should reject request lines exceeding size limit");
    }

    #[tokio::test]
    async fn test_extract_request_parts_whitespace_handling() {
        let request = "  GET   /path   HTTP/1.1  \r\n";
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader).await.unwrap();
        assert_eq!(result.0, Method::GET);
        assert_eq!(result.1, "/path");
    }
}
