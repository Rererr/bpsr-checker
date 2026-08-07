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

pub fn format_number(n: f64) -> String {
    if n >= 1_000_000.0 {
        format!("{:.2}M", n / 1_000_000.0)
    } else if n >= 1_000.0 {
        format!("{:.1}K", n / 1_000.0)
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

/// 名前列テンプレートの職アイコン トークン。文字列へは展開されず（format_row_name で空に潰す）、
/// 有無だけがアイコン列の表示可否になる。表示判定はこの1箇所に集約する。
pub const CLASS_ICON_TOKEN: &str = "{classIcon}";

/// 名前列テンプレートが職アイコンを含むか（アイコン表示の唯一の判定）。
pub fn template_shows_class_icon(template: &str) -> bool {
    template.contains(CLASS_ICON_TOKEN)
}

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

/// バフ残時間表示（BuffIconCell formatRemaining 相当）。
pub fn format_remaining(remaining_ms: i64, duration_ms: i64) -> String {
    if duration_ms == 0 {
        return "∞".to_string();
    }
    if remaining_ms <= 0 {
        return "0s".to_string();
    }
    let sec = remaining_ms as f64 / 1000.0;
    if sec > 10.0 {
        format!("{}s", sec.ceil() as i64)
    } else {
        format!("{sec:.1}s")
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

/// 名前列テンプレート展開（utils.ts formatRowAsText のメタ系キー）。
/// 既定テンプレート: "{name} {spec}({score} - {seasonLv} - {seasonStr})"
#[allow(clippy::too_many_arguments)]
pub fn format_row_name(
    name: &str,
    class_name: &str,
    class_spec_name: &str,
    ability_score: f64,
    season_level: f64,
    season_strength: f64,
    imagine_suffix: &str,
    rank: i32,
    template: &str,
    abbreviate: bool,
) -> String {
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

    let mut out = String::with_capacity(template.len() + 16);
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
            "rank" => out.push_str(&rank.to_string()),
            "name" => out.push_str(name),
            "class" => out.push_str(class_name),
            "spec" => out.push_str(spec),
            "score" => out.push_str(&score),
            "seasonLv" => out.push_str(&season_lv),
            "seasonStr" => out.push_str(&season_str),
            "imagine" => out.push_str(imagine_suffix),
            // アイコンは別要素で描くのでここでは空に潰す（設定のテンプレプレビューなど
            // アイコン要素を持たない表示に生の {classIcon} を出さないため）。
            "classIcon" => {}
            other => {
                out.push('{');
                out.push_str(other);
                out.push('}');
            }
        }
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
        class_icon_id, class_role_color, format_consumable_remaining, format_row_name,
        template_shows_class_icon, ALL_CLASSES,
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

    // {classIcon} は文字列へ展開されない（設定のテンプレプレビューに生の波括弧を出さない）。
    // 未知キーが素通しされる仕様なので、専用の分岐が消えると即座にこのテストが落ちる。
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
                1,
                t,
                true,
            )
        };
        assert_eq!(name("{classIcon}{name}"), "ソラ");
        assert_eq!(name("{name}"), name("{classIcon}{name}"));
        // 既定テンプレートはアイコンありで、展開結果は旧既定（アイコン抜き）と一致する。
        assert!(template_shows_class_icon(
            crate::settings::DEFAULT_NAME_TEMPLATE
        ));
        assert_eq!(
            name(crate::settings::DEFAULT_NAME_TEMPLATE),
            name("{name} {spec}({score} - {seasonLv} - {seasonStr}){imagine}")
        );
    }

    // アイコン表示の判定はトークンの有無だけで決まる。
    #[test]
    fn template_shows_class_icon_detects_token() {
        assert!(template_shows_class_icon("{classIcon}{name}"));
        assert!(!template_shows_class_icon("{name} {spec}"));
        // 別キーの部分一致で誤検出しないこと。
        assert!(!template_shows_class_icon("{class}{name}"));
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
}
