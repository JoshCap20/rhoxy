# rhoxy

[![Tests](https://github.com/JoshCap20/rhoxy/actions/workflows/test.yml/badge.svg?branch=main)](https://github.com/JoshCap20/rhoxy/actions/workflows/test.yml)
[![Publish](https://github.com/JoshCap20/rhoxy/actions/workflows/deploy.yml/badge.svg)](https://github.com/JoshCap20/rhoxy/actions/workflows/deploy.yml)
[![Crates.io](https://img.shields.io/crates/v/rhoxy.svg)](https://crates.io/crates/rhoxy)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

An async HTTP/HTTPS forward proxy built with Rust, Tokio, and reqwest.

## Features

- **HTTP forwarding** — Parses client requests, forwards to upstream servers via a static `reqwest` connection pool, and streams responses back
- **HTTPS tunneling** — Handles `CONNECT` requests with bidirectional `tokio::io::copy` tunneling
- **SSRF protection** — Blocks requests to private/loopback addresses with DNS rebinding detection
- **DoS mitigation** — Bounded line reads, body size limits (10 MiB), header count limits, connection concurrency cap (1024), and per-connection timeouts
- **Graceful shutdown** — Drains in-flight connections on `Ctrl-C` before exiting
- **Health endpoint** — Responds to `/health` requests directed at the proxy

## Usage

```
rhoxy [OPTIONS]

Options:
      --host <HOST>  Host to bind to [default: 127.0.0.1]
  -p, --port <PORT>  Port to listen on [default: 8080]
      --verbose      Enable debug logging
  -h, --help         Print help
  -V, --version      Print version
```

### Quick start

```bash
# Start proxy on port 8081 with debug logging
rhoxy --port 8081 --verbose

# Test with curl
curl -x http://127.0.0.1:8081 http://httpbin.org/ip
curl -x http://127.0.0.1:8081 https://httpbin.org/ip
```

### System proxy (macOS)

Go to **System Settings > Wi-Fi > Details > Proxies**, enable **Web Proxy (HTTP)** and **Secure Web Proxy (HTTPS)**, set server to `127.0.0.1` and port to `8081`.

## Installation

### From crates.io

```bash
cargo install rhoxy
```

### From source

```bash
git clone https://github.com/JoshCap20/rhoxy.git
cd rhoxy
cargo build --release
cargo install --path .
```

### As a library dependency

```bash
cargo add rhoxy
```

## Development

```bash
cargo run -- --port 8081 --verbose   # Run with debug logging
cargo test                            # Run all 62 tests
cargo clippy                          # Lint
cargo fmt                             # Format
```

## Architecture

```
src/
├── main.rs              # CLI, server loop, connection handling
├── lib.rs               # Shared utilities (line reader, SSRF checks, health)
├── constants.rs         # All configuration constants
└── protocol/
    ├── mod.rs           # Protocol enum and dispatch
    ├── http.rs          # HTTP forward proxy (reqwest-based)
    └── https.rs         # HTTPS CONNECT tunnel
```

**HTTP flow:** Client request → parse headers/body → SSRF check → DNS verification → forward via reqwest connection pool → stream response back

**HTTPS flow:** CONNECT request → drain headers → SSRF check → DNS verification → TCP connect to resolved address → `200 Connection Established` → bidirectional tunnel via `tokio::io::copy`

## License

[MIT](LICENSE)
