//! Spotify user authorization: Authorization Code with PKCE, cached tokens.

mod accounts;
mod cache;
mod callback;
mod pkce;

use secrecy::SecretString;
use tracing::{debug, info, warn};

pub use accounts::SpotifyAccounts;
pub use cache::TokenCache;

/// Must match, byte for byte, the redirect URI registered for the app in the
/// Spotify developer dashboard.
pub const REDIRECT_URI: &str = "http://127.0.0.1:8888/auth/callback";
const CALLBACK_ADDR: &str = "127.0.0.1:8888";
pub const SCOPE: &str = "user-library-read";

/// A cached token is refreshed when it expires within this many seconds, so
/// that it outlives a long fetch.
const EXPIRY_MARGIN_SECS: u64 = 300;

/// Tokens as stored in the cache.
pub struct Token {
    pub access: SecretString,
    pub refresh: Option<SecretString>,
    /// Unix time, in seconds.
    pub expires_at: u64,
    pub scope: String,
}

impl Token {
    fn grants(&self, scope: &str) -> bool {
        self.scope
            .split_whitespace()
            .any(|granted| granted == scope)
    }
}

/// What the token endpoint answers.
pub struct Grant {
    pub access: SecretString,
    pub refresh: Option<SecretString>,
    pub expires_in: u64,
    pub scope: String,
}

/// The Spotify accounts service.
#[cfg_attr(test, mockall::automock)]
// automock copies this onto the mock, whose `async fn`s never await.
#[cfg_attr(test, allow(clippy::unused_async_trait_impl))]
pub trait Accounts {
    /// Runs the browser flow and exchanges the code it yields.
    async fn login(&self) -> anyhow::Result<Grant>;
    async fn refresh(&self, refresh_token: &SecretString) -> anyhow::Result<Grant>;
}

/// Returns an access token valid for a while, refreshing the cached one when
/// possible and falling back to the browser login otherwise.
pub async fn access_token(
    cache: &TokenCache,
    accounts: &impl Accounts,
    now: u64,
) -> anyhow::Result<SecretString> {
    let cached = cache.load().unwrap_or_else(|error| {
        warn!(error = format!("{error:#}"), "ignoring the token cache");
        None
    });
    if let Some(token) = cached.filter(|token| token.grants(SCOPE)) {
        if token.expires_at > now + EXPIRY_MARGIN_SECS {
            debug!("using the cached access token");
            return Ok(token.access);
        }
        if let Some(refresh) = token.refresh {
            debug!("refreshing the access token");
            match accounts.refresh(&refresh).await {
                Ok(grant) => return store(cache, grant, Some(refresh), now),
                Err(error) => warn!(
                    error = format!("{error:#}"),
                    "cannot refresh the access token, logging in again"
                ),
            }
        }
    }
    let grant = accounts.login().await?;
    info!("logged in to Spotify");
    store(cache, grant, None, now)
}

