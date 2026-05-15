use crate::protocol::constants::entity;
use crate::protocol::pb;
use std::collections::HashMap;

#[derive(Clone, Default, Debug)]
pub struct BuffTracker {
    buffs: HashMap<i32, BuffState>,
}

#[derive(Clone, Debug)]
pub struct BuffState {
    pub buff_uuid: i32,
    pub base_id: i32,
    pub host_uuid: i64,
    pub fire_uuid: i64,
    pub create_time_server: i64,
    pub received_at_local_ms: u128,
    pub duration_ms: i64,
    pub layer: i32,
    pub count: i32,
    pub source_config_id: i32,
}

#[derive(Clone)]
pub struct BuffStateSnapshot {
    pub buff_uuid: i32,
    pub base_id: i32,
    pub fire_uuid: i64,
    pub received_at_local_ms: u128,
    pub duration_ms: i64,
    pub remaining_ms: i64,
    pub layer: i32,
    pub count: i32,
}

impl BuffTracker {
    pub fn new() -> Self {
        Self {
            buffs: HashMap::new(),
        }
    }

    /// host_uuid が local_uid と一致するバフのみ保存。
    /// 保存した場合は true を返す。
    pub fn apply_full_info(&mut self, info: &pb::BuffInfo, now_ms: u128, local_uid: i64) -> bool {
        // host_uuid は entity UUID (packed)、local_uid は char_id (>> 16 済み)
        if entity::get_player_uid(info.host_uuid) != local_uid {
            return false;
        }

        let source_config_id = info
            .fight_source_info
            .as_ref()
            .map(|s| s.source_config_id)
            .unwrap_or(0);

        let state = BuffState {
            buff_uuid: info.buff_uuid,
            base_id: info.base_id,
            host_uuid: info.host_uuid,
            fire_uuid: info.fire_uuid,
            create_time_server: info.create_time,
            received_at_local_ms: now_ms,
            duration_ms: info.duration as i64,
            layer: info.layer,
            count: info.count,
            source_config_id,
        };

        self.buffs.insert(info.buff_uuid, state);
        true
    }

    /// 差分更新。host_uuid が local_uid と異なる場合は無視する。
    pub fn apply_change(&mut self, change: &pb::BuffChangeNotify, now_ms: u128, local_uid: i64) {
        if entity::get_player_uid(change.host_uuid) != local_uid {
            return;
        }

        let entry = self.buffs.entry(change.buff_uuid).or_insert_with(|| BuffState {
            buff_uuid: change.buff_uuid,
            base_id: change.base_id,
            host_uuid: change.host_uuid,
            fire_uuid: 0,
            create_time_server: change.create_time,
            received_at_local_ms: now_ms,
            duration_ms: change.duration,
            layer: change.layer,
            count: 0,
            source_config_id: 0,
        });

        // 既存エントリのリフレッシュ
        entry.received_at_local_ms = now_ms;
        entry.duration_ms = change.duration;
        entry.layer = change.layer;
        entry.base_id = change.base_id;
        entry.create_time_server = change.create_time;
    }

    /// AoiSyncToMeDelta.effects から取得した EffectInfo を追跡。
    /// duration_ms <= 0 は無期限扱いでスキップ（デバフ表示対象外）。
    /// 同じ activated_at なら周期同期なので時刻をリセットしない。
    pub fn apply_effect(&mut self, effect: &pb::EffectInfo, now_ms: u128) {
        if effect.duration_ms <= 0 {
            return;
        }
        let id = effect.id as i32;
        if let Some(existing) = self.buffs.get(&id) {
            if existing.create_time_server == effect.activated_at {
                return;
            }
        }
        self.buffs.insert(id, BuffState {
            buff_uuid: id,
            base_id: id,
            host_uuid: 0,
            fire_uuid: 0,
            create_time_server: effect.activated_at,
            received_at_local_ms: now_ms,
            duration_ms: effect.duration_ms,
            layer: 1,
            count: 1,
            source_config_id: 0,
        });
    }

    pub fn remove(&mut self, buff_uuid: i32) {
        self.buffs.remove(&buff_uuid);
    }

