use crate::bridge::models::{
    EncounterSnapshot, HeaderInfo, MeasureModeStatus, PlayerBuffSnapshot, PlayerRow, PlayersWindow,
    SelfBuffSnapshot, SkillRow, SkillsWindow, TimeSeriesPoint, TrackedBuffsData,
};
use crate::engine::buff_source::BuffSourceKind;
use crate::engine::class::{Class, ClassSpec};
use crate::engine::combat_stats::CombatStats;
use crate::engine::encounter::{Encounter, EncounterMutex};
use crate::engine::name_cache;
use crate::engine::selected_uid;
use crate::engine::skill_names::get_skill_name;
use crate::protocol::pb::EntityKind;
use log::info;
use std::collections::VecDeque;
use tauri::{AppHandle, Emitter, Manager};

#[derive(serde::Serialize, specta::Type, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CachedPlayerDto {
    pub name: String,
    pub class_id: Option<i32>,
    pub ability_score: Option<i32>,
}

#[inline]
fn ratio_pct(num: i64, denom: i64) -> f64 {
    if denom == 0 {
        0.0
    } else {
        num as f64 / denom as f64 * 100.0
    }
}

#[inline]
fn ratio_count_pct(num: u32, denom: u32) -> f64 {
    if denom == 0 {
        0.0
    } else {
        num as f64 / denom as f64 * 100.0
    }
}

#[inline]
fn rate_per_sec(total: i64, elapsed_secs: f64) -> f64 {
    if elapsed_secs <= 0.0 {
        0.0
    } else {
        total as f64 / elapsed_secs
    }
}

#[inline]
fn rate_per_minute(count: u32, elapsed_secs: f64) -> f64 {
    if elapsed_secs <= 0.0 {
        0.0
    } else {
        count as f64 / elapsed_secs * 60.0
    }
}

