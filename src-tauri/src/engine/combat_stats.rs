use crate::protocol::constants::damage;
use crate::protocol::pb::SyncDamageInfo;

#[derive(Debug, Default, Clone)]
pub struct CombatStats {
    pub total: i64,
    pub hit_count: u32,
    pub crit_count: u32,
    pub crit_value: i64,
    pub lucky_count: u32,
    pub lucky_value: i64,
    pub normal_value: i64,
}

impl CombatStats {
    pub fn record_hit(&mut self, value: i64, is_crit: bool, is_lucky: bool) {
        self.total += value;
        self.hit_count += 1;

        if is_crit {
            self.crit_count += 1;
            self.crit_value += value;
        }
        if is_lucky {
            self.lucky_count += 1;
            self.lucky_value += value;
        }
        if !is_crit && !is_lucky {
            self.normal_value += value;
        }
    }
}

/// Process a SyncDamageInfo packet and update the given CombatStats.
/// Prefers lucky_value over value (same logic as bpsr-logs).
pub fn process_stats(sync_damage_info: &SyncDamageInfo, stats: &mut CombatStats) {
    let actual_value = if sync_damage_info.lucky_value != 0 {
        sync_damage_info.lucky_value
    } else {
        sync_damage_info.value
    };

    let is_lucky = sync_damage_info.lucky_value != 0;
    let is_crit = (sync_damage_info.type_flag & damage::CRIT_BIT) != 0;

    stats.record_hit(actual_value, is_crit, is_lucky);
}
