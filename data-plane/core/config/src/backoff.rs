pub mod exponential;
pub mod fixedinterval;

use std::time::Duration;

pub trait Strategy {
    fn get_strategy(&self) -> Box<dyn Iterator<Item = Duration> + Send>;
}

pub(crate) fn default_max_attempts() -> usize {
    usize::MAX
}

/// Produces durations growing as `base_ms × factor^n`, capped at `max_delay`.
pub(crate) struct ExponentialBackoffIter {
    current_ms: u64,
    factor: u64,
    max_delay_ms: u64,
}

impl ExponentialBackoffIter {
    pub(crate) fn new(base_ms: u64, factor: u64, max_delay: Duration) -> Self {
        Self {
            current_ms: base_ms,
            factor,
            max_delay_ms: max_delay.as_millis() as u64,
        }
    }
}

impl Iterator for ExponentialBackoffIter {
    type Item = Duration;

    fn next(&mut self) -> Option<Duration> {
        let delay = self.current_ms.min(self.max_delay_ms);
        self.current_ms = self
            .current_ms
            .saturating_mul(self.factor)
            .min(self.max_delay_ms);
        Some(Duration::from_millis(delay))
    }
}

/// Produces the same duration on every call.
pub(crate) struct FixedIntervalIter {
    interval: Duration,
}

impl FixedIntervalIter {
    pub(crate) fn new(interval: Duration) -> Self {
        Self { interval }
    }
}

impl Iterator for FixedIntervalIter {
    type Item = Duration;

    fn next(&mut self) -> Option<Duration> {
        Some(self.interval)
    }
}

/// Adds a uniformly random amount in `[0, duration)` to prevent synchronized retries.
pub(crate) fn jitter(duration: Duration) -> Duration {
    if duration.is_zero() {
        return duration;
    }
    use rand::Rng;
    let millis = duration.as_millis() as u64;
    let extra = rand::rng().random_range(0..millis);
    duration + Duration::from_millis(extra)
}

/// Calls `action` repeatedly, waiting between attempts according to `strategy`,
/// as long as `action` returns an error for which `should_retry` returns `true`.
/// Returns the first successful output, or the last error when the strategy is
/// exhausted or `should_retry` returns `false`.
pub(crate) async fn retry_if<S, F, Fut, T, E, C>(
    mut strategy: S,
    mut action: F,
    should_retry: C,
) -> Result<T, E>
where
    S: Iterator<Item = Duration>,
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    C: Fn(&E) -> bool,
{
    loop {
        match action().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                if !should_retry(&e) {
                    return Err(e);
                }
                match strategy.next() {
                    Some(delay) => tokio::time::sleep(delay).await,
                    None => return Err(e),
                }
            }
        }
    }
}
