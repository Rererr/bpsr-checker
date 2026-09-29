//! 表示整形ヘルパ（フロント src/utils.ts を移植）。

use bpsr_core::engine::class::{Class, Role};
use slint::Color;

/// 素材の有無に関わらない全 Class（Unknown は「未マッチ」の意味で使うため除外）。
const ALL_CLASSES: &[Class] = &[
    Class::Stormblade,
    Class::FrostMage,
    Class::TwinStriker,
    Class::WindKnight,
    Class::VerdantOracle,
    Class::HeavyGuardian,
    Class::Marksman,
    Class::ShieldKnight,
    Class::BeatPerformer,
    Class::Dorothy,
    Class::Lucy,
    Class::Natsu,
    Class::Unimplemented,
];

/// 表示名（ja/en 両対応。表示言語で名前が変わっても判定できるよう name_ja()/name_en() の
/// 両方と突合する）→ Class。class_color()/class_icon_id() の判定を一本化する共通入口。
fn class_of(class_name: &str) -> Option<Class> {
    ALL_CLASSES
        .iter()
        .copied()
        .find(|c| c.name_ja() == class_name || c.name_en() == class_name)
}

/// 数値を M/K 単位へ丸めて表示する。小数は **M・K とも2桁**に揃える。
///
/// K を1桁にしていると DPS が `98.3K` 刻みでしか読めず、ゲーム内の計測パネル（`92.65K` の
/// ように2桁表示）と突き合わせたときに、丸めのせいなのか実際の差なのかが判別できない。
/// 桁を揃えることで、差が出たときに原因の切り分けができるようにする。
pub fn format_number(n: f64) -> String {
    if n >= 1_000_000.0 {
        format!("{:.2}M", n / 1_000_000.0)
    } else if n >= 1_000.0 {
        format!("{:.2}K", n / 1_000.0)
    } else {
        format!("{}", n.round() as i64)
    }
}

pub fn format_dps(n: f64) -> String {
    format_number(n)
}

pub fn format_pct(n: f64) -> String {
    format!("{n:.1}%")
}

pub fn format_elapsed(ms: f64) -> String {
    let total = (ms / 1000.0).max(0.0).floor() as i64;
    format!("{}:{:02}", total / 60, total % 60)
}

pub fn format_score(n: f64, abbreviate: bool) -> String {
    if abbreviate {
        format_number(n)
    } else {
        format!("{}", n.round() as i64)
    }
}

