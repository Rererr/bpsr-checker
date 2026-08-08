//! DPS一覧の行バー幅比率モード（Issue #5 由来。他メーターの模倣ではない独自実装）。
//! 4方式（トップ比/全体比/固定基準/自分基準）の判定・計算をここへ1本化し、
//! main.rs 側でモード文字列の比較が散らばらないようにする。
//! 数値設定の検証（正の有限値／窓秒のクランプ範囲）も settings::load() と main.rs
//! on_set_str の双方から共用する（判定を1箇所へ集約し、乖離を防ぐ）。

use bpsr_core::models::{PlayerRow, TimeSeriesPoint};

pub const MODE_TOP: &str = "top";
pub const MODE_SHARE: &str = "share";
pub const MODE_FIXED: &str = "fixed";
pub const MODE_SELF: &str = "self";

/// バー幅比率の基準モード。settings.rs の `dps_bar_mode`（生文字列・既定 "top"）を
/// パースした表現。未知の文字列は Top へフォールバックする（parse() 参照）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DpsBarMode {
    /// トップ比（既定・現行）: pct = total_value / top * 100
    Top,
    /// 全体比: pct = value_pct（パーティ全体に対する貢献シェア%。pct_text 列と同じ値）
    Share,
    /// 固定基準: 右端をユーザー指定の基準DPS値とし、直近 window_secs 秒の平均DPSで伸縮
    Fixed,
    /// 自分基準: 自キャラのバーを常に50%位置に固定し、他メンバーは自分の2倍で右端
    SelfRelative,
}

impl DpsBarMode {
    /// 設定文字列 → モード。未知の値（旧設定ファイルの破損等を含む）は既定の Top。
    pub fn parse(s: &str) -> Self {
        match s {
            MODE_SHARE => Self::Share,
            MODE_FIXED => Self::Fixed,
            MODE_SELF => Self::SelfRelative,
            _ => Self::Top,
        }
    }

    /// モード → 正規化済み設定文字列（parse の逆）。settings::load() で不正値を
    /// 書き戻す際に使う（未知値は Top を経由するため必ず "top" に正規化される）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Top => MODE_TOP,
            Self::Share => MODE_SHARE,
            Self::Fixed => MODE_FIXED,
            Self::SelfRelative => MODE_SELF,
        }
    }
}

pub const INTENSITY_NONE: &str = "none";
pub const INTENSITY_SUBTLE: &str = "subtle";
pub const INTENSITY_STRONG: &str = "strong";

/// バー濃度。settings.rs の `dps_bar_intensity`（生文字列・既定 "subtle"）をパースした表現。
/// 未知の文字列は Subtle（既定・現行の見た目相当）へフォールバックする（parse() 参照）。
/// 値自体の描画（グラデーション alpha・エッジ線の有無）は app.slint 側の導出プロパティに
/// 一本化し、判定文字列がそこかしこに散らばらないようにする（DpsBarMode と同じ方針）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarIntensity {
    /// なし: バー非表示（行背景のみ）
    None,
    /// 控えめ（既定・現行）: alpha 0.22 グラデーション、終端は transparent
    Subtle,
    /// くっきり: 終端まで視認できる alpha ＋ クラス色のエッジ線
    Strong,
}

impl BarIntensity {
    /// 設定文字列 → 濃度。未知の値は既定の Subtle。
    pub fn parse(s: &str) -> Self {
        match s {
            INTENSITY_NONE => Self::None,
            INTENSITY_STRONG => Self::Strong,
            _ => Self::Subtle,
        }
    }

    /// 濃度 → 正規化済み設定文字列（parse の逆）。settings::load() で不正値を書き戻す際に使う。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => INTENSITY_NONE,
            Self::Subtle => INTENSITY_SUBTLE,
            Self::Strong => INTENSITY_STRONG,
        }
    }
}

/// バー幅比率の計算に必要な設定値一式（Settings から都度構築する軽量なコピー）。
#[derive(Debug, Clone, Copy)]
pub struct DpsBarConfig {
    pub mode: DpsBarMode,
    /// 固定基準モードの右端DPS値 k（正の有限値。settings.rs の set-str 側で検証済み）。
    pub fixed_max: f64,
    /// 固定基準モードの平均窓秒 s。
    pub window_secs: f64,
}

