use anyhow::Result;
use http::Method;
use log::{debug, error};
use reqwest::Url;
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    time::Duration,
};

use crate::constants;

#[derive(Debug)]
struct HttpRequest {
    method: Method,
    url: Url,
    headers: HashMap<String, String>,
    body: Option<Vec<u8>>,
}

pub fn handle_http_request(
    stream: &mut TcpStream,
    reader: &mut BufReader<TcpStream>,
    method: Method,
    url_string: String,
) -> Result<()> {
    let url = Url::parse(url_string.as_str())?;

    let headers = parse_request_headers(reader)?;

    let content_length = headers
        .get("Content-Length")
        .and_then(|s| s.parse::<usize>().ok());

    let body = parse_request_body(reader, content_length)?;

    let request = HttpRequest {
        method,
        url,
        headers,
        body,
    };

    debug!("Received HTTP request: {:?}", request);

    match send_request(&request) {
        Ok(response) => {
            debug!("Forwarding response for request: {:?}", request);
            forward_response(stream, response)?;
            debug!(
                "Forwarded HTTP response from {} for request: {:?}",
                request.url, request
            );
        }
        Err(e) => {
            let error_message = format!("Failed to send request to {}: {}", request.url, e);
            error!(
                "HTTP request failed: {} (error kind: {:?})",
                error_message,
                e.source()
            );
            write!(
                stream,
                "{}{}",
                constants::BAD_GATEWAY_RESPONSE_HEADER,
                error_message
            )?;
            stream.flush()?;
            return Err(e);
        }
    }

    Ok(())
}

fn send_request(request: &HttpRequest) -> Result<reqwest::blocking::Response> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .no_proxy()
        .build()?;

    let mut req = client.request(request.method.clone(), request.url.clone());
    for (key, value) in &request.headers {
        req = req.header(key, value);
    }
    if let Some(body) = &request.body {
        req = req.body(body.clone());
    }
    let response = req.send()?;
    Ok(response)
}

fn forward_response(stream: &mut TcpStream, response: reqwest::blocking::Response) -> Result<()> {
    write!(
        stream,
        "{} {} {}\r\n",
        http_version_to_string(response.version()),
        response.status().as_u16(),
        response.status().canonical_reason().unwrap_or("")
    )?;

    for (key, value) in response.headers().iter() {
        write!(stream, "{}: {}\r\n", key, value.to_str().unwrap_or(""))?;
    }
    write!(stream, "\r\n")?;

    stream.write_all(&response.bytes()?)?;
    stream.flush()?;

    Ok(())
}

fn parse_request_headers(reader: &mut impl BufRead) -> Result<HashMap<String, String>> {
    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
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

fn parse_request_body(
    reader: &mut impl Read,
    content_length: Option<usize>,
) -> Result<Option<Vec<u8>>> {
    if let Some(length) = content_length {
        let mut body = vec![0; length];
        reader.read_exact(&mut body)?;
        Ok(Some(body))
    } else {
        Ok(None)
    }
}

const fn http_version_to_string(version: http::Version) -> &'static str {
    match version {
        http::Version::HTTP_09 => "HTTP/0.9",
        http::Version::HTTP_10 => "HTTP/1.0",
        http::Version::HTTP_11 => "HTTP/1.1",
        http::Version::HTTP_2 => "HTTP/2.0",
        http::Version::HTTP_3 => "HTTP/3.0",
        _ => "HTTP/1.1",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_http_version_to_string() {
        assert_eq!(http_version_to_string(http::Version::HTTP_09), "HTTP/0.9");
        assert_eq!(http_version_to_string(http::Version::HTTP_10), "HTTP/1.0");
        assert_eq!(http_version_to_string(http::Version::HTTP_11), "HTTP/1.1");
        assert_eq!(http_version_to_string(http::Version::HTTP_2), "HTTP/2.0");
        assert_eq!(http_version_to_string(http::Version::HTTP_3), "HTTP/3.0");
    }

    #[test]
    fn test_parse_request_body_with_content_length() {
        let body_data = b"test body content";
        let mut reader = Cursor::new(body_data);

        let result = parse_request_body(&mut reader, Some(17)).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap(), body_data);
    }

    #[test]
    fn test_parse_request_body_no_content_length() {
        let body_data = b"test body content";
        let mut reader = Cursor::new(body_data);

        let result = parse_request_body(&mut reader, None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_request_body_zero_length() {
        let body_data = b"";
        let mut reader = Cursor::new(body_data);

        let result = parse_request_body(&mut reader, Some(0)).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn test_parse_request_headers_valid() {
        let headers_data =
            "Host: example.com\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n";
        let mut reader = Cursor::new(headers_data);

        let result = parse_request_headers(&mut reader).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result.get("Host").unwrap(), "example.com");
        assert_eq!(result.get("Content-Type").unwrap(), "application/json");
        assert_eq!(result.get("Content-Length").unwrap(), "100");
    }

    #[test]
    fn test_parse_request_headers_empty() {
        let headers_data = "\r\n";
        let mut reader = Cursor::new(headers_data);

        let result = parse_request_headers(&mut reader).unwrap();
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_parse_request_headers_whitespace_handling() {
        let headers_data =
            "  Host  :  example.com  \r\n  Content-Type  :  application/json  \r\n\r\n";
        let mut reader = Cursor::new(headers_data);

        let result = parse_request_headers(&mut reader).unwrap();
        assert_eq!(result.get("Host").unwrap(), "example.com");
        assert_eq!(result.get("Content-Type").unwrap(), "application/json");
    }

    #[test]
    fn test_parse_request_headers_invalid_format() {
        let headers_data = "Invalid header line without colon\r\n\r\n";
        let mut reader = Cursor::new(headers_data);

        let result = parse_request_headers(&mut reader);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid header line")
        );
    }

    #[test]
    fn test_parse_request_headers_colon_in_value() {
        let headers_data = "Authorization: Bearer token:with:colons\r\n\r\n";
        let mut reader = Cursor::new(headers_data);

        let result = parse_request_headers(&mut reader).unwrap();
        assert_eq!(
            result.get("Authorization").unwrap(),
            "Bearer token:with:colons"
        );
    }

    #[test]
    fn test_parse_request_headers_empty_value() {
        let headers_data = "Empty-Header:\r\n\r\n";
        let mut reader = Cursor::new(headers_data);

        let result = parse_request_headers(&mut reader).unwrap();
        assert_eq!(result.get("Empty-Header").unwrap(), "");
    }
}
