//! 食事(food)/シロップ(alchemy)バフの判定。
//! base_id 集合は ConsumableBuffIds.json を埋め込む（姉妹リポ ../resonance-logs-cn の
//! BuffName.json で Icon が `buff_food_up*`=食事 / `buff_agentia_up*`=シロップ のものを抽出）。

use crate::engine::buff_tracker::BuffTracker;
use std::collections::HashSet;
use std::sync::LazyLock;

#[derive(serde::Deserialize)]
struct Ids {
    food: Vec<i32>,
    syrup: Vec<i32>,
}

static IDS: LazyLock<(HashSet<i32>, HashSet<i32>)> = LazyLock::new(|| {
    let data = include_str!("../../data/json/ConsumableBuffIds.json");
    let parsed: Ids = serde_json::from_str(data).expect("invalid ConsumableBuffIds.json");
    (
        parsed.food.into_iter().collect(),
        parsed.syrup.into_iter().collect(),
    )
});

/// プレイヤーの現在のバフから (食事あり, シロップあり) を判定する。
pub fn detect(tracker: &BuffTracker, player_uid: i64, now_ms: u128) -> (bool, bool) {
    let (food_ids, syrup_ids) = &*IDS;
    let mut food = false;
    let mut syrup = false;
    for s in tracker.snapshot_for(player_uid, now_ms) {
        // 有効中のみ（無期限 duration_ms==0 は有効扱い）
        if s.duration_ms != 0 && s.remaining_ms <= 0 {
            continue;
        }
        if !food && food_ids.contains(&s.base_id) {
            food = true;
        }
        if !syrup && syrup_ids.contains(&s.base_id) {
            syrup = true;
        }
        if food && syrup {
            break;
        }
    }
    (food, syrup)
}
