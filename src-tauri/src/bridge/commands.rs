use crate::bridge::models::{HeaderInfo, PlayerRow, PlayersWindow, SkillRow, SkillsWindow};
use crate::engine::class::{Class, ClassSpec};
use crate::engine::combat_stats::CombatStats;
use crate::engine::encounter::{Encounter, EncounterMutex};
use crate::engine::skill_names::get_skill_name;
use crate::protocol::pb::EEntityType;
use log::info;
use std::sync::MutexGuard;
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
    build_players_window(encounter, StatType::Dmg)
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
    build_players_window(encounter, StatType::DmgBossOnly)
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
    build_players_window(encounter, StatType::Heal)
}

fn build_players_window(
    encounter: MutexGuard<'_, Encounter>,
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
        local_player_uid: 0.0,
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
        );
        window.player_rows.push(row);
    }
    drop(encounter);

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
    );

    let mut skill_window = SkillsWindow {
        inspected_player,
        skill_rows: Vec::new(),
        local_player_uid: 0.0,
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
            encounter.clone_from(&Encounter::default());
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
    app.exit(0);
}
