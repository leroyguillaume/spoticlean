//! Reads the whole library: concurrent pagination with retries.

use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::Duration;

use futures::{StreamExt, TryStreamExt, stream};
use tokio::time::{Instant, sleep, sleep_until};
use tracing::{debug, info, warn};

use super::{ApiError, Library, PAGE_SIZE, SavedTracksPage};
use crate::matching::Track;

/// How hard to insist when Spotify fails or throttles.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Retries of a page after a 5xx or a network error.
    pub transient_retries: u32,
    /// Retries of a page after a 429.
    pub rate_limit_retries: u32,
    /// First wait after a transient failure, doubled on each retry.
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    /// A `Retry-After` longer than this aborts instead of waiting: Spotify
    /// sends hours when the development-mode quota is exhausted.
    pub max_retry_after: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            transient_retries: 4,
            rate_limit_retries: 10,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(30),
            max_retry_after: Duration::from_secs(120),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("cannot fetch the saved tracks at offset {offset}")]
    Api {
        offset: u32,
        #[source]
        source: ApiError,
    },
    #[error("gave up on the saved tracks at offset {offset} after {attempts} attempts")]
    GaveUp {
        offset: u32,
        attempts: u32,
        #[source]
        source: ApiError,
    },
    #[error(
        "Spotify asks to wait {}s before the next request; try again later",
        wait.as_secs()
    )]
    RetryAfterTooLong { wait: Duration },
}

/// Shared by every in-flight request, so that one 429 pauses them all rather
/// than letting the others keep hitting the limit.
#[derive(Default)]
struct RateGate(Mutex<Option<Instant>>);

impl RateGate {
    async fn wait(&self) {
        loop {
            let until = *self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match until {
                Some(until) if until > Instant::now() => sleep_until(until).await,
                _ => return,
            }
        }
    }

    fn close_for(&self, wait: Duration) {
        let until = Instant::now() + wait;
        let mut guard = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.is_none_or(|current| current < until) {
            *guard = Some(until);
        }
    }
}

/// Fetches every saved track: the first page gives the total, the remaining
/// pages are fetched `concurrency` at a time.
pub async fn fetch_saved_tracks(
    library: &impl Library,
    concurrency: NonZeroUsize,
    policy: &RetryPolicy,
) -> Result<Vec<Track>, FetchError> {
    let gate = RateGate::default();
    let first = fetch_page(library, &gate, policy, 0).await?;
    let total = first.total;
    info!(total, "fetching liked songs");

    let rest: Vec<SavedTracksPage> = stream::iter((PAGE_SIZE..total).step_by(PAGE_SIZE as usize))
        .map(|offset| fetch_page(library, &gate, policy, offset))
        .buffered(concurrency.get())
        .try_collect()
        .await?;

    let mut seen = HashSet::new();
    let mut tracks = Vec::with_capacity(total as usize);
    for track in std::iter::once(first)
        .chain(rest)
        .flat_map(|page| page.tracks)
    {
        if seen.insert(track.id.clone()) {
            tracks.push(track);
        } else {
            debug!(id = %track.id, "track seen twice, the library changed while paginating");
        }
    }
    info!(fetched = tracks.len(), "fetched liked songs");
    Ok(tracks)
}

async fn fetch_page(
    library: &impl Library,
    gate: &RateGate,
    policy: &RetryPolicy,
    offset: u32,
) -> Result<SavedTracksPage, FetchError> {
    let mut transient_failures = 0;
    let mut rate_limits = 0;
    loop {
        gate.wait().await;
        let error = match library.saved_tracks(offset, PAGE_SIZE).await {
            Ok(page) => {
                debug!(offset, tracks = page.tracks.len(), "fetched page");
                return Ok(page);
            }
            Err(error) => error,
        };
        let attempts = transient_failures + rate_limits + 1;
        match error {
            ApiError::RateLimited { retry_after } => {
                rate_limits += 1;
                if rate_limits > policy.rate_limit_retries {
                    return Err(FetchError::GaveUp {
                        offset,
                        attempts,
                        source: error,
                    });
                }
                let wait = retry_after.unwrap_or_else(|| backoff(policy, rate_limits));
                if wait > policy.max_retry_after {
                    return Err(FetchError::RetryAfterTooLong { wait });
                }
                warn!(
                    offset,
                    wait_secs = wait.as_secs(),
                    "rate limited by Spotify, pausing requests"
                );
                gate.close_for(wait);
            }
            error if error.is_transient() => {
                transient_failures += 1;
                if transient_failures > policy.transient_retries {
                    return Err(FetchError::GaveUp {
                        offset,
                        attempts,
                        source: error,
                    });
                }
                let wait = backoff(policy, transient_failures);
                warn!(offset, %error, wait_ms = wait.as_millis(), "transient failure, retrying");
                sleep(wait).await;
            }
            error => {
                return Err(FetchError::Api {
                    offset,
                    source: error,
                });
            }
        }
    }
}

