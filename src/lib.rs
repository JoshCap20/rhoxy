pub mod constants;
pub mod protocol;

use ::http::Method;
use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

pub async fn read_line_bounded<R>(reader: &mut R, buf: &mut String, max_len: usize) -> Result<()>
where
    R: AsyncBufReadExt + Unpin,
{
    let mut bytes = Vec::new();
    let mut total = 0;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            break;
        }

        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            let to_consume = pos + 1;
            if total + to_consume > max_len {
                return Err(anyhow::anyhow!(
                    "Line exceeds maximum length of {} bytes",
                    max_len
                ));
            }
            bytes.extend_from_slice(&available[..to_consume]);
            reader.consume(to_consume);
            break;
        }

        let len = available.len();
        if total + len > max_len {
            return Err(anyhow::anyhow!(
                "Line exceeds maximum length of {} bytes",
                max_len
            ));
        }
        bytes.extend_from_slice(available);
        reader.consume(len);
        total += len;
    }

    *buf = String::from_utf8(bytes).map_err(|e| anyhow::anyhow!("Invalid UTF-8: {}", e))?;
    Ok(())
}

pub async fn extract_request_parts<R>(reader: &mut R) -> Result<(Method, String)>
where
    R: AsyncBufReadExt + Unpin,
{
    let mut first_line = String::new();
    read_line_bounded(
        &mut *reader,
        &mut first_line,
        constants::MAX_REQUEST_LINE_LEN,
    )
    .await?;
    let first_line = first_line.trim();

    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() != 3 {
        return Err(anyhow::anyhow!("Invalid request line: {}", first_line));
    }

    let method = Method::from_bytes(parts[0].as_bytes())?;
    let url_string = parts[1].to_string();

    Ok((method, url_string))
}

pub fn is_private_address(host: &str) -> bool {
    if host == "localhost" {
        return true;
    }

    // Strip IPv6 zone ID (e.g., "fe80::1%eth0" → "fe80::1") since Rust's
    // IpAddr parser rejects the % suffix.
    let host = host.split('%').next().unwrap_or(host);

    if let Ok(addr) = host.parse::<std::net::IpAddr>() {
        return is_private_ip(&addr);
    }

    false
}

pub fn is_private_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(addr) => is_private_ipv4(addr),
        std::net::IpAddr::V6(addr) => {
            if addr.is_loopback() || addr.is_unspecified() {
                return true;
            }
            // Unique-local (fc00::/7) — IPv6 equivalent of RFC 1918
            if (addr.segments()[0] & 0xfe00) == 0xfc00 {
                return true;
            }
            // Link-local (fe80::/10)
            if (addr.segments()[0] & 0xffc0) == 0xfe80 {
                return true;
            }
            // IPv4-mapped IPv6 (::ffff:0:0/96) — check the embedded IPv4
            if let Some(v4) = addr.to_ipv4_mapped() {
                return is_private_ipv4(&v4);
            }
            false
        }
    }
}

fn is_private_ipv4(addr: &std::net::Ipv4Addr) -> bool {
    addr.is_loopback() || addr.is_private() || addr.is_link_local() || addr.is_unspecified()
}

pub async fn resolve_and_verify_non_private(
    host: &str,
    port: u16,
) -> Result<Vec<std::net::SocketAddr>> {
    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host(format!("{}:{}", host, port))
        .await?
        .collect();

    if addrs.is_empty() {
        return Err(anyhow::anyhow!(
            "DNS resolution returned no addresses for {}:{}",
            host,
            port
        ));
    }

    for addr in &addrs {
        if is_private_ip(&addr.ip()) {
            return Err(anyhow::anyhow!(
                "DNS rebinding detected: {} resolved to private IP {}",
                host,
                addr.ip()
            ));
        }
    }

    Ok(addrs)
}

