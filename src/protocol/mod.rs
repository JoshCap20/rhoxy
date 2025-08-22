pub mod http;
pub mod https;

use ::http::Method;
use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

pub enum Protocol {
    Http,
    Https,
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

    pub fn to_string(&self) -> String {
        match self {
            Protocol::Http => "HTTP".to_string(),
            Protocol::Https => "HTTPS".to_string(),
        }
    }

    pub fn get_protocol_from_method(method: &Method) -> Self {
        if method == Method::CONNECT {
            Protocol::Https
        } else {
            Protocol::Http
        }
    }
}
