use std::collections::HashMap;
use std::sync::LazyLock;

use crate::engine::runtime_settings::{self, Lang};

/// スキル名（英語・BPSR-ZDPS 由来で品質向上済み）。全 id を網羅する基準辞書。
/// 公式 JA 名が無い id・JA 以外の表示言語はこちらを使う。
static SKILL_NAMES_EN: LazyLock<HashMap<i32, String>> = LazyLock::new(|| {
    let data = include_str!("../../data/json/SkillName.json");
    serde_json::from_str(data).expect("invalid SkillName.json")
});

/// スキル名（日本語・公式）。所有 JA ビルドのゲーム loc から EN→JA が一意に定まる
/// 確実分のみを抽出した 1084 件（曖昧な近接推定は除外）。JA 表示時のみ優先し、
/// 未収録 id は EN へフォールバックする。
static SKILL_NAMES_JA: LazyLock<HashMap<i32, String>> = LazyLock::new(|| {
    let data = include_str!("../../data/json/SkillName.ja.json");
    serde_json::from_str(data).expect("invalid SkillName.ja.json")
});

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
