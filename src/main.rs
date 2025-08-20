use clap::Parser;
use std::net::{TcpListener, TcpStream};

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
        let _stream: TcpStream = stream.unwrap();

        println!("Connection established!");
    }
}