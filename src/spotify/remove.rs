//! Removes tracks from the likes in batches, keeping each batch's outcome.

use std::collections::HashSet;

use secrecy::SecretString;
use tracing::{Instrument as _, debug_span, info, warn};

use super::retry::{RateGate, RetryError, RetryPolicy, retrying};
use super::{ApiError, Library, REMOVE_BATCH_SIZE};

/// What happened to each requested track.
#[derive(Debug, Default)]
pub struct RemovalOutcome {
    pub removed: Vec<String>,
    pub failed: Vec<FailedBatch>,
}

#[derive(Debug)]
pub struct FailedBatch {
    pub track_ids: Vec<String>,
    pub failure: BatchFailure,
}

#[derive(Debug)]
pub enum BatchFailure {
    /// Spotify refused or failed this batch.
    Spotify(RetryError),
    /// Not sent: an earlier batch failed in a way every later one would too.
    Skipped,
}

/// Removes `ids` from the user's likes, [`REMOVE_BATCH_SIZE`] at a time, one
/// batch after the other. A batch that fails does not stop the next one,
/// unless the failure is bound to repeat: a rejected token, a refused
/// access, or a quota exhausted for longer than `policy` waits.
pub async fn remove_saved_tracks(
    library: &impl Library,
    access_token: &SecretString,
    ids: &[String],
    policy: &RetryPolicy,
) -> RemovalOutcome {
    let mut seen = HashSet::new();
    let unique: Vec<String> = ids
        .iter()
        .filter(|id| seen.insert(id.as_str()))
        .cloned()
        .collect();
    let gate = RateGate::default();
    let mut outcome = RemovalOutcome::default();
    let mut doomed = false;
    for (index, batch) in unique.chunks(REMOVE_BATCH_SIZE).enumerate() {
        if doomed {
            outcome.failed.push(FailedBatch {
                track_ids: batch.to_vec(),
                failure: BatchFailure::Skipped,
            });
            continue;
        }
        let result = retrying(&gate, policy, || {
            library.remove_saved_tracks(access_token, batch)
        })
        .instrument(debug_span!("batch", index, size = batch.len()))
        .await;
        match result {
            Ok(()) => outcome.removed.extend_from_slice(batch),
            Err(error) => {
                warn!(index, error = %error, "cannot remove a batch of liked songs");
                doomed = dooms_the_rest(&error);
                outcome.failed.push(FailedBatch {
                    track_ids: batch.to_vec(),
                    failure: BatchFailure::Spotify(error),
                });
            }
        }
    }
    info!(
        removed = outcome.removed.len(),
        failed_batches = outcome.failed.len(),
        "removed liked songs"
    );
    outcome
}

fn dooms_the_rest(error: &RetryError) -> bool {
    matches!(
        error,
        RetryError::RetryAfterTooLong { .. }
            | RetryError::Permanent(ApiError::Unauthorized | ApiError::Forbidden(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spotify::{MockLibrary, ready};
    use mockall::Sequence;
    use std::time::Duration;

    fn ids(count: usize) -> Vec<String> {
        (0..count).map(|index| format!("track{index}")).collect()
    }

    async fn remove(library: &MockLibrary, ids: &[String]) -> RemovalOutcome {
        remove_saved_tracks(
            library,
            &SecretString::from("access"),
            ids,
            &RetryPolicy::default(),
        )
        .await
    }

    #[tokio::test]
    async fn removes_in_batches_of_the_spotify_limit() {
        let mut library = MockLibrary::new();
        let mut sequence = Sequence::new();
        for (start, size) in [(0, 40), (40, 40), (80, 5)] {
            library
                .expect_remove_saved_tracks()
                .withf(move |_, batch| batch == ids(85)[start..start + size].as_ref())
                .times(1)
                .in_sequence(&mut sequence)
                .returning(|_, _| ready(Ok(())));
        }
        let outcome = remove(&library, &ids(85)).await;
        assert_eq!(outcome.removed, ids(85));
        assert!(outcome.failed.is_empty());
    }

    #[tokio::test]
    async fn duplicate_ids_are_sent_once() {
        let mut library = MockLibrary::new();
        library
            .expect_remove_saved_tracks()
            .withf(|_, batch| batch == ["a".to_owned(), "b".to_owned()].as_ref())
            .times(1)
            .returning(|_, _| ready(Ok(())));
        let requested = ["a", "b", "a"].map(str::to_owned);
        let outcome = remove(&library, &requested).await;
        assert_eq!(outcome.removed, ["a", "b"]);
    }

    #[tokio::test]
    async fn a_failed_batch_does_not_stop_the_next_one() {
        let mut library = MockLibrary::new();
        let mut sequence = Sequence::new();
        library
            .expect_remove_saved_tracks()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, _| {
                ready(Err(ApiError::Unexpected {
                    status: 400,
                    message: "Invalid id".to_owned(),
                }))
            });
        library
            .expect_remove_saved_tracks()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, _| ready(Ok(())));
        let outcome = remove(&library, &ids(50)).await;
        assert_eq!(outcome.removed, ids(50)[40..]);
        assert_eq!(outcome.failed.len(), 1);
        assert_eq!(outcome.failed[0].track_ids, ids(50)[..40]);
        assert!(matches!(
            outcome.failed[0].failure,
            BatchFailure::Spotify(RetryError::Permanent(ApiError::Unexpected {
                status: 400,
                ..
            }))
        ));
    }

    #[tokio::test]
    async fn a_rejected_token_skips_the_remaining_batches() {
        let mut library = MockLibrary::new();
        library
            .expect_remove_saved_tracks()
            .times(1)
            .returning(|_, _| ready(Err(ApiError::Unauthorized)));
        let outcome = remove(&library, &ids(90)).await;
        assert_eq!(outcome.removed, Vec::<String>::new());
        let failures: Vec<(usize, bool)> = outcome
            .failed
            .iter()
            .map(|batch| {
                (
                    batch.track_ids.len(),
                    matches!(batch.failure, BatchFailure::Skipped),
                )
            })
            .collect();
        assert_eq!(failures, [(40, false), (40, true), (10, true)]);
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limited_batches_are_retried() {
        let mut library = MockLibrary::new();
        let mut sequence = Sequence::new();
        library
            .expect_remove_saved_tracks()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, _| {
                ready(Err(ApiError::RateLimited {
                    retry_after: Some(Duration::from_secs(2)),
                }))
            });
        library
            .expect_remove_saved_tracks()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, _| ready(Ok(())));
        let outcome = remove(&library, &ids(3)).await;
        assert_eq!(outcome.removed, ids(3));
    }
}
