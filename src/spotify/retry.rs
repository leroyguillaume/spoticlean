//! Retry policy shared by every Web API call: 429 and 5xx handling.

use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use tokio::time::{Instant, sleep, sleep_until};
use tracing::warn;

use super::ApiError;

/// How hard to insist when Spotify fails or throttles.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Retries of a request after a 5xx or a network error.
    pub transient_retries: u32,
    /// Retries of a request after a 429.
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
pub enum RetryError {
    #[error(transparent)]
    Permanent(ApiError),
    #[error("gave up after {attempts} attempts")]
    GaveUp {
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
pub struct RateGate(Mutex<Option<Instant>>);

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

/// Runs `call` until it succeeds, fails permanently, or `policy` runs out.
pub async fn retrying<T, F>(
    gate: &RateGate,
    policy: &RetryPolicy,
    mut call: impl FnMut() -> F,
) -> Result<T, RetryError>
where
    F: Future<Output = Result<T, ApiError>>,
{
    let mut transient_failures = 0;
    let mut rate_limits = 0;
    loop {
        gate.wait().await;
        let error = match call().await {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        let attempts = transient_failures + rate_limits + 1;
        match error {
            ApiError::RateLimited { retry_after } => {
                rate_limits += 1;
                if rate_limits > policy.rate_limit_retries {
                    return Err(RetryError::GaveUp {
                        attempts,
                        source: error,
                    });
                }
                let wait = retry_after.unwrap_or_else(|| backoff(policy, rate_limits));
                if wait > policy.max_retry_after {
                    return Err(RetryError::RetryAfterTooLong { wait });
                }
                warn!(
                    wait_secs = wait.as_secs(),
                    "rate limited by Spotify, pausing requests"
                );
                gate.close_for(wait);
            }
            error if error.is_transient() => {
                transient_failures += 1;
                if transient_failures > policy.transient_retries {
                    return Err(RetryError::GaveUp {
                        attempts,
                        source: error,
                    });
                }
                let wait = backoff(policy, transient_failures);
                warn!(%error, wait_ms = wait.as_millis(), "transient failure, retrying");
                sleep(wait).await;
            }
            error => return Err(RetryError::Permanent(error)),
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

    #[tokio::test(start_paused = true)]
    async fn rate_gate_holds_until_the_latest_deadline() {
        let gate = RateGate::default();
        let started = Instant::now();
        gate.close_for(Duration::from_secs(3));
        gate.close_for(Duration::from_secs(1));
        gate.wait().await;
        assert_eq!(started.elapsed(), Duration::from_secs(3));
    }

    #[test]
    fn backoff_doubles_up_to_the_cap() {
        let policy = RetryPolicy::default();
        let waits: Vec<u64> = (1..=7).map(|n| backoff(&policy, n).as_secs()).collect();
        assert_eq!(waits, [1, 2, 4, 8, 16, 30, 30]);
        assert_eq!(backoff(&policy, 100), Duration::from_secs(30));
    }
}
