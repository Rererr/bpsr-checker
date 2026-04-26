use crate::engine::class::{
    Class, ClassSpec, get_class_from_spec, get_class_spec_from_skill_id,
};
use crate::engine::combat_stats::process_stats;
use crate::engine::encounter::{Encounter, EncounterMutex};
use crate::engine::entity::Entity;
use crate::engine::monster_names::MONSTER_NAMES_BOSS;
use crate::error::{AppError, AppResult};
use crate::protocol::constants::{attr_type, entity};
use crate::protocol::opcodes::Pkt;
use crate::protocol::pb::{self, EEntityType};
use bytes::Bytes;
use log::{debug, info, warn};
use prost::Message;
use std::io::Cursor;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

fn decode_packet<T: Message + Default>(data: Vec<u8>, packet_name: &str) -> Option<T> {
    match T::decode(Bytes::from(data)) {
        Ok(v) => Some(v),
        Err(e) => {
            warn!("Error decoding {packet_name}, ignoring: {e}");
            None
        }
    }
}

fn decode_protobuf_int32(data: &[u8]) -> AppResult<i32> {
    if data.is_empty() {
        return Err(AppError::Parse("Empty data for protobuf int32".into()));
    }
    let mut cursor = Cursor::new(data);
    prost::encoding::decode_varint(&mut cursor)
        .map(|v| v as i32)
        .map_err(|e| AppError::Parse(format!("decode_varint i32: {e}")))
}