fn sort_skill_rows_desc(rows: &mut [SkillRow]) {
    rows.sort_by(|a, b| {
        b.total_value
            .partial_cmp(&a.total_value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

fn skill_row_for(
    uid: f64,
    name: String,
    element: u8,
    damage_mode: u8,
    stats: &CombatStats,
    elapsed_secs: f64,
    denominator: i64,
) -> SkillRow {
    SkillRow {
        uid,
        name,
        element,
        damage_mode,
        total_value: stats.total as f64,
        value_per_sec: rate_per_sec(stats.total, elapsed_secs),
        value_pct: ratio_pct(stats.total, denominator),
        crit_rate: ratio_count_pct(stats.crit_count, stats.hit_count),
        crit_value_rate: ratio_pct(stats.crit_value, stats.total),
        lucky_rate: ratio_count_pct(stats.lucky_count, stats.hit_count),
        lucky_value_rate: ratio_pct(stats.lucky_value, stats.total),
        hits: stats.hit_count as f64,
        hits_per_minute: rate_per_minute(stats.hit_count, elapsed_secs),
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
        total_dps: rate_per_sec(encounter.dmg_stats.total, elapsed_secs),
        total_dmg: encounter.dmg_stats.total as f64,
        elapsed_ms: elapsed_ms as f64,
        time_last_combat_packet_ms: encounter.time_last_combat_packet_ms as f64,
    }
}

// ─── Players windows ─────────────────────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub fn get_dps_players(state: tauri::State<'_, EncounterMutex>) -> PlayersWindow {
    let mut window = match state.lock() {
        Ok(e) => build_players_window_unsorted(&*e, StatType::Dmg),
        Err(e) => {
            log::error!("Lock poisoned in get_dps_players: {e}");
            return PlayersWindow::default();
        }
    };
    window.player_rows.sort_by(|a, b| b.total_value.partial_cmp(&a.total_value).unwrap_or(std::cmp::Ordering::Equal));
    window
}

#[tauri::command]
#[specta::specta]
pub fn get_dps_boss_players(state: tauri::State<'_, EncounterMutex>) -> PlayersWindow {
    let mut window = match state.lock() {
        Ok(e) => build_players_window_unsorted(&*e, StatType::DmgBossOnly),
        Err(e) => {
            log::error!("Lock poisoned in get_dps_boss_players: {e}");
            return PlayersWindow::default();
        }
    };
    window.player_rows.sort_by(|a, b| b.total_value.partial_cmp(&a.total_value).unwrap_or(std::cmp::Ordering::Equal));
    window
}

#[tauri::command]
#[specta::specta]
pub fn get_heal_players(state: tauri::State<'_, EncounterMutex>) -> PlayersWindow {
    let mut window = match state.lock() {
        Ok(e) => build_players_window_unsorted(&*e, StatType::Heal),
        Err(e) => {
            log::error!("Lock poisoned in get_heal_players: {e}");
            return PlayersWindow::default();
        }
    };
    window.player_rows.sort_by(|a, b| b.total_value.partial_cmp(&a.total_value).unwrap_or(std::cmp::Ordering::Equal));
    window
}

#[tauri::command]
#[specta::specta]
pub fn get_dmg_taken_players(state: tauri::State<'_, EncounterMutex>) -> PlayersWindow {
    let mut window = match state.lock() {
        Ok(e) => build_players_window_unsorted(&*e, StatType::DmgTaken),
        Err(e) => {
            log::error!("Lock poisoned in get_dmg_taken_players: {e}");
            return PlayersWindow::default();
        }
    };
    window.player_rows.sort_by(|a, b| b.total_value.partial_cmp(&a.total_value).unwrap_or(std::cmp::Ordering::Equal));
    window
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

    let mut top_value = 0.0_f64;
    let mut skill_rows: Vec<SkillRow> = player
        .attacker_uid_to_dmg_taken_stats
        .iter()
        .map(|(&attacker_uid, stats)| {
            top_value = top_value.max(stats.total as f64);
            skill_row_for(
                attacker_uid as f64,
                attacker_display_name(&encounter, attacker_uid),
                0,
                0,
                stats,
                elapsed_secs,
                player_stats.total,
            )
        })
        .collect();

    sort_skill_rows_desc(&mut skill_rows);

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

    let attacker_total_i64 = attacker_total as i64;
    let mut top_value = 0.0_f64;
    let mut skill_rows: Vec<SkillRow> = player
        .attacker_skill_to_dmg_taken_stats
        .iter()
        .filter(|((uid, _), _)| *uid == attacker_uid)
        .map(|((_, skill_uid), stats)| {
            top_value = top_value.max(stats.total as f64);
            let meta = player.skill_meta.get(skill_uid).copied().unwrap_or_default();
            skill_row_for(
                f64::from(*skill_uid),
                crate::engine::skill_names::get_skill_name(*skill_uid),
                meta.property,
                meta.damage_mode,
                stats,
                elapsed_secs,
                attacker_total_i64,
            )
        })
        .collect();

    sort_skill_rows_desc(&mut skill_rows);

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
    if e.entity_type == EntityKind::Player {
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

/// ロック保持中に呼ぶ。ソートはロック解放後に呼び出し元で行う。
fn build_players_window_unsorted(encounter: &Encounter, stat_type: StatType) -> PlayersWindow {
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

        if entity.entity_type != EntityKind::Player || entity_stats.total == 0 {
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
        value_per_sec: rate_per_sec(entity_stats.total, elapsed_secs),
        value_pct: ratio_pct(entity_stats.total, encounter_stats.total),
        crit_rate: ratio_count_pct(entity_stats.crit_count, entity_stats.hit_count),
        crit_value_rate: ratio_pct(entity_stats.crit_value, entity_stats.total),
        lucky_rate: ratio_count_pct(entity_stats.lucky_count, entity_stats.hit_count),
        lucky_value_rate: ratio_pct(entity_stats.lucky_value, entity_stats.total),
        hits: entity_stats.hit_count as f64,
        hits_per_minute: rate_per_minute(entity_stats.hit_count, elapsed_secs),
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
        let row = skill_row_for(
            f64::from(skill_uid),
            get_skill_name(skill_uid),
            meta.property,
            meta.damage_mode,
            skill_stat,
            elapsed_secs,
            player_stats.total,
        );
        skill_window.skill_rows.push(row);
    }
    drop(encounter);

    sort_skill_rows_desc(&mut skill_window.skill_rows);

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

    let mut window = build_players_window_unsorted(encounter, StatType::Dmg);
    window.player_rows.sort_by(|a, b| {
        b.total_value
            .partial_cmp(&a.total_value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

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

fn aggregate_player_buffs(
    snapshots: Vec<crate::engine::buff_tracker::BuffStateSnapshot>,
    uid: f64,
    name: String,
) -> PlayerBuffSnapshot {
    use crate::engine::buff_source::{classify, classify_buff};
    use std::collections::HashMap;

    let mut by_kind: HashMap<String, SelfBuffSnapshot> = HashMap::new();
    for snap in &snapshots {
        // 免疫デバフ(buff_config_id 211xxxx)優先。なければリキャスト(effect_id 39xxxx等)で分類。
        let kind = {
            let k = classify_buff(snap.base_id as i64);
            if k != BuffSourceKind::Other { k } else { classify(snap.base_id) }
        };
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

    PlayerBuffSnapshot {
        uid,
        name,
        buffs: by_kind.into_values().collect(),
    }
}

#[tauri::command]
#[specta::specta]
pub fn get_tracked_buffs(
    state: tauri::State<'_, EncounterMutex>,
    uids: Vec<f64>,
) -> TrackedBuffsData {
    use crate::engine::processor::now_ms;

    // ロック内: gc と snapshot のみ実施
    let (raw_snapshots, now_ms, local_uid) = {
        let mut enc = match state.lock() {
            Ok(e) => e,
            Err(e) => {
                log::error!("Lock poisoned in get_tracked_buffs: {e}");
                return TrackedBuffsData::default();
            }
        };
        let now_ms = now_ms();
        let local_uid = enc.local_player_uid;
        enc.buff_tracker.gc(now_ms);
        let raw: Vec<(f64, i64, _)> = uids
            .iter()
            .map(|&uid_f64| {
                let uid_i64 = uid_f64 as i64;
                let snapshots = enc.buff_tracker.snapshot_for(uid_i64, now_ms);
                (uid_f64, uid_i64, snapshots)
            })
            .collect();
        (raw, now_ms, local_uid)
    }; // ロック解放

    // ロック外: name_cache 参照・kind 分類・HashMap 構築
    let players = raw_snapshots
        .into_iter()
        .map(|(uid_f64, uid_i64, snapshots)| {
            let name = name_cache::lookup(uid_i64)
                .map(|c| c.name)
                .unwrap_or_default();
            aggregate_player_buffs(snapshots, uid_f64, name)
        })
        .collect();

    TrackedBuffsData {
        players,
        now_ms: now_ms as f64,
        local_player_uid: local_uid as f64,
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
                let remaining = duration_ms.saturating_sub(elapsed) as f64;
                MeasureModeStatus {
                    kind: "active".to_string(),
                    remaining_ms: Some(remaining),
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
