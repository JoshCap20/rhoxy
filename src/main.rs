use anyhow::Result;
use clap::Parser;
use http::Method;
use std::{
    io::{BufRead, BufReader},
    net::{TcpListener, TcpStream},
    thread,
    time::Duration,
};

#[derive(Parser)]
struct CommandLineArguments {
    port: u16, // allows values 0...65535
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
        // TODO: Handle with thread pool
        thread::spawn(|| {
            if let Err(e) = handle_connection(stream) {
                println!("Error handling connection: {}", e);
            }
            println!("Connection closed");
        });
    }
    Ok(())
}

fn handle_connection(mut stream: TcpStream) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut reader = BufReader::new(stream.try_clone()?);

    let mut first_line = String::new();
    reader.read_line(&mut first_line)?;
    let first_line = first_line.trim();

    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() != 3 {
        return Err(anyhow::anyhow!("Invalid request line: {}", first_line));
    }

    let method = Method::from_bytes(parts[0].as_bytes())
        .map_err(|e| anyhow::anyhow!("Invalid method: {}", e))?;

    if method == Method::CONNECT {
        return rhoxy::https::handle_connect_method(&mut stream, &mut reader, parts[1]);
    }

    rhoxy::handle_http_request(&mut stream, &mut reader, method, parts)?;

    Ok(())
}
