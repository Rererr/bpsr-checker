use crate::engine::combat_stats::CombatStats;
use crate::engine::entity::Entity;
use std::collections::HashMap;

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
}
