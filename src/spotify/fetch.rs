//! Reads the whole library: concurrent pagination with retries.

use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU32, Ordering};

use futures::{StreamExt, TryStreamExt, stream};
use secrecy::SecretString;
use tracing::{Instrument as _, debug, debug_span, info};

use super::retry::{RateGate, RetryError, RetryPolicy, retrying};
use super::{Library, PAGE_SIZE, SavedTracksPage};
use crate::matching::Track;

/// How far a fetch has got, in tracks of the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub fetched: u32,
    pub total: u32,
}

#[derive(Debug, thiserror::Error)]
#[error("cannot fetch the saved tracks at offset {offset}")]
pub struct FetchError {
    pub offset: u32,
    #[source]
    pub source: RetryError,
}

/// Fetches every saved track: the first page gives the total, the remaining
/// pages are fetched `concurrency` at a time. `on_progress` is called each
/// time a page arrives.
pub async fn fetch_saved_tracks(
    library: &impl Library,
    access_token: &SecretString,
    concurrency: NonZeroUsize,
    policy: &RetryPolicy,
    on_progress: impl Fn(Progress) + Sync,
) -> Result<Vec<Track>, FetchError> {
    let gate = RateGate::default();
    let first = fetch_page(library, access_token, &gate, policy, 0).await?;
    let total = first.total;
    info!(total, "fetching liked songs");
    let fetched = AtomicU32::new(0);
    let report = |offset: u32| {
        let covered = total.saturating_sub(offset).min(PAGE_SIZE);
        let fetched = fetched.fetch_add(covered, Ordering::Relaxed) + covered;
        on_progress(Progress { fetched, total });
    };
    report(0);

    let rest: Vec<SavedTracksPage> = stream::iter((PAGE_SIZE..total).step_by(PAGE_SIZE as usize))
        .map(|offset| {
            let report = &report;
            let gate = &gate;
            async move {
                let page = fetch_page(library, access_token, gate, policy, offset).await?;
                report(offset);
                Ok::<_, FetchError>(page)
            }
        })
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
    access_token: &SecretString,
    gate: &RateGate,
    policy: &RetryPolicy,
    offset: u32,
) -> Result<SavedTracksPage, FetchError> {
    let page = retrying(gate, policy, || {
        library.saved_tracks(access_token, offset, PAGE_SIZE)
    })
    .instrument(debug_span!("page", offset))
    .await
    .map_err(|source| FetchError { offset, source })?;
    debug!(offset, tracks = page.tracks.len(), "fetched page");
    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spotify::{ApiError, MockLibrary, ready};
    use mockall::Sequence;
    use mockall::predicate::{always, eq};
    use std::sync::Mutex;
    use std::time::Duration;
    use tokio::time::Instant;

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

    async fn fetch(
        library: &MockLibrary,
        concurrency: NonZeroUsize,
    ) -> Result<Vec<Track>, FetchError> {
        fetch_saved_tracks(
            library,
            &SecretString::from("access"),
            concurrency,
            &RetryPolicy::default(),
            |_| {},
        )
        .await
    }

    fn rate_limited(seconds: Option<u64>) -> ApiError {
        ApiError::RateLimited {
            retry_after: seconds.map(Duration::from_secs),
        }
    }

    #[tokio::test]
    async fn fetches_every_page_in_order() {
        let mut library = MockLibrary::new();
        for offset in [0, 50, 100] {
            library
                .expect_saved_tracks()
                .with(always(), eq(offset), eq(PAGE_SIZE))
                .times(1)
                .returning(|_, offset, _| ready(Ok(page(120, offset))));
        }
        let tracks = fetch(&library, NonZeroUsize::new(4).unwrap())
            .await
            .unwrap();
        let ids: Vec<String> = tracks.into_iter().map(|t| t.id).collect();
        let expected: Vec<String> = (0..120).map(|i: u32| i.to_string()).collect();
        assert_eq!(ids, expected);
    }

    #[tokio::test]
    async fn passes_the_access_token() {
        let mut library = MockLibrary::new();
        library
            .expect_saved_tracks()
            .withf(|token, _, _| secrecy::ExposeSecret::expose_secret(token) == "access")
            .times(1)
            .returning(|_, offset, _| ready(Ok(page(1, offset))));
        fetch(&library, ONE).await.unwrap();
    }

    #[tokio::test]
    async fn reports_progress_per_page() {
        let mut library = MockLibrary::new();
        library
            .expect_saved_tracks()
            .returning(|_, offset, _| ready(Ok(page(120, offset))));
        let reports = Mutex::new(Vec::new());
        fetch_saved_tracks(
            &library,
            &SecretString::from("access"),
            ONE,
            &RetryPolicy::default(),
            |progress| reports.lock().unwrap().push(progress),
        )
        .await
        .unwrap();
        let fetched: Vec<(u32, u32)> = reports
            .into_inner()
            .unwrap()
            .into_iter()
            .map(|progress| (progress.fetched, progress.total))
            .collect();
        assert_eq!(fetched, [(50, 120), (100, 120), (120, 120)]);
    }

    #[tokio::test]
    async fn empty_library_needs_one_request() {
        let mut library = MockLibrary::new();
        library
            .expect_saved_tracks()
            .times(1)
            .returning(|_, offset, _| ready(Ok(page(0, offset))));
        let tracks = fetch(&library, ONE).await.unwrap();
        assert!(tracks.is_empty());
    }

    #[tokio::test]
    async fn exact_multiple_of_the_page_size() {
        let mut library = MockLibrary::new();
        for offset in [0, 50] {
            library
                .expect_saved_tracks()
                .with(always(), eq(offset), eq(PAGE_SIZE))
                .times(1)
                .returning(|_, offset, _| ready(Ok(page(100, offset))));
        }
        let tracks = fetch(&library, ONE).await.unwrap();
        assert_eq!(tracks.len(), 100);
    }

    #[tokio::test]
    async fn tracks_shifted_between_pages_are_kept_once() {
        let mut library = MockLibrary::new();
        library
            .expect_saved_tracks()
            .with(always(), eq(0), eq(PAGE_SIZE))
            .returning(|_, _, _| ready(Ok(page(60, 0))));
        library
            .expect_saved_tracks()
            .with(always(), eq(50), eq(PAGE_SIZE))
            .returning(|_, _, _| ready(Ok(page(60, 49))));
        let tracks = fetch(&library, ONE).await.unwrap();
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
            .returning(|_, _, _| ready(Err(rate_limited(Some(7)))));
        library
            .expect_saved_tracks()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, offset, _| ready(Ok(page(10, offset))));
        let started = Instant::now();
        let tracks = fetch(&library, ONE).await.unwrap();
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
            .with(always(), eq(0), eq(PAGE_SIZE))
            .returning(|_, _, _| ready(Ok(page(150, 0))));
        let mut sequence = Sequence::new();
        library
            .expect_saved_tracks()
            .with(always(), eq(50), eq(PAGE_SIZE))
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, _, _| ready(Err(rate_limited(Some(5)))));
        library
            .expect_saved_tracks()
            .with(always(), eq(50), eq(PAGE_SIZE))
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, _, _| ready(Ok(page(150, 50))));
        library
            .expect_saved_tracks()
            .with(always(), eq(100), eq(PAGE_SIZE))
            .times(1)
            .returning(move |_, _, _| {
                assert!(
                    Instant::now() >= paused_until,
                    "requested while rate limited"
                );
                ready(Ok(page(150, 100)))
            });
        let tracks = fetch(&library, NonZeroUsize::new(2).unwrap())
            .await
            .unwrap();
        assert_eq!(tracks.len(), 150);
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_without_retry_after_backs_off() {
        let mut library = MockLibrary::new();
        let mut sequence = Sequence::new();
        library
            .expect_saved_tracks()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, _, _| ready(Err(rate_limited(None))));
        library
            .expect_saved_tracks()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, offset, _| ready(Ok(page(1, offset))));
        let started = Instant::now();
        fetch(&library, ONE).await.unwrap();
        assert_eq!(started.elapsed(), Duration::from_secs(1));
    }

    #[tokio::test(start_paused = true)]
    async fn overly_long_retry_after_aborts() {
        let mut library = MockLibrary::new();
        library
            .expect_saved_tracks()
            .times(1)
            .returning(|_, _, _| ready(Err(rate_limited(Some(3600)))));
        let error = fetch(&library, ONE).await.unwrap_err();
        assert!(matches!(
            error.source,
            RetryError::RetryAfterTooLong { wait } if wait == Duration::from_secs(3600)
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn server_errors_are_retried_with_backoff() {
        let mut library = MockLibrary::new();
        let mut sequence = Sequence::new();
        library
            .expect_saved_tracks()
            .times(3)
            .in_sequence(&mut sequence)
            .returning(|_, _, _| ready(Err(ApiError::Server { status: 503 })));
        library
            .expect_saved_tracks()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, offset, _| ready(Ok(page(1, offset))));
        let started = Instant::now();
        fetch(&library, ONE).await.unwrap();
        assert_eq!(started.elapsed(), Duration::from_secs(1 + 2 + 4));
    }

    #[tokio::test(start_paused = true)]
    async fn server_errors_give_up_after_the_retries() {
        let mut library = MockLibrary::new();
        library
            .expect_saved_tracks()
            .times(5)
            .returning(|_, _, _| ready(Err(ApiError::Server { status: 500 })));
        let error = fetch(&library, ONE).await.unwrap_err();
        assert_eq!(error.offset, 0);
        assert!(matches!(
            error.source,
            RetryError::GaveUp {
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
            .with(always(), eq(0), eq(PAGE_SIZE))
            .returning(|_, _, _| ready(Ok(page(80, 0))));
        library
            .expect_saved_tracks()
            .with(always(), eq(50), eq(PAGE_SIZE))
            .times(1)
            .returning(|_, _, _| ready(Err(ApiError::Unauthorized)));
        let error = fetch(&library, ONE).await.unwrap_err();
        assert_eq!(error.offset, 50);
        assert!(matches!(
            error.source,
            RetryError::Permanent(ApiError::Unauthorized)
        ));
    }
}
