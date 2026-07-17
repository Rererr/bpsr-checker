//! 設定の永続化（src/stores/settings.ts を移植）。
//! %APPDATA%\bpsr-checker\settings.json に JSON で保存する。

use bpsr_core::engine::runtime_settings::{self, Lang};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_NAME_TEMPLATE: &str = "{name} {spec}({score} - {seasonLv} - {seasonStr}){imagine}";
pub const DEFAULT_COPY_TEMPLATE: &str = "{rank}. {name} ({class}) {dmg} / {dps} DPS ({pct})";

/// 自キャラ ステータス窓の項目定義。ラベル・グループはゲーム内ステータス画面の表記に準拠
/// （docs/images の参考スクショ）。日本語・英語の両ラベルを持ち、表示は `display_lang()` に追従。
/// ※ key は設定 `stats_enabled` と Slint の "stat.<key>" トグルキーに対応。
/// ※ 「会心/幸運」はステータス値（attr由来の%）、「会心率/幸運率(実測)」は命中データ由来の別項目。
/// ※ attr_id 未確定の項目（魔攻/魔防/敏捷/会心/万能/レジスト/詠唱速度/会心ダメージ/幸運倍率/器用さ）は
///   実機 probe 確定までは値が「—」表示になる（デモモードではデモ値を表示）。
pub struct StatDef {
    pub key: &'static str,
    pub label_ja: &'static str,
    pub label_en: &'static str,
    pub group_ja: &'static str,
    pub group_en: &'static str,
    pub default_on: bool,
}

impl StatDef {
    /// 表示言語に応じたラベル（ja 以外は en。zh/ko は保留中のため en にフォールバック）。
    pub fn label(&self) -> &'static str {
        match runtime_settings::display_lang() {
            Lang::Ja => self.label_ja,
            _ => self.label_en,
        }
    }

    /// 表示言語に応じたグループ見出し（ja 以外は en）。
    pub fn group(&self) -> &'static str {
        match runtime_settings::display_lang() {
            Lang::Ja => self.group_ja,
            _ => self.group_en,
        }
    }
}

/// 自キャラ ステータス窓の項目カタログ（表示順）。
pub const STAT_CATALOG: &[StatDef] = &[
    // 基本 / Basic
    StatDef { key: "hp", label_ja: "HP", label_en: "HP", group_ja: "基本", group_en: "Basic", default_on: true },
    StatDef { key: "atk-phys", label_ja: "物理攻撃力", label_en: "Phys ATK", group_ja: "基本", group_en: "Basic", default_on: true },
    StatDef { key: "atk-magic", label_ja: "魔法攻撃力", label_en: "Magic ATK", group_ja: "基本", group_en: "Basic", default_on: false },
    StatDef { key: "strength", label_ja: "筋力", label_en: "Strength", group_ja: "基本", group_en: "Basic", default_on: true },
    StatDef { key: "intelligence", label_ja: "知力", label_en: "Intelligence", group_ja: "基本", group_en: "Basic", default_on: false },
    StatDef { key: "agility", label_ja: "敏捷", label_en: "Agility", group_ja: "基本", group_en: "Basic", default_on: false },
    StatDef { key: "endurance", label_ja: "耐久力", label_en: "Endurance", group_ja: "基本", group_en: "Basic", default_on: true },
    StatDef { key: "ability-score", label_ja: "能力スコア", label_en: "Ability Score", group_ja: "基本", group_en: "Basic", default_on: false },
    StatDef { key: "season-strength", label_ja: "幻夢強度", label_en: "Season Power", group_ja: "基本", group_en: "Basic", default_on: false },
    // 会心・幸運 / Crit & Luck（実測率＋ステータス値）
    StatDef { key: "crit-rate", label_ja: "会心率(実測)", label_en: "Crit Rate (Measured)", group_ja: "会心・幸運", group_en: "Crit & Luck", default_on: true },
    StatDef { key: "crit", label_ja: "会心", label_en: "Crit", group_ja: "会心・幸運", group_en: "Crit & Luck", default_on: true },
    StatDef { key: "lucky-rate", label_ja: "幸運率(実測)", label_en: "Luck Rate (Measured)", group_ja: "会心・幸運", group_en: "Crit & Luck", default_on: true },
    StatDef { key: "lucky", label_ja: "幸運", label_en: "Luck", group_ja: "会心・幸運", group_en: "Crit & Luck", default_on: true },
    // 副次 / Secondary（％）
    StatDef { key: "haste", label_ja: "ファスト", label_en: "Haste", group_ja: "副次", group_en: "Secondary", default_on: true },
    StatDef { key: "dexterity", label_ja: "器用さ", label_en: "Dexterity", group_ja: "副次", group_en: "Secondary", default_on: false },
    StatDef { key: "versatility", label_ja: "万能", label_en: "Versatility", group_ja: "副次", group_en: "Secondary", default_on: false },
    StatDef { key: "resist", label_ja: "レジスト", label_en: "Resist", group_ja: "副次", group_en: "Secondary", default_on: false },
    // 攻撃 / Offense
    StatDef { key: "attack-speed", label_ja: "攻撃速度", label_en: "Attack Speed", group_ja: "攻撃", group_en: "Offense", default_on: false },
    StatDef { key: "cast-speed", label_ja: "詠唱速度", label_en: "Cast Speed", group_ja: "攻撃", group_en: "Offense", default_on: false },
    StatDef { key: "crit-dmg", label_ja: "会心ダメージ", label_en: "Crit DMG", group_ja: "攻撃", group_en: "Offense", default_on: false },
    StatDef { key: "lucky-dmg", label_ja: "幸運の一撃倍率", label_en: "Lucky Strike Mult.", group_ja: "攻撃", group_en: "Offense", default_on: false },
    // 生存 / Survival
    StatDef { key: "def-phys", label_ja: "物理防御力", label_en: "Phys DEF", group_ja: "生存", group_en: "Survival", default_on: false },
    StatDef { key: "def-magic", label_ja: "魔法防御力", label_en: "Magic DEF", group_ja: "生存", group_en: "Survival", default_on: false },
];

