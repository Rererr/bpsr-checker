//! パーティ(PT)構成の追跡。TeamNtf service(0x399fca69) の通知から構築する
//! （processor.rs の process_opcode / packet_parser.rs の team 分岐参照）。
//! メンバーは char_id（=プレイヤー uid。バフ/DPS集計側の uuid ではない）で識別する。

/// 現在のパーティ構成。`Encounter::team` として保持し、戦闘リセット
/// (`clear_combat_stats`)・ServerHandover を跨いで消さない（PT構成はキャラ選択・戦闘状態と
/// 無関係にアプリ全体で使うため。consumables と同じ扱い）。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TeamState {
    pub team_id: i64,
    pub leader_uid: i64,
    pub member_uids: Vec<i64>,
}

impl TeamState {
    /// uid を重複無く末尾へ追加する（0 は無効値として追加しない）。
    fn add_member(&mut self, uid: i64) {
        if uid != 0 && !self.member_uids.contains(&uid) {
            self.member_uids.push(uid);
        }
    }

    /// NoticeUpdateTeamInfo(0x1): team_id/leader を更新し、leader もメンバーへ追加する。
    pub fn update_info(&mut self, team_id: i64, leader_uid: i64) {
        self.team_id = team_id;
        self.leader_uid = leader_uid;
        self.add_member(leader_uid);
    }

    /// NoticeUpdateTeamMemberInfo(0x2): 差分の upsert（追加のみ。離脱はこの経路では届かない）。
    pub fn update_members(&mut self, uids: impl IntoIterator<Item = i64>) {
        for uid in uids {
            self.add_member(uid);
        }
    }

    /// NotifyJoinTeam(0x3): team_id/leader を設定し、メンバーを全置換する（leader も含める）。
    pub fn join(&mut self, team_id: i64, leader_uid: i64, uids: impl IntoIterator<Item = i64>) {
        self.team_id = team_id;
        self.leader_uid = leader_uid;
        self.member_uids.clear();
        for uid in uids {
            self.add_member(uid);
        }
        self.add_member(leader_uid);
    }

    /// NotifyLeaveTeam(0x4): uid が自分自身(local_uid)なら PT情報を全クリア（自分が抜けた＝
    /// PT自体が自分にとって無意味になる）。他人なら該当 uid だけメンバーから除去する。
    pub fn leave(&mut self, uid: i64, local_uid: i64) {
        // local_uid が未確定(0)のときは離脱者が自分かどうか判定できない。安全側として
        // PT情報ごとクリアする（PT外行を誤表示するより、PTメンバー行が一時的に隠れる方を
        // 選ぶ）。ダンジョンでは NoticeUpdateTeamMemberInfo(0x2) が周期的に来るため
        // member_uids は自己回復する。
        if local_uid == 0 {
            *self = TeamState::default();
            return;
        }
        if uid != 0 && uid == local_uid {
            *self = TeamState::default();
            return;
        }
        self.member_uids.retain(|&m| m != uid);
    }

    /// NoticeTeamDissolve(0xd): PT情報を全クリア。
    pub fn dissolve(&mut self) {
        *self = TeamState::default();
    }

    /// uid が現在のPTメンバーか。
    pub fn is_member(&self, uid: i64) -> bool {
        self.member_uids.contains(&uid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_replaces_members_and_adds_leader() {
        let mut t = TeamState::default();
        t.join(100, 1, [1, 2, 3]);
        assert_eq!(t.team_id, 100);
        assert_eq!(t.leader_uid, 1);
        assert_eq!(t.member_uids, vec![1, 2, 3]);
        assert!(t.is_member(2));
        assert!(!t.is_member(99));

        // 2回目の join は全置換（前回のメンバーは残らない）。
        t.join(200, 4, [4, 5]);
        assert_eq!(t.team_id, 200);
        assert_eq!(t.member_uids, vec![4, 5]);
        assert!(!t.is_member(1));
    }

    #[test]
    fn update_members_dedups_and_upserts() {
        let mut t = TeamState::default();
        t.update_members([1, 2]);
        t.update_members([2, 3]);
        assert_eq!(t.member_uids, vec![1, 2, 3]);
    }

    #[test]
    fn leave_self_clears_all() {
        let mut t = TeamState::default();
        t.join(100, 1, [1, 2, 3]);
        t.leave(1, 1);
        assert_eq!(t, TeamState::default());
    }

    // local_uid が未確定(0)のときは離脱者が自分かどうか判定できないため、安全側で
    // PT情報ごとクリアする。
    #[test]
    fn leave_with_unknown_local_uid_clears_all() {
        let mut t = TeamState::default();
        t.join(100, 1, [1, 2, 3]);
        t.leave(2, 0);
        assert_eq!(t, TeamState::default());
    }

    #[test]
    fn leave_other_removes_only_that_member() {
        let mut t = TeamState::default();
        t.join(100, 1, [1, 2, 3]);
        t.leave(2, 1);
        assert_eq!(t.team_id, 100);
        assert_eq!(t.member_uids, vec![1, 3]);
        assert!(!t.is_member(2));
    }

    #[test]
    fn dissolve_clears_all() {
        let mut t = TeamState::default();
        t.join(100, 1, [1, 2, 3]);
        t.dissolve();
        assert_eq!(t, TeamState::default());
    }

    #[test]
    fn update_info_sets_team_and_adds_leader() {
        let mut t = TeamState::default();
        t.update_members([2, 3]);
        t.update_info(100, 1);
        assert_eq!(t.team_id, 100);
        assert_eq!(t.leader_uid, 1);
        assert_eq!(t.member_uids, vec![2, 3, 1]);
    }
}
