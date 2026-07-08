// Blue Protocol: Star Resonance DPSチェッカー（Slint版・移行中）
// S1: core→Slint のライブ配線（capture スレッド→共有 EncounterMutex→UIポーリング）。
// リリースではコンソールを出さない（CJK の ICU 行分割警告は dev 時のみ・実害なし）。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

slint::include_modules!();

mod buff_names;
mod capture;
mod consumable_names;
mod format;
mod overlay;
mod settings;
#[cfg(windows)]
mod tray;
mod watchlist;
mod window_state;

use bpsr_core::compute;
use bpsr_core::engine;
use bpsr_core::engine::encounter::EncounterMutex;
use slint::{ComponentHandle, Model, Timer, TimerMode, VecModel};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

const SETTLE_TICKS: u64 = 5;

/// オーバーレイ窓の枠操作コールバック(ドラッグ/リサイズ/最小化/×閉じ)を一括配線する。
/// self_overlay と buff_overlay で同一の配線を共有するためのマクロ。
/// `$close_key` は ×閉じで OFF にする設定トグルのキー(invoke_set_bool 経由)。
macro_rules! wire_overlay_chrome {
    ($overlay:expr, $main:expr, $close_key:literal) => {{
        {
            let w = $overlay.as_weak();
            $overlay.on_start_drag(move || {
                if let Some(o) = w.upgrade() {
                    overlay::start_drag(o.window());
                }
            });
        }
        {
            let w = $overlay.as_weak();
            $overlay.on_start_resize(move |dir| {
                if let Some(o) = w.upgrade() {
                    overlay::start_resize(o.window(), dir);
                }
            });
        }
        // ×閉じる → 設定トグルOFFと連動（invoke_set_bool で既存ハンドラを再利用）
        {
            let mw = $main.as_weak();
            $overlay.on_close_window(move || {
                if let Some(m) = mw.upgrade() {
                    m.invoke_set_bool($close_key.into(), false);
                }
            });
        }
        // 最小化（タスクバー常駐モード時のみボタン表示）→ OS最小化でタスクバーへ格納
        {
            let w = $overlay.as_weak();
            $overlay.on_minimize(move || {
                if let Some(o) = w.upgrade() {
                    overlay::minimize_window(o.window());
                }
            });
        }
    }};
}

/// 最小ロガー。core の capture / 本体の診断ログを stderr へ出す。
/// （Slint/parley の CJK 警告は log ではなく直接 eprintln のため別物・ここでは触れない）
struct ConsoleLog;
impl log::Log for ConsoleLog {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, r: &log::Record) {
        let line = format!("[{}] {}: {}", r.level(), r.target(), r.args());
        eprintln!("{line}");
        // 管理者権限(UAC)起動では cargo の端末に stderr が届かないため、診断用にファイルへも出す。
        // 起動ごとに truncate して 1 起動 = 1 ファイルにする。
        log_to_file(&line);
    }
    fn flush(&self) {}
}

/// ログ出力先ファイルのパス（%APPDATA%\bpsr-checker\bpsr-checker.log）。
fn log_file_path() -> std::path::PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(base)
        .join("bpsr-checker")
        .join("bpsr-checker.log")
}

