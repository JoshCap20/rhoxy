use anyhow::Result;
use http::Method;
use log::{error};
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

fn http_version_to_string(version: http::Version) -> &'static str {
    match version {
        http::Version::HTTP_09 => "HTTP/0.9",
        http::Version::HTTP_10 => "HTTP/1.0",
        http::Version::HTTP_11 => "HTTP/1.1",
        http::Version::HTTP_2 => "HTTP/2.0",
        http::Version::HTTP_3 => "HTTP/3.0",
        _ => {
            log::warn!("Unknown HTTP version: {:?}", version);
            "HTTP/1.1"
        }
    }
}
