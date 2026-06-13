//! 食事(food)/シロップ(alchemy)バフの判定と、戦闘終了をまたいで残時間を保持する
//! 永続ストア。base_id 集合は ConsumableBuffIds.json を埋め込む（姉妹リポ
//! ../resonance-logs-cn の BuffName.json で Icon が `buff_food_up*`=食事 /
//! `buff_agentia_up*`=シロップ のものを抽出）。
//!
//! clear_combat_stats は buff_tracker を消すため、戦闘終了→新規戦闘で食事バフを
//! 忘れてしまう。ゲーム内では効果が継続するので、観測時に終了時刻を控えて
//! buff_tracker が消えても保持し、自然失効/手動リセット/履歴クリアで消す。

use crate::engine::buff_tracker::{BuffStateSnapshot, BuffTracker};
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
/// `buff_uuid`/`create_time`/`layer` は付与の同一性キー。受動再観測では expire を
/// 凍結し、別インスタンスの再付与（buff_uuid 変化＝再食）・同一インスタンスの
/// タイマーリフレッシュ（create_time 変化）・重ねがけ（layer 増）でのみ更新する。
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub expire_at_ms: u128,
    pub duration_ms: u128,
    pub base_id: i32,
    pub buff_uuid: i32,
    pub create_time: i64,
    pub layer: i32,
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
///
/// 残り時間はゲームが送る duration を尊重し、新規付与（create_time 変化）・
/// 重ねがけ（layer 増）でのみ更新する。受動的な BuffTick/Snapshot 再観測は
/// `received_at_local_ms` を再ベースするが、ここでは expire を凍結して
/// 残り時間が膨張・リセットしないようにする（ゲーム実値と食い違わせない）。
pub fn refresh(store: &mut HashMap<i64, PlayerConsumables>, tracker: &BuffTracker, now_ms: u128) {
    let (food_ids, syrup_ids) = &*IDS;
    for (uid, snaps) in tracker.snapshot_all(now_ms) {
        // 食事/シロップそれぞれ、終了時刻が最も遅い候補を代表（重ねがけ後の最新）とする。
        let mut food_cand: Option<&BuffStateSnapshot> = None;
        let mut syrup_cand: Option<&BuffStateSnapshot> = None;
        for s in &snaps {
            if s.duration_ms <= 0 {
                continue; // 無期限はタイマー対象外
            }
            if food_ids.contains(&s.base_id) && later_expire(s, food_cand) {
                food_cand = Some(s);
            }
            if syrup_ids.contains(&s.base_id) && later_expire(s, syrup_cand) {
                syrup_cand = Some(s);
            }
        }

        let existing = store.get(&uid).copied().unwrap_or_default();
        let food = merge(existing.food, food_cand, now_ms);
        let syrup = merge(existing.syrup, syrup_cand, now_ms);
        if food.is_some() || syrup.is_some() {
            let e = store.entry(uid).or_default();
            e.food = food;
            e.syrup = syrup;
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

/// `s` の終了時刻が現候補より遅ければ true（無期限は上で除外済み）。
fn later_expire(s: &BuffStateSnapshot, cand: Option<&BuffStateSnapshot>) -> bool {
    let s_expire = s.received_at_local_ms + s.duration_ms as u128;
    cand.is_none_or(|c| s_expire > c.received_at_local_ms + c.duration_ms as u128)
}

/// 既存 Timing と観測候補から、更新後の Timing を決める。
/// - 候補なし: 既存を保持（戦闘クリアで buff_tracker が消えても凍結）。
/// - 既存なし: 観測値で初期化。
/// - 既存失効済み: 観測値で再付与扱い。
/// - 別インスタンスの再付与（buff_uuid 変化＝再食）/ 重ねがけ（layer 増）/
///   同一インスタンスのリフレッシュ（create_time が両者非0で変化）: 観測値で更新。
/// - それ以外（受動再観測・同一 buff_uuid・create_time 据置/0）: 既存 expire を凍結。
fn merge(existing: Option<Timing>, cand: Option<&BuffStateSnapshot>, now_ms: u128) -> Option<Timing> {
    let Some(s) = cand else {
        return existing;
    };
    let fresh = || Timing {
        expire_at_ms: s.received_at_local_ms + s.duration_ms as u128,
        duration_ms: s.duration_ms as u128,
        base_id: s.base_id,
        buff_uuid: s.buff_uuid,
        create_time: s.create_time_server,
        layer: s.layer,
    };
    let Some(e) = existing else {
        return Some(fresh());
    };
    if now_ms >= e.expire_at_ms {
        return Some(fresh()); // 既存は失効済み → 新規付与として採用
    }
    if s.buff_uuid != e.buff_uuid {
        // 別インスタンスの再付与（再食＝新規付与）。残時間の長短は比較せず、観測された
        // 現行インスタンスを信頼する。候補は refresh 側の later_expire が現スナップショット
        // 群から最遅 expire を選んでおり、より長い既存インスタンスが tracker に残っていれば
        // そちらが候補になる。新 uuid が候補に選ばれる＝旧インスタンスは既に tracker から
        // 消えている＝新 uuid が正規バフ、なので過大評価された旧 expire を実値へ補正する。
        // 伸長時のみ採用するガードは、古い expire に凍結したままグレーへ戻る不具合を
        // 別経路で再発させるため入れない。
        return Some(fresh());
    }
    if s.layer > e.layer {
        return Some(fresh()); // 重ねがけ（スタック増）
    }
    // ここに到達するのは buff_uuid が一致する場合のみ（再食は上で処理済み）。
    if s.create_time_server != 0 && e.create_time != 0 && s.create_time_server != e.create_time {
        return Some(fresh()); // 同一 buff_uuid のタイマーリフレッシュ（create_time 変化）
    }
    Some(e) // 受動再観測 → 凍結
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::buff_tracker::BuffTracker;
    use crate::protocol::pb;

    const FOOD_ID: i32 = 700083; // ConsumableBuffIds.json food[0]
    const SYRUP_ID: i32 = 681836; // ConsumableBuffIds.json syrup[0]
    const UID: i64 = 5000;

    fn buff_info(base_id: i32, duration: i32, create_time: i64, layer: i32) -> pb::BuffSnapshot {
        pb::BuffSnapshot {
            buff_uuid: 1,
            base_id,
            level: 1,
            host_uuid: 0,
            table_uuid: 0,
            create_time,
            fire_uuid: 0,
            layer,
            part_id: 0,
            count: 1,
            duration,
            fight_source_info: None,
        }
    }

    fn food_info(duration: i32, create_time: i64, layer: i32) -> pb::BuffSnapshot {
        buff_info(FOOD_ID, duration, create_time, layer)
    }

    // 受動 BuffTick（create_time=0 で received_at を再ベース）では expire を凍結する。
    #[test]
    fn passive_reobservation_does_not_rebase() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();
        tracker.apply_buff_add(1, &food_info(600_000, 1000, 1), 0, UID);
        refresh(&mut store, &tracker, 0);
        assert_eq!(store[&UID].food.unwrap().remaining_ms(0), 600_000);

        // create_time=0 の受動 tick で now=100s に再ベース
        let tick = pb::BuffTick {
            host_uuid: (UID << 16) | 640,
            buff_uuid: 1,
            base_id: FOOD_ID,
            duration: 600_000,
            create_time: 0,
            layer: 1,
        };
        tracker.apply_change(&tick, 100_000);
        refresh(&mut store, &tracker, 100_000);
        // 凍結されていれば残 500s（再ベースされると 600s に膨張する）
        assert_eq!(store[&UID].food.unwrap().remaining_ms(100_000), 500_000);
    }

    // 重ねがけ（layer 増）で expire を更新する。
    #[test]
    fn stacking_layer_increase_refreshes() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();
        tracker.apply_buff_add(1, &food_info(600_000, 1000, 1), 0, UID);
        refresh(&mut store, &tracker, 0);

        let change = pb::BuffChange { layer: 2, duration: 600_000, create_time: 2000 };
        tracker.apply_buff_change(UID, 1, &change, 100_000);
        refresh(&mut store, &tracker, 100_000);
        // 100s + 600s = 700s 終了 → 残 600s、layer=2
        assert_eq!(store[&UID].food.unwrap().remaining_ms(100_000), 600_000);
        assert_eq!(store[&UID].food.unwrap().layer, 2);
    }

    // 再食（別 buff_uuid の新規付与）は古い残時間に固まらず expire を延長する。
    // ＝ ボス戦リセット後に再食してもアイコンがグレーへ戻らない。
    #[test]
    fn reeat_new_instance_extends() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();
        // 最初の食事: buff_uuid=1, 残 600s
        tracker.apply_buff_add(1, &food_info(600_000, 1000, 1), 0, UID);
        refresh(&mut store, &tracker, 0);
        assert_eq!(store[&UID].food.unwrap().remaining_ms(0), 600_000);

        // 300s 後に再食: 別インスタンス buff_uuid=2, 残 600s（古いインスタンスは残存）
        tracker.apply_buff_add(2, &food_info(600_000, 2000, 1), 300_000, UID);
        refresh(&mut store, &tracker, 300_000);
        // 300s + 600s = 900s 終了 → 残 600s に延長され、新インスタンスが採用される
        assert_eq!(store[&UID].food.unwrap().remaining_ms(300_000), 600_000);
        assert_eq!(store[&UID].food.unwrap().buff_uuid, 2);
    }

    // create_time=0 の受動再食でも buff_uuid が変われば延長する（create_time に依存しない）。
    #[test]
    fn reeat_zero_create_time_still_extends_via_buff_uuid() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();
        tracker.apply_buff_add(1, &food_info(600_000, 0, 1), 0, UID);
        refresh(&mut store, &tracker, 0);

        tracker.apply_buff_add(2, &food_info(600_000, 0, 1), 300_000, UID);
        refresh(&mut store, &tracker, 300_000);
        assert_eq!(store[&UID].food.unwrap().remaining_ms(300_000), 600_000);
    }

    // 片方（食事）のみ再食しても、もう片方（シロップ）の凍結残時間は影響を受けない。
    #[test]
    fn reeat_food_does_not_disturb_syrup() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();
        tracker.apply_buff_add(10, &food_info(600_000, 0, 1), 0, UID);
        tracker.apply_buff_add(20, &buff_info(SYRUP_ID, 600_000, 0, 1), 0, UID);
        refresh(&mut store, &tracker, 0);
        assert_eq!(store[&UID].food.unwrap().remaining_ms(0), 600_000);
        assert_eq!(store[&UID].syrup.unwrap().remaining_ms(0), 600_000);

        // 300s 後に食事のみ再食（シロップは再観測されるが据え置き）
        tracker.apply_buff_add(11, &food_info(600_000, 0, 1), 300_000, UID);
        refresh(&mut store, &tracker, 300_000);
        assert_eq!(store[&UID].food.unwrap().remaining_ms(300_000), 600_000); // 延長
        assert_eq!(store[&UID].syrup.unwrap().remaining_ms(300_000), 300_000); // 凍結維持
    }

    // buff_tracker.clear（戦闘終了）後も保持し、失効時に除去する。
    #[test]
    fn persists_across_clear_until_expiry() {
        let mut tracker = BuffTracker::new();
        let mut store = HashMap::new();
        tracker.apply_buff_add(1, &food_info(600_000, 1000, 1), 0, UID);
        refresh(&mut store, &tracker, 0);

        tracker.clear(); // 戦闘終了で buff_tracker が空に
        refresh(&mut store, &tracker, 300_000);
        assert_eq!(store[&UID].food.unwrap().remaining_ms(300_000), 300_000);

        refresh(&mut store, &tracker, 600_000); // 失効
        assert!(store.get(&UID).is_none());
    }
}