fn backoff(policy: &RetryPolicy, failures: u32) -> Duration {
    policy
        .initial_backoff
        .saturating_mul(1 << failures.saturating_sub(1).min(16))
        .min(policy.max_backoff)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spotify::MockLibrary;
    use mockall::Sequence;
    use mockall::predicate::eq;

    const ONE: NonZeroUsize = NonZeroUsize::MIN;

    fn track(id: &str) -> Track {
        Track {
            id: id.to_owned(),
            title: format!("Song {id}"),
            artists: vec!["Artist".to_owned()],
            album: "Album".to_owned(),
            release_date: None,
            duration: Duration::from_secs(200),
            isrc: None,
            added_at: "2024-01-01T00:00:00Z".to_owned(),
        }
    }

    fn page(total: u32, offset: u32) -> SavedTracksPage {
        let end = (offset + PAGE_SIZE).min(total);
        SavedTracksPage {
            total,
            tracks: (offset..end)
                .map(|index| track(&index.to_string()))
                .collect(),
        }
    }

    #[tokio::test]
    async fn fetches_every_page_in_order() {
        let mut library = MockLibrary::new();
        for offset in [0, 50, 100] {
            library
                .expect_saved_tracks()
                .with(eq(offset), eq(PAGE_SIZE))
                .times(1)
                .returning(move |offset, _| Ok(page(120, offset)));
        }
        let tracks = fetch_saved_tracks(
            &library,
            NonZeroUsize::new(4).unwrap(),
            &RetryPolicy::default(),
        )
        .await
        .unwrap();
        let ids: Vec<String> = tracks.into_iter().map(|t| t.id).collect();
        let expected: Vec<String> = (0..120).map(|i: u32| i.to_string()).collect();
        assert_eq!(ids, expected);
    }

    #[tokio::test]
    async fn empty_library_needs_one_request() {
        let mut library = MockLibrary::new();
        library
            .expect_saved_tracks()
            .times(1)
            .returning(|offset, _| Ok(page(0, offset)));
        let tracks = fetch_saved_tracks(&library, ONE, &RetryPolicy::default())
            .await
            .unwrap();
        assert!(tracks.is_empty());
    }

    #[tokio::test]
    async fn exact_multiple_of_the_page_size() {
        let mut library = MockLibrary::new();
        for offset in [0, 50] {
            library
                .expect_saved_tracks()
                .with(eq(offset), eq(PAGE_SIZE))
                .times(1)
                .returning(move |offset, _| Ok(page(100, offset)));
        }
        let tracks = fetch_saved_tracks(&library, ONE, &RetryPolicy::default())
            .await
            .unwrap();
        assert_eq!(tracks.len(), 100);
    }

    #[tokio::test]
    async fn tracks_shifted_between_pages_are_kept_once() {
        let mut library = MockLibrary::new();
        library
            .expect_saved_tracks()
            .with(eq(0), eq(PAGE_SIZE))
            .returning(|_, _| Ok(page(60, 0)));
        library
            .expect_saved_tracks()
            .with(eq(50), eq(PAGE_SIZE))
            .returning(|_, _| Ok(page(60, 49)));
        let tracks = fetch_saved_tracks(&library, ONE, &RetryPolicy::default())
            .await
            .unwrap();
        assert_eq!(tracks.len(), 60);
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_honours_retry_after() {
        let mut library = MockLibrary::new();
        let mut sequence = Sequence::new();
        library
            .expect_saved_tracks()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, _| {
                Err(ApiError::RateLimited {
                    retry_after: Some(Duration::from_secs(7)),
                })
            });
        library
            .expect_saved_tracks()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|offset, _| Ok(page(10, offset)));
        let started = Instant::now();
        let tracks = fetch_saved_tracks(&library, ONE, &RetryPolicy::default())
            .await
            .unwrap();
        assert_eq!(tracks.len(), 10);
        assert!(started.elapsed() >= Duration::from_secs(7));
        assert!(started.elapsed() < Duration::from_secs(8));
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_on_one_page_pauses_the_others() {
        let started = Instant::now();
        let paused_until = started + Duration::from_secs(5);
        let mut library = MockLibrary::new();
        library
            .expect_saved_tracks()
            .with(eq(0), eq(PAGE_SIZE))
            .returning(|_, _| Ok(page(150, 0)));
        let mut sequence = Sequence::new();
        library
            .expect_saved_tracks()
            .with(eq(50), eq(PAGE_SIZE))
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, _| {
                Err(ApiError::RateLimited {
                    retry_after: Some(Duration::from_secs(5)),
                })
            });
        library
            .expect_saved_tracks()
            .with(eq(50), eq(PAGE_SIZE))
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, _| Ok(page(150, 50)));
        library
            .expect_saved_tracks()
            .with(eq(100), eq(PAGE_SIZE))
            .times(1)
            .returning(move |_, _| {
                assert!(
                    Instant::now() >= paused_until,
                    "requested while rate limited"
                );
                Ok(page(150, 100))
            });
        let tracks = fetch_saved_tracks(
            &library,
            NonZeroUsize::new(2).unwrap(),
            &RetryPolicy::default(),
        )
        .await
        .unwrap();
        assert_eq!(tracks.len(), 150);
    }

    #[tokio::test(start_paused = true)]
    async fn rate_gate_holds_until_the_latest_deadline() {
        let gate = RateGate::default();
        let started = Instant::now();
        gate.close_for(Duration::from_secs(3));
        gate.close_for(Duration::from_secs(1));
        gate.wait().await;
        assert_eq!(started.elapsed(), Duration::from_secs(3));
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_without_retry_after_backs_off() {
        let mut library = MockLibrary::new();
        let mut sequence = Sequence::new();
        library
            .expect_saved_tracks()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, _| Err(ApiError::RateLimited { retry_after: None }));
        library
            .expect_saved_tracks()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|offset, _| Ok(page(1, offset)));
        let started = Instant::now();
        fetch_saved_tracks(&library, ONE, &RetryPolicy::default())
            .await
            .unwrap();
        assert_eq!(started.elapsed(), Duration::from_secs(1));
    }

    #[tokio::test(start_paused = true)]
    async fn overly_long_retry_after_aborts() {
        let mut library = MockLibrary::new();
        library.expect_saved_tracks().times(1).returning(|_, _| {
            Err(ApiError::RateLimited {
                retry_after: Some(Duration::from_secs(3600)),
            })
        });
        let error = fetch_saved_tracks(&library, ONE, &RetryPolicy::default())
            .await
            .unwrap_err();
        assert!(
            matches!(error, FetchError::RetryAfterTooLong { wait } if wait == Duration::from_secs(3600))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn server_errors_are_retried_with_backoff() {
        let mut library = MockLibrary::new();
        let mut sequence = Sequence::new();
        library
            .expect_saved_tracks()
            .times(3)
            .in_sequence(&mut sequence)
            .returning(|_, _| Err(ApiError::Server { status: 503 }));
        library
            .expect_saved_tracks()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|offset, _| Ok(page(1, offset)));
        let started = Instant::now();
        fetch_saved_tracks(&library, ONE, &RetryPolicy::default())
            .await
            .unwrap();
        assert_eq!(started.elapsed(), Duration::from_secs(1 + 2 + 4));
    }

    #[tokio::test(start_paused = true)]
    async fn server_errors_give_up_after_the_retries() {
        let mut library = MockLibrary::new();
        library
            .expect_saved_tracks()
            .times(5)
            .returning(|_, _| Err(ApiError::Server { status: 500 }));
        let error = fetch_saved_tracks(&library, ONE, &RetryPolicy::default())
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            FetchError::GaveUp {
                offset: 0,
                attempts: 5,
                source: ApiError::Server { status: 500 }
            }
        ));
    }

    #[tokio::test]
    async fn permanent_errors_are_not_retried() {
        let mut library = MockLibrary::new();
        library
            .expect_saved_tracks()
            .with(eq(0), eq(PAGE_SIZE))
            .returning(|_, _| Ok(page(80, 0)));
        library
            .expect_saved_tracks()
            .with(eq(50), eq(PAGE_SIZE))
            .times(1)
            .returning(|_, _| Err(ApiError::Unauthorized));
        let error = fetch_saved_tracks(&library, ONE, &RetryPolicy::default())
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            FetchError::Api {
                offset: 50,
                source: ApiError::Unauthorized
            }
        ));
    }

    #[test]
    fn backoff_doubles_up_to_the_cap() {
        let policy = RetryPolicy::default();
        let waits: Vec<u64> = (1..=7).map(|n| backoff(&policy, n).as_secs()).collect();
        assert_eq!(waits, [1, 2, 4, 8, 16, 30, 30]);
        assert_eq!(backoff(&policy, 100), Duration::from_secs(30));
    }
}
