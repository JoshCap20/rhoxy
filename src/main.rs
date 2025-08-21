use anyhow::Result;
use clap::Parser;
use http::Method;
use log::{debug, error, info};
use std::{
    io::BufReader,
    net::{TcpListener, TcpStream},
    time::Duration,
};
use threadpool::ThreadPool;

#[derive(Parser)]
struct CommandLineArguments {
    #[arg(long, default_value = "127.0.0.1", help = "Host to listen on")]
    host: String,

    #[arg(short, long, default_value = "8080", help = "Port to listen on")]
    port: u16, // allows values 0...65535

    #[arg(short, long, help = "Number of worker threads (default: CPU count)")]
    threads: Option<usize>,

    #[arg(long, help = "Enable debug logging")]
    verbose: bool,
}

fn main() {
    let args = CommandLineArguments::parse();

    if args.verbose {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Debug)
            .init();
    } else {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Info)
            .init();
    }

    if let Err(e) = start_server(&args.host, args.port, args.threads) {
        error!("Server error: {}", e);
    }
}

fn start_server(host: &str, port: u16, threads: Option<usize>) -> Result<()> {
    let listener = TcpListener::bind((host, port))?;
    info!("Server listening on {}", listener.local_addr()?);

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

    let (method, url_string) = rhoxy::extract_request_parts(&mut reader)?;
    debug!("Received request: {} {}", method, url_string);

    if method == Method::CONNECT {
        return rhoxy::protocol::https::handle_connect_method(&mut stream, &mut reader, url_string);
    }
    rhoxy::protocol::http::handle_http_request(
        &mut stream,
        &mut reader,
        method,
        url_string,
    )
}
