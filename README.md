# rhoxy - Rust HTTP/HTTPS Proxy
[![Tests](https://github.com/JoshCap20/rhoxy/actions/workflows/rust.yml/badge.svg?branch=main)](https://github.com/JoshCap20/rhoxy/actions/workflows/rust.yml)

Simple HTTP/HTTPS proxy in Rust (my inaugural rust project)

## Running

Arguments:

```
#[arg(long, default_value = "127.0.0.1", help = "Host to listen on")]
host: String,

#[arg(short, long, default_value = "8080", help = "Port to listen on")]
port: u16, // allows values 0...65535

#[arg(short, long, help = "Number of worker threads (default: CPU count)")]
threads: Option<usize>,

#[arg(long, help = "Enable debug logging")]
verbose: bool,
```

### Development

```bash
# listen on port 8081 on host 127.0.0.1 with 20 worker threads and debug logging
cargo run -- --port 8081 --threads 20 --verbose

# use defaults (port 8080, CPU thread count, no verbose, host 127.0.0.1)
cargo run --
```

### Build

```bash
cargo build --release
cargo install --path .
rhoxy --port 8080
```

### TODO
- Handle IPv6 properly
- MITM proxy mode