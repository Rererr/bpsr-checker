//! 食事(food)/シロップ(alchemy)バフの判定と、戦闘終了をまたいで残時間を保持する
//! 永続ストア。base_id 集合は ConsumableBuffIds.json を埋め込む（姉妹リポ
//! ../resonance-logs-cn の BuffName.json で Icon が `buff_food_up*`=食事 /
//! `buff_agentia_up*`=シロップ のものを抽出）。
//!
//! clear_combat_stats は buff_tracker を消すため、戦闘終了→新規戦闘で食事バフを
//! 忘れてしまう。ゲーム内では効果が継続するので、観測時に終了時刻を控えて
//! buff_tracker が消えても保持し、自然失効/手動リセット/履歴クリアで消す。

use crate::engine::buff_tracker::BuffTracker;
use std::collections::{HashMap, HashSet};
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

/// 1バフの終了時刻・総時間（残量比率算出用）と種類解決用の base_id。
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub expire_at_ms: u128,
    pub duration_ms: u128,
    pub base_id: i32,
}

impl Timing {
    pub fn remaining_ms(&self, now_ms: u128) -> i64 {
        (self.expire_at_ms as i128 - now_ms as i128) as i64
    }
}

/// プレイヤーの食事/シロップ状態。
#[derive(Clone, Copy, Default, Debug)]
pub struct PlayerConsumables {
    pub food: Option<Timing>,
    pub syrup: Option<Timing>,
}

/// buff_tracker の観測でストアを更新し、失効分を除去する。
/// buff_tracker に無い（戦闘終了で消えた）バフは保持し続け、now が終了時刻を
/// 過ぎたら除去する。
pub fn refresh(store: &mut HashMap<i64, PlayerConsumables>, tracker: &BuffTracker, now_ms: u128) {
    let (food_ids, syrup_ids) = &*IDS;
    for (uid, snaps) in tracker.snapshot_all(now_ms) {
        let mut food: Option<Timing> = None;
        let mut syrup: Option<Timing> = None;
        for s in &snaps {
            if s.duration_ms <= 0 {
                continue; // 無期限はタイマー対象外
            }
            let t = Timing {
                expire_at_ms: s.received_at_local_ms + s.duration_ms as u128,
                duration_ms: s.duration_ms as u128,
                base_id: s.base_id,
            };
            if food_ids.contains(&s.base_id) && food.is_none_or(|f| t.expire_at_ms > f.expire_at_ms)
            {
                food = Some(t);
            }
            if syrup_ids.contains(&s.base_id)
                && syrup.is_none_or(|f| t.expire_at_ms > f.expire_at_ms)
            {
                syrup = Some(t);
            }
        }
        if food.is_some() || syrup.is_some() {
            let e = store.entry(uid).or_default();
            if food.is_some() {
                e.food = food;
            }
            if syrup.is_some() {
                e.syrup = syrup;
            }
        }
    }
    // 失効除去
    for pc in store.values_mut() {
        if pc.food.is_some_and(|f| now_ms >= f.expire_at_ms) {
            pc.food = None;
        }
        if pc.syrup.is_some_and(|f| now_ms >= f.expire_at_ms) {
            pc.syrup = None;
        }
    }
    store.retain(|_, pc| pc.food.is_some() || pc.syrup.is_some());
}
