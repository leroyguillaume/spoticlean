//! The browser side of the Authorization Code flow with PKCE.

use aide::transform::TransformOperation;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::Redirect;
use schemars::JsonSchema;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use tracing::{info, warn};

use super::error::AppError;
use super::session::{LoginAttempt, Session, unix_now};
use super::{AppState, TAG_AUTHENTICATION};
use crate::auth::{Accounts, AccountsError, pkce};

pub const LOGIN_PATH: &str = "/auth/login";
pub const CALLBACK_PATH: &str = "/auth/callback";

pub async fn login<L, A>(
    State(state): State<AppState<L, A>>,
) -> Result<([(header::HeaderName, HeaderValue); 1], Redirect), AppError> {
    let random = |bytes| {
        pkce::random_string(bytes)
            .map_err(|error| AppError::Internal(format!("cannot draw random bytes: {error}")))
    };
    let attempt = LoginAttempt {
        verifier: random(64)?,
        state: random(16)?,
    };
    let settings = state.settings();
    let url = pkce::authorize_url(
        &settings.client_id,
        &settings.redirect_uri(),
        &pkce::challenge(&attempt.verifier),
        attempt.state.expose_secret(),
    );
    Ok((
        [(header::SET_COOKIE, state.cookies().login(&attempt))],
        Redirect::to(url.as_str()),
    ))
}

pub fn login_docs(op: TransformOperation<'_>) -> TransformOperation<'_> {
    op.tag(TAG_AUTHENTICATION)
        .summary("Log in with Spotify")
        .description(
            "A page the browser navigates to, not an API call: redirects to the \
             Spotify consent page, which sends the user back to \
             `/auth/callback`.",
        )
        .response_with::<303, (), _>(|res| res.description("Redirect to the Spotify consent page"))
}

/// What Spotify appends to the redirect URI. Holds the authorization code:
/// never logged.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CallbackQuery {
    /// The authorization code, when the user accepted.
    pub code: Option<String>,
    /// Echo of the `state` sent to Spotify.
    pub state: Option<String>,
    /// Why there is no code, e.g. `access_denied`.
    pub error: Option<String>,
}

/// Why a login did not go through, passed to the UI as `/?authError=<code>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoginFailure {
    /// The login cookie is missing, expired, or holds another `state`.
    InvalidState,
    /// The user refused the consent.
    AccessDenied,
    /// Spotify answered with an error, or refused the code.
    LoginFailed,
    SpotifyUnavailable,
}

impl LoginFailure {
    fn code(self) -> &'static str {
        match self {
            Self::InvalidState => "invalidState",
            Self::AccessDenied => "accessDenied",
            Self::LoginFailed => "loginFailed",
            Self::SpotifyUnavailable => "spotifyUnavailable",
        }
    }
}

pub async fn callback<L, A>(
    State(state): State<AppState<L, A>>,
    request_headers: HeaderMap,
    Query(query): Query<CallbackQuery>,
) -> (HeaderMap, Redirect)
where
    A: Accounts,
{
    let cookies = state.cookies();
    let mut headers = HeaderMap::new();
    headers.append(header::SET_COOKIE, cookies.clear_login());
    match complete_login(&state, &request_headers, query).await {
        Ok(session) => {
            info!("logged in to Spotify");
            headers.append(header::SET_COOKIE, cookies.session(&session));
            (headers, Redirect::to("/"))
        }
        Err(failure) => {
            warn!(reason = failure.code(), "login failed");
            (
                headers,
                Redirect::to(&format!("/?authError={}", failure.code())),
            )
        }
    }
}

async fn complete_login<L, A: Accounts>(
    state: &AppState<L, A>,
    request_headers: &HeaderMap,
    query: CallbackQuery,
) -> Result<Session, LoginFailure> {
    let attempt = state
        .cookies()
        .read_login(request_headers)
        .ok_or(LoginFailure::InvalidState)?;
    if query.state.as_deref() != Some(attempt.state.expose_secret()) {
        return Err(LoginFailure::InvalidState);
    }
    if let Some(error) = query.error {
        warn!(%error, "Spotify did not grant access");
        return Err(if error == "access_denied" {
            LoginFailure::AccessDenied
        } else {
            LoginFailure::LoginFailed
        });
    }
    let code = SecretString::from(query.code.ok_or(LoginFailure::LoginFailed)?);
    let grant = state
        .accounts()
        .exchange_code(&code, &attempt.verifier, &state.settings().redirect_uri())
        .await
        .map_err(|error| {
            warn!(%error, "cannot exchange the authorization code");
            match error {
                AccountsError::Rejected(_) => LoginFailure::LoginFailed,
                AccountsError::Unavailable(_) => LoginFailure::SpotifyUnavailable,
            }
        })?;
    Ok(Session::from_grant(grant, None, unix_now()))
}

pub fn callback_docs(op: TransformOperation<'_>) -> TransformOperation<'_> {
    op.tag(TAG_AUTHENTICATION)
        .summary("Complete the Spotify login")
        .description(
            "Spotify's redirect target. Checks `state` against the login \
             cookie, exchanges the code, sets the session cookie, and \
             redirects to `/`. On failure, redirects to `/?authError=<code>`, \
             `<code>` being `invalidState`, `accessDenied`, `loginFailed` or \
             `spotifyUnavailable`.",
        )
        .response_with::<303, (), _>(|res| res.description("Redirect to the UI"))
}
