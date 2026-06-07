//! 検証用のダミーデータ生成。tick（200ms 間隔）で時間変化を作る。

use crate::{BuffTimer, PlayerRow};
use slint::SharedString;

// NOTE: ラベルは ASCII。Slint/parley は CJK の行分割セグメンテーションモデルを
// 同梱せず、CJK 文字列のレイアウト毎に "No segmentation model for language: ja" を
// 直接 stderr へ出力してコンソールを埋める（描画自体は正常）。検証ログを読みやすく
// 保つため PoC では ASCII を用いる。CJK グリフ描画を見たい場合はここを日本語に戻す。
const CLASSES: [&str; 8] = [
    "Blade", "Ranger", "Guard", "Cleric", "Frost", "Bolt", "Lance", "Twin",
];

fn fmt_dps(v: f32) -> SharedString {
    if v >= 1000.0 {
        format!("{:.1}k", v / 1000.0).into()
    } else {
        format!("{:.0}", v).into()
    }
}

/// 20 人分の DPS 行。値は揺らぎを持たせ、毎 tick で並びも変動する。
pub fn players(tick: u64) -> Vec<PlayerRow> {
    let n = 20usize;
    let t = tick as f32;
    let mut raw: Vec<(f32, usize)> = (0..n)
        .map(|i| {
            let base = 120_000.0 / (i as f32 + 1.5);
            let jitter = (t * 0.13 + i as f32 * 1.7).sin() * 0.4 + 1.0; // 0.6..1.4
            (base * jitter, i)
        })
        .collect();
    raw.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let top = raw.first().map(|x| x.0).unwrap_or(1.0).max(1.0);
    raw.into_iter()
        .map(|(dps, i)| {
            let pct = (dps / top) * 100.0;
            PlayerRow {
                name: format!("Player{:02}", i + 1).into(),
                class_name: CLASSES[i % CLASSES.len()].into(),
                dps_text: fmt_dps(dps),
                pct,
                pct_text: format!("{:.0}%", pct).into(),
            }
        })
        .collect()
}

/// 残量比 progress(0..1) から円形ゲージ用の SVG パスを算出。
/// viewbox 0..100、中心(50,50)、半径45、上端(50,5)から時計回り。
fn arc_commands(progress: f32) -> SharedString {
    let p = progress.clamp(0.0, 0.9999);
    let theta = p * std::f32::consts::TAU; // 0..2π
    let end_x = 50.0 + 45.0 * theta.sin();
    let end_y = 50.0 - 45.0 * theta.cos();
    let large = if p > 0.5 { 1 } else { 0 };
    format!("M 50 5 A 45 45 0 {large} 1 {end_x:.2} {end_y:.2}").into()
}

fn timer(name: &str, kind: &str, dur: f32, t: f32, offset: f32) -> BuffTimer {
    let phase = (t + offset).rem_euclid(dur);
    let remaining = dur - phase; // dur → 0 を周期的に
    let progress = (remaining / dur).clamp(0.0, 1.0);
    BuffTimer {
        name: name.into(),
        kind: kind.into(),
        progress,
        seconds_text: format!("{:.1}", remaining).into(),
        arc_commands: arc_commands(progress),
    }
}

/// (bars, circles) を返す。bars=デバフ(棒)、circles=バフ(円形)。
pub fn buffs(tick: u64) -> (Vec<BuffTimer>, Vec<BuffTimer>) {
    let t = tick as f32 * 0.2; // 秒

    let circles = vec![
        timer("Atk+", "buff", 8.0, t, 0.0),
        timer("Crit+", "buff", 5.0, t, 1.3),
        timer("Spd+", "buff", 6.5, t, 3.1),
    ];
    let bars = vec![
        timer("Vuln", "debuff", 12.0, t, 0.0),
        timer("Silence", "debuff", 4.0, t, 2.2),
    ];
    (bars, circles)
}
