use crate::protocol::constants::entity;
use crate::protocol::pb;
use crate::protocol::pb::EntityKind;
use std::collections::HashMap;

#[derive(Clone, Default, Debug)]
pub struct BuffTracker {
    // player_uid (uuid >> 16) -> buff_uuid -> state
    buffs: HashMap<i64, HashMap<i32, BuffState>>,
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

    /// host_uuid が Player エンティティのバフのみ保存。保存した場合は true を返す。
    pub fn apply_full_info(&mut self, info: &pb::BuffSnapshot, now_ms: u128) -> bool {
        if EntityKind::from(info.host_uuid) != EntityKind::Player {
            return false;
        }
        let player_uid = entity::get_player_uid(info.host_uuid);

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

        self.buffs.entry(player_uid).or_default().insert(info.buff_uuid, state);
        true
    }

    /// 差分更新。host_uuid が Player でない場合は無視する。
    pub fn apply_change(&mut self, change: &pb::BuffTick, now_ms: u128) {
        if EntityKind::from(change.host_uuid) != EntityKind::Player {
            return;
        }
        let player_uid = entity::get_player_uid(change.host_uuid);
        let player_buffs = self.buffs.entry(player_uid).or_default();

        let entry = player_buffs.entry(change.buff_uuid).or_insert_with(|| BuffState {
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

        // v0.8.3 以前と同様、create_time のガードなしで無条件更新。
        // サーバが BuffTick に create_time を付与しない実装の場合（0 で到来）、
        // 旧ガードは全 tick をスキップし残り秒数が凍結していた。
        entry.received_at_local_ms = now_ms;
        entry.duration_ms = change.duration;
        entry.layer = change.layer;
        entry.base_id = change.base_id;
        entry.create_time_server = change.create_time;
    }

    /// SceneDelta.buff_list から取得した BuffPayloadDetail を追跡。
    /// duration_ms > 0 のデバフのみ保存。apply_time が同一 かつ duration_ms も同一なら周期同期スキップ。
    /// apply_time が同じでも duration_ms が増加した場合は延長・再付与と判断して更新する。
    pub fn apply_buff_detail(&mut self, detail: &pb::BuffPayloadDetail, now_ms: u128, target_uid: i64) {
        if detail.duration_ms <= 0 {
            return;
        }
        let id = detail.buff_config_id as i32;
        let player_buffs = self.buffs.entry(target_uid).or_default();
        if let Some(existing) = player_buffs.get(&id) {
            if existing.create_time_server == detail.apply_time && detail.duration_ms <= existing.duration_ms {
                return;
            }
        }
        player_buffs.insert(id, BuffState {
            buff_uuid: id,
            base_id: id,
            host_uuid: detail.target_uuid,
            fire_uuid: 0,
            create_time_server: detail.apply_time,
            received_at_local_ms: now_ms,
            duration_ms: detail.duration_ms,
            layer: 1,
            count: 1,
            source_config_id: 0,
        });
    }

    /// LocalSceneDelta.effects から取得した TimedEffect を追跡（常に local player 宛）。
    /// duration_ms <= 0 は無期限扱いでスキップ。
    /// activated_at が同一 かつ duration_ms も同一なら周期同期スキップ。
    /// activated_at が同じでも duration_ms が増加した場合は延長・再付与と判断して更新する。
    pub fn apply_effect(&mut self, effect: &pb::TimedEffect, now_ms: u128, local_uid: i64) {
        if effect.duration_ms <= 0 {
            return;
        }
        let id = effect.id as i32;
        let player_buffs = self.buffs.entry(local_uid).or_default();
        if let Some(existing) = player_buffs.get(&id) {
            if existing.create_time_server == effect.activated_at && effect.duration_ms <= existing.duration_ms {
                return;
            }
        }
        player_buffs.insert(id, BuffState {
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

    pub fn remove(&mut self, player_uid: i64, buff_uuid: i32) {
        if let Some(player_buffs) = self.buffs.get_mut(&player_uid) {
            player_buffs.remove(&buff_uuid);
        }
    }

    /// 期限切れバフを削除する。duration_ms == 0 は無期限扱いで削除しない。
    /// バフが空になったプレイヤーエントリも除去する。
    pub fn gc(&mut self, now_ms: u128) {
        for player_buffs in self.buffs.values_mut() {
            player_buffs.retain(|_, state| {
                if state.duration_ms == 0 {
                    return true;
                }
                let expire_at = state.received_at_local_ms + state.duration_ms as u128;
                now_ms < expire_at
            });
        }
        self.buffs.retain(|_, player_buffs| !player_buffs.is_empty());
    }

    /// 特定プレイヤーの remaining_ms を計算したスナップショットを返す。
    pub fn snapshot_for(&self, player_uid: i64, now_ms: u128) -> Vec<BuffStateSnapshot> {
        let Some(player_buffs) = self.buffs.get(&player_uid) else {
            return vec![];
        };
        make_snapshots(player_buffs, now_ms)
    }

    /// 全プレイヤーのスナップショットを player_uid ごとに返す。
    pub fn snapshot_all(&self, now_ms: u128) -> HashMap<i64, Vec<BuffStateSnapshot>> {
        self.buffs
            .iter()
            .map(|(uid, player_buffs)| (*uid, make_snapshots(player_buffs, now_ms)))
            .collect()
    }

    pub fn clear(&mut self) {
        self.buffs.clear();
    }
}

fn make_snapshots(player_buffs: &HashMap<i32, BuffState>, now_ms: u128) -> Vec<BuffStateSnapshot> {
    player_buffs
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_buff_info(buff_uuid: i32, host_uuid: i64, duration: i32) -> pb::BuffSnapshot {
        pb::BuffSnapshot {
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

    // player_uid << 16 | 640 でプレイヤー packed UUID を構成する
    fn player_uuid(player_uid: i64) -> i64 {
        (player_uid << 16) | 640
    }

    // monster_uid << 16 | 64 でモンスター packed UUID を構成する
    fn monster_uuid(monster_uid: i64) -> i64 {
        (monster_uid << 16) | 64
    }

    #[test]
    fn test_player_uuid_filter() {
        let mut tracker = BuffTracker::new();
        let uid_a = entity::get_player_uid(player_uuid(1));
        let uid_b = entity::get_player_uid(player_uuid(2));

        let info_a = make_buff_info(1, player_uuid(1), 5000);
        let info_b = make_buff_info(2, player_uuid(2), 5000);
        let info_monster = make_buff_info(3, monster_uuid(99), 5000);

        assert!(tracker.apply_full_info(&info_a, 0));
        assert!(tracker.apply_full_info(&info_b, 0));
        assert!(!tracker.apply_full_info(&info_monster, 0));

        // uid_a と uid_b は独立して保持される
        assert_eq!(tracker.snapshot_for(uid_a, 0).len(), 1);
        assert_eq!(tracker.snapshot_for(uid_b, 0).len(), 1);
        assert_eq!(tracker.snapshot_for(uid_a, 0)[0].buff_uuid, 1);
        assert_eq!(tracker.snapshot_for(uid_b, 0)[0].buff_uuid, 2);

        // モンスターは保存されない
        let all = tracker.snapshot_all(0);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_gc_removes_expired() {
        let mut tracker = BuffTracker::new();
        let uuid = player_uuid(1);
        let uid = entity::get_player_uid(uuid);

        let info = make_buff_info(1, uuid, 1000);
        tracker.apply_full_info(&info, 0);

        tracker.gc(999);
        assert_eq!(tracker.snapshot_for(uid, 999).len(), 1);

        tracker.gc(1000);
        assert_eq!(tracker.snapshot_for(uid, 1000).len(), 0);
        // 空になったプレイヤーエントリも除去
        assert!(tracker.snapshot_all(1000).is_empty());
    }

    #[test]
    fn test_snapshot_remaining_ms() {
        let mut tracker = BuffTracker::new();
        let uuid = player_uuid(1);
        let uid = entity::get_player_uid(uuid);

        let info = make_buff_info(1, uuid, 5000);
        tracker.apply_full_info(&info, 0);

        let snaps = tracker.snapshot_for(uid, 2000);
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].remaining_ms, 3000);
    }

    #[test]
    fn test_refresh_resets_received_at() {
        let mut tracker = BuffTracker::new();
        let uuid = player_uuid(1);
        let uid = entity::get_player_uid(uuid);

        let info = make_buff_info(1, uuid, 1000);
        tracker.apply_full_info(&info, 0);
        tracker.apply_full_info(&info, 800);

        let snaps = tracker.snapshot_for(uid, 1500);
        assert_eq!(snaps[0].remaining_ms, 300);
    }

    #[test]
    fn test_duration_zero_is_permanent() {
        let mut tracker = BuffTracker::new();
        let uuid = player_uuid(1);
        let uid = entity::get_player_uid(uuid);

        let info = make_buff_info(1, uuid, 0);
        tracker.apply_full_info(&info, 0);

        tracker.gc(u128::MAX / 2);
        assert_eq!(tracker.snapshot_for(uid, u128::MAX / 2).len(), 1);
    }

    #[test]
    fn test_two_players_isolated() {
        let mut tracker = BuffTracker::new();
        let uuid_a = player_uuid(1);
        let uuid_b = player_uuid(2);
        let uid_a = entity::get_player_uid(uuid_a);
        let uid_b = entity::get_player_uid(uuid_b);

        tracker.apply_full_info(&make_buff_info(10, uuid_a, 5000), 0);
        tracker.apply_full_info(&make_buff_info(10, uuid_b, 2000), 0);

        // 同じ buff_uuid でも別プレイヤーとして独立
        let snap_a = tracker.snapshot_for(uid_a, 0);
        let snap_b = tracker.snapshot_for(uid_b, 0);
        assert_eq!(snap_a[0].duration_ms, 5000);
        assert_eq!(snap_b[0].duration_ms, 2000);
    }
}
