use crate::bridge::models::{
    EncounterSnapshot, HeaderInfo, MeasureModeStatus, PlayerRow, PlayersWindow, SelfBuffSnapshot,
    SelfBuffsData, SkillRow, SkillsWindow, TimeSeriesPoint,
};
use crate::engine::buff_source::BuffSourceKind;
use crate::engine::class::{Class, ClassSpec};
use crate::engine::combat_stats::CombatStats;
use crate::engine::encounter::{Encounter, EncounterMutex};
use crate::engine::name_cache;
use crate::engine::selected_uid;
use crate::engine::skill_names::get_skill_name;
use crate::protocol::pb::EEntityType;
use log::info;
use std::collections::VecDeque;
use tauri::{AppHandle, Emitter};

#[derive(serde::Serialize, specta::Type, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CachedPlayerDto {
    pub name: String,
    pub class_id: Option<i32>,
    pub ability_score: Option<i32>,
}

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
    DmgTaken,
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

    let selected = selected_uid::get();
    if selected.is_some() && !encounter.has_selected_participant {
        return HeaderInfo::default();
    }

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

#[tauri::command]
#[specta::specta]
pub fn get_dmg_taken_players(state: tauri::State<'_, EncounterMutex>) -> PlayersWindow {
    let encounter = match state.lock() {
        Ok(e) => e,
        Err(e) => {
            log::error!("Lock poisoned in get_dmg_taken_players: {e}");
            return PlayersWindow::default();
        }
    };
    build_players_window(&*encounter, StatType::DmgTaken)
}

