use super::Strategy;
use duration_string::DurationString;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio_retry::strategy::FixedInterval;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
#[serde(default)]
pub struct Config {
    #[schemars(with = "String")]
    interval: DurationString,
}
impl Default for Config {
    fn default() -> Self {
        Config {
            interval: Duration::from_millis(1000).into(),
        }
    }
}

impl Strategy for Config {
    fn get_strategy(&self) -> Box<dyn Iterator<Item = Duration> + Send> {
        Box::new(FixedInterval::new(self.interval.into()))
    }
}
