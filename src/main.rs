use clap::Parser;
use anyhow::Result;
use std::{io::{BufRead, BufReader, Write}, net::{TcpListener, TcpStream}};

#[derive(Parser)]
struct CommandLineArguments {
    port: u16 // allows values 0...65535
}

fn main() {
    let args = CommandLineArguments::parse();
    start_server(args.port);
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
    let buf_reader: BufReader<&TcpStream> = BufReader::new(&stream);
    let mut lines = buf_reader.lines();

    let first_line = lines.next().ok_or_else(|| anyhow::anyhow!("No request line found"))??;
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() != 3 {
        return Err(anyhow::anyhow!("Invalid request line: {}", first_line));
    }

    let response = match send_request(parts[0], parts[1]) {
        Ok(res) => format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}", res.content_length().unwrap_or(0), res.text().unwrap()),
        Err(err) => format!("HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\n\r\n{}", err.to_string().len(), err),
    };
    stream.write_all(response.as_bytes()).unwrap();
    stream.flush().unwrap();
    println!("Response sent for request: {}", first_line);
    Ok(())
}

fn send_request(method: &str, url: &str) -> Result<reqwest::blocking::Response, anyhow::Error> {
    // todo add support for POST body and other headers
    match method {
        "GET" => reqwest::blocking::get(url).map_err(|e| anyhow::anyhow!(e)),
        "POST" => reqwest::blocking::Client::new().post(url).send().map_err(|e| anyhow::anyhow!(e)),
        _ => Err(anyhow::anyhow!("Method not allowed")),
    }
}