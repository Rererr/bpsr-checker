//! バフ/デバフ base_id → 表示名（data/BuffName.{ja,en}.json を埋め込み）。
//! 表示言語は core の runtime_settings::display_lang() に追従。簡体字のバフ名ソースが
//! 無いため zh は英語を表示する（ja は既存 curated、en は既存英訳辞書由来）。

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

#[cfg(test)]
mod tests {
    use super::*;

    /// 辞書に表示対象として載っている base_id は必ず表示名を持つ。持たないと UI が
    /// `#3060441` のような生 ID を出す（S3 追補で26件が漏れていた実例あり）。
    /// JA 名が確定できないものは公式 EN 名を en 側に入れてフォールバックさせる。
    #[test]
    fn every_visible_buff_has_a_display_name() {
        let missing: Vec<i32> = bpsr_core::engine::buff_dictionary::visible_base_ids()
            .into_iter()
            .filter(|id| !JA.contains_key(id) && !EN.contains_key(id))
            .collect();
        assert!(
            missing.is_empty(),
            "表示名が無い base_id: {missing:?} (BuffName.ja.json / BuffName.en.json へ追加が必要)"
        );
    }
}
