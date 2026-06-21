use std::collections::HashMap;
use std::sync::LazyLock;

use crate::engine::runtime_settings::{self, Lang};

/// ボス/モンスター名（日本語・既存 curated）。is_boss の判定にも使う全 id 集合。
pub static MONSTER_NAMES_BOSS: LazyLock<HashMap<u32, String>> = LazyLock::new(|| {
    let data = include_str!("../../data/json/MonsterNameBoss.json");
    serde_json::from_str(data).expect("invalid MonsterNameBoss.json")
});

/// ボス/モンスター名（英語・BPSR-ZDPS 由来）。ja と同じ id 集合。
static MONSTER_NAMES_EN: LazyLock<HashMap<u32, String>> = LazyLock::new(|| {
    let data = include_str!("../../data/json/MonsterNameBoss.en.json");
    serde_json::from_str(data).expect("invalid MonsterNameBoss.en.json")
});

pub fn is_boss(monster_id: u32) -> bool {
    MONSTER_NAMES_BOSS.contains_key(&monster_id)
}

/// 表示言語に応じたボス名。ja は curated 日本語、en/zh は英語を優先し、互いにフォールバックする
/// （簡体字のゲーム名ソースが無いため zh も英語を表示する）。
pub fn get_boss_name(monster_id: u32) -> Option<String> {
    let (primary, secondary) = match runtime_settings::display_lang() {
        Lang::Ja => (&*MONSTER_NAMES_BOSS, &*MONSTER_NAMES_EN),
        Lang::En | Lang::Zh => (&*MONSTER_NAMES_EN, &*MONSTER_NAMES_BOSS),
    };
    primary
        .get(&monster_id)
        .or_else(|| secondary.get(&monster_id))
        .cloned()
}
