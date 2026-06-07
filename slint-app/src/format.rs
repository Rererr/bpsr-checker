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

/// クラス名 → 表示色（utils.ts CLASS_COLORS）。
pub fn class_color(class_name: &str) -> Color {
    let hex: u32 = match class_name {
        "ストームブレイド" => 0xfd7cff,
        "フロストメイジ" => 0x3498db,
        "ゲイルランサー" => 0xc6ffd8,
        "ヴァーダントオラクル" => 0x139348,
        "ヘビーガーディアン" => 0x724d2d,
        "ディバインアーチャー" => 0xfff090,
        "シールドファイター" => 0xd1a700,
        "ビートパフォーマー" => 0xe91e63,
        "未実装クラス" => 0x7f8c8d,
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
            other => {
                out.push('{');
                out.push_str(other);
                out.push('}');
            }
        }
    }
    out
}