/// ログ1行をファイルへ追記（初回呼び出しで truncate して開く）。
fn log_to_file(line: &str) {
    use std::io::Write;
    use std::sync::{Mutex, OnceLock};
    static FILE: OnceLock<Mutex<Option<std::fs::File>>> = OnceLock::new();
    let m = FILE.get_or_init(|| {
        let path = log_file_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .ok();
        Mutex::new(f)
    });
    if let Ok(mut g) = m.lock() {
        if let Some(f) = g.as_mut() {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// 二重起動防止（Windows 名前付き Mutex）。既に起動済みなら true。
#[cfg(windows)]
fn already_running() -> bool {
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::PCWSTR;
    let name: Vec<u16> = "Global\\bpsr-checker-slint-instance\0"
        .encode_utf16()
        .collect();
    unsafe {
        match CreateMutexW(None, true, PCWSTR(name.as_ptr())) {
            // ハンドルは閉じない＝プロセス寿命まで mutex を保持する。
            Ok(_handle) => GetLastError() == ERROR_ALREADY_EXISTS,
            Err(_) => false,
        }
    }
}

#[cfg(not(windows))]
fn already_running() -> bool {
    false
}

fn data_dir() -> std::path::PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(base).join("bpsr-checker")
}

/// タブ(0=dps 1=heal 2=taken 3=history)に応じてプレイヤー一覧を取得。
/// history(3) は S5 実装まで dps を表示する暫定。
fn fetch_players(enc: &EncounterMutex, tab: i32) -> bpsr_core::models::PlayersWindow {
    match tab {
        1 => compute::get_heal_players(enc),
        2 => compute::get_dmg_taken_players(enc),
        _ => compute::get_dps_players(enc),
    }
}

/// グラフ列(DPS推移)を出すか。graph設定が有効で、被ダメ(tab=2)以外。
fn graph_col_active(c: &settings::Settings, tab: i32) -> bool {
    (c.graph_player_count > 0.0 || c.graph_for_local_player) && tab != 2
}

/// 通常 rows へ反映しつつ、軽量分割表示用に前半/後半カラムへも分配する。
fn apply_player_rows(
    rows: &slint::VecModel<Row>,
    left: &slint::VecModel<Row>,
    right: &slint::VecModel<Row>,
    built: Vec<Row>,
) {
    let half = built.len().div_ceil(2);
    sync_rows(left, &built[..half]);
    sync_rows(right, &built[half..]);
    sync_rows(rows, &built);
}

/// 行数が同じならデリゲートを再生成せず in-place 更新する。
/// set_vec はモデルをリセットしリピータが要素を作り直すため、ホバー中の
/// 食事/シロップ ツールチップ（PopupWindow）が毎poll閉じ開きしてちらつく。
/// 行数が一致する間は set_row_data でデータのみ差し替え、ホバー状態を保つ。
fn sync_rows(model: &slint::VecModel<Row>, data: &[Row]) {
    if model.row_count() == data.len() {
        for (i, r) in data.iter().enumerate() {
            model.set_row_data(i, r.clone());
        }
    } else {
        model.set_vec(data.to_vec());
    }
}

#[allow(clippy::too_many_arguments)]
/// 食事/シロップ等の消耗バフの表示用派生値を計算する。
/// 戻り値: (アクティブか, 残量割合0..1, 残り時間テキスト, 種類ラベル)。
/// duration/remaining いずれかが 0 以下なら未使用扱い(空文字・0)。
fn consumable_display(remaining_ms: f64, duration_ms: f64, base_id: i32) -> (bool, f32, String, String) {
    if duration_ms <= 0.0 || remaining_ms <= 0.0 {
        return (false, 0.0, String::new(), String::new());
    }
    let ratio = (remaining_ms / duration_ms).clamp(0.0, 1.0) as f32;
    let time = format::format_consumable_remaining(remaining_ms as i64, duration_ms as i64);
    let label = consumable_names::label(base_id).unwrap_or_default();
    (true, ratio, time, label)
}

fn build_rows(
    pw: &bpsr_core::models::PlayersWindow,
    template: &str,
    abbreviate: bool,
    privacy: bool,
    watched: &[i64],
    graph_count: i32,
    graph_for_local: bool,
) -> Vec<Row> {
    let top = pw.top_value.max(1.0);
    let local = pw.local_player_uid;
    // 非ローカルの上位 graph_count 人＋（設定時）ローカルにグラフを出す。
    let mut non_local_above: i32 = 0;
    let mut out = Vec::with_capacity(pw.player_rows.len());
    for (i, p) in pw.player_rows.iter().enumerate() {
        let rank = (i + 1) as i32;
        let is_local = p.uid == local;
        let show_spark = if is_local {
            graph_for_local
        } else {
            non_local_above < graph_count
        };
        let spark = if show_spark {
            build_spark_commands(&p.time_series, 100.0, 16.0)
        } else {
            String::new()
        };
        if !is_local {
            non_local_above += 1;
        }
        // 食事/シロップの残量割合（0..1。アイコンの色が上から縦に抜ける）＋ホバー用の
        // 残り時間テキスト・種類ラベル（base_id→日本語効果名）。
        let (food_act, food_remaining, food_time, food_label) =
            consumable_display(p.food_remaining_ms, p.food_duration_ms, p.food_base_id);
        let (syrup_act, syrup_remaining, syrup_time, syrup_label) =
            consumable_display(p.syrup_remaining_ms, p.syrup_duration_ms, p.syrup_base_id);
        let display = if privacy {
            format::mask_player_name(p.uid as i64)
        } else {
            p.name.clone()
        };
        out.push(Row {
            rank,
            uid_str: format!("{}", p.uid as i64).into(),
            name: format::format_row_name(
                &display,
                &p.class_name,
                &p.class_spec_name,
                p.ability_score,
                p.season_level,
                p.season_strength,
                &p.imagine_suffix,
                rank,
                template,
                abbreviate,
            )
            .into(),
            class_color: format::class_color(&p.class_name),
            dmg_text: format::format_number(p.total_value).into(),
            dps_text: format::format_dps(p.value_per_sec).into(),
            pct_text: format::format_pct(p.value_pct).into(),
            pct: ((p.total_value / top) * 100.0) as f32,
            is_local,
            crit_text: format::format_pct(p.crit_rate).into(),
            crit_value_text: format::format_pct(p.crit_value_rate).into(),
            lucky_text: format::format_pct(p.lucky_rate).into(),
            lucky_value_text: format::format_pct(p.lucky_value_rate).into(),
            hits_text: format!("{}", p.hits as i64).into(),
            hpm_text: format!("{:.1}", p.hits_per_minute).into(),
            score_text: if p.ability_score > 0.0 {
                format::format_score(p.ability_score, abbreviate)
            } else {
                "-".to_string()
            }
            .into(),
            watched: watched.contains(&(p.uid as i64)),
            spark_commands: spark.into(),
            food_active: food_act,
            food_remaining,
            food_time: food_time.into(),
            food_label: food_label.into(),
            syrup_active: syrup_act,
            syrup_remaining,
            syrup_time: syrup_time.into(),
            syrup_label: syrup_label.into(),
        });
    }
    out
}

fn build_skill_rows(sw: &bpsr_core::models::SkillsWindow) -> Vec<SkillRowUi> {
    let top = sw.top_value.max(1.0);
    sw.skill_rows
        .iter()
        .map(|s| {
            let (_, ec) = format::element_label(s.element);
            SkillRowUi {
                uid_str: format!("{}", s.uid as i64).into(),
                name: s.name.clone().into(),
                elem_id: s.element as i32,
                elem_color: ec,
                total_text: format::format_number(s.total_value).into(),
                dps_text: format::format_dps(s.value_per_sec).into(),
                pct_text: format::format_pct(s.value_pct).into(),
                pct: ((s.total_value / top) * 100.0) as f32,
            }
        })
        .collect()
}

/// 履歴ビューのフラット行を構築（見出し＋展開中のみプレイヤー行）。
fn build_history_rows(
    hist: &[bpsr_core::models::EncounterSnapshot],
    expanded: Option<i64>,
    privacy: bool,
) -> Vec<HistoryRowUi> {
    let mut out = Vec::new();
    for snap in hist {
        let id = snap.id as i64;
        let is_exp = expanded == Some(id);
        out.push(HistoryRowUi {
            is_header: true,
            snap_id: format!("{id}").into(),
            expanded: is_exp,
            duration_text: format::format_elapsed(snap.duration_ms).into(),
            dps_text: format::format_dps(snap.total_dps).into(),
            dmg_text: format::format_number(snap.total_dmg).into(),
            count_text: format!("{}", snap.player_rows.len()).into(),
            ..Default::default()
        });
        if is_exp {
            let top = snap
                .player_rows
                .first()
                .map(|p| p.total_value)
                .unwrap_or(1.0)
                .max(1.0);
            for (i, p) in snap.player_rows.iter().enumerate() {
                let name = if privacy {
                    format::mask_player_name(p.uid as i64)
                } else {
                    p.name.clone()
                };
                out.push(HistoryRowUi {
                    is_header: false,
                    rank_text: format!("{}.", i + 1).into(),
                    name: name.into(),
                    class_color: format::class_color(&p.class_name),
                    p_dps_text: format::format_dps(p.value_per_sec).into(),
                    p_pct_text: format::format_pct(p.value_pct).into(),
                    p_pct: ((p.total_value / top) * 100.0) as f32,
                    ..Default::default()
                });
            }
        }
    }
    out
}

/// 時系列を viewbox(vw×vh) 内の折れ線 SVG パスへ。値抽出は sel で指定。Sparkline.tsx 移植。
/// 点が2未満なら空文字（呼び出し側で非表示判定に使う）。
fn build_spark_with(
    points: &[bpsr_core::models::TimeSeriesPoint],
    vw: f32,
    vh: f32,
    sel: impl Fn(&bpsr_core::models::TimeSeriesPoint) -> f64,
) -> String {
    if points.len() < 2 {
        return String::new();
    }
    let max = points.iter().map(&sel).fold(1.0_f64, f64::max);
    let step = vw / (points.len() - 1) as f32;
    let mut s = String::with_capacity(points.len() * 12);
    for (i, p) in points.iter().enumerate() {
        let x = i as f32 * step;
        let y = vh - (sel(p) / max) as f32 * vh;
        if i == 0 {
            s.push_str(&format!("M {x:.1} {y:.1}"));
        } else {
            s.push_str(&format!(" L {x:.1} {y:.1}"));
        }
    }
    s
}

/// 窓DPS の折れ線（ヘッダー/プレイヤースパークライン＋3分計測 結果のキャラ/スキル推移用）。
/// 累積ダメージだと単調右肩上がりでバースト区間が判別できないため、区間DPS で起伏を見せる。
fn build_spark_commands(points: &[bpsr_core::models::TimeSeriesPoint], vw: f32, vh: f32) -> String {
    build_spark_with(points, vw, vh, |p| p.total_dps)
}

/// PlayerRow → コピーテンプレ用データ（copy-list / 結果コピーで共用）。
fn copy_row_data(p: &bpsr_core::models::PlayerRow, rank: i32) -> format::CopyRowData<'_> {
    format::CopyRowData {
        rank,
        name: &p.name,
        class_name: &p.class_name,
        class_spec_name: &p.class_spec_name,
        total_value: p.total_value,
        value_per_sec: p.value_per_sec,
        value_pct: p.value_pct,
        crit_rate: p.crit_rate,
        crit_value_rate: p.crit_value_rate,
        lucky_rate: p.lucky_rate,
        lucky_value_rate: p.lucky_value_rate,
        hits: p.hits,
        hits_per_minute: p.hits_per_minute,
        ability_score: p.ability_score,
        season_level: p.season_level,
        season_strength: p.season_strength,
    }
}

/// スキル内訳 円グラフのパレット（スキル毎に色を変えて区別する）。
const SKILL_PIE_PALETTE: [u32; 10] = [
    0x4fc3f7, 0xff7043, 0x66bb6a, 0xffca28, 0xab47bc, 0x26c6da, 0xec407a, 0x9ccc65, 0xff8a65,
    0x7e57c2,
];

fn palette_color(i: usize) -> slint::Color {
    let hex = SKILL_PIE_PALETTE[i % SKILL_PIE_PALETTE.len()];
    slint::Color::from_rgb_u8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// 「その他」スライスの色（TOP10 以外の集約・グレー）。パレットと合わせて全11色。
const OTHER_SLICE_COLOR: u32 = 0x9e9e9e;

fn rgb(hex: u32) -> slint::Color {
    slint::Color::from_rgb_u8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// 着色済み (色, 値) リストから扇形＋灰色の内向き区切り線を生成。viewbox 100x100。
fn pie_slices(entries: &[(slint::Color, f64)]) -> Vec<PieSlice> {
    let sum: f64 = entries.iter().map(|e| e.1).sum();
    if sum <= 0.0 {
        return Vec::new();
    }
    let (cx, cy, r) = (50.0_f32, 50.0_f32, 48.0_f32);
    let mut out = Vec::new();
    // 扇形（塗り・stroke なし）
    let mut start = -std::f32::consts::FRAC_PI_2;
    for (color, v) in entries.iter() {
        let frac = (*v / sum) as f32;
        if frac <= 0.0 {
            continue;
        }
        if frac >= 0.999 {
            out.push(PieSlice {
                commands: format!(
                    "M {cx} {} A {r} {r} 0 1 1 {cx} {} A {r} {r} 0 1 1 {cx} {} Z",
                    cy - r,
                    cy + r,
                    cy - r
                )
                .into(),
                color: *color,
                is_line: false,
            });
            return out; // 単独100%は区切り線不要
        }
        let sweep = frac * std::f32::consts::TAU;
        let end = start + sweep;
        let x0 = cx + r * start.cos();
        let y0 = cy + r * start.sin();
        let x1 = cx + r * end.cos();
        let y1 = cy + r * end.sin();
        let large = if sweep > std::f32::consts::PI { 1 } else { 0 };
        out.push(PieSlice {
            commands: format!(
                "M {cx} {cy} L {x0:.2} {y0:.2} A {r} {r} 0 {large} 1 {x1:.2} {y1:.2} Z"
            )
            .into(),
            color: *color,
            is_line: false,
        });
        start = end;
    }
    // 区切り線（中心→外周手前 0.86r・灰色）。各スライス境界に1本。
    let r_out = r * 0.86;
    let line_color = rgb(0x8a8a8a);
    let mut a = -std::f32::consts::FRAC_PI_2;
    for (_, v) in entries.iter() {
        let frac = (*v / sum) as f32;
        if frac <= 0.0 {
            continue;
        }
        let lx = cx + r_out * a.cos();
        let ly = cy + r_out * a.sin();
        out.push(PieSlice {
            commands: format!("M {cx} {cy} L {lx:.2} {ly:.2}").into(),
            color: line_color,
            is_line: true,
        });
        a += frac * std::f32::consts::TAU;
    }
    out
}

/// 表示言語が日本語か。Rust 側で組み立てる動的文字列の ja/en 切替に使う
/// （ja 以外は en。zh/ko は保留中のため en にフォールバック）。
fn is_ja() -> bool {
    engine::runtime_settings::display_lang() == engine::runtime_settings::Lang::Ja
}

/// スキル内訳を TOP10（詳細名）＋「その他」(残り集約) の (表示名, 色, 値) へ集約。
/// 円グラフ・凡例で共用。skills は降順ソート済を前提。
fn top10_with_other(skills: &[bpsr_core::models::SkillRow]) -> Vec<(String, slint::Color, f64)> {
    let mut out: Vec<(String, slint::Color, f64)> = skills
        .iter()
        .take(10)
        .enumerate()
        .map(|(i, s)| (s.name.clone(), palette_color(i), s.total_value))
        .collect();
    let other: f64 = skills.iter().skip(10).map(|s| s.total_value).sum();
    if other > 0.0 {
        let other_label = if is_ja() { "その他" } else { "Other" };
        out.push((other_label.to_string(), rgb(OTHER_SLICE_COLOR), other));
    }
    out
}

/// 集約済みエントリ (表示名, 色, 値) から凡例行を生成（色・割合は円グラフと一致）。
fn legend_from(entries: &[(String, slint::Color, f64)]) -> Vec<SkillLegendUi> {
    let sum = entries.iter().map(|e| e.2).sum::<f64>().max(1.0);
    entries
        .iter()
        .map(|(name, color, v)| SkillLegendUi {
            name: name.clone().into(),
            color: *color,
            pct_text: format!("{:.1}%", v / sum * 100.0).into(),
        })
        .collect()
}

/// 結果パネルのプレイヤー行（uid・選択状態付き）。エリア1 は最大5キャラ。
fn build_result_rows(
    snap: &bpsr_core::models::EncounterSnapshot,
    selected_uid: i64,
    privacy: bool,
) -> Vec<ResultRowUi> {
    snap.player_rows
        .iter()
        .take(5)
        .enumerate()
        .map(|(i, p)| {
            let uid = p.uid as i64;
            let name = if privacy {
                format::mask_player_name(uid)
            } else {
                p.name.clone()
            };
            ResultRowUi {
                rank_text: format!("{}.", i + 1).into(),
                name: name.into(),
                class_color: format::class_color(&p.class_name),
                dps_text: format::format_dps(p.value_per_sec).into(),
                dmg_text: format::format_number(p.total_value).into(),
                pct_text: format::format_pct(p.value_pct).into(),
                uid_str: format!("{uid}").into(),
                selected: uid == selected_uid,
            }
        })
        .collect()
}

/// 結果パネル エリア2 のスキル行（属性色・選択状態付き）。skills は降順ソート済。
fn build_result_skill_rows(
    skills: &[bpsr_core::models::SkillRow],
    selected_skill_uid: i64,
) -> Vec<ResultSkillRowUi> {
    skills
        .iter()
        .map(|s| {
            let (_, ec) = format::element_label(s.element);
            let uid = s.uid as i64;
            ResultSkillRowUi {
                uid_str: format!("{uid}").into(),
                name: s.name.clone().into(),
                elem_id: s.element as i32,
                elem_color: ec,
                dmg_text: format::format_number(s.total_value).into(),
                pct_text: format::format_pct(s.value_pct).into(),
                selected: uid == selected_skill_uid,
            }
        })
        .collect()
}

/// 区間DPS の折れ線を「時間軸」で配置（x = t_ms/duration）。結果画面の X軸時間ラベルと整合させる。
/// バーストの起伏は実時間位置で描く（index 等間隔だと全幅へ引き伸ばして誤解を招くため）。
/// ただし左端は 0:00 へ接地する: 初使用が 0:00 より後の系列（途中から使ったスキル/途中参戦
/// キャラ）は (x=0, dps=0) から初使用直前まで底辺の平坦線を引き、折れ線を必ず左端へ届かせる。
/// 右端は確定時の終端サンプル（compute::seal_3min_series）で計測末尾へ接地済み。
/// 折れ線パスを「プロット実寸(px)座標」で生成する（M/L コマンド文字列）。
/// SparkGraph 側は viewbox をプロット実寸に一致させるため、ここでも実寸 vw×vh で座標を出す
/// （Slint Path の縦横比保持による中央寄せ・横潰れを回避＝X全幅・Y軸グリッドと整合）。
/// 実寸が変わったら再生成が必要（SparkGraph.resized 経由で呼ぶ）。
fn build_spark_dps_time(
    points: &[bpsr_core::models::TimeSeriesPoint],
    duration_ms: f64,
    vw: f32,
    vh: f32,
) -> String {
    if points.len() < 2 || vw <= 0.0 || vh <= 0.0 {
        return String::new();
    }
    let max = points.iter().map(|p| p.total_dps).fold(1.0_f64, f64::max);
    let dur = duration_ms.max(1.0);
    let mut s = String::with_capacity((points.len() + 2) * 14);

    // 左端接地: 最初のサンプルが 0:00 より後（途中から使ったスキル/途中参戦キャラ）なら、
    // (x=0, dps=0) から初使用直前まで底辺の平坦線を引き、折れ線を必ず左端へ届かせる。
    // 未使用区間=0 の表現なので誤解はなく、右端は終端サンプルで既に接地している。
    let first_x = (points[0].t_ms / dur).clamp(0.0, 1.0) as f32 * vw;
    let mut drawn = false;
    if first_x > 0.5 {
        s.push_str(&format!("M 0.0 {vh:.1} L {first_x:.2} {vh:.1}"));
        drawn = true;
    }
    for p in points {
        let x = (p.t_ms / dur).clamp(0.0, 1.0) as f32 * vw;
        let y = vh - (p.total_dps / max) as f32 * vh;
        if drawn {
            s.push_str(&format!(" L {x:.2} {y:.2}"));
        } else {
            s.push_str(&format!("M {x:.2} {y:.2}"));
            drawn = true;
        }
    }
    s
}

/// 選択キャラの区間DPS折れ線（エリア1）。snap の player_rows[uid] の time_series から。
fn build_char_spark(snap: &bpsr_core::models::EncounterSnapshot, uid: i64, vw: f32, vh: f32) -> String {
    snap.player_rows
        .iter()
        .find(|p| p.uid as i64 == uid)
        .map(|p| build_spark_dps_time(&p.time_series, snap.duration_ms, vw, vh))
        .unwrap_or_default()
}

/// 選択スキルの区間DPS折れ線（エリア4）。duration は計測全体（snap.duration_ms）で統一。
fn build_skill_spark(
    skills: &[bpsr_core::models::SkillRow],
    selected_skill_uid: i64,
    duration_ms: f64,
    vw: f32,
    vh: f32,
) -> String {
    skills
        .iter()
        .find(|s| s.uid as i64 == selected_skill_uid)
        .map(|s| build_spark_dps_time(&s.time_series, duration_ms, vw, vh))
        .unwrap_or_default()
}

/// 折れ線の Y軸目安ラベル (上=最大DPS, 中=その半分)。下端は UI 側で常に "0"。
fn spark_axis_labels(points: &[bpsr_core::models::TimeSeriesPoint]) -> (String, String) {
    let max = points.iter().map(|p| p.total_dps).fold(0.0_f64, f64::max);
    (format::format_number(max), format::format_number(max / 2.0))
}

fn char_axis_labels(snap: &bpsr_core::models::EncounterSnapshot, uid: i64) -> (String, String) {
    snap.player_rows
        .iter()
        .find(|p| p.uid as i64 == uid)
        .map(|p| spark_axis_labels(&p.time_series))
        .unwrap_or_default()
}

fn skill_axis_labels(
    skills: &[bpsr_core::models::SkillRow],
    selected_skill_uid: i64,
) -> (String, String) {
    skills
        .iter()
        .find(|s| s.uid as i64 == selected_skill_uid)
        .map(|s| spark_axis_labels(&s.time_series))
        .unwrap_or_default()
}

/// 選択スキルに応じて スキル行ハイライト・スキル折れ線・ラベル のみ更新（円グラフ/凡例は据置）。
fn apply_result_skill_selection(
    m: &MainWindow,
    skills: &[bpsr_core::models::SkillRow],
    selected_skill_uid: i64,
    result_skill_rows: &slint::VecModel<ResultSkillRowUi>,
    duration_ms: f64,
    skill_dims: (f32, f32),
) {
    result_skill_rows.set_vec(build_result_skill_rows(skills, selected_skill_uid));
    let spark = build_skill_spark(skills, selected_skill_uid, duration_ms, skill_dims.0, skill_dims.1);
    m.set_result_skill_spark_visible(!spark.is_empty());
    m.set_result_skill_spark(spark.into());
    let (stop, smid) = skill_axis_labels(skills, selected_skill_uid);
    m.set_result_skill_axis_top(stop.into());
    m.set_result_skill_axis_mid(smid.into());
    // エリア4 折れ線ラベル: 選択スキル名
    let skill_name = skills
        .iter()
        .find(|s| s.uid as i64 == selected_skill_uid)
        .map(|s| s.name.clone())
        .unwrap_or_default();
    m.set_result_skill_name(skill_name.into());
}

/// 選択プレイヤーに応じて 行ハイライト・キャラ折れ線・タイトル・スキル行/折れ線・円グラフ/凡例 を更新。
/// 既定スキルは内訳の先頭（最大）を選択する。
#[allow(clippy::too_many_arguments)]
fn apply_result_selection(
    m: &MainWindow,
    uid: i64,
    snap: &bpsr_core::models::EncounterSnapshot,
    captured: &std::collections::HashMap<i64, Vec<bpsr_core::models::SkillRow>>,
    result_rows: &slint::VecModel<ResultRowUi>,
    result_skill_rows: &slint::VecModel<ResultSkillRowUi>,
    result_pie: &slint::VecModel<PieSlice>,
    result_legend: &slint::VecModel<SkillLegendUi>,
    selected_player: &std::cell::Cell<i64>,
    selected_skill: &std::cell::Cell<i64>,
    privacy: bool,
    char_dims: (f32, f32),
    skill_dims: (f32, f32),
) {
    selected_player.set(uid);
    result_rows.set_vec(build_result_rows(snap, uid, privacy));
    // エリア1: 選択キャラの区間DPS折れ線
    let char_spark = build_char_spark(snap, uid, char_dims.0, char_dims.1);
    m.set_result_char_spark_visible(!char_spark.is_empty());
    m.set_result_char_spark(char_spark.into());
    let (ctop, cmid) = char_axis_labels(snap, uid);
    m.set_result_char_axis_top(ctop.into());
    m.set_result_char_axis_mid(cmid.into());
    // タイトル
    let pname = snap
        .player_rows
        .iter()
        .find(|p| p.uid as i64 == uid)
        .map(|p| {
            if privacy {
                format::mask_player_name(uid)
            } else {
                p.name.clone()
            }
        })
        .unwrap_or_default();
    let pie_title = if is_ja() {
        format!("{pname} のスキル内訳")
    } else {
        format!("{pname} — Skill Breakdown")
    };
    m.set_result_pie_title(pie_title.into());
    m.set_result_char_name(pname.clone().into()); // エリア1 折れ線ラベル
    // エリア2/3: スキル内訳。既定選択=先頭(最大)。
    let empty = Vec::new();
    let skills = captured.get(&uid).unwrap_or(&empty);
    let default_skill = skills.first().map(|s| s.uid as i64).unwrap_or(0);
    selected_skill.set(default_skill);
    apply_result_skill_selection(m, skills, default_skill, result_skill_rows, snap.duration_ms, skill_dims);
    // エリア3: TOP10＋その他 の円グラフ＋凡例
    let entries = top10_with_other(skills);
    let colored: Vec<(slint::Color, f64)> = entries.iter().map(|(_, c, v)| (*c, *v)).collect();
    result_pie.set_vec(pie_slices(&colored));
    result_legend.set_vec(legend_from(&entries));
}

/// 3分計測 結果パネルへスナップショットを反映して開く。
#[allow(clippy::too_many_arguments)]
fn show_result(
    m: &MainWindow,
    snap: &bpsr_core::models::EncounterSnapshot,
    captured: &std::collections::HashMap<i64, Vec<bpsr_core::models::SkillRow>>,
    default_uid: i64,
    result_rows: &slint::VecModel<ResultRowUi>,
    result_skill_rows: &slint::VecModel<ResultSkillRowUi>,
    result_pie: &slint::VecModel<PieSlice>,
    result_legend: &slint::VecModel<SkillLegendUi>,
    selected_player: &std::cell::Cell<i64>,
    selected_skill: &std::cell::Cell<i64>,
    privacy: bool,
    char_dims: (f32, f32),
    skill_dims: (f32, f32),
) {
    m.set_result_dps(format::format_dps(snap.total_dps).into());
    m.set_result_dmg(format::format_number(snap.total_dmg).into());
    m.set_result_duration(format::format_elapsed(snap.duration_ms).into());
    m.set_result_duration_ms(snap.duration_ms as f32); // X軸 時間ラベル用
    apply_result_selection(
        m,
        default_uid,
        snap,
        captured,
        result_rows,
        result_skill_rows,
        result_pie,
        result_legend,
        selected_player,
        selected_skill,
        privacy,
        char_dims,
        skill_dims,
    );
    m.set_result_open(true);
}

/// uid の下4桁（候補ラベル用。元 UI の String(uid).slice(-4) 相当）。
fn last4(uid: i64) -> String {
    let s = uid.to_string();
    s[s.len().saturating_sub(4)..].to_string()
}

/// 自キャラUID 候補（現在の DPS プレイヤー）を再構築。selected も反映。
/// 入力欄(selected-uid-value)は触らない＝設定パネルを開いたまま poll で呼んでも
/// 入力中をクロバーしない。
fn refresh_uid_candidates(
    enc: &EncounterMutex,
    candidates: &slint::VecModel<UidCandidate>,
) {
    let sel = compute::get_selected_uid().map(|v| v as i64);
    let pw = compute::get_dps_players(enc);
    let cands: Vec<UidCandidate> = pw
        .player_rows
        .iter()
        .take(12)
        .map(|p| {
            let uid = p.uid as i64;
            UidCandidate {
                uid_str: format!("{uid}").into(),
                label: format!("{} #{}", p.name, last4(uid)).into(),
                selected: Some(uid) == sel,
            }
        })
        .collect();
    candidates.set_vec(cands);
}

/// 入力欄・解決名・候補をまとめて更新（パネル開時／確定時のみ。入力欄を push する）。
fn refresh_selected_uid(
    m: &MainWindow,
    enc: &EncounterMutex,
    candidates: &slint::VecModel<UidCandidate>,
) {
    let sel = compute::get_selected_uid();
    let sel_i64 = sel.map(|v| v as i64);
    m.set_selected_uid_value(
        sel_i64
            .map(|u| u.to_string())
            .unwrap_or_default()
            .into(),
    );
    let name = sel.and_then(compute::lookup_name_cache).map(|d| d.name);
    m.set_selected_uid_name(match (name, sel_i64) {
        (Some(n), _) => n.into(),
        (None, Some(_)) => if is_ja() { "（名前未解決）" } else { "(name unresolved)" }.into(),
        (None, None) => if is_ja() { "（未設定）" } else { "(none)" }.into(),
    });
    refresh_uid_candidates(enc, candidates);
}

/// ドリルダウン状態。
#[derive(Clone, Copy)]
enum Drill {
    None,
    Skills(i64),            // dps/heal: そのプレイヤーの技別
    TakenAttackers(i64),    // 被ダメ: 被害者の攻撃元一覧
    TakenSkills(i64, i64),  // 被ダメ: (被害者, 攻撃元) の技別
}

/// SkillsWindow を内訳ビューへ反映する共通処理。
fn show_drill(
    m: &MainWindow,
    sk_rows: &slint::VecModel<SkillRowUi>,
    sw: &bpsr_core::models::SkillsWindow,
    clickable: bool,
) {
    // 名前列テンプレートで {imagine} が消されていても、見出しは装備中イマジンを強制表示する。
    m.set_inspected_name(
        format!(
            "{}{}",
            sw.inspected_player.name, sw.inspected_player.imagine_suffix
        )
        .into(),
    );
    sk_rows.set_vec(build_skill_rows(sw));
    m.set_skills_clickable(clickable);
    m.set_view(1);
}

/// アクセントカラー設定 → (accent, accent-strong)（0xAARRGGBB）。
/// "#rrggbb" 指定時は彩度を上げ明度を落とした派生色を accent-strong（塗り）に用いる。
/// プリセット名（旧設定）も後方互換で受け付け、未知値は既定 sky にフォールバックする。
fn accent_colors(theme: &str) -> (u32, u32) {
    if let Some(hex) = theme.strip_prefix('#') {
        if hex.len() == 6 {
            if let Ok(val) = u32::from_str_radix(hex, 16) {
                let (r, g, b) = (
                    ((val >> 16) & 0xff) as u8,
                    ((val >> 8) & 0xff) as u8,
                    (val & 0xff) as u8,
                );
                let (h, s, v) = rgb_to_hsv(r, g, b);
                let (sr, sg, sb) = hsv_to_rgb_u8(h, (s * 1.2).min(1.0), v * 0.62);
                let accent = 0xff00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32;
                let strong = 0xff00_0000 | ((sr as u32) << 16) | ((sg as u32) << 8) | sb as u32;
                return (accent, strong);
            }
        }
    }
    match theme {
        "emerald" => (0xff34d399, 0xff0f9d6e),
        "amber" => (0xfffbc02d, 0xffb07a1e),
        "rose" => (0xfffb7185, 0xffc23a5c),
        "violet" => (0xffa78bfa, 0xff6d4fd9),
        "mono" => (0xffcfd2dc, 0xff5a5f6e),
        _ => (0xff4fc3f7, 0xff2d6cdf), // sky（既定）
    }
}

/// 設定の表示系を UI へ反映（列フラグ・自分強調・最前面・パネルのトグル状態）。
fn apply_settings(m: &MainWindow, c: &settings::Settings) {
    let (accent, accent_strong) = accent_colors(&c.accent_theme);
    let theme = m.global::<Theme>();
    theme.set_accent(slint::Color::from_argb_encoded(accent));
    theme.set_accent_strong(slint::Color::from_argb_encoded(accent_strong));
    // アクセント色HSVピッカーの位置（accent 色を HSV へ変換して反映）。
    {
        let (ar, ag, ab) = (
            ((accent >> 16) & 0xff) as u8,
            ((accent >> 8) & 0xff) as u8,
            (accent & 0xff) as u8,
        );
        let (ah, asat, av) = rgb_to_hsv(ar, ag, ab);
        m.set_accent_h(ah);
        m.set_accent_s(asat);
        m.set_accent_v(av);
    }
    m.set_cols(ColumnFlags {
        crit: c.show_crit,
        crit_value: c.show_crit_value,
        lucky: c.show_lucky,
        lucky_value: c.show_lucky_value,
        hits: c.show_hits,
        hpm: c.show_hpm,
        score: c.show_score,
    });
    m.set_highlight_local(c.highlight_local_player);
    m.set_aot(c.always_on_top);
    m.set_win_opacity(c.opacity as f32);
    m.set_overlay_opacity(c.overlay_opacity as f32);
    m.set_font_scale((c.font_size / 12.0) as f32);
    // 本体フォント（バフ/デバフ オーバーレイもこのファミリ・太字・サイズに追随する）。
    m.set_main_font(c.main_font.clone().into());
    m.set_main_font_bold(c.main_font_bold);
    m.set_show_consumable(c.show_consumable);
    m.set_cfg_ui(SettingsUi {
        show_crit: c.show_crit,
        show_crit_value: c.show_crit_value,
        show_lucky: c.show_lucky,
        show_lucky_value: c.show_lucky_value,
        show_hits: c.show_hits,
        show_hpm: c.show_hpm,
        show_score: c.show_score,
        highlight_local: c.highlight_local_player,
        abbreviate_scores: c.abbreviate_scores,
        privacy_mask: c.privacy_mask_names,
        self_status: c.show_self_status_overlay,
        stats_overlay: c.show_stats_overlay,
        buff_overlay: c.show_buff_overlay,
        imagine_only: c.imagine_only_mode,
        aot: c.always_on_top,
        three_min_auto_open: c.three_min_auto_open,
        compact_split: c.compact_split_mode,
        graph_for_local: c.graph_for_local_player,
        startup_tab: c.startup_tab.clone().into(),
        language: c.language.clone().into(),
        accent_theme: c.accent_theme.clone().into(),
        sync_timer: c.sync_timer_with_main,
        sync_order_follow: c.sync_order_follow,
        show_imagine_tina: c.show_imagine_tina,
        show_imagine_aluna: c.show_imagine_aluna,
        show_imagine_tarta: c.show_imagine_tarta,
        show_imagine_basilisk: c.show_imagine_basilisk,
        show_consumable: c.show_consumable,
        show_in_taskbar: c.show_in_taskbar,
        overlay_text_color: c.overlay_text_color.clone().into(),
        main_font: c.main_font.clone().into(),
        main_font_bold: c.main_font_bold,
        stats_overlay_font: c.stats_overlay_font.clone().into(),
        stats_overlay_font_bold: c.stats_overlay_font_bold,
        imagine_overlay_font: c.imagine_overlay_font.clone().into(),
        imagine_overlay_font_bold: c.imagine_overlay_font_bold,
        imagine_compact_rows: c.imagine_compact_rows,
        overlay_outline: c.overlay_outline,
        overlay_shadow: c.overlay_shadow,
    });
    // 文字色HSVピッカーの初期/同期位置（現在の文字色を HSV へ変換して反映）。
    {
        let col = resolve_overlay_text_color(&c.overlay_text_color);
        let (th, ts, tv) = rgb_to_hsv(col.red(), col.green(), col.blue());
        m.set_overlay_text_h(th);
        m.set_overlay_text_s(ts);
        m.set_overlay_text_v(tv);
    }
    let int_str = |v: f64| -> slint::SharedString { format!("{}", v as i64).into() };
    m.set_nums(SettingsNumUi {
        combat_exit: int_str(c.combat_exit_sec),
        poll_interval: int_str(c.poll_interval_ms),
        history_limit: int_str(c.history_limit),
        ts_samples: int_str(c.time_series_samples),
        ts_interval: int_str(c.time_series_interval_ms),
        three_min_dur: int_str(c.three_min_duration_sec),
        graph_count: int_str(c.graph_player_count),
        font_size: int_str(c.font_size),
        stats_overlay_font_size: int_str(c.stats_overlay_font_size),
        imagine_overlay_font_size: int_str(c.imagine_overlay_font_size),
    });
}

/// オーバーレイ文字色文字列を実際の色へ解決する。
/// プリセットキー（white/warm/cool/green/amber）または "#rrggbb" 形式を受け付ける。
fn resolve_overlay_text_color(s: &str) -> slint::Color {
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            if let Ok(v) = u32::from_str_radix(hex, 16) {
                return slint::Color::from_rgb_u8(
                    ((v >> 16) & 0xff) as u8,
                    ((v >> 8) & 0xff) as u8,
                    (v & 0xff) as u8,
                );
            }
        }
    }
    let (r, g, b) = match s {
        "warm" => (0xff, 0xe9, 0xcf),
        "cool" => (0xcf, 0xe6, 0xff),
        "green" => (0xcd, 0xfb, 0xd8),
        "amber" => (0xff, 0xe7, 0xa8),
        _ => (0xff, 0xff, 0xff), // white（既定）
    };
    slint::Color::from_rgb_u8(r, g, b)
}

/// RGB(各 u8) → HSV（h/s/v とも 0..1）。h は色相を 0..1 に正規化（×360 で度）。
fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (rf, gf, bf) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let d = max - min;
    let v = max;
    let s = if max <= 0.0 { 0.0 } else { d / max };
    let h = if d <= 0.0 {
        0.0
    } else if (max - rf).abs() < f32::EPSILON {
        (((gf - bf) / d).rem_euclid(6.0)) / 6.0
    } else if (max - gf).abs() < f32::EPSILON {
        (((bf - rf) / d) + 2.0) / 6.0
    } else {
        (((rf - gf) / d) + 4.0) / 6.0
    };
    (h.rem_euclid(1.0), s, v)
}

