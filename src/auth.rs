//! Spotify user authorization: Authorization Code with PKCE.

mod accounts;
pub mod pkce;

use std::future::Future;

use secrecy::SecretString;

pub use accounts::SpotifyAccounts;

/// Read the Liked Songs, and remove tracks from them.
pub const SCOPES: &str = "user-library-read user-library-modify";

/// What the token endpoint answers.
pub struct Grant {
    pub access: SecretString,
    pub refresh: Option<SecretString>,
    pub expires_in: u64,
    pub scope: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AccountsError {
    /// The code or the refresh token is invalid, expired or revoked: the user
    /// has to log in again.
    #[error("the Spotify accounts service refused the request ({0})")]
    Rejected(String),
    #[error("cannot reach the Spotify accounts service: {0}")]
    Unavailable(String),
}

/// The Spotify accounts service's token endpoint.
#[cfg_attr(test, mockall::automock)]
pub trait Accounts {
    /// Exchanges the authorization code the user was redirected back with.
    fn exchange_code(
        &self,
        code: &SecretString,
        verifier: &SecretString,
        redirect_uri: &str,
    ) -> impl Future<Output = Result<Grant, AccountsError>> + Send;

    fn refresh(
        &self,
        refresh_token: &SecretString,
    ) -> impl Future<Output = Result<Grant, AccountsError>> + Send;
}
