use crate::error::AppError;

#[non_exhaustive]
#[derive(Debug)]
pub enum Pkt {
    ServerChangeInfo,
    NotifySocialData,
    SyncNearEntities,
    SyncContainerData,
    SyncToMeDeltaInfo,
    SyncNearDeltaInfo,
    NotifyBuffChange,
    SyncBuffInfo,
    SyncEntityState,
}

pub struct PktEnvelope {
    pub op: Pkt,
    pub data: Vec<u8>,
    pub conn: Option<crate::capture::server::Server>,
}

impl TryFrom<u32> for Pkt {
    type Error = AppError;

    fn try_from(pkt: u32) -> Result<Self, Self::Error> {
        match pkt {
            0x00000006 => Ok(Pkt::SyncNearEntities),
            0x00000015 => Ok(Pkt::SyncContainerData),
            0x0000002e => Ok(Pkt::SyncToMeDeltaInfo),
            0x0000002d => Ok(Pkt::SyncNearDeltaInfo),
            0x00003003 => Ok(Pkt::NotifyBuffChange),
            0x00003005 => Ok(Pkt::SyncBuffInfo),
            0x0000002b => Ok(Pkt::SyncEntityState),
            _ => Err(AppError::Parse(format!("Unknown opcode: 0x{pkt:08x}"))),
        }
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
