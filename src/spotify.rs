//! Spotify Web API access to the user's saved tracks.

pub mod fetch;

use std::time::Duration;

use reqwest::{StatusCode, header};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use tracing::debug;

use crate::matching::Track;

const API_BASE_URL: &str = "https://api.spotify.com/v1";

/// Largest `limit` the saved-tracks endpoint accepts.
pub const PAGE_SIZE: u32 = 50;

/// One page of the user's saved tracks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedTracksPage {
    /// Number of saved tracks in the whole library.
    pub total: u32,
    pub tracks: Vec<Track>,
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("rate limited by Spotify")]
    RateLimited { retry_after: Option<Duration> },
    #[error("Spotify answered HTTP {status}")]
    Server { status: u16 },
    #[error("cannot reach Spotify: {0}")]
    Network(String),
    #[error("Spotify rejected the access token (HTTP 401); delete the token cache to log in again")]
    Unauthorized,
    #[error(
        "Spotify refused access (HTTP 403): {0}; a development-mode app only \
         serves the users added to it in the Spotify developer dashboard"
    )]
    Forbidden(String),
    #[error("unexpected HTTP {status} from Spotify: {message}")]
    Unexpected { status: u16, message: String },
    #[error("cannot decode the Spotify response: {0}")]
    Decode(String),
}

impl ApiError {
    /// Whether the same request may succeed if sent again later.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. } | Self::Server { .. } | Self::Network(_)
        )
    }
}

/// The part of the Web API spoticlean reads.
#[cfg_attr(test, mockall::automock)]
// automock copies this onto the mock, whose `async fn`s never await.
#[cfg_attr(test, allow(clippy::unused_async_trait_impl))]
pub trait Library {
    async fn saved_tracks(&self, offset: u32, limit: u32) -> Result<SavedTracksPage, ApiError>;
}

/// [`Library`] over HTTPS, authenticated as the user.
pub struct SpotifyApi {
    http: reqwest::Client,
    access_token: SecretString,
}

impl SpotifyApi {
    pub fn new(http: reqwest::Client, access_token: SecretString) -> Self {
        Self { http, access_token }
    }
}

impl Library for SpotifyApi {
    async fn saved_tracks(&self, offset: u32, limit: u32) -> Result<SavedTracksPage, ApiError> {
        debug!(offset, limit, "requesting saved tracks");
        let response = self
            .http
            .get(format!("{API_BASE_URL}/me/tracks"))
            .query(&[("offset", offset), ("limit", limit)])
            .bearer_auth(self.access_token.expose_secret())
            .send()
            .await
            .map_err(|error| ApiError::Network(error.without_url().to_string()))?;
        let status = response.status();
        debug!(offset, status = status.as_u16(), "saved tracks response");
        if status.is_success() {
            let body = response
                .bytes()
                .await
                .map_err(|error| ApiError::Network(error.without_url().to_string()))?;
            return parse_page(&body);
        }
        let retry_after = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_retry_after);
        let body = response.text().await.unwrap_or_default();
        Err(classify_failure(status, retry_after, &body))
    }
}

fn parse_retry_after(value: &str) -> Option<Duration> {
    value.trim().parse().ok().map(Duration::from_secs)
}

fn classify_failure(status: StatusCode, retry_after: Option<Duration>, body: &str) -> ApiError {
    let message = serde_json::from_str::<ErrorBody>(body).map_or_else(
        |_| body.chars().take(200).collect(),
        |body| body.error.message,
    );
    match status {
        StatusCode::TOO_MANY_REQUESTS => ApiError::RateLimited { retry_after },
        StatusCode::UNAUTHORIZED => ApiError::Unauthorized,
        StatusCode::FORBIDDEN => ApiError::Forbidden(message),
        status if status.is_server_error() => ApiError::Server {
            status: status.as_u16(),
        },
        status => ApiError::Unexpected {
            status: status.as_u16(),
            message,
        },
    }
}

#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct PageDto {
    total: u32,
    #[serde(default)]
    items: Vec<SavedItemDto>,
}

#[derive(Deserialize)]
struct SavedItemDto {
    added_at: String,
    track: Option<TrackDto>,
}

#[derive(Deserialize)]
struct TrackDto {
    id: Option<String>,
    name: String,
    #[serde(default)]
    artists: Vec<NamedDto>,
    album: Option<AlbumDto>,
    #[serde(default)]
    duration_ms: u64,
    external_ids: Option<ExternalIdsDto>,
}

#[derive(Deserialize)]
struct NamedDto {
    name: String,
}

#[derive(Deserialize)]
struct AlbumDto {
    name: String,
    release_date: Option<String>,
}

#[derive(Deserialize)]
struct ExternalIdsDto {
    isrc: Option<String>,
}

