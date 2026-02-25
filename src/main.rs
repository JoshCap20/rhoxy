use anyhow::Result;
use clap::Parser;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tracing::{debug, error, info, warn};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct CommandLineArguments {
    #[arg(long, default_value = "127.0.0.1", help = "Host to bind to")]
    host: String,

    #[arg(short, long, default_value = "8080", help = "Port to listen on")]
    port: u16,

    #[arg(long, help = "Enable debug logging")]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = CommandLineArguments::parse();

    if args.verbose {
        tracing_subscriber::fmt()
            .with_env_filter("rhoxy=debug")
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter("rhoxy=info")
            .init();
    }

    start_server(&args.host, args.port).await
}

async fn start_server(host: &str, port: u16) -> Result<()> {
    let listener = TcpListener::bind((host, port)).await?;
    info!("Server listening on {}", listener.local_addr()?);

    let semaphore = Arc::new(Semaphore::new(rhoxy::constants::MAX_CONCURRENT_CONNECTIONS));

    loop {
        match listener.accept().await {
            Ok((stream, peer_addr)) => {
                let permit = match semaphore.clone().try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => {
                        warn!("[{peer_addr}] Connection rejected: max connections reached");
                        drop(stream);
                        continue;
                    }
                };

                debug!("[{peer_addr}] Connection established");

                tokio::spawn(async move {
                    let _permit = permit;
                    let timeout = Duration::from_secs(rhoxy::constants::CONNECTION_TIMEOUT_SECS);
                    match tokio::time::timeout(timeout, handle_connection(stream, peer_addr)).await {
                        Ok(Err(e)) => error!("[{peer_addr}] Error handling request: {}", e),
                        Err(_) => warn!("[{peer_addr}] Connection timed out"),
                        Ok(Ok(())) => {}
                    }
                    debug!("[{peer_addr}] Connection closed");
                });
            }
            Err(e) => {
                error!("Failed to accept connection: {}", e);
            }
        }
    }
}

async fn handle_connection(stream: TcpStream, peer_addr: std::net::SocketAddr) -> Result<()> {
    let (reader, writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut writer = BufWriter::new(writer);

    let (method, url_string) = rhoxy::extract_request_parts(&mut reader).await?;

    let protocol = rhoxy::protocol::Protocol::get_protocol_from_method(&method);

    info!("[{peer_addr}::{}] {url_string}", protocol.display());

    if url_string == rhoxy::constants::HEALTH_ENDPOINT_PATH {
        return rhoxy::handle_health_check(&mut writer).await;
    }

    protocol
        .handle_request(&mut writer, &mut reader, method, url_string)
        .await?;

    Ok(())
}
