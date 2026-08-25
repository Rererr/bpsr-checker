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
    /// 全項目0の定数。`&'static CombatStats` を返したい場面（集計対象がまだ存在しないときの
    /// 母集団など）で使う。`Default::default()` は const 文脈で参照を取れないため。
    pub const ZERO: CombatStats = CombatStats {
        total: 0,
        hit_count: 0,
        crit_count: 0,
        crit_value: 0,
        lucky_count: 0,
        lucky_value: 0,
        normal_value: 0,
    };

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

/// 有効DPS（実働時間ベース）の間隔キャップ。resonance-logs-cn 採用値の踏襲
/// （StarResonanceDps も同じ3秒キャップ）: 直前のダメージイベントからの間隔が
/// この値以内ならそのまま実働時間へ加算し、超えたら [`ACTIVE_TIME_GAP_GRACE_MS`] の
/// 猶予のみ加算する（長い無操作は実質切り捨て、通常DPSの分母には一切影響しない）。
pub const ACTIVE_TIME_GAP_CAP_MS: u128 = 3_000;
/// [`ACTIVE_TIME_GAP_CAP_MS`] を超えた間隔に対して積む猶予（ms）。
pub const ACTIVE_TIME_GAP_GRACE_MS: u128 = 500;

/// 有効DPS（実働時間ベース）の分母を積み上げるトラッカー。`Entity` がプレイヤーごとに
/// 1つ持ち、「同一対象の前回ダメージイベントからの間隔」を積算する。実測スパンでの
/// クランプはここでは行わない（このトラッカー自身は実測スパンを知らないため）。
/// 表示側（`compute::make_player_row`）が `elapsed_secs` を使ってクランプする。
#[derive(Debug, Default, Clone, Copy)]
pub struct ActiveTime {
    /// 実働時間の累計(ms)。有効DPS = total / (active_ms / 1000)。
    pub active_ms: u128,
    /// 直前イベントの ts。0=まだ1件も記録していない（他の `*_ms` フィールドと同じ sentinel 規約）。
    last_event_ms: u128,
}

impl ActiveTime {
    /// ダメージイベントを1件記録する。初回（直前イベントが無い）も含めて必ず
    /// [`ACTIVE_TIME_GAP_GRACE_MS`] の猶予を積む（resonance-logs-cn 踏襲。初回だけ0を
    /// 積むと「1発しか当てていない」プレイヤーの実働時間がいつまでも0のままになり、
    /// 有効DPSが常に0と表示されてしまうため）。2件目以降は間隔キャップに従って積む。
    /// `active_ms` 自体はここでは実測スパンでクランプしない（このトラッカーは対象の
    /// タイムスタンプしか知らず、実測スパン＝`Encounter::time_last_combat_packet_ms -
    /// time_fight_start_ms` を知らないため）。クランプは表示側
    /// （`compute::make_player_row`）が `elapsed_secs` を使って行う。
    pub fn record_event(&mut self, ts: u128) {
        self.active_ms += if self.last_event_ms == 0 {
            ACTIVE_TIME_GAP_GRACE_MS
        } else {
            let gap = ts.saturating_sub(self.last_event_ms);
            if gap <= ACTIVE_TIME_GAP_CAP_MS {
                gap
            } else {
                ACTIVE_TIME_GAP_GRACE_MS
            }
        };
        self.last_event_ms = ts;
    }
}

#[cfg(test)]
mod active_time_tests {
    use super::*;

    // 初回イベントも猶予を積む＝1発しか当てていないプレイヤーでも active_ms は0にならない
    // （有効DPSが常時0と表示される不具合の回帰防止。実測スパンでのクランプは
    // compute.rs 側の責務のためここでは検証しない）。
    #[test]
    fn first_event_grants_grace_not_zero() {
        let mut t = ActiveTime::default();
        t.record_event(1_000);
        assert_eq!(t.active_ms, ACTIVE_TIME_GAP_GRACE_MS);
    }

    #[test]
    fn interval_within_cap_is_added_in_full() {
        let mut t = ActiveTime::default();
        t.record_event(1_000); // 初回: 猶予500ms
        t.record_event(3_500); // 間隔2500ms（3秒以内）を全額加算
        assert_eq!(t.active_ms, ACTIVE_TIME_GAP_GRACE_MS + 2_500);
    }

    #[test]
    fn interval_at_cap_boundary_is_added_in_full() {
        let mut t = ActiveTime::default();
        t.record_event(1_000); // 初回: 猶予500ms
        t.record_event(1_000 + ACTIVE_TIME_GAP_CAP_MS);
        assert_eq!(t.active_ms, ACTIVE_TIME_GAP_GRACE_MS + ACTIVE_TIME_GAP_CAP_MS);
    }

    #[test]
    fn interval_beyond_cap_adds_only_grace() {
        let mut t = ActiveTime::default();
        t.record_event(1_000); // 初回: 猶予500ms
        t.record_event(1_000 + ACTIVE_TIME_GAP_CAP_MS + 1); // 3秒超→猶予のみ
        assert_eq!(t.active_ms, ACTIVE_TIME_GAP_GRACE_MS * 2);
    }

    #[test]
    fn accumulates_across_multiple_events() {
        let mut t = ActiveTime::default();
        t.record_event(0 + 1); // 初回: 猶予500ms
        t.record_event(1 + 1_000); // +1000（3秒以内）
        t.record_event(1_001 + 10_000); // +500（3秒超→猶予のみ）
        assert_eq!(t.active_ms, ACTIVE_TIME_GAP_GRACE_MS + 1_000 + ACTIVE_TIME_GAP_GRACE_MS);
    }
}
