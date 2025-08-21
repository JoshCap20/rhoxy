use anyhow::Result;
use log::{debug, warn};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::constants;

pub async fn handle_connect_method<W, R>(
    writer: &mut W,
    reader: &mut R,
    target: String,
) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
    R: AsyncBufReadExt + Unpin,
{
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await?;
        if line.trim().is_empty() {
            break;
        }
    }

    let (host, port) = parse_host_port(target.as_str())?;

    let target_stream = match TcpStream::connect(format!("{}:{}", host, port)).await {
        Ok(stream) => stream,
        Err(e) => {
            let error_message = format!("Failed to connect to {}: {}", target, e);
            warn!("{}", error_message);
            writer
                .write_all(
                    format!(
                        "{}{}",
                        constants::BAD_GATEWAY_RESPONSE_HEADER,
                        error_message
                    )
                    .as_bytes(),
                )
                .await?;
            writer.flush().await?;
            return Err(e.into());
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

    let (client_to_target, target_to_client) = tokio::join!(
        tokio::io::copy(&mut *client_reader, &mut target_writer),
        tokio::io::copy(&mut target_reader, &mut *client_writer)
    );

    client_to_target?;
    target_to_client?;

    debug!("Tunnel closed");
    Ok(())
}

fn parse_host_port(target: &str) -> Result<(String, u16)> {
    let parts: Vec<&str> = target.split(':').collect();
    match parts.len() {
        1 => Ok((parts[0].to_string(), 443)),
        2 => {
            let port = parts[1]
                .parse::<u16>()
                .map_err(|_| anyhow::anyhow!("Invalid port: {}", parts[1]))?;
            Ok((parts[0].to_string(), port))
        }
        _ => Err(anyhow::anyhow!("Invalid target format: {}", target)),
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
        // TODO: Fix this to support IPv6 addresses
        let result = parse_host_port("[::1]:8080");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid target format")
        );
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
    fn test_parse_host_port_too_many_colons() {
        let result = parse_host_port("example.com:8080:extra");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid target format")
        );
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
