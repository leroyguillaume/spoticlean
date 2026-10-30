//! Spotify accounts service over HTTPS.

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use tracing::debug;

use super::{Accounts, AccountsError, Grant};

const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";

pub struct SpotifyAccounts {
    http: reqwest::Client,
    client_id: SecretString,
}

impl SpotifyAccounts {
    pub fn new(http: reqwest::Client, client_id: SecretString) -> Self {
        Self { http, client_id }
    }

    async fn request_token(&self, form: &[(&str, &str)]) -> Result<Grant, AccountsError> {
        let response = self
            .http
            .post(TOKEN_URL)
            .form(form)
            .send()
            .await
            .map_err(|error| AccountsError::Unavailable(error.without_url().to_string()))?;
        let status = response.status();
        debug!(status = status.as_u16(), "token endpoint response");
        if status.is_server_error() {
            return Err(AccountsError::Unavailable(format!("HTTP {status}")));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let reason = serde_json::from_str::<TokenErrorDto>(&body).map_or_else(
                |_| format!("HTTP {status}"),
                |error| format!("{}: {}", error.error, error.error_description),
            );
            return Err(AccountsError::Rejected(reason));
        }
        let token: TokenDto = response.json().await.map_err(|error| {
            AccountsError::Unavailable(format!(
                "undecodable token response: {}",
                error.without_url()
            ))
        })?;
        Ok(Grant {
            access: token.access_token,
            refresh: token.refresh_token,
            expires_in: token.expires_in,
            scope: token.scope,
        })
    }
}

impl Accounts for SpotifyAccounts {
    async fn exchange_code(
        &self,
        code: &SecretString,
        verifier: &SecretString,
        redirect_uri: &str,
    ) -> Result<Grant, AccountsError> {
        self.request_token(&[
            ("grant_type", "authorization_code"),
            ("code", code.expose_secret()),
            ("redirect_uri", redirect_uri),
            ("client_id", self.client_id.expose_secret()),
            ("code_verifier", verifier.expose_secret()),
        ])
        .await
    }

    async fn refresh(&self, refresh_token: &SecretString) -> Result<Grant, AccountsError> {
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
