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
    url_string: &str,
) -> Result<()> {
    let url = Url::parse(url_string).map_err(|e| anyhow::anyhow!("Invalid URL: {}", e))?;

    let headers = parse_request_headers(reader)?;
    debug!("Parsed headers: {:?}", headers);

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

    match send_request(&request) {
        Ok(response) => forward_response(stream, response)?,
        Err(e) => {
            error!("Failed to send request: {}", e);
            let error_response = format!("HTTP/1.1 502 Bad Gateway\r\n\r\nProxy error: {}", e);
            stream.write_all(error_response.as_bytes())?;
            stream.flush()?;
            return Err(e);
        }
    }

    Ok(())
}

fn send_request(request: &HttpRequest) -> Result<reqwest::blocking::Response> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .danger_accept_invalid_certs(true) // for testing
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

fn forward_response(stream: &mut TcpStream, res: reqwest::blocking::Response) -> Result<()> {
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

fn parse_request_headers(reader: &mut BufReader<TcpStream>) -> Result<HashMap<String, String>> {
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
