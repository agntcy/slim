// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

/// Compute the backoff delay for a given retry attempt.
///
/// Uses exponential backoff (`base × 2^(attempt-1)`) capped at 30 s.
///
/// | attempt | delay (base=200 ms) |
/// |---------|---------------------|
/// | 1       | 200 ms              |
/// | 2       | 400 ms              |
/// | 3       | 800 ms              |
/// | 4       | 1.6 s               |
/// | 5       | 3.2 s               |
/// | 6       | 6.4 s               |
/// | 7       | 12.8 s              |
/// | 8+      | 30 s (capped)       |
pub fn backoff_delay(attempt: usize, base: Duration) -> Duration {
    let exp = attempt.saturating_sub(1).min(7) as u32; // cap exponent at 7 → 128×
    base.saturating_mul(1u32 << exp)
        .min(Duration::from_secs(30))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_attempt_equals_base() {
        assert_eq!(
            backoff_delay(1, Duration::from_millis(200)),
            Duration::from_millis(200)
        );
    }

    #[test]
    fn doubles_each_attempt() {
        let base = Duration::from_millis(200);
        assert_eq!(backoff_delay(2, base), Duration::from_millis(400));
        assert_eq!(backoff_delay(3, base), Duration::from_millis(800));
        assert_eq!(backoff_delay(4, base), Duration::from_millis(1600));
    }

    #[test]
    fn capped_at_30s() {
        let base = Duration::from_millis(200);
        assert!(backoff_delay(8, base) <= Duration::from_secs(30));
        assert!(backoff_delay(100, base) <= Duration::from_secs(30));
    }

    #[test]
    fn attempt_zero_treated_as_one() {
        // saturating_sub(1) on 0 → exp=0 → duration=base
        assert_eq!(
            backoff_delay(0, Duration::from_millis(200)),
            Duration::from_millis(200)
        );
    }
}
