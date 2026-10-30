//! `POST /api/v1/removals`: removes tracks from the Liked Songs.

use aide::transform::TransformOperation;
use axum::extract::State;
use axum::http::StatusCode;
use axum::{Extension, Json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use validator::{Validate, ValidationError};

use super::error::{AppError, ProblemResponse, SpotifyFailure};
use super::session::Session;
use super::{AppState, SESSION_SCHEME, TAG_LIBRARY, ValidJson};
use crate::spotify::Library;
use crate::spotify::remove::{BatchFailure, FailedBatch, remove_saved_tracks};
use crate::spotify::retry::RetryError;

/// Far above any real library, low enough to bound one request's work.
const MAX_TRACKS: u64 = 10_000;

#[derive(Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase")]
#[schemars(extend("example" = serde_json::json!({
    "trackIds": ["0aym2LBJBk9DAYuHHutrIl", "3n3Ppam7vgaVa1iaRUc9Lp"],
})))]
pub struct CreateRemovalRequest {
    /// Spotify IDs of the tracks to remove from the Liked Songs.
    #[validate(length(min = 1, max = MAX_TRACKS), custom(function = track_ids))]
    pub track_ids: Vec<String>,
}

fn track_ids(ids: &[String]) -> Result<(), ValidationError> {
    let valid = |id: &String| id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric());
    if ids.iter().all(valid) {
        Ok(())
    } else {
        Err(ValidationError::new("trackId")
            .with_message("each track ID is 22 base-62 characters".into()))
    }
}

/// What happened to the requested tracks. Asking twice for the same track
/// lists it once.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RemovalResponse {
    /// Removed from the Liked Songs, or already absent from them.
    pub removed: Vec<String>,
    /// Still liked, in the batches Spotify was sent.
    pub failed: Vec<FailedRemovalResponse>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FailedRemovalResponse {
    pub track_ids: Vec<String>,
    pub reason: RemovalFailureReason,
    pub detail: String,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum RemovalFailureReason {
    /// Spotify rejected the access token.
    SpotifyUnauthorized,
    /// Spotify refused access, e.g. a scope is missing.
    SpotifyForbidden,
    /// Spotify throttled the requests for longer than the server waits.
    SpotifyRateLimited,
    /// Spotify kept failing, or could not be reached.
    SpotifyUnavailable,
    /// Spotify refused this batch.
    SpotifyRejected,
    /// Not sent, since an earlier batch failed in a way this one would too.
    Skipped,
}

impl From<FailedBatch> for FailedRemovalResponse {
    fn from(batch: FailedBatch) -> Self {
        let (reason, detail) = match batch.failure {
            BatchFailure::Skipped => (
                RemovalFailureReason::Skipped,
                "not sent after an earlier batch failed".to_owned(),
            ),
            BatchFailure::Spotify(error) => {
                let reason = match SpotifyFailure::of(&error) {
                    SpotifyFailure::Unauthorized => RemovalFailureReason::SpotifyUnauthorized,
                    SpotifyFailure::Forbidden => RemovalFailureReason::SpotifyForbidden,
                    SpotifyFailure::RateLimited => RemovalFailureReason::SpotifyRateLimited,
                    SpotifyFailure::Unavailable => RemovalFailureReason::SpotifyUnavailable,
                    SpotifyFailure::Rejected => RemovalFailureReason::SpotifyRejected,
                };
                (reason, AppError::Spotify(error).problem().detail)
            }
        };
        Self {
            track_ids: batch.track_ids,
            reason,
            detail,
        }
    }
}

/// The removal is not stored, so there is nothing to point a `Location` at:
/// the outcome is the answer.
pub async fn create<L, A>(
    State(state): State<AppState<L, A>>,
    Extension(session): Extension<Session>,
    ValidJson(request): ValidJson<CreateRemovalRequest>,
) -> Result<(StatusCode, Json<RemovalResponse>), AppError>
where
    L: Library,
{
    let outcome = remove_saved_tracks(
        state.library(),
        &session.access,
        &request.track_ids,
        &state.settings().retry,
    )
    .await;
    if outcome.failed.is_empty() {
        return Ok((
            StatusCode::OK,
            Json(RemovalResponse {
                removed: outcome.removed,
                failed: Vec::new(),
            }),
        ));
    }
    if outcome.removed.is_empty() {
        let first: Option<RetryError> =
            outcome
                .failed
                .into_iter()
                .find_map(|batch| match batch.failure {
                    BatchFailure::Spotify(error) => Some(error),
                    BatchFailure::Skipped => None,
                });
        return Err(first.map_or_else(
            || AppError::Internal("no track was removed".to_owned()),
            AppError::Spotify,
        ));
    }
    Ok((
        StatusCode::MULTI_STATUS,
        Json(RemovalResponse {
            removed: outcome.removed,
            failed: outcome
                .failed
                .into_iter()
                .map(FailedRemovalResponse::from)
                .collect(),
        }),
    ))
}

pub fn create_docs(op: TransformOperation<'_>) -> TransformOperation<'_> {
    op.tag(TAG_LIBRARY)
        .summary("Remove tracks from the Liked Songs")
        .description(
            "Sends the tracks to Spotify in batches of 40, its limit, one batch \
             after the other. Answers 200 when every track is removed, 207 \
             when only some are (the others are listed with the reason), and \
             the status of the first failure, as a problem, when none is.",
        )
        .security_requirement(SESSION_SCHEME)
        .response_with::<200, Json<RemovalResponse>, _>(|res| {
            res.description("Every track is removed")
        })
        .response_with::<207, Json<RemovalResponse>, _>(|res| {
            res.description("Some tracks are removed, the others are listed in `failed`")
        })
        .response_with::<401, Json<ProblemResponse>, _>(|res| {
            res.description("No session, or Spotify rejected its token")
        })
        .response_with::<422, Json<ProblemResponse>, _>(|res| {
            res.description("No track, too many, or a malformed track ID")
        })
        .response_with::<502, Json<ProblemResponse>, _>(|res| {
            res.description("Spotify failed every batch")
        })
}
