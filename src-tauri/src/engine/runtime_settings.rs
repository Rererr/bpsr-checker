use std::sync::atomic::{AtomicU64, AtomicUsize};

pub static COMBAT_EXIT_TIMEOUT_MS: AtomicU64 = AtomicU64::new(8000);
pub static HISTORY_LIMIT: AtomicUsize = AtomicUsize::new(20);
pub static TS_INTERVAL_MS: AtomicU64 = AtomicU64::new(1000);
pub static TS_SAMPLES: AtomicUsize = AtomicUsize::new(60);
