use crate::models::TimeSeriesPoint;
use crate::engine::buff_tracker::BuffTracker;
use crate::engine::combat_stats::{ActiveTime, CombatStats};
use crate::engine::entity::Entity;
use crate::protocol::pb::EntityKind;
use std::collections::{HashMap, HashSet, VecDeque};

pub type EncounterMutex = std::sync::Mutex<Encounter>;

#[derive(Debug, Clone, Default)]
pub enum MeasureMode {
    #[default]
    Normal,
    Pending3Min {
        duration_ms: u128,
    },
    Active3Min {
        armed_at_ms: u128,
        duration_ms: u128,
    },
}

#[derive(Debug, Default, Clone)]
pub struct Encounter {
    pub is_paused: bool,
    pub time_fight_start_ms: u128,
    pub time_last_combat_packet_ms: u128,
    pub entities: HashMap<i64, Entity>,
    pub dmg_stats: CombatStats,
    pub dmg_stats_boss_only: CombatStats,
    pub heal_stats: CombatStats,
    pub dmg_taken_stats: CombatStats,
    pub time_series: VecDeque<TimeSeriesPoint>,
    pub last_sample_ms: u128,
    pub last_sample_total_dmg: i64,
    pub local_player_uid: i64,
    pub has_selected_participant: bool,
    pub participant_player_uids: HashSet<i64>,
    pub measure_mode: MeasureMode,
    pub active_connection: Option<crate::capture::server::Server>,
    pub conn_to_uid: std::collections::HashMap<crate::capture::server::Server, i64>,
    pub buff_tracker: BuffTracker,
    /// 食事/シロップバフの残時間ストア。clear_combat_stats・手動リセットでは消さず
    /// （戦闘終了後もゲーム内効果は継続）、自然失効・履歴クリアでのみ消す。
    /// consumables.json にディスク永続化され、アプリ再起動後に復元される。
    pub consumables: std::collections::HashMap<i64, crate::engine::consumables::PlayerConsumables>,
}

impl Encounter {
    /// 「戦闘中」判定の純粋版（`timeout_ms` を明示注入できる。テスト用途）。
    /// 最終着弾(`time_last_combat_packet_ms`)から `timeout_ms` 以内なら戦闘中とみなす
    /// （processor.rs の旧ロールオーバー判定 `diff > timeout_ms` と境界を一致させるため
    /// `<=` を使う。`<` にすると diff == timeout_ms の1点だけロールオーバーが早まる）。
    /// `timeout_ms == 0`（タイムアウト無効化設定）のときは常に戦闘中とみなす
    /// （ロールオーバー自体が発火しない既存仕様と合わせる）。一度も戦闘していなければ常に false。
    pub fn is_combat_active_with_timeout(&self, now: u128, timeout_ms: u128) -> bool {
        if self.time_last_combat_packet_ms == 0 {
            return false;
        }
        if timeout_ms == 0 {
            return true;
        }
        now.saturating_sub(self.time_last_combat_packet_ms) <= timeout_ms
    }

    /// 「戦闘中」判定（実運用版・`runtime_settings::COMBAT_EXIT_TIMEOUT_MS` を使う）。
    /// processor.rs のロールオーバー判定が呼ぶ唯一の定義（他に同じ述語を書かない）。
    /// ライブ表示の分母（compute.rs）は「戦闘中は現在時刻を分母にする」設計を撤回済みのため
    /// 本メソッドを参照しない（通常モードは実測スパン固定、3分計測は armed_at 基準）。
    pub fn is_combat_active(&self, now: u128) -> bool {
        let timeout_ms = u128::from(
            crate::engine::runtime_settings::COMBAT_EXIT_TIMEOUT_MS
                .load(std::sync::atomic::Ordering::Relaxed),
        );
        self.is_combat_active_with_timeout(now, timeout_ms)
    }