/// カタログの既定表示項目（キー一覧）。
pub fn default_stats_enabled() -> Vec<String> {
    STAT_CATALOG
        .iter()
        .filter(|d| d.default_on)
        .map(|d| d.key.to_string())
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
    /// UI 表示言語（"ja"=日本語 / "en"=English）。bundled translations のロケール名。
    pub language: String,
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
    /// イマジンデバフタイマーをメインDPS画面と同期するか（既定 true）。
    /// imagine_only_mode と相互排他（片方ONはもう片方を自動OFF）。
    /// true: 表示する顔ぶれ・並び順をメイン一覧へ自動追従し、ピンは「タイマーから隠す/表示」
    ///       （excluded の出し入れ）として機能する。並びは sync_order_follow を参照。
    /// false: 従来の手動ウォッチ（ピンで追加した watched のみ表示）。
    pub sync_timer_with_main: bool,
    /// 同期ON時、並び順までメインDPS一覧へ追従するか（既定 true）。sync_timer_with_main の子設定。
    /// true: メインDPS順そのまま。false: 自分(local uid)を先頭固定＋以降は安定順（uid昇順）。
    /// sync_timer_with_main=false または imagine_only_mode=true のときは参照されない。
    pub sync_order_follow: bool,
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
    /// イマジンタイマーの行を詰めて高さを下げるか（密表示）。
    pub imagine_compact_rows: bool,
    /// オーバーレイ文字の縁取り（stroke）を描くか。低不透明度・透明 HUD で文字を背景から浮かせる。
    /// 影（overlay_shadow）とは独立に ON/OFF できる（既定 true）。
    pub overlay_outline: bool,
    /// オーバーレイ文字の影（ドロップシャドウ）を描くか。縁取りとは独立。
    /// 小さい文字（ステータス行 10px 等）では 1px の影が文字を二重化し黒く汚れて見えるため既定 false。
    /// 透明 HUD で更に視認性を上げたい場合のみ任意で ON。
    pub overlay_shadow: bool,
    /// メイン窓最下部のフッター（お問い合わせ／GitHub報告リンク）を表示するか（既定 true）。
    pub show_footer: bool,
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
            language: "ja".to_string(),
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
            sync_timer_with_main: true,
            sync_order_follow: true,
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
            imagine_compact_rows: false,
            overlay_outline: true,
            overlay_shadow: false,
            show_footer: true,
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
