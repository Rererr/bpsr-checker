//! バフ/デバフ base_id → 表示名（data/BuffName.{ja,en}.json を埋め込み）。
//! 表示言語は core の runtime_settings::display_lang() に追従。簡体字のバフ名ソースが
//! 無いため zh は英語を表示する（ja は既存 curated、en は BPSR-ZDPS 由来）。

use std::collections::HashMap;
use std::sync::LazyLock;

use bpsr_core::engine::runtime_settings::{self, Lang};

fn parse(json: &str) -> HashMap<i32, String> {
    #[derive(serde::Deserialize)]
    struct Entry {
        name: String,
    }
    let raw: HashMap<String, Entry> = serde_json::from_str(json).unwrap_or_default();
    raw.into_iter()
        .filter_map(|(k, v)| k.parse::<i32>().ok().map(|id| (id, v.name)))
        .collect()
}

static JA: LazyLock<HashMap<i32, String>> =
    LazyLock::new(|| parse(include_str!("../data/BuffName.ja.json")));
static EN: LazyLock<HashMap<i32, String>> =
    LazyLock::new(|| parse(include_str!("../data/BuffName.en.json")));

/// 表示名。表示言語を優先しつつ ja/en 間でフォールバック。未知なら `#<base_id>`。
pub fn label(base_id: i32) -> String {
    let order: [&LazyLock<HashMap<i32, String>>; 2] = match runtime_settings::display_lang() {
        Lang::Ja => [&JA, &EN],
        Lang::En | Lang::Zh => [&EN, &JA],
    };
    for m in order {
        if let Some(name) = m.get(&base_id) {
            return name.clone();
        }
    }
    format!("#{base_id}")
}
