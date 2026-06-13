//! 設定の永続化（src/stores/settings.ts を移植）。
//! %APPDATA%\bpsr-checker\settings.json に JSON で保存する。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_NAME_TEMPLATE: &str = "{name} {spec}({score} - {seasonLv} - {seasonStr})";
pub const DEFAULT_COPY_TEMPLATE: &str = "{rank}. {name} ({class}) {dmg} / {dps} DPS ({pct})";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub opacity: f64,
    pub show_crit: bool,
    pub show_lucky: bool,
    pub show_hpm: bool,
    pub show_score: bool,
    pub show_crit_value: bool,
    pub show_lucky_value: bool,
    pub show_hits: bool,
    pub copy_template: String,
    pub name_template: String,
    pub copy_separator: String,
    pub combat_exit_sec: f64,
    pub poll_interval_ms: f64,
    pub history_limit: f64,
    pub time_series_samples: f64,
    pub time_series_interval_ms: f64,
    pub always_on_top: bool,
    pub font_size: f64,
    pub highlight_local_player: bool,
    pub privacy_mask_names: bool,
    pub startup_tab: String,
    pub remember_window_pos: bool,
    pub graph_player_count: f64,
    pub graph_for_local_player: bool,
    pub three_min_duration_sec: f64,
    pub three_min_auto_open: bool,
    pub abbreviate_scores: bool,
    pub show_buff_overlay: bool,
    pub imagine_only_mode: bool,
    pub show_self_status_overlay: bool,
    pub show_element: bool,
    pub show_damage_mode: bool,
    pub compact_split_mode: bool,
    pub accent_theme: String,
    /// イマジンデバフタイマーへ全プレイヤーを自動追加するか（旧Tauri版の挙動）。
    pub auto_add_players: bool,
    /// イマジンデバフタイマーで表示するイマジン列（4種を個別にON/OFF）。
    pub show_imagine_tina: bool,
    pub show_imagine_aluna: bool,
    pub show_imagine_tarta: bool,
    pub show_imagine_basilisk: bool,
    /// DPS一覧の名前列に食事/シロップバッジを表示するか。
    pub show_consumable: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            opacity: 0.85,
            show_crit: true,
            show_lucky: true,
            show_hpm: false,
            show_score: false,
            show_crit_value: false,
            show_lucky_value: false,
            show_hits: false,
            copy_template: DEFAULT_COPY_TEMPLATE.to_string(),
            name_template: DEFAULT_NAME_TEMPLATE.to_string(),
            copy_separator: "\t".to_string(),
            combat_exit_sec: 8.0,
            poll_interval_ms: 200.0,
            history_limit: 20.0,
            time_series_samples: 60.0,
            time_series_interval_ms: 1000.0,
            always_on_top: true,
            font_size: 12.0,
            highlight_local_player: true,
            privacy_mask_names: false,
            startup_tab: "dps".to_string(),
            remember_window_pos: true,
            graph_player_count: 3.0,
            graph_for_local_player: true,
            three_min_duration_sec: 180.0,
            three_min_auto_open: true,
            abbreviate_scores: false,
            show_buff_overlay: false,
            imagine_only_mode: false,
            show_self_status_overlay: false,
            show_element: true,
            show_damage_mode: true,
            compact_split_mode: false,
            accent_theme: "sky".to_string(),
            auto_add_players: true,
            show_imagine_tina: true,
            show_imagine_aluna: true,
            show_imagine_tarta: true,
            show_imagine_basilisk: true,
            show_consumable: true,
        }
    }
}

fn path() -> PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base).join("bpsr-checker").join("settings.json")
}

pub fn load() -> Settings {
    match std::fs::read_to_string(path()) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

pub fn save(s: &Settings) {
    let p = path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    match serde_json::to_string_pretty(s) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&p, json) {
                log::warn!("settings save failed: {e}");
            }
        }
        Err(e) => log::warn!("settings serialize failed: {e}"),
    }
}
