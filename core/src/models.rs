/*
f64 is used in the models even when it doesn't make sense due to limitations with
serde serializing u128 as a JSON number instead of a string.
*/

#[derive(serde::Serialize, serde::Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HeaderInfo {
    pub total_dps: f64,
    pub total_dmg: f64,
    pub elapsed_ms: f64,
    pub time_last_combat_packet_ms: f64,
}

pub type PlayerRows = Vec<PlayerRow>;

#[derive(serde::Serialize, serde::Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlayersWindow {
    pub player_rows: PlayerRows,
    pub local_player_uid: f64,
    pub top_value: f64,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlayerRow {
    pub uid: f64,
    pub name: String,
    pub name_resolved: bool,
    pub class_name: String,
    pub class_spec_name: String,
    pub ability_score: f64,
    pub season_level: f64,
    pub season_strength: f64,
    // Stats
    pub total_value: f64,
    pub value_per_sec: f64,
    pub value_pct: f64,
    pub crit_rate: f64,
    pub crit_value_rate: f64,
    pub lucky_rate: f64,
    pub lucky_value_rate: f64,
    pub hits: f64,
    pub hits_per_minute: f64,
    // 食事/シロップ(錬金)バフの残時間・総時間（縦型タイマー用。0=未使用）
    #[serde(default)]
    pub food_remaining_ms: f64,
    #[serde(default)]
    pub food_duration_ms: f64,
    #[serde(default)]
    pub syrup_remaining_ms: f64,
    #[serde(default)]
    pub syrup_duration_ms: f64,
    // 使用中の食事/シロップの base_id（種類ラベル解決用。0=未使用）
    #[serde(default)]
    pub food_base_id: i32,
    #[serde(default)]
    pub syrup_base_id: i32,
    pub time_series: Vec<TimeSeriesPoint>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TimeSeriesPoint {
    pub t_ms: f64,
    pub total_dmg: f64,
    pub total_dps: f64,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EncounterSnapshot {
    pub id: f64,
    pub start_ms: f64,
    pub end_ms: f64,
    pub duration_ms: f64,
    pub total_dmg: f64,
    pub total_dps: f64,
    pub player_rows: Vec<PlayerRow>,
    pub time_series: Vec<TimeSeriesPoint>,
    #[serde(default)]
    pub participant_player_uids: Vec<f64>,
}

#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SelfBuffSnapshot {
    pub kind: String,
    pub base_id: i32,
    pub buff_uuid: i32,
    pub layer: i32,
    pub remaining_ms: i64,
    pub duration_ms: i64,
    pub received_at_ms: f64,
}

#[derive(serde::Serialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlayerBuffSnapshot {
    pub uid: f64,
    pub name: String,
    pub buffs: Vec<SelfBuffSnapshot>,
}

#[derive(serde::Serialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TrackedBuffsData {
    pub players: Vec<PlayerBuffSnapshot>,
    pub now_ms: f64,
    pub local_player_uid: f64,
}

#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SelfStatusEntry {
    pub instance_id: i64,
    pub base_id: i32,
    pub category: String,  // "buff" | "debuff" | "recovery" | "item" | "unknown"
    pub priority: String,  // "hidden" | "low" | "normal" | "high" | "alert"
    pub remaining_ms: i64,
    pub duration_ms: i64,
    pub layer: i32,
    pub source_config_id: i32,
}

#[derive(serde::Serialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SelfStatusData {
    pub buffs: Vec<SelfStatusEntry>,
    pub debuffs: Vec<SelfStatusEntry>,
    pub now_ms: f64,
    pub local_player_uid: f64,
}

/// 自キャラの戦闘ステータス（リアルタイム表示用）。
/// パケット attr 由来の「ステータス値」と、命中データ由来の「実測率」を別々に保持する。
/// ※ 割合系（attack_speed/haste/lucky/crit_stat/mastery/versatility/block）は「値 / 100 = %」。
/// ※ *_measured は戦闘の命中集計から算出した実測パーセント（0〜100）。
#[derive(serde::Serialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SelfStatsData {
    pub local_player_uid: f64,
    pub has_combat: bool,
    // HP
    pub curr_hp: Option<f64>,
    pub max_hp: Option<f64>,
    // 整数系ステータス（ゲーム内表記準拠）
    pub attack_power: Option<i32>,   // 物理攻撃力
    pub magic_attack: Option<i32>,   // 魔法攻撃力
    pub defense_power: Option<i32>,  // 物理防御力
    pub magic_defense: Option<i32>,  // 魔法防御力
    pub endurance: Option<i32>,      // 耐久力
    pub strength: Option<i32>,       // 筋力
    pub intelligence: Option<i32>,   // 知力
    pub agility: Option<i32>,        // 敏捷
    pub ability_score: Option<i32>,  // 能力スコア
    pub season_strength: Option<i32>, // 幻夢強度
    // 割合系ステータス（値 / 100 = %）
    pub attack_speed: Option<i32>,   // 攻撃速度
    pub cast_speed: Option<i32>,     // 詠唱速度
    pub haste: Option<i32>,          // ファスト
    pub lucky: Option<i32>,          // 幸運
    pub crit_stat: Option<i32>,      // 会心
    pub versatility: Option<i32>,    // 万能
    pub resist: Option<i32>,         // レジスト
    pub dexterity: Option<i32>,      // 器用さ
    pub crit_dmg: Option<i32>,       // 会心ダメージ
    pub lucky_dmg: Option<i32>,      // 幸運の一撃ダメージ倍率
    // 実測率（命中データ由来・%）
    pub crit_rate_measured: f64,
    pub lucky_rate_measured: f64,
}

#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MeasureModeStatus {
    pub kind: String,
    pub remaining_ms: Option<f64>,
    pub duration_ms: Option<f64>,
    pub armed_at_ms: Option<f64>,
}

pub type SkillRows = Vec<SkillRow>;

#[derive(serde::Serialize, serde::Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SkillsWindow {
    pub inspected_player: PlayerRow,
    pub skill_rows: SkillRows,
    pub local_player_uid: f64,
    pub top_value: f64,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SkillRow {
    pub uid: f64,
    pub name: String,
    pub element: u8,
    pub damage_mode: u8,
    // Stats
    pub total_value: f64,
    pub value_per_sec: f64,
    pub value_pct: f64,
    pub crit_rate: f64,
    pub crit_value_rate: f64,
    pub lucky_rate: f64,
    pub lucky_value_rate: f64,
    pub hits: f64,
    pub hits_per_minute: f64,
    // スキル別ダメージ推移（結果画面の折れ線グラフ用。3分計測の finalize 前に捕捉）
    #[serde(default)]
    pub time_series: Vec<TimeSeriesPoint>,
}
