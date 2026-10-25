//! Client-side rate limiting.
//!
//! VNDB documents roughly **200 requests per 5 minutes**, which averages to one
//! request every 1.5 s. This limiter enforces a minimum spacing between requests
//! rather than a burst budget: it is simpler to reason about, and it keeps the
//! app far away from the limit during interactive searching (a user types a few
//! characters, not a burst of hundreds).

use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

/// Default spacing between requests: 1.5 s, matching the documented average.
pub const DEFAULT_MIN_INTERVAL: Duration = Duration::from_millis(1_500);

/// Serialises requests so the client never exceeds the documented rate.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    next_slot: std::sync::Arc<Mutex<Instant>>,
    min_interval: Duration,
}

impl RateLimiter {
    /// A limiter with the documented average spacing.
    #[must_use]
    pub fn new() -> Self {
        Self::with_interval(DEFAULT_MIN_INTERVAL)
    }

    /// A limiter with an explicit spacing. Tests use a zero interval.
    #[must_use]
    pub fn with_interval(min_interval: Duration) -> Self {
        Self {
            next_slot: std::sync::Arc::new(Mutex::new(Instant::now())),
            min_interval,
        }
    }

    /// Wait until the next request slot and claim it.
    pub async fn acquire(&self) {
        if self.min_interval.is_zero() {
            return;
        }
        let mut slot = self.next_slot.lock().await;
        let now = Instant::now();
        if *slot > now {
            tokio::time::sleep_until(*slot).await;
        }
        *slot = Instant::now() + self.min_interval;
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn zero_interval_does_not_block() {
        let limiter = RateLimiter::with_interval(Duration::ZERO);
        let start = Instant::now();
        for _ in 0..50 {
            limiter.acquire().await;
        }
        assert!(start.elapsed() < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn spacing_is_enforced_between_requests() {
        let interval = Duration::from_millis(60);
        let limiter = RateLimiter::with_interval(interval);
        limiter.acquire().await;
        let start = Instant::now();
        limiter.acquire().await;
        assert!(
            start.elapsed() >= Duration::from_millis(40),
            "second acquire must wait roughly one interval"
        );
    }

    #[tokio::test]
    async fn clones_share_the_same_schedule() {
        let limiter = RateLimiter::with_interval(Duration::from_millis(50));
        let clone = limiter.clone();
        limiter.acquire().await;
        let start = Instant::now();
        clone.acquire().await;
        assert!(start.elapsed() >= Duration::from_millis(30));
    }

    #[test]
    fn default_interval_is_the_documented_average() {
        assert_eq!(DEFAULT_MIN_INTERVAL, Duration::from_millis(1_500));
    }
}