pub fn is_health_check(url: &str) -> bool {
    // Only match relative /health — this targets the proxy itself.
    // Absolute URLs (http://host/health) target upstream servers and must be forwarded.
    let path = url.split('?').next().unwrap_or(url);
    path == constants::HEALTH_ENDPOINT_PATH
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
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid request line"));
    }

    #[tokio::test]
    async fn test_extract_request_parts_too_many_parts() {
        let request = "GET /path HTTP/1.1 extra\r\n";
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid request line"));
    }

    #[tokio::test]
    async fn test_extract_request_parts_empty_line() {
        let request = "\r\n";
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid request line"));
    }

    #[test]
    fn test_is_health_check_matches_relative_path() {
        assert!(is_health_check("/health"));
    }

    #[test]
    fn test_is_health_check_ignores_absolute_url() {
        // Absolute URLs target upstream servers, not the proxy itself.
        // Only relative /health should be intercepted.
        assert!(!is_health_check("http://localhost:8080/health"));
        assert!(!is_health_check("http://127.0.0.1:8081/health"));
        assert!(!is_health_check("http://api.example.com/health"));
    }

    #[test]
    fn test_is_health_check_rejects_non_health() {
        assert!(!is_health_check("/other"));
        assert!(!is_health_check("http://example.com/api"));
        assert!(!is_health_check("/healthcheck"));
    }

    #[test]
    fn test_is_health_check_with_query_string() {
        assert!(is_health_check("/health?foo=bar"));
        // Absolute URL with query string targets upstream, not proxy
        assert!(!is_health_check("http://localhost:8080/health?check=1"));
    }

    #[tokio::test]
    async fn test_extract_request_parts_rejects_oversized_line() {
        let long_path = "X".repeat(constants::MAX_REQUEST_LINE_LEN + 1);
        let request = format!("GET /{} HTTP/1.1\r\n", long_path);
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader).await;
        assert!(
            result.is_err(),
            "Should reject request lines exceeding size limit"
        );
    }

    #[tokio::test]
    async fn test_extract_request_parts_whitespace_handling() {
        let request = "  GET   /path   HTTP/1.1  \r\n";
        let mut reader = Cursor::new(request);

        let result = extract_request_parts(&mut reader).await.unwrap();
        assert_eq!(result.0, Method::GET);
        assert_eq!(result.1, "/path");
    }

    #[tokio::test]
    async fn test_read_line_bounded_within_limit() {
        let data = "hello world\n";
        let mut reader = Cursor::new(data);
        let mut buf = String::new();
        read_line_bounded(&mut reader, &mut buf, 100).await.unwrap();
        assert_eq!(buf, "hello world\n");
    }

    #[tokio::test]
    async fn test_read_line_bounded_rejects_oversized() {
        let long_data = "X".repeat(100) + "\n";
        let mut reader = Cursor::new(long_data);
        let mut buf = String::new();
        let result = read_line_bounded(&mut reader, &mut buf, 50).await;
        assert!(result.is_err(), "Should reject lines exceeding the bound");
    }

    #[tokio::test]
    async fn test_read_line_bounded_exact_limit() {
        let data = "12345\n";
        let mut reader = Cursor::new(data);
        let mut buf = String::new();
        let result = read_line_bounded(&mut reader, &mut buf, 6).await;
        assert!(result.is_ok());
        assert_eq!(buf, "12345\n");
    }

    #[tokio::test]
    async fn test_read_line_bounded_one_over_limit() {
        let data = "123456\n";
        let mut reader = Cursor::new(data);
        let mut buf = String::new();
        let result = read_line_bounded(&mut reader, &mut buf, 6).await;
        assert!(
            result.is_err(),
            "Should reject when line is one byte over limit"
        );
    }

    #[tokio::test]
    async fn test_read_line_bounded_eof_within_limit() {
        let data = "no newline";
        let mut reader = Cursor::new(data);
        let mut buf = String::new();
        read_line_bounded(&mut reader, &mut buf, 100).await.unwrap();
        assert_eq!(buf, "no newline");
    }

    #[tokio::test]
    async fn test_read_line_bounded_no_newline_exceeds_limit() {
        let long_data = "X".repeat(100);
        let mut reader = Cursor::new(long_data);
        let mut buf = String::new();
        let result = read_line_bounded(&mut reader, &mut buf, 50).await;
        assert!(
            result.is_err(),
            "Should reject even without newline when exceeding limit"
        );
    }

    #[test]
    fn test_is_private_address() {
        assert!(is_private_address("127.0.0.1"));
        assert!(is_private_address("10.0.0.1"));
        assert!(is_private_address("10.255.255.255"));
        assert!(is_private_address("172.16.0.1"));
        assert!(is_private_address("172.31.255.255"));
        assert!(is_private_address("192.168.1.1"));
        assert!(is_private_address("169.254.169.254"));
        assert!(is_private_address("0.0.0.0"));
        assert!(is_private_address("::1"));
        assert!(is_private_address("localhost"));

        assert!(!is_private_address("8.8.8.8"));
        assert!(!is_private_address("example.com"));
        assert!(!is_private_address("203.0.113.1"));
    }

    #[test]
    fn test_is_private_ip_v4() {
        use std::net::{IpAddr, Ipv4Addr};
        assert!(is_private_ip(&IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        assert!(is_private_ip(&IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(is_private_ip(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        assert!(is_private_ip(&IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));
        assert!(is_private_ip(&IpAddr::V4(Ipv4Addr::new(
            169, 254, 169, 254
        ))));
        assert!(is_private_ip(&IpAddr::V4(Ipv4Addr::UNSPECIFIED)));
        assert!(!is_private_ip(&IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_private_ip(&IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1))));
    }

    #[test]
    fn test_is_private_ip_v6() {
        use std::net::{IpAddr, Ipv6Addr};
        assert!(is_private_ip(&IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(is_private_ip(&IpAddr::V6(Ipv6Addr::UNSPECIFIED)));
        assert!(!is_private_ip(&IpAddr::V6("2001:db8::1".parse().unwrap())));
    }

    #[test]
    fn test_is_private_ip_v6_unique_local() {
        use std::net::IpAddr;
        // fc00::/7 (unique-local) — IPv6 equivalent of RFC 1918 private ranges
        assert!(is_private_ip(&IpAddr::V6("fc00::1".parse().unwrap())));
        assert!(is_private_ip(&IpAddr::V6("fd00::1".parse().unwrap())));
        assert!(is_private_ip(&IpAddr::V6(
            "fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff".parse().unwrap()
        )));
        // fe80::/10 (link-local)
        assert!(is_private_ip(&IpAddr::V6("fe80::1".parse().unwrap())));
    }

    #[test]
    fn test_is_private_ip_v4_mapped_v6() {
        use std::net::IpAddr;
        // IPv4-mapped IPv6 addresses must be checked against IPv4 private ranges
        assert!(is_private_ip(
            &"::ffff:127.0.0.1".parse::<IpAddr>().unwrap()
        ));
        assert!(is_private_ip(&"::ffff:10.0.0.1".parse::<IpAddr>().unwrap()));
        assert!(is_private_ip(
            &"::ffff:192.168.1.1".parse::<IpAddr>().unwrap()
        ));
        assert!(is_private_ip(
            &"::ffff:169.254.169.254".parse::<IpAddr>().unwrap()
        ));
        // Public IPv4-mapped should NOT be flagged
        assert!(!is_private_ip(&"::ffff:8.8.8.8".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn test_is_private_address_v6_unique_local() {
        assert!(is_private_address("fc00::1"));
        assert!(is_private_address("fd12:3456::1"));
        assert!(is_private_address("fe80::1"));
    }

    #[test]
    fn test_is_private_address_v6_zone_id() {
        // IPv6 link-local with zone ID — Rust's IpAddr parser rejects the %
        // suffix, so we must strip it before parsing.
        assert!(is_private_address("fe80::1%eth0"));
        assert!(is_private_address("fe80::1%25eth0"));
        assert!(is_private_address("::1%lo"));
    }

    #[test]
    fn test_is_private_address_v4_mapped_v6() {
        assert!(is_private_address("::ffff:127.0.0.1"));
        assert!(is_private_address("::ffff:10.0.0.1"));
        assert!(!is_private_address("::ffff:8.8.8.8"));
    }

    #[tokio::test]
    async fn test_resolve_and_verify_blocks_localhost() {
        let result = resolve_and_verify_non_private("localhost", 80).await;
        assert!(
            result.is_err(),
            "Should block hostnames resolving to private IPs"
        );
    }
}
