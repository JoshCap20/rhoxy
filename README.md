# rhoxy - Rust HTTP/HTTPS Proxy

Simple HTTP/HTTPS proxy in Rust (my inaugural rust project)

## Running

Arguments:

```
#[arg(short, long, default_value = "8080", help = "Port to listen on")]
port: u16, // allows values 0...65535

#[arg(short, long, help = "Number of worker threads (default: CPU count)")]
threads: Option<usize>,

#[arg(long, help = "Enable debug logging")]
verbose: bool,
```

### Development

```bash
# listen on port 8081 with 20 worker threads and debug logging
cargo run -- --port 8081 --threads 20 --verbose

# use defaults (port 8080, CPU thread count, no verbose)
cargo run --
```

### Build

```bash
cargo build --release
cargo install --path .
rhoxy --port 8080
```