/// Spotify may not rotate the refresh token, in which case the previous one
/// stays valid and is kept.
fn store(
    cache: &TokenCache,
    grant: Grant,
    previous_refresh: Option<SecretString>,
    now: u64,
) -> anyhow::Result<SecretString> {
    let token = Token {
        access: grant.access,
        refresh: grant.refresh.or(previous_refresh),
        expires_at: now + grant.expires_in,
        scope: grant.scope,
    };
    cache.save(&token)?;
    Ok(token.access)
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    const NOW: u64 = 1_700_000_000;

    fn grant(access: &str, refresh: Option<&str>) -> Grant {
        Grant {
            access: access.into(),
            refresh: refresh.map(SecretString::from),
            expires_in: 3600,
            scope: SCOPE.to_owned(),
        }
    }

    fn cache_with(dir: &tempfile::TempDir, token: Option<Token>) -> TokenCache {
        let cache = TokenCache::new(dir.path().join("token.json"));
        if let Some(token) = token {
            cache.save(&token).unwrap();
        }
        cache
    }

    fn token(expires_at: u64, scope: &str) -> Token {
        Token {
            access: "cached-access".into(),
            refresh: Some("cached-refresh".into()),
            expires_at,
            scope: scope.to_owned(),
        }
    }

    #[tokio::test]
    async fn valid_cached_token_is_used_as_is() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache_with(&dir, Some(token(NOW + 3000, SCOPE)));
        let accounts = MockAccounts::new();
        let access = access_token(&cache, &accounts, NOW).await.unwrap();
        assert_eq!(access.expose_secret(), "cached-access");
    }

    #[tokio::test]
    async fn expiring_token_is_refreshed_and_keeps_its_refresh_token() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache_with(&dir, Some(token(NOW + 60, SCOPE)));
        let mut accounts = MockAccounts::new();
        accounts
            .expect_refresh()
            .withf(|refresh| refresh.expose_secret() == "cached-refresh")
            .times(1)
            .returning(|_| Ok(grant("new-access", None)));
        let access = access_token(&cache, &accounts, NOW).await.unwrap();
        assert_eq!(access.expose_secret(), "new-access");

        let saved = cache.load().unwrap().unwrap();
        assert_eq!(saved.access.expose_secret(), "new-access");
        assert_eq!(saved.refresh.unwrap().expose_secret(), "cached-refresh");
        assert_eq!(saved.expires_at, NOW + 3600);
    }

    #[tokio::test]
    async fn rotated_refresh_token_replaces_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache_with(&dir, Some(token(NOW - 10, SCOPE)));
        let mut accounts = MockAccounts::new();
        accounts
            .expect_refresh()
            .returning(|_| Ok(grant("new-access", Some("new-refresh"))));
        access_token(&cache, &accounts, NOW).await.unwrap();
        let saved = cache.load().unwrap().unwrap();
        assert_eq!(saved.refresh.unwrap().expose_secret(), "new-refresh");
    }

    #[tokio::test]
    async fn failed_refresh_falls_back_to_the_browser() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache_with(&dir, Some(token(NOW - 10, SCOPE)));
        let mut accounts = MockAccounts::new();
        accounts
            .expect_refresh()
            .times(1)
            .returning(|_| Err(anyhow::anyhow!("invalid_grant")));
        accounts
            .expect_login()
            .times(1)
            .returning(|| Ok(grant("login-access", Some("login-refresh"))));
        let access = access_token(&cache, &accounts, NOW).await.unwrap();
        assert_eq!(access.expose_secret(), "login-access");
        let saved = cache.load().unwrap().unwrap();
        assert_eq!(saved.refresh.unwrap().expose_secret(), "login-refresh");
    }

    #[tokio::test]
    async fn no_cache_logs_in() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache_with(&dir, None);
        let mut accounts = MockAccounts::new();
        accounts
            .expect_login()
            .times(1)
            .returning(|| Ok(grant("login-access", Some("login-refresh"))));
        let access = access_token(&cache, &accounts, NOW).await.unwrap();
        assert_eq!(access.expose_secret(), "login-access");
        assert!(cache.load().unwrap().is_some());
    }

    #[tokio::test]
    async fn token_missing_the_scope_logs_in_again() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache_with(&dir, Some(token(NOW + 3000, "playlist-read-private")));
        let mut accounts = MockAccounts::new();
        accounts
            .expect_login()
            .times(1)
            .returning(|| Ok(grant("login-access", None)));
        let access = access_token(&cache, &accounts, NOW).await.unwrap();
        assert_eq!(access.expose_secret(), "login-access");
    }

    #[tokio::test]
    async fn corrupt_cache_logs_in_and_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("token.json"), "{").unwrap();
        let cache = cache_with(&dir, None);
        let mut accounts = MockAccounts::new();
        accounts
            .expect_login()
            .times(1)
            .returning(|| Ok(grant("login-access", None)));
        access_token(&cache, &accounts, NOW).await.unwrap();
        assert!(cache.load().unwrap().is_some());
    }

    #[tokio::test]
    async fn failed_login_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache_with(&dir, None);
        let mut accounts = MockAccounts::new();
        accounts
            .expect_login()
            .returning(|| Err(anyhow::anyhow!("access_denied")));
        assert!(access_token(&cache, &accounts, NOW).await.is_err());
        assert!(cache.load().unwrap().is_none());
    }
}