    /// 期限切れバフを削除する。duration_ms == 0 は無期限扱いで削除しない。
    pub fn gc(&mut self, now_ms: u128) {
        self.buffs.retain(|_, state| {
            if state.duration_ms == 0 {
                return true;
            }
            let expire_at = state.received_at_local_ms + state.duration_ms as u128;
            now_ms < expire_at
        });
    }

    /// remaining_ms を計算したスナップショットを返す。
    pub fn snapshot(&self, now_ms: u128) -> Vec<BuffStateSnapshot> {
        self.buffs
            .values()
            .map(|state| {
                let remaining_ms = if state.duration_ms == 0 {
                    i64::MAX
                } else {
                    let expire_at = state.received_at_local_ms + state.duration_ms as u128;
                    (expire_at as i128 - now_ms as i128) as i64
                };

                BuffStateSnapshot {
                    buff_uuid: state.buff_uuid,
                    base_id: state.base_id,
                    fire_uuid: state.fire_uuid,
                    received_at_local_ms: state.received_at_local_ms,
                    duration_ms: state.duration_ms,
                    remaining_ms,
                    layer: state.layer,
                    count: state.count,
                }
            })
            .collect()
    }

    pub fn clear(&mut self) {
        self.buffs.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_buff_info(buff_uuid: i32, host_uuid: i64, duration: i32) -> pb::BuffInfo {
        pb::BuffInfo {
            buff_uuid,
            base_id: 30001,
            level: 1,
            host_uuid,
            table_uuid: 0,
            create_time: 0,
            fire_uuid: 99,
            layer: 1,
            part_id: 0,
            count: 1,
            duration,
            fight_source_info: None,
        }
    }

    #[test]
    fn test_host_uuid_filter() {
        let mut tracker = BuffTracker::new();
        let local_uid: i64 = 1000;

        let info_self = make_buff_info(1, local_uid, 5000);
        let info_other = make_buff_info(2, 9999, 5000);

        assert!(tracker.apply_full_info(&info_self, 0, local_uid));
        assert!(!tracker.apply_full_info(&info_other, 0, local_uid));

        let snaps = tracker.snapshot(0);
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].buff_uuid, 1);
    }

    #[test]
    fn test_gc_removes_expired() {
        let mut tracker = BuffTracker::new();
        let local_uid: i64 = 1000;

        // duration 1000ms、received_at 0ms → 1000ms で期限切れ
        let info = make_buff_info(1, local_uid, 1000);
        tracker.apply_full_info(&info, 0, local_uid);

        // 999ms ではまだ残っている
        tracker.gc(999);
        assert_eq!(tracker.snapshot(999).len(), 1);

        // 1000ms で期限切れ
        tracker.gc(1000);
        assert_eq!(tracker.snapshot(1000).len(), 0);
    }

    #[test]
    fn test_snapshot_remaining_ms() {
        let mut tracker = BuffTracker::new();
        let local_uid: i64 = 1000;

        // received_at=0, duration=5000ms
        let info = make_buff_info(1, local_uid, 5000);
        tracker.apply_full_info(&info, 0, local_uid);

        // now=2000ms → remaining=3000ms
        let snaps = tracker.snapshot(2000);
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].remaining_ms, 3000);
    }

    #[test]
    fn test_refresh_resets_received_at() {
        let mut tracker = BuffTracker::new();
        let local_uid: i64 = 1000;

        let info = make_buff_info(1, local_uid, 1000);
        tracker.apply_full_info(&info, 0, local_uid);

        // 800ms 経過後に同じ buff_uuid で再 apply（リフレッシュ）
        tracker.apply_full_info(&info, 800, local_uid);

        // now=1500ms: received_at=800, duration=1000 → remaining=300
        let snaps = tracker.snapshot(1500);
        assert_eq!(snaps[0].remaining_ms, 300);
    }

    #[test]
    fn test_duration_zero_is_permanent() {
        let mut tracker = BuffTracker::new();
        let local_uid: i64 = 1000;

        let info = make_buff_info(1, local_uid, 0);
        tracker.apply_full_info(&info, 0, local_uid);

        // 大きな now_ms でも gc で削除されない
        tracker.gc(u128::MAX / 2);
        assert_eq!(tracker.snapshot(u128::MAX / 2).len(), 1);
    }
}
