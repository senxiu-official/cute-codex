//! Opt-in retries only for failures while establishing a connection.
//!
//! A response (even an HTTP error), request-send ambiguity, or body/stream
//! failure always leaves this loop immediately. No response stream is replayed.

use crate::TransportError;
use std::collections::hash_map::RandomState;
use std::future::Future;
use std::hash::BuildHasher;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

#[derive(Clone, Debug, Default)]
pub(crate) enum ConnectRetry {
    #[default]
    Disabled,
    Enabled {
        redirect_observed: Arc<AtomicBool>,
        max_retries: u64,
    },
}

impl ConnectRetry {
    pub(crate) async fn run<T, F, Fut>(&self, mut operation: F) -> Result<T, TransportError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, TransportError>>,
    {
        let retries = match self {
            Self::Disabled => 0,
            Self::Enabled { max_retries, .. } => *max_retries,
        };
        for attempt in 0..=retries {
            match operation().await {
                Err(TransportError::Connection(_))
                    if attempt < retries
                        && matches!(self, Self::Enabled { redirect_observed, .. } if !redirect_observed.load(Ordering::Relaxed)) =>
                {
                    // RandomState supplies per-instance random seeds without a
                    // separate RNG dependency. Jitter never shortens the floor.
                    let jitter_ms = RandomState::new().hash_one(attempt) % 251;
                    let delay =
                        Duration::from_secs(1 << attempt) + Duration::from_millis(jitter_ms);
                    tracing::debug!(
                        attempt = attempt + 1,
                        retries,
                        delay_ms = delay.as_millis(),
                        "retrying connection establishment before HTTP response"
                    );
                    tokio::time::sleep(delay).await;
                }
                result => return result,
            }
        }
        unreachable!("the last connection attempt always returns its result")
    }
}
