//! Opens a plain connection to a host for a CONNECT the front does not read:
//! directly, or through the proxy Cursor used before AgentHarbor took over.
use std::io;

use base64::Engine;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use crate::host::Egress;

pub async fn open(host: &str, port: u16, egress: &Egress) -> io::Result<TcpStream> {
    let Egress::Proxy(proxy) = egress else {
        return TcpStream::connect((host, port)).await;
    };
    let url = url::Url::parse(proxy).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let proxy_host = url.host_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "proxy has no host"))?;
    let proxy_port = url.port_or_known_default().unwrap_or(1080);
    let mut stream = TcpStream::connect((proxy_host, proxy_port)).await?;
    match url.scheme() {
        "http" | "https" => http_connect(&mut stream, host, port, &url).await?,
        "socks5" | "socks5h" => socks5_connect(&mut stream, host, port, &url).await?,
        scheme => return Err(io::Error::new(io::ErrorKind::Unsupported, format!("{scheme} proxies are not supported"))),
    }
    Ok(stream)
}

async fn http_connect(stream: &mut TcpStream, host: &str, port: u16, proxy: &url::Url) -> io::Result<()> {
    let mut request = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n");
    if !proxy.username().is_empty() {
        let credentials = format!(
            "{}:{}",
            percent_decode(proxy.username()),
            percent_decode(proxy.password().unwrap_or_default())
        );
        request.push_str(&format!(
            "Proxy-Authorization: Basic {}\r\n",
            base64::engine::general_purpose::STANDARD.encode(credentials)
        ));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await?;
    let mut head = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 16 * 1024 || stream.read(&mut byte).await? == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "proxy closed the CONNECT"));
        }
        head.push(byte[0]);
    }
    let status = String::from_utf8_lossy(&head);
    let ok = status.lines().next().and_then(|line| line.split_whitespace().nth(1)) == Some("200");
    if !ok {
        return Err(io::Error::other(format!("proxy refused CONNECT: {}", status.lines().next().unwrap_or_default())));
    }
    Ok(())
}

async fn socks5_connect(stream: &mut TcpStream, host: &str, port: u16, proxy: &url::Url) -> io::Result<()> {
    let user = percent_decode(proxy.username());
    let password = percent_decode(proxy.password().unwrap_or_default());
    let with_password = !user.is_empty();
    stream.write_all(if with_password { &[5, 1, 2] } else { &[5, 1, 0] }).await?;
    let mut choice = [0u8; 2];
    stream.read_exact(&mut choice).await?;
    match choice {
        [5, 0] => {}
        [5, 2] if with_password => {
            let mut auth = vec![1, user.len() as u8];
            auth.extend_from_slice(user.as_bytes());
            auth.push(password.len() as u8);
            auth.extend_from_slice(password.as_bytes());
            stream.write_all(&auth).await?;
            let mut reply = [0u8; 2];
            stream.read_exact(&mut reply).await?;
            if reply[1] != 0 {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "SOCKS proxy refused the credentials"));
            }
        }
        _ => return Err(io::Error::other("SOCKS proxy offered no usable method")),
    }
    let mut request = vec![5, 1, 0, 3, host.len() as u8];
    request.extend_from_slice(host.as_bytes());
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request).await?;
    let mut reply = [0u8; 4];
    stream.read_exact(&mut reply).await?;
    if reply[1] != 0 {
        return Err(io::Error::other(format!("SOCKS proxy refused the connection ({})", reply[1])));
    }
    let skip = match reply[3] {
        1 => 4,
        4 => 16,
        3 => {
            let mut length = [0u8; 1];
            stream.read_exact(&mut length).await?;
            length[0] as usize
        }
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "SOCKS proxy sent a bad address")),
    };
    let mut rest = vec![0u8; skip + 2];
    stream.read_exact(&mut rest).await?;
    Ok(())
}

fn percent_decode(value: &str) -> String {
    percent_encoding::percent_decode_str(value).decode_utf8_lossy().into_owned()
}
