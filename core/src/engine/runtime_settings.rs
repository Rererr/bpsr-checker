use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

pub static COMBAT_EXIT_TIMEOUT_MS: AtomicU64 = AtomicU64::new(8000);
pub static HISTORY_LIMIT: AtomicUsize = AtomicUsize::new(20);
pub static TS_INTERVAL_MS: AtomicU64 = AtomicU64::new(1000);
pub static TS_SAMPLES: AtomicUsize = AtomicUsize::new(60);

/// ON のとき DPS / ヒール / スキル / 時系列の集計をすべて省略し、
/// バフ追跡（イマジンデバフタイマー）のみ動作させる軽量モード。
pub static IMAGINE_ONLY_MODE: AtomicBool = AtomicBool::new(false);

pub fn imagine_only_mode() -> bool {
    IMAGINE_ONLY_MODE.load(Ordering::Relaxed)
}