/// HSV（各 0..1）→ RGB(各 u8)。
fn hsv_to_rgb_u8(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h6 = h.rem_euclid(1.0) * 6.0;
    let i = h6.floor() as i32;
    let f = h6 - i as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    let (rf, gf, bf) = match i.rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    let to_u8 = |x: f32| (x * 255.0).round().clamp(0.0, 255.0) as u8;
    (to_u8(rf), to_u8(gf), to_u8(bf))
}

/// HSV（各 0..1）→ "#rrggbb"。
fn hsv_to_hex(h: f32, s: f32, v: f32) -> String {
    let (r, g, b) = hsv_to_rgb_u8(h, s, v);
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// オーバーレイ3窓（バフ/デバフ・ステータス・イマジンタイマー）の外観を反映する。
/// 不透明度・基準テキスト色は3窓共通。文字サイズ・フォント・太字は窓ごとに独立：
/// バフ/デバフ=メイン窓設定(font_size/main_font/main_font_bold)に追随、
/// ステータス・イマジンは各専用設定を使う。
/// 表示/非表示に関わらずプロパティは窓側に保持されるため、起動時と設定変更時のみ呼べばよい。
fn apply_overlay_appearance(
    c: &settings::Settings,
    self_o: &slint::Weak<SelfStatusOverlay>,
    buff_o: &slint::Weak<BuffOverlay>,
    stats_o: &slint::Weak<StatsOverlay>,
) {
    let op = c.overlay_opacity as f32;
    let text_col = resolve_overlay_text_color(&c.overlay_text_color);
    // 完全透明（不透明度0）のときは当該オーバーレイをクリック透過（HUD化）にし、
    // ゲーム側へクリックを通す＝オーバーレイ自体は移動/最小化/×できなくなる。
    // 不透明度を0より上げると hittest が戻り操作可能に復帰する。
    // （トレイの「クリックスルー」一括切替とは別系統。不透明度変更時に本値で上書きする。）
    let pass_through = op <= 0.001;
    // バフ/デバフ窓: メイン窓のフォントサイズ・フォント・太字に追随。
    if let Some(o) = self_o.upgrade() {
        o.set_overlay_opacity(op);
        o.set_overlay_outline(c.overlay_outline);
        o.set_overlay_shadow(c.overlay_shadow);
        o.set_overlay_font(c.main_font.clone().into());
        o.set_overlay_font_bold(c.main_font_bold);
        o.set_text_base(text_col);
        o.set_font_scale((c.font_size / 12.0) as f32);
        overlay::set_click_through(o.window(), pass_through);
    }
    // イマジンタイマー窓: 専用フォント設定。
    if let Some(o) = buff_o.upgrade() {
        o.set_overlay_opacity(op);
        o.set_overlay_outline(c.overlay_outline);
        o.set_overlay_shadow(c.overlay_shadow);
        o.set_overlay_font(c.imagine_overlay_font.clone().into());
        o.set_overlay_font_bold(c.imagine_overlay_font_bold);
        o.set_text_base(text_col);
        o.set_font_scale((c.imagine_overlay_font_size / 12.0) as f32);
        overlay::set_click_through(o.window(), pass_through);
    }
    // ステータス窓: 専用フォント設定。
    if let Some(o) = stats_o.upgrade() {
        o.set_overlay_opacity(op);
        o.set_overlay_outline(c.overlay_outline);
        o.set_overlay_shadow(c.overlay_shadow);
        o.set_overlay_font(c.stats_overlay_font.clone().into());
        o.set_overlay_font_bold(c.stats_overlay_font_bold);
        o.set_text_base(text_col);
        o.set_font_scale((c.stats_overlay_font_size / 12.0) as f32);
        overlay::set_click_through(o.window(), pass_through);
    }
}

/// テンプレートのプレビュー（固定サンプル行で name/copy 両テンプレを展開）。
fn template_previews(c: &settings::Settings) -> (slint::SharedString, slint::SharedString) {
    // プレビューのサンプル職業/特化名も表示言語に揃える。
    let (sample_class, sample_spec) = if is_ja() {
        ("ストームブレイド", "雷刃型")
    } else {
        ("Stormblade", "Iaido")
    };
    let name = format::format_row_name(
        "Sample",
        sample_class,
        sample_spec,
        12345.0,
        38.0,
        8200.0,
        "-タータ/アルーナ",
        1,
        &c.name_template,
        c.abbreviate_scores,
    );
    let copy = format::format_row_template(
        &format::CopyRowData {
            rank: 1,
            name: "Sample",
            class_name: sample_class,
            class_spec_name: sample_spec,
            total_value: 1_234_567.0,
            value_per_sec: 45678.0,
            value_pct: 35.5,
            crit_rate: 42.3,
            crit_value_rate: 18.7,
            lucky_rate: 5.5,
            lucky_value_rate: 2.1,
            hits: 124.0,
            hits_per_minute: 78.5,
            ability_score: 12345.0,
            season_level: 38.0,
            season_strength: 8200.0,
        },
        &c.copy_template,
        c.abbreviate_scores,
    );
    (name.into(), copy.into())
}

/// テンプレ入力欄の value を push（パネル開時／リセット時のみ）＋プレビュー更新。
fn refresh_templates(m: &MainWindow, c: &settings::Settings) {
    m.set_name_template_value(c.name_template.clone().into());
    m.set_copy_template_value(c.copy_template.clone().into());
    let (np, cp) = template_previews(c);
    m.set_name_preview(np);
    m.set_copy_preview(cp);
}

/// SelfStatusEntry 群を UI 行へ変換（BuffIconCell 相当）。
fn build_status_entries(entries: &[bpsr_core::models::SelfStatusEntry]) -> Vec<StatusEntryUi> {
    entries
        .iter()
        .map(|e| {
            let is_debuff = e.category == "debuff";
            let is_low = e.duration_ms > 0 && e.remaining_ms < 3000;
            let ratio = if e.duration_ms == 0 {
                1.0
            } else {
                (e.remaining_ms as f32 / e.duration_ms as f32).clamp(0.0, 1.0)
            };
            let bar_color = if is_low {
                slint::Color::from_rgb_u8(0xff, 0x70, 0x43)
            } else if is_debuff {
                slint::Color::from_rgb_u8(0xef, 0x53, 0x50)
            } else {
                slint::Color::from_rgb_u8(0x4f, 0xc3, 0xf7)
            };
            // 枠色は優先度に依らず固定（バフ/デバフで色分けしない）。
            let border_color = slint::Color::from_argb_u8(0x33, 0xff, 0xff, 0xff);
            StatusEntryUi {
                name: buff_names::label(e.base_id).into(),
                remaining_text: format::format_remaining(e.remaining_ms, e.duration_ms).into(),
                bar_ratio: ratio,
                bar_color,
                layer_text: if e.layer > 1 {
                    format!("×{}", e.layer).into()
                } else {
                    "".into()
                },
                is_low,
                border_color,
            }
        })
        .collect()
}

/// ステータス窓の表示項目トグル一覧（カタログ × 現在の有効集合）。
/// グループが切り替わる先頭項目にのみ group-head（見出し文字列）を設定する。
fn build_stat_catalog(enabled: &[String]) -> Vec<StatCatalogItem> {
    let mut last_group = "";
    let mut out = Vec::with_capacity(settings::STAT_CATALOG.len());
    for d in settings::STAT_CATALOG {
        let group = d.group();
        let head = if group != last_group {
            last_group = group;
            group
        } else {
            ""
        };
        out.push(StatCatalogItem {
            key: d.key.into(),
            label: d.label().into(),
            group_head: head.into(),
            enabled: enabled.iter().any(|e| e == d.key),
        });
    }
    out
}

/// カタログを2列へ分割する。グループ境界（group_head が非空＝グループ先頭）を跨がず、
/// 左右の項目数がなるべく均等になる境界で割る。各列の先頭グループ見出しを保持するため。
fn split_stat_catalog(items: &[StatCatalogItem]) -> (Vec<StatCatalogItem>, Vec<StatCatalogItem>) {
    let half = items.len() / 2;
    // half 以上に達した最初のグループ先頭で割る（無ければ末尾＝右列空）。
    let split = items
        .iter()
        .enumerate()
        .skip(1)
        .find(|(i, it)| *i >= half && !it.group_head.is_empty())
        .map(|(i, _)| i)
        .unwrap_or(items.len());
    (items[..split].to_vec(), items[split..].to_vec())
}

/// 3桁区切りの整数文字列（例: 28500 → "28,500"）。
fn group_int(n: i64) -> String {
    let neg = n < 0;
    let digits = n.unsigned_abs().to_string();
    let len = digits.len();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if neg { format!("-{out}") } else { out }
}

/// SelfStatsData と有効項目（表示順）から UI 行を生成。値が無い項目は "—"。
/// ※ 割合系ステータス（値/100=%）と実測率（命中由来・既に%）を区別して整形する。
fn build_stat_entries(s: &bpsr_core::models::SelfStatsData, enabled: &[String]) -> Vec<StatEntryUi> {
    let dash = "—".to_string();
    let int_v = |o: Option<i32>| o.map(|v| group_int(v as i64)).unwrap_or_else(|| dash.clone());
    // 割合系の生値は「整数 ×100 の %」（例: 2485 = 24.85%）。/100 は厳密に 2 桁なので
    // 全精度（小数 2 桁）で表示する。format_pct（1 桁）は丸めで実在桁を捨てるため使わない。
    let pct_v = |o: Option<i32>| {
        o.map(|v| format!("{:.2}%", v as f64 / 100.0))
            .unwrap_or_else(|| dash.clone())
    };
    enabled
        .iter()
        .map(|key| {
            let (value, accent) = match key.as_str() {
                "hp" => {
                    // HP は全桁（3 桁区切り）で表示する。略記（K/M）は実数を丸めて精度を捨てるため使わない。
                    let v = match (s.curr_hp, s.max_hp) {
                        (Some(c), Some(m)) => {
                            format!("{} / {}", group_int(c as i64), group_int(m as i64))
                        }
                        (Some(c), None) => group_int(c as i64),
                        _ => dash.clone(),
                    };
                    (v, false)
                }
                // 整数系
                "atk-phys" => (int_v(s.attack_power), false),
                "atk-magic" => (int_v(s.magic_attack), false),
                "def-phys" => (int_v(s.defense_power), false),
                "def-magic" => (int_v(s.magic_defense), false),
                "endurance" => (int_v(s.endurance), false),
                "strength" => (int_v(s.strength), false),
                "intelligence" => (int_v(s.intelligence), false),
                "agility" => (int_v(s.agility), false),
                "ability-score" => (int_v(s.ability_score), false),
                "season-strength" => (int_v(s.season_strength), false),
                // 割合系（値/100=%）
                "haste" => (pct_v(s.haste), false),
                "attack-speed" => (pct_v(s.attack_speed), false),
                "cast-speed" => (pct_v(s.cast_speed), false),
                "lucky" => (pct_v(s.lucky), false),
                "crit" => (pct_v(s.crit_stat), false),
                "versatility" => (pct_v(s.versatility), false),
                "resist" => (pct_v(s.resist), false),
                "dexterity" => (pct_v(s.dexterity), false),
                "crit-dmg" => (pct_v(s.crit_dmg), false),
                "lucky-dmg" => (pct_v(s.lucky_dmg), false),
                // 実測率（命中データ由来・%・強調）
                "crit-rate" => (
                    if s.has_combat {
                        format::format_pct(s.crit_rate_measured)
                    } else {
                        dash.clone()
                    },
                    true,
                ),
                "lucky-rate" => (
                    if s.has_combat {
                        format::format_pct(s.lucky_rate_measured)
                    } else {
                        dash.clone()
                    },
                    true,
                ),
                _ => (dash.clone(), false),
            };
            let label = settings::STAT_CATALOG
                .iter()
                .find(|d| d.key == key)
                .map(|d| d.label())
                .unwrap_or(key.as_str());
            StatEntryUi {
                label: label.into(),
                value: value.into(),
                accent,
            }
        })
        .collect()
}

/// 円形タイマーの進捗アーク SVG（viewbox 28、中心14,14、半径12.5、上端から時計回り）。
fn buff_arc(ratio: f32) -> String {
    let p = ratio.clamp(0.0, 0.9999);
    let theta = p * std::f32::consts::TAU;
    let (cx, cy, r) = (14.0_f32, 14.0_f32, 12.5_f32);
    let end_x = cx + r * theta.sin();
    let end_y = cy - r * theta.cos();
    let large = if p > 0.5 { 1 } else { 0 };
    format!("M {cx} {} A {r} {r} 0 {large} 1 {end_x:.2} {end_y:.2}", cy - r)
}

fn buff_cell(snap: Option<&bpsr_core::models::SelfBuffSnapshot>, kind_hex: u32) -> BuffCell {
    let color =
        slint::Color::from_rgb_u8((kind_hex >> 16) as u8, (kind_hex >> 8) as u8, kind_hex as u8);
    match snap {
        Some(b) => {
            let active = b.remaining_ms > 0 || b.duration_ms <= 0;
            let ratio = if b.duration_ms <= 0 {
                0.0
            } else {
                (b.remaining_ms as f32 / b.duration_ms as f32).clamp(0.0, 1.0)
            };
            let text = if b.duration_ms <= 0 {
                "∞".to_string()
            } else if b.remaining_ms <= 0 {
                "OK".to_string()
            } else {
                let s = (b.remaining_ms as f64 / 1000.0).ceil() as i64;
                if s > 999 {
                    "999+".to_string()
                } else {
                    s.to_string()
                }
            };
            let text_color = if !active || b.remaining_ms <= 0 {
                slint::Color::from_rgb_u8(0x88, 0x88, 0x88)
            } else if b.duration_ms > 0 && b.remaining_ms < 3000 {
                slint::Color::from_rgb_u8(0xff, 0x52, 0x52)
            } else {
                slint::Color::from_rgb_u8(0xdd, 0xdd, 0xdd)
            };
            BuffCell {
                active,
                arc_commands: buff_arc(ratio).into(),
                color,
                text: text.into(),
                text_color,
            }
        }
        None => BuffCell {
            active: false,
            arc_commands: "".into(),
            color,
            text: "".into(),
            text_color: slint::Color::from_rgb_u8(0x88, 0x88, 0x88),
        },
    }
}

/// `uids` を自分(local_uid)を先頭固定＋以降 uid 昇順（チラつかせない安定順）に並べ替える。
/// 同期OFF時の並びの土台（項目3「並び順の追従」OFF時）・専用モードの名簿順の土台に使う。
fn order_local_first_stable(uids: &[i64], local_uid: i64) -> Vec<i64> {
    let mut rest: Vec<i64> = uids.iter().copied().filter(|&u| u != local_uid).collect();
    rest.sort_unstable();
    if local_uid != 0 && uids.contains(&local_uid) {
        let mut out = Vec::with_capacity(uids.len());
        out.push(local_uid);
        out.extend(rest);
        out
    } else {
        rest
    }
}

/// イマジンタイマーの行順をメインDPS画面の並び(`main_ordered`)へ追従させる。
/// `roster` のうち main_ordered に在る uid をその順で先頭に、main に居ない roster
/// (戦闘から外れた等)は従来順で末尾へ。main_ordered が空なら roster を素通し。
fn order_by_main(roster: &[i64], main_ordered: &[i64]) -> Vec<i64> {
    if main_ordered.is_empty() {
        return roster.to_vec();
    }
    let roster_set: std::collections::HashSet<i64> = roster.iter().copied().collect();
    let in_main: std::collections::HashSet<i64> = main_ordered.iter().copied().collect();
    let mut out: Vec<i64> = main_ordered
        .iter()
        .copied()
        .filter(|u| roster_set.contains(u))
        .collect();
    // main に居ない roster は従来順で末尾に温存（漏れ防止）。
    out.extend(roster.iter().copied().filter(|u| !in_main.contains(u)));
    out
}

/// イマジンタイマーに実際に表示するプレイヤーの uid 列（＝メイン一覧のピン点灯集合と同一）。
///
/// 名簿源は3分岐:
/// - 専用モードON: `buff_tracked_uids`（バフ追跡から自動・first-seen順）から excluded を除き、
///   自分を先頭固定＋以降 first-seen 順（=元の並びをそのまま使う。安定ソート不要）で上限内に。
/// - 専用OFF・同期ON: メイン名簿順(`main_ordered`)から excluded を除いたもの。
///   並びは `order_follow` 次第（ON=メイン順そのまま／OFF=自分先頭＋uid昇順の安定順）。
/// - 専用OFF・同期OFF: 手動ウォッチ(`wl.watched`)のみ。メイン順があれば追従させる。
#[allow(clippy::too_many_arguments)]
fn timer_roster(
    wl: &watchlist::Watchlist,
    imagine_only: bool,
    sync: bool,
    order_follow: bool,
    main_ordered: &[i64],
    buff_tracked_uids: &[i64],
    local_uid: i64,
) -> Vec<i64> {
    if imagine_only {
        let filtered: Vec<i64> = buff_tracked_uids
            .iter()
            .copied()
            .filter(|u| !wl.excluded.contains(u))
            .collect();
        return order_local_first_stable(&filtered, local_uid)
            .into_iter()
            .take(watchlist::MAX)
            .collect();
    }
    if sync {
        let filtered: Vec<i64> = main_ordered
            .iter()
            .copied()
            .filter(|u| !wl.excluded.contains(u))
            .collect();
        let ordered = if order_follow {
            filtered
        } else {
            order_local_first_stable(&filtered, local_uid)
        };
        return ordered.into_iter().take(watchlist::MAX).collect();
    }
    order_by_main(&wl.watched, main_ordered)
}

/// `roster` の表示順で行を組む。`privacy_mask`=true のとき名前は `format::mask_player_name` で
/// マスクする（メイン行 `build_rows` と同じ規約）。見つからない uid は uid 下16bit の数値表示。
fn build_buff_rows(
    tracked: &bpsr_core::models::TrackedBuffsData,
    roster: &[i64],
    privacy_mask: bool,
) -> Vec<BuffPlayerRow> {
    roster
        .iter()
        .map(|&uid| {
            let snap = tracked.players.iter().find(|p| p.uid as i64 == uid);
            let display = if privacy_mask {
                format::mask_player_name(uid)
            } else {
                let name = snap.map(|s| s.name.clone()).unwrap_or_default();
                if name.is_empty() {
                    format!("{}", uid & 0xffff)
                } else {
                    name
                }
            };
            let find = |kind: &str| snap.and_then(|s| s.buffs.iter().find(|b| b.kind == kind));
            BuffPlayerRow {
                name: display.into(),
                tina: buff_cell(find("Tina"), 0xff4d6d),
                aluna: buff_cell(find("Aluna"), 0x5fd35f),
                tarta: buff_cell(find("Tarta"), 0xb98bff),
                basilisk: buff_cell(find("Basilisk"), 0xd9a05b),
            }
        })
        .collect()
}

/// ポーリングループが tick 間で持ち越す可変状態（位置復元の進行管理）。
#[derive(Default)]
struct PollState {
    tick: u64,
    setup_tick: u64,
    setup_done: bool,
    // オーバーレイ復元tick（None=未復元）。復元前のデフォルト位置で保存上書きしないよう、
    // 復元から SETTLE_TICKS 経過後に保存対象へ含める。非表示で None に戻す。
    self_rtick: Option<u64>,
    buff_rtick: Option<u64>,
    stats_rtick: Option<u64>,
    // 復元が実際に適用した（クランプ後の）矩形。settle 期間中はこのサイズを毎tick 再適用。
    restored_main: Option<window_state::WinRect>,
    restored_self: Option<window_state::WinRect>,
    restored_buffs: Option<window_state::WinRect>,
    restored_stats: Option<window_state::WinRect>,
    // オーバーレイへ最後に push した内容（前回と同一なら set_vec を省いて無駄な再描画を避ける）。
    // オーバーレイは sync_rows と違い set_vec で毎tick モデル全置換していたため、内容不変でも
    // 5Hz で再描画され CPU を浪費していた（実測: 2窓表示で約11%/1コア）。
    last_self_buffs: Vec<StatusEntryUi>,
    last_self_debuffs: Vec<StatusEntryUi>,
    last_stats_rows: Vec<StatEntryUi>,
    last_buff_players: Vec<BuffPlayerRow>,
}

/// 内容が前回 push と一致する間は `set_vec` を呼ばない（VecModel 全置換による
/// repeater 作り直し＝再描画を避ける）。変化時のみ更新し、キャッシュも差し替える。
fn set_vec_if_changed<T>(model: &slint::VecModel<T>, last: &mut Vec<T>, next: Vec<T>)
where
    T: Clone + PartialEq + 'static,
{
    if *last != next {
        model.set_vec(next.clone());
        *last = next;
    }
}

/// 初回 tick: winit 実体化後にメイン窓を復元する。復元が完了した tick で true を返す
/// （呼び出し側はその tick でトレイ生成などの後処理を行う）。
fn poll_setup_once(m: &MainWindow, st: &mut PollState, saved: &window_state::Layout) -> bool {
    if st.setup_done {
        return false;
    }
    let mons = overlay::monitors(m.window());
    if mons.is_empty() {
        return false;
    }
    st.restored_main =
        Some(window_state::restore(m.window(), saved.main.as_ref(), &mons, 0, (520, 350)));
    st.setup_done = true;
    st.setup_tick = st.tick;
    log::info!("window restored on {} monitor(s)", mons.len());
    true
}

/// オーバーレイの位置/サイズ復元（表示された最初の tick で一度）。非表示で None に戻す。
fn poll_overlay_restore(
    st: &mut PollState,
    cfg: &RefCell<settings::Settings>,
    self_overlay_w: &slint::Weak<SelfStatusOverlay>,
    buff_overlay_w: &slint::Weak<BuffOverlay>,
    stats_overlay_w: &slint::Weak<StatsOverlay>,
    last_saved: &RefCell<window_state::Layout>,
) {
    let c = cfg.borrow();
    // 完全透明(不透明度0)のオーバーレイはクリック透過(HUD)にする。apply_overlay_appearance は
    // 起動時/表示トグル時に窓の winit 実体化より前へ走り set_cursor_hittest が空振りするため、
    // 実体化を確認できるこの復元 tick で改めて適用する（EXSTYLE 競合回避のため taskbar_mode の前）。
    let pass_through = c.overlay_opacity <= 0.001;
    if c.show_stats_overlay {
        if st.stats_rtick.is_none() {
            if let Some(o) = stats_overlay_w.upgrade() {
                let mons = overlay::monitors(o.window());
                if !mons.is_empty() {
                    st.restored_stats = Some(window_state::restore(
                        o.window(),
                        last_saved.borrow().stats.as_ref(),
                        &mons,
                        0,
                        (200, 220),
                    ));
                    st.stats_rtick = Some(st.tick);
                    overlay::set_click_through(o.window(), pass_through);
                    #[cfg(windows)]
                    overlay::apply_taskbar_mode(o.window(), c.show_in_taskbar);
                }
            }
        }
    } else {
        st.stats_rtick = None;
        st.restored_stats = None;
    }
    if c.show_self_status_overlay {
        if st.self_rtick.is_none() {
            if let Some(o) = self_overlay_w.upgrade() {
                let mons = overlay::monitors(o.window());
                if !mons.is_empty() {
                    st.restored_self = Some(window_state::restore(
                        o.window(),
                        last_saved.borrow().self_status.as_ref(),
                        &mons,
                        0,
                        (220, 180),
                    ));
                    st.self_rtick = Some(st.tick);
                    overlay::set_click_through(o.window(), pass_through);
                    // 実体化したオーバーレイへ現在のタスクバー常駐モードを適用。
                    #[cfg(windows)]
                    overlay::apply_taskbar_mode(o.window(), c.show_in_taskbar);
                }
            }
        }
    } else {
        st.self_rtick = None;
        st.restored_self = None;
    }
    if c.show_buff_overlay {
        if st.buff_rtick.is_none() {
            if let Some(o) = buff_overlay_w.upgrade() {
                let mons = overlay::monitors(o.window());
                if !mons.is_empty() {
                    st.restored_buffs = Some(window_state::restore(
                        o.window(),
                        last_saved.borrow().buffs.as_ref(),
                        &mons,
                        0,
                        (250, 150),
                    ));
                    st.buff_rtick = Some(st.tick);
                    overlay::set_click_through(o.window(), pass_through);
                    // 実体化したオーバーレイへ現在のタスクバー常駐モードを適用。
                    #[cfg(windows)]
                    overlay::apply_taskbar_mode(o.window(), c.show_in_taskbar);
                }
            }
        }
    } else {
        st.buff_rtick = None;
        st.restored_buffs = None;
    }
}

/// トレイメニューのイベント処理（クリックスルー切替・表示/非表示・終了）。
#[cfg(windows)]
fn poll_tray_events(
    m: &MainWindow,
    cfg: &RefCell<settings::Settings>,
    self_overlay_w: &slint::Weak<SelfStatusOverlay>,
    buff_overlay_w: &slint::Weak<BuffOverlay>,
    stats_overlay_w: &slint::Weak<StatsOverlay>,
    tray_holder: &RefCell<Option<tray::Tray>>,
    main_visible: &Cell<bool>,
    click_through: &Cell<bool>,
) {
    let holder = tray_holder.borrow();
    let Some(tray) = holder.as_ref() else {
        return;
    };
    while let Ok(ev) = tray_icon::menu::MenuEvent::receiver().try_recv() {
        if ev.id == tray.id_quit {
            let _ = slint::quit_event_loop();
        } else if ev.id == tray.id_show_hide {
            let vis = !main_visible.get();
            main_visible.set(vis);
            let _ = if vis { m.show() } else { m.hide() };
            // オーバーレイもメインの表示/格納に追従（復帰時は設定で有効なものだけ）
            let c = cfg.borrow();
            if let Some(o) = self_overlay_w.upgrade() {
                let _ = if vis && c.show_self_status_overlay {
                    o.show()
                } else {
                    o.hide()
                };
            }
            if let Some(o) = buff_overlay_w.upgrade() {
                let _ = if vis && c.show_buff_overlay {
                    o.show()
                } else {
                    o.hide()
                };
            }
            if let Some(o) = stats_overlay_w.upgrade() {
                let _ = if vis && c.show_stats_overlay {
                    o.show()
                } else {
                    o.hide()
                };
            }
        } else if ev.id == tray.id_click_through {
            let on = !click_through.get();
            click_through.set(on);
            tray.click_through.set_checked(on);
            overlay::set_click_through(m.window(), on);
            if let Some(o) = self_overlay_w.upgrade() {
                overlay::set_click_through(o.window(), on);
            }
            if let Some(o) = buff_overlay_w.upgrade() {
                overlay::set_click_through(o.window(), on);
            }
            if let Some(o) = stats_overlay_w.upgrade() {
                overlay::set_click_through(o.window(), on);
            }
        }
    }
    // トレイアイコン左クリックでメインを復帰（トレイ格納モードの復帰口）。
    while let Ok(ev) = tray_icon::TrayIconEvent::receiver().try_recv() {
        if let tray_icon::TrayIconEvent::Click {
            button: tray_icon::MouseButton::Left,
            button_state: tray_icon::MouseButtonState::Up,
            ..
        } = ev
        {
            main_visible.set(true);
            let _ = m.show();
            overlay::restore_window(m.window());
            // 設定で有効なオーバーレイも一緒に復帰させる
            let c = cfg.borrow();
            if c.show_self_status_overlay {
                if let Some(o) = self_overlay_w.upgrade() {
                    let _ = o.show();
                }
            }
            if c.show_buff_overlay {
                if let Some(o) = buff_overlay_w.upgrade() {
                    let _ = o.show();
                }
            }
            if c.show_stats_overlay {
                if let Some(o) = stats_overlay_w.upgrade() {
                    let _ = o.show();
                }
            }
        }
    }
}

/// 起動/表示直後に Slint が preferred サイズを再アサートして保存サイズを上書きする
/// ことがあるため、settle 期間中は毎tick 復元サイズを再適用する（サイズ一致なら no-op）。
fn poll_window_settle(
    m: &MainWindow,
    st: &PollState,
    self_overlay_w: &slint::Weak<SelfStatusOverlay>,
    buff_overlay_w: &slint::Weak<BuffOverlay>,
    stats_overlay_w: &slint::Weak<StatsOverlay>,
) {
    if st.setup_done && st.tick < st.setup_tick + SETTLE_TICKS {
        if let Some(r) = &st.restored_main {
            window_state::enforce_size(m.window(), r);
        }
    }
    if let (Some(rt), Some(r)) = (st.stats_rtick, st.restored_stats.as_ref()) {
        if st.tick < rt + SETTLE_TICKS {
            if let Some(o) = stats_overlay_w.upgrade() {
                window_state::enforce_size(o.window(), r);
            }
        }
    }
    if let (Some(rt), Some(r)) = (st.self_rtick, st.restored_self.as_ref()) {
        if st.tick < rt + SETTLE_TICKS {
            if let Some(o) = self_overlay_w.upgrade() {
                window_state::enforce_size(o.window(), r);
            }
        }
    }
    if let (Some(rt), Some(r)) = (st.buff_rtick, st.restored_buffs.as_ref()) {
        if st.tick < rt + SETTLE_TICKS {
            if let Some(o) = buff_overlay_w.upgrade() {
                window_state::enforce_size(o.window(), r);
            }
        }
    }
}

/// レイアウト自動保存（復元確定後・差分時のみ）。オーバーレイは復元から SETTLE_TICKS
/// 経過後のみ保存対象に含める（復元前の既定位置で上書き防止）。
fn poll_auto_save(
    m: &MainWindow,
    st: &PollState,
    cfg: &RefCell<settings::Settings>,
    self_overlay_w: &slint::Weak<SelfStatusOverlay>,
    buff_overlay_w: &slint::Weak<BuffOverlay>,
    stats_overlay_w: &slint::Weak<StatsOverlay>,
    last_saved: &RefCell<window_state::Layout>,
) {
    if !st.setup_done || st.tick < st.setup_tick + SETTLE_TICKS {
        return;
    }
    let tick = st.tick;
    let settled = |rt: Option<u64>| rt.map(|t| tick >= t + SETTLE_TICKS).unwrap_or(false);
    let cur = {
        let c = cfg.borrow();
        let prev = last_saved.borrow();
        window_state::Layout {
            main: Some(window_state::capture(m.window())),
            self_status: if c.show_self_status_overlay && settled(st.self_rtick) {
                self_overlay_w
                    .upgrade()
                    .map(|o| window_state::capture(o.window()))
            } else {
                prev.self_status.clone()
            },
            buffs: if c.show_buff_overlay && settled(st.buff_rtick) {
                buff_overlay_w
                    .upgrade()
                    .map(|o| window_state::capture(o.window()))
            } else {
                prev.buffs.clone()
            },
            stats: if c.show_stats_overlay && settled(st.stats_rtick) {
                stats_overlay_w
                    .upgrade()
                    .map(|o| window_state::capture(o.window()))
            } else {
                prev.stats.clone()
            },
        }
    };
    let mut ls = last_saved.borrow_mut();
    if *ls != cur {
        window_state::save(&cur);
        *ls = cur;
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    static LOGGER: ConsoleLog = ConsoleLog;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
    log::info!("log file: {}", log_file_path().display());

    if already_running() {
        log::warn!("another instance is already running; exiting");
        return Ok(());
    }

    // 全ウィンドウ共通のタスクバー常駐フラグ。生成時 hook が参照するため設定を先読みする
    // （cfg より前。以降の切替は on_set_bool でこの Cell とウィンドウへ反映）。
    let taskbar_flag = Rc::new(Cell::new(settings::load().show_in_taskbar));

    // winit backend（透明合成可能。skip_taskbar は設定でメイン/オーバーレイ一括切替）
    let backend = {
        let tf = taskbar_flag.clone();
        i_slint_backend_winit::Backend::builder()
            .with_window_attributes_hook(move |attrs| {
                let attrs = attrs.with_transparent(true);
                #[cfg(target_os = "windows")]
                let attrs = {
                    use i_slint_backend_winit::winit::platform::windows::WindowAttributesExtWindows;
                    attrs.with_skip_taskbar(!tf.get())
                };
                attrs
            })
            .build()?
    };
    slint::platform::set_platform(Box::new(backend)).map_err(|e| format!("set_platform: {e:?}"))?;

    // ローカルデバッグ専用: Slint 埋め込み MCP サーバーを起動する（feature "mcp" 時のみ）。
    // 通常はこの init は backend-selector が呼ぶが、本アプリは set_platform で winit を直接
    // 注入し selector をバイパスしているため、ここで明示的に呼ぶ必要がある。
    // 実際に待受けるのは起動時 env `SLINT_MCP_PORT` 設定時のみ（未設定なら即 return）。
    #[cfg(feature = "mcp")]
    if let Err(e) = i_slint_backend_testing::mcp_server::init() {
        log::warn!("MCP サーバー初期化に失敗: {e:?}");
    }

    // 永続キャッシュ初期化
    let dir = data_dir();
    engine::name_cache::init(dir.join("name_cache.json"));
    engine::selected_uid::init(dir.join("selected_uid.json"));
    engine::consumables::init(dir.join("consumables.json"));

    // 共有エンカウンター＋パケット観測スレッド
    // BPSR_DEMO=1 のときは観測の代わりに合成データを流す（撮影・UI確認用）
    let demo_mode = std::env::var("BPSR_DEMO").is_ok_and(|v| v == "1");
    let enc = Arc::new(EncounterMutex::default());
    if let Some(uid) = engine::selected_uid::get() {
        if let Ok(mut e) = enc.lock() {
            e.local_player_uid = uid;
        }
    }
    // 前回終了時の食事/シロップ残時間を復元（失効分は load 側で除去）。
    compute::load_consumables(&enc);
    if demo_mode {
        bpsr_core::engine::demo::spawn(enc.clone());
    } else {
        capture::spawn(enc.clone());
    }

    // 設定（%APPDATA%\bpsr-checker\settings.json）。UIスレッドで共有・編集する。
    let cfg = Rc::new(RefCell::new(settings::load()));
    // 中文UIは日本語フォント(Yu Gothic UI)に簡体字グリフが無く豆腐(□)になるため、
    // 既定フォントのままなら簡体字対応フォントへ差し替える（ユーザーが明示変更していれば尊重）。
    {
        let mut c = cfg.borrow_mut();
        if c.language == "zh" && c.main_font == settings::Settings::default().main_font {
            c.main_font = "Microsoft YaHei".to_string();
        }
    }
    // ウォッチリスト（バフタイマー追跡対象）。
    let wl = Rc::new(RefCell::new(watchlist::Watchlist::load()));

    // メイン窓
    let main = MainWindow::new()?;
    // 言語選択（設定 language。既定は日本語）。最初のコンポーネント生成後に呼ぶ。
    match slint::select_bundled_translation(&cfg.borrow().language) {
        Ok(()) => log::info!("translation: selected '{}'", cfg.borrow().language),
        Err(e) => log::warn!("translation: select '{}' failed: {e}", cfg.borrow().language),
    }
    // 名前辞書（スキル/モンスター/バフ）の表示言語も起動時に揃える（UI @tr と同じく再起動反映）。
    engine::runtime_settings::set_display_lang(engine::runtime_settings::Lang::from_code(
        &cfg.borrow().language,
    ));
    // バージョン表示（設定パネル最下部＋結果モーダルの透かし。画像コピーに写り込むクレジット）
    main.set_app_version(format!("bpsr-checker v{}", env!("CARGO_PKG_VERSION")).into());
    let rows = Rc::new(VecModel::<Row>::default());
    main.set_rows(rows.clone().into());
    // 軽量分割表示の左右カラム（rows を前半/後半に分配）
    let compact_left = Rc::new(VecModel::<Row>::default());
    let compact_right = Rc::new(VecModel::<Row>::default());
    main.set_compact_left(compact_left.clone().into());
    main.set_compact_right(compact_right.clone().into());

    // 現在タブ（UIスレッド共有）。タブクリックと周期ポーリングの両方が参照する。
    let tab_cell = Rc::new(Cell::new(0i32));

    // スキル内訳ビュー用モデル＋対象プレイヤー uid（0=なし）
    let skill_rows = Rc::new(VecModel::<SkillRowUi>::default());
    main.set_skill_rows(skill_rows.clone().into());
    let drill = Rc::new(Cell::new(Drill::None));

    // 履歴ビュー用モデル＋展開中エンカウンタ id（None=折りたたみ）
    let history_rows = Rc::new(VecModel::<HistoryRowUi>::default());
    main.set_history_rows(history_rows.clone().into());
    let history_expanded = Rc::new(Cell::new(None::<i64>));

    // 3分計測 結果パネル用モデル＋最後の結果スナップショット（コピー/再計測で参照）
    let result_rows = Rc::new(VecModel::<ResultRowUi>::default());
    main.set_result_rows(result_rows.clone().into());
    let result_skill_rows = Rc::new(VecModel::<ResultSkillRowUi>::default());
    main.set_result_skill_rows(result_skill_rows.clone().into());
    let result_pie = Rc::new(VecModel::<PieSlice>::default());
    main.set_result_pie(result_pie.clone().into());
    let result_legend = Rc::new(VecModel::<SkillLegendUi>::default());
    main.set_result_legend(result_legend.clone().into());
    let last_result = Rc::new(RefCell::new(None::<bpsr_core::models::EncounterSnapshot>));
    // finalize 前に捕捉した各プレイヤーのスキル内訳（uid → スキル行[降順・時系列付き]）
    let captured_skills = Rc::new(RefCell::new(std::collections::HashMap::<
        i64,
        Vec<bpsr_core::models::SkillRow>,
    >::new()));
    // 結果パネルの選択状態（エリア1=キャラ / エリア2=スキル）
    let selected_result_player = Rc::new(std::cell::Cell::<i64>::new(0));
    let selected_result_skill = Rc::new(std::cell::Cell::<i64>::new(0));
    // 折れ線プロットの実寸(px)。SparkGraph.resized で更新し、再生成時の座標スケールに使う。
    // 初期値は初回レイアウト前の暫定（resized 発火で実値に置換される）。
    let char_plot_dims = Rc::new(Cell::new((600.0_f32, 40.0_f32)));
    let skill_plot_dims = Rc::new(Cell::new((600.0_f32, 40.0_f32)));

    // 自キャラUID 候補モデル
    let uid_candidates = Rc::new(VecModel::<UidCandidate>::default());
    main.set_uid_candidates(uid_candidates.clone().into());

    // ステータス窓の表示項目トグル一覧（設定の有効集合から生成・トグルで再生成）。
    // 設定パネルでは2列表示するため、グループ境界で左右へ分割した2モデルを持つ。
    let stat_catalog_left = Rc::new(VecModel::<StatCatalogItem>::default());
    let stat_catalog_right = Rc::new(VecModel::<StatCatalogItem>::default());
    {
        let (l, r) = split_stat_catalog(&build_stat_catalog(&cfg.borrow().stats_enabled));
        stat_catalog_left.set_vec(l);
        stat_catalog_right.set_vec(r);
    }
    main.set_stat_catalog_left(stat_catalog_left.clone().into());
    main.set_stat_catalog_right(stat_catalog_right.clone().into());

    // 自キャラ ステータス オーバーレイ（別ウィンドウ）
    let stats_overlay = StatsOverlay::new()?;
    let stats_rows = Rc::new(VecModel::<StatEntryUi>::default());
    stats_overlay.set_stats(stats_rows.clone().into());
    wire_overlay_chrome!(stats_overlay, main, "stats-overlay");
    stats_overlay.set_show_minimize(cfg.borrow().show_in_taskbar);

    // 自キャラ バフ/デバフ オーバーレイ（別ウィンドウ）
    let self_overlay = SelfStatusOverlay::new()?;
    let self_buffs = Rc::new(VecModel::<StatusEntryUi>::default());
    let self_debuffs = Rc::new(VecModel::<StatusEntryUi>::default());
    self_overlay.set_buffs(self_buffs.clone().into());
    self_overlay.set_debuffs(self_debuffs.clone().into());
    wire_overlay_chrome!(self_overlay, main, "self-status-overlay");
    self_overlay.set_show_minimize(cfg.borrow().show_in_taskbar);

    // バフタイマー オーバーレイ（別ウィンドウ）
    let buff_overlay = BuffOverlay::new()?;
    let buff_players = Rc::new(VecModel::<BuffPlayerRow>::default());
    buff_overlay.set_players(buff_players.clone().into());
    wire_overlay_chrome!(buff_overlay, main, "buff-overlay");
    buff_overlay.set_show_minimize(cfg.borrow().show_in_taskbar);

    // オーバーレイ共通の外観（不透明度・フォント・基準色・サイズ）を起動時に反映。
    apply_overlay_appearance(
        &cfg.borrow(),
        &self_overlay.as_weak(),
        &buff_overlay.as_weak(),
        &stats_overlay.as_weak(),
    );

    // 設定を起動時に適用（列フラグ・自分強調・最前面・起動タブ・runtime settings）
    {
        let c = cfg.borrow();
        apply_settings(&main, &c);
        let init_tab = match c.startup_tab.as_str() {
            "heal" => 1,
            "taken" => 2,
            "history" => 3,
            _ => 0,
        };
        main.set_tab(init_tab);
        tab_cell.set(init_tab);
        compute::set_combat_exit_timeout(c.combat_exit_sec);
        compute::set_history_limit(c.history_limit);
        compute::set_time_series_config(c.time_series_samples, c.time_series_interval_ms);
        compute::set_imagine_only_mode(&enc, c.imagine_only_mode);
    }
    // 戦闘履歴の永続化（%APPDATA%\bpsr-checker\history.json）。
    // set_history_limit 適用後に呼ぶことで、起動時 load が設定済みの上限件数で正しく
    // 切り詰められる。デモモードでは合成データで実ファイルを汚染しないため init 自体を
    // 呼ばない（consumables/name_cache 同様、init 未呼び出しなら load/save は no-op）。
    if !demo_mode {
        engine::history::init(dir.join("history.json"));
    }

    // デモモードの撮影補助: 設定パネルを開いた状態にする / 3分計測を自動開始する
    if demo_mode {
        if std::env::var("BPSR_DEMO_OPEN_SETTINGS").is_ok_and(|v| v == "1") {
            main.set_settings_open(true);
        }
        let demo_3min = std::env::var("BPSR_DEMO_3MIN")
            .ok()
            .and_then(|s| s.parse::<f64>().ok());
        if let Some(secs) = demo_3min {
            if secs > 0.0 {
                compute::start_3min_measure_mode(&enc, secs);
            }
        }
    }

    main.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    // タスクトレイ表示/非表示と共有する可視状態。最小化ボタンもこの経路でトレイへ格納する
    // （skip_taskbar 窓は OS 最小化だとタスクバーにもトレイにも復帰口が無いため hide で代替）。
    let main_visible = Rc::new(Cell::new(true));
    {
        let w = main.as_weak();
        let mv = main_visible.clone();
        let self_ov = self_overlay.as_weak();
        let buff_ov = buff_overlay.as_weak();
        let cfg_min = cfg.clone();
        main.on_minimize(move || {
            let taskbar = cfg_min.borrow().show_in_taskbar;
            if let Some(m) = w.upgrade() {
                if taskbar {
                    // タスクバー常駐: OS最小化（タスクバーボタンから復帰）。
                    // 復帰がトレイ経路を通らないため可視状態は維持し、
                    // オーバーレイ(HUD)も退避せずそのまま残す。
                    overlay::minimize_window(m.window());
                    return;
                }
                mv.set(false);
                let _ = m.hide();
            }
            // トレイ格納: メイン最小化にオーバーレイも追従して退避（設定フラグは変更しない）
            if let Some(o) = self_ov.upgrade() {
                let _ = o.hide();
            }
            if let Some(o) = buff_ov.upgrade() {
                let _ = o.hide();
            }
        });
    }
    {
        let w = main.as_weak();
        main.on_start_drag(move || {
            if let Some(m) = w.upgrade() {
                overlay::start_drag(m.window());
            }
        });
    }
    {
        let w = main.as_weak();
        main.on_start_resize(move |dir| {
            if let Some(m) = w.upgrade() {
                overlay::start_resize(m.window(), dir);
            }
        });
    }
    // タブ選択: 共有セルを更新し、即時に再取得して反映（ポーリング待ちにしない）。
    // タブ切替時はドリルダウン/内訳ビューを解除して一覧へ戻す。
    {
        let w = main.as_weak();
        let enc_sel = enc.clone();
        let rows_sel = rows.clone();
        let tab_sel = tab_cell.clone();
        let cfg_sel = cfg.clone();
        let wl_sel = wl.clone();
        let drill_sel = drill.clone();
        let hist_rows_sel = history_rows.clone();
        let hist_exp_sel = history_expanded.clone();
        let cl_sel = compact_left.clone();
        let cr_sel = compact_right.clone();
        main.on_select_tab(move |n| {
            tab_sel.set(n);
            drill_sel.set(Drill::None);
            if let Some(m) = w.upgrade() {
                m.set_tab(n);
                m.set_view(0);
                if n == 3 {
                    let hist = compute::get_history();
                    hist_rows_sel.set_vec(build_history_rows(
                        &hist,
                        hist_exp_sel.get(),
                        cfg_sel.borrow().privacy_mask_names,
                    ));
                } else {
                    let c = cfg_sel.borrow();
                    m.set_show_graph_col(graph_col_active(&c, n));
                    let pw = fetch_players(&enc_sel, n);
                    let main_uids: Vec<i64> =
                        pw.player_rows.iter().map(|p| p.uid as i64).collect();
                    // 専用モード時はメイン一覧自体が空のため、ここでは常に非専用扱いでよい
                    // （main_uids が空なら結果も空集合になるだけ）。
                    let pin_uids = timer_roster(
                        &wl_sel.borrow(),
                        false,
                        c.sync_timer_with_main,
                        c.sync_order_follow,
                        &main_uids,
                        &[],
                        pw.local_player_uid as i64,
                    );
                    apply_player_rows(
                        &rows_sel,
                        &cl_sel,
                        &cr_sel,
                        build_rows(
                            &pw,
                            &c.name_template,
                            c.abbreviate_scores,
                            c.privacy_mask_names,
                            &pin_uids,
                            c.graph_player_count as i32,
                            c.graph_for_local_player,
                        ),
                    );
                }
            }
        });
    }
    // 行クリック → ドリルダウン（dps/heal: 技別 / 被ダメ: 攻撃元一覧）
    {
        let w = main.as_weak();
        let enc_sk = enc.clone();
        let sk_rows = skill_rows.clone();
        let drill_h = drill.clone();
        let tab_h = tab_cell.clone();
        main.on_open_drill(move |uid_str| {
            let uid: i64 = uid_str.as_str().parse().unwrap_or(0);
            if uid == 0 {
                return;
            }
            let Some(m) = w.upgrade() else {
                return;
            };
            if tab_h.get() == 2 {
                match compute::get_dmg_taken_attackers(&enc_sk, uid) {
                    Ok(sw) => {
                        drill_h.set(Drill::TakenAttackers(uid));
                        show_drill(&m, &sk_rows, &sw, true);
                    }
                    Err(e) => log::warn!("get_dmg_taken_attackers({uid}) failed: {e}"),
                }
            } else {
                match compute::get_skills(&enc_sk, uid) {
                    Ok(sw) => {
                        drill_h.set(Drill::Skills(uid));
                        show_drill(&m, &sk_rows, &sw, false);
                    }
                    Err(e) => log::warn!("get_skills({uid}) failed: {e}"),
                }
            }
        });
    }
    // 攻撃元クリック（被ダメ）→ その攻撃元の技別へ
    {
        let w = main.as_weak();
        let enc_sk = enc.clone();
        let sk_rows = skill_rows.clone();
        let drill_h = drill.clone();
        main.on_drill_row(move |uid_str| {
            let attacker: i64 = uid_str.as_str().parse().unwrap_or(0);
            if attacker == 0 {
                return;
            }
            let Some(m) = w.upgrade() else {
                return;
            };
            if let Drill::TakenAttackers(player) = drill_h.get() {
                match compute::get_dmg_taken_skills(&enc_sk, player, attacker) {
                    Ok(sw) => {
                        drill_h.set(Drill::TakenSkills(player, attacker));
                        show_drill(&m, &sk_rows, &sw, false);
                    }
                    Err(e) => log::warn!("get_dmg_taken_skills failed: {e}"),
                }
            }
        });
    }
    // 戻る（被ダメ技別→攻撃元一覧、それ以外→一覧へ）
    {
        let w = main.as_weak();
        let enc_b = enc.clone();
        let sk_rows = skill_rows.clone();
        let drill_h = drill.clone();
        main.on_back(move || {
            let Some(m) = w.upgrade() else {
                return;
            };
            if let Drill::TakenSkills(player, _) = drill_h.get() {
                if let Ok(sw) = compute::get_dmg_taken_attackers(&enc_b, player) {
                    drill_h.set(Drill::TakenAttackers(player));
                    show_drill(&m, &sk_rows, &sw, true);
                    return;
                }
            }
            drill_h.set(Drill::None);
            m.set_view(0);
        });
    }
    // ウォッチ切替（DPS一覧のピン）→ watchlist 更新・保存・即再描画
    {
        let w = main.as_weak();
        let wl_t = wl.clone();
        let enc_t = enc.clone();
        let rows_t = rows.clone();
        let cfg_t = cfg.clone();
        let tab_t = tab_cell.clone();
        let cl_t = compact_left.clone();
        let cr_t = compact_right.clone();
        main.on_toggle_watch(move |uid_str| {
            let uid: i64 = uid_str.as_str().parse().unwrap_or(0);
            if uid == 0 {
                return;
            }
            let sync = cfg_t.borrow().sync_timer_with_main;
            {
                let mut wl = wl_t.borrow_mut();
                // 同期ON: ピンは「タイマーから隠す/表示」(excluded の出し入れ)。
                // 同期OFF: 従来の手動ウォッチ(watched の出し入れ)。
                if sync {
                    wl.toggle_excluded(uid);
                } else {
                    wl.toggle(uid);
                }
                wl.save();
            }
            if w.upgrade().is_some() {
                let c = cfg_t.borrow();
                let pw = fetch_players(&enc_t, tab_t.get());
                let main_uids: Vec<i64> =
                    pw.player_rows.iter().map(|p| p.uid as i64).collect();
                let pin_uids = timer_roster(
                    &wl_t.borrow(),
                    false,
                    c.sync_timer_with_main,
                    c.sync_order_follow,
                    &main_uids,
                    &[],
                    pw.local_player_uid as i64,
                );
                apply_player_rows(
                    &rows_t,
                    &cl_t,
                    &cr_t,
                    build_rows(
                        &pw,
                        &c.name_template,
                        c.abbreviate_scores,
                        c.privacy_mask_names,
                        &pin_uids,
                        c.graph_player_count as i32,
                        c.graph_for_local_player,
                    ),
                );
            }
        });
    }
    // ウォッチ一括クリア（手動運用＝同期OFF時のみ設定UIに導線あり）。
    // watched・excluded を両方消し、過去の幽霊エントリ（離脱済プレイヤー等）を掃除する。
    {
        let w = main.as_weak();
        let wl_cw = wl.clone();
        let enc_cw = enc.clone();
        let rows_cw = rows.clone();
        let cfg_cw = cfg.clone();
        let tab_cw = tab_cell.clone();
        let cl_cw = compact_left.clone();
        let cr_cw = compact_right.clone();
        main.on_clear_watchlist(move || {
            wl_cw.borrow_mut().clear_all();
            wl_cw.borrow().save();
            if w.upgrade().is_some() {
                let c = cfg_cw.borrow();
                let pw = fetch_players(&enc_cw, tab_cw.get());
                let main_uids: Vec<i64> = pw.player_rows.iter().map(|p| p.uid as i64).collect();
                let pin_uids = timer_roster(
                    &wl_cw.borrow(),
                    false,
                    c.sync_timer_with_main,
                    c.sync_order_follow,
                    &main_uids,
                    &[],
                    pw.local_player_uid as i64,
                );
                apply_player_rows(
                    &rows_cw,
                    &cl_cw,
                    &cr_cw,
                    build_rows(
                        &pw,
                        &c.name_template,
                        c.abbreviate_scores,
                        c.privacy_mask_names,
                        &pin_uids,
                        c.graph_player_count as i32,
                        c.graph_for_local_player,
                    ),
                );
            }
        });
    }
    // 設定パネルの開閉。開く瞬間にテンプレ入力欄・自キャラUID欄へ最新値を push。
    {
        let w = main.as_weak();
        let cfg_ts = cfg.clone();
        let enc_ts = enc.clone();
        let cands_ts = uid_candidates.clone();
        main.on_toggle_settings(move || {
            if let Some(m) = w.upgrade() {
                let opening = !m.get_settings_open();
                if opening {
                    refresh_templates(&m, &cfg_ts.borrow());
                    refresh_selected_uid(&m, &enc_ts, &cands_ts);
                }
                m.set_settings_open(opening);
            }
        });
    }
    // 設定トグル変更 → cfg 更新・即適用・保存
    {
        let w = main.as_weak();
        let cfg_b = cfg.clone();
        let enc_sb = enc.clone();
        let self_ov = self_overlay.as_weak();
        let buff_ov = buff_overlay.as_weak();
        let stats_ov = stats_overlay.as_weak();
        let stat_catalog_left_sb = stat_catalog_left.clone();
        let stat_catalog_right_sb = stat_catalog_right.clone();
        let taskbar_flag_cb = taskbar_flag.clone();
        main.on_set_bool(move |key, val| {
            {
                let mut c = cfg_b.borrow_mut();
                match key.as_str() {
                    "self-status-overlay" => c.show_self_status_overlay = val,
                    "stats-overlay" => c.show_stats_overlay = val,
                    // ステータス窓の表示項目トグル（カタログ順で並べ直して安定化）
                    k if k.starts_with("stat.") => {
                        let key = k.trim_start_matches("stat.").to_string();
                        let mut set: std::collections::HashSet<String> =
                            c.stats_enabled.iter().cloned().collect();
                        if val {
                            set.insert(key);
                        } else {
                            set.remove(&key);
                        }
                        c.stats_enabled = settings::STAT_CATALOG
                            .iter()
                            .map(|d| d.key.to_string())
                            .filter(|k| set.contains(k))
                            .collect();
                    }
                    // ステータス表示項目の一括ON/OFF（カタログ全項目を1クリックで切替）
                    "stats-all-on" => {
                        c.stats_enabled = settings::STAT_CATALOG
                            .iter()
                            .map(|d| d.key.to_string())
                            .collect();
                    }
                    "stats-all-off" => c.stats_enabled.clear(),
                    "buff-overlay" => c.show_buff_overlay = val,
                    // 専用モードON時はイマジンタイマーを強制表示（旧UIと同挙動）。
                    // 専用モードは集計を早期returnし軽量化する仕組みのため、導出元(メインDPS一覧)
                    // が空になる「メインDPSと同期」とは相互排他（ONにしたら他方を自動OFF）。
                    "imagine-only" => {
                        c.imagine_only_mode = val;
                        if val {
                            c.show_buff_overlay = true;
                            c.sync_timer_with_main = false;
                        }
                    }
                    "show-crit" => c.show_crit = val,
                    "show-crit-value" => c.show_crit_value = val,
                    "show-lucky" => c.show_lucky = val,
                    "show-lucky-value" => c.show_lucky_value = val,
                    "show-hits" => c.show_hits = val,
                    "show-hpm" => c.show_hpm = val,
                    "show-score" => c.show_score = val,
                    "highlight-local" => c.highlight_local_player = val,
                    "abbreviate-scores" => c.abbreviate_scores = val,
                    "privacy-mask" => c.privacy_mask_names = val,
                    "aot" => c.always_on_top = val,
                    "three-min-auto-open" => c.three_min_auto_open = val,
                    "compact-split" => c.compact_split_mode = val,
                    "graph-for-local" => c.graph_for_local_player = val,
                    // imagine_only_mode と相互排他（ONにしたら専用モードを自動OFF）。
                    "sync-timer-with-main" => {
                        c.sync_timer_with_main = val;
                        if val {
                            c.imagine_only_mode = false;
                        }
                    }
                    "sync-order-follow" => c.sync_order_follow = val,
                    "imagine-col-tina" => c.show_imagine_tina = val,
                    "imagine-col-aluna" => c.show_imagine_aluna = val,
                    "imagine-col-tarta" => c.show_imagine_tarta = val,
                    "imagine-col-basilisk" => c.show_imagine_basilisk = val,
                    "imagine-compact-rows" => c.imagine_compact_rows = val,
                    "show-consumable" => c.show_consumable = val,
                    "show-in-taskbar" => c.show_in_taskbar = val,
                    "main-font-bold" => c.main_font_bold = val,
                    "stats-overlay-font-bold" => c.stats_overlay_font_bold = val,
                    "imagine-overlay-font-bold" => c.imagine_overlay_font_bold = val,
                    "overlay-outline" => c.overlay_outline = val,
                    "overlay-shadow" => c.overlay_shadow = val,
                    other => log::warn!("unknown setting key: {other}"),
                }
            }
            let c = cfg_b.borrow();
            if let Some(m) = w.upgrade() {
                apply_settings(&m, &c);
            }
            // タスクバー常駐⇔トレイ格納を全ウィンドウへ即時反映（再起動不要）。
            // 共有フラグも更新し、以降に再生成されるウィンドウへも引き継ぐ。
            if key.as_str() == "show-in-taskbar" {
                taskbar_flag_cb.set(c.show_in_taskbar);
                // オーバーレイの最小化ボタン表示を切替（トレイ格納時はOS最小化で復帰口が無いため隠す）
                if let Some(o) = self_ov.upgrade() {
                    o.set_show_minimize(c.show_in_taskbar);
                }
                if let Some(o) = buff_ov.upgrade() {
                    o.set_show_minimize(c.show_in_taskbar);
                }
                if let Some(o) = stats_ov.upgrade() {
                    o.set_show_minimize(c.show_in_taskbar);
                }
                #[cfg(windows)]
                {
                    let show = c.show_in_taskbar;
                    if let Some(m) = w.upgrade() {
                        overlay::apply_taskbar_mode(m.window(), show);
                    }
                    if let Some(o) = self_ov.upgrade() {
                        overlay::apply_taskbar_mode(o.window(), show);
                    }
                    if let Some(o) = buff_ov.upgrade() {
                        overlay::apply_taskbar_mode(o.window(), show);
                    }
                    if let Some(o) = stats_ov.upgrade() {
                        overlay::apply_taskbar_mode(o.window(), show);
                    }
                }
            }
            // ステータス表示項目のトグル・一括切替はカタログモデルを再生成してチェック状態へ反映。
            if key.as_str().starts_with("stat.")
                || key.as_str() == "stats-all-on"
                || key.as_str() == "stats-all-off"
            {
                let (l, r) = split_stat_catalog(&build_stat_catalog(&c.stats_enabled));
                stat_catalog_left_sb.set_vec(l);
                stat_catalog_right_sb.set_vec(r);
            }
            // フォント太字はオーバーレイ外観へ即反映（メイン太字はバフ/デバフ窓へ波及）。
            if matches!(
                key.as_str(),
                "main-font-bold"
                    | "stats-overlay-font-bold"
                    | "imagine-overlay-font-bold"
                    | "overlay-outline"
                    | "overlay-shadow"
            ) {
                apply_overlay_appearance(&c, &self_ov, &buff_ov, &stats_ov);
            }
            settings::save(&c);
            if key.as_str() == "self-status-overlay" {
                if let Some(o) = self_ov.upgrade() {
                    if c.show_self_status_overlay {
                        let _ = o.show();
                    } else {
                        let _ = o.hide();
                    }
                }
            }
            if key.as_str() == "stats-overlay" {
                if let Some(o) = stats_ov.upgrade() {
                    if c.show_stats_overlay {
                        let _ = o.show();
                    } else {
                        let _ = o.hide();
                    }
                }
            }
            if key.as_str() == "buff-overlay" {
                if let Some(o) = buff_ov.upgrade() {
                    if c.show_buff_overlay {
                        let _ = o.show();
                    } else {
                        let _ = o.hide();
                    }
                }
            }
            // sync-timer-with-main 側からの排他連動でも imagine_only_mode が変わるため、
            // どちらのキー経由でも compute 側の状態を実際の値へ整合させる。
            if matches!(key.as_str(), "imagine-only" | "sync-timer-with-main") {
                compute::set_imagine_only_mode(&enc_sb, c.imagine_only_mode);
                // 専用モードON時は強制表示にした buff overlay を実際に出す
                if c.imagine_only_mode && c.show_buff_overlay {
                    if let Some(o) = buff_ov.upgrade() {
                        let _ = o.show();
                    }
                }
            }
        });
    }
    // 不透明度スライダー
    {
        let w = main.as_weak();
        let cfg_o = cfg.clone();
        main.on_set_opacity(move |v| {
            let clamped = v.clamp(0.05, 1.0) as f64;
            cfg_o.borrow_mut().opacity = clamped;
            if let Some(m) = w.upgrade() {
                m.set_win_opacity(clamped as f32);
            }
            settings::save(&cfg_o.borrow());
        });
    }
    // オーバーレイ共通の不透明度（メインとは独立）。スライダー操作で全オーバーレイへ即反映。
    {
        let w = main.as_weak();
        let cfg_o = cfg.clone();
        let self_o = self_overlay.as_weak();
        let buff_o = buff_overlay.as_weak();
        let stats_o = stats_overlay.as_weak();
        main.on_set_overlay_opacity(move |v| {
            // オーバーレイのみ完全透明(0)を許可。スライダー左端付近(<0.04)は0へスナップして
            // 「完全透明＝クリック透過」をはっきり選べるようにする。それ以外は下限0.04。
            let clamped: f64 = if v < 0.04 { 0.0 } else { v.clamp(0.04, 1.0) as f64 };
            cfg_o.borrow_mut().overlay_opacity = clamped;
            if let Some(m) = w.upgrade() {
                m.set_overlay_opacity(clamped as f32);
            }
            apply_overlay_appearance(&cfg_o.borrow(), &self_o, &buff_o, &stats_o);
            settings::save(&cfg_o.borrow());
        });
    }
    // 文字色 HSV ピッカー操作。h/s/v(各0..1)を hex 化して保存し、全オーバーレイへ即反映。
    {
        let w = main.as_weak();
        let cfg_p = cfg.clone();
        let self_o = self_overlay.as_weak();
        let buff_o = buff_overlay.as_weak();
        let stats_o = stats_overlay.as_weak();
        main.on_pick_overlay_text(move |h, s, v| {
            cfg_p.borrow_mut().overlay_text_color = hsv_to_hex(h, s, v);
            let c = cfg_p.borrow();
            if let Some(m) = w.upgrade() {
                apply_settings(&m, &c);
                // ドラッグ追従を滑らかに: h/s/v は入力値そのままで上書き（hex 量子化の戻りで跳ねさせない）
                m.set_overlay_text_h(h);
                m.set_overlay_text_s(s);
                m.set_overlay_text_v(v);
            }
            apply_overlay_appearance(&c, &self_o, &buff_o, &stats_o);
            settings::save(&c);
        });
    }
    // アクセント色 HSV ピッカー操作。h/s/v(各0..1)を hex 化して保存し、Theme へ即反映。
    {
        let w = main.as_weak();
        let cfg_a = cfg.clone();
        main.on_pick_accent(move |h, s, v| {
            cfg_a.borrow_mut().accent_theme = hsv_to_hex(h, s, v);
            let c = cfg_a.borrow();
            if let Some(m) = w.upgrade() {
                apply_settings(&m, &c);
                // ドラッグ追従を滑らかに: h/s/v は入力値そのままで上書き（hex 量子化の戻りで跳ねさせない）
                m.set_accent_h(h);
                m.set_accent_s(s);
                m.set_accent_v(v);
            }
            settings::save(&c);
        });
    }
    // 数値設定ステッパー（key と方向 dir=±1）。キー毎に step/範囲を持ち、必要なら即適用。
    // poll-interval はポーリングタイマー再構築が要るため次回起動時に反映（永続化のみ）。
    {
        let w = main.as_weak();
        let cfg_n = cfg.clone();
        let self_o = self_overlay.as_weak();
        let buff_o = buff_overlay.as_weak();
        let stats_o = stats_overlay.as_weak();
        main.on_bump_num(move |key, dir| {
            let d = dir as f64;
            {
                let mut c = cfg_n.borrow_mut();
                match key.as_str() {
                    "combat-exit" => {
                        c.combat_exit_sec = (c.combat_exit_sec + d).clamp(0.0, 60.0);
                        compute::set_combat_exit_timeout(c.combat_exit_sec);
                    }
                    "poll-interval" => {
                        c.poll_interval_ms = (c.poll_interval_ms + d * 50.0).clamp(50.0, 2000.0);
                    }
                    "three-min-dur" => {
                        c.three_min_duration_sec = (c.three_min_duration_sec + d * 30.0).clamp(30.0, 1800.0);
                    }
                    "history-limit" => {
                        c.history_limit = (c.history_limit + d * 5.0).clamp(0.0, 100.0);
                        compute::set_history_limit(c.history_limit);
                    }
                    "ts-samples" => {
                        c.time_series_samples = (c.time_series_samples + d * 10.0).clamp(10.0, 200.0);
                        compute::set_time_series_config(c.time_series_samples, c.time_series_interval_ms);
                    }
                    "ts-interval" => {
                        c.time_series_interval_ms = (c.time_series_interval_ms + d * 250.0).clamp(250.0, 5000.0);
                        compute::set_time_series_config(c.time_series_samples, c.time_series_interval_ms);
                    }
                    "graph-count" => {
                        c.graph_player_count = (c.graph_player_count + d).clamp(0.0, 10.0);
                    }
                    "font-size" => {
                        c.font_size = (c.font_size + d).clamp(10.0, 18.0);
                    }
                    "stats-overlay-font-size" => {
                        c.stats_overlay_font_size = (c.stats_overlay_font_size + d).clamp(8.0, 100.0);
                    }
                    "imagine-overlay-font-size" => {
                        c.imagine_overlay_font_size =
                            (c.imagine_overlay_font_size + d).clamp(8.0, 24.0);
                    }
                    other => log::warn!("unknown num key: {other}"),
                }
            }
            let c = cfg_n.borrow();
            if let Some(m) = w.upgrade() {
                apply_settings(&m, &c);
            }
            apply_overlay_appearance(&c, &self_o, &buff_o, &stats_o);
            settings::save(&c);
        });
    }
    // テンプレ編集（edited）。cfg と preview のみ更新（value は push しない＝入力中クロバー防止）。
    {
        let w = main.as_weak();
        let cfg_s = cfg.clone();
        let self_o = self_overlay.as_weak();
        let buff_o = buff_overlay.as_weak();
        let stats_o = stats_overlay.as_weak();
        main.on_set_str(move |key, val| {
            {
                let mut c = cfg_s.borrow_mut();
                match key.as_str() {
                    "name-template" => c.name_template = val.to_string(),
                    "copy-template" => c.copy_template = val.to_string(),
                    "startup-tab" => c.startup_tab = val.to_string(),
                    // Slint 1.16 は @tr リテラルを定数畳み込みするため select_bundled_translation を
                    // ランタイムで呼んでも既存 UI は再翻訳されない。永続化のみ行い、反映は次回起動時
                    // （main.rs 起動時の select_bundled_translation(&cfg.language)）。UI 側で再起動要を明示。
                    "language" => c.language = val.to_string(),
                    "accent-theme" => c.accent_theme = val.to_string(),
                    "main-font" => c.main_font = val.to_string(),
                    "stats-overlay-font" => c.stats_overlay_font = val.to_string(),
                    "imagine-overlay-font" => c.imagine_overlay_font = val.to_string(),
                    "overlay-text-color" => c.overlay_text_color = val.to_string(),
                    other => log::warn!("unknown str key: {other}"),
                }
            }
            let c = cfg_s.borrow();
            if let Some(m) = w.upgrade() {
                // cfg-ui(起動タブ強調)・nums・font-scale を反映。テンプレ value は
                // push されない（apply_settings は触らない）ため入力中もクロバーしない。
                apply_settings(&m, &c);
                let (np, cp) = template_previews(&c);
                m.set_name_preview(np);
                m.set_copy_preview(cp);
            }
            apply_overlay_appearance(&c, &self_o, &buff_o, &stats_o);
            settings::save(&c);
        });
    }
    // テンプレ リセット。既定値へ戻し、value を push して入力欄も更新する。
    {
        let w = main.as_weak();
        let cfg_r = cfg.clone();
        main.on_reset_str(move |key| {
            {
                let mut c = cfg_r.borrow_mut();
                match key.as_str() {
                    "name-template" => c.name_template = settings::DEFAULT_NAME_TEMPLATE.to_string(),
                    "copy-template" => c.copy_template = settings::DEFAULT_COPY_TEMPLATE.to_string(),
                    other => log::warn!("unknown reset key: {other}"),
                }
            }
            let c = cfg_r.borrow();
            if let Some(m) = w.upgrade() {
                refresh_templates(&m, &c);
            }
            settings::save(&c);
        });
    }
    // 自キャラUID 確定（空文字=クリア）。set_selected_uid は集計をリセットするため
    // Enter / 候補クリック / クリア の明示操作時のみ呼ぶ。
    {
        let w = main.as_weak();
        let enc_su = enc.clone();
        let cands = uid_candidates.clone();
        main.on_set_selected_uid(move |s| {
            let t = s.as_str().trim();
            let uid: Option<f64> = if t.is_empty() {
                None
            } else {
                t.parse::<f64>().ok().filter(|v| *v > 0.0)
            };
            compute::set_selected_uid(&enc_su, uid);
            if let Some(m) = w.upgrade() {
                refresh_selected_uid(&m, &enc_su, &cands);
            }
        });
    }
    // 一覧コピー（現在タブの行を copy_template で整形して \n 連結→クリップボード）。
    {
        let w = main.as_weak();
        let enc_c = enc.clone();
        let cfg_c = cfg.clone();
        let tab_c = tab_cell.clone();
        main.on_copy_list(move || {
            let pw = fetch_players(&enc_c, tab_c.get());
            if pw.player_rows.is_empty() {
                return;
            }
            let text = {
                let c = cfg_c.borrow();
                pw.player_rows
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        format::format_row_template(
                            &copy_row_data(p, (i + 1) as i32),
                            &c.copy_template,
                            c.abbreviate_scores,
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text)) {
                Ok(()) => {
                    if let Some(m) = w.upgrade() {
                        m.set_copied(true);
                        let wk = m.as_weak();
                        Timer::single_shot(Duration::from_millis(800), move || {
                            if let Some(m) = wk.upgrade() {
                                m.set_copied(false);
                            }
                        });
                    }
                }
                Err(e) => log::warn!("clipboard copy failed: {e}"),
            }
        });
    }
    // 履歴: 見出しクリックで展開トグル（単一展開）。
    {
        let w = main.as_weak();
        let hr = history_rows.clone();
        let he = history_expanded.clone();
        let cfg_h = cfg.clone();
        main.on_toggle_history(move |id_str| {
            let id: i64 = id_str.as_str().parse().unwrap_or(0);
            he.set(if he.get() == Some(id) { None } else { Some(id) });
            if w.upgrade().is_some() {
                let hist = compute::get_history();
                hr.set_vec(build_history_rows(
                    &hist,
                    he.get(),
                    cfg_h.borrow().privacy_mask_names,
                ));
            }
        });
    }
    // 履歴クリア。食事/シロップの表示も一緒に消す。
    {
        let hr = history_rows.clone();
        let he = history_expanded.clone();
        let enc_ch = enc.clone();
        main.on_clear_history(move || {
            compute::clear_history();
            compute::clear_consumables(&enc_ch);
            he.set(None);
            hr.set_vec(Vec::new());
        });
    }
    // 3分計測: 通常→開始 / 待機・計測中→キャンセル（確認ダイアログは省略）。
    {
        let enc_m = enc.clone();
        let cfg_m = cfg.clone();
        main.on_toggle_measure(move || {
            let status = compute::get_measure_mode_status(&enc_m);
            if status.kind == "normal" {
                compute::start_3min_measure_mode(&enc_m, cfg_m.borrow().three_min_duration_sec);
            } else {
                compute::cancel_3min_measure_mode(&enc_m);
            }
        });
    }
    // 集計の一時停止トグル / 手動リセット
    {
        let enc_p = enc.clone();
        main.on_toggle_pause(move || compute::toggle_pause(&enc_p));
    }
    {
        let enc_r = enc.clone();
        let wl_r = wl.clone();
        main.on_reset_encounter(move || {
            compute::reset_encounter(&enc_r);
            // 旧版同様リセットでウォッチ対象をクリア（excludedは維持）。
            // 自動追加ONなら次tickで再充填される。
            let mut wl = wl_r.borrow_mut();
            wl.clear_watched();
            wl.save();
        });
    }
    // 3分計測 結果パネル: 閉じる
    {
        let w = main.as_weak();
        main.on_close_result(move || {
            if let Some(m) = w.upgrade() {
                m.set_result_open(false);
            }
        });
    }
    // 3分計測 結果パネル: 行クリックでスキル内訳の対象プレイヤーを切替
    {
        let w = main.as_weak();
        let lr = last_result.clone();
        let cs = captured_skills.clone();
        let rr = result_rows.clone();
        let rsr = result_skill_rows.clone();
        let rp = result_pie.clone();
        let rl = result_legend.clone();
        let sel_p = selected_result_player.clone();
        let sel_s = selected_result_skill.clone();
        let cfg_sp = cfg.clone();
        let cdim = char_plot_dims.clone();
        let sdim = skill_plot_dims.clone();
        main.on_select_result_player(move |uid_str| {
            let uid: i64 = uid_str.as_str().parse().unwrap_or(0);
            let snap = lr.borrow();
            let Some(snap) = snap.as_ref() else {
                return;
            };
            if let Some(m) = w.upgrade() {
                apply_result_selection(
                    &m,
                    uid,
                    snap,
                    &cs.borrow(),
                    &rr,
                    &rsr,
                    &rp,
                    &rl,
                    &sel_p,
                    &sel_s,
                    cfg_sp.borrow().privacy_mask_names,
                    cdim.get(),
                    sdim.get(),
                );
            }
        });
    }
    // 3分計測 結果パネル: スキル行クリックでエリア2 折れ線の対象スキルを切替
    {
        let w = main.as_weak();
        let cs = captured_skills.clone();
        let rsr = result_skill_rows.clone();
        let sel_p = selected_result_player.clone();
        let sel_s = selected_result_skill.clone();
        let sdim = skill_plot_dims.clone();
        main.on_select_result_skill(move |uid_str| {
            let skill_uid: i64 = uid_str.as_str().parse().unwrap_or(0);
            sel_s.set(skill_uid);
            let captured = cs.borrow();
            let empty = Vec::new();
            let skills = captured.get(&sel_p.get()).unwrap_or(&empty);
            if let Some(m) = w.upgrade() {
                let dur = m.get_result_duration_ms() as f64;
                apply_result_skill_selection(&m, skills, skill_uid, &rsr, dur, sdim.get());
            }
        });
    }
    // 3分計測 結果パネル: キャラ折れ線プロットの実寸変化→実寸座標でパス再生成
    {
        let w = main.as_weak();
        let lr = last_result.clone();
        let sel_p = selected_result_player.clone();
        let cdim = char_plot_dims.clone();
        main.on_result_char_spark_resized(move |pw, ph| {
            cdim.set((pw, ph));
            let Some(m) = w.upgrade() else { return; };
            let snap = lr.borrow();
            let Some(snap) = snap.as_ref() else { return; };
            let s = build_char_spark(snap, sel_p.get(), pw, ph);
            m.set_result_char_spark_visible(!s.is_empty());
            m.set_result_char_spark(s.into());
        });
    }
    // 3分計測 結果パネル: スキル折れ線プロットの実寸変化→実寸座標でパス再生成
    {
        let w = main.as_weak();
        let lr = last_result.clone();
        let cs = captured_skills.clone();
        let sel_p = selected_result_player.clone();
        let sel_s = selected_result_skill.clone();
        let sdim = skill_plot_dims.clone();
        main.on_result_skill_spark_resized(move |pw, ph| {
            sdim.set((pw, ph));
            let Some(m) = w.upgrade() else { return; };
            let dur = lr.borrow().as_ref().map(|s| s.duration_ms).unwrap_or(1.0);
            let captured = cs.borrow();
            let empty = Vec::new();
            let skills = captured.get(&sel_p.get()).unwrap_or(&empty);
            let s = build_skill_spark(skills, sel_s.get(), dur, pw, ph);
            m.set_result_skill_spark_visible(!s.is_empty());
            m.set_result_skill_spark(s.into());
        });
    }
    // 3分計測 結果パネル: 上位10行を copy_template でコピー
    {
        let lr = last_result.clone();
        let cfg_cr = cfg.clone();
        main.on_copy_result(move || {
            let snap = lr.borrow();
            let Some(snap) = snap.as_ref() else {
                return;
            };
            let text = {
                let c = cfg_cr.borrow();
                snap.player_rows
                    .iter()
                    .take(10)
                    .enumerate()
                    .map(|(i, p)| {
                        format::format_row_template(
                            &copy_row_data(p, (i + 1) as i32),
                            &c.copy_template,
                            c.abbreviate_scores,
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            if let Err(e) = arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text)) {
                log::warn!("clipboard copy (result) failed: {e}");
            }
        });
    }
    // 3分計測 結果パネル: 閉じて再計測を開始
    {
        let w = main.as_weak();
        let enc_rm = enc.clone();
        let cfg_rm = cfg.clone();
        main.on_restart_measure(move || {
            if let Some(m) = w.upgrade() {
                m.set_result_open(false);
            }
            compute::start_3min_measure_mode(&enc_rm, cfg_rm.borrow().three_min_duration_sec);
        });
    }
    // 結果画面の画像コピー（ウィンドウのスナップショット→モーダル矩形へクロップ→クリップボード）。
    // ウィンドウは半透明合成のため α=255 を強制しないと貼り付け先で透ける。
    {
        let w = main.as_weak();
        main.on_copy_result_image(move || {
            let Some(m) = w.upgrade() else { return };
            let win = m.window();
            let snap = match win.take_snapshot() {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("result image: snapshot failed: {e}");
                    return;
                }
            };
            // モーダルは論理 12px マージン（app.slint の結果モーダル矩形と一致させる）
            let margin = (12.0 * win.scale_factor()).round() as usize;
            let (full_w, full_h) = (snap.width() as usize, snap.height() as usize);
            let crop_w = full_w.saturating_sub(margin * 2);
            let crop_h = full_h.saturating_sub(margin * 2);
            if crop_w == 0 || crop_h == 0 {
                log::warn!("result image: window too small to crop ({full_w}x{full_h})");
                return;
            }
            let src = snap.as_slice();
            let mut bytes = Vec::with_capacity(crop_w * crop_h * 4);
            for row in margin..margin + crop_h {
                let line = &src[row * full_w + margin..row * full_w + margin + crop_w];
                for px in line {
                    bytes.extend_from_slice(&[px.r, px.g, px.b, 255]);
                }
            }
            let img = arboard::ImageData {
                width: crop_w,
                height: crop_h,
                bytes: bytes.into(),
            };
            match arboard::Clipboard::new().and_then(|mut cb| cb.set_image(img)) {
                Ok(()) => {
                    m.set_result_img_copied(true);
                    let wk = m.as_weak();
                    Timer::single_shot(Duration::from_millis(800), move || {
                        if let Some(m) = wk.upgrade() {
                            m.set_result_img_copied(false);
                        }
                    });
                }
                Err(e) => log::warn!("clipboard image copy failed: {e}"),
            }
        });
    }

    main.show()?;
    if cfg.borrow().show_self_status_overlay {
        let _ = self_overlay.show();
    }
    if cfg.borrow().show_buff_overlay {
        let _ = buff_overlay.show();
    }
    if cfg.borrow().show_stats_overlay {
        let _ = stats_overlay.show();
    }

    // 周期ポーリング＋初回セットアップ（位置復元）＋自動保存
    let main_w = main.as_weak();
    let enc_poll = enc.clone();
    let saved = window_state::load();
    let last_saved = Rc::new(RefCell::new(saved.clone()));
    let mut st = PollState::default();
    let tab_cell_poll = tab_cell.clone();
    let drill_poll = drill.clone();
    let skill_rows_poll = skill_rows.clone();
    let self_overlay_w = self_overlay.as_weak();
    let self_buffs_poll = self_buffs.clone();
    let self_debuffs_poll = self_debuffs.clone();
    let buff_overlay_w = buff_overlay.as_weak();
    let buff_players_poll = buff_players.clone();
    let stats_overlay_w = stats_overlay.as_weak();
    let stats_rows_poll = stats_rows.clone();
    let cfg_poll = cfg.clone();
    let wl_poll = wl.clone();
    let history_rows_poll = history_rows.clone();
    let history_expanded_poll = history_expanded.clone();
    let result_rows_poll = result_rows.clone();
    let result_skill_rows_poll = result_skill_rows.clone();
    let result_pie_poll = result_pie.clone();
    let result_legend_poll = result_legend.clone();
    let selected_result_player_poll = selected_result_player.clone();
    let selected_result_skill_poll = selected_result_skill.clone();
    let captured_skills_poll = captured_skills.clone();
    let last_result_poll = last_result.clone();
    let char_plot_dims_poll = char_plot_dims.clone();
    let skill_plot_dims_poll = skill_plot_dims.clone();
    let compact_left_poll = compact_left.clone();
    let compact_right_poll = compact_right.clone();
    let uid_candidates_poll = uid_candidates.clone();
    // タスクトレイ／クリックスルー状態（poll closure が move で保持）
    let click_through = Rc::new(Cell::new(false));
    #[cfg(windows)]
    let tray_holder: Rc<RefCell<Option<tray::Tray>>> = Rc::new(RefCell::new(None));
    let poll_ms = cfg.borrow().poll_interval_ms.max(50.0) as u64;
    // オーバーレイ(バフ/ステータス/イマジン)はメイン表より低頻度で更新する。
    // 稼働中バフのアーク/バーは残量比から毎tick変化するため set_vec_if_changed では
    // 抑止できず、200ms poll では 5Hz で再描画され続ける（実測で戦闘中コストの主因）。
    // 体感を保てる ~3Hz 相当へ間引く（poll が既に十分遅ければ stride=1＝毎tick）。
    const OVERLAY_REFRESH_MS: u64 = 333;
    let overlay_stride =
        ((OVERLAY_REFRESH_MS as f64 / poll_ms as f64).round() as u64).max(1);

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(poll_ms), move || {
        st.tick += 1;
        let Some(m) = main_w.upgrade() else {
            return;
        };

        // 初回: winit 実体化後に位置復元 → 完了 tick でトレイ生成。
        let just_setup = poll_setup_once(&m, &mut st, &saved);
        #[cfg(windows)]
        if just_setup {
            *tray_holder.borrow_mut() = tray::create();
            log::info!("tray created: {}", tray_holder.borrow().is_some());
            // 起動時のタスクバー常駐モードをメインへ適用（実体化後）。
            overlay::apply_taskbar_mode(m.window(), cfg_poll.borrow().show_in_taskbar);
        }
        #[cfg(not(windows))]
        let _ = just_setup;

        // オーバーレイの位置/サイズ復元（表示された最初のtickで一度）。非表示で None に戻す。
        poll_overlay_restore(
            &mut st,
            &cfg_poll,
            &self_overlay_w,
            &buff_overlay_w,
            &stats_overlay_w,
            &last_saved,
        );

        // トレイメニューのイベント処理（クリックスルー切替・表示/非表示・終了）
        #[cfg(windows)]
        poll_tray_events(
            &m,
            &cfg_poll,
            &self_overlay_w,
            &buff_overlay_w,
            &stats_overlay_w,
            &tray_holder,
            &main_visible,
            &click_through,
        );

        // 食事/シロップ残時間ストアを更新（戦闘終了をまたいで保持・失効除去）
        compute::refresh_consumables(&enc_poll);

        // ライブ集計を反映（共有セルの現在タブに応じて取得）
        let header = compute::get_header_info(&enc_poll, tab_cell_poll.get());
        m.set_total_text(format::format_dps(header.total_dps).into());
        m.set_elapsed_text(format::format_elapsed(header.elapsed_ms).into());

        // 観測ステータス（0=起動中 1=待機 2=受信中 3=失敗）。
        // 「受信中」はゲームサーバのパケットを直近10秒以内に処理した場合のみ。
        let cs = compute::get_capture_status();
        m.set_capture_state(match cs.state {
            2 => 3,
            1 if (0.0..10_000.0).contains(&cs.ms_since_last_game_packet) => 2,
            1 => 1,
            _ => 0,
        });

        // 一時停止状態をボタンへ反映
        m.set_paused(compute::is_paused(&enc_poll));

        // 設定パネルを開いている間は自キャラUID候補を生きたまま更新（入力欄は触らない）
        if m.get_settings_open() {
            refresh_uid_candidates(&enc_poll, &uid_candidates_poll);
        }

        // 3分計測の状態反映＋残0で自動確定（→履歴。結果パネルは後続増分）
        let ms = compute::get_measure_mode_status(&enc_poll);
        let mkind = match ms.kind.as_str() {
            "pending" => 1,
            "active" => 2,
            _ => 0,
        };
        m.set_measure_kind(mkind);
        if mkind == 2 {
            let rem = ms.remaining_ms.unwrap_or(0.0).max(0.0);
            m.set_measure_text(format::format_elapsed(rem).into());
            if rem <= 0.0 {
                // 折れ線を右端(=計測末尾)まで届かせるため、捕捉・確定の前に終端サンプルを足す。
                // （スキル内訳は下の get_skills で確定前に取得されるため順序が重要）
                compute::seal_3min_series(&enc_poll);
                // finalize で集計が消えるため、直前にライブのスキル内訳と自分uidを捕捉。
                let pw = compute::get_dps_players(&enc_poll);
                let local_uid = pw.local_player_uid as i64;
                let mut skills: std::collections::HashMap<
                    i64,
                    Vec<bpsr_core::models::SkillRow>,
                > = std::collections::HashMap::new();
                for p in &pw.player_rows {
                    let uid = p.uid as i64;
                    if let Ok(sw) = compute::get_skills(&enc_poll, uid) {
                        skills.insert(uid, sw.skill_rows);
                    }
                }
                if let Some(snap) = compute::finalize_3min_measure_mode(&enc_poll) {
                    let c = cfg_poll.borrow();
                    if c.three_min_auto_open && !c.imagine_only_mode {
                        // 既定の選択=自分(スキル有り)・無ければ最上位プレイヤー
                        let default_uid = if local_uid != 0 && skills.contains_key(&local_uid) {
                            local_uid
                        } else {
                            snap.player_rows.first().map(|p| p.uid as i64).unwrap_or(0)
                        };
                        *captured_skills_poll.borrow_mut() = skills;
                        *last_result_poll.borrow_mut() = Some(snap.clone());
                        show_result(
                            &m,
                            &snap,
                            &captured_skills_poll.borrow(),
                            default_uid,
                            &result_rows_poll,
                            &result_skill_rows_poll,
                            &result_pie_poll,
                            &result_legend_poll,
                            &selected_result_player_poll,
                            &selected_result_skill_poll,
                            c.privacy_mask_names,
                            char_plot_dims_poll.get(),
                            skill_plot_dims_poll.get(),
                        );
                    }
                }
            }
        }

        // メイン表示中タブの並び順(uid列)。イマジンタイマーの行順をこれに追従させる。
        // 履歴タブ等で算出できない場合は空＝従来の watched 順へフォールバック。
        let mut main_ordered_uids: Vec<i64> = Vec::new();
        let mut main_local_uid: i64 = 0;
        let cur_tab = tab_cell_poll.get();
        if cur_tab == 3 {
            // 履歴タブ: 確定済みエンカウンタ一覧を反映（展開状態は維持）。
            let privacy = cfg_poll.borrow().privacy_mask_names;
            let hist = compute::get_history();
            history_rows_poll.set_vec(build_history_rows(
                &hist,
                history_expanded_poll.get(),
                privacy,
            ));
        } else {
            let pw = fetch_players(&enc_poll, cur_tab);
            main_ordered_uids = pw.player_rows.iter().map(|p| p.uid as i64).collect();
            main_local_uid = pw.local_player_uid as i64;
            let c = cfg_poll.borrow();
            m.set_show_graph_col(graph_col_active(&c, cur_tab));
            // メイン行のピン点灯集合（専用モードは導出元が異なるため常に非専用扱い）。
            let pin_uids = timer_roster(
                &wl_poll.borrow(),
                false,
                c.sync_timer_with_main,
                c.sync_order_follow,
                &main_ordered_uids,
                &[],
                main_local_uid,
            );
            apply_player_rows(
                &rows,
                &compact_left_poll,
                &compact_right_poll,
                build_rows(
                    &pw,
                    &c.name_template,
                    c.abbreviate_scores,
                    c.privacy_mask_names,
                    &pin_uids,
                    c.graph_player_count as i32,
                    c.graph_for_local_player,
                ),
            );
        }

        // ドリルダウン中はライブ更新
        match drill_poll.get() {
            Drill::Skills(uid) => {
                if let Ok(sw) = compute::get_skills(&enc_poll, uid) {
                    skill_rows_poll.set_vec(build_skill_rows(&sw));
                }
            }
            Drill::TakenAttackers(uid) => {
                if let Ok(sw) = compute::get_dmg_taken_attackers(&enc_poll, uid) {
                    skill_rows_poll.set_vec(build_skill_rows(&sw));
                }
            }
            Drill::TakenSkills(p, a) => {
                if let Ok(sw) = compute::get_dmg_taken_skills(&enc_poll, p, a) {
                    skill_rows_poll.set_vec(build_skill_rows(&sw));
                }
            }
            Drill::None => {}
        }

        // オーバーレイは overlay_stride tick ごとにのみ更新（稼働中アーク/バーの 5Hz 再描画を間引く）。
        // メイン表・ヘッダは毎tick維持。設定の即時反映も最大 ~stride 分だけ遅延するが体感影響は無い。
        let refresh_overlays = st.tick % overlay_stride == 0;

        // オーバーレイの文字サイズは窓ごとに独立。
        // バフ/デバフ=メイン窓に追随、ステータス・イマジンは各専用設定。
        let (self_scale, stats_scale, imagine_scale) = {
            let c = cfg_poll.borrow();
            (
                (c.font_size / 12.0) as f32,
                (c.stats_overlay_font_size / 12.0) as f32,
                (c.imagine_overlay_font_size / 12.0) as f32,
            )
        };

        // 自キャラ オーバーレイ更新（表示中のみ・低頻度）
        if refresh_overlays && cfg_poll.borrow().show_self_status_overlay {
            if let Some(o) = self_overlay_w.upgrade() {
                o.set_font_scale(self_scale);
                let s = compute::get_self_buff_status(&enc_poll);
                o.set_waiting(s.local_player_uid == 0.0);
                set_vec_if_changed(&self_buffs_poll, &mut st.last_self_buffs, build_status_entries(&s.buffs));
                set_vec_if_changed(&self_debuffs_poll, &mut st.last_self_debuffs, build_status_entries(&s.debuffs));
            }
        }

        // 自キャラ ステータス オーバーレイ更新（表示中のみ・低頻度）
        if refresh_overlays && cfg_poll.borrow().show_stats_overlay {
            if let Some(o) = stats_overlay_w.upgrade() {
                o.set_font_scale(stats_scale);
                let s = compute::get_self_stats(&enc_poll);
                o.set_waiting(s.local_player_uid == 0.0);
                let enabled = cfg_poll.borrow().stats_enabled.clone();
                set_vec_if_changed(&stats_rows_poll, &mut st.last_stats_rows, build_stat_entries(&s, &enabled));
            }
        }

        // バフタイマー オーバーレイ更新（表示中のみ・低頻度）
        if refresh_overlays && cfg_poll.borrow().show_buff_overlay {
            if let Some(o) = buff_overlay_w.upgrade() {
                o.set_font_scale(imagine_scale);
                let imagine_only = cfg_poll.borrow().imagine_only_mode;
                {
                    // 表示するイマジン列・レイアウトを設定から反映（極小コスト・即時反映）
                    let c = cfg_poll.borrow();
                    o.set_show_tina(c.show_imagine_tina);
                    o.set_show_aluna(c.show_imagine_aluna);
                    o.set_show_tarta(c.show_imagine_tarta);
                    o.set_show_basilisk(c.show_imagine_basilisk);
                    o.set_compact(c.imagine_compact_rows);
                }
                // 名簿源は3分岐（timer_roster 参照）。専用モードはバフ追跡から自動（メイン一覧が
                // 空集計のため使えない）。専用OFFはメイン順(main_ordered_uids)、無い(履歴タブ等)
                // 場合は live DPS 順を代用。
                let (order_src, local_uid): (Vec<i64>, i64) = if imagine_only {
                    (Vec::new(), main_local_uid)
                } else if main_ordered_uids.is_empty() {
                    let pw = compute::get_dps_players(&enc_poll);
                    let uids = pw.player_rows.iter().map(|p| p.uid as i64).collect();
                    (uids, pw.local_player_uid as i64)
                } else {
                    (main_ordered_uids.clone(), main_local_uid)
                };
                let buff_tracked_uids = if imagine_only {
                    compute::get_buff_tracked_uids(&enc_poll)
                } else {
                    Vec::new()
                };
                let (sync, order_follow) = {
                    let c = cfg_poll.borrow();
                    (c.sync_timer_with_main, c.sync_order_follow)
                };
                let display_uids = timer_roster(
                    &wl_poll.borrow(),
                    imagine_only,
                    sync,
                    order_follow,
                    &order_src,
                    &buff_tracked_uids,
                    local_uid,
                );
                o.set_empty(display_uids.is_empty());
                // 表示集合が空なら空行で更新（古い行が残って名前が消えない不具合を防ぐ）。
                // いずれも set_vec_if_changed で内容不変時は再描画を省く。
                let privacy_mask = cfg_poll.borrow().privacy_mask_names;
                let next_buff_rows = if !display_uids.is_empty() {
                    let uids: Vec<f64> = display_uids.iter().map(|&u| u as f64).collect();
                    let t = compute::get_tracked_buffs(&enc_poll, uids);
                    build_buff_rows(&t, &display_uids, privacy_mask)
                } else {
                    Vec::new()
                };
                set_vec_if_changed(&buff_players_poll, &mut st.last_buff_players, next_buff_rows);
            }
        }

        // 起動/表示直後の preferred サイズ再アサートを settle 期間中の再適用で打ち消す
        // （自動保存ガードより手前で実施）。
        poll_window_settle(&m, &st, &self_overlay_w, &buff_overlay_w, &stats_overlay_w);

        // レイアウト自動保存（復元確定後・差分時のみ）。
        poll_auto_save(
            &m,
            &st,
            &cfg_poll,
            &self_overlay_w,
            &buff_overlay_w,
            &stats_overlay_w,
            &last_saved,
        );
    });

    // トレイ格納（全ウィンドウ hide）でアプリが終了しないよう、最後のウィンドウが
    // 閉じてもループを止めない。終了はトレイ「終了」/ ×ボタンの quit_event_loop のみ。
    slint::run_event_loop_until_quit()?;

    engine::name_cache::flush();
    engine::selected_uid::flush();
    compute::save_consumables(&enc); // 終了時に最終状態を永続化
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wl_with(watched: &[i64], excluded: &[i64]) -> watchlist::Watchlist {
        watchlist::Watchlist {
            watched: watched.to_vec(),
            excluded: excluded.to_vec(),
        }
    }

    // 専用モードON: 名簿源は buff_tracked_uids（first-seen順）。excluded を除き、
    // 自分(local_uid)を先頭固定＋以降は元の順（first-seen）をそのまま使う。
    #[test]
    fn test_timer_roster_imagine_only_orders_local_first_then_first_seen() {
        let wl = wl_with(&[], &[300]); // 300 は手動で隠している
        let buff_tracked = vec![200, 100, 300]; // first-seen 順（200が最初に検出）
        let roster = timer_roster(&wl, true, false, true, &[], &buff_tracked, 100);
        // 100(自分)が先頭、300はexcludedで除外、残りはfirst-seen順(200)
        assert_eq!(roster, vec![100, 200]);
    }

    // 専用OFF・同期ON・追従ON: メイン順そのまま（excluded のみ除外）。
    #[test]
    fn test_timer_roster_sync_on_follow_on_uses_main_order() {
        let wl = wl_with(&[], &[20]);
        let main_ordered = vec![30, 20, 10];
        let roster = timer_roster(&wl, false, true, true, &main_ordered, &[], 10);
        assert_eq!(roster, vec![30, 10]);
    }

    // 専用OFF・同期ON・追従OFF: 顔ぶれはメイン−excludedと同じだが、並びは自分が先頭固定＋
    // 残りは uid 昇順の安定順（live DPS 順を無視）。
    #[test]
    fn test_timer_roster_sync_on_follow_off_uses_stable_local_first_order() {
        let wl = wl_with(&[], &[]);
        let main_ordered = vec![30, 10, 20]; // 仮にDPS順でシャッフルされていても無視される
        let roster = timer_roster(&wl, false, true, false, &main_ordered, &[], 10);
        // 自分(10)が先頭、残り(20,30)はuid昇順
        assert_eq!(roster, vec![10, 20, 30]);
    }

    // 専用OFF・同期OFF: 手動ウォッチ(watched)のみ。メイン順があれば追従させる。
    #[test]
    fn test_timer_roster_manual_uses_watched_ordered_by_main() {
        let wl = wl_with(&[10, 20], &[]);
        let main_ordered = vec![20, 10, 99];
        let roster = timer_roster(&wl, false, false, true, &main_ordered, &[], 10);
        assert_eq!(roster, vec![20, 10]);
    }

    // 専用モードは上限(watchlist::MAX)を超えない。
    #[test]
    fn test_timer_roster_imagine_only_respects_max_limit() {
        let wl = wl_with(&[], &[]);
        let buff_tracked: Vec<i64> = (1..=(watchlist::MAX as i64 + 10)).collect();
        let roster = timer_roster(&wl, true, false, true, &[], &buff_tracked, 0);
        assert_eq!(roster.len(), watchlist::MAX);
    }
}
