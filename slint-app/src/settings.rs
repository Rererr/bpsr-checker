//! 設定の永続化（src/stores/settings.ts を移植）。
//! %APPDATA%\bpsr-checker\settings.json に JSON で保存する。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_NAME_TEMPLATE: &str = "{name} {spec}({score} - {seasonLv} - {seasonStr})";
pub const DEFAULT_COPY_TEMPLATE: &str = "{rank}. {name} ({class}) {dmg} / {dps} DPS ({pct})";

/// 自キャラ ステータス窓の項目カタログ: (key, 日本語ラベル, グループ, 既定で表示するか)。
/// ラベル・グループはゲーム内ステータス画面の表記に準拠（docs/images の参考スクショ）。
/// ※ key は設定 `stats_enabled` と Slint の "stat.<key>" トグルキーに対応。
/// ※ 「会心/幸運」はステータス値（attr由来の%）、「会心率/幸運率(実測)」は命中データ由来の別項目。
/// ※ attr_id 未確定の項目（魔攻/魔防/敏捷/会心/万能/レジスト/詠唱速度/会心ダメージ/幸運倍率/器用さ）は
///   実機 probe 確定までは値が「—」表示になる（デモモードではデモ値を表示）。
pub const STAT_CATALOG: &[(&str, &str, &str, bool)] = &[
    // 基本
    ("hp", "HP", "基本", true),
    ("atk-phys", "物理攻撃力", "基本", true),
    ("atk-magic", "魔法攻撃力", "基本", false),
    ("strength", "筋力", "基本", true),
    ("intelligence", "知力", "基本", false),
    ("agility", "敏捷", "基本", false),
    ("endurance", "耐久力", "基本", true),
    ("ability-score", "能力スコア", "基本", false),
    ("season-strength", "幻夢強度", "基本", false),
    // 会心・幸運（実測率＋ステータス値）
    ("crit-rate", "会心率(実測)", "会心・幸運", true),
    ("crit", "会心", "会心・幸運", true),
    ("lucky-rate", "幸運率(実測)", "会心・幸運", true),
    ("lucky", "幸運", "会心・幸運", true),
    // 副次（％）
    ("haste", "ファスト", "副次", true),
    ("dexterity", "器用さ", "副次", false),
    ("versatility", "万能", "副次", false),
    ("resist", "レジスト", "副次", false),
    // 攻撃詳細
    ("attack-speed", "攻撃速度", "攻撃", false),
    ("cast-speed", "詠唱速度", "攻撃", false),
    ("crit-dmg", "会心ダメージ", "攻撃", false),
    ("lucky-dmg", "幸運の一撃倍率", "攻撃", false),
    // 生存
    ("def-phys", "物理防御力", "生存", false),
    ("def-magic", "魔法防御力", "生存", false),
];

/// カタログの既定表示項目（キー一覧）。
pub fn default_stats_enabled() -> Vec<String> {
    STAT_CATALOG
        .iter()
        .filter(|(_, _, _, on)| *on)
        .map(|(k, _, _, _)| k.to_string())
        .collect()
}

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
    /// 自キャラ ステータス窓（攻撃力/会心/HP 等のリアルタイム表示）を出すか。
    pub show_stats_overlay: bool,
    /// ステータス窓に表示する項目キーの一覧（順序＝表示順）。`stat_catalog()` のキーから選択。
    pub stats_enabled: Vec<String>,
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
    /// メインウィンドウをタスクバーに常駐させるか（true=タスクバー表示／最小化はOS最小化、
    /// false=従来のトレイ格納・skip_taskbar）。
    pub show_in_taskbar: bool,
    /// オーバーレイ窓（ステータス／ステータス窓／イマジンタイマー）共通の背景不透明度（0.05〜1.0）。
    /// メイン窓の opacity とは独立。文字・バーは不透明のまま（背景のみ透ける）。
    pub overlay_opacity: f64,
    /// オーバーレイ窓共通の基準テキスト色（プリセットキー white/warm/... または "#rrggbb"）。
    /// 白系ラベル・値テキストのみ再着色し、意味色（アクセント／職業色／デバフ赤）は対象外。
    /// 不透明度・文字色は3窓共通だが、文字サイズ・フォントは窓ごとに独立（下記）。
    pub overlay_text_color: String,
    /// メイン窓のフォントファミリ名（システムにある実フォント名をそのまま保持）。
    /// バフ/デバフ オーバーレイはメイン窓のフォントサイズ(font_size)とこのフォントに追随する。
    pub main_font: String,
    /// メイン窓フォントの太字（既定 false）。バフ/デバフ オーバーレイも追随。
    pub main_font_bold: bool,
    /// ステータス オーバーレイ専用のフォントサイズ（px）・フォント・太字（メインとは独立）。
    pub stats_overlay_font_size: f64,
    pub stats_overlay_font: String,
    pub stats_overlay_font_bold: bool,
    /// イマジンタイマー オーバーレイ専用のフォントサイズ（px）・フォント・太字（メインとは独立）。
    pub imagine_overlay_font_size: f64,
    pub imagine_overlay_font: String,
    pub imagine_overlay_font_bold: bool,
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
            show_stats_overlay: false,
            stats_enabled: default_stats_enabled(),
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
            show_in_taskbar: false,
            overlay_opacity: 0.82,
            overlay_text_color: "white".to_string(),
            main_font: "Yu Gothic UI".to_string(),
            main_font_bold: false,
            stats_overlay_font_size: 12.0,
            stats_overlay_font: "Yu Gothic UI".to_string(),
            stats_overlay_font_bold: false,
            imagine_overlay_font_size: 12.0,
            imagine_overlay_font: "Yu Gothic UI".to_string(),
            imagine_overlay_font_bold: false,
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
