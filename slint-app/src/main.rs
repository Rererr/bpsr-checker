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

/// 最小ロガー。core の capture / 本体の診断ログを stderr へ出す。
/// （Slint/parley の CJK 警告は log ではなく直接 eprintln のため別物・ここでは触れない）
struct ConsoleLog;
impl log::Log for ConsoleLog {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, r: &log::Record) {
        eprintln!("[{}] {}: {}", r.level(), r.target(), r.args());
    }
    fn flush(&self) {}
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
        let food_act = p.food_duration_ms > 0.0 && p.food_remaining_ms > 0.0;
        let food_remaining = if food_act {
            (p.food_remaining_ms / p.food_duration_ms).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let food_time = if food_act {
            format::format_consumable_remaining(p.food_remaining_ms as i64, p.food_duration_ms as i64)
        } else {
            String::new()
        };
        let food_label = if food_act {
            consumable_names::label(p.food_base_id).unwrap_or_default()
        } else {
            String::new()
        };
        let syrup_act = p.syrup_duration_ms > 0.0 && p.syrup_remaining_ms > 0.0;
        let syrup_remaining = if syrup_act {
            (p.syrup_remaining_ms / p.syrup_duration_ms).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let syrup_time = if syrup_act {
            format::format_consumable_remaining(p.syrup_remaining_ms as i64, p.syrup_duration_ms as i64)
        } else {
            String::new()
        };
        let syrup_label = if syrup_act {
            consumable_names::label(p.syrup_base_id).unwrap_or_default()
        } else {
            String::new()
        };
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
            let (en, ec) = format::element_label(s.element);
            SkillRowUi {
                uid_str: format!("{}", s.uid as i64).into(),
                name: s.name.clone().into(),
                elem_text: en.into(),
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
        out.push(("その他".to_string(), rgb(OTHER_SLICE_COLOR), other));
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
            let (en, ec) = format::element_label(s.element);
            let uid = s.uid as i64;
            ResultSkillRowUi {
                uid_str: format!("{uid}").into(),
                name: s.name.clone().into(),
                elem_text: en.into(),
                elem_color: ec,
                dmg_text: format::format_number(s.total_value).into(),
                pct_text: format::format_pct(s.value_pct).into(),
                selected: uid == selected_skill_uid,
            }
        })
        .collect()
}

/// 区間DPS の折れ線を「時間軸」で配置（x = t_ms/duration）。結果画面の X軸時間ラベルと整合させる。
/// 途中参戦スキルも実時間の位置に描かれる（index 等間隔だと全幅に伸びて誤解を招くため）。
fn build_spark_dps_time(
    points: &[bpsr_core::models::TimeSeriesPoint],
    duration_ms: f64,
    vw: f32,
    vh: f32,
) -> String {
    if points.len() < 2 {
        return String::new();
    }
    let max = points.iter().map(|p| p.total_dps).fold(1.0_f64, f64::max);
    let dur = duration_ms.max(1.0);
    let mut s = String::with_capacity(points.len() * 12);
    for (i, p) in points.iter().enumerate() {
        let x = (p.t_ms / dur).clamp(0.0, 1.0) as f32 * vw;
        let y = vh - (p.total_dps / max) as f32 * vh;
        if i == 0 {
            s.push_str(&format!("M {x:.1} {y:.1}"));
        } else {
            s.push_str(&format!(" L {x:.1} {y:.1}"));
        }
    }
    s
}

/// 選択キャラの区間DPS折れ線（エリア1）。snap の player_rows[uid] の time_series から。
fn build_char_spark(snap: &bpsr_core::models::EncounterSnapshot, uid: i64) -> String {
    snap.player_rows
        .iter()
        .find(|p| p.uid as i64 == uid)
        .map(|p| build_spark_dps_time(&p.time_series, snap.duration_ms, 100.0, 16.0))
        .unwrap_or_default()
}

