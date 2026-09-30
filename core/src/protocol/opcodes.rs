use crate::error::AppError;
use crate::protocol::constants::team_method;

#[non_exhaustive]
#[derive(Debug)]
pub enum Pkt {
    ServerHandover,
    SocialEnvelope,
    WorldEnterScene,
    WorldEntityBatch,
    WorldEnterSnapshot,
    WorldSyncServerTime,
    WorldSyncDungeonData,
    LocalDeltaBatch,
    WorldDeltaBatch,
    BuffTick,
    BuffSnapshotBundle,
    TeamUpdateInfo,
    TeamUpdateMemberInfo,
    TeamJoin,
    TeamLeave,
    TeamDissolve,
}

pub struct PktEnvelope {
    pub op: Pkt,
    pub data: Vec<u8>,
    pub conn: Option<crate::capture::server::Server>,
}

impl TryFrom<u32> for Pkt {
    type Error = AppError;

    fn try_from(pkt: u32) -> Result<Self, Self::Error> {
        Ok(match pkt {
            0x00000003 => Pkt::WorldEnterScene, // EnterScene: 自キャラ入場時のフル属性(PlayerEnt.Attrs)
            0x00000006 => Pkt::WorldEntityBatch,
            0x00000015 => Pkt::WorldEnterSnapshot,
            0x00000017 => Pkt::WorldSyncDungeonData, // SyncDungeonData: ダンジョン難易度(マスター段階)
            0x0000002b => Pkt::WorldSyncServerTime, // SyncServerTime: client/server 時刻同期
            0x0000002d => Pkt::WorldDeltaBatch,
            0x0000002e => Pkt::LocalDeltaBatch,
            0x00003003 => Pkt::BuffTick,
            0x00003005 => Pkt::BuffSnapshotBundle,
            unknown => return Err(AppError::Parse(format!("Unknown opcode: 0x{unknown:08x}"))),
        })
    }
}

impl Pkt {
    /// チーム(PT)通知 service (`TEAM_NTF_SERVICE_ID`) の method を解決する。メインの
    /// `try_from(u32)` はメイン service 専用の名前空間のため、チーム service 由来の method は
    /// ここで別途解決する（packet_parser.rs の service 分岐から呼ぶ）。
    pub fn from_team_method(method: u32) -> Option<Self> {
        Some(match method {
            team_method::UPDATE_TEAM_INFO => Pkt::TeamUpdateInfo,
            team_method::UPDATE_TEAM_MEMBER_INFO => Pkt::TeamUpdateMemberInfo,
            team_method::JOIN_TEAM => Pkt::TeamJoin,
            team_method::LEAVE_TEAM => Pkt::TeamLeave,
            team_method::TEAM_DISSOLVE => Pkt::TeamDissolve,
            _ => return None,
        })
    }
}

#[repr(u16)]
#[non_exhaustive]
#[derive(Debug)]
pub enum FragmentType {
    None = 0,
    Call = 1,
    Notify = 2,
    Return = 3,
    Echo = 4,
    FrameUp = 5,
    FrameDown = 6,
}

impl From<u16> for FragmentType {
    fn from(ft: u16) -> Self {
        match ft {
            1 => FragmentType::Call,
            2 => FragmentType::Notify,
            3 => FragmentType::Return,
            4 => FragmentType::Echo,
            5 => FragmentType::FrameUp,
            6 => FragmentType::FrameDown,
            _ => FragmentType::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // WorldNtf method 0x2B (SyncServerTime) が Pkt::WorldSyncServerTime にマップされる。
    #[test]
    fn try_from_maps_sync_server_time_opcode() {
        assert!(matches!(Pkt::try_from(0x0000002b).unwrap(), Pkt::WorldSyncServerTime));
    }

    // WorldNtf method 0x17 (SyncDungeonData) が Pkt::WorldSyncDungeonData にマップされる。
    #[test]
    fn try_from_maps_sync_dungeon_data_opcode() {
        assert!(matches!(Pkt::try_from(0x00000017).unwrap(), Pkt::WorldSyncDungeonData));
    }
}
