//! Tauri コマンド層: 集計ロジックは `bpsr_core::compute` に移管済み。
//! ここは specta 付きの薄いラッパ（State<Arc<EncounterMutex>> を core へ橋渡し）と、
//! ウィンドウ/アプリ制御コマンドだけを持つ。

use bpsr_core::compute;
use bpsr_core::compute::CachedPlayerDto;
use bpsr_core::engine::encounter::EncounterMutex;
use bpsr_core::models::{
    EncounterSnapshot, HeaderInfo, MeasureModeStatus, PlayersWindow, SelfStatusData, SkillsWindow,
    TimeSeriesPoint, TrackedBuffsData,
};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

type EncState<'a> = State<'a, Arc<EncounterMutex>>;

// ─── Header / Players ──────────────────────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub fn get_header_info(state: EncState<'_>) -> HeaderInfo {
    compute::get_header_info(&state)
}

#[tauri::command]
#[specta::specta]
pub fn get_dps_players(state: EncState<'_>) -> PlayersWindow {
    compute::get_dps_players(&state)
}

#[tauri::command]
#[specta::specta]
pub fn get_dps_boss_players(state: EncState<'_>) -> PlayersWindow {
    compute::get_dps_boss_players(&state)
}

#[tauri::command]
#[specta::specta]
pub fn get_heal_players(state: EncState<'_>) -> PlayersWindow {
    compute::get_heal_players(&state)
}

#[tauri::command]
#[specta::specta]
pub fn get_dmg_taken_players(state: EncState<'_>) -> PlayersWindow {
    compute::get_dmg_taken_players(&state)
}

#[tauri::command]
#[specta::specta]
pub fn get_dmg_taken_attackers(
    state: EncState<'_>,
    player_uid: i64,
) -> Result<SkillsWindow, String> {
    compute::get_dmg_taken_attackers(&state, player_uid)
}

#[tauri::command]
#[specta::specta]
pub fn get_dmg_taken_skills(
    state: EncState<'_>,
    player_uid: i64,
    attacker_uid: i64,
) -> Result<SkillsWindow, String> {
    compute::get_dmg_taken_skills(&state, player_uid, attacker_uid)
}

#[tauri::command]
#[specta::specta]
pub fn get_skills(state: EncState<'_>, player_uid: i64) -> Result<SkillsWindow, String> {
    compute::get_skills(&state, player_uid)
}

// ─── Control ───────────────────────────────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub fn reset_encounter(state: EncState<'_>) {
    compute::reset_encounter(&state)
}

#[tauri::command]
#[specta::specta]
pub fn toggle_pause(state: EncState<'_>) {
    compute::toggle_pause(&state)
}

#[tauri::command]
#[specta::specta]
pub fn quit_app(app: AppHandle) {
    crate::begin_exit();
    app.exit(0);
}

// ─── History / settings ──────────────────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub fn set_combat_exit_timeout(secs: f64) {
    compute::set_combat_exit_timeout(secs)
}

#[tauri::command]
#[specta::specta]
pub fn set_history_limit(limit: f64) {
    compute::set_history_limit(limit)
}

#[tauri::command]
#[specta::specta]
pub fn get_history() -> Vec<EncounterSnapshot> {
    compute::get_history()
}

#[tauri::command]
#[specta::specta]
pub fn clear_history() {
    compute::clear_history()
}

#[tauri::command]
#[specta::specta]
pub fn set_time_series_config(samples: f64, interval_ms: f64) {
    compute::set_time_series_config(samples, interval_ms)
}

#[tauri::command]
#[specta::specta]
pub fn get_time_series(state: EncState<'_>) -> Vec<TimeSeriesPoint> {
    compute::get_time_series(&state)
}

#[tauri::command]
#[specta::specta]
pub fn set_imagine_only_mode(state: EncState<'_>, enabled: bool) {
    compute::set_imagine_only_mode(&state, enabled)
}

