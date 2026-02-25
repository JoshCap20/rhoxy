pub mod http;
pub mod https;

use ::http::Method;
use anyhow::Result;
use std::fmt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

pub enum Protocol {
    Http,
    Https,
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Protocol::Http => f.write_str("HTTP"),
            Protocol::Https => f.write_str("HTTPS"),
        }
    }
}

impl Protocol {
    pub async fn handle_request<W, R>(
        &self,
        writer: &mut W,
        reader: &mut R,
        method: Method,
        target: String,
    ) -> Result<()>
    where
        W: AsyncWriteExt + Unpin,
        R: AsyncBufReadExt + Unpin,
    {
        match self {
            Protocol::Http => http::handle_request(writer, reader, method, target).await,
            Protocol::Https => https::handle_request(writer, reader, target).await,
        }
    }

    pub fn from_method(method: &Method) -> Self {
        if method == Method::CONNECT {
            Protocol::Https
        } else {
            Protocol::Http
        }
    }
}
