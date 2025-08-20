pub mod https;

use anyhow::Result;
use std::{io::Write, net::TcpStream};

pub fn forward_response(stream: &mut TcpStream, res: reqwest::blocking::Response) -> Result<()> {
    let mut response = format!(
        "HTTP/1.1 {} {}\r\n",
        res.status().as_u16(),
        res.status().canonical_reason().unwrap_or("")
    );

    for (key, value) in res.headers() {
        let key_str = key.as_str();
        if key_str != "connection" && key_str != "transfer-encoding" {
            response.push_str(&format!("{}: {}\r\n", key, value.to_str().unwrap_or("")));
        }
    }
    response.push_str("\r\n");

    stream.write_all(response.as_bytes())?;

    let body_bytes = res.bytes()?;
    stream.write_all(&body_bytes)?;
    stream.flush()?;

    Ok(())
}
