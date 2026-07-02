//! 表示整形ヘルパ（フロント src/utils.ts を移植）。

use slint::Color;

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
pub fn class_color(class_name: &str) -> Color {
    let hex: u32 = match class_name {
        "ストームブレイド" | "Stormblade" => 0xfd7cff,
        "フロストメイジ" | "Frost Mage" => 0x3498db,
        "ゲイルランサー" | "Wind Knight" => 0xc6ffd8,
        "ヴァーダントオラクル" | "Verdant Oracle" => 0x139348,
        "ヘヴィガーディアン" | "Heavy Guardian" => 0x724d2d,
        "ディバインアーチャー" | "Marksman" => 0xfff090,
        "シールドファイター" | "Shield Knight" => 0xd1a700,
        "ビートパフォーマー" | "Beat Performer" => 0xe91e63,
        "未実装クラス" | "Unimplemented Class" => 0x7f8c8d,
        _ => 0x95a5a6,
    };
    Color::from_rgb_u8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
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
    use super::format_consumable_remaining;

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
}