// ─── Self status / buffs ──────────────────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub fn get_self_buff_status(state: EncState<'_>) -> SelfStatusData {
    compute::get_self_buff_status(&state)
}

#[tauri::command]
#[specta::specta]
pub fn get_tracked_buffs(state: EncState<'_>, uids: Vec<f64>) -> TrackedBuffsData {
    compute::get_tracked_buffs(&state, uids)
}

// ─── selected_uid / name cache ────────────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub fn get_selected_uid() -> Option<f64> {
    compute::get_selected_uid()
}

#[tauri::command]
#[specta::specta]
pub fn set_selected_uid(state: EncState<'_>, uid: Option<f64>) {
    compute::set_selected_uid(&state, uid)
}

#[tauri::command]
#[specta::specta]
pub fn lookup_name_cache(uid: f64) -> Option<CachedPlayerDto> {
    compute::lookup_name_cache(uid)
}

// ─── 3min measure mode ───────────────────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub fn start_3min_measure_mode(state: EncState<'_>, duration_secs: f64) {
    compute::start_3min_measure_mode(&state, duration_secs)
}

#[tauri::command]
#[specta::specta]
pub fn cancel_3min_measure_mode(state: EncState<'_>) {
    compute::cancel_3min_measure_mode(&state)
}

#[tauri::command]
#[specta::specta]
pub fn finalize_3min_measure_mode(app: AppHandle, state: EncState<'_>) {
    match state.lock() {
        Ok(mut enc) => {
            let snapshot = compute::finalize_3min_locked(&mut enc);
            if let Err(e) = app.emit("3min-measure-finalized", snapshot) {
                log::error!("Failed to emit 3min-measure-finalized: {e}");
            }
            log::info!("3min measure mode: finalized (UI-driven)");
        }
        Err(e) => log::error!("Lock poisoned in finalize_3min_measure_mode: {e}"),
    }
}

#[tauri::command]
#[specta::specta]
pub fn get_measure_mode_status(state: EncState<'_>) -> MeasureModeStatus {
    compute::get_measure_mode_status(&state)
}

// ─── Window / overlay control ─────────────────────────────────────────────────

#[tauri::command]
#[specta::specta]
pub fn set_always_on_top(window: WebviewWindow, enabled: bool) -> Result<(), String> {
    window.set_always_on_top(enabled).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn set_click_through(window: WebviewWindow, enabled: bool) -> Result<(), String> {
    window
        .set_ignore_cursor_events(enabled)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn set_buffs_window_visible(app: AppHandle, visible: bool) -> Result<(), String> {
    set_overlay_active(&app, "buffs", visible)
}

#[tauri::command]
#[specta::specta]
pub fn set_self_status_window_visible(app: AppHandle, visible: bool) -> Result<(), String> {
    set_overlay_active(&app, "self_status", visible)
}

/// オーバーレイの表示/非表示切替。
///
/// 重要: `hide()`/`show()` (ShowWindow) は使わない。混在DPIのモニタ上では、
/// 兄弟ウィンドウに対する ShowWindow が main(透明)の合成面を不可逆に破壊し
/// 不可視化させるため。ウィンドウは常時表示のまま、ここでは当たり判定
/// (クリックスルー)の切替と、表示ON時の画面内収めだけを行う。
fn set_overlay_active(app: &AppHandle, label: &str, visible: bool) -> Result<(), String> {
    let win = app
        .get_webview_window(label)
        .ok_or_else(|| format!("{label} window not found"))?;
    win.set_ignore_cursor_events(!visible)
        .map_err(|e| e.to_string())?;
    if visible {
        crate::ensure_on_screen(&win);
    }
    Ok(())
}

/// main の不透明度を OS のレイヤードウィンドウ・アルファで適用する。
#[tauri::command]
#[specta::specta]
pub fn set_main_opacity(window: WebviewWindow, opacity: f64) {
    crate::set_window_alpha(&window, opacity);
}
