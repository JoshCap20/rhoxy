pub const BAD_GATEWAY_RESPONSE_HEADER: &str =
    "HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain\r\n\r\n";
pub const CONNECTION_ESTABLISHED_RESPONSE: &[u8] = b"HTTP/1.1 200 Connection Established\r\n\r\n";

pub const HEALTH_ENDPOINT_PATH: &str = "/health";
pub const HEALTH_CHECK_RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK";
