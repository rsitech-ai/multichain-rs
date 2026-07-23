use std::sync::{
    Arc,
    atomic::{AtomicI64, AtomicU64, Ordering},
};

use observation_envelope::Clock;

/// A cloneable deterministic clock for tests and replay fixtures.
#[derive(Clone, Debug)]
pub struct FakeClock {
    wall_time_unix_ns: Arc<AtomicI64>,
    monotonic_ns: Arc<AtomicU64>,
}

impl FakeClock {
    /// Creates a clock with fixed wall and monotonic values.
    #[must_use]
    pub fn new(wall_time_unix_ns: i64, monotonic_ns: u64) -> Self {
        Self {
            wall_time_unix_ns: Arc::new(AtomicI64::new(wall_time_unix_ns)),
            monotonic_ns: Arc::new(AtomicU64::new(monotonic_ns)),
        }
    }

    /// Replaces the wall-clock value.
    pub fn set_wall_time_unix_ns(&self, value: i64) {
        self.wall_time_unix_ns.store(value, Ordering::SeqCst);
    }

    /// Replaces the monotonic value.
    pub fn set_monotonic_ns(&self, value: u64) {
        self.monotonic_ns.store(value, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn wall_time_unix_ns(&self) -> i64 {
        self.wall_time_unix_ns.load(Ordering::SeqCst)
    }

    fn monotonic_ns(&self) -> u64 {
        self.monotonic_ns.load(Ordering::SeqCst)
    }
}
