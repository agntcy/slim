use super::Strategy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio_retry::strategy::FixedInterval;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
#[serde(default)]
pub struct Config {
    interval: u64,
}
impl Default for Config {
    fn default() -> Self {
        Config { interval: 1000 }
    }
}

impl Strategy for Config {
    fn get_strategy(&self) -> Box<dyn Iterator<Item = Duration> + Send> {
        Box::new(FixedInterval::from_millis(self.interval))
    }
}
