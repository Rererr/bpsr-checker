use crate::bridge::models::{EncounterSnapshot, HeaderInfo, PlayerRow, PlayersWindow, SkillRow, SkillsWindow, TimeSeriesPoint};
use crate::engine::class::{Class, ClassSpec};
use crate::engine::combat_stats::CombatStats;
use crate::engine::encounter::{Encounter, EncounterMutex};
use crate::engine::skill_names::get_skill_name;
use crate::protocol::pb::EEntityType;
use std::collections::VecDeque;
use log::info;
use tauri::AppHandle;

fn nan_is_zero(value: f64) -> f64 {
    if value.is_nan() || value.is_infinite() {
        0.0
    } else {
        value
    }
}

#[derive(Debug, Clone, Copy)]
enum StatType {
    Dmg,
    DmgBossOnly,
    Heal,
}

// ─── Header ──────────────────────────────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub fn get_header_info(state: tauri::State<'_, EncounterMutex>) -> HeaderInfo {
    let encounter = match state.lock() {
        Ok(e) => e,
        Err(e) => {
            log::error!("Lock poisoned in get_header_info: {e}");
            return HeaderInfo::default();
        }
    };

    let elapsed_ms = encounter
        .time_last_combat_packet_ms
        .saturating_sub(encounter.time_fight_start_ms);
    let elapsed_secs = elapsed_ms as f64 / 1000.0;

    HeaderInfo {
        total_dps: nan_is_zero(encounter.dmg_stats.total as f64 / elapsed_secs),
        total_dmg: encounter.dmg_stats.total as f64,
        elapsed_ms: elapsed_ms as f64,
        time_last_combat_packet_ms: encounter.time_last_combat_packet_ms as f64,
    }
}

// ─── Players windows ─────────────────────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub fn get_dps_players(state: tauri::State<'_, EncounterMutex>) -> PlayersWindow {
    let encounter = match state.lock() {
        Ok(e) => e,
        Err(e) => {
            log::error!("Lock poisoned in get_dps_players: {e}");
            return PlayersWindow::default();
        }
    };
    build_players_window(&*encounter, StatType::Dmg)
}

#[tauri::command]
#[specta::specta]
pub fn get_dps_boss_players(state: tauri::State<'_, EncounterMutex>) -> PlayersWindow {
    let encounter = match state.lock() {
        Ok(e) => e,
        Err(e) => {
            log::error!("Lock poisoned in get_dps_boss_players: {e}");
            return PlayersWindow::default();
        }
    };
    build_players_window(&*encounter, StatType::DmgBossOnly)
}

#[tauri::command]
#[specta::specta]
pub fn get_heal_players(state: tauri::State<'_, EncounterMutex>) -> PlayersWindow {
    let encounter = match state.lock() {
        Ok(e) => e,
        Err(e) => {
            log::error!("Lock poisoned in get_heal_players: {e}");
            return PlayersWindow::default();
        }
    };
    build_players_window(&*encounter, StatType::Heal)
}

