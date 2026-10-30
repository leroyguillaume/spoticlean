//! The API's single error shape: RFC 9457 problem details.

use std::error::Error as _;
use std::time::Duration;

use aide::OperationOutput;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use schemars::JsonSchema;
use serde::Serialize;

use crate::spotify::ApiError;
use crate::spotify::fetch::FetchError;
use crate::spotify::retry::RetryError;

/// What every failed request answers, as `application/problem+json`.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(extend("example" = serde_json::json!({
    "type": "urn:spoticlean:problem:unauthenticated",
    "title": "Unauthorized",
    "status": 401,
    "detail": "no valid session; log in first",
})))]
pub struct ProblemResponse {
    /// Stable identifier of the kind of problem, `urn:spoticlean:problem:<code>`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The HTTP status phrase.
    pub title: String,
    pub status: u16,
    /// What went wrong, for a human.
    pub detail: String,
}

/// What a failed Spotify call means for the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpotifyFailure {
    /// Throttled for longer than the retry policy waits.
    RateLimited,
    /// The access token is rejected.
    Unauthorized,
    /// Access is refused: a user outside the app's allowlist, a missing scope.
    Forbidden,
    /// Spotify refused this request; sending it again changes nothing.
    Rejected,
    /// Spotify kept failing, or could not be reached.
    Unavailable,
}

impl SpotifyFailure {
    pub fn of(error: &RetryError) -> Self {
        match error {
            RetryError::RetryAfterTooLong { .. }
            | RetryError::GaveUp {
                source: ApiError::RateLimited { .. },
                ..
            } => Self::RateLimited,
            RetryError::Permanent(ApiError::Unauthorized) => Self::Unauthorized,
            RetryError::Permanent(ApiError::Forbidden(_)) => Self::Forbidden,
            RetryError::Permanent(_) => Self::Rejected,
            RetryError::GaveUp { .. } => Self::Unavailable,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("no valid session; log in first")]
    Unauthenticated,
    #[error("the Spotify session cannot be renewed; log in again")]
    SessionExpired,
    #[error("{detail}")]
    InvalidRequest { status: StatusCode, detail: String },
    #[error("no such resource")]
    NotFound,
    #[error(transparent)]
    Spotify(#[from] RetryError),
    #[error("cannot reach the Spotify accounts service: {0}")]
    AccountsUnavailable(String),
    #[error("{0}")]
    Internal(String),
}

impl From<FetchError> for AppError {
    fn from(error: FetchError) -> Self {
        Self::Spotify(error.source)
    }
}

impl From<validator::ValidationErrors> for AppError {
    fn from(errors: validator::ValidationErrors) -> Self {
        Self::InvalidRequest {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            detail: errors.to_string(),
        }
    }
}

impl AppError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Unauthenticated | Self::SessionExpired => StatusCode::UNAUTHORIZED,
            Self::InvalidRequest { status, .. } => *status,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Spotify(error) => match SpotifyFailure::of(error) {
                SpotifyFailure::RateLimited => StatusCode::SERVICE_UNAVAILABLE,
                SpotifyFailure::Unauthorized => StatusCode::UNAUTHORIZED,
                SpotifyFailure::Forbidden => StatusCode::FORBIDDEN,
                SpotifyFailure::Rejected | SpotifyFailure::Unavailable => StatusCode::BAD_GATEWAY,
            },
            Self::AccountsUnavailable(_) => StatusCode::BAD_GATEWAY,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Unauthenticated => "unauthenticated",
            Self::SessionExpired => "sessionExpired",
            Self::InvalidRequest { .. } => "invalidRequest",
            Self::NotFound => "notFound",
            Self::Spotify(error) => match SpotifyFailure::of(error) {
                SpotifyFailure::RateLimited => "spotifyRateLimited",
                SpotifyFailure::Unauthorized => "spotifyUnauthorized",
                SpotifyFailure::Forbidden => "spotifyForbidden",
                SpotifyFailure::Rejected => "spotifyRejected",
                SpotifyFailure::Unavailable => "spotifyUnavailable",
            },
            Self::AccountsUnavailable(_) => "spotifyUnavailable",
            Self::Internal(_) => "internal",
        }
    }

    fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Spotify(RetryError::RetryAfterTooLong { wait }) => Some(*wait),
            _ => None,
        }
    }

    pub fn problem(&self) -> ProblemResponse {
        let status = self.status();
        let mut detail = self.to_string();
        let mut source = self.source();
        while let Some(cause) = source {
            detail = format!("{detail}: {cause}");
            source = cause.source();
        }
        ProblemResponse {
            kind: format!("urn:spoticlean:problem:{}", self.code()),
            title: status.canonical_reason().unwrap_or_default().to_owned(),
            status: status.as_u16(),
            detail,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let body = serde_json::to_vec(&self.problem()).unwrap_or_default();
        let mut response = (self.status(), body).into_response();
        let headers = response.headers_mut();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );
        if let Some(wait) = self.retry_after() {
            headers.insert(header::RETRY_AFTER, HeaderValue::from(wait.as_secs()));
        }
        response
    }
}

impl OperationOutput for AppError {
    type Inner = ();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spotify_failures_map_to_upstream_statuses() {
        let cases = [
            (
                AppError::Spotify(RetryError::Permanent(ApiError::Unauthorized)),
                StatusCode::UNAUTHORIZED,
                "spotifyUnauthorized",
            ),
            (
                AppError::Spotify(RetryError::Permanent(ApiError::Forbidden(String::new()))),
                StatusCode::FORBIDDEN,
                "spotifyForbidden",
            ),
            (
                AppError::Spotify(RetryError::RetryAfterTooLong {
                    wait: Duration::from_secs(3600),
                }),
                StatusCode::SERVICE_UNAVAILABLE,
                "spotifyRateLimited",
            ),
            (
                AppError::Spotify(RetryError::GaveUp {
                    attempts: 5,
                    source: ApiError::Server { status: 503 },
                }),
                StatusCode::BAD_GATEWAY,
                "spotifyUnavailable",
            ),
        ];
        for (error, status, code) in cases {
            assert_eq!(error.status(), status, "{error}");
            assert_eq!(error.code(), code, "{error}");
        }
    }

    #[test]
    fn problem_detail_includes_the_causes() {
        let error = AppError::Spotify(RetryError::GaveUp {
            attempts: 5,
            source: ApiError::Server { status: 503 },
        });
        assert_eq!(
            error.problem().detail,
            "gave up after 5 attempts: Spotify answered HTTP 503"
        );
    }

    #[test]
    fn rate_limit_sets_retry_after() {
        let response = AppError::Spotify(RetryError::RetryAfterTooLong {
            wait: Duration::from_secs(3600),
        })
        .into_response();
        assert_eq!(response.headers()[header::RETRY_AFTER], "3600");
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/problem+json"
        );
    }
}