/// クラス名 → 表示色（utils.ts CLASS_COLORS）。class.rs の name_ja()/name_en() 両方の
/// 表記を受け付ける（表示言語で名前が変わっても色は固定）。
/// ※ ドロシーは色未確定のため既定グレーへフォールバック（class_of() 経由でも据え置き）。
pub fn class_color(class_name: &str) -> Color {
    let hex: u32 = match class_of(class_name) {
        Some(Class::Stormblade) => 0xfd7cff,
        Some(Class::FrostMage) => 0x3498db,
        Some(Class::TwinStriker) => 0xe67e22,
        Some(Class::Lucy) => 0xf1c40f,
        Some(Class::Natsu) => 0xe74c3c,
        Some(Class::WindKnight) => 0xc6ffd8,
        Some(Class::VerdantOracle) => 0x139348,
        Some(Class::HeavyGuardian) => 0x724d2d,
        Some(Class::Marksman) => 0xfff090,
        Some(Class::ShieldKnight) => 0xd1a700,
        Some(Class::BeatPerformer) => 0xe91e63,
        Some(Class::Unimplemented) => 0x7f8c8d,
        _ => 0x95a5a6, // Unknown / Dorothy（色未確定）/ 未マッチ
    };
    Color::from_rgb_u8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// クラス名 → ロール表示色（職アイコンの tint 専用。名前・バー・グラフは職別の class_color() のまま）。
/// 職ごとに色を散らすとアイコンの形と色が二重の識別子になるため、アイコンはロール3色に寄せる。
/// 色相は他ツールの慣行（アタッカー=赤 e32424 / タンク=青 1188d4 / ヒーラー=緑 00cc00）に合わせつつ、
/// 明度は 0.8 倍。他ツールは tint を alpha 0.5 の乗算で当てるため、素の値では鮮やか過ぎるため。
pub fn class_role_color(class_name: &str) -> Color {
    let hex: u32 = match class_of(class_name).map(Class::role) {
        Some(Role::Attacker) => 0xb61d1d,
        Some(Role::Tank) => 0x0e6daa,
        Some(Role::Healer) => 0x00a300,
        _ => 0x95a5a6, // Unknown / 未実装 / 未マッチ
    };
    Color::from_rgb_u8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// クラス名 → 職アイコン用 ID（class.rs の profession id と一致。ClassIcon の id プロパティに渡す）。
/// 素材が無いクラス（ドロシー/ルーシィ/ナツ）・未実装/不明クラスは 0（.slint 側でフォールバック表示）。
/// 返り値は必ず ClassIcon の 9 分岐 {1,2,3,4,5,9,11,12,13} または 0（tests::class_icon_id_* で保証）。
pub fn class_icon_id(class_name: &str) -> i32 {
    const HAS_ICON: [i32; 9] = [1, 2, 3, 4, 5, 9, 11, 12, 13];
    class_of(class_name)
        .map(|c| c.profession_id())
        .filter(|id| HAS_ICON.contains(id))
        .unwrap_or(0)
}

/// 名前列テンプレートを構成する文字列または職アイコン。
///
/// `class_icon=true` の要素は文字列へ展開せず、UI 側で職アイコンとして描画する。
/// 文字列とアイコンを同じ配列で保持することで、テンプレート内の配置をそのまま
/// 描画順へ引き継ぐ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamePart {
    pub text: String,
    pub class_icon: bool,
    /// 列幅が足りないときに省略される順序。大きいほど先に削られる
    /// （[`SHRINK_RANK_BODY`] < [`SHRINK_RANK_IMAGINE`] < [`SHRINK_RANK_ROLE_SKILL`]）。
    /// UI 側はこの値を HorizontalLayout の horizontal-stretch へ写して縮小順を決める。
    pub shrink_rank: i32,
    /// `{name}` を含むパーツ。幅が尽きても最低限の文字数は残す（UI 側で min-width を与える）。
    pub has_name: bool,
}

/// 名前を含む本体（`{name}`/`{spec}`/`{score}` 等）。最後まで削らない。
pub const SHRINK_RANK_BODY: i32 = 0;
/// `{imagine}`（装備中バトルイマジン）。本体より先に削る。
pub const SHRINK_RANK_IMAGINE: i32 = 1;
/// `{roleSkill}`（ロールスキル＝簡易版バトルイマジン）。最初に削る。
pub const SHRINK_RANK_ROLE_SKILL: i32 = 2;

/// 属性 → (短い表示名, 色)（utils.ts ELEMENT_TABLE）。
pub fn element_label(e: u8) -> (&'static str, Color) {
    let (name, hex): (&str, u32) = match e {
        0 => ("物", 0xaaaaaa),
        1 => ("炎", 0xe74c3c),
        2 => ("氷", 0x4fc3f7),
        3 => ("雷", 0xf1c40f),
        4 => ("森", 0x2ecc71),
        5 => ("風", 0x1abc9c),
        6 => ("岩", 0xa0522d),
        7 => ("光", 0xecf0f1),
        8 => ("闇", 0x9b59b6),
        _ => ("-", 0x666666),
    };
    (
        name,
        Color::from_rgb_u8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8),
    )
}

/// `format_remaining` が整数秒(ceil)表示から小数点1桁({:.1})表示へ切り替える閾値(秒)。
/// `next_text_change_ms` もこの定数から導出し、表示ロジックと「次に表示が変わる時刻」の
/// 算出が食い違わないようにする（同じ対象を判定する条件式を2箇所に書かない）。
const REMAINING_TENTHS_THRESHOLD_SEC: f64 = 10.0;

/// バフ残時間表示（BuffIconCell formatRemaining 相当）。
pub fn format_remaining(remaining_ms: i64, duration_ms: i64) -> String {
    if duration_ms == 0 {
        return "∞".to_string();
    }
    if remaining_ms <= 0 {
        return "0s".to_string();
    }
    let sec = remaining_ms as f64 / 1000.0;
    if sec > REMAINING_TENTHS_THRESHOLD_SEC {
        format!("{}s", sec.ceil() as i64)
    } else {
        format!("{sec:.1}s")
    }
}

