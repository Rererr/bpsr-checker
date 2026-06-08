//! バフ/デバフ base_id → 表示名（src/lib/data/json/BuffName.ja.json を埋め込み）。

use std::collections::HashMap;
use std::sync::OnceLock;

static NAMES: OnceLock<HashMap<i32, String>> = OnceLock::new();

fn load() -> HashMap<i32, String> {
    #[derive(serde::Deserialize)]
    struct Entry {
        name: String,
    }
    let raw: HashMap<String, Entry> =
        serde_json::from_str(include_str!("../data/BuffName.ja.json")).unwrap_or_default();
    raw.into_iter()
        .filter_map(|(k, v)| k.parse::<i32>().ok().map(|id| (id, v.name)))
        .collect()
}

/// 表示名。未知なら `不明 #<base_id>`。
pub fn label(base_id: i32) -> String {
    NAMES
        .get_or_init(load)
        .get(&base_id)
        .cloned()
        .unwrap_or_else(|| format!("不明 #{base_id}"))
}
