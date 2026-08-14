use crate::protocol::constants::damage;
use crate::protocol::pb::DamageRecord;

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

/// DamageRecord から実際に採用する値を導出する（lucky_value が立っているときは
/// value より優先して採用する）。集計本体(process_stats)と probe 計測の両方から呼び、
/// 「lucky優先」ルールの定義箇所を1つに保つ（processor.rs 側で複製しない）。
pub(crate) fn actual_value(record: &DamageRecord) -> i64 {
    if record.lucky_value != 0 {
        record.lucky_value
    } else {
        record.value
    }
}

/// 1件のダメージ記録を CombatStats に集計する。
/// lucky_value が立っているときは value より優先して採用する。
pub fn process_stats(record: &DamageRecord, stats: &mut CombatStats) {
    let is_lucky = record.lucky_value != 0;
    let is_crit = (record.type_flag & damage::CRIT_BIT) != 0;

    stats.record_hit(actual_value(record), is_crit, is_lucky);
}