/// `format_remaining` が次に表示テキストを変える時刻までの時間(ms)。
/// 閾値超は整数秒(ceil)表示なので remaining_ms が1000msの倍数を跨ぐたびに、閾値以下は
/// 0.1秒刻み({:.1})表示なので100msの倍数を跨ぐたびに変わる（丸め境界は四捨五入のため
/// `n*100+50ms` 側。境界算出の端数はオーバーレイ側の発火マージンで吸収する想定）。
/// 無期限(duration_ms==0)・表示上ゼロ以下(remaining_ms<=0)は変化しないため None。
/// この窓は毎秒の桁が10秒以下で0.1秒刻みに切り替わる（自キャラ バフ/デバフ オーバーレイ用）。
/// バトルイマジンタイマー（`main.rs` の `buff_cell`／常に ceil・1000ms格子）は表示規則が
/// 異なるため対象外＝main.rs 側に専用の関数を別途持つ。
pub fn next_text_change_ms(remaining_ms: i64, duration_ms: i64) -> Option<u64> {
    if duration_ms == 0 || remaining_ms <= 0 {
        return None;
    }
    let sec = remaining_ms as f64 / 1000.0;
    if sec > REMAINING_TENTHS_THRESHOLD_SEC {
        let rem_mod = (remaining_ms % 1000) as u64;
        Some(if rem_mod == 0 { 1000 } else { rem_mod })
    } else {
        let rem_mod = ((remaining_ms + 50) % 100) as u64;
        Some(if rem_mod == 0 { 100 } else { rem_mod })
    }
}

/// 食事/シロップ残時間表示。30分/10分など長時間が多いため分+秒（例 29m3s）で表す。
pub fn format_consumable_remaining(remaining_ms: i64, duration_ms: i64) -> String {
    if duration_ms == 0 {
        return "∞".to_string();
    }
    if remaining_ms <= 0 {
        return "0s".to_string();
    }
    let total_sec = (remaining_ms as f64 / 1000.0).ceil() as i64;
    let min = total_sec / 60;
    let sec = total_sec % 60;
    if min == 0 {
        format!("{sec}s")
    } else if sec == 0 {
        format!("{min}m")
    } else {
        format!("{min}m{sec}s")
    }
}

/// 名前マスク（utils.ts maskPlayerName）。
pub fn mask_player_name(uid: i64) -> String {
    format!("Player#{:04X}", uid & 0xffff)
}

const MISSING: &str = "—";

/// 溜めた文字列を本体パーツとして確定する（空なら何もしない）。
/// `{classIcon}`/`{imagine}`/`{roleSkill}` は独立パーツになるため、その手前で呼ぶ。
fn flush_body(parts: &mut Vec<NamePart>, text: &mut String, has_name: &mut bool) {
    if text.is_empty() {
        // 名前トークンだけが空展開だった場合に has_name を次パーツへ持ち越さない。
        *has_name = false;
        return;
    }
    parts.push(NamePart {
        text: std::mem::take(text),
        class_icon: false,
        shrink_rank: SHRINK_RANK_BODY,
        has_name: std::mem::replace(has_name, false),
    });
}

/// イマジン系サフィックスを独立パーツとして追加する（空文字なら追加しない）。
/// 本体と別パーツにすることで、幅不足時に本体より先に縮められる。
fn push_suffix(
    parts: &mut Vec<NamePart>,
    text: &mut String,
    has_name: &mut bool,
    suffix: &str,
    shrink_rank: i32,
) {
    if suffix.is_empty() {
        return;
    }
    flush_body(parts, text, has_name);
    parts.push(NamePart {
        text: suffix.to_string(),
        class_icon: false,
        shrink_rank,
        has_name: false,
    });
}

