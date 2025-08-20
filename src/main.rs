use clap::Parser;

#[derive(Parser)]
struct CommandLineArguments {
    port: u16
}

fn main() {
    let args = CommandLineArguments::parse();
    start_server(args.port);
}

fn start_server(port: u16) {
    println!("Starting server on port {}", port);
}