/// 値が正の有限値か（固定基準の右端DPS・平均窓秒の入力検証で共用）。
pub fn is_positive_finite(v: f64) -> bool {
    v > 0.0 && v.is_finite()
}

/// 平均窓秒の許容上限（秒）。グラフの保持期間（サンプル数×間隔）を超えて指定しても
/// windowed_avg_dps が黙って全期間平均にフォールバックし、指定値と実効値が乖離するため、
/// 保持期間そのものを上限として採用する。
pub fn max_window_secs(ts_samples: f64, ts_interval_ms: f64) -> f64 {
    (ts_samples.max(1.0) * ts_interval_ms.max(1.0) / 1000.0).max(1.0)
}

/// 平均窓秒を許容範囲（1秒〜グラフの保持期間）へクランプする。
pub fn clamp_window_secs(v: f64, ts_samples: f64, ts_interval_ms: f64) -> f64 {
    v.clamp(1.0, max_window_secs(ts_samples, ts_interval_ms))
}

/// 直近 `window_secs` 秒間の平均DPSを時系列点（古い→新しい順）から導出する。
///
/// 手順: 最新点を t_last とし、`t_ms >= t_last - window_secs*1000`（＝窓に入っている）を
/// 満たす最も古い点を基準点とする。該当点が無ければ次の2通りに分かれる:
/// - 戦闘経過が window_secs 未満（窓が記録全体を覆う）: 最古の記録点を基準点にする＝
///   結果的に実際の経過時間で割ることになる。
/// - 窓がサンプル間隔より短い（窓内に最新点しか無い）: 直前の点を基準にする＝
///   最古点まで遡って保持期間全体の平均に劣化するのを防ぐ。
/// いずれも `series[..series.len() - 1]` の末尾要素（＝最新点の直前点）へのフォールバックで
/// 両ケースを同時に満たす（前者は find が最古点をそのまま返すため到達しない）。
/// （窓境界ちょうどの点を含めないと実効窓がサンプル1個分長くなるため、境界は `>=` で含める）
/// 基準点との時間差が0以下（同一タイムスタンプ等）になる場合や点が1つ以下の場合は
/// None を返す（呼び出し側で value_per_sec へフォールバックする想定）。
pub fn windowed_avg_dps(series: &[TimeSeriesPoint], window_secs: f64) -> Option<f64> {
    if series.len() <= 1 {
        return None;
    }
    let last = series.last()?;
    let cutoff = last.t_ms - window_secs.max(0.0) * 1000.0;
    let prior = &series[..series.len() - 1];
    let base = prior.iter().find(|p| p.t_ms >= cutoff).unwrap_or(&prior[prior.len() - 1]);
    let dt_secs = (last.t_ms - base.t_ms) / 1000.0;
    if dt_secs <= 0.0 {
        return None;
    }
    Some(((last.total_dmg - base.total_dmg) / dt_secs).max(0.0))
}

fn top_pct(total_value: f64, top: f64) -> f32 {
    ((total_value / top.max(1.0)) * 100.0) as f32
}

/// 行バーの幅比率(0..100、自分基準のみ200%ぶんの値域を50%に圧縮)を計算する。
///
/// - Top: 現行どおり total_value / top * 100
/// - Share: value_pct をそのまま使う（pct_text 列と同じ値。バー幅のみ変わる）
/// - Fixed: 直近 window_secs 秒の平均DPS（導出不能なら value_per_sec）を fixed_max で
///   正規化し 0..100 にクランプ。`p.time_series` は呼び出し元(build_rows)が現在タブと
///   一致する指標（与ダメ/回復/被ダメ）の系列を渡す前提のため、ここでタブ判定は不要
///   （旧実装は time_series が常に与ダメ由来だったため dmg_tab 特別扱いで回避していたが、
///   core 側が指標別に系列を持つようになり不要化）。
/// - SelfRelative: 自キャラの total_value（self_total）を基準に 50%位置固定。
///   自キャラ不在・0以下（self_total = None）なら Top へフォールバック
pub fn bar_pct(cfg: &DpsBarConfig, p: &PlayerRow, top: f64, self_total: Option<f64>) -> f32 {
    match cfg.mode {
        DpsBarMode::Top => top_pct(p.total_value, top),
        DpsBarMode::Share => (p.value_pct as f32).clamp(0.0, 100.0),
        DpsBarMode::Fixed => {
            let avg = windowed_avg_dps(&p.time_series, cfg.window_secs).unwrap_or(p.value_per_sec);
            let k = if is_positive_finite(cfg.fixed_max) { cfg.fixed_max } else { 1.0 };
            ((avg / k).clamp(0.0, 1.0) * 100.0) as f32
        }
        DpsBarMode::SelfRelative => match self_total {
            Some(s) if s > 0.0 => ((p.total_value / s * 50.0).clamp(0.0, 100.0)) as f32,
            _ => top_pct(p.total_value, top),
        },
    }
}