/// 選択スキルの区間DPS折れ線（エリア4）。duration は計測全体（snap.duration_ms）で統一。
fn build_skill_spark(
    skills: &[bpsr_core::models::SkillRow],
    selected_skill_uid: i64,
    duration_ms: f64,
) -> String {
    skills
        .iter()
        .find(|s| s.uid as i64 == selected_skill_uid)
        .map(|s| build_spark_dps_time(&s.time_series, duration_ms, 100.0, 16.0))
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
) {
    result_skill_rows.set_vec(build_result_skill_rows(skills, selected_skill_uid));
    let spark = build_skill_spark(skills, selected_skill_uid, duration_ms);
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
) {
    selected_player.set(uid);
    result_rows.set_vec(build_result_rows(snap, uid, privacy));
    // エリア1: 選択キャラの累積合計ダメージ折れ線
    let char_spark = build_char_spark(snap, uid);
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
    m.set_result_pie_title(format!("{pname} のスキル内訳").into());
    m.set_result_char_name(pname.clone().into()); // エリア1 折れ線ラベル
    // エリア2/3: スキル内訳。既定選択=先頭(最大)。
    let empty = Vec::new();
    let skills = captured.get(&uid).unwrap_or(&empty);
    let default_skill = skills.first().map(|s| s.uid as i64).unwrap_or(0);
    selected_skill.set(default_skill);
    apply_result_skill_selection(m, skills, default_skill, result_skill_rows, snap.duration_ms);
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
        (None, Some(_)) => "（名前未解決）".into(),
        (None, None) => "（未設定）".into(),
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
    m.set_inspected_name(sw.inspected_player.name.clone().into());
    sk_rows.set_vec(build_skill_rows(sw));
    m.set_skills_clickable(clickable);
    m.set_view(1);
}

/// 設定の表示系を UI へ反映（列フラグ・自分強調・最前面・パネルのトグル状態）。
fn apply_settings(m: &MainWindow, c: &settings::Settings) {
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
    m.set_font_scale((c.font_size / 12.0) as f32);
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
        buff_overlay: c.show_buff_overlay,
        imagine_only: c.imagine_only_mode,
        aot: c.always_on_top,
        three_min_auto_open: c.three_min_auto_open,
        compact_split: c.compact_split_mode,
        header_sparkline: c.show_header_sparkline,
        graph_for_local: c.graph_for_local_player,
        startup_tab: c.startup_tab.clone().into(),
    });
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
    });
}

