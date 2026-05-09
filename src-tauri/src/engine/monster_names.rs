use std::collections::HashMap;
use std::sync::LazyLock;

pub static MONSTER_NAMES_BOSS: LazyLock<HashMap<u32, String>> = LazyLock::new(|| {
    let data = include_str!("../../../src/lib/data/json/MonsterNameBoss.json");
    serde_json::from_str(data).expect("invalid MonsterNameBoss.json")
});

pub fn is_boss(monster_id: u32) -> bool {
    MONSTER_NAMES_BOSS.contains_key(&monster_id)
}

pub fn get_boss_name(monster_id: u32) -> Option<&'static str> {
    MONSTER_NAMES_BOSS.get(&monster_id).map(|s| s.as_str())
}
