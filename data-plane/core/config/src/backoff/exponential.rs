use super::Strategy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio_retry::strategy::{ExponentialBackoff, jitter};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
#[serde(default)]
pub struct Config {
    base: u64,
    factor: u64,
    max_delay: u64,
    jitter: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            base: 100,
            factor: 2,
            max_delay: 1000,
            jitter: true,
        }
    }
}

impl Strategy for Config {
    fn get_strategy(&self) -> Box<dyn Iterator<Item = Duration> + Send> {
        if self.jitter {
            Box::new(
                ExponentialBackoff::from_millis(self.base)
                    .factor(self.factor)
                    .max_delay(Duration::from_secs(self.max_delay))
                    .map(jitter),
            )
        } else {
            Box::new(
                ExponentialBackoff::from_millis(self.base)
                    .factor(self.factor)
                    .max_delay(Duration::from_secs(self.max_delay)),
            )
        }
    }
}
