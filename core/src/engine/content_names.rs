use std::collections::HashMap;
use std::sync::LazyLock;

use crate::engine::runtime_settings::Lang;

#[derive(serde::Deserialize)]
struct ContentNameEntry {
    en: String,
    #[serde(default)]
    ja: Option<String>,
}

/// コンテンツ(ダンジョン/フィールド等のシーン)名。id は `SocialBody.scene_data.level_map_id`
/// と同じ空間（出自は docs/NAMES_PROVENANCE.md）。
static CONTENT_NAMES: LazyLock<HashMap<u32, ContentNameEntry>> = LazyLock::new(|| {
    let data = include_str!("../../data/json/ContentName.json");
    serde_json::from_str(data).expect("invalid ContentName.json")
});

/// id のコンテンツ名。ja 表示時のみ公式 ja 名があれば優先し、無ければ en へフォールバック
/// （zh も en 辞書を使う。ja 以外の言語ソースが無いため skill_names/monster_names と同じ扱い）。
/// 表に無い id は None。
pub fn content_name(id: u32, lang: Lang) -> Option<&'static str> {
    let entry = CONTENT_NAMES.get(&id)?;
    match lang {
        Lang::Ja => Some(entry.ja.as_deref().unwrap_or(entry.en.as_str())),
        Lang::En | Lang::Zh => Some(entry.en.as_str()),
    }
}

/// 表示用ラベル。0（未確定/シーン不明）は空文字列、表に無い id は `#<id>`。
pub fn content_label(id: u32, lang: Lang) -> String {
    if id == 0 {
        return String::new();
    }
    content_name(id, lang)
        .map(str::to_string)
        .unwrap_or_else(|| format!("#{id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_id_returns_ja_and_en() {
        // 6545 = 霧海の猟場 マスター難易度1（ユーザーがゲーム内で確認済み）
        assert_eq!(content_name(6545, Lang::Ja), Some("霧海の猟場 マスター難易度1"));
        assert_eq!(content_name(6545, Lang::En), Some("Mistveil Hunting Ground Master 1"));
    }

    // ティナ・精神領域系（ユーザーがゲーム内の表記を確認済み）。
    #[test]
    fn tina_mindrealm_ja_names() {
        assert_eq!(content_name(1001, Lang::Ja), Some("ティナ・精神領域"));
        assert_eq!(content_name(1002, Lang::Ja), Some("ティナ・精神領域"));
        assert_eq!(content_name(1031, Lang::Ja), Some("ティナ・精神領域 ノーマル難易度"));
        assert_eq!(content_name(1032, Lang::Ja), Some("ティナ・精神領域 ハード難易度"));
        assert_eq!(content_name(1033, Lang::Ja), Some("ティナ・精神領域 マスター難易度1"));
        assert_eq!(content_name(1633, Lang::Ja), Some("蝕・ティナ・精神領域 マスター難易度1"));
    }

    // ゲーム内表記は「ティナ・精神領域」（ユーザー確認済み）。loc 由来の旧表記の再混入を防ぐ。
    #[test]
    fn no_legacy_tina_spelling() {
        for (id, entry) in CONTENT_NAMES.iter() {
            if let Some(ja) = &entry.ja {
                assert!(!ja.contains("ティナの精神領域"), "id={id} に旧表記: {ja}");
            }
        }
    }

    #[test]
    fn unknown_id_returns_none() {
        assert_eq!(content_name(999_999_999, Lang::Ja), None);
        assert_eq!(content_label(999_999_999, Lang::Ja), "#999999999");
    }

    #[test]
    fn zero_id_label_is_empty() {
        assert_eq!(content_label(0, Lang::Ja), "");
        assert_eq!(content_label(0, Lang::En), "");
    }

    // 全エントリが en を持つ（ja は任意）ことのデータ整合テスト。
    #[test]
    fn all_entries_have_en_name() {
        for (id, entry) in CONTENT_NAMES.iter() {
            assert!(!entry.en.is_empty(), "id={id} の en が空");
        }
    }
}
