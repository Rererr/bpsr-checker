use crate::bridge::models::TimeSeriesPoint;
use crate::engine::combat_stats::CombatStats;
use crate::engine::entity::Entity;
use std::collections::{HashMap, HashSet, VecDeque};

pub type EncounterMutex = std::sync::Mutex<Encounter>;

#[derive(Debug, Default, Clone)]
pub struct Encounter {
    pub is_paused: bool,
    pub time_fight_start_ms: u128,
    pub time_last_combat_packet_ms: u128,
    pub entities: HashMap<i64, Entity>,
    pub dmg_stats: CombatStats,
    pub dmg_stats_boss_only: CombatStats,
    pub heal_stats: CombatStats,
    pub time_series: VecDeque<TimeSeriesPoint>,
    pub last_sample_ms: u128,
    pub last_sample_total_dmg: i64,
    pub local_player_uid: i64,
    pub has_selected_participant: bool,
    pub participant_player_uids: HashSet<i64>,
}

impl Encounter {
    /// Reset combat statistics while keeping each entity's identity (name,
    /// class, ability score, monster id, etc.) intact. Used for both manual
    /// reset and automatic encounter rollover so player names don't get
    /// replaced by `プレイヤー#XXXX` fallbacks after a reset.
    pub fn clear_combat_stats(&mut self) {
        self.is_paused = false;
        self.time_fight_start_ms = 0;
        self.time_last_combat_packet_ms = 0;
        self.dmg_stats = CombatStats::default();
        self.dmg_stats_boss_only = CombatStats::default();
        self.heal_stats = CombatStats::default();
        self.time_series.clear();
        self.last_sample_ms = 0;
        self.last_sample_total_dmg = 0;
        self.has_selected_participant = false;
        self.participant_player_uids.clear();
        for entity in self.entities.values_mut() {
            entity.dmg_stats = CombatStats::default();
            entity.dmg_stats_boss_only = CombatStats::default();
            entity.heal_stats = CombatStats::default();
            entity.skill_uid_to_dps_stats.clear();
            entity.skill_uid_to_dps_stats_boss_only.clear();
            entity.skill_uid_to_heal_stats.clear();
            entity.time_series.clear();
            entity.last_sample_total_dmg = 0;
        }
    }
}