/// 名前列テンプレートを文字列とアイコンのパーツへ展開する。
/// 既定テンプレート: "{classIcon}{name} {spec}({score} - {seasonLv} - {seasonStr}){imagine}{roleSkill}"
///
/// `{imagine}`/`{roleSkill}` は本体と別パーツに切り出す（[`NamePart::shrink_rank`]）。UI 側は
/// この単位で幅を配分するため、列幅が足りないときはロールスキル→イマジン→本体の順に省略される。
#[allow(clippy::too_many_arguments)]
pub fn format_row_name_parts(
    name: &str,
    class_name: &str,
    class_spec_name: &str,
    ability_score: f64,
    season_level: f64,
    season_strength: f64,
    imagine_suffix: &str,
    role_skill_suffix: &str,
    rank: i32,
    template: &str,
    abbreviate: bool,
) -> Vec<NamePart> {
    let spec = if !class_spec_name.is_empty() && class_spec_name != "不明" {
        class_spec_name
    } else {
        ""
    };
    let score = if ability_score > 0.0 {
        format_score(ability_score, abbreviate)
    } else {
        MISSING.to_string()
    };
    let season_lv = if season_level > 0.0 {
        format!("{}", season_level.round() as i64)
    } else {
        MISSING.to_string()
    };
    let season_str = if season_strength > 0.0 {
        format_score(season_strength, abbreviate)
    } else {
        MISSING.to_string()
    };

    let mut parts = Vec::with_capacity(4);
    let mut text = String::with_capacity(template.len() + 16);
    let mut has_name = false;
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '{' {
            text.push(c);
            continue;
        }
        let mut key = String::new();
        while let Some(&nc) = chars.peek() {
            if nc == '}' {
                chars.next();
                break;
            }
            key.push(nc);
            chars.next();
        }
        match key.as_str() {
            "rank" => text.push_str(&rank.to_string()),
            "name" => {
                text.push_str(name);
                has_name = true;
            }
            "class" => text.push_str(class_name),
            "spec" => text.push_str(spec),
            "score" => text.push_str(&score),
            "seasonLv" => text.push_str(&season_lv),
            "seasonStr" => text.push_str(&season_str),
            "imagine" => push_suffix(
                &mut parts,
                &mut text,
                &mut has_name,
                imagine_suffix,
                SHRINK_RANK_IMAGINE,
            ),
            "roleSkill" => push_suffix(
                &mut parts,
                &mut text,
                &mut has_name,
                role_skill_suffix,
                SHRINK_RANK_ROLE_SKILL,
            ),
            "classIcon" => {
                flush_body(&mut parts, &mut text, &mut has_name);
                parts.push(NamePart {
                    text: String::new(),
                    class_icon: true,
                    shrink_rank: SHRINK_RANK_BODY,
                    has_name: false,
                });
            }
            other => {
                text.push('{');
                text.push_str(other);
                text.push('}');
            }
        }
    }
    flush_body(&mut parts, &mut text, &mut has_name);
    parts
}

/// 名前列テンプレート展開（utils.ts formatRowAsText のメタ系キー）。
/// アイコンは表示要素を持たない設定プレビューなどでは文字列へ含めない。
#[allow(clippy::too_many_arguments)]
pub fn format_row_name(
    name: &str,
    class_name: &str,
    class_spec_name: &str,
    ability_score: f64,
    season_level: f64,
    season_strength: f64,
    imagine_suffix: &str,
    role_skill_suffix: &str,
    rank: i32,
    template: &str,
    abbreviate: bool,
) -> String {
    format_row_name_parts(
        name,
        class_name,
        class_spec_name,
        ability_score,
        season_level,
        season_strength,
        imagine_suffix,
        role_skill_suffix,
        rank,
        template,
        abbreviate,
    )
    .into_iter()
    .filter(|part| !part.class_icon)
    .map(|part| part.text)
    .collect()
}

