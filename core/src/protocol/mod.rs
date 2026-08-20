pub mod constants;
pub mod opcodes;
pub mod packet_parser;

#[allow(clippy::all, non_snake_case)]
pub mod pb;

use crate::protocol::constants::entity;
use crate::protocol::pb::EntityKind;

impl From<i64> for EntityKind {
    fn from(entity_type: i64) -> Self {
        match entity_type & entity::TYPE_MASK as i64 {
            entity::MONSTER_TYPE_CODE => EntityKind::Monster,
            entity::PLAYER_TYPE_CODE => EntityKind::Player,
            _ => EntityKind::Unknown,
        }
    }
}
