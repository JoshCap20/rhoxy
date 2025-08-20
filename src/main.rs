use clap::Parser;
use http::{Request, Response};
use std::{io::{BufRead, BufReader}, net::{TcpListener, TcpStream}};

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

/*
HTTP Request Format:
Method Request-URI HTTP-Version CRLF
headers CRLF
message-body
*/

fn handle_connection(stream: TcpStream) {
    let buf_reader: BufReader<&TcpStream> = BufReader::new(&stream);
    let http_request: Vec<_> = buf_reader
        .lines()
        .map(|result| result.unwrap())
        .take_while(|line| !line.is_empty())
        .collect();

    println!("Request: {http_request:#?}");
}