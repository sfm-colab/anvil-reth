use parking_lot::RwLock;
use std::sync::Arc;

/// Manages block timestamp overrides.
#[derive(Clone, Default, Debug)]
pub struct TimeManager {
    /// The interval to use when determining the next block's timestamp.
    interval: Arc<RwLock<Option<u64>>>,
}

impl TimeManager {
    /// Sets the interval to use when determining the next block's timestamp.
    pub fn set_block_timestamp_interval(&self, interval: u64) {
        self.interval.write().replace(interval);
    }

    /// Removes the interval if it exists, returning whether one was removed.
    pub fn remove_block_timestamp_interval(&self) -> bool {
        self.interval.write().take().is_some()
    }
}
