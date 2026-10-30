//! Spotify accounts service: browser login and token endpoint.

use std::time::Duration;

use anyhow::{Context as _, bail};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use tokio::net::TcpListener;
use tracing::{debug, warn};

use super::{Accounts, CALLBACK_ADDR, Grant, REDIRECT_URI, callback, pkce};

const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

pub struct SpotifyAccounts {
    http: reqwest::Client,
    client_id: SecretString,
}

impl SpotifyAccounts {
    pub fn new(http: reqwest::Client, client_id: SecretString) -> Self {
        Self { http, client_id }
    }

    async fn request_token(&self, form: &[(&str, &str)]) -> anyhow::Result<Grant> {
        let response = self
            .http
            .post(TOKEN_URL)
            .form(form)
            .send()
            .await
            .map_err(reqwest::Error::without_url)
            .context("cannot reach the Spotify accounts service")?;
        let status = response.status();
        debug!(status = status.as_u16(), "token endpoint response");
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let reason = serde_json::from_str::<TokenErrorDto>(&body).map_or_else(
                |_| format!("HTTP {status}"),
                |error| format!("{}: {}", error.error, error.error_description),
            );
            bail!("the Spotify accounts service refused the request ({reason})");
        }
        let token: TokenDto = response
            .json()
            .await
            .context("cannot decode the Spotify token response")?;
        Ok(Grant {
            access: token.access_token,
            refresh: token.refresh_token,
            expires_in: token.expires_in,
            scope: token.scope,
        })
    }
}

impl Accounts for SpotifyAccounts {
    async fn login(&self) -> anyhow::Result<Grant> {
        let listener = TcpListener::bind(CALLBACK_ADDR).await.with_context(|| {
            format!(
                "cannot listen on {CALLBACK_ADDR} for the Spotify redirect; is the port in use?"
            )
        })?;
        let verifier = pkce::random_string(64)?;
        let state = pkce::random_string(16)?;
        let authorize_url = pkce::authorize_url(
            &self.client_id,
            &pkce::challenge(&verifier),
            state.expose_secret(),
        );

        let login_url = format!("http://{CALLBACK_ADDR}{}", callback::LOGIN_PATH);
        debug!(url = %login_url, "opening the browser");
        eprintln!("Log in to Spotify in your browser. If it does not open, visit {login_url}");
        if let Err(error) = open::that_detached(&login_url) {
            warn!(%error, "cannot open the browser");
        }

        let code = tokio::time::timeout(
            LOGIN_TIMEOUT,
            callback::receive_code(listener, &authorize_url, &state),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!("no answer from Spotify after {}s", LOGIN_TIMEOUT.as_secs())
        })??;
        debug!("received the authorization code");

        self.request_token(&[
            ("grant_type", "authorization_code"),
            ("code", code.expose_secret()),
            ("redirect_uri", REDIRECT_URI),
            ("client_id", self.client_id.expose_secret()),
            ("code_verifier", verifier.expose_secret()),
        ])
        .await
    }

    async fn refresh(&self, refresh_token: &SecretString) -> anyhow::Result<Grant> {
        self.request_token(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.expose_secret()),
            ("client_id", self.client_id.expose_secret()),
        ])
        .await
    }
}

#[derive(Deserialize)]
struct TokenDto {
    access_token: SecretString,
    refresh_token: Option<SecretString>,
    expires_in: u64,
    #[serde(default)]
    scope: String,
}

#[derive(Deserialize)]
struct TokenErrorDto {
    error: String,
    #[serde(default)]
    error_description: String,
}
