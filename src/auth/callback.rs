//! Short-lived local HTTP listener that receives the OAuth redirect.

use std::sync::Arc;
use std::time::Duration;

use anyhow::bail;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tracing::debug;
use url::Url;

/// Local page that redirects to the Spotify consent page.
pub const LOGIN_PATH: &str = "/auth/login";
pub const CALLBACK_PATH: &str = "/auth/callback";

const MAX_REQUEST_HEAD: usize = 8 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(10);

enum Route {
    Login,
    Callback(Callback),
    NotFound,
}

enum Callback {
    Code(SecretString),
    Denied(String),
    StateMismatch,
    Malformed,
}

fn route(target: &str, expected_state: &str) -> Route {
    let Ok(url) = Url::parse("http://127.0.0.1").and_then(|base| base.join(target)) else {
        return Route::NotFound;
    };
    match url.path() {
        LOGIN_PATH => Route::Login,
        CALLBACK_PATH => {
            let mut code = None;
            let mut state = None;
            let mut error = None;
            for (key, value) in url.query_pairs() {
                match &*key {
                    "code" => code = Some(value.into_owned()),
                    "state" => state = Some(value.into_owned()),
                    "error" => error = Some(value.into_owned()),
                    _ => {}
                }
            }
            Route::Callback(if state.as_deref() != Some(expected_state) {
                Callback::StateMismatch
            } else if let Some(error) = error {
                Callback::Denied(error)
            } else if let Some(code) = code {
                Callback::Code(code.into())
            } else {
                Callback::Malformed
            })
        }
        _ => Route::NotFound,
    }
}

/// Serves [`LOGIN_PATH`] and [`CALLBACK_PATH`] on `listener` until Spotify
/// redirects back, then returns the authorization code.
///
/// Each connection is handled on its own task: browsers open speculative
/// connections that never send a request, and waiting on one of them would
/// hold up the real redirect.
pub async fn receive_code(
    listener: TcpListener,
    authorize_url: &Url,
    state: &SecretString,
) -> anyhow::Result<SecretString> {
    let authorize_url: Arc<str> = authorize_url.as_str().into();
    let state: Arc<str> = secrecy::ExposeSecret::expose_secret(state).into();
    let (sender, mut receiver) = mpsc::channel(1);
    let outcome = loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, peer) = accepted?;
                debug!(%peer, "callback connection");
                let sender = sender.clone();
                let authorize_url = Arc::clone(&authorize_url);
                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    if let Some(callback) = handle(stream, &authorize_url, &state).await {
                        let _ = sender.send(callback).await;
                    }
                });
            }
            Some(callback) = receiver.recv() => break callback,
        }
    };
    match outcome {
        Callback::Code(code) => Ok(code),
        Callback::Denied(error) => bail!("Spotify login was not granted: {error}"),
        Callback::StateMismatch => {
            bail!("the Spotify redirect carried an unexpected state, refusing it; log in again")
        }
        Callback::Malformed => bail!("the Spotify redirect carried neither a code nor an error"),
    }
}

async fn handle(mut stream: TcpStream, authorize_url: &str, state: &str) -> Option<Callback> {
    let Ok(Some(target)) = tokio::time::timeout(READ_TIMEOUT, read_target(&mut stream)).await
    else {
        debug!("callback connection closed without a request");
        return None;
    };
    let route = route(&target, state);
    let (response, callback) = match route {
        Route::Login => (redirect(authorize_url), None),
        Route::NotFound => (page("404 Not Found", "Not found."), None),
        Route::Callback(callback) => {
            let response = match &callback {
                Callback::Code(_) => page(
                    "200 OK",
                    "spoticlean is logged in to Spotify. You can close this tab.",
                ),
                Callback::Denied(_) => page("200 OK", "Login cancelled. You can close this tab."),
                Callback::StateMismatch | Callback::Malformed => page(
                    "400 Bad Request",
                    "Unexpected redirect, run spoticlean again.",
                ),
            };
            (response, Some(callback))
        }
    };
    if let Err(error) = stream.write_all(response.as_bytes()).await {
        debug!(%error, "cannot answer the browser");
    }
    let _ = stream.shutdown().await;
    callback
}