/// 数値入力欄の表示用フォーマット。整数値は小数点無しで表示し、端数がある場合のみ残す
/// （ユーザーが打った桁を尊重しつつ、既定値 100000/10 は "100000"/"10" と素直に出す）。
pub fn format_bar_num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(t_ms: f64, total_dmg: f64) -> TimeSeriesPoint {
        TimeSeriesPoint { t_ms, total_dmg, total_dps: 0.0 }
    }

    fn player(total_value: f64, value_pct: f64, value_per_sec: f64, time_series: Vec<TimeSeriesPoint>) -> PlayerRow {
        PlayerRow { total_value, value_pct, value_per_sec, time_series, ..Default::default() }
    }

    #[test]
    fn windowed_avg_dps_empty_series_is_none() {
        assert_eq!(windowed_avg_dps(&[], 10.0), None);
    }

    #[test]
    fn windowed_avg_dps_single_point_is_none() {
        let series = [pt(0.0, 1000.0)];
        assert_eq!(windowed_avg_dps(&series, 10.0), None);
    }

    #[test]
    fn windowed_avg_dps_short_combat_uses_full_elapsed() {
        // 戦闘経過が window_secs(10s) 未満 → 基準点は最古の記録点(t=0)になり、
        // 結果的に実際の経過時間(3s)で割った値になる。
        let series = [pt(0.0, 0.0), pt(1000.0, 3000.0), pt(3000.0, 9000.0)];
        // (9000-0) / (3000/1000) = 3000
        assert_eq!(windowed_avg_dps(&series, 10.0), Some(3000.0));
    }

    #[test]
    fn windowed_avg_dps_uses_point_within_window() {
        // t=0..12000ms、window=5s → cutoff=7000。t_ms>=7000 の最古点は t=8000。
        let series = [
            pt(0.0, 0.0),
            pt(2000.0, 2000.0),
            pt(4000.0, 4000.0),
            pt(6000.0, 6000.0),
            pt(8000.0, 8000.0),
            pt(10000.0, 10000.0),
            pt(12000.0, 12000.0),
        ];
        // base = t=8000(dmg=8000), last = t=12000(dmg=12000) → (12000-8000)/4 = 1000
        assert_eq!(windowed_avg_dps(&series, 5.0), Some(1000.0));
    }

    #[test]
    fn windowed_avg_dps_includes_exact_boundary_point() {
        // 旧実装は「cutoffより厳密に前」の点を基準にしていたため、境界ちょうどの点(t=5000)を
        // 除外し実効窓が指定(5s)よりサンプル1個分長くなっていた(0..10000=10s分を平均)。
        // 新実装は t_ms>=cutoff の最古点(t=5000)を基準にし、実際に指定通り5sぶんだけを見る。
        let series = [pt(0.0, 0.0), pt(5000.0, 1000.0), pt(10000.0, 6000.0)];
        // base=t=5000(dmg=1000), last=t=10000(dmg=6000) → (6000-1000)/5 = 1000
        assert_eq!(windowed_avg_dps(&series, 5.0), Some(1000.0));
    }

    #[test]
    fn windowed_avg_dps_shorter_than_sample_interval_uses_prior_point() {
        // 窓(1s)がサンプル間隔(5s)より短く、窓内に最新点(t=10000)しか無いケース。
        // 最古点(t=0)まで遡ると保持期間全体(10s)の平均に劣化するため、直前点(t=5000)を基準にする。
        let series = [pt(0.0, 0.0), pt(5000.0, 0.0), pt(10000.0, 5000.0)];
        // base=t=5000(dmg=0), last=t=10000(dmg=5000) → (5000-0)/5 = 1000
        assert_eq!(windowed_avg_dps(&series, 1.0), Some(1000.0));
    }

    #[test]
    fn windowed_avg_dps_zero_dt_guard() {
        // 直前点と最新点が同一タイムスタンプ → dt=0 でゼロ除算を避け None を返す。
        let series = [pt(0.0, 0.0), pt(0.0, 500.0)];
        assert_eq!(windowed_avg_dps(&series, 10.0), None);
    }

    #[test]
    fn windowed_avg_dps_never_negative() {
        // ダメージが減る(スナップショット再構築等)ことは想定しないが、防御的に0未満は出さない。
        let series = [pt(0.0, 5000.0), pt(1000.0, 1000.0)];
        assert_eq!(windowed_avg_dps(&series, 10.0), Some(0.0));
    }

    #[test]
    fn mode_parse_unknown_falls_back_to_top() {
        assert_eq!(DpsBarMode::parse("top"), DpsBarMode::Top);
        assert_eq!(DpsBarMode::parse("share"), DpsBarMode::Share);
        assert_eq!(DpsBarMode::parse("fixed"), DpsBarMode::Fixed);
        assert_eq!(DpsBarMode::parse("self"), DpsBarMode::SelfRelative);
        assert_eq!(DpsBarMode::parse("garbage"), DpsBarMode::Top);
        assert_eq!(DpsBarMode::parse(""), DpsBarMode::Top);
    }

    #[test]
    fn mode_as_str_round_trips_through_parse() {
        for m in [DpsBarMode::Top, DpsBarMode::Share, DpsBarMode::Fixed, DpsBarMode::SelfRelative] {
            assert_eq!(DpsBarMode::parse(m.as_str()), m);
        }
    }

    #[test]
    fn intensity_parse_unknown_falls_back_to_subtle() {
        assert_eq!(BarIntensity::parse("none"), BarIntensity::None);
        assert_eq!(BarIntensity::parse("subtle"), BarIntensity::Subtle);
        assert_eq!(BarIntensity::parse("strong"), BarIntensity::Strong);
        assert_eq!(BarIntensity::parse("garbage"), BarIntensity::Subtle);
        assert_eq!(BarIntensity::parse(""), BarIntensity::Subtle);
    }

    #[test]
    fn intensity_as_str_round_trips_through_parse() {
        for i in [BarIntensity::None, BarIntensity::Subtle, BarIntensity::Strong] {
            assert_eq!(BarIntensity::parse(i.as_str()), i);
        }
    }

    #[test]
    fn max_window_secs_derives_from_retention() {
        // 既定60サンプル×1000ms = 60秒。
        assert_eq!(max_window_secs(60.0, 1000.0), 60.0);
        // 極端な設定(1サンプル×1ms)でも最低1秒は確保する。
        assert_eq!(max_window_secs(1.0, 1.0), 1.0);
    }

    #[test]
    fn clamp_window_secs_respects_dynamic_upper_bound() {
        // 保持期間(60s)を超える指定は上限でクランプされる(旧固定300秒ではなく実効上限に追従)。
        assert_eq!(clamp_window_secs(10500.0, 60.0, 1000.0), 60.0);
        // 保持期間が広い設定(200サンプル×5000ms=1000秒)なら300超も通る。
        assert_eq!(clamp_window_secs(500.0, 200.0, 5000.0), 500.0);
        // 下限は1秒。
        assert_eq!(clamp_window_secs(0.0, 60.0, 1000.0), 1.0);
    }

    fn cfg(mode: DpsBarMode) -> DpsBarConfig {
        DpsBarConfig { mode, fixed_max: 100_000.0, window_secs: 10.0 }
    }

    #[test]
    fn bar_pct_top_matches_legacy_formula() {
        let p = player(5000.0, 50.0, 5000.0, vec![]);
        let pct = bar_pct(&cfg(DpsBarMode::Top), &p, 10_000.0, None);
        assert_eq!(pct, 50.0);
    }

    #[test]
    fn bar_pct_share_uses_value_pct_directly() {
        let p = player(5000.0, 33.3, 5000.0, vec![]);
        let pct = bar_pct(&cfg(DpsBarMode::Share), &p, 10_000.0, None);
        assert_eq!(pct, 33.3);
    }

    #[test]
    fn bar_pct_share_clamps_out_of_range_pct() {
        // value_pct は通常0..100だが、防御的に範囲外もクランプする。
        let over = player(0.0, 150.0, 0.0, vec![]);
        assert_eq!(bar_pct(&cfg(DpsBarMode::Share), &over, 1.0, None), 100.0);
        let under = player(0.0, -10.0, 0.0, vec![]);
        assert_eq!(bar_pct(&cfg(DpsBarMode::Share), &under, 1.0, None), 0.0);
    }

    #[test]
    fn bar_pct_fixed_normalizes_by_k_and_clamps() {
        // 窓平均が導出できない(点1つ以下)ため value_per_sec(60000) にフォールバック。
        // 60000 / 100000 * 100 = 60。
        let p = player(0.0, 0.0, 60_000.0, vec![pt(0.0, 0.0)]);
        let pct = bar_pct(&cfg(DpsBarMode::Fixed), &p, 0.0, None);
        assert_eq!(pct, 60.0);
        // 上限クランプ: k を超える場合は100で頭打ち。
        let over = player(0.0, 0.0, 200_000.0, vec![pt(0.0, 0.0)]);
        let pct_over = bar_pct(&cfg(DpsBarMode::Fixed), &over, 0.0, None);
        assert_eq!(pct_over, 100.0);
    }

    #[test]
    fn bar_pct_fixed_uses_windowed_avg_when_available() {
        // 窓平均が導出できるならそちらを優先する(value_per_secより優先度が高い)。タブに関係なく
        // 呼び出し元(build_rows)が現在タブと一致する指標の time_series を渡す前提（コメント参照）。
        let series = vec![pt(0.0, 0.0), pt(10_000.0, 100_000.0)]; // 10s平均 = 10000/s
        let p = player(0.0, 0.0, 999_999.0, series);
        let pct = bar_pct(&cfg(DpsBarMode::Fixed), &p, 0.0, None);
        // 10000 / 100000 * 100 = 10 (value_per_sec の 999999 は使われない)
        assert_eq!(pct, 10.0);
    }

    #[test]
    fn bar_pct_self_relative_places_self_at_50_and_double_at_100() {
        let c = cfg(DpsBarMode::SelfRelative);
        // 自分自身: total_value == self_total → 50%
        let self_p = player(4000.0, 0.0, 0.0, vec![]);
        assert_eq!(bar_pct(&c, &self_p, 4000.0, Some(4000.0)), 50.0);
        // 自分の2倍 → 100%（右端で頭打ち開始点）
        let double_p = player(8000.0, 0.0, 0.0, vec![]);
        assert_eq!(bar_pct(&c, &double_p, 8000.0, Some(4000.0)), 100.0);
        // 自分の3倍 → 100%でクランプ
        let triple_p = player(12000.0, 0.0, 0.0, vec![]);
        assert_eq!(bar_pct(&c, &triple_p, 8000.0, Some(4000.0)), 100.0);
    }

    #[test]
    fn bar_pct_self_relative_falls_back_to_top_when_no_self() {
        let c = cfg(DpsBarMode::SelfRelative);
        let p = player(5000.0, 0.0, 0.0, vec![]);
        // 自キャラ不在(None)・0以下は Top と同じ式へフォールバック。
        assert_eq!(bar_pct(&c, &p, 10_000.0, None), 50.0);
        assert_eq!(bar_pct(&c, &p, 10_000.0, Some(0.0)), 50.0);
    }

    #[test]
    fn format_bar_num_integer_and_fraction() {
        assert_eq!(format_bar_num(100_000.0), "100000");
        assert_eq!(format_bar_num(10.0), "10");
        assert_eq!(format_bar_num(10.5), "10.5");
    }
}
