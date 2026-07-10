use std::collections::HashMap;
use std::sync::LazyLock;

use crate::engine::runtime_settings::{self, Lang};

/// スキル名（英語）。全 id を網羅する基準辞書。
/// 公式 JA 名が無い id・JA 以外の表示言語はこちらを使う。
static SKILL_NAMES_EN: LazyLock<HashMap<i32, String>> = LazyLock::new(|| {
    let data = include_str!("../../data/json/SkillName.json");
    serde_json::from_str(data).expect("invalid SkillName.json")
});

/// スキル名（日本語）。包括的な日本語スキル辞書（旧 SkillName.json 由来を復元、約 1.5 万件）。
/// JA 表示時のみ優先し、未収録 id は EN（全言語共通の基準辞書）へフォールバックする。
/// 出自はゲーム内日本語名称（CN→JA 変換/upstream 同期由来）で公式 loc からの抽出ではない。
/// 別 pkg 探索による公式名抽出は後続タスク（docs/i18n-game-extraction.md）。
static SKILL_NAMES_JA: LazyLock<HashMap<i32, String>> = LazyLock::new(|| {
    let data = include_str!("../../data/json/SkillName.ja.json");
    serde_json::from_str(data).expect("invalid SkillName.ja.json")
});

/// 日本語スキル名の直接参照（表示言語設定に依存しない）。イマジン奥義/絶技スキルの
/// 判別（名前が「奥義！」「絶技！」で始まるか）等、内部ロジック用。未収録 id は None。
pub fn skill_name_ja(id: i32) -> Option<&'static str> {
    SKILL_NAMES_JA.get(&id).map(String::as_str)
}

pub fn get_skill_name(id: i32) -> String {
    // 表示言語が JA かつ公式 JA 名がある id のみ JA を優先。それ以外は EN（全言語共通）。
    if runtime_settings::display_lang() == Lang::Ja {
        if let Some(name) = SKILL_NAMES_JA.get(&id) {
            return name.clone();
        }
    }
    SKILL_NAMES_EN.get(&id).cloned().unwrap_or_else(|| {
        // 未知 id のフォールバック文言のみ表示言語に追従する。
        match runtime_settings::display_lang() {
            Lang::En => format!("Unknown skill ({id})"),
            Lang::Zh => format!("未知技能 ({id})"),
            Lang::Ja => format!("不明な技 ({id})"),
        }
    })
}
