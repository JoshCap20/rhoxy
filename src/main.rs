use clap::Parser;
use anyhow::Result;
use http::Method;
use reqwest::Url;
use std::{collections::HashMap, io::{BufRead, BufReader, Read, Write}, net::{TcpListener, TcpStream}, time::Duration};

#[derive(Parser)]
struct CommandLineArguments {
    port: u16 // allows values 0...65535
}

struct HttpRequest {
    method: Method,
    url: Url,
    headers: HashMap<String, String>,
    body: Option<Vec<u8>>,
}

fn main() {
    let args = CommandLineArguments::parse();
    if let Err(e) = start_server(args.port) {
        eprintln!("Server error: {}", e);
    }
}

fn start_server(port: u16) -> Result<()> {
    let addr = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&addr)?;
    println!("Server listening on {}", addr);

    for stream in listener.incoming() {
        let stream = stream?;
        println!("Connection from {}", stream.peer_addr()?);
        if let Err(e) = handle_connection(stream) {
            println!("Error handling connection: {}", e);
        }
        println!("Connection closed");
    }
    Ok(())
}


// GET and HTTP only for now (no validation cus idk how)
// need to handle CONNECT requests
fn handle_connection(mut stream: TcpStream) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(&stream);
    let mut lines = reader.lines();

    let first_line = lines.next().ok_or_else(|| anyhow::anyhow!("No request line found"))??;
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() != 3 {
        return Err(anyhow::anyhow!("Invalid request line: {}", first_line));
    }
    let method = Method::from_bytes(parts[0].as_bytes())
        .map_err(|e| anyhow::anyhow!("Invalid method: {}", e))?;
    let url = Url::parse(parts[1])
        .map_err(|e| anyhow::anyhow!("Invalid URL: {}", e))?;

    let mut headers = HashMap::new();
    for line in lines.by_ref().map_while(|res| res.ok().filter(|s| !s.is_empty())) {
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(key.trim().to_string(), value.trim().to_string());
        } else {
            return Err(anyhow::anyhow!("Invalid header line: {}", line));
        }
    }

    let body = if let Some(len_str) = headers.get("Content-Length") {
        let len: usize = len_str.parse().map_err(|_| anyhow::anyhow!("Invalid Content-Length"))?;
        let mut body_vec = vec![0u8; len];
        reader.read_exact(&mut body_vec)?;
        Some(body_vec)
    } else {
        None
    };

    let request = HttpRequest {
        method,
        url,
        headers,
        body
    };

    let res = send_request(&request)?;

    let mut response = format!("HTTP/1.1 {} {}\r\n", res.status().as_u16(), res.status().canonical_reason().unwrap_or(""));
    for (key, value) in res.headers() {
        response.push_str(&format!("{}: {}\r\n", key, value.to_str().unwrap_or("")));
    }
    response.push_str("\r\n");
    stream.write_all(response.as_bytes())?;

    let body_bytes = res.bytes()?;
    stream.write_all(&body_bytes)?;

    stream.flush()?;
    println!("Response sent for request: {}, {}", first_line, response);
    Ok(())
}

fn send_request(request: &HttpRequest) -> Result<reqwest::blocking::Response> {
    // todo add support for POST body and other headers
    let client = reqwest::blocking::Client::new();
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