/// テンプレートのプレビュー（固定サンプル行で name/copy 両テンプレを展開）。
fn template_previews(c: &settings::Settings) -> (slint::SharedString, slint::SharedString) {
    let name = format::format_row_name(
        "Sample",
        "ストームブレイド",
        "雷刃型",
        12345.0,
        38.0,
        8200.0,
        1,
        &c.name_template,
        c.abbreviate_scores,
    );
    let copy = format::format_row_template(
        &format::CopyRowData {
            rank: 1,
            name: "Sample",
            class_name: "ストームブレイド",
            class_spec_name: "雷刃型",
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

fn build_buff_rows(
    tracked: &bpsr_core::models::TrackedBuffsData,
    watched: &[i64],
) -> Vec<BuffPlayerRow> {
    watched
        .iter()
        .map(|&uid| {
            let snap = tracked.players.iter().find(|p| p.uid as i64 == uid);
            let name = snap.map(|s| s.name.clone()).unwrap_or_default();
            let display = if name.is_empty() {
                format!("{}", uid & 0xffff)
            } else {
                name
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    static LOGGER: ConsoleLog = ConsoleLog;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);

    if already_running() {
        log::warn!("another instance is already running; exiting");
        return Ok(());
    }

    // winit backend（透明合成可能＋タスクバー非表示）
    let backend = i_slint_backend_winit::Backend::builder()
        .with_window_attributes_hook(|attrs| {
            let attrs = attrs.with_transparent(true);
            #[cfg(target_os = "windows")]
            let attrs = {
                use i_slint_backend_winit::winit::platform::windows::WindowAttributesExtWindows;
                attrs.with_skip_taskbar(true)
            };
            attrs
        })
        .build()?;
    slint::platform::set_platform(Box::new(backend)).map_err(|e| format!("set_platform: {e:?}"))?;

    // 永続キャッシュ初期化
    let dir = data_dir();
    engine::name_cache::init(dir.join("name_cache.json"));
    engine::selected_uid::init(dir.join("selected_uid.json"));

    // 共有エンカウンター＋パケット観測スレッド
    // BPSR_DEMO=1 のときは観測の代わりに合成データを流す（撮影・UI確認用）
    let demo_mode = std::env::var("BPSR_DEMO").is_ok_and(|v| v == "1");
    let enc = Arc::new(EncounterMutex::default());
    if let Some(uid) = engine::selected_uid::get() {
        if let Ok(mut e) = enc.lock() {
            e.local_player_uid = uid;
        }
    }
    if demo_mode {
        bpsr_core::engine::demo::spawn(enc.clone());
    } else {
        capture::spawn(enc.clone());
    }

    // 設定（%APPDATA%\bpsr-checker\settings.json）。UIスレッドで共有・編集する。
    let cfg = Rc::new(RefCell::new(settings::load()));
    // ウォッチリスト（バフタイマー追跡対象）。
    let wl = Rc::new(RefCell::new(watchlist::Watchlist::load()));

    // メイン窓
    let main = MainWindow::new()?;
    // 言語選択（既定は日本語。将来は設定から）。最初のコンポーネント生成後に呼ぶ。
    let _ = slint::select_bundled_translation("ja");
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

    // 自キャラUID 候補モデル
    let uid_candidates = Rc::new(VecModel::<UidCandidate>::default());
    main.set_uid_candidates(uid_candidates.clone().into());

    // 自キャラ バフ/デバフ オーバーレイ（別ウィンドウ）
    let self_overlay = SelfStatusOverlay::new()?;
    let self_buffs = Rc::new(VecModel::<StatusEntryUi>::default());
    let self_debuffs = Rc::new(VecModel::<StatusEntryUi>::default());
    self_overlay.set_buffs(self_buffs.clone().into());
    self_overlay.set_debuffs(self_debuffs.clone().into());
    {
        let w = self_overlay.as_weak();
        self_overlay.on_start_drag(move || {
            if let Some(o) = w.upgrade() {
                overlay::start_drag(o.window());
            }
        });
    }
    {
        let w = self_overlay.as_weak();
        self_overlay.on_start_resize(move |dir| {
            if let Some(o) = w.upgrade() {
                overlay::start_resize(o.window(), dir);
            }
        });
    }

    // バフタイマー オーバーレイ（別ウィンドウ）
    let buff_overlay = BuffOverlay::new()?;
    let buff_players = Rc::new(VecModel::<BuffPlayerRow>::default());
    buff_overlay.set_players(buff_players.clone().into());
    {
        let w = buff_overlay.as_weak();
        buff_overlay.on_start_drag(move || {
            if let Some(o) = w.upgrade() {
                overlay::start_drag(o.window());
            }
        });
    }
    {
        let w = buff_overlay.as_weak();
        buff_overlay.on_start_resize(move |dir| {
            if let Some(o) = w.upgrade() {
                overlay::start_resize(o.window(), dir);
            }
        });
    }

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
                    apply_player_rows(
                        &rows_sel,
                        &cl_sel,
                        &cr_sel,
                        build_rows(
                            &fetch_players(&enc_sel, n),
                            &c.name_template,
                            c.abbreviate_scores,
                            c.privacy_mask_names,
                            &wl_sel.borrow().watched,
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
            {
                let mut wl = wl_t.borrow_mut();
                wl.toggle(uid);
                wl.save();
            }
            if w.upgrade().is_some() {
                let c = cfg_t.borrow();
                let pw = fetch_players(&enc_t, tab_t.get());
                apply_player_rows(
                    &rows_t,
                    &cl_t,
                    &cr_t,
                    build_rows(
                        &pw,
                        &c.name_template,
                        c.abbreviate_scores,
                        c.privacy_mask_names,
                        &wl_t.borrow().watched,
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
        main.on_set_bool(move |key, val| {
            {
                let mut c = cfg_b.borrow_mut();
                match key.as_str() {
                    "self-status-overlay" => c.show_self_status_overlay = val,
                    "buff-overlay" => c.show_buff_overlay = val,
                    // 専用モードON時はイマジンタイマーを強制表示（旧UIと同挙動）
                    "imagine-only" => {
                        c.imagine_only_mode = val;
                        if val {
                            c.show_buff_overlay = true;
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
                    "header-sparkline" => c.show_header_sparkline = val,
                    "graph-for-local" => c.graph_for_local_player = val,
                    other => log::warn!("unknown setting key: {other}"),
                }
            }
            let c = cfg_b.borrow();
            if let Some(m) = w.upgrade() {
                apply_settings(&m, &c);
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
            if key.as_str() == "buff-overlay" {
                if let Some(o) = buff_ov.upgrade() {
                    if c.show_buff_overlay {
                        let _ = o.show();
                    } else {
                        let _ = o.hide();
                    }
                }
            }
            if key.as_str() == "imagine-only" {
                compute::set_imagine_only_mode(&enc_sb, c.imagine_only_mode);
                // 専用モードON時は強制表示にした buff overlay を実際に出す
                if c.show_buff_overlay {
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
    // 数値設定ステッパー（key と方向 dir=±1）。キー毎に step/範囲を持ち、必要なら即適用。
    // poll-interval はポーリングタイマー再構築が要るため次回起動時に反映（永続化のみ）。
    {
        let w = main.as_weak();
        let cfg_n = cfg.clone();
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
                    other => log::warn!("unknown num key: {other}"),
                }
            }
            let c = cfg_n.borrow();
            if let Some(m) = w.upgrade() {
                apply_settings(&m, &c);
            }
            settings::save(&c);
        });
    }
    // テンプレ編集（edited）。cfg と preview のみ更新（value は push しない＝入力中クロバー防止）。
    {
        let w = main.as_weak();
        let cfg_s = cfg.clone();
        main.on_set_str(move |key, val| {
            {
                let mut c = cfg_s.borrow_mut();
                match key.as_str() {
                    "name-template" => c.name_template = val.to_string(),
                    "copy-template" => c.copy_template = val.to_string(),
                    "startup-tab" => c.startup_tab = val.to_string(),
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
        main.on_reset_encounter(move || compute::reset_encounter(&enc_r));
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
        main.on_select_result_skill(move |uid_str| {
            let skill_uid: i64 = uid_str.as_str().parse().unwrap_or(0);
            sel_s.set(skill_uid);
            let captured = cs.borrow();
            let empty = Vec::new();
            let skills = captured.get(&sel_p.get()).unwrap_or(&empty);
            if let Some(m) = w.upgrade() {
                let dur = m.get_result_duration_ms() as f64;
                apply_result_skill_selection(&m, skills, skill_uid, &rsr, dur);
            }
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

    main.show()?;
    if cfg.borrow().show_self_status_overlay {
        let _ = self_overlay.show();
    }
    if cfg.borrow().show_buff_overlay {
        let _ = buff_overlay.show();
    }

    // 周期ポーリング＋初回セットアップ（位置復元）＋自動保存
    let main_w = main.as_weak();
    let enc_poll = enc.clone();
    let saved = window_state::load();
    let last_saved = Rc::new(RefCell::new(saved.clone()));
    let mut tick: u64 = 0;
    let mut setup_tick: u64 = 0;
    let mut setup_done = false;
    // オーバーレイ復元tick（None=未復元）。復元前のデフォルト位置で保存上書きしないよう、
    // 復元から SETTLE_TICKS 経過後に保存対象へ含める。非表示で None に戻す。
    let mut self_rtick: Option<u64> = None;
    let mut buff_rtick: Option<u64> = None;
    // 復元が実際に適用した（クランプ後の）矩形。settle 期間中はこのサイズを
    // 毎tick 再適用して、Slint の preferred 再アサートによる上書きを打ち消す。
    let mut restored_main: Option<window_state::WinRect> = None;
    let mut restored_self: Option<window_state::WinRect> = None;
    let mut restored_buffs: Option<window_state::WinRect> = None;
    let tab_cell_poll = tab_cell.clone();
    let drill_poll = drill.clone();
    let skill_rows_poll = skill_rows.clone();
    let self_overlay_w = self_overlay.as_weak();
    let self_buffs_poll = self_buffs.clone();
    let self_debuffs_poll = self_debuffs.clone();
    let buff_overlay_w = buff_overlay.as_weak();
    let buff_players_poll = buff_players.clone();
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
    let compact_left_poll = compact_left.clone();
    let compact_right_poll = compact_right.clone();
    let uid_candidates_poll = uid_candidates.clone();
    // タスクトレイ／クリックスルー状態（poll closure が move で保持）
    let click_through = Rc::new(Cell::new(false));
    let main_visible = Rc::new(Cell::new(true));
    #[cfg(windows)]
    let tray_holder: Rc<RefCell<Option<tray::Tray>>> = Rc::new(RefCell::new(None));
    let poll_ms = cfg.borrow().poll_interval_ms.max(50.0) as u64;

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(poll_ms), move || {
        tick += 1;
        let Some(m) = main_w.upgrade() else {
            return;
        };

        // 初回: winit 実体化後に位置復元
        if !setup_done {
            let mons = overlay::monitors(m.window());
            if !mons.is_empty() {
                restored_main =
                    Some(window_state::restore(m.window(), saved.main.as_ref(), &mons, 0, (520, 350)));
                setup_done = true;
                setup_tick = tick;
                log::info!("window restored on {} monitor(s)", mons.len());
                // イベントループ稼働後にトレイを生成
                #[cfg(windows)]
                {
                    *tray_holder.borrow_mut() = tray::create();
                    log::info!("tray created: {}", tray_holder.borrow().is_some());
                }
            }
        }

        // オーバーレイの位置/サイズ復元（表示された最初のtickで一度）。非表示で None に戻す。
        {
            let c = cfg_poll.borrow();
            if c.show_self_status_overlay {
                if self_rtick.is_none() {
                    if let Some(o) = self_overlay_w.upgrade() {
                        let mons = overlay::monitors(o.window());
                        if !mons.is_empty() {
                            restored_self = Some(window_state::restore(
                                o.window(),
                                last_saved.borrow().self_status.as_ref(),
                                &mons,
                                0,
                                (220, 180),
                            ));
                            self_rtick = Some(tick);
                        }
                    }
                }
            } else {
                self_rtick = None;
                restored_self = None;
            }
            if c.show_buff_overlay {
                if buff_rtick.is_none() {
                    if let Some(o) = buff_overlay_w.upgrade() {
                        let mons = overlay::monitors(o.window());
                        if !mons.is_empty() {
                            restored_buffs = Some(window_state::restore(
                                o.window(),
                                last_saved.borrow().buffs.as_ref(),
                                &mons,
                                0,
                                (250, 150),
                            ));
                            buff_rtick = Some(tick);
                        }
                    }
                }
            } else {
                buff_rtick = None;
                restored_buffs = None;
            }
        }

        // トレイメニューのイベント処理（クリックスルー切替・表示/非表示・終了）
        #[cfg(windows)]
        {
            let holder = tray_holder.borrow();
            if let Some(tray) = holder.as_ref() {
                while let Ok(ev) = tray_icon::menu::MenuEvent::receiver().try_recv() {
                    if ev.id == tray.id_quit {
                        let _ = slint::quit_event_loop();
                    } else if ev.id == tray.id_show_hide {
                        let vis = !main_visible.get();
                        main_visible.set(vis);
                        let _ = if vis { m.show() } else { m.hide() };
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
                    }
                }
            }
        }

        // 食事/シロップ残時間ストアを更新（戦闘終了をまたいで保持・失効除去）
        compute::refresh_consumables(&enc_poll);

        // ライブ集計を反映（共有セルの現在タブに応じて取得）
        let header = compute::get_header_info(&enc_poll);
        m.set_total_text(format::format_dps(header.total_dps).into());
        m.set_elapsed_text(format::format_elapsed(header.elapsed_ms).into());

        // ヘッダースパークライン（有効時のみ time_series を取得して折れ線へ）
        if cfg_poll.borrow().show_header_sparkline {
            let ts = compute::get_time_series(&enc_poll);
            let cmds = build_spark_commands(&ts, 100.0, 16.0);
            m.set_spark_visible(!cmds.is_empty());
            m.set_spark_commands(cmds.into());
        } else if m.get_spark_visible() {
            m.set_spark_visible(false);
        }

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
                        );
                    }
                }
            }
        }

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
            let c = cfg_poll.borrow();
            m.set_show_graph_col(graph_col_active(&c, cur_tab));
            apply_player_rows(
                &rows,
                &compact_left_poll,
                &compact_right_poll,
                build_rows(
                    &pw,
                    &c.name_template,
                    c.abbreviate_scores,
                    c.privacy_mask_names,
                    &wl_poll.borrow().watched,
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

        // オーバーレイの文字サイズをメインの font_size に連動
        let overlay_scale = (cfg_poll.borrow().font_size / 12.0) as f32;

        // 自キャラ オーバーレイ更新（表示中のみ）
        if cfg_poll.borrow().show_self_status_overlay {
            if let Some(o) = self_overlay_w.upgrade() {
                o.set_font_scale(overlay_scale);
                let s = compute::get_self_buff_status(&enc_poll);
                o.set_waiting(s.local_player_uid == 0.0);
                self_buffs_poll.set_vec(build_status_entries(&s.buffs));
                self_debuffs_poll.set_vec(build_status_entries(&s.debuffs));
            }
        }

        // バフタイマー オーバーレイ更新（表示中のみ）
        if cfg_poll.borrow().show_buff_overlay {
            if let Some(o) = buff_overlay_w.upgrade() {
                o.set_font_scale(overlay_scale);
                let watched_i: Vec<i64> = wl_poll.borrow().watched.clone();
                o.set_empty(watched_i.is_empty());
                if !watched_i.is_empty() {
                    let uids: Vec<f64> = watched_i.iter().map(|&u| u as f64).collect();
                    let t = compute::get_tracked_buffs(&enc_poll, uids);
                    buff_players_poll.set_vec(build_buff_rows(&t, &watched_i));
                }
            }
        }

        // 起動/表示直後、Slint が preferred サイズを再アサートして保存サイズを
        // 上書きすることがあるため、settle 期間中は毎tick 再適用して確実に効かせる。
        // （保存ガードの return より手前に置くこと。サイズ一致時は no-op。）
        if setup_done && tick < setup_tick + SETTLE_TICKS {
            if let Some(r) = &restored_main {
                window_state::enforce_size(m.window(), r);
            }
        }
        if let (Some(rt), Some(r)) = (self_rtick, restored_self.as_ref()) {
            if tick < rt + SETTLE_TICKS {
                if let Some(o) = self_overlay_w.upgrade() {
                    window_state::enforce_size(o.window(), r);
                }
            }
        }
        if let (Some(rt), Some(r)) = (buff_rtick, restored_buffs.as_ref()) {
            if tick < rt + SETTLE_TICKS {
                if let Some(o) = buff_overlay_w.upgrade() {
                    window_state::enforce_size(o.window(), r);
                }
            }
        }

        // レイアウト自動保存（復元確定後・差分時のみ）。オーバーレイは復元から
        // SETTLE_TICKS 経過後のみ保存対象に含める（復元前の既定位置で上書き防止）。
        if !setup_done || tick < setup_tick + SETTLE_TICKS {
            return;
        }
        let settled = |rt: Option<u64>| rt.map(|t| tick >= t + SETTLE_TICKS).unwrap_or(false);
        let cur = {
            let c = cfg_poll.borrow();
            let prev = last_saved.borrow();
            window_state::Layout {
                main: Some(window_state::capture(m.window())),
                self_status: if c.show_self_status_overlay && settled(self_rtick) {
                    self_overlay_w
                        .upgrade()
                        .map(|o| window_state::capture(o.window()))
                } else {
                    prev.self_status.clone()
                },
                buffs: if c.show_buff_overlay && settled(buff_rtick) {
                    buff_overlay_w
                        .upgrade()
                        .map(|o| window_state::capture(o.window()))
                } else {
                    prev.buffs.clone()
                },
            }
        };
        let mut ls = last_saved.borrow_mut();
        if *ls != cur {
            window_state::save(&cur);
            *ls = cur;
        }
    });

    slint::run_event_loop()?;

    engine::name_cache::flush();
    engine::selected_uid::flush();
    Ok(())
}
