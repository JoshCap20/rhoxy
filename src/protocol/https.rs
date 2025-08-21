use anyhow::Result;
use log::{debug, error, warn};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    thread,
    time::Duration,
};

use crate::constants;

pub fn handle_connect_method(
    client_stream: &mut TcpStream,
    reader: &mut BufReader<TcpStream>,
    target: String,
) -> Result<()> {
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line.trim().is_empty() {
            break;
        }
    }

    let (host, port) = parse_host_port(target.as_str())?;

    let target_stream = match TcpStream::connect(format!("{}:{}", host, port)) {
        Ok(stream) => stream,
        Err(e) => {
            let error_message = format!("Failed to connect to {}: {}", target, e);
            error!("{}", error_message);
            write!(
                client_stream,
                "{}{}",
                constants::BAD_GATEWAY_RESPONSE_HEADER,
                error_message
            )?;
            client_stream.flush()?;
            return Err(e.into());
        }
    };

    write!(
        client_stream,
        "{}",
        constants::CONNECTION_ESTABLISHED_RESPONSE
    )?;
    client_stream.flush()?;
    debug!("Tunnel established to {}", target);

    tunnel_data(client_stream.try_clone()?, target_stream)?;

    Ok(())
}

fn tunnel_data(client_stream: TcpStream, target_stream: TcpStream) -> Result<()> {
    let mut client_reader = client_stream.try_clone()?;
    let mut client_writer = client_stream;
    let mut target_reader = target_stream.try_clone()?;
    let mut target_writer = target_stream;

    client_reader.set_read_timeout(Some(Duration::from_secs(60)))?;
    target_reader.set_read_timeout(Some(Duration::from_secs(60)))?;

    let client_to_target = thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            match client_reader.read(&mut buffer) {
                Ok(0) => {
                    debug!("Client closed connection");
                    break;
                }
                Ok(n) => {
                    if let Err(e) = target_writer.write_all(&buffer[..n]) {
                        warn!("Error writing to target: {}", e);
                        break;
                    }
                    if let Err(e) = target_writer.flush() {
                        warn!("Error flushing target: {}", e);
                        break;
                    }
                }
                Err(e) => {
                    if !is_timeout_or_would_block(&e) {
                        warn!("Error reading from client: {}", e);
                    }
                    break;
                }
            }
        }
        let _ = target_writer.shutdown(std::net::Shutdown::Both);
    });

    let mut buffer = [0u8; 8192];
    loop {
        match target_reader.read(&mut buffer) {
            Ok(0) => {
                debug!("Target closed connection");
                break;
            }
            Ok(n) => {
                if let Err(e) = client_writer.write_all(&buffer[..n]) {
                    warn!("Error writing to client: {}", e);
                    break;
                }
                if let Err(e) = client_writer.flush() {
                    warn!("Error flushing client: {}", e);
                    break;
                }
            }
            Err(e) => {
                if !is_timeout_or_would_block(&e) {
                    warn!("Error reading from target: {}", e);
                }
                break;
            }
        }
    }

    let _ = client_writer.shutdown(std::net::Shutdown::Both);
    let _ = client_to_target.join();

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

fn is_timeout_or_would_block(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
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
        assert!(result.unwrap_err().to_string().contains("Invalid target format"));
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
        assert!(result.unwrap_err().to_string().contains("Invalid target format"));
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

    #[test]
    fn test_is_timeout_or_would_block_timeout() {
        let error = std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout");
        assert!(is_timeout_or_would_block(&error));
    }

    #[test]
    fn test_is_timeout_or_would_block_would_block() {
        let error = std::io::Error::new(std::io::ErrorKind::WouldBlock, "would block");
        assert!(is_timeout_or_would_block(&error));
    }

    #[test]
    fn test_is_timeout_or_would_block_other_error() {
        let error = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "connection refused");
        assert!(!is_timeout_or_would_block(&error));
    }

    #[test]
    fn test_is_timeout_or_would_block_permission_denied() {
        let error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "permission denied");
        assert!(!is_timeout_or_would_block(&error));
    }
}