/// 展開済みの名前パーツから職アイコンを取り除き、跡に空白を残さない形へ整える。
/// アイコンを出さない履歴の展開行が、一覧と同じテンプレートを使うために使う。
///
/// 手順: アイコンを除く → 隣り合った本体パーツを結合（継ぎ目の空白は重ねない）→
/// 先頭パーツの前・末尾パーツの後の空白を落とす → 空パーツを落とす。中間のパーツ
/// （`{imagine}` の "-…" や `{roleSkill}` の " (R:…)"）の空白は触らない。結果が空なら
/// `fallback_name`（表示名）だけの本体1パーツにする。
/// 区切りに空白以外の文字を使うテンプレートでは、その文字は残る。
pub fn strip_icon_parts(parts: Vec<NamePart>, fallback_name: &str) -> Vec<NamePart> {
    let mut out: Vec<NamePart> = Vec::with_capacity(parts.len());
    for part in parts.into_iter().filter(|p| !p.class_icon) {
        match out.last_mut() {
            Some(prev)
                if prev.shrink_rank == SHRINK_RANK_BODY && part.shrink_rank == SHRINK_RANK_BODY =>
            {
                let tail = if prev.text.ends_with(char::is_whitespace) {
                    part.text.trim_start()
                } else {
                    part.text.as_str()
                };
                prev.text.push_str(tail);
                prev.has_name |= part.has_name;
            }
            _ => out.push(part),
        }
    }
    if let Some(first) = out.first_mut() {
        first.text = first.text.trim_start().to_string();
    }
    if let Some(last) = out.last_mut() {
        last.text = last.text.trim_end().to_string();
    }
    out.retain(|p| !p.text.is_empty());
    if out.is_empty() {
        out.push(NamePart {
            text: fallback_name.to_string(),
            class_icon: false,
            shrink_rank: SHRINK_RANK_BODY,
            has_name: true,
        });
    }
    out
}

/// コピー用テンプレートの全キーを展開する元データ（utils.ts formatRowAsText 相当）。
/// S5 のクリップボードコピーでも実プレイヤー行から組み立てて再利用する。
pub struct CopyRowData<'a> {
    pub rank: i32,
    pub name: &'a str,
    pub class_name: &'a str,
    pub class_spec_name: &'a str,
    pub total_value: f64,
    pub value_per_sec: f64,
    pub value_pct: f64,
    pub crit_rate: f64,
    pub crit_value_rate: f64,
    pub lucky_rate: f64,
    pub lucky_value_rate: f64,
    pub hits: f64,
    pub hits_per_minute: f64,
    pub ability_score: f64,
    pub season_level: f64,
    pub season_strength: f64,
}

