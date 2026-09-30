use std::collections::HashMap;
use std::sync::LazyLock;

use crate::engine::runtime_settings::Lang;

/// コンテンツの難易度種別。名前データは基底名だけを持ち、接尾辞は `content_label` が付ける。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DifficultyKind {
    Normal,
    Hard,
    Master,
}

#[derive(serde::Deserialize)]
struct ContentNameEntry {
    en: String,
    #[serde(default)]
    ja: Option<String>,
    #[serde(default)]
    difficulty: Option<DifficultyKind>,
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

/// id の難易度種別。種別の無い id（フィールド・名前自体に難易度を含むもの等）と表に無い id は None。
pub fn difficulty_kind(id: u32) -> Option<DifficultyKind> {
    CONTENT_NAMES.get(&id)?.difficulty
}

/// `SyncDungeonData` の `difficulty`（int32）をマスターの段階（1始まり）へ変換する。負の値は 0（不明）。
pub fn master_stage(difficulty: i32) -> u32 {
    u32::try_from(difficulty).unwrap_or(0)
}

/// 難易度接尾辞（先頭スペース込み）。ja/en の対応はここだけに置く。
/// `master_stage == 0`（段階不明）のマスターは数字を付けない。
fn difficulty_suffix(kind: DifficultyKind, master_stage: u32, ja: bool) -> String {
    match (kind, ja, master_stage) {
        (DifficultyKind::Normal, true, _) => " ノーマル難易度".to_string(),
        (DifficultyKind::Hard, true, _) => " ハード難易度".to_string(),
        (DifficultyKind::Master, true, 0) => " マスター難易度".to_string(),
        (DifficultyKind::Master, true, n) => format!(" マスター難易度{n}"),
        (DifficultyKind::Normal, false, _) => " Normal".to_string(),
        (DifficultyKind::Hard, false, _) => " Hard".to_string(),
        (DifficultyKind::Master, false, 0) => " Master".to_string(),
        (DifficultyKind::Master, false, n) => format!(" Master {n}"),
    }
}

/// 表示用ラベル。0（未確定/シーン不明）は空文字列、表に無い id は `#<id>`。
/// `master_stage` はマスター種別の id にだけ使い、それ以外では無視する。
/// ja 表示で ja 名が無く en 名へフォールバックしたときは、接尾辞も en に揃える。
pub fn content_label(id: u32, master_stage: u32, lang: Lang) -> String {
    if id == 0 {
        return String::new();
    }
    let Some(entry) = CONTENT_NAMES.get(&id) else {
        return format!("#{id}");
    };
    let (name, ja) = match (lang, entry.ja.as_deref()) {
        (Lang::Ja, Some(ja_name)) => (ja_name, true),
        _ => (entry.en.as_str(), false),
    };
    match entry.difficulty {
        Some(kind) => format!("{name}{}", difficulty_suffix(kind, master_stage, ja)),
        None => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_id_returns_base_name_ja_and_en() {
        // 6545 = 霧海の猟場（マスター）。名前データは基底名のみ、接尾辞は content_label が付ける。
        assert_eq!(content_name(6545, Lang::Ja), Some("霧海の猟場"));
        assert_eq!(content_name(6545, Lang::En), Some("Mistveil Hunting Ground"));
        assert_eq!(difficulty_kind(6545), Some(DifficultyKind::Master));
        assert_eq!(difficulty_kind(6543), Some(DifficultyKind::Normal));
        assert_eq!(difficulty_kind(6544), Some(DifficultyKind::Hard));
        assert_eq!(difficulty_kind(12011), None);
        assert_eq!(difficulty_kind(999_999_999), None);
    }

    #[test]
    fn label_with_master_stage() {
        assert_eq!(content_label(6545, 0, Lang::Ja), "霧海の猟場 マスター難易度");
        assert_eq!(content_label(6545, 3, Lang::Ja), "霧海の猟場 マスター難易度3");
        assert_eq!(content_label(6545, 3, Lang::En), "Mistveil Hunting Ground Master 3");
        assert_eq!(content_label(6545, 0, Lang::En), "Mistveil Hunting Ground Master");
    }

    #[test]
    fn label_ignores_stage_for_non_master() {
        assert_eq!(content_label(6543, 3, Lang::Ja), "霧海の猟場 ノーマル難易度");
        assert_eq!(content_label(6544, 0, Lang::En), "Mistveil Hunting Ground Hard");
        assert_eq!(content_label(6544, 7, Lang::Ja), "霧海の猟場 ハード難易度");
        // 種別の無い id（名前自体に難易度を含むもの）も段階を無視する。
        assert_eq!(
            content_label(12011, 5, Lang::Ja),
            content_name(12011, Lang::Ja).unwrap()
        );
    }

    // ja 名の無い id は ja 表示でも en 名へフォールバックし、接尾辞も en に揃う。
    #[test]
    fn label_falls_back_to_en_suffix_without_ja_name() {
        assert_eq!(content_label(6023, 2, Lang::Ja), "Kanamia Trial Master 2");
    }

    #[test]
    fn master_stage_conversion() {
        assert_eq!(master_stage(1), 1);
        assert_eq!(master_stage(20), 20);
        assert_eq!(master_stage(0), 0);
        assert_eq!(master_stage(-1), 0);
    }

    // ティナ・精神領域系（ユーザーがゲーム内の表記を確認済み）。
    #[test]
    fn tina_mindrealm_ja_names() {
        assert_eq!(content_label(1001, 0, Lang::Ja), "ティナ・精神領域");
        assert_eq!(content_label(1002, 0, Lang::Ja), "ティナ・精神領域");
        assert_eq!(content_label(1031, 0, Lang::Ja), "ティナ・精神領域 ノーマル難易度");
        assert_eq!(content_label(1032, 0, Lang::Ja), "ティナ・精神領域 ハード難易度");
        assert_eq!(content_label(1033, 2, Lang::Ja), "ティナ・精神領域 マスター難易度2");
        assert_eq!(content_label(1633, 1, Lang::Ja), "蝕・ティナ・精神領域 マスター難易度1");
    }

    // 難易度接尾辞は content_label が付けるため、データ側に残っていない（二重付与の防止）。
    // 名前自体に難易度を含む "Guild Hunt - Hard" 系は種別なしのまま許容する。
    #[test]
    fn no_difficulty_suffix_left_in_data() {
        for (id, entry) in CONTENT_NAMES.iter() {
            if entry.en.starts_with("Guild Hunt") {
                assert!(entry.difficulty.is_none(), "id={id} は種別なしのはず");
                continue;
            }
            for suffix in [" Normal", " Hard", " Master"] {
                assert!(!entry.en.contains(suffix), "id={id} の en に接尾辞: {}", entry.en);
            }
            if let Some(ja) = &entry.ja {
                assert!(!ja.contains("難易度"), "id={id} の ja に接尾辞: {ja}");
            }
        }
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
        assert_eq!(content_label(999_999_999, 3, Lang::Ja), "#999999999");
    }

    #[test]
    fn zero_id_label_is_empty() {
        assert_eq!(content_label(0, 0, Lang::Ja), "");
        assert_eq!(content_label(0, 5, Lang::En), "");
    }

    // 全エントリが en を持つ（ja は任意）ことのデータ整合テスト。
    #[test]
    fn all_entries_have_en_name() {
        for (id, entry) in CONTENT_NAMES.iter() {
            assert!(!entry.en.is_empty(), "id={id} の en が空");
        }
    }
}
