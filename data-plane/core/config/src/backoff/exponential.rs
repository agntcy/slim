use crate::backoff::default_max_attempts;

use super::Strategy;
use crate::backoff::{ExponentialBackoffIter, jitter};
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
        let ret = ExponentialBackoffIter::new(self.base, self.factor, self.max_delay.into())
            .take(self.max_attempts);
        let jitter_flag = self.jitter;

        Box::new(ret.map(move |d| if jitter_flag { jitter(d) } else { d }))
    }
}
