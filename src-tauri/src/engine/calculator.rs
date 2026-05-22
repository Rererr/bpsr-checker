use crate::engine::combat_stats::{CombatStats, process_stats};
use crate::protocol::pb::DamageRecord;

/// CombatStats への集計を抽象化するためのトレイト。
pub trait StatisticsCalculator {
    fn apply(&self, record: &DamageRecord, stats: &mut CombatStats);
}

/// 既定の集計実装。process_stats に委譲する。
pub struct DefaultCalculator;

impl StatisticsCalculator for DefaultCalculator {
    fn apply(&self, record: &DamageRecord, stats: &mut CombatStats) {
        process_stats(record, stats);
    }
}