fn parse_page(body: &[u8]) -> Result<SavedTracksPage, ApiError> {
    let page: PageDto =
        serde_json::from_slice(body).map_err(|error| ApiError::Decode(error.to_string()))?;
    let tracks = page
        .items
        .into_iter()
        .filter_map(|item| {
            let track = item.track?;
            let id = track.id?;
            let (album, release_date) = track
                .album
                .map(|album| (album.name, album.release_date))
                .unwrap_or_default();
            Some(Track {
                id,
                title: track.name,
                artists: track
                    .artists
                    .into_iter()
                    .map(|artist| artist.name)
                    .collect(),
                album,
                release_date: release_date.filter(|date| !date.is_empty()),
                duration: Duration::from_millis(track.duration_ms),
                isrc: track.external_ids.and_then(|ids| ids.isrc),
                added_at: item.added_at,
            })
        })
        .collect();
    Ok(SavedTracksPage {
        total: page.total,
        tracks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_page_maps_the_fields_used_for_matching() {
        let body = br#"{
            "href": "https://api.spotify.com/v1/me/tracks?offset=0&limit=50",
            "limit": 50, "next": null, "offset": 0, "previous": null, "total": 3,
            "items": [
                {
                    "added_at": "2023-04-05T06:07:08Z",
                    "track": {
                        "id": "t1", "name": "Let It Be - Remastered 2009",
                        "artists": [{"id": "a1", "name": "The Beatles"}],
                        "album": {"name": "Let It Be", "release_date": "1970-05-08",
                                  "release_date_precision": "day"},
                        "duration_ms": 243026,
                        "external_ids": {"isrc": "GBAYE0601713"},
                        "is_local": false
                    }
                },
                {
                    "added_at": "2023-04-06T00:00:00Z",
                    "track": {
                        "id": "t2", "name": "Song", "artists": [],
                        "album": {"name": "Album", "release_date": "1981",
                                  "release_date_precision": "year"},
                        "duration_ms": 1000
                    }
                },
                {"added_at": "2023-04-07T00:00:00Z", "track": null}
            ]
        }"#;
        let page = parse_page(body).unwrap();
        assert_eq!(page.total, 3);
        assert_eq!(
            page.tracks,
            [
                Track {
                    id: "t1".to_owned(),
                    title: "Let It Be - Remastered 2009".to_owned(),
                    artists: vec!["The Beatles".to_owned()],
                    album: "Let It Be".to_owned(),
                    release_date: Some("1970-05-08".to_owned()),
                    duration: Duration::from_millis(243_026),
                    isrc: Some("GBAYE0601713".to_owned()),
                    added_at: "2023-04-05T06:07:08Z".to_owned(),
                },
                Track {
                    id: "t2".to_owned(),
                    title: "Song".to_owned(),
                    artists: Vec::new(),
                    album: "Album".to_owned(),
                    release_date: Some("1981".to_owned()),
                    duration: Duration::from_secs(1),
                    isrc: None,
                    added_at: "2023-04-06T00:00:00Z".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn parse_page_rejects_garbage() {
        assert!(matches!(parse_page(b"<html>"), Err(ApiError::Decode(_))));
    }

    #[test]
    fn classify_failure_by_status() {
        assert!(matches!(
            classify_failure(StatusCode::TOO_MANY_REQUESTS, Some(Duration::from_secs(3)), ""),
            ApiError::RateLimited { retry_after: Some(d) } if d == Duration::from_secs(3)
        ));
        assert!(matches!(
            classify_failure(StatusCode::BAD_GATEWAY, None, ""),
            ApiError::Server { status: 502 }
        ));
        assert!(matches!(
            classify_failure(StatusCode::UNAUTHORIZED, None, ""),
            ApiError::Unauthorized
        ));
        let forbidden = classify_failure(
            StatusCode::FORBIDDEN,
            None,
            r#"{"error": {"status": 403, "message": "User not registered"}}"#,
        );
        assert!(
            matches!(&forbidden, ApiError::Forbidden(message) if message == "User not registered")
        );
        assert!(matches!(
            classify_failure(StatusCode::BAD_REQUEST, None, "plain text"),
            ApiError::Unexpected { status: 400, message } if message == "plain text"
        ));
    }

    #[test]
    fn transient_errors() {
        assert!(ApiError::RateLimited { retry_after: None }.is_transient());
        assert!(ApiError::Server { status: 503 }.is_transient());
        assert!(ApiError::Network("reset".to_owned()).is_transient());
        assert!(!ApiError::Unauthorized.is_transient());
        assert!(!ApiError::Forbidden(String::new()).is_transient());
    }

    #[test]
    fn parse_retry_after_reads_seconds() {
        assert_eq!(parse_retry_after(" 7 "), Some(Duration::from_secs(7)));
        assert_eq!(parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT"), None);
    }
}
