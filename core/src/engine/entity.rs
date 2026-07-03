use crate::models::TimeSeriesPoint;
use crate::engine::class::{Class, ClassSpec};
use crate::engine::combat_stats::CombatStats;
use crate::protocol::pb::EntityKind;
use std::collections::{HashMap, VecDeque};

/// バトルイマジンの装備枠数（SlotPositionId 7/8）。プレイヤーが同時に装備・表示できる
/// バトルイマジンは最大この数。`imagine_names` の上限として processor/compute が共有する。
pub const MAX_IMAGINE_NAMES: usize = 2;

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
    // 命名はゲーム内ステータス画面の表記に合わせる（物理/魔法攻撃力, ファスト=haste など）。
    // ※ 整数系: attack_power(物攻) / magic_attack(魔攻) / defense_power(物防) / magic_defense(魔防)
    //          / endurance(耐久) / strength(筋力) / intelligence(知力) / agility(敏捷)
    // ※ 割合系(値/100=%): attack_speed(攻撃速度) / cast_speed(詠唱速度) / haste(ファスト)
    //          / lucky(幸運) / crit_stat(会心) / versatility(万能) / resist(レジスト)
    //          / crit_dmg(会心ダメージ) / lucky_dmg(幸運の一撃倍率) / dexterity(器用さ)
    // ※ attack_power / defense_power / endurance / dexterity / attack_speed / haste / lucky は
    //   旧 probe で decode 配線済み。それ以外（magic_*/agility/crit_stat/versatility/resist/
    //   cast_speed/crit_dmg/lucky_dmg）は attr_id 未確定で decode 未配線（実機 probe で確定後に配線）。
    pub attack_power: Option<i32>,
    pub magic_attack: Option<i32>,
    pub defense_power: Option<i32>,
    pub magic_defense: Option<i32>,
    pub endurance: Option<i32>,
    pub strength: Option<i32>,
    pub intelligence: Option<i32>,
    pub agility: Option<i32>,
    pub dexterity: Option<i32>,
    pub attack_speed: Option<i32>,
    pub cast_speed: Option<i32>,
    pub haste: Option<i32>,
    pub lucky: Option<i32>,
    pub crit_stat: Option<i32>,
    pub versatility: Option<i32>,
    pub resist: Option<i32>,
    pub crit_dmg: Option<i32>,
    pub lucky_dmg: Option<i32>,

    /// 使用が確認できたバトルイマジンの表示名（召喚エンティティの `AttrSkillId` から解決・発見順・
    /// 重複なし）。compute/UI はこの Vec を読む。更新は processor の `try_attribute_summon_imagine` 経由、
    /// 永続は name_cache。詳細は [[imagine-2stage-display]]。
    pub imagine_names: Vec<String>,

    // Monsters（curr_hp / max_hp は自キャラの HP にも流用する）
    pub monster_id: Option<u32>,
    pub curr_hp: Option<u64>,
    pub max_hp: Option<u64>,

    // Per-entity DPS time series (sampled alongside encounter-wide series)
    pub time_series: VecDeque<TimeSeriesPoint>,
    pub last_sample_total_dmg: i64,

    // Per-skill DPS time series（スキル別の推移グラフ用。entity の time_series と同タイミングで採取）
    pub skill_time_series: HashMap<i32, VecDeque<TimeSeriesPoint>>,
    pub skill_last_sample_total_dmg: HashMap<i32, i64>,
}
