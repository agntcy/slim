pub mod exponential;
pub mod fixedinterval;

use std::future::Future;
use std::time::Duration;

pub trait Strategy {
    fn get_strategy(&self) -> Box<dyn Iterator<Item = Duration> + Send>;
}

pub(crate) fn default_max_attempts() -> usize {
    usize::MAX
}

pub async fn retry_if<I, F, Fut, T, E, C>(
    strategy: I,
    mut action: F,
    mut condition: C,
) -> Result<T, E>
where
    I: IntoIterator<Item = Duration>,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    C: FnMut(&E) -> bool,
{
    let mut iter = strategy.into_iter();

    loop {
        match action().await {
            Ok(val) => return Ok(val),
            Err(err) => {
                if !condition(&err) {
                    return Err(err);
                }
                match iter.next() {
                    Some(delay) => tokio::time::sleep(delay).await,
                    None => return Err(err),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn exponential_backoff_no_jitter() {
        let config = exponential::Config::new(10, 2, Duration::from_millis(500), 5, false);
        let delays: Vec<Duration> = config.get_strategy().collect();
        assert_eq!(
            delays,
            vec![
                Duration::from_millis(10),
                Duration::from_millis(20),
                Duration::from_millis(40),
                Duration::from_millis(80),
                Duration::from_millis(160),
            ]
        );
    }

    #[test]
    fn exponential_backoff_caps_at_max_delay() {
        let config = exponential::Config::new(100, 2, Duration::from_millis(300), 5, false);
        let delays: Vec<Duration> = config.get_strategy().collect();
        assert_eq!(
            delays,
            vec![
                Duration::from_millis(100),
                Duration::from_millis(200),
                Duration::from_millis(300),
                Duration::from_millis(300),
                Duration::from_millis(300),
            ]
        );
    }

    #[test]
    fn exponential_backoff_with_jitter_bounded() {
        let config = exponential::Config::new(100, 2, Duration::from_millis(1000), 4, true);
        for _ in 0..100 {
            let delays: Vec<Duration> = config.get_strategy().collect();
            assert_eq!(delays.len(), 4);
            assert!(delays[0] <= Duration::from_millis(100));
            assert!(delays[1] <= Duration::from_millis(200));
            assert!(delays[2] <= Duration::from_millis(400));
            assert!(delays[3] <= Duration::from_millis(800));
        }
    }

    #[test]
    fn fixed_interval_strategy() {
        let config = fixedinterval::Config::new(Duration::from_millis(250), 3);
        let delays: Vec<Duration> = config.get_strategy().collect();
        assert_eq!(
            delays,
            vec![
                Duration::from_millis(250),
                Duration::from_millis(250),
                Duration::from_millis(250),
            ]
        );
    }

    #[test]
    fn fixed_interval_respects_max_attempts() {
        let config = fixedinterval::Config::new(Duration::from_millis(100), 0);
        let delays: Vec<Duration> = config.get_strategy().collect();
        assert!(delays.is_empty());
    }

    #[tokio::test]
    async fn retry_if_succeeds_immediately() {
        let result: Result<u32, &str> = retry_if(
            vec![Duration::from_millis(10)],
            || async { Ok(42) },
            |_: &&str| true,
        )
        .await;
        assert_eq!(result, Ok(42));
    }

    #[tokio::test]
    async fn retry_if_retries_then_succeeds() {
        let attempts = AtomicUsize::new(0);
        let result: Result<u32, &str> = retry_if(
            vec![Duration::ZERO, Duration::ZERO, Duration::ZERO],
            || {
                let n = attempts.fetch_add(1, Ordering::SeqCst);
                async move { if n < 2 { Err("not yet") } else { Ok(99) } }
            },
            |_: &&str| true,
        )
        .await;
        assert_eq!(result, Ok(99));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retry_if_stops_on_non_retryable_error() {
        let attempts = AtomicUsize::new(0);
        let result: Result<u32, &str> = retry_if(
            vec![Duration::ZERO, Duration::ZERO],
            || {
                attempts.fetch_add(1, Ordering::SeqCst);
                async { Err("fatal") }
            },
            |_: &&str| false,
        )
        .await;
        assert_eq!(result, Err("fatal"));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retry_if_exhausts_strategy() {
        let attempts = AtomicUsize::new(0);
        let result: Result<u32, &str> = retry_if(
            vec![Duration::ZERO, Duration::ZERO],
            || {
                attempts.fetch_add(1, Ordering::SeqCst);
                async { Err("still failing") }
            },
            |_: &&str| true,
        )
        .await;
        assert_eq!(result, Err("still failing"));
        // initial attempt + 2 retries = 3
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }
}
