//! バトルイマジン(装備枠 SlotPositionId 7/8)のスキルID→表示名。
//! ID集合・名称は ImagineSkillNames.json を埋め込む静的テーブル。
//! 日本語(names_ja)を優先し、無ければ英語(names_en)にフォールバックする。
//! 中国語のみ判明のもの・完全に未登録のIDは表示しない（誤表示を避ける安全側デフォルト）。

use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(serde::Deserialize)]
struct Names {
    names_ja: HashMap<String, String>,
    names_en: HashMap<String, String>,
}

fn parse_id_map(map: HashMap<String, String>) -> HashMap<i32, String> {
    map.into_iter()
        .filter_map(|(k, v)| k.parse::<i32>().ok().map(|id| (id, v)))
        .collect()
}

static NAMES: LazyLock<(HashMap<i32, String>, HashMap<i32, String>)> = LazyLock::new(|| {
    let data = include_str!("../../data/json/ImagineSkillNames.json");
    let parsed: Names = serde_json::from_str(data).expect("invalid ImagineSkillNames.json");
    (parse_id_map(parsed.names_ja), parse_id_map(parsed.names_en))
});

/// `skill_id` に対応するバトルイマジン表示名（未登録なら None）。日本語優先、無ければ英語。
pub fn imagine_name(skill_id: i32) -> Option<String> {
    let (names_ja, names_en) = &*NAMES;
    names_ja.get(&skill_id).or_else(|| names_en.get(&skill_id)).cloned()
}
