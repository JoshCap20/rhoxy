use anyhow::Result;
use clap::Parser;
use http::Method;
use log::{debug, error, info};
use tokio::io::{BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};

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
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Debug)
            .init();
    } else {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Info)
            .init();
    }

    start_server(&args.host, args.port).await
}

async fn start_server(host: &str, port: u16) -> Result<()> {
    let listener = TcpListener::bind((host, port)).await?;
    info!("Server listening on {}", listener.local_addr()?);

    loop {
        match listener.accept().await {
            Ok((stream, peer_addr)) => {
                debug!("Connection from {}", peer_addr);

                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream).await {
                        error!("Error handling {}: {}", peer_addr, e);
                    }
                    debug!("Connection closed: {}", peer_addr);
                });
            }
            Err(e) => {
                error!("Failed to accept connection: {}", e);
            }
        }
    }
}

async fn handle_connection(stream: TcpStream) -> Result<()> {
    let (reader, writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut writer = BufWriter::new(writer);

    let (method, url_string) = rhoxy::extract_request_parts(&mut reader).await?;
    debug!("Received request: {} {}", method, url_string);

    if url_string == rhoxy::constants::HEALTH_ENDPOINT_PATH {
        rhoxy::handle_health_check(&mut writer).await
    } else if method == Method::CONNECT {
        rhoxy::protocol::https::handle_connect_method(&mut writer, &mut reader, url_string).await
    } else {
        rhoxy::protocol::http::handle_http_request(&mut writer, &mut reader, method, url_string)
            .await
    }
}
