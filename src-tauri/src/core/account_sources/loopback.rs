//! A browser sign-in's way back: the vendor sends the browser to a port on
//! this machine with the code in its query, and this answers with a short
//! page saying how it went.
use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

/// The query of a `GET <path>` request, `None` for anything else.
async fn read_query(socket: &mut TcpStream, path: &str) -> Option<String> {
    let mut buffer = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    while !buffer.windows(4).any(|window| window == b"\r\n\r\n") && buffer.len() < 16 * 1024 {
        let read = tokio::time::timeout(Duration::from_secs(10), socket.read(&mut chunk)).await.ok()?.ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let head = String::from_utf8_lossy(&buffer);
    let target = head.lines().next()?.strip_prefix("GET ")?.split(' ').next()?;
    let (found, query) = target.split_once('?').unwrap_or((target, ""));
    (found == path).then(|| query.to_owned())
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Answer the browser with a page.
pub async fn reply(socket: &mut TcpStream, ok: bool, detail: &str) {
    let title = if ok { "Signed in" } else { "Sign-in didn't finish" };
    let page = format!("<!doctype html><meta charset=utf-8><title>AgentHarbor</title><h1>{}</h1><p>{}</p>", escape(title), escape(detail));
    let _ = socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
        page.len()).as_bytes()).await;
}

/// Wait for the browser at `path` with this sign-in's `state`; the socket is
/// kept so the caller can answer once the code is traded.
pub async fn wait(
    listener: &TcpListener,
    path: &str,
    state: &str,
    window: Duration,
    mut cancel: watch::Receiver<bool>,
) -> Result<(HashMap<String, String>, TcpStream), String> {
    let accept = async {
        loop {
            let (mut socket, _) = listener.accept().await.map_err(|_| "accounts.loginFailed".to_owned())?;
            let Some(query) = read_query(&mut socket, path).await else {
                let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                continue;
            };
            let params: HashMap<String, String> = reqwest::Url::parse(&format!("http://localhost/?{query}"))
                .map(|url| url.query_pairs().into_owned().collect()).unwrap_or_default();
            if params.get("state").map(String::as_str) != Some(state) {
                reply(&mut socket, false, "This sign-in was not started here.").await;
                return Err("accounts.loginFailed".to_owned());
            }
            if let Some(error) = params.get("error") {
                let detail = params.get("error_description").unwrap_or(error).clone();
                reply(&mut socket, false, &detail).await;
                return Err(if error == "access_denied" { "accounts.loginCancelled".to_owned() } else { format!("sign-in: {detail}") });
            }
            return Ok((params, socket));
        }
    };
    tokio::select! {
        biased;
        _ = super::cancelled(&mut cancel) => Err("accounts.loginCancelled".into()),
        result = tokio::time::timeout(window, accept) => result.unwrap_or_else(|_| Err("accounts.loginTimeout".into())),
    }
}

/// The path and query of a `GET` request, for a sign-in that comes back to
/// more than one path.
pub async fn read_request(socket: &mut TcpStream) -> Option<(String, HashMap<String, String>)> {
    let mut buffer = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    while !buffer.windows(4).any(|window| window == b"\r\n\r\n") && buffer.len() < 16 * 1024 {
        let read = tokio::time::timeout(Duration::from_secs(10), socket.read(&mut chunk)).await.ok()?.ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let head = String::from_utf8_lossy(&buffer);
    let target = head.lines().next()?.strip_prefix("GET ")?.split(' ').next()?.to_owned();
    let url = reqwest::Url::parse(&format!("http://localhost{target}")).ok()?;
    Some((url.path().to_owned(), url.query_pairs().into_owned().collect()))
}

/// Send the browser on to `url`.
pub async fn redirect(socket: &mut TcpStream, url: &str) {
    let _ = socket.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await;
}

/// Turn away a request this sign-in does not answer.
pub async fn not_found(socket: &mut TcpStream) {
    let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
}

/// One HTTP request: method, path, headers and body.
pub struct Incoming {
    pub method: String,
    pub path: String,
    pub query: std::collections::HashMap<String, String>,
    headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Incoming {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }
}

/// Read one request off a socket (a body of up to 64 KiB).
pub async fn read_incoming(socket: &mut TcpStream) -> Option<Incoming> {
    let mut buffer = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break end + 4;
        }
        if buffer.len() > 32 * 1024 {
            return None;
        }
        let read = tokio::time::timeout(Duration::from_secs(10), socket.read(&mut chunk)).await.ok()?.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let mut lines = head.lines();
    let mut first = lines.next()?.split(' ');
    let (method, target) = (first.next()?.to_owned(), first.next()?.to_owned());
    let headers: Vec<(String, String)> = lines.filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned())).collect();
    let length: usize = headers.iter().find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse().ok()).unwrap_or(0).min(64 * 1024);
    let mut body = buffer[head_end..].to_vec();
    while body.len() < length {
        let read = tokio::time::timeout(Duration::from_secs(10), socket.read(&mut chunk)).await.ok()?.ok()?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    let url = reqwest::Url::parse(&format!("http://127.0.0.1{target}")).ok()?;
    Some(Incoming { method, path: url.path().to_owned(), query: url.query_pairs().into_owned().collect(), headers, body })
}

/// Answer with a status, headers and a body, closing the connection.
pub async fn respond(socket: &mut TcpStream, status: &str, headers: &[(&str, String)], body: &str) {
    let mut head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    let _ = socket.write_all(head.as_bytes()).await;
    let _ = socket.write_all(body.as_bytes()).await;
}

/// A short page saying how a sign-in went.
pub fn page(ok: bool, title: &str, text: &str) -> String {
    let escape = |text: &str| text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    format!("<!doctype html><meta charset=utf-8><title>{}</title><main style=\"font:15px system-ui;text-align:center;margin-top:20vh\">\
        <div style=\"font-size:2rem;color:{}\">{}</div><h1>{}</h1><p>{}</p></main>",
        escape(title), if ok { "#2a9d5c" } else { "#c0392b" }, if ok { "✓" } else { "✕" }, escape(title), escape(text))
}
