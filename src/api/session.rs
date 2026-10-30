//! The session: Spotify tokens in an encrypted cookie, renewed on the way in.

use std::time::{SystemTime, UNIX_EPOCH};

use aide::transform::TransformOperation;
use axum::Json;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use cookie::{Cookie, CookieJar, Key, SameSite};
use schemars::JsonSchema;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use super::error::{AppError, ProblemResponse};
use super::{AppState, SESSION_SCHEME, TAG_AUTHENTICATION};
use crate::auth::{Accounts, AccountsError, Grant};

pub const SESSION_COOKIE: &str = "spoticlean_session";
const LOGIN_COOKIE: &str = "spoticlean_login";
const LOGIN_COOKIE_PATH: &str = "/auth";

/// Renewed with the access token, so it only lapses after a month of disuse.
const SESSION_LIFETIME: cookie::time::Duration = cookie::time::Duration::days(30);
const LOGIN_LIFETIME: cookie::time::Duration = cookie::time::Duration::minutes(10);

/// An access token expiring within this many seconds is renewed first, so
/// that it outlives the request it is used for.
const EXPIRY_MARGIN_SECS: u64 = 300;

/// The user's Spotify tokens.
#[derive(Clone)]
pub struct Session {
    pub access: SecretString,
    pub refresh: Option<SecretString>,
    /// Unix time, in seconds.
    pub expires_at: u64,
    pub scope: String,
}

impl Session {
    /// Spotify may not rotate the refresh token, in which case the previous
    /// one stays valid and is kept, and may omit the scope when it is
    /// unchanged.
    pub fn from_grant(grant: Grant, previous: Option<&Self>, now: u64) -> Self {
        Self {
            access: grant.access,
            refresh: grant
                .refresh
                .or_else(|| previous.and_then(|session| session.refresh.clone())),
            expires_at: now + grant.expires_in,
            scope: if grant.scope.is_empty() {
                previous
                    .map(|session| session.scope.clone())
                    .unwrap_or_default()
            } else {
                grant.scope
            },
        }
    }
}

#[derive(Deserialize)]
struct StoredSession {
    access: SecretString,
    refresh: Option<SecretString>,
    expires_at: u64,
    scope: String,
}

#[derive(Serialize)]
struct StoredSessionRef<'a> {
    access: &'a str,
    refresh: Option<&'a str>,
    expires_at: u64,
    scope: &'a str,
}

/// What `/auth/callback` needs from the `/auth/login` that sent the user away.
pub struct LoginAttempt {
    pub verifier: SecretString,
    pub state: SecretString,
}

#[derive(Deserialize)]
struct StoredLogin {
    verifier: SecretString,
    state: SecretString,
}

#[derive(Serialize)]
struct StoredLoginRef<'a> {
    verifier: &'a str,
    state: &'a str,
}

/// Seals and opens the cookies: AES-256-GCM with the cookie name as
/// associated data, so a cookie can be neither read, altered, nor passed off
/// under another name.
pub struct CookieCodec {
    key: Key,
    secure: bool,
}

impl CookieCodec {
    pub fn new(key: Key, secure: bool) -> Self {
        Self { key, secure }
    }

    pub fn session(&self, session: &Session) -> HeaderValue {
        let stored = StoredSessionRef {
            access: session.access.expose_secret(),
            refresh: session.refresh.as_ref().map(ExposeSecret::expose_secret),
            expires_at: session.expires_at,
            scope: &session.scope,
        };
        self.seal(SESSION_COOKIE, "/", &stored, SESSION_LIFETIME)
    }

    pub fn clear_session(&self) -> HeaderValue {
        self.removal(SESSION_COOKIE, "/")
    }

    pub fn login(&self, attempt: &LoginAttempt) -> HeaderValue {
        let stored = StoredLoginRef {
            verifier: attempt.verifier.expose_secret(),
            state: attempt.state.expose_secret(),
        };
        self.seal(LOGIN_COOKIE, LOGIN_COOKIE_PATH, &stored, LOGIN_LIFETIME)
    }

    pub fn clear_login(&self) -> HeaderValue {
        self.removal(LOGIN_COOKIE, LOGIN_COOKIE_PATH)
    }

    pub fn read_session(&self, headers: &HeaderMap) -> Option<Session> {
        let stored: StoredSession = self.open(headers, SESSION_COOKIE)?;
        Some(Session {
            access: stored.access,
            refresh: stored.refresh,
            expires_at: stored.expires_at,
            scope: stored.scope,
        })
    }

    pub fn read_login(&self, headers: &HeaderMap) -> Option<LoginAttempt> {
        let stored: StoredLogin = self.open(headers, LOGIN_COOKIE)?;
        Some(LoginAttempt {
            verifier: stored.verifier,
            state: stored.state,
        })
    }

