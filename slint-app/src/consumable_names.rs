//! 食事/シロップ base_id → 日本語効果ラベル（data/ConsumableBuffNames.ja.json を埋め込み）。
//! JSON は scripts/gen-consumable-names.py が BuffTable データから生成する。

use std::collections::HashMap;
use std::sync::OnceLock;

static NAMES: OnceLock<HashMap<i32, String>> = OnceLock::new();

fn load() -> HashMap<i32, String> {
    let raw: HashMap<String, String> =
        serde_json::from_str(include_str!("../data/ConsumableBuffNames.ja.json"))
            .unwrap_or_default();
    raw.into_iter()
        .filter_map(|(k, v)| k.parse::<i32>().ok().map(|id| (id, v)))
        .collect()
}

/// 効果ラベル（例: `物攻 +15`）。未収録 base_id は `None`。
pub fn label(base_id: i32) -> Option<String> {
    NAMES.get_or_init(load).get(&base_id).cloned()
}
