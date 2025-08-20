use anyhow::Result;
use clap::Parser;
use http::Method;
use std::{
    io::{BufRead, BufReader},
    net::{TcpListener, TcpStream},
    thread,
    time::Duration,
};
use threadpool::ThreadPool;

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
    info!("Server listening on {}", addr);

    let pool = ThreadPool::new(threads.unwrap_or_else(num_cpus::get));
    info!("Using thread pool with {} threads", pool.max_count());

    for stream in listener.incoming() {
        let stream = stream?;
        let peer_addr = stream.peer_addr()?;

        pool.execute(move || {
            debug!("Connection from {}", peer_addr);
            if let Err(e) = handle_connection(stream) {
                error!("Error handling {}: {}", peer_addr, e);
            }
            debug!("Connection closed: {}", peer_addr);
        });
    }

    pool.join();

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
