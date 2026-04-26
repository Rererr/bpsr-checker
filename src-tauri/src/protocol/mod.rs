pub mod constants;
pub mod opcodes;
pub mod packet_parser;

#[allow(clippy::all, non_snake_case)]
pub mod pb;

use crate::protocol::constants::entity;
use crate::protocol::pb::EEntityType;

impl From<i64> for EEntityType {
    fn from(entity_type: i64) -> Self {
        match entity_type & entity::TYPE_MASK as i64 {
            64 => EEntityType::EntMonster,
            640 => EEntityType::EntChar,
            _ => EEntityType::EntErrType,
        }
    }
}
