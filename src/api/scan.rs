//! `GET /api/v1/scan`: reads the Liked Songs and streams the duplicate groups.

use std::convert::Infallible;
use std::time::Duration;

use aide::OperationOutput;
use aide::generate::GenContext;
use aide::openapi::{MediaType, Operation, SchemaObject};
use aide::transform::TransformOperation;
use axum::Extension;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures::{FutureExt as _, Stream, StreamExt as _, future, stream};
use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::Serialize;
use tracing::{info, warn};

use super::error::{AppError, ProblemResponse};
use super::session::Session;
use super::{AppState, SESSION_SCHEME, TAG_LIBRARY};
use crate::matching::{self, Confidence, Group, Track};
use crate::spotify::Library;
use crate::spotify::fetch::{Progress, fetch_saved_tracks};

/// How far the scan has got.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(extend("example" = serde_json::json!({"fetched": 150, "total": 2034})))]
pub struct ScanProgressResponse {
    /// Liked Songs read so far.
    pub fetched: u32,
    /// Liked Songs in the library.
    pub total: u32,
}

/// The duplicate groups found among the Liked Songs.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScanResponse {
    /// Liked Songs read.
    pub scanned: usize,
    /// Most certain first.
    pub groups: Vec<GroupResponse>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ConfidenceResponse {
    /// Every track carries the same ISRC.
    Certain,
    /// Same title and credits once normalized, same version, close durations.
    Probable,
    /// The same song in different recordings: live, remix, new guests, …
    Variant,
}

impl From<Confidence> for ConfidenceResponse {
    fn from(confidence: Confidence) -> Self {
        match confidence {
            Confidence::Certain => Self::Certain,
            Confidence::Probable => Self::Probable,
            Confidence::Variant => Self::Variant,
        }
    }
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupResponse {
    /// Opaque and stable across scans as long as the group's tracks keep
    /// their names, so the UI can remember choices made about the group.
    pub key: String,
    pub confidence: ConfidenceResponse,
    /// Longest track minus shortest track, in milliseconds.
    pub duration_gap_ms: u64,
    /// Sorted by release date, then by the date they were liked.
    pub tracks: Vec<TrackResponse>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(extend("example" = serde_json::json!({
    "id": "0aym2LBJBk9DAYuHHutrIl",
    "title": "Hey Jude - Remastered 2015",
    "artists": ["The Beatles"],
    "album": "1 (Remastered)",
    "releaseDate": "2000-11-13",
    "durationMs": 425_653,
    "addedAt": "2021-06-02T19:10:03Z",
    "isrc": "GBUM71505078",
    "versionLabel": "Remastered 2015",
})))]
pub struct TrackResponse {
    /// Spotify track ID.
    pub id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    /// Album release date as Spotify gives it: `YYYY`, `YYYY-MM` or `YYYY-MM-DD`.
    pub release_date: Option<String>,
    pub duration_ms: u64,
    /// When the track was liked, RFC 3339.
    pub added_at: String,
    pub isrc: Option<String>,
    /// The title's version or reissue suffixes as written (`2012 Remaster`,
    /// `Live`), `null` for a bare title.
    pub version_label: Option<String>,
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

impl From<&Track> for TrackResponse {
    fn from(track: &Track) -> Self {
        Self {
            id: track.id.clone(),
            title: track.title.clone(),
            artists: track.artists.clone(),
            album: track.album.clone(),
            release_date: track.release_date.clone(),
            duration_ms: millis(track.duration),
            added_at: track.added_at.clone(),
            isrc: track.isrc.clone(),
            version_label: matching::version_label(&track.title),
        }
    }
}

impl From<&Group<'_>> for GroupResponse {
    fn from(group: &Group<'_>) -> Self {
        let durations = group.tracks.iter().map(|track| track.duration);
        let gap = durations
            .clone()
            .max()
            .unwrap_or_default()
            .saturating_sub(durations.min().unwrap_or_default());
        Self {
            key: group.key.clone(),
            confidence: group.confidence.into(),
            duration_gap_ms: millis(gap),
            tracks: group
                .tracks
                .iter()
                .map(|&track| TrackResponse::from(track))
                .collect(),
        }
    }
}

/// One server-sent event of a scan: its `event` field names the variant, its
/// `data` field holds the JSON payload. `progress` events come first, then
/// exactly one `result` or `error`, then the stream ends.
#[derive(Serialize, JsonSchema)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum ScanEvent {
    Progress(ScanProgressResponse),
    Result(ScanResponse),
    Error(ProblemResponse),
}

