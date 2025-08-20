use anyhow::Result;
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read},
    net::TcpStream,
};

pub fn parse_request_headers(reader: &mut BufReader<TcpStream>) -> Result<HashMap<String, String>> {
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

pub fn parse_request_body(
    reader: &mut BufReader<TcpStream>,
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