/// コピーテンプレート展開（utils.ts formatRowAsText の全キー）。
pub fn format_row_template(d: &CopyRowData, template: &str, abbreviate: bool) -> String {
    let spec = if !d.class_spec_name.is_empty() && d.class_spec_name != "不明" {
        d.class_spec_name
    } else {
        ""
    };
    let score = if d.ability_score > 0.0 {
        format_score(d.ability_score, abbreviate)
    } else {
        MISSING.to_string()
    };
    let season_lv = if d.season_level > 0.0 {
        format!("{}", d.season_level.round() as i64)
    } else {
        MISSING.to_string()
    };
    let season_str = if d.season_strength > 0.0 {
        format_score(d.season_strength, abbreviate)
    } else {
        MISSING.to_string()
    };

    let mut out = String::with_capacity(template.len() + 32);
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '{' {
            out.push(c);
            continue;
        }
        let mut key = String::new();
        while let Some(&nc) = chars.peek() {
            if nc == '}' {
                chars.next();
                break;
            }
            key.push(nc);
            chars.next();
        }
        match key.as_str() {
            "rank" => out.push_str(&d.rank.to_string()),
            "name" => out.push_str(d.name),
            "class" => out.push_str(d.class_name),
            "spec" => out.push_str(spec),
            "dmg" => out.push_str(&format_number(d.total_value)),
            "dps" => out.push_str(&format_dps(d.value_per_sec)),
            "pct" => out.push_str(&format_pct(d.value_pct)),
            "crit" => out.push_str(&format_pct(d.crit_rate)),
            "critV" => out.push_str(&format_pct(d.crit_value_rate)),
            "lucky" => out.push_str(&format_pct(d.lucky_rate)),
            "luckyV" => out.push_str(&format_pct(d.lucky_value_rate)),
            "hits" => out.push_str(&format!("{}", d.hits as i64)),
            "hpm" => out.push_str(&format!("{:.1}", d.hits_per_minute)),
            "score" => out.push_str(&score),
            "seasonLv" => out.push_str(&season_lv),
            "seasonStr" => out.push_str(&season_str),
            other => {
                out.push('{');
                out.push_str(other);
                out.push('}');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        class_icon_id, class_role_color, format_consumable_remaining, format_remaining,
        format_row_name, format_row_name_parts, next_text_change_ms, strip_icon_parts, ALL_CLASSES,
    };
    use bpsr_core::engine::class::{Class, Role};

    #[test]
    fn consumable_remaining_minutes_and_seconds() {
        // 29m3s（端数は切り上げ）
        assert_eq!(format_consumable_remaining(1_742_500, 1_800_000), "29m3s");
        // ちょうど分は秒を省く
        assert_eq!(format_consumable_remaining(600_000, 1_800_000), "10m");
        // 1分未満は秒のみ
        assert_eq!(format_consumable_remaining(45_000, 600_000), "45s");
        // 無期限・失効
        assert_eq!(format_consumable_remaining(100, 0), "∞");
        assert_eq!(format_consumable_remaining(0, 600_000), "0s");
    }

    // 表示言語（ja/en）が切り替わっても同じアイコンが選ばれること。
    #[test]
    fn class_icon_id_ja_en_match() {
        for c in ALL_CLASSES {
            assert_eq!(
                class_icon_id(c.name_ja()),
                class_icon_id(c.name_en()),
                "class_icon_id differs between ja/en for {:?}",
                c
            );
        }
    }

    // ClassIcon（.slint）の 9 分岐＋フォールバック(0)以外を返さないこと。
    // ここから外れる id を渡すと .slint 側で無音で何も描かれないため、この保証が防波堤になる。
    #[test]
    fn class_icon_id_in_known_set() {
        const HAS_ICON: [i32; 9] = [1, 2, 3, 4, 5, 9, 11, 12, 13];
        for c in ALL_CLASSES {
            let id = class_icon_id(c.name_ja());
            assert!(
                id == 0 || HAS_ICON.contains(&id),
                "unexpected class_icon_id {id} for {:?}",
                c
            );
        }
        // 未マッチの文字列も 0 に落ちること。
        assert_eq!(class_icon_id("存在しないクラス"), 0);
    }

    // ロール分類（タンク/ヒーラー以外はアタッカー。未実装/不明のみ Unknown）。
    #[test]
    fn class_role_assignment() {
        assert_eq!(Class::HeavyGuardian.role(), Role::Tank);
        assert_eq!(Class::ShieldKnight.role(), Role::Tank);
        assert_eq!(Class::VerdantOracle.role(), Role::Healer);
        assert_eq!(Class::BeatPerformer.role(), Role::Healer);
        for c in ALL_CLASSES {
            let role = c.role();
            if matches!(c, Class::Unimplemented) {
                assert_eq!(role, Role::Unknown, "{:?} should be Unknown role", c);
            } else {
                assert_ne!(role, Role::Unknown, "{:?} has no role assigned", c);
            }
        }
    }

    // テスト用のパーツ生成（イマジン/ロールスキルのサフィックスを明示して渡す）。
    fn parts_of(imagine: &str, role_skill: &str, template: &str) -> Vec<super::NamePart> {
        format_row_name_parts(
            "ソラ",
            "ストームブレイド",
            "雷刃型",
            47421.0,
            3184.0,
            0.0,
            imagine,
            role_skill,
            1,
            template,
            true,
        )
    }

    fn body(text: &str, has_name: bool) -> super::NamePart {
        super::NamePart {
            text: text.to_string(),
            class_icon: false,
            shrink_rank: super::SHRINK_RANK_BODY,
            has_name,
        }
    }

    fn icon() -> super::NamePart {
        super::NamePart {
            text: String::new(),
            class_icon: true,
            shrink_rank: super::SHRINK_RANK_BODY,
            has_name: false,
        }
    }

    // {classIcon} は設定のテンプレプレビューなど文字列だけの表示には含めない。
    #[test]
    fn class_icon_token_expands_to_nothing() {
        let name = |t: &str| {
            format_row_name(
                "ソラ",
                "ストームブレイド",
                "雷刃型",
                47421.0,
                3184.0,
                0.0,
                "",
                "",
                1,
                t,
                true,
            )
        };
        assert_eq!(name("{classIcon}{name}"), "ソラ");
        assert_eq!(name("{name}"), name("{classIcon}{name}"));
        // 既定テンプレートも文字列部分だけを取り出すと旧既定と一致する。
        assert_eq!(
            name(crate::settings::DEFAULT_NAME_TEMPLATE),
            name("{name} {spec}({score} - {seasonLv} - {seasonStr}){imagine}{roleSkill}")
        );
    }

    #[test]
    fn class_icon_token_preserves_template_position() {
        assert_eq!(
            parts_of("(ティナ)", "", "前{name}{classIcon}後{imagine}"),
            vec![
                body("前ソラ", true),
                icon(),
                body("後", false),
                super::NamePart {
                    text: "(ティナ)".to_string(),
                    class_icon: false,
                    shrink_rank: super::SHRINK_RANK_IMAGINE,
                    has_name: false,
                },
            ]
        );
    }

    // 未知キーは従来どおりそのまま残し、classIcon の部分一致を誤ってアイコンにしない。
    #[test]
    fn class_icon_token_requires_exact_key() {
        assert_eq!(
            parts_of("", "", "{class}{classIconName}{name}"),
            vec![body("ストームブレイド{classIconName}ソラ", true)]
        );
    }

    // {imagine}/{roleSkill} は本体と別パーツになり、省略順位（本体 < イマジン < ロールスキル）を持つ。
    // UI 側はこの順位を stretch へ写して「ロールスキルから先に削る」配分を作る。
    #[test]
    fn imagine_and_role_skill_become_separate_ranked_parts() {
        let parts = parts_of("-ティナ/アルーナ", " (R:ファルファラ)", "{name}{imagine}{roleSkill}");
        assert_eq!(
            parts,
            vec![
                body("ソラ", true),
                super::NamePart {
                    text: "-ティナ/アルーナ".to_string(),
                    class_icon: false,
                    shrink_rank: super::SHRINK_RANK_IMAGINE,
                    has_name: false,
                },
                super::NamePart {
                    text: " (R:ファルファラ)".to_string(),
                    class_icon: false,
                    shrink_rank: super::SHRINK_RANK_ROLE_SKILL,
                    has_name: false,
                },
            ]
        );
        // 文字列版（コピー/プレビュー）は分割前と同じ連結結果になる。
        assert_eq!(
            parts.iter().map(|p| p.text.as_str()).collect::<String>(),
            "ソラ-ティナ/アルーナ (R:ファルファラ)"
        );
    }

    // 未装備（空文字）のイマジン/ロールスキルはパーツを作らず、前後の文字列も分断しない。
    // 空パーツを残すと UI 側で無駄な間隔と min-width を消費するため。
    #[test]
    fn empty_imagine_tokens_do_not_split_body() {
        assert_eq!(
            parts_of("", "", "{name}{imagine}{roleSkill}({score})"),
            vec![body("ソラ(47.42K)", true)]
        );
    }

    // 名前を含まないパーツ（アイコン・イマジン・{name} を使わないテンプレ）に has_name を立てない。
    #[test]
    fn has_name_marks_only_the_part_holding_the_name() {
        let parts = parts_of("-ティナ", "", "{classIcon}{class} {name}{imagine}");
        assert_eq!(parts.iter().filter(|p| p.has_name).count(), 1);
        assert!(parts.iter().find(|p| p.has_name).unwrap().text.contains("ソラ"));
    }

    fn stripped(imagine: &str, role_skill: &str, template: &str) -> Vec<super::NamePart> {
        strip_icon_parts(parts_of(imagine, role_skill, template), "FALLBACK")
    }

    fn texts(parts: &[super::NamePart]) -> Vec<&str> {
        parts.iter().map(|p| p.text.as_str()).collect()
    }

    // 既定テンプレートでは先頭のアイコンだけが消え、本体・イマジン・ロールスキルは残る。
    #[test]
    fn strip_icon_default_template_removes_only_leading_icon() {
        let full = parts_of("-ティナ", " (R:ファルファラ)", crate::settings::DEFAULT_NAME_TEMPLATE);
        assert!(full[0].class_icon);
        let got = stripped("-ティナ", " (R:ファルファラ)", crate::settings::DEFAULT_NAME_TEMPLATE);
        assert_eq!(got, full[1..].to_vec());
    }

    #[test]
    fn strip_icon_trims_space_left_by_leading_icon() {
        assert_eq!(stripped("", "", "{classIcon} {name}"), vec![body("ソラ", true)]);
    }

    // 名前とアイコンの間に空白が無ければ、結合して本体1個になる。
    #[test]
    fn strip_icon_merges_bodies_around_icon() {
        assert_eq!(stripped("", "", "{name}{classIcon} {spec}"), vec![body("ソラ 雷刃型", true)]);
    }

    // 両側の空白が継ぎ目で二重にならない。
    #[test]
    fn strip_icon_does_not_double_space_at_seam() {
        assert_eq!(stripped("", "", "{name} {classIcon} {spec}"), vec![body("ソラ 雷刃型", true)]);
    }

    #[test]
    fn strip_icon_is_identity_without_icon() {
        let template = "{name} {spec}({score}){imagine}{roleSkill}";
        let full = parts_of("-ティナ", " (R:ファルファラ)", template);
        assert_eq!(stripped("-ティナ", " (R:ファルファラ)", template), full);
    }

    #[test]
    fn strip_icon_only_template_falls_back_to_name() {
        assert_eq!(
            strip_icon_parts(parts_of("", "", "{classIcon}"), "Player#00AB"),
            vec![body("Player#00AB", true)]
        );
    }

    // 空白だけの本体が末尾に残っても落とし、イマジン/ロールスキルの順位と空白は保つ。
    #[test]
    fn strip_icon_keeps_shrink_rank_and_inner_spaces() {
        let got = stripped("-ティナ", " (R:ファルファラ)", "{classIcon}{name}{imagine}{roleSkill} {classIcon}");
        assert_eq!(texts(&got), vec!["ソラ", "-ティナ", " (R:ファルファラ)"]);
        assert_eq!(
            got.iter().map(|p| p.shrink_rank).collect::<Vec<_>>(),
            vec![super::SHRINK_RANK_BODY, super::SHRINK_RANK_IMAGINE, super::SHRINK_RANK_ROLE_SKILL]
        );
        assert_eq!(got.iter().filter(|p| p.has_name).count(), 1);
    }

    // アイコン tint は ja/en どちらの表記でも同じ色になること。
    #[test]
    fn class_role_color_ja_en_match() {
        for c in ALL_CLASSES {
            assert_eq!(
                class_role_color(c.name_ja()),
                class_role_color(c.name_en()),
                "class_role_color differs between ja/en for {:?}",
                c
            );
        }
    }

    // 10秒超は整数秒(ceil)表示。1000msの倍数を跨ぐまでの残余が発火間隔になる。
    #[test]
    fn next_text_change_ms_matches_ceil_display_above_threshold() {
        assert_eq!(format_remaining(11_400, 60_000), "12s");
        assert_eq!(next_text_change_ms(11_400, 60_000), Some(400));
    }

    // 10秒以下は0.1秒刻み({:.1})表示。100msの倍数(四捨五入境界=n*100+50ms)を跨ぐまでの
    // 残余が発火間隔になる。
    #[test]
    fn next_text_change_ms_matches_tenths_display_at_or_below_threshold() {
        assert_eq!(format_remaining(4_260, 60_000), "4.3s");
        // 境界(4.3s/4.2sの切り替わり)は remaining_ms=4250 なので、4260からは10ms。
        assert_eq!(next_text_change_ms(4_260, 60_000), Some(10));
    }

    // 無期限(duration==0)・表示上ゼロ以下(remaining<=0)は秒が動かないため対象外。
    #[test]
    fn next_text_change_ms_none_for_infinite_or_expired() {
        assert_eq!(next_text_change_ms(4_400, 0), None);
        assert_eq!(next_text_change_ms(0, 60_000), None);
        assert_eq!(next_text_change_ms(-1, 60_000), None);
    }
}