fn decode_protobuf_int64(data: &[u8]) -> AppResult<i64> {
    if data.is_empty() {
        return Err(AppError::Parse("Empty data for protobuf int64".into()));
    }
    let mut cursor = Cursor::new(data);
    prost::encoding::decode_varint(&mut cursor)
        .map(|v| v as i64)
        .map_err(|e| AppError::Parse(format!("decode_varint i64: {e}")))
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

pub fn process_opcode(app_handle: &AppHandle, op: Pkt, data: Vec<u8>) -> AppResult<()> {
    match op {
        Pkt::ServerChangeInfo => {
            let state = app_handle.state::<EncounterMutex>();
            let mut encounter = state
                .lock()
                .map_err(|e| AppError::LockPoisoned(e.to_string()))?;
            on_server_change(&mut encounter);
        }

        Pkt::NotifySocialData => {
            let Some(notify) = decode_packet::<pb::NotifySocialData>(data, "NotifySocialData")
            else {
                return Ok(());
            };

            let scene_data = notify
                .v_request
                .as_ref()
                .and_then(|r| r.data.as_ref())
                .and_then(|s| s.scene_data.as_ref());

            if let Some(scene) = scene_data {
                if scene.line_id != 0 {
                    info!(
                        "[SocialNtf] scene changed: line_id={} level_map_id={}",
                        scene.line_id, scene.level_map_id
                    );
                    let state = app_handle.state::<EncounterMutex>();
                    let mut encounter = state
                        .lock()
                        .map_err(|e| AppError::LockPoisoned(e.to_string()))?;
                    encounter.entities.clear();
                }
            }
        }

        Pkt::SyncNearEntities => {
            let Some(msg) =
                decode_packet::<pb::SyncNearEntities>(data, "SyncNearEntities")
            else {
                return Ok(());
            };
            let state = app_handle.state::<EncounterMutex>();
            let mut encounter = state
                .lock()
                .map_err(|e| AppError::LockPoisoned(e.to_string()))?;
            process_sync_near_entities(&mut encounter, msg);
        }

        Pkt::SyncContainerData => {
            let Some(msg) =
                decode_packet::<pb::SyncContainerData>(data, "SyncContainerData")
            else {
                return Ok(());
            };
            let state = app_handle.state::<EncounterMutex>();
            let mut encounter = state
                .lock()
                .map_err(|e| AppError::LockPoisoned(e.to_string()))?;
            process_sync_container_data(&mut encounter, msg);
        }

        Pkt::SyncToMeDeltaInfo => {
            let Some(msg) =
                decode_packet::<pb::SyncToMeDeltaInfo>(data, "SyncToMeDeltaInfo")
            else {
                return Ok(());
            };
            let state = app_handle.state::<EncounterMutex>();
            let mut encounter = state
                .lock()
                .map_err(|e| AppError::LockPoisoned(e.to_string()))?;
            process_sync_to_me_delta_info(&mut encounter, msg);
        }

        Pkt::SyncNearDeltaInfo => {
            let Some(msg) =
                decode_packet::<pb::SyncNearDeltaInfo>(data, "SyncNearDeltaInfo")
            else {
                return Ok(());
            };
            let state = app_handle.state::<EncounterMutex>();
            let mut encounter = state
                .lock()
                .map_err(|e| AppError::LockPoisoned(e.to_string()))?;
            for aoi_sync_delta in msg.delta_infos {
                process_aoi_sync_delta(&mut encounter, aoi_sync_delta);
            }
        }
    }

    Ok(())
}

fn on_server_change(encounter: &mut Encounter) {
    info!("on server change");
    encounter.clone_from(&Encounter::default());
}

fn process_sync_near_entities(encounter: &mut Encounter, msg: pb::SyncNearEntities) {
    for pkt_entity in msg.appear {
        let target_uuid = pkt_entity.uuid;
        if target_uuid == 0 {
            continue;
        }
        let target_uid = entity::get_player_uid(target_uuid);
        let target_entity_type = EEntityType::from(target_uuid);

        let target_entity = encounter.entities.entry(target_uid).or_default();
        target_entity.entity_type = target_entity_type;

        if let Some(attrs) = &pkt_entity.attrs {
            match target_entity_type {
                EEntityType::EntChar => {
                    process_player_attrs(target_entity, attrs.attrs.clone());
                }
                EEntityType::EntMonster => {
                    process_monster_attrs(target_entity, attrs.attrs.clone());
                }
                _ => {}
            }
        }
    }
}

fn process_sync_container_data(encounter: &mut Encounter, msg: pb::SyncContainerData) {
    let Some(v_data) = &msg.v_data else {
        return;
    };

    let player_uid = v_data.char_id;
    if player_uid == 0 {
        return;
    }

    let target_entity = encounter.entities.entry(player_uid).or_default();
    target_entity.entity_type = EEntityType::EntChar;

    if let Some(char_base) = &v_data.char_base {
        if !char_base.name.is_empty() {
            target_entity.name = Some(char_base.name.clone());
        }
        if char_base.fight_point != 0 {
            target_entity.ability_score = Some(char_base.fight_point);
        }
    }

    if let Some(profession_list) = &v_data.profession_list {
        if profession_list.cur_profession_id != 0 {
            let player_class = Class::from(profession_list.cur_profession_id);
            target_entity.class = Some(player_class);
        }
    }
}

fn process_sync_to_me_delta_info(encounter: &mut Encounter, msg: pb::SyncToMeDeltaInfo) {
    let Some(delta_info) = msg.delta_info else {
        return;
    };
    let Some(base_delta) = delta_info.base_delta else {
        return;
    };
    process_aoi_sync_delta(encounter, base_delta);
}

fn process_aoi_sync_delta(encounter: &mut Encounter, aoi_sync_delta: pb::AoiSyncDelta) {
    let target_uuid = aoi_sync_delta.uuid;
    if target_uuid == 0 {
        return;
    }
    let target_uid = entity::get_player_uid(target_uuid);
    let target_entity_type = EEntityType::from(target_uuid);

    // Process attributes on the target entity
    {
        let target_entity = encounter
            .entities
            .entry(target_uid)
            .or_insert_with(|| Entity {
                entity_type: target_entity_type,
                ..Default::default()
            });

        if let Some(attrs_collection) = aoi_sync_delta.attrs {
            match target_entity_type {
                EEntityType::EntChar => {
                    process_player_attrs(target_entity, attrs_collection.attrs);
                }
                EEntityType::EntMonster => {
                    process_monster_attrs(target_entity, attrs_collection.attrs);
                }
                _ => {}
            }
        }
    }

    let Some(skill_effect) = aoi_sync_delta.skill_effects else {
        return; // no damage in this delta, that's fine
    };

    // Process each damage event
    for sync_damage_info in skill_effect.damages {
        let is_boss = encounter
            .entities
            .get(&target_uid)
            .and_then(|e| e.monster_id)
            .is_some_and(|id| MONSTER_NAMES_BOSS.contains_key(&id));

        let attacker_uuid = if sync_damage_info.top_summoner_id != 0 {
            sync_damage_info.top_summoner_id
        } else if sync_damage_info.attacker_uuid != 0 {
            sync_damage_info.attacker_uuid
        } else {
            continue; // no attacker — skip
        };
        let attacker_uid = entity::get_player_uid(attacker_uuid);

        let attacker_entity = encounter
            .entities
            .entry(attacker_uid)
            .or_insert_with(|| Entity {
                entity_type: EEntityType::from(attacker_uuid),
                ..Default::default()
            });

        let skill_uid = sync_damage_info.owner_id;
        if skill_uid == 0 {
            continue;
        }

        // Infer class spec from skill id
        if attacker_entity
            .class_spec
            .is_none_or(|cs| cs == ClassSpec::Unknown)
        {
            let class_spec = get_class_spec_from_skill_id(skill_uid);
            attacker_entity.class_spec = Some(class_spec);

            if attacker_entity
                .class
                .is_none_or(|c| matches!(c, Class::Unknown | Class::Unimplemented))
            {
                attacker_entity.class = Some(get_class_from_spec(class_spec));
            }
        }

        let is_heal = sync_damage_info.r#type == pb::EDamageType::Heal as i32;
        if is_heal {
            let heal_skill = attacker_entity
                .skill_uid_to_heal_stats
                .entry(skill_uid)
                .or_default();
            process_stats(&sync_damage_info, heal_skill);
            process_stats(&sync_damage_info, &mut attacker_entity.heal_stats);
            process_stats(&sync_damage_info, &mut encounter.heal_stats);
        } else {
            let dps_skill = attacker_entity
                .skill_uid_to_dps_stats
                .entry(skill_uid)
                .or_default();
            process_stats(&sync_damage_info, dps_skill);
            process_stats(&sync_damage_info, &mut attacker_entity.dmg_stats);
            process_stats(&sync_damage_info, &mut encounter.dmg_stats);
            if is_boss {
                let skill_boss = attacker_entity
                    .skill_uid_to_dps_stats_boss_only
                    .entry(skill_uid)
                    .or_default();
                process_stats(&sync_damage_info, skill_boss);
                process_stats(&sync_damage_info, &mut attacker_entity.dmg_stats_boss_only);
                process_stats(&sync_damage_info, &mut encounter.dmg_stats_boss_only);
            }
        }
    }

    // Update timestamps
    let ts = now_ms();
    if encounter.time_fight_start_ms == 0 {
        encounter.time_fight_start_ms = ts;
    }
    encounter.time_last_combat_packet_ms = ts;
}

fn process_player_attrs(player_entity: &mut Entity, attrs: Vec<pb::Attr>) {
    use crate::capture::binary_reader::BinaryReader;

    for attr in attrs {
        if attr.raw_data.is_empty() || attr.id == 0 {
            continue;
        }

        match attr.id {
            attr_type::ATTR_NAME => {
                let mut raw_bytes = attr.raw_data;
                // Skip the leading length byte
                raw_bytes.remove(0);
                match BinaryReader::from(raw_bytes).read_string() {
                    Ok(player_name) => {
                        debug!("Found player name: {player_name}");
                        player_entity.name = Some(player_name);
                    }
                    Err(e) => {
                        warn!("Failed to read player name: {e}");
                    }
                }
            }
            attr_type::ATTR_PROFESSION_ID => {
                if let Ok(class_id) = decode_protobuf_int32(&attr.raw_data) {
                    player_entity.class = Some(Class::from(class_id));
                }
            }
            attr_type::ATTR_FIGHT_POINT => {
                if let Ok(ability_score) = decode_protobuf_int32(&attr.raw_data) {
                    player_entity.ability_score = Some(ability_score);
                }
            }
            _ => {}
        }
    }
}

fn process_monster_attrs(monster_entity: &mut Entity, attrs: Vec<pb::Attr>) {
    for attr in attrs {
        if attr.raw_data.is_empty() || attr.id == 0 {
            continue;
        }

        match attr.id {
            attr_type::ATTR_ID => {
                if let Ok(id) = decode_protobuf_int32(&attr.raw_data) {
                    if id >= 0 {
                        monster_entity.monster_id = Some(id as u32);
                    }
                }
            }
            attr_type::ATTR_HP => {
                if let Ok(curr_hp) = decode_protobuf_int64(&attr.raw_data) {
                    if curr_hp >= 0 {
                        monster_entity.curr_hp = Some(curr_hp as u64);
                    }
                }
            }
            attr_type::ATTR_MAX_HP => {
                if let Ok(max_hp) = decode_protobuf_int64(&attr.raw_data) {
                    if max_hp >= 0 {
                        monster_entity.max_hp = Some(max_hp as u64);
                    }
                }
            }
            _ => {}
        }
    }
}
