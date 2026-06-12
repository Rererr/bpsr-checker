use crate::models::TimeSeriesPoint;
use crate::engine::buff_tracker::BuffTracker;
use crate::engine::combat_stats::CombatStats;
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
    /// 食事/シロップバフの残時間ストア。clear_combat_stats では消さず（戦闘終了後も
    /// ゲーム内効果は継続）、自然失効・手動リセット・履歴クリアで消す。
    pub consumables: std::collections::HashMap<i64, crate::engine::consumables::PlayerConsumables>,
}

impl Encounter {
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
            entity.skill_uid_to_dps_stats.clear();
            entity.skill_uid_to_dps_stats_boss_only.clear();
            entity.skill_uid_to_heal_stats.clear();
            entity.skill_meta.clear();
            entity.attacker_uid_to_dmg_taken_stats.clear();
            entity.attacker_skill_to_dmg_taken_stats.clear();
            entity.time_series.clear();
            entity.last_sample_total_dmg = 0;
            entity.skill_time_series.clear();
            entity.skill_last_sample_total_dmg.clear();
        }
    }
}
