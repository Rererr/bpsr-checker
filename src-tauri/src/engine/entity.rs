use crate::bridge::models::TimeSeriesPoint;
use crate::engine::class::{Class, ClassSpec};
use crate::engine::combat_stats::CombatStats;
use crate::protocol::pb::EEntityType;
use std::collections::{HashMap, VecDeque};

#[derive(Debug, Default, Clone)]
pub struct Entity {
    pub entity_type: EEntityType,

    pub dmg_stats: CombatStats,
    pub skill_uid_to_dps_stats: HashMap<i32, CombatStats>,

    pub dmg_stats_boss_only: CombatStats,
    pub skill_uid_to_dps_stats_boss_only: HashMap<i32, CombatStats>,

    pub heal_stats: CombatStats,
    pub skill_uid_to_heal_stats: HashMap<i32, CombatStats>,

    // Players
    pub name: Option<String>,
    pub class: Option<Class>,
    pub class_spec: Option<ClassSpec>,
    pub ability_score: Option<i32>,

    // Monsters
    pub monster_id: Option<u32>,
    pub curr_hp: Option<u64>,
    pub max_hp: Option<u64>,

    // Per-entity DPS time series (sampled alongside encounter-wide series)
    pub time_series: VecDeque<TimeSeriesPoint>,
    pub last_sample_total_dmg: i64,
}
