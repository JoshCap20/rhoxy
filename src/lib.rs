pub mod constants;
pub mod protocol;

use std::io::BufRead;

use anyhow::Result;
use http::Method;

pub fn extract_request_parts(reader: &mut impl BufRead) -> Result<(Method, String)> {
    let mut first_line = String::new();
    reader.read_line(&mut first_line)?;
    let first_line = first_line.trim();

    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() != 3 {
        return Err(anyhow::anyhow!("Invalid request line: {}", first_line));
    }

    let method = Method::from_bytes(parts[0].as_bytes())?;
    let url_string = parts[1].to_string();

    Ok((method, url_string))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_extract_request_parts_valid_get() {
        let request = "GET /path HTTP/1.1\r\n";
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader).unwrap();
        assert_eq!(result.0, Method::GET);
        assert_eq!(result.1, "/path");
    }

    #[test]
    fn test_extract_request_parts_valid_post() {
        let request = "POST /api/users HTTP/1.1\r\n";
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader).unwrap();
        assert_eq!(result.0, Method::POST);
        assert_eq!(result.1, "/api/users");
    }

    #[test]
    fn test_extract_request_parts_valid_connect() {
        let request = "CONNECT example.com:443 HTTP/1.1\r\n";
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader).unwrap();
        assert_eq!(result.0, Method::CONNECT);
        assert_eq!(result.1, "example.com:443");
    }

    #[test]
    fn test_extract_request_parts_full_url() {
        let request = "GET https://example.com/path HTTP/1.1\r\n";
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader).unwrap();
        assert_eq!(result.0, Method::GET);
        assert_eq!(result.1, "https://example.com/path");
    }

    #[test]
    fn test_extract_request_parts_allows_unstandard_methods() {
        let request = "INVALID /path HTTP/1.1\r\n";
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader).unwrap();
        assert_eq!(result.0.as_str(), "INVALID");
        assert_eq!(result.1, "/path");
    }

    #[test]
    fn test_extract_request_parts_too_few_parts() {
        let request = "GET /path\r\n";
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid request line")
        );
    }

    #[test]
    fn test_extract_request_parts_too_many_parts() {
        let request = "GET /path HTTP/1.1 extra\r\n";
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid request line")
        );
    }

    #[test]
    fn test_extract_request_parts_empty_line() {
        let request = "\r\n";
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid request line")
        );
    }

    #[test]
    fn test_extract_request_parts_whitespace_handling() {
        let request = "  GET   /path   HTTP/1.1  \r\n";
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader).unwrap();
        assert_eq!(result.0, Method::GET);
        assert_eq!(result.1, "/path");
    }
}
