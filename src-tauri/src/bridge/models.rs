/*
f64 is used in the models even when it doesn't make sense due to limitations with
serde serializing u128 as a JSON number instead of a string.
*/

#[derive(specta::Type, serde::Serialize, serde::Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HeaderInfo {
    pub total_dps: f64,
    pub total_dmg: f64,
    pub elapsed_ms: f64,
    pub time_last_combat_packet_ms: f64,
}

pub type PlayerRows = Vec<PlayerRow>;

#[derive(specta::Type, serde::Serialize, serde::Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlayersWindow {
    pub player_rows: PlayerRows,
    pub local_player_uid: f64,
    pub top_value: f64,
}

#[derive(specta::Type, serde::Serialize, serde::Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlayerRow {
    pub uid: f64,
    pub name: String,
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
    pub time_series: Vec<TimeSeriesPoint>,
}

#[derive(specta::Type, serde::Serialize, serde::Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TimeSeriesPoint {
    pub t_ms: f64,
    pub total_dmg: f64,
    pub total_dps: f64,
}

#[derive(specta::Type, serde::Serialize, serde::Deserialize, Debug, Default, Clone)]
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

#[derive(specta::Type, serde::Serialize, Clone, Debug)]
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

#[derive(specta::Type, serde::Serialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlayerBuffSnapshot {
    pub uid: f64,
    pub name: String,
    pub buffs: Vec<SelfBuffSnapshot>,
}

#[derive(specta::Type, serde::Serialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TrackedBuffsData {
    pub players: Vec<PlayerBuffSnapshot>,
    pub now_ms: f64,
    pub local_player_uid: f64,
}

#[derive(serde::Serialize, specta::Type, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MeasureModeStatus {
    pub kind: String,
    pub remaining_ms: Option<f64>,
    pub duration_ms: Option<f64>,
    pub armed_at_ms: Option<f64>,
}

pub type SkillRows = Vec<SkillRow>;

#[derive(specta::Type, serde::Serialize, serde::Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SkillsWindow {
    pub inspected_player: PlayerRow,
    pub skill_rows: SkillRows,
    pub local_player_uid: f64,
    pub top_value: f64,
}

#[derive(specta::Type, serde::Serialize, serde::Deserialize, Debug, Default, Clone)]
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
}