async fn read_target(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut buffer = [0; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut buffer).await.ok()?;
        if read == 0 || head.len() > MAX_REQUEST_HEAD {
            return None;
        }
        head.extend_from_slice(&buffer[..read]);
    }
    let head = String::from_utf8_lossy(&head);
    let mut request_line = head.lines().next()?.split_whitespace();
    match (request_line.next(), request_line.next()) {
        (Some("GET"), Some(target)) if target.starts_with('/') => Some(target.to_owned()),
        _ => None,
    }
}

fn redirect(location: &str) -> String {
    format!(
        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

fn page(status: &str, message: &str) -> String {
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>spoticlean</title></head>\
         <body><p>{message}</p></body></html>"
    );
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    const STATE: &str = "expected-state";

    #[test]
    fn route_extracts_the_code() {
        let route = route("/auth/callback?code=abc&state=expected-state", STATE);
        assert!(
            matches!(route, Route::Callback(Callback::Code(code)) if code.expose_secret() == "abc")
        );
    }

    #[test]
    fn route_rejects_a_wrong_or_missing_state() {
        for target in [
            "/auth/callback?code=abc&state=other",
            "/auth/callback?code=abc",
            "/auth/callback?error=access_denied",
        ] {
            assert!(
                matches!(
                    route(target, STATE),
                    Route::Callback(Callback::StateMismatch)
                ),
                "{target}"
            );
        }
    }

    #[test]
    fn route_reports_a_denial() {
        let route = route(
            "/auth/callback?error=access_denied&state=expected-state",
            STATE,
        );
        assert!(
            matches!(route, Route::Callback(Callback::Denied(error)) if error == "access_denied")
        );
    }

    #[test]
    fn route_without_code_nor_error_is_malformed() {
        assert!(matches!(
            route("/auth/callback?state=expected-state", STATE),
            Route::Callback(Callback::Malformed)
        ));
    }

    #[test]
    fn route_other_paths() {
        assert!(matches!(route("/auth/login", STATE), Route::Login));
        assert!(matches!(route("/favicon.ico", STATE), Route::NotFound));
    }

    async fn get(addr: std::net::SocketAddr, target: &str) -> String {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        response
    }

    #[tokio::test]
    async fn receive_code_serves_login_then_returns_the_code() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let authorize_url = Url::parse("https://accounts.spotify.com/authorize?x=1").unwrap();
        let state = SecretString::from(STATE);
        let server =
            tokio::spawn(async move { receive_code(listener, &authorize_url, &state).await });

        let idle = TcpStream::connect(addr).await.unwrap();
        let login = get(addr, "/auth/login").await;
        assert!(login.starts_with("HTTP/1.1 302 Found\r\n"));
        assert!(login.contains("\r\nLocation: https://accounts.spotify.com/authorize?x=1\r\n"));
        assert!(get(addr, "/favicon.ico").await.starts_with("HTTP/1.1 404"));
        let callback = get(addr, "/auth/callback?code=the-code&state=expected-state").await;
        assert!(callback.starts_with("HTTP/1.1 200 OK\r\n"));

        let code = server.await.unwrap().unwrap();
        assert_eq!(code.expose_secret(), "the-code");
        drop(idle);
    }

    #[tokio::test]
    async fn receive_code_fails_on_a_state_mismatch() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let authorize_url = Url::parse("https://accounts.spotify.com/authorize").unwrap();
        let state = SecretString::from(STATE);
        let server =
            tokio::spawn(async move { receive_code(listener, &authorize_url, &state).await });

        let response = get(addr, "/auth/callback?code=the-code&state=forged").await;
        assert!(response.starts_with("HTTP/1.1 400"));
        let error = server.await.unwrap().unwrap_err();
        assert!(error.to_string().contains("unexpected state"));
    }
}
