use anyhow::Result;
use log::{debug, error, warn};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    thread,
    time::Duration,
};

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
            error!("Failed to connect to target: {}", e);
            let error_response = format!(
                "HTTP/1.1 502 Bad Gateway\r\n\r\nFailed to connect to {}: {}",
                target, e
            );
            client_stream.write_all(error_response.as_bytes())?;
            client_stream.flush()?;
            return Err(e.into());
        }
    };

    let response = "HTTP/1.1 200 Connection Established\r\n\r\n";
    client_stream.write_all(response.as_bytes())?;
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
