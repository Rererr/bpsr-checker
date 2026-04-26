use crate::engine::combat_stats::{CombatStats, process_stats};
use crate::protocol::pb::SyncDamageInfo;

/// Trait for anything that can process a damage/heal event into CombatStats.
pub trait StatisticsCalculator {
    fn apply(&self, sync_damage_info: &SyncDamageInfo, stats: &mut CombatStats);
}

/// Default calculator: applies process_stats logic (prefer lucky_value, check CRIT_BIT).
pub struct DefaultCalculator;

impl StatisticsCalculator for DefaultCalculator {
    fn apply(&self, sync_damage_info: &SyncDamageInfo, stats: &mut CombatStats) {
        process_stats(sync_damage_info, stats);
    }
}
