use clap::Parser;
use anyhow::Result;
use reqwest::blocking::Response;
use std::{io::{BufRead, BufReader, Write}, net::{TcpListener, TcpStream}};

#[derive(Parser)]
struct CommandLineArguments {
    port: u16 // allows values 0...65535
}

fn main() {
    let args = CommandLineArguments::parse();
    start_server(args.port);
}

fn start_server(port: u16) {
    let addr: String = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&addr).unwrap();
    println!("Server listening on {}", &addr);

    for stream in listener.incoming() {
        let stream: TcpStream = stream.unwrap();

        println!("Connection established from {}", stream.peer_addr().unwrap());
        handle_connection(stream);
        println!("Connection closed.");
    }
}

fn handle_connection(mut stream: TcpStream) {
    let buf_reader: BufReader<&TcpStream> = BufReader::new(&stream);
    let http_request: Vec<_> = buf_reader
        .lines()
        .map(|result| result.unwrap())
        .take_while(|line| !line.is_empty())
        .collect();

    let response = match send_request() {
        Ok(res) => format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}", res.content_length().unwrap_or(0), res.text().unwrap()),
        Err(err) => format!("HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\n\r\n{}", err.to_string().len(), err),
    };
    stream.write_all(response.as_bytes()).unwrap();
}

fn send_request() -> Result<Response> {
    let res = reqwest::blocking::get("http://example.com")?;
    Ok(res)
}