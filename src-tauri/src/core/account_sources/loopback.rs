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