    /// Player entities are removed so their identity is re-populated fresh
    /// from disk cache on next appearance. Monster entities are kept with stats
    /// reset so HP/monster_id tracking survives the rollover.
    pub fn clear_combat_stats(&mut self) {
        // active_connection と conn_to_uid は保持する。
        // これらはセッション間でコネクション識別に再利用するため、
        // ServerHandover 受信時と set_selected_uid 変更時のみクリアする。
        self.is_paused = false;
        self.time_fight_start_ms = 0;
        self.time_last_combat_packet_ms = 0;
        self.dmg_stats = CombatStats::default();
        self.dmg_stats_boss_only = CombatStats::default();
        self.heal_stats = CombatStats::default();
        self.dmg_taken_stats = CombatStats::default();
        self.time_series.clear();
        self.last_sample_ms = 0;
        self.last_sample_total_dmg = 0;
        self.has_selected_participant = false;
        self.participant_player_uids.clear();
        self.buff_tracker.clear();
        self.entities
            .retain(|_, entity| entity.entity_type != EntityKind::Player);
        for entity in self.entities.values_mut() {
            entity.dmg_stats = CombatStats::default();
            entity.dmg_stats_boss_only = CombatStats::default();
            entity.heal_stats = CombatStats::default();
            entity.dmg_taken_stats = CombatStats::default();
            entity.active_dmg_time = ActiveTime::default();
            entity.skill_uid_to_dps_stats.clear();
            entity.skill_uid_to_dps_stats_boss_only.clear();
            entity.skill_uid_to_heal_stats.clear();
            entity.skill_meta.clear();
            entity.attacker_uid_to_dmg_taken_stats.clear();
            entity.attacker_skill_to_dmg_taken_stats.clear();
            entity.time_series.clear();
            entity.last_sample_total_dmg = 0;
            entity.heal_time_series.clear();
            entity.last_sample_total_heal = 0;
            entity.dmg_taken_time_series.clear();
            entity.last_sample_total_dmg_taken = 0;
            entity.skill_time_series.clear();
            entity.skill_last_sample_total_dmg.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // timeout_ms==0（タイムアウト無効化設定）はどれだけ経過しても常に戦闘中とみなす
    // （processor.rs のロールオーバー自体が発火しない既存仕様と合わせる）。
    #[test]
    fn is_combat_active_with_timeout_never_expires_when_disabled() {
        let enc = Encounter {
            time_last_combat_packet_ms: 1_000,
            ..Default::default()
        };
        assert!(enc.is_combat_active_with_timeout(1_000_000_000, 0));
    }

    // 一度も戦闘していなければ（time_last_combat_packet_ms==0）常に false。
    #[test]
    fn is_combat_active_with_timeout_false_when_never_fought() {
        let enc = Encounter::default();
        assert!(!enc.is_combat_active_with_timeout(999_999, 8_000));
    }

    // 境界は processor.rs の旧ロールオーバー判定 `diff > timeout_ms` と一致させるため `<=`。
    // diff == timeout_ms はまだ戦闘中（ロールオーバーしない）、diff > timeout_ms で戦闘終了。
    #[test]
    fn is_combat_active_with_timeout_boundary_matches_legacy_gt_comparison() {
        let enc = Encounter {
            time_last_combat_packet_ms: 1_000,
            ..Default::default()
        };
        assert!(enc.is_combat_active_with_timeout(1_000 + 8_000, 8_000), "diff==timeoutはまだ戦闘中");
        assert!(!enc.is_combat_active_with_timeout(1_000 + 8_001, 8_000), "diff>timeoutで戦闘終了");
    }

    // 有効DPS（実働時間ベース）のトラッカー(Entity::active_dmg_time)は
    // clear_combat_stats で他の集計と同じく0へ戻る（Encounter 側にも同名フィールドが
    // あったが、production コードから一度も読まれなかったため削除済み）。
    #[test]
    fn clear_combat_stats_resets_active_dmg_time() {
        let mut enc = Encounter::default();
        let entity = enc.entities.entry(1).or_default();
        entity.active_dmg_time.record_event(1_000);
        entity.active_dmg_time.record_event(2_000);
        assert_ne!(entity.active_dmg_time.active_ms, 0);

        enc.clear_combat_stats();

        // モンスターは entities に残るがプレイヤーは除去されるため、モンスターで確認する。
        let entity = enc.entities.entry(1).or_default();
        assert_eq!(entity.active_dmg_time.active_ms, 0);
    }
}
