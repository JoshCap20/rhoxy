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

    let method = Method::from_bytes(parts[0].as_bytes())
        .map_err(|e| anyhow::anyhow!("Invalid method: {}", e))?;
    let url_string = parts[1].to_string();

    Ok((method, url_string))
}
