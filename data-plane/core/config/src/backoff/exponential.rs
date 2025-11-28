use super::Strategy;
use duration_string::DurationString;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio_retry::strategy::{ExponentialBackoff, jitter};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
#[serde(default)]
pub struct Config {
    base: u64,
    factor: u64,
    #[schemars(with = "String")]
    max_delay: DurationString,
    jitter: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            base: 100,
            factor: 1,
            max_delay: Duration::from_millis(1000).into(),
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
                    .max_delay(self.max_delay.into())
                    .map(jitter),
            )
        } else {
            Box::new(
                ExponentialBackoff::from_millis(self.base)
                    .factor(self.factor)
                    .max_delay(self.max_delay.into()),
            )
        }
    }
}
