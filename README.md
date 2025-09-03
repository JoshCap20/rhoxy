# rhoxy - Rust HTTP/HTTPS Proxy
[![Tests](https://github.com/JoshCap20/rhoxy/actions/workflows/test.yml/badge.svg?branch=main)](https://github.com/JoshCap20/rhoxy/actions/workflows/test.yml)
[![Publish](https://github.com/JoshCap20/rhoxy/actions/workflows/deploy.yml/badge.svg)](https://github.com/JoshCap20/rhoxy/actions/workflows/deploy.yml)

An async HTTP/HTTPS proxy in Rust

## Running

Arguments:

```
#[arg(long, default_value = "127.0.0.1", help = "Host to listen on")]
host: String,

#[arg(short, long, default_value = "8080", help = "Port to listen on")]
port: u16, // allows values 0...65535

#[arg(long, help = "Enable debug logging")]
verbose: bool,
```

Once installed and built, run `rhoxy --port 8081` to run the HTTP/HTTPS proxy locally on port 8081. 

If you were using mac, you would enable system usage in the wifi settings with `localhost` or `127.0.0.1` and port `8081` to serve traffic through this proxy.

## Install

```bash
cargo install rhoxy
```
**Running the above command will globally install the rhoxy binary.**

### Install as library

Run the following Cargo command in your project directory:

```bash
cargo add rhoxy
```

Or add the following line to your Cargo.toml:

```
rhoxy = "0.2.6"
```

### Source Install

### Development

```bash
# listen on port 8081 on host 127.0.0.1 with debug logging
cargo run -- --port 8081 --verbose
```

### Build

```bash
cargo build --release
cargo install --path .
rhoxy --port 8080
```

### TODO
- Authentication
- Access logging
- Rate limiting
