use crate::backoff::default_max_attempts;

use super::Strategy;
use duration_string::DurationString;
use rand::Rng;
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

fn jitter(duration: Duration) -> Duration {
    let nanos = duration.as_nanos();
    if nanos == 0 {
        return duration;
    }
    let jittered = rand::rng().random_range(0..=nanos);
    Duration::from_nanos(jittered as u64)
}

impl Strategy for Config {
    fn get_strategy(&self) -> Box<dyn Iterator<Item = Duration> + Send> {
        let base = self.base;
        let factor = self.factor;
        let max_delay: Duration = self.max_delay.into();
        let jitter_flag = self.jitter;

        let iter = (0..self.max_attempts).map(move |attempt| {
            let delay =
                Duration::from_millis(base.saturating_mul(factor.saturating_pow(attempt as u32)));
            let capped = delay.min(max_delay);
            if jitter_flag { jitter(capped) } else { capped }
        });

        Box::new(iter)
    }
}
