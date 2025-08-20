# rhoxy - Rust HTTP/HTTPS Proxy

Simple HTTP/HTTPS proxy in Rust (my inaugural rust project)

## Running

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