#[tauri::command]
#[specta::specta]
pub fn get_dmg_taken_attackers(
    state: tauri::State<'_, EncounterMutex>,
    player_uid: i64,
) -> Result<SkillsWindow, String> {
    let encounter = state.lock().map_err(|e| format!("Lock poisoned: {e}"))?;

    let Some(player) = encounter.entities.get(&player_uid) else {
        return Err(format!("Could not find player with uid {player_uid}"));
    };

    let elapsed_ms = encounter
        .time_last_combat_packet_ms
        .saturating_sub(encounter.time_fight_start_ms);
    let elapsed_secs = elapsed_ms as f64 / 1000.0;

    let player_stats = &player.dmg_taken_stats;
    let encounter_stats = &encounter.dmg_taken_stats;

    let inspected_player = make_player_row(
        player_uid,
        player.name.as_deref().unwrap_or(""),
        player.class,
        player.class_spec,
        player.ability_score,
        player.season_level,
        player.season_strength,
        player_stats,
        encounter_stats,
        elapsed_secs,
        &player.time_series,
    );

    let attacker_uid_to_stats: Vec<(i64, &crate::engine::combat_stats::CombatStats)> =
        player.attacker_uid_to_dmg_taken_stats.iter().collect();

    let top_value = attacker_uid_to_stats
        .iter()
        .map(|(_, s)| s.total as f64)
        .fold(0.0_f64, f64::max);

    let mut skill_rows: Vec<SkillRow> = attacker_uid_to_stats
        .iter()
        .map(|(&attacker_uid, stats)| SkillRow {
            uid: attacker_uid as f64,
            name: attacker_display_name(&encounter, attacker_uid),
            element: 0,
            damage_mode: 0,
            total_value: stats.total as f64,
            value_per_sec: nan_is_zero(stats.total as f64 / elapsed_secs),
            value_pct: nan_is_zero(stats.total as f64 / player_stats.total as f64 * 100.0),
            crit_rate: nan_is_zero(stats.crit_count as f64 / stats.hit_count as f64 * 100.0),
            crit_value_rate: nan_is_zero(stats.crit_value as f64 / stats.total as f64 * 100.0),
            lucky_rate: nan_is_zero(stats.lucky_count as f64 / stats.hit_count as f64 * 100.0),
            lucky_value_rate: nan_is_zero(stats.lucky_value as f64 / stats.total as f64 * 100.0),
            hits: stats.hit_count as f64,
            hits_per_minute: nan_is_zero(stats.hit_count as f64 / elapsed_secs * 60.0),
        })
        .collect();

    skill_rows.sort_by(|a, b| {
        b.total_value
            .partial_cmp(&a.total_value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(SkillsWindow {
        inspected_player,
        skill_rows,
        local_player_uid: encounter.local_player_uid as f64,
        top_value,
    })
}

#[tauri::command]
#[specta::specta]
pub fn get_dmg_taken_skills(
    state: tauri::State<'_, EncounterMutex>,
    player_uid: i64,
    attacker_uid: i64,
) -> Result<SkillsWindow, String> {
    let encounter = state.lock().map_err(|e| format!("Lock poisoned: {e}"))?;

    let Some(player) = encounter.entities.get(&player_uid) else {
        return Err(format!("Could not find player with uid {player_uid}"));
    };

    let elapsed_ms = encounter
        .time_last_combat_packet_ms
        .saturating_sub(encounter.time_fight_start_ms);
    let elapsed_secs = elapsed_ms as f64 / 1000.0;

    let attacker_total = player
        .attacker_uid_to_dmg_taken_stats
        .get(&attacker_uid)
        .map(|s| s.total as f64)
        .unwrap_or(0.0);
    let encounter_stats = &encounter.dmg_taken_stats;

    let player_stats = &player.dmg_taken_stats;

    let inspected_player = make_player_row(
        player_uid,
        player.name.as_deref().unwrap_or(""),
        player.class,
        player.class_spec,
        player.ability_score,
        player.season_level,
        player.season_strength,
        player_stats,
        encounter_stats,
        elapsed_secs,
        &player.time_series,
    );

    let top_value = player
        .attacker_skill_to_dmg_taken_stats
        .iter()
        .filter(|((uid, _), _)| *uid == attacker_uid)
        .map(|(_, s)| s.total as f64)
        .fold(0.0_f64, f64::max);

    let mut skill_rows: Vec<SkillRow> = player
        .attacker_skill_to_dmg_taken_stats
        .iter()
        .filter(|((uid, _), _)| *uid == attacker_uid)
        .map(|((_, skill_uid), stats)| {
            let meta = player.skill_meta.get(skill_uid).copied().unwrap_or_default();
            SkillRow {
                uid: f64::from(*skill_uid),
                name: crate::engine::skill_names::get_skill_name(*skill_uid),
                element: meta.property,
                damage_mode: meta.damage_mode,
                total_value: stats.total as f64,
                value_per_sec: nan_is_zero(stats.total as f64 / elapsed_secs),
                value_pct: nan_is_zero(stats.total as f64 / attacker_total * 100.0),
                crit_rate: nan_is_zero(stats.crit_count as f64 / stats.hit_count as f64 * 100.0),
                crit_value_rate: nan_is_zero(stats.crit_value as f64 / stats.total as f64 * 100.0),
                lucky_rate: nan_is_zero(stats.lucky_count as f64 / stats.hit_count as f64 * 100.0),
                lucky_value_rate: nan_is_zero(stats.lucky_value as f64 / stats.total as f64 * 100.0),
                hits: stats.hit_count as f64,
                hits_per_minute: nan_is_zero(stats.hit_count as f64 / elapsed_secs * 60.0),
            }
        })
        .collect();

    skill_rows.sort_by(|a, b| {
        b.total_value
            .partial_cmp(&a.total_value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(SkillsWindow {
        inspected_player,
        skill_rows,
        local_player_uid: encounter.local_player_uid as f64,
        top_value,
    })
}

fn attacker_display_name(encounter: &Encounter, attacker_uid: i64) -> String {
    let Some(e) = encounter.entities.get(&attacker_uid) else {
        return format!("#{}", attacker_uid & 0xFFFF);
    };
    if e.entity_type == EEntityType::EntChar {
        return e
            .name
            .clone()
            .unwrap_or_else(|| format!("プレイヤー#{}", attacker_uid & 0xFFFF));
    }
    if let Some(mid) = e.monster_id {
        if let Some(name) = crate::engine::monster_names::get_boss_name(mid) {
            return name.to_string();
        }
        return format!("モンスター#{mid}");
    }
    format!("#{}", attacker_uid & 0xFFFF)
}

fn build_players_window(encounter: &Encounter, stat_type: StatType) -> PlayersWindow {
    let selected = selected_uid::get();
    if selected.is_some() && !encounter.has_selected_participant {
        return PlayersWindow::default();
    }

    let elapsed_ms = encounter
        .time_last_combat_packet_ms
        .saturating_sub(encounter.time_fight_start_ms);
    let elapsed_secs = elapsed_ms as f64 / 1000.0;

    let encounter_stats = match stat_type {
        StatType::Dmg => &encounter.dmg_stats,
        StatType::DmgBossOnly => &encounter.dmg_stats_boss_only,
        StatType::Heal => &encounter.heal_stats,
        StatType::DmgTaken => &encounter.dmg_taken_stats,
    };

    let mut window = PlayersWindow {
        player_rows: Vec::new(),
        local_player_uid: selected.unwrap_or(encounter.local_player_uid) as f64,
        top_value: 0.0,
    };

    for (&entity_uid, entity) in &encounter.entities {
        let entity_stats = match stat_type {
            StatType::Dmg => &entity.dmg_stats,
            StatType::DmgBossOnly => &entity.dmg_stats_boss_only,
            StatType::Heal => &entity.heal_stats,
            StatType::DmgTaken => &entity.dmg_taken_stats,
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
            entity.season_level,
            entity.season_strength,
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
    season_level: Option<i32>,
    season_strength: Option<i32>,
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
        class_spec_name: class_spec
            .unwrap_or(ClassSpec::Unknown)
            .name_ja()
            .to_string(),
        ability_score: f64::from(ability_score.unwrap_or(-1)),
        season_level: f64::from(season_level.unwrap_or(-1)),
        season_strength: f64::from(season_strength.unwrap_or(-1)),
        total_value: entity_stats.total as f64,
        value_per_sec: nan_is_zero(entity_stats.total as f64 / elapsed_secs),
        value_pct: nan_is_zero(entity_stats.total as f64 / encounter_stats.total as f64 * 100.0),
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
    let encounter = state.lock().map_err(|e| format!("Lock poisoned: {e}"))?;

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
        player.season_level,
        player.season_strength,
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
        let meta = player.skill_meta.get(&skill_uid).copied().unwrap_or_default();
        let row = SkillRow {
            uid: f64::from(skill_uid),
            name: get_skill_name(skill_uid),
            element: meta.property,
            damage_mode: meta.damage_mode,
            total_value: skill_stat.total as f64,
            value_per_sec: nan_is_zero(skill_stat.total as f64 / elapsed_secs),
            value_pct: nan_is_zero(skill_stat.total as f64 / player_stats.total as f64 * 100.0),
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
            hits_per_minute: nan_is_zero(skill_stat.hit_count as f64 / elapsed_secs * 60.0),
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
    let total_dps = if elapsed_secs > 0.0 {
        total_dmg / elapsed_secs
    } else {
        0.0
    };

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
        participant_player_uids: encounter
            .participant_player_uids
            .iter()
            .map(|&v| v as f64)
            .collect(),
    }
}

// ─── History commands ─────────────────────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub fn set_combat_exit_timeout(secs: f64) {
    let ms = (secs * 1000.0).max(0.0) as u64;
    crate::engine::runtime_settings::COMBAT_EXIT_TIMEOUT_MS
        .store(ms, std::sync::atomic::Ordering::Relaxed);
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
    let all = crate::engine::history::snapshot_list();
    let Some(sel) = selected_uid::get() else {
        return all;
    };
    let sel_f64 = sel as f64;
    all.into_iter()
        .filter(|snap| {
            snap.participant_player_uids.is_empty()
                || snap.participant_player_uids.contains(&sel_f64)
        })
        .collect()
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
pub fn set_imagine_only_mode(state: tauri::State<'_, EncounterMutex>, enabled: bool) {
    let was_enabled = crate::engine::runtime_settings::IMAGINE_ONLY_MODE
        .swap(enabled, std::sync::atomic::Ordering::Relaxed);
    // 切替時は古い集計結果を残さないようにエンカウンターをクリア
    if was_enabled != enabled {
        if let Ok(mut enc) = state.lock() {
            enc.clear_combat_stats();
        }
        info!("Imagine-only mode: {enabled}");
    }
}

#[tauri::command]
#[specta::specta]
pub fn get_time_series(
    state: tauri::State<'_, EncounterMutex>,
) -> Vec<crate::bridge::models::TimeSeriesPoint> {
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
    window
        .set_ignore_cursor_events(enabled)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn set_buffs_window_visible(app: tauri::AppHandle, visible: bool) -> Result<(), String> {
    let win = app
        .get_webview_window("buffs")
        .ok_or_else(|| "buffs window not found".to_string())?;
    if visible {
        win.show().map_err(|e| e.to_string())
    } else {
        win.hide().map_err(|e| e.to_string())
    }
}

// ─── selected_uid コマンド ────────────────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub fn get_selected_uid() -> Option<f64> {
    selected_uid::get().map(|v| v as f64)
}

#[tauri::command]
#[specta::specta]
pub fn set_selected_uid(state: tauri::State<'_, EncounterMutex>, uid: Option<f64>) {
    let uid_i64 = uid.map(|v| v as i64);
    selected_uid::set(uid_i64);
    match state.lock() {
        Ok(mut encounter) => {
            encounter.clear_combat_stats();
            encounter.active_connection = None;
            encounter.local_player_uid = uid_i64.unwrap_or(0);
            encounter.measure_mode = crate::engine::encounter::MeasureMode::Normal;
        }
        Err(e) => log::error!("Lock poisoned in set_selected_uid: {e}"),
    }
}

#[tauri::command]
#[specta::specta]
pub fn lookup_name_cache(uid: f64) -> Option<CachedPlayerDto> {
    let cached = name_cache::lookup(uid as i64)?;
    Some(CachedPlayerDto {
        name: cached.name,
        class_id: cached.class_id,
        ability_score: cached.ability_score,
    })
}

// ─── 3min measure mode ───────────────────────────────────────────────────────

pub fn finalize_3min_internal(encounter: &mut Encounter, app: &AppHandle) {
    let snapshot = build_encounter_snapshot(encounter);
    if !snapshot.player_rows.is_empty() {
        crate::engine::history::push(snapshot.clone());
    }
    if let Err(e) = app.emit("3min-measure-finalized", snapshot) {
        log::error!("Failed to emit 3min-measure-finalized: {e}");
    }
    encounter.clear_combat_stats();
    encounter.measure_mode = crate::engine::encounter::MeasureMode::Normal;
}

#[tauri::command]
#[specta::specta]
pub fn start_3min_measure_mode(state: tauri::State<'_, EncounterMutex>, duration_secs: f64) {
    let duration_ms = (duration_secs * 1000.0).max(1000.0) as u128;
    match state.lock() {
        Ok(mut enc) => {
            enc.clear_combat_stats();
            enc.measure_mode = crate::engine::encounter::MeasureMode::Pending3Min { duration_ms };
            info!("3min measure mode: pending (duration={duration_ms}ms)");
        }
        Err(e) => log::error!("Lock poisoned in start_3min_measure_mode: {e}"),
    }
}

#[tauri::command]
#[specta::specta]
pub fn cancel_3min_measure_mode(state: tauri::State<'_, EncounterMutex>) {
    match state.lock() {
        Ok(mut enc) => {
            enc.clear_combat_stats();
            enc.measure_mode = crate::engine::encounter::MeasureMode::Normal;
            info!("3min measure mode: cancelled");
        }
        Err(e) => log::error!("Lock poisoned in cancel_3min_measure_mode: {e}"),
    }
}

#[tauri::command]
#[specta::specta]
pub fn get_self_buffs(state: tauri::State<'_, EncounterMutex>) -> SelfBuffsData {
    use crate::engine::buff_source::classify_buff;
    use crate::engine::processor::now_ms;
    use std::collections::HashMap;

    let mut enc = match state.lock() {
        Ok(e) => e,
        Err(e) => {
            log::error!("Lock poisoned in get_self_buffs: {e}");
            return SelfBuffsData::default();
        }
    };

    let now_ms = now_ms();
    enc.buff_tracker.gc(now_ms);
    let snapshots = enc.buff_tracker.snapshot(now_ms);

    let mut by_kind: HashMap<String, SelfBuffSnapshot> = HashMap::new();
    for snap in &snapshots {
        // 免疫デバフ (field_10 由来の buff_config_id) のみ表示。
        // リキャストタイマー (EffectInfo の 39XXXX) は表示しない。
        let kind = classify_buff(snap.base_id as i64);
        if kind == BuffSourceKind::Other {
            continue;
        }
        let kind_str = kind.as_str().to_string();
        let entry = by_kind.entry(kind_str.clone()).or_insert_with(|| SelfBuffSnapshot {
            kind: kind_str.clone(),
            base_id: snap.base_id,
            buff_uuid: snap.buff_uuid,
            layer: snap.layer,
            remaining_ms: snap.remaining_ms,
            duration_ms: snap.duration_ms,
            received_at_ms: snap.received_at_local_ms as f64,
        });
        // 最長残時間を採用
        if snap.remaining_ms > entry.remaining_ms {
            *entry = SelfBuffSnapshot {
                kind: kind_str,
                base_id: snap.base_id,
                buff_uuid: snap.buff_uuid,
                layer: snap.layer,
                remaining_ms: snap.remaining_ms,
                duration_ms: snap.duration_ms,
                received_at_ms: snap.received_at_local_ms as f64,
            };
        }
    }

    SelfBuffsData {
        buffs: by_kind.into_values().collect(),
        now_ms: now_ms as f64,
    }
}

#[tauri::command]
#[specta::specta]
pub fn finalize_3min_measure_mode(app: AppHandle, state: tauri::State<'_, EncounterMutex>) {
    match state.lock() {
        Ok(mut enc) => {
            finalize_3min_internal(&mut enc, &app);
            info!("3min measure mode: finalized (UI-driven)");
        }
        Err(e) => log::error!("Lock poisoned in finalize_3min_measure_mode: {e}"),
    }
}

#[tauri::command]
#[specta::specta]
pub fn get_measure_mode_status(state: tauri::State<'_, EncounterMutex>) -> MeasureModeStatus {
    use crate::engine::encounter::MeasureMode;
    use crate::engine::processor::now_ms;

    match state.lock() {
        Ok(enc) => match enc.measure_mode {
            MeasureMode::Normal => MeasureModeStatus {
                kind: "normal".to_string(),
                remaining_ms: None,
                duration_ms: None,
                armed_at_ms: None,
            },
            MeasureMode::Pending3Min { duration_ms } => MeasureModeStatus {
                kind: "pending".to_string(),
                remaining_ms: None,
                duration_ms: Some(duration_ms as f64),
                armed_at_ms: None,
            },
            MeasureMode::Active3Min {
                armed_at_ms,
                duration_ms,
            } => {
                let elapsed = now_ms().saturating_sub(armed_at_ms);
                let remaining = (duration_ms as i128) - (elapsed as i128);
                MeasureModeStatus {
                    kind: "active".to_string(),
                    remaining_ms: Some(remaining.max(0) as f64),
                    duration_ms: Some(duration_ms as f64),
                    armed_at_ms: Some(armed_at_ms as f64),
                }
            }
        },
        Err(e) => {
            log::error!("Lock poisoned in get_measure_mode_status: {e}");
            MeasureModeStatus {
                kind: "normal".to_string(),
                remaining_ms: None,
                duration_ms: None,
                armed_at_ms: None,
            }
        }
    }
}
