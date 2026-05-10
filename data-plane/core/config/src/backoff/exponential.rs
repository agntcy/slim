use crate::backoff::default_max_attempts;

use super::Strategy;
use duration_string::DurationString;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
#[serde(default)]
pub struct Config {
    pub base: u64,
    pub factor: u64,
    #[schemars(with = "String")]
    pub max_delay: DurationString,
    #[serde(default = "default_max_attempts")]
    pub max_attempts: usize,
    #[serde(default)]
    pub jitter: bool,
}

impl Config {
    pub fn new(
        base: u64,
        factor: u64,
        max_delay: Duration,
        max_attempts: usize,
        jitter: bool,
    ) -> Self {
        Config {
            base,
            factor,
            max_delay: max_delay.into(),
            max_attempts,
            jitter,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            base: 100,
            factor: 1,
            max_delay: Duration::from_millis(1000).into(),
            max_attempts: default_max_attempts(),
            jitter: true,
        }
    }
}

impl Strategy for Config {
    fn get_strategy(&self) -> Box<dyn Iterator<Item = Duration> + Send> {
        let base = self.base;
        let factor = self.factor;
        let max_delay: Duration = self.max_delay.into();
        let max_attempts = self.max_attempts;
        let jitter = self.jitter;

        Box::new((0..max_attempts).scan(base, move |current, _| {
            let delay_ms = (*current).min(max_delay.as_millis() as u64);
            let next = if factor == 0 {
                0
            } else {
                current.saturating_mul(factor)
            };
            *current = next.max(base);

            let delay = Duration::from_millis(delay_ms);
            Some(if jitter { apply_jitter(delay) } else { delay })
        }))
    }
}

fn apply_jitter(delay: Duration) -> Duration {
    if delay.is_zero() {
        return delay;
    }

    let cap = delay.as_millis() as u64;
    let jitter: u64 = rand::random::<u64>() % (cap + 1);
    Duration::from_millis(jitter)
}
