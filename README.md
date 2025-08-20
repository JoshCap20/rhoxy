Trying to learn Rust by creating a HTTP proxy server (hopefully will extend to HTTPS after). SOCKS would be cooler but I don't want to kill myself in the process.

Testing on port 8081

GET request
```bash
curl -x localhost:8081 http://example.com/  
```
for post add `-d "test"`