impl ScanEvent {
    fn into_sse(self) -> Event {
        let (name, data) = match &self {
            Self::Progress(progress) => ("progress", serde_json::to_string(progress)),
            Self::Result(result) => ("result", serde_json::to_string(result)),
            Self::Error(problem) => ("error", serde_json::to_string(problem)),
        };
        Event::default()
            .event(name)
            .data(data.expect("scan events serialize"))
    }
}

/// The `text/event-stream` response, documented as [`ScanEvent`]s.
pub struct ScanEvents<S>(Sse<S>);

impl<S> IntoResponse for ScanEvents<S>
where
    Sse<S>: IntoResponse,
{
    fn into_response(self) -> Response {
        self.0.into_response()
    }
}

impl<S> OperationOutput for ScanEvents<S> {
    type Inner = ScanEvent;

    fn operation_response(
        ctx: &mut GenContext,
        _operation: &mut Operation,
    ) -> Option<aide::openapi::Response> {
        Some(aide::openapi::Response {
            description: "Server-sent events: `progress`, then `result` or `error`".to_owned(),
            content: IndexMap::from_iter([(
                "text/event-stream".to_owned(),
                MediaType {
                    schema: Some(SchemaObject {
                        json_schema: ctx.schema.subschema_for::<ScanEvent>(),
                        example: None,
                        external_docs: None,
                    }),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        })
    }

    fn inferred_responses(
        ctx: &mut GenContext,
        operation: &mut Operation,
    ) -> Vec<(Option<u16>, aide::openapi::Response)> {
        Self::operation_response(ctx, operation)
            .map(|response| vec![(Some(200), response)])
            .unwrap_or_default()
    }
}

/// The scan runs inside the response stream: a client that disconnects drops
/// the stream, which cancels the requests still in flight to Spotify.
pub async fn scan<L, A>(
    State(state): State<AppState<L, A>>,
    Extension(session): Extension<Session>,
) -> ScanEvents<impl Stream<Item = Result<Event, Infallible>> + Send + 'static>
where
    L: Library + Send + Sync + 'static,
    A: Send + Sync + 'static,
{
    let (sender, receiver) = futures::channel::mpsc::unbounded();
    let work = async move {
        let settings = state.settings();
        let progress = sender.clone();
        let fetched = fetch_saved_tracks(
            state.library(),
            &session.access,
            settings.scan_concurrency,
            &settings.retry,
            move |Progress { fetched, total }| {
                let _ = progress
                    .unbounded_send(ScanEvent::Progress(ScanProgressResponse { fetched, total }));
            },
        )
        .await;
        let event = match fetched {
            Ok(tracks) => {
                let groups = matching::find_groups(&tracks, settings.duration_tolerance);
                info!(
                    tracks = tracks.len(),
                    groups = groups.len(),
                    "scanned liked songs"
                );
                ScanEvent::Result(ScanResponse {
                    scanned: tracks.len(),
                    groups: groups.iter().map(GroupResponse::from).collect(),
                })
            }
            Err(error) => {
                let error = AppError::from(error);
                warn!(%error, "scan failed");
                ScanEvent::Error(error.problem())
            }
        };
        let _ = sender.unbounded_send(event);
    };
    let events = stream::select(
        receiver.map(|event: ScanEvent| Ok(event.into_sse())),
        work.into_stream().filter_map(|()| future::ready(None)),
    );
    ScanEvents(Sse::new(events).keep_alive(KeepAlive::default()))
}

pub fn scan_docs(op: TransformOperation<'_>) -> TransformOperation<'_> {
    op.tag(TAG_LIBRARY)
        .summary("Scan the Liked Songs for duplicates")
        .description(
            "Reads the whole library and answers with a `text/event-stream`: \
             a `progress` event per page read, then one `result` event holding \
             the duplicate groups, or one `error` event holding a problem, and \
             the stream ends. Nothing is stored: each request scans anew, and \
             closing the stream cancels the scan. A missing session is a 401 \
             before the stream starts.",
        )
        .security_requirement(SESSION_SCHEME)
        .response_with::<401, axum::Json<ProblemResponse>, _>(|res| {
            res.description("No session, or one that cannot be renewed")
        })
}
