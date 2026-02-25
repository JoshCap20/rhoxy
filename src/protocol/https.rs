use anyhow::Result;
use tokio::io::{copy, AsyncBufReadExt, AsyncWriteExt};
use tokio::join;
use tokio::net::TcpStream;
use tracing::{debug, warn};

use crate::constants;

pub async fn handle_request<W, R>(writer: &mut W, reader: &mut R, target: String) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
    R: AsyncBufReadExt + Unpin,
{
    let mut header_count = 0;
    loop {
        let mut line = String::new();
        crate::read_line_bounded(&mut *reader, &mut line, constants::MAX_HEADER_LINE_LEN).await?;
        if line.trim().is_empty() {
            break;
        }
        header_count += 1;
        if header_count > constants::MAX_HEADER_COUNT {
            return Err(anyhow::anyhow!("Too many headers"));
        }
    }

    let (host, port) = parse_host_port(target.as_str())?;

    if crate::is_private_address(host) {
        warn!("Blocked CONNECT to private address: {}", target);
        writer.write_all(constants::FORBIDDEN_RESPONSE).await?;
        writer.flush().await?;
        return Ok(());
    }

    // Resolve DNS and verify resolved IPs are not private (prevents DNS rebinding)
    let resolved_addrs = match crate::resolve_and_verify_non_private(host, port).await {
        Ok(addrs) => addrs,
        Err(e) => {
            warn!("Blocked CONNECT to {}: {}", target, e);
            writer.write_all(constants::FORBIDDEN_RESPONSE).await?;
            writer.flush().await?;
            return Ok(());
        }
    };

    debug!("Establishing HTTPS connection to {}:{}", host, port);

    let target_stream = match TcpStream::connect(resolved_addrs.as_slice()).await {
        Ok(stream) => stream,
        Err(e) => {
            let error_message = format!("Failed to connect to {}: {}", target, e);
            warn!("{}", error_message);
            writer
                .write_all(constants::BAD_GATEWAY_RESPONSE_HEADER)
                .await?;
            writer.flush().await?;
            // Return Ok — the error is already logged and a 502 sent to the client.
            // Returning Err here would cause the caller to log the same error again.
            return Ok(());
        }
    };

    writer
        .write_all(constants::CONNECTION_ESTABLISHED_RESPONSE)
        .await?;
    writer.flush().await?;
    debug!("Tunnel established to {}", target);

    tunnel_data(writer, reader, target_stream).await?;

    Ok(())
}

async fn tunnel_data<W, R>(
    client_writer: &mut W,
    client_reader: &mut R,
    target_stream: TcpStream,
) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
    R: AsyncBufReadExt + Unpin,
{
    let (mut target_reader, mut target_writer) = target_stream.into_split();

    let (client_to_target, target_to_client) = join!(
        copy(&mut *client_reader, &mut target_writer),
        copy(&mut target_reader, &mut *client_writer)
    );

    client_to_target?;
    target_to_client?;

    debug!("Tunnel closed");
    Ok(())
}

fn parse_host_port(target: &str) -> Result<(&str, u16)> {
    // IPv6
    if target.starts_with('[') {
        if let Some(bracket_end) = target.find("]:") {
            let host = &target[1..bracket_end];
            let port_str = &target[bracket_end + 2..];
            let port = port_str
                .parse::<u16>()
                .map_err(|_| anyhow::anyhow!("Invalid port: {}", port_str))?;
            return Ok((host, port));
        } else if target.ends_with(']') {
            let host = &target[1..target.len() - 1];
            return Ok((host, 443));
        } else {
            return Err(anyhow::anyhow!("Invalid IPv6 format: {}", target));
        }
    }

    // IPv6 without port or IPv4 with port
    if let Some(colon_pos) = target.rfind(':') {
        let colon_count = target.matches(':').count();
        if colon_count > 1 {
            return Ok((target, 443));
        }

        let host = &target[..colon_pos];
        let port_str = &target[colon_pos + 1..];
        let port = port_str
            .parse::<u16>()
            .map_err(|_| anyhow::anyhow!("Invalid port: {}", port_str))?;
        Ok((host, port))
    } else {
        Ok((target, 443))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_host_port_with_port() {
        let result = parse_host_port("example.com:8080").unwrap();
        assert_eq!(result.0, "example.com");
        assert_eq!(result.1, 8080);
    }

    #[test]
    fn test_parse_host_port_without_port() {
        let result = parse_host_port("example.com").unwrap();
        assert_eq!(result.0, "example.com");
        assert_eq!(result.1, 443);
    }

    #[test]
    fn test_parse_host_port_localhost() {
        let result = parse_host_port("localhost:3000").unwrap();
        assert_eq!(result.0, "localhost");
        assert_eq!(result.1, 3000);
    }

    #[test]
    fn test_parse_host_port_ip_address() {
        let result = parse_host_port("192.168.1.1:80").unwrap();
        assert_eq!(result.0, "192.168.1.1");
        assert_eq!(result.1, 80);
    }

    #[test]
    fn test_parse_host_port_ipv6() {
        let result = parse_host_port("[::1]:8080").unwrap();
        assert_eq!(result.0, "::1");
        assert_eq!(result.1, 8080);
    }

    #[test]
    fn test_parse_host_port_invalid_port() {
        let result = parse_host_port("example.com:invalid");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid port"));
    }

    #[test]
    fn test_parse_host_port_port_out_of_range() {
        let result = parse_host_port("example.com:65536");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid port"));
    }

    #[test]
    fn test_parse_host_port_ipv6_with_brackets_and_port() {
        let result = parse_host_port("[2001:db8::1]:8080").unwrap();
        assert_eq!(result.0, "2001:db8::1");
        assert_eq!(result.1, 8080);
    }

    #[test]
    fn test_parse_host_port_ipv6_with_brackets_no_port() {
        let result = parse_host_port("[2001:db8::1]").unwrap();
        assert_eq!(result.0, "2001:db8::1");
        assert_eq!(result.1, 443);
    }

    #[test]
    fn test_parse_host_port_ipv6_without_brackets() {
        let result = parse_host_port("2001:db8::1").unwrap();
        assert_eq!(result.0, "2001:db8::1");
        assert_eq!(result.1, 443);
    }

    #[test]
    fn test_parse_host_port_ipv6_localhost() {
        let result = parse_host_port("[::1]:3000").unwrap();
        assert_eq!(result.0, "::1");
        assert_eq!(result.1, 3000);
    }

    #[test]
    fn test_parse_host_port_invalid_ipv6_brackets() {
        let result = parse_host_port("[2001:db8::1:invalid");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid IPv6 format"));
    }

    #[test]
    fn test_parse_host_port_zero_port() {
        let result = parse_host_port("example.com:0").unwrap();
        assert_eq!(result.0, "example.com");
        assert_eq!(result.1, 0);
    }

    #[test]
    fn test_parse_host_port_max_port() {
        let result = parse_host_port("example.com:65535").unwrap();
        assert_eq!(result.0, "example.com");
        assert_eq!(result.1, 65535);
    }
}