    fn base(&self, name: &'static str, value: String, path: &'static str) -> Cookie<'static> {
        Cookie::build((name, value))
            .path(path)
            .http_only(true)
            .secure(self.secure)
            .same_site(SameSite::Lax)
            .build()
    }

    fn seal(
        &self,
        name: &'static str,
        path: &'static str,
        value: &impl Serialize,
        lifetime: cookie::time::Duration,
    ) -> HeaderValue {
        let value = serde_json::to_string(value).expect("cookie payloads serialize");
        let mut cookie = self.base(name, value, path);
        cookie.set_max_age(lifetime);
        let mut jar = CookieJar::new();
        jar.private_mut(&self.key).add(cookie);
        let sealed = jar.get(name).expect("the cookie was just added");
        HeaderValue::try_from(sealed.to_string()).expect("a sealed cookie is base64")
    }

    fn removal(&self, name: &'static str, path: &'static str) -> HeaderValue {
        let mut cookie = self.base(name, String::new(), path);
        cookie.make_removal();
        HeaderValue::try_from(cookie.to_string()).expect("a removal cookie is ASCII")
    }

    fn open<T: for<'de> Deserialize<'de>>(&self, headers: &HeaderMap, name: &str) -> Option<T> {
        let mut jar = CookieJar::new();
        for value in headers.get_all(header::COOKIE) {
            let Ok(value) = value.to_str() else { continue };
            for cookie in Cookie::split_parse(value).flatten() {
                if cookie.name() == name {
                    jar.add_original(cookie.into_owned());
                }
            }
        }
        let Some(cookie) = jar.private(&self.key).get(name) else {
            if jar.get(name).is_some() {
                debug!(cookie = name, "ignoring a cookie that does not decrypt");
            }
            return None;
        };
        serde_json::from_str(cookie.value())
            .inspect_err(|error| debug!(cookie = name, %error, "ignoring an undecodable cookie"))
            .ok()
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// `session`, renewed when its access token is about to expire; the flag
/// says whether it was.
pub async fn renewed(
    session: Session,
    accounts: &impl Accounts,
    now: u64,
) -> Result<(Session, bool), AppError> {
    if session.expires_at > now + EXPIRY_MARGIN_SECS {
        return Ok((session, false));
    }
    let Some(refresh) = &session.refresh else {
        debug!("the access token expired and there is no refresh token");
        return Err(AppError::SessionExpired);
    };
    debug!("refreshing the access token");
    match accounts.refresh(refresh).await {
        Ok(grant) => Ok((Session::from_grant(grant, Some(&session), now), true)),
        Err(AccountsError::Rejected(reason)) => {
            warn!(%reason, "cannot refresh the access token");
            Err(AppError::SessionExpired)
        }
        Err(AccountsError::Unavailable(reason)) => Err(AppError::AccountsUnavailable(reason)),
    }
}

/// Lets a request through only with a valid session, which the handler then
/// finds in the request extensions. A renewed session is sent back in the
/// response; one that cannot be renewed is cleared.
pub async fn require<L, A>(
    State(state): State<AppState<L, A>>,
    mut request: Request,
    next: Next,
) -> Response
where
    L: Send + Sync + 'static,
    A: Accounts + Send + Sync + 'static,
{
    let cookies = state.cookies();
    let Some(session) = cookies.read_session(request.headers()) else {
        return AppError::Unauthenticated.into_response();
    };
    match renewed(session, state.accounts(), unix_now()).await {
        Ok((session, renewed)) => {
            let cookie = renewed.then(|| cookies.session(&session));
            request.extensions_mut().insert(session);
            let mut response = next.run(request).await;
            if let Some(cookie) = cookie {
                response.headers_mut().append(header::SET_COOKIE, cookie);
            }
            response
        }
        Err(error @ AppError::SessionExpired) => {
            let mut response = error.into_response();
            response
                .headers_mut()
                .append(header::SET_COOKIE, cookies.clear_session());
            response
        }
        Err(error) => error.into_response(),
    }
}

/// The session as the UI sees it.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(extend("example" = serde_json::json!({
    "scopes": ["user-library-read", "user-library-modify"],
})))]
pub struct SessionResponse {
    /// The Spotify scopes the user granted.
    pub scopes: Vec<String>,
}

pub async fn get(axum::Extension(session): axum::Extension<Session>) -> Json<SessionResponse> {
    Json(SessionResponse {
        scopes: session
            .scope
            .split_whitespace()
            .map(str::to_owned)
            .collect(),
    })
}

pub fn get_docs(op: TransformOperation<'_>) -> TransformOperation<'_> {
    op.tag(TAG_AUTHENTICATION)
        .summary("Read the current session")
        .description(
            "Answers 200 while the user is logged in, renewing the Spotify \
             access token when it is about to expire, and 401 otherwise.",
        )
        .security_requirement(SESSION_SCHEME)
        .response::<200, Json<SessionResponse>>()
        .response_with::<401, Json<ProblemResponse>, _>(|res| {
            res.description("No session, or one that cannot be renewed")
        })
}

pub async fn delete<L, A>(
    State(state): State<AppState<L, A>>,
) -> ([(header::HeaderName, HeaderValue); 1], StatusCode) {
    (
        [(header::SET_COOKIE, state.cookies().clear_session())],
        StatusCode::NO_CONTENT,
    )
}

pub fn delete_docs(op: TransformOperation<'_>) -> TransformOperation<'_> {
    op.tag(TAG_AUTHENTICATION)
        .summary("Log out")
        .description("Clears the session cookie. Answers 204 whether or not there was one.")
        .response_with::<204, (), _>(|res| res.description("Logged out"))
}
