pub mod exponential;
pub mod fixedinterval;

use std::time::Duration;

pub trait Strategy {
    fn get_strategy(&self) -> Box<dyn Iterator<Item = Duration> + Send>;
}

pub(crate) fn default_max_attempts() -> usize {
    usize::MAX
}
