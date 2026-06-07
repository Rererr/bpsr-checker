use crate::bridge::models::TimeSeriesPoint;
use crate::engine::class::{Class, ClassSpec};
use crate::engine::combat_stats::CombatStats;
use crate::protocol::pb::EntityKind;
use std::collections::{HashMap, VecDeque};

#[derive(Debug, Default, Clone, Copy)]
pub struct SkillMeta {
    pub property: u8,
    pub damage_mode: u8,
}

#[derive(Debug, Default, Clone)]
pub struct Entity {
    pub entity_type: EntityKind,

    pub dmg_stats: CombatStats,
    pub skill_uid_to_dps_stats: HashMap<i32, CombatStats>,
    pub skill_meta: HashMap<i32, SkillMeta>,

    pub dmg_stats_boss_only: CombatStats,
    pub skill_uid_to_dps_stats_boss_only: HashMap<i32, CombatStats>,

    pub heal_stats: CombatStats,
    pub skill_uid_to_heal_stats: HashMap<i32, CombatStats>,

    pub dmg_taken_stats: CombatStats,
    pub attacker_uid_to_dmg_taken_stats: HashMap<i64, CombatStats>,
    pub attacker_skill_to_dmg_taken_stats: HashMap<(i64, i32), CombatStats>,

    // Players
    pub name: Option<String>,
    pub class: Option<Class>,
    pub class_spec: Option<ClassSpec>,
    pub ability_score: Option<i32>,
    pub season_level: Option<i32>,
    pub season_strength: Option<i32>,

    // Player combat stats (主に自キャラ。パケット attr から取得し戦闘中も追従する)
    // ※ 整数系: attack_power / defense_power / endurance / dexterity
    // ※ 割合系: attack_speed / haste / lucky は「値 / 100 = パーセント」(1% = 100)
    pub attack_power: Option<i32>,
    pub defense_power: Option<i32>,
    pub endurance: Option<i32>,
    pub dexterity: Option<i32>,
    pub attack_speed: Option<i32>,
    pub haste: Option<i32>,
    pub lucky: Option<i32>,

    // Monsters（curr_hp / max_hp は自キャラの HP にも流用する）
    pub monster_id: Option<u32>,
    pub curr_hp: Option<u64>,
    pub max_hp: Option<u64>,

    // Per-entity DPS time series (sampled alongside encounter-wide series)
    pub time_series: VecDeque<TimeSeriesPoint>,
    pub last_sample_total_dmg: i64,
}
