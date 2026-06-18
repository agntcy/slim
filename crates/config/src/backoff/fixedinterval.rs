use super::Strategy;
use crate::backoff::default_max_attempts;
use duration_string::DurationString;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio_retry::strategy::FixedInterval;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
#[serde(default)]
pub struct Config {
    #[schemars(with = "String")]
    pub interval: DurationString,
    #[serde(default = "default_max_attempts")]
    pub max_attempts: usize,
}

impl Config {
    pub fn new(interval: Duration, max_attempts: usize) -> Self {
        Config {
            interval: interval.into(),
            max_attempts,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            interval: Duration::from_millis(1000).into(),
            max_attempts: default_max_attempts(),
        }
    }
}

impl Strategy for Config {
    fn get_strategy(&self) -> Box<dyn Iterator<Item = Duration> + Send> {
        Box::new(FixedInterval::new(self.interval.into()).take(self.max_attempts))
    }
}
