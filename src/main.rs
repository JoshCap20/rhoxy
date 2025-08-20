use anyhow::Result;
use clap::Parser;
use http::Method;
use reqwest::Url;
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    time::Duration,
};

#[derive(Parser)]
struct CommandLineArguments {
    port: u16, // allows values 0...65535
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

fn handle_connection(mut stream: TcpStream) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(&stream);

    let mut first_line = String::new();
    reader.read_line(&mut first_line)?;
    let first_line = first_line.trim();

    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() != 3 {
        return Err(anyhow::anyhow!("Invalid request line: {}", first_line));
    }

    let method = Method::from_bytes(parts[0].as_bytes())
        .map_err(|e| anyhow::anyhow!("Invalid method: {}", e))?;
    let url = Url::parse(parts[1]).map_err(|e| anyhow::anyhow!("Invalid URL: {}", e))?;

    if method == Method::CONNECT {
        return handle_connect_method(&mut stream, &url);
    }

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

    let body = if let Some(len_str) = headers.get("Content-Length") {
        let len: usize = len_str
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid Content-Length"))?;
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
        body,
    };

    let res = send_request(&request)?;

    let mut response = format!(
        "HTTP/1.1 {} {}\r\n",
        res.status().as_u16(),
        res.status().canonical_reason().unwrap_or("")
    );
    for (key, value) in res.headers() {
        response.push_str(&format!("{}: {}\r\n", key, value.to_str().unwrap_or("")));
    }
    response.push_str("\r\n");
    stream.write_all(response.as_bytes())?;

    let body_bytes = res.bytes()?;
    stream.write_all(&body_bytes)?;

    stream.flush()?;
    println!(
        "Response sent for request: {}\n\n\n\n{}\n{}",
        first_line,
        response,
        String::from_utf8_lossy(&body_bytes)
    );
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

fn handle_connect_method(stream: &mut TcpStream, target: &Url) -> Result<()> {
    println!("CONNECT request to: {}", target);
    // TODO: establish a tunnel to the target
    let response = "HTTP/1.1 200 Connection established\r\n\r\n";
    stream.write_all(response.as_bytes())?;
    stream.flush()?;
    
    println!("CONNECT tunneling not fully implemented - HTTPS may not work");
    Ok(())
}