fn build_players_window(
    encounter: &Encounter,
    stat_type: StatType,
) -> PlayersWindow {
    let elapsed_ms = encounter
        .time_last_combat_packet_ms
        .saturating_sub(encounter.time_fight_start_ms);
    let elapsed_secs = elapsed_ms as f64 / 1000.0;

    let encounter_stats = match stat_type {
        StatType::Dmg => &encounter.dmg_stats,
        StatType::DmgBossOnly => &encounter.dmg_stats_boss_only,
        StatType::Heal => &encounter.heal_stats,
    };

    let mut window = PlayersWindow {
        player_rows: Vec::new(),
        local_player_uid: encounter.local_player_uid as f64,
        top_value: 0.0,
    };

    for (&entity_uid, entity) in &encounter.entities {
        let entity_stats = match stat_type {
            StatType::Dmg => &entity.dmg_stats,
            StatType::DmgBossOnly => &entity.dmg_stats_boss_only,
            StatType::Heal => &entity.heal_stats,
        };

        if entity.entity_type != EEntityType::EntChar || entity_stats.total == 0 {
            continue;
        }

        window.top_value = window.top_value.max(entity_stats.total as f64);

        let row = make_player_row(
            entity_uid,
            entity.name.as_deref().unwrap_or(""),
            entity.class,
            entity.class_spec,
            entity.ability_score,
            entity_stats,
            encounter_stats,
            elapsed_secs,
            &entity.time_series,
        );
        window.player_rows.push(row);
    }

    window.player_rows.sort_by(|a, b| {
        b.total_value
            .partial_cmp(&a.total_value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    window
}

fn make_player_row(
    uid: i64,
    name: &str,
    class: Option<Class>,
    class_spec: Option<ClassSpec>,
    ability_score: Option<i32>,
    entity_stats: &CombatStats,
    encounter_stats: &CombatStats,
    elapsed_secs: f64,
    time_series: &VecDeque<TimeSeriesPoint>,
) -> PlayerRow {
    let display_name = if name.is_empty() {
        format!("プレイヤー#{}", uid & 0xFFFF)
    } else {
        name.to_string()
    };

    PlayerRow {
        uid: uid as f64,
        name: display_name,
        class_name: class.unwrap_or(Class::Unknown).name_ja().to_string(),
        class_spec_name: class_spec.unwrap_or(ClassSpec::Unknown).name_ja().to_string(),
        ability_score: f64::from(ability_score.unwrap_or(-1)),
        total_value: entity_stats.total as f64,
        value_per_sec: nan_is_zero(entity_stats.total as f64 / elapsed_secs),
        value_pct: nan_is_zero(
            entity_stats.total as f64 / encounter_stats.total as f64 * 100.0,
        ),
        crit_rate: nan_is_zero(
            entity_stats.crit_count as f64 / entity_stats.hit_count as f64 * 100.0,
        ),
        crit_value_rate: nan_is_zero(
            entity_stats.crit_value as f64 / entity_stats.total as f64 * 100.0,
        ),
        lucky_rate: nan_is_zero(
            entity_stats.lucky_count as f64 / entity_stats.hit_count as f64 * 100.0,
        ),
        lucky_value_rate: nan_is_zero(
            entity_stats.lucky_value as f64 / entity_stats.total as f64 * 100.0,
        ),
        hits: entity_stats.hit_count as f64,
        hits_per_minute: nan_is_zero(entity_stats.hit_count as f64 / elapsed_secs * 60.0),
        time_series: time_series.iter().cloned().collect(),
    }
}

// ─── Skills window ───────────────────────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub fn get_skills(
    state: tauri::State<'_, EncounterMutex>,
    player_uid: i64,
) -> Result<SkillsWindow, String> {
    let encounter = state
        .lock()
        .map_err(|e| format!("Lock poisoned: {e}"))?;

    let Some(player) = encounter.entities.get(&player_uid) else {
        return Err(format!("Could not find player with uid {player_uid}"));
    };

    let elapsed_ms = encounter
        .time_last_combat_packet_ms
        .saturating_sub(encounter.time_fight_start_ms);
    let elapsed_secs = elapsed_ms as f64 / 1000.0;

    let player_stats = &player.dmg_stats;
    let encounter_stats = &encounter.dmg_stats;

    let inspected_player = make_player_row(
        player_uid,
        player.name.as_deref().unwrap_or(""),
        player.class,
        player.class_spec,
        player.ability_score,
        player_stats,
        encounter_stats,
        elapsed_secs,
        &player.time_series,
    );

    let mut skill_window = SkillsWindow {
        inspected_player,
        skill_rows: Vec::new(),
        local_player_uid: encounter.local_player_uid as f64,
        top_value: 0.0,
    };

    for (&skill_uid, skill_stat) in &player.skill_uid_to_dps_stats {
        skill_window.top_value = skill_window.top_value.max(skill_stat.total as f64);
        let row = SkillRow {
            uid: f64::from(skill_uid),
            name: get_skill_name(skill_uid),
            total_value: skill_stat.total as f64,
            value_per_sec: nan_is_zero(skill_stat.total as f64 / elapsed_secs),
            value_pct: nan_is_zero(
                skill_stat.total as f64 / player_stats.total as f64 * 100.0,
            ),
            crit_rate: nan_is_zero(
                skill_stat.crit_count as f64 / skill_stat.hit_count as f64 * 100.0,
            ),
            crit_value_rate: nan_is_zero(
                skill_stat.crit_value as f64 / skill_stat.total as f64 * 100.0,
            ),
            lucky_rate: nan_is_zero(
                skill_stat.lucky_count as f64 / skill_stat.hit_count as f64 * 100.0,
            ),
            lucky_value_rate: nan_is_zero(
                skill_stat.lucky_value as f64 / skill_stat.total as f64 * 100.0,
            ),
            hits: skill_stat.hit_count as f64,
            hits_per_minute: nan_is_zero(
                skill_stat.hit_count as f64 / elapsed_secs * 60.0,
            ),
        };
        skill_window.skill_rows.push(row);
    }
    drop(encounter);

    skill_window.skill_rows.sort_by(|a, b| {
        b.total_value
            .partial_cmp(&a.total_value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(skill_window)
}

// ─── Control commands ─────────────────────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub fn reset_encounter(state: tauri::State<'_, EncounterMutex>) {
    match state.lock() {
        Ok(mut encounter) => {
            encounter.clear_combat_stats();
            info!("Encounter reset");
        }
        Err(e) => log::error!("Lock poisoned in reset_encounter: {e}"),
    }
}

#[tauri::command]
#[specta::specta]
pub fn toggle_pause(state: tauri::State<'_, EncounterMutex>) {
    match state.lock() {
        Ok(mut encounter) => {
            encounter.is_paused = !encounter.is_paused;
            info!("Encounter paused: {}", encounter.is_paused);
        }
        Err(e) => log::error!("Lock poisoned in toggle_pause: {e}"),
    }
}

#[tauri::command]
#[specta::specta]
pub fn quit_app(app: AppHandle) {
    crate::begin_exit();
    app.exit(0);
}

// ─── Encounter snapshot ───────────────────────────────────────────────────────

pub fn build_encounter_snapshot(encounter: &Encounter) -> EncounterSnapshot {
    let elapsed_ms = encounter
        .time_last_combat_packet_ms
        .saturating_sub(encounter.time_fight_start_ms);
    let elapsed_secs = elapsed_ms as f64 / 1000.0;
    let total_dmg = encounter.dmg_stats.total as f64;
    let total_dps = if elapsed_secs > 0.0 { total_dmg / elapsed_secs } else { 0.0 };

    let window = build_players_window(encounter, StatType::Dmg);

    EncounterSnapshot {
        id: 0.0,
        start_ms: encounter.time_fight_start_ms as f64,
        end_ms: encounter.time_last_combat_packet_ms as f64,
        duration_ms: elapsed_ms as f64,
        total_dmg,
        total_dps,
        player_rows: window.player_rows,
        time_series: encounter.time_series.iter().cloned().collect(),
    }
}

// ─── History commands ─────────────────────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub fn set_combat_exit_timeout(secs: f64) {
    let ms = (secs * 1000.0).max(0.0) as u64;
    crate::engine::runtime_settings::COMBAT_EXIT_TIMEOUT_MS.store(ms, std::sync::atomic::Ordering::Relaxed);
}

#[tauri::command]
#[specta::specta]
pub fn set_history_limit(limit: f64) {
    let n = limit.max(0.0) as usize;
    crate::engine::runtime_settings::HISTORY_LIMIT.store(n, std::sync::atomic::Ordering::Relaxed);
    crate::engine::history::trim_to_limit();
}

#[tauri::command]
#[specta::specta]
pub fn get_history() -> Vec<crate::bridge::models::EncounterSnapshot> {
    crate::engine::history::snapshot_list()
}

#[tauri::command]
#[specta::specta]
pub fn set_time_series_config(samples: f64, interval_ms: f64) {
    let n = samples.max(1.0) as usize;
    let i = interval_ms.max(50.0) as u64;
    crate::engine::runtime_settings::TS_SAMPLES.store(n, std::sync::atomic::Ordering::Relaxed);
    crate::engine::runtime_settings::TS_INTERVAL_MS.store(i, std::sync::atomic::Ordering::Relaxed);
}

#[tauri::command]
#[specta::specta]
pub fn get_time_series(state: tauri::State<'_, EncounterMutex>) -> Vec<crate::bridge::models::TimeSeriesPoint> {
    match state.lock() {
        Ok(e) => e.time_series.iter().cloned().collect(),
        Err(_) => Vec::new(),
    }
}

#[tauri::command]
#[specta::specta]
pub fn clear_history() {
    crate::engine::history::clear();
}

// ─── Overlay commands ─────────────────────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub fn set_always_on_top(window: tauri::WebviewWindow, enabled: bool) -> Result<(), String> {
    window.set_always_on_top(enabled).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn set_click_through(window: tauri::WebviewWindow, enabled: bool) -> Result<(), String> {
    window.set_ignore_cursor_events(enabled).map_err(|e| e.to_string())
}
