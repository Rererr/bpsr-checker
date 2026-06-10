use std::collections::HashMap;
use std::sync::LazyLock;

static SKILL_NAMES: LazyLock<HashMap<i32, String>> = LazyLock::new(|| {
    let data = include_str!("../../data/json/SkillName.json");
    serde_json::from_str(data).expect("invalid SkillName.json")
});

pub fn get_skill_name(id: i32) -> String {
    SKILL_NAMES
        .get(&id)
        .cloned()
        .unwrap_or_else(|| format!("不明な技 ({id})"))
}
