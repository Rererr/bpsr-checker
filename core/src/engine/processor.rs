use crate::capture::server::Server;
use crate::engine::class::{Class, ClassSpec, get_class_from_spec, get_class_spec_from_skill_id};
use crate::engine::combat_stats::process_stats;
use crate::engine::encounter::{Encounter, EncounterMutex};
use crate::engine::entity::{Entity, SkillMeta};
use crate::engine::monster_names::MONSTER_NAMES_BOSS;
use crate::engine::name_cache;
use crate::engine::selected_uid;
use crate::error::{AppError, AppResult};
use crate::protocol::constants::{attr_type, entity};
use crate::protocol::opcodes::{Pkt, PktEnvelope};
use crate::protocol::pb::{self, EntityKind};
use bytes::Bytes;
use log::{debug, info, warn};
use prost::Message;
use std::io::Cursor;
use std::time::{SystemTime, UNIX_EPOCH};

/// Get-or-create an entity, pre-populating identity (name/class/score)
/// from the persistent name cache when the entity is freshly created
/// and represents a player. Lets us show real names for players whose
/// ATTR_NAME packets we missed (e.g., started the checker mid-session).
fn get_or_create_entity(
    encounter: &mut Encounter,
    uid: i64,
    entity_type: EntityKind,
) -> &mut Entity {
    let was_new = !encounter.entities.contains_key(&uid);
    let entity = encounter.entities.entry(uid).or_insert_with(|| Entity {
        entity_type,
        ..Default::default()
    });
    if was_new && entity_type == EntityKind::Player {
        if let Some(cached) = name_cache::lookup(uid) {
            if !cached.name.is_empty() {
                entity.name = Some(cached.name);
            }
            if let Some(cid) = cached.class_id {
                if cid != 0 {
                    entity.class = Some(Class::from(cid));
                }
            }
            if let Some(score) = cached.ability_score {
                if score > 0 {
                    entity.ability_score = Some(score);
                }
            }
            if let Some(lv) = cached.season_level {
                if lv > 0 {
                    entity.season_level = Some(lv);
                }
            }
            if let Some(st) = cached.season_strength {
                if st > 0 {
                    entity.season_strength = Some(st);
                }
            }
        }
    }
    entity
}

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

pub(crate) fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn should_accept(encounter: &mut Encounter, conn: Option<Server>, op: &Pkt) -> bool {
    if matches!(op, Pkt::ServerHandover | Pkt::SocialEnvelope) {
        return true;
    }
    let Some(conn) = conn else {
        return true;
    };
    let sel = selected_uid::get();

    // conn の char_id が学習済みなら厳密判定
    if let Some(&uid_for_conn) = encounter.conn_to_uid.get(&conn) {
        return match sel {
            Some(sel_uid) => {
                if uid_for_conn == sel_uid {
                    encounter.active_connection = Some(conn);
                    true
                } else {
                    false
                }
            }
            None => match encounter.active_connection {
                Some(active) => conn == active,
                None => true,
            },
        };
    }

    // 未学習 conn: active_connection が確定しているなら一致のみ通す
    if let Some(active) = encounter.active_connection {
        return conn == active;
    }

    // 完全未確定 (起動直後): 全 accept。WorldEnterSnapshot 受信後に active_connection が確定する
    true
}

pub fn process_opcode(enc: &EncounterMutex, env: PktEnvelope) -> AppResult<()> {
    let PktEnvelope { op, data, conn } = env;

    match op {
        Pkt::ServerHandover => {
            let state = enc;
            let mut encounter = state
                .lock()
                .map_err(|e| AppError::LockPoisoned(e.to_string()))?;
            // ServerHandover でコネクション状態をリセット（新しいサーバ接続 or ログアウト後）
            encounter.active_connection = None;
            encounter.conn_to_uid.clear();
            info!("[ServerHandover] received (encounter retained; use reset to clear)");
        }

        Pkt::SocialEnvelope => {
            let Some(notify) = decode_packet::<pb::SocialEnvelope>(data, "SocialEnvelope") else {
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
                        "[SocialEnvelope] scene changed: line_id={} level_map_id={} (encounter retained)",
                        scene.line_id, scene.level_map_id
                    );
                }
            }
        }

        _ => {
            let state = enc;
            let mut encounter = state
                .lock()
                .map_err(|e| AppError::LockPoisoned(e.to_string()))?;

            if encounter.is_paused {
                return Ok(());
            }

            if !should_accept(&mut encounter, conn, &op) {
                return Ok(());
            }

            match op {
                Pkt::WorldEntityBatch => {
                    let Some(msg) =
                        decode_packet::<pb::WorldEntityBatch>(data, "WorldEntityBatch")
                    else {
                        return Ok(());
                    };
                    process_world_entity_batch(&mut encounter, msg);
                }

                Pkt::WorldEnterSnapshot => {
                    let Some(msg) =
                        decode_packet::<pb::WorldEnterSnapshot>(data, "WorldEnterSnapshot")
                    else {
                        return Ok(());
                    };
                    if let Some(c) = conn {
                        process_world_enter_snapshot(&mut encounter, msg, c);
                    } else {
                        warn!("[WorldEnterSnapshot] conn is None, skipping connection learning");
                    }
                }

                Pkt::LocalDeltaBatch => {
                    let Some(msg) = decode_packet::<pb::LocalDeltaBatch>(data, "LocalDeltaBatch")
                    else {
                        return Ok(());
                    };
                    process_local_delta_batch(&mut encounter, msg);
                }

                Pkt::WorldDeltaBatch => {
                    let Some(msg) = decode_packet::<pb::WorldDeltaBatch>(data, "WorldDeltaBatch")
                    else {
                        return Ok(());
                    };
                    for scene_delta in msg.delta_infos {
                        process_scene_delta(&mut encounter, scene_delta);
                    }
                }

                Pkt::BuffTick => {
                    let ts = now_ms();

                    // BuffSnapshot と BuffTick は同一 op で届き、フィールドが全て varint で
                    // 番号も重なるため protobuf 上どちらの decode も常に成功してしまう。
                    // よって型では判別できず、両方を試す必要がある。誤った型で decode された
                    // 側は host_uuid が Player にならず apply_* 内で無視されるため害はない。
                    // ここを else-if にすると BuffTick 形式のデバフ更新が落ちる（regression 注意）。
                    if let Ok(msg) = pb::BuffSnapshot::decode(data.as_slice()) {
                        encounter.buff_tracker.apply_full_info(&msg, ts);
                    }
                    if let Ok(msg) = pb::BuffTick::decode(data.as_slice()) {
                        encounter.buff_tracker.apply_change(&msg, ts);
                    }
                }

                Pkt::BuffSnapshotBundle => {
                    let ts = now_ms();
                    if let Ok(msg) = pb::BuffSnapshotBundle::decode(data.as_slice()) {
                        for buff in &msg.buff_infos {
                            encounter.buff_tracker.apply_full_info(buff, ts);
                        }
                    }
                }

                _ => {}
            }
        }
    }

    Ok(())
}

fn process_world_entity_batch(encounter: &mut Encounter, msg: pb::WorldEntityBatch) {
    // イマジンデバフタイマー専用モードではエンティティ集計を全て省略
    if crate::engine::runtime_settings::imagine_only_mode() {
        return;
    }

    for pkt_entity in msg.appear {
        let target_uuid = pkt_entity.uuid;
        if target_uuid == 0 {
            continue;
        }
        let target_uid = entity::get_player_uid(target_uuid);
        let target_entity_type = EntityKind::from(target_uuid);

        let target_entity = get_or_create_entity(encounter, target_uid, target_entity_type);
        target_entity.entity_type = target_entity_type;

        if let Some(attrs) = &pkt_entity.attrs {
            match target_entity_type {
                EntityKind::Player => {
                    process_player_attrs(target_uid, target_entity, &attrs.attrs);
                }
                EntityKind::Monster => {
                    process_monster_attrs(target_entity, &attrs.attrs);
                }
                _ => {}
            }
        }
    }
}

fn process_world_enter_snapshot(
    encounter: &mut Encounter,
    msg: pb::WorldEnterSnapshot,
    conn: Server,
) {
    let Some(v_data) = &msg.v_data else {
        return;
    };

    let player_uid = v_data.char_id;
    if player_uid == 0 {
        return;
    }

    // connection ↔ char_id を学習
    encounter.conn_to_uid.insert(conn, player_uid);

    // active_connection の確定
    let sel = selected_uid::get();
    match sel {
        None if encounter.active_connection.is_none() => {
            // 自動検出: 先着固定
            encounter.active_connection = Some(conn);
            encounter.local_player_uid = player_uid;
        }
        Some(sel_uid) if sel_uid == player_uid => {
            // UID 一致: この connection を active に
            encounter.active_connection = Some(conn);
            encounter.local_player_uid = player_uid;
        }
        _ => {
            // 他クライアント由来: エンティティ作成・name_cache 更新をスキップ
            return;
        }
    }

    let target_entity = get_or_create_entity(encounter, player_uid, EntityKind::Player);
    target_entity.entity_type = EntityKind::Player;

    let mut cache_name: Option<String> = None;
    let mut cache_class: Option<i32> = None;
    let mut cache_score: Option<i32> = None;

    if let Some(char_base) = &v_data.char_base {
        if !char_base.name.is_empty() {
            target_entity.name = Some(char_base.name.clone());
            cache_name = Some(char_base.name.clone());
        }
        if char_base.fight_point != 0 {
            target_entity.ability_score = Some(char_base.fight_point);
            cache_score = Some(char_base.fight_point);
        }
    }

    if let Some(profession_list) = &v_data.profession_list {
        if profession_list.cur_profession_id != 0 {
            let player_class = Class::from(profession_list.cur_profession_id);
            target_entity.class = Some(player_class);
            cache_class = Some(profession_list.cur_profession_id);
        }
    }

    name_cache::update(
        player_uid,
        cache_name.as_deref(),
        cache_class,
        cache_score,
        None,
        None,
    );
}

fn process_local_delta_batch(encounter: &mut Encounter, msg: pb::LocalDeltaBatch) {
    let Some(delta_info) = msg.delta_info else {
        return;
    };

    // LocalSceneDelta.effects(field 3): 自プレイヤーへのバフ/デバフ効果リスト
    if !delta_info.effects.is_empty() {
        let ts = now_ms();
        let local_uid = encounter.local_player_uid;
        for effect in &delta_info.effects {
            encounter.buff_tracker.apply_effect(effect, ts, local_uid);
        }
    }

    let Some(base_delta) = delta_info.base_delta else {
        return;
    };
    process_scene_delta(encounter, base_delta);
}

fn process_scene_delta(encounter: &mut Encounter, scene_delta: pb::SceneDelta) {
    let target_uuid = scene_delta.uuid;
    if target_uuid == 0 {
        return;
    }
    let target_uid = entity::get_player_uid(target_uuid);
    let target_entity_type = EntityKind::from(target_uuid);
    let imagine_only = crate::engine::runtime_settings::imagine_only_mode();

    // Process attributes on the target entity（軽量モードではスキップ）
    if !imagine_only {
        let target_entity = get_or_create_entity(encounter, target_uid, target_entity_type);

        if let Some(attrs_collection) = scene_delta.attrs {
            match target_entity_type {
                EntityKind::Player => {
                    process_player_attrs(target_uid, target_entity, &attrs_collection.attrs);
                }
                EntityKind::Monster => {
                    process_monster_attrs(target_entity, &attrs_collection.attrs);
                }
                _ => {}
            }
        }
    }

    // SceneDelta.buff_list: バフイベント (BuffEffect) リスト。
    // 各イベントは BuffEffect.BuffUuid (= buff_uuid, インスタンスキー) で対象バフを識別し、
    // Type (EBuffEventType) と LogicEffect.EffectType (EBuffEffectLogicPbType) で処理を分岐する。
    //   Type==2 (BuffEventRemove): 解除
    //   EffectType==18 (BuffEffectAddBuff): RawData=BuffInfo(=BuffSnapshot) → 付与/再付与
    //   EffectType==19 (BuffEffectBuffChange): RawData=BuffChange{layer,duration,createTime}
    //       → スタック増加・タイマーリフレッシュ（同一 BuffUuid を更新し received_at を再ベース）
    if target_entity_type == EntityKind::Player {
        if let Some(buff_list) = &scene_delta.buff_list {
            const BUFF_EVENT_REMOVE: i32 = 2;
            const LOGIC_EFFECT_ADD_BUFF: i32 = 18;
            const LOGIC_EFFECT_BUFF_CHANGE: i32 = 19;
            let ts = now_ms();
            for buff in &buff_list.buffs {
                let buff_uuid = buff.buff_uuid; // BuffEffect.BuffUuid（インスタンスキー）

                if buff.event_type == BUFF_EVENT_REMOVE {
                    encounter.buff_tracker.remove(target_uid, buff_uuid);
                    continue;
                }

                if buff.body_raw.is_empty() {
                    continue;
                }
                let body = match pb::BuffPayload::decode(buff.body_raw.as_slice()) {
                    Ok(b) => b,
                    Err(_) => continue,
                };
                if body.detail_raw.is_empty() {
                    continue;
                }
                match body.buff_type {
                    LOGIC_EFFECT_ADD_BUFF => {
                        let Ok(info) = pb::BuffSnapshot::decode(body.detail_raw.as_slice()) else {
                            continue;
                        };
                        encounter
                            .buff_tracker
                            .apply_buff_add(buff_uuid, &info, ts, target_uid);
                    }
                    LOGIC_EFFECT_BUFF_CHANGE => {
                        let Ok(change) = pb::BuffChange::decode(body.detail_raw.as_slice()) else {
                            continue;
                        };
                        encounter
                            .buff_tracker
                            .apply_buff_change(target_uid, buff_uuid, &change, ts);
                    }
                    _ => {}
                }
            }
        }
    }

    // 軽量モードでは以降のダメージ/ヒール/時系列集計を全て省略
    if imagine_only {
        return;
    }

    let Some(skill_effect) = scene_delta.skill_effects else {
        return; // no damage in this delta, that's fine
    };

    if !skill_effect.damages.is_empty() {
        let ts = now_ms();
        let timeout_ms = u128::from(
            crate::engine::runtime_settings::COMBAT_EXIT_TIMEOUT_MS
                .load(std::sync::atomic::Ordering::Relaxed),
        );
        if timeout_ms > 0
            && matches!(
                encounter.measure_mode,
                crate::engine::encounter::MeasureMode::Normal
            )
            && encounter.time_last_combat_packet_ms != 0
            && ts.saturating_sub(encounter.time_last_combat_packet_ms) > timeout_ms
        {
            let snapshot = crate::compute::build_encounter_snapshot(encounter);
            let selected = selected_uid::get();
            let should_push = !snapshot.player_rows.is_empty()
                && selected.map_or(true, |_| encounter.has_selected_participant);
            if should_push {
                crate::engine::history::push(snapshot);
            }
            // v0.8.3 以前と同様 clear 後もフォールスルーして当該フレームのダメージを集計する。
            // emit("encounter-reset") は廃止。フロントは次のポーリングで自然に更新される。
            encounter.clear_combat_stats();
        }
    }

    // Process each damage event
    for damage in skill_effect.damages {
        let is_boss = encounter
            .entities
            .get(&target_uid)
            .and_then(|e| e.monster_id)
            .is_some_and(|id| MONSTER_NAMES_BOSS.contains_key(&id));

        let attacker_uuid = if damage.top_summoner_id != 0 {
            damage.top_summoner_id
        } else if damage.attacker_uuid != 0 {
            damage.attacker_uuid
        } else {
            continue; // no attacker — skip
        };
        let attacker_uid = entity::get_player_uid(attacker_uuid);
        let attacker_entity_type = EntityKind::from(attacker_uuid);

        let skill_uid = damage.owner_id;
        if skill_uid == 0 {
            continue;
        }

        // selected_uid 参加判定
        if let Some(sel) = selected_uid::get() {
            if attacker_uid == sel || target_uid == sel {
                encounter.has_selected_participant = true;
            }
        }
        if attacker_entity_type == EntityKind::Player {
            encounter.participant_player_uids.insert(attacker_uid);
        }
        if target_entity_type == EntityKind::Player {
            encounter.participant_player_uids.insert(target_uid);
        }

        let is_heal = damage.r#type == pb::DmgKind::Heal as i32;

        // Encounter-level totals first (avoids holding attacker_entity borrow across encounter.* mutations)
        if is_heal {
            process_stats(&damage, &mut encounter.heal_stats);
        } else {
            process_stats(&damage, &mut encounter.dmg_stats);
            if is_boss {
                process_stats(&damage, &mut encounter.dmg_stats_boss_only);
            }
        }

        // Target-side damage-taken aggregation (player targets only)
        if !is_heal && target_entity_type == EntityKind::Player {
            process_stats(&damage, &mut encounter.dmg_taken_stats);
            let target_entity = get_or_create_entity(encounter, target_uid, target_entity_type);
            process_stats(&damage, &mut target_entity.dmg_taken_stats);
            target_entity.skill_meta.entry(skill_uid).or_insert(SkillMeta {
                property: damage.property as u8,
                damage_mode: damage.damage_mode as u8,
            });
            let by_attacker = target_entity
                .attacker_uid_to_dmg_taken_stats
                .entry(attacker_uid)
                .or_default();
            process_stats(&damage, by_attacker);
            let by_attacker_skill = target_entity
                .attacker_skill_to_dmg_taken_stats
                .entry((attacker_uid, skill_uid))
                .or_default();
            process_stats(&damage, by_attacker_skill);
        }

        let attacker_entity = get_or_create_entity(encounter, attacker_uid, attacker_entity_type);

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

        attacker_entity.skill_meta.entry(skill_uid).or_insert(SkillMeta {
            property: damage.property as u8,
            damage_mode: damage.damage_mode as u8,
        });

        if is_heal {
            let heal_skill = attacker_entity
                .skill_uid_to_heal_stats
                .entry(skill_uid)
                .or_default();
            process_stats(&damage, heal_skill);
            process_stats(&damage, &mut attacker_entity.heal_stats);
        } else {
            let dps_skill = attacker_entity
                .skill_uid_to_dps_stats
                .entry(skill_uid)
                .or_default();
            process_stats(&damage, dps_skill);
            process_stats(&damage, &mut attacker_entity.dmg_stats);
            if is_boss {
                let skill_boss = attacker_entity
                    .skill_uid_to_dps_stats_boss_only
                    .entry(skill_uid)
                    .or_default();
                process_stats(&damage, skill_boss);
                process_stats(&damage, &mut attacker_entity.dmg_stats_boss_only);
            }
        }
    }

    // Update timestamps
    let ts = now_ms();
    if encounter.time_fight_start_ms == 0 {
        encounter.time_fight_start_ms = ts;
        if let crate::engine::encounter::MeasureMode::Pending3Min { duration_ms } =
            encounter.measure_mode
        {
            encounter.measure_mode = crate::engine::encounter::MeasureMode::Active3Min {
                armed_at_ms: ts,
                duration_ms,
            };
            info!("3min measure mode: active (armed_at={ts}ms)");
        }
    }
    encounter.time_last_combat_packet_ms = ts;

    // Time-series sampling
    let interval_ms = u128::from(
        crate::engine::runtime_settings::TS_INTERVAL_MS.load(std::sync::atomic::Ordering::Relaxed),
    );
    if interval_ms > 0 {
        let due = encounter.last_sample_ms == 0
            || ts.saturating_sub(encounter.last_sample_ms) >= interval_ms;
        if due {
            // When first sample, use interval_ms as the window so DPS isn't artificially inflated
            let interval_actual = if encounter.last_sample_ms == 0 {
                interval_ms
            } else {
                ts.saturating_sub(encounter.last_sample_ms)
            };
            let elapsed_since_start = ts.saturating_sub(encounter.time_fight_start_ms);
            let cap = crate::engine::runtime_settings::TS_SAMPLES
                .load(std::sync::atomic::Ordering::Relaxed);

            let dmg_delta = encounter.dmg_stats.total - encounter.last_sample_total_dmg;
            let dps_window = if interval_actual > 0 {
                (dmg_delta as f64) * 1000.0 / (interval_actual as f64)
            } else {
                0.0
            };
            encounter
                .time_series
                .push_back(crate::models::TimeSeriesPoint {
                    t_ms: elapsed_since_start as f64,
                    total_dmg: encounter.dmg_stats.total as f64,
                    total_dps: dps_window.max(0.0),
                });
            while encounter.time_series.len() > cap {
                encounter.time_series.pop_front();
            }

            // Per-entity sampling (only for entities that have dealt damage)
            for entity in encounter.entities.values_mut() {
                if entity.entity_type != EntityKind::Player {
                    continue;
                }
                if entity.dmg_stats.total == 0 && entity.time_series.is_empty() {
                    continue;
                }
                let entity_delta = entity.dmg_stats.total - entity.last_sample_total_dmg;
                let entity_dps = if interval_actual > 0 {
                    (entity_delta as f64) * 1000.0 / (interval_actual as f64)
                } else {
                    0.0
                };
                entity
                    .time_series
                    .push_back(crate::models::TimeSeriesPoint {
                        t_ms: elapsed_since_start as f64,
                        total_dmg: entity.dmg_stats.total as f64,
                        total_dps: entity_dps.max(0.0),
                    });
                while entity.time_series.len() > cap {
                    entity.time_series.pop_front();
                }
                entity.last_sample_total_dmg = entity.dmg_stats.total;

                // Per-skill sampling（スキル別の累積/窓DPS を採取。借用衝突回避のため先に値を収集）
                let skill_samples: Vec<(i32, i64)> = entity
                    .skill_uid_to_dps_stats
                    .iter()
                    .map(|(&uid, s)| (uid, s.total))
                    .collect();
                for (skill_uid, skill_total) in skill_samples {
                    let last = entity.skill_last_sample_total_dmg.entry(skill_uid).or_insert(0);
                    let skill_delta = skill_total - *last;
                    *last = skill_total;
                    let skill_dps = if interval_actual > 0 {
                        (skill_delta as f64) * 1000.0 / (interval_actual as f64)
                    } else {
                        0.0
                    };
                    let series = entity.skill_time_series.entry(skill_uid).or_default();
                    series.push_back(crate::models::TimeSeriesPoint {
                        t_ms: elapsed_since_start as f64,
                        total_dmg: skill_total as f64,
                        total_dps: skill_dps.max(0.0),
                    });
                    while series.len() > cap {
                        series.pop_front();
                    }
                }
            }

            encounter.last_sample_ms = ts;
            encounter.last_sample_total_dmg = encounter.dmg_stats.total;
        }
    }
}

fn process_player_attrs(uid: i64, player_entity: &mut Entity, attrs: &[pb::RawAttr]) {
    use crate::capture::binary_reader::BinaryReader;

    let mut cache_name: Option<String> = None;
    let mut cache_class: Option<i32> = None;
    let mut cache_score: Option<i32> = None;
    let mut cache_season_lv: Option<i32> = None;
    let mut cache_season_str: Option<i32> = None;

    for attr in attrs {
        if attr.raw_data.is_empty() || attr.id == 0 {
            continue;
        }

        match attr.id {
            attr_type::ATTR_NAME => {
                // Skip the leading length byte
                let raw_bytes = attr.raw_data[1..].to_vec();
                match BinaryReader::from(raw_bytes).read_string() {
                    Ok(player_name) => {
                        debug!("Found player name: {player_name}");
                        cache_name = Some(player_name.clone());
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
                    cache_class = Some(class_id);
                }
            }
            attr_type::ATTR_FIGHT_POINT => {
                if let Ok(ability_score) = decode_protobuf_int32(&attr.raw_data) {
                    player_entity.ability_score = Some(ability_score);
                    cache_score = Some(ability_score);
                }
            }
            attr_type::ATTR_SEASON_LEVEL => {
                if let Ok(lv) = decode_protobuf_int32(&attr.raw_data) {
                    player_entity.season_level = Some(lv);
                    cache_season_lv = Some(lv);
                }
            }
            attr_type::ATTR_SEASON_STRENGTH => {
                if let Ok(st) = decode_protobuf_int32(&attr.raw_data) {
                    player_entity.season_strength = Some(st);
                    cache_season_str = Some(st);
                }
            }
            // 自キャラ戦闘ステータス（戦闘中も追従。name_cache には載せない）
            attr_type::ATTR_HP => {
                if let Ok(hp) = decode_protobuf_int64(&attr.raw_data) {
                    if hp >= 0 {
                        player_entity.curr_hp = Some(hp as u64);
                    }
                }
            }
            attr_type::ATTR_MAX_HP => {
                if let Ok(hp) = decode_protobuf_int64(&attr.raw_data) {
                    if hp >= 0 {
                        player_entity.max_hp = Some(hp as u64);
                    }
                }
            }
            attr_type::ATTR_ATTACK_POWER => {
                if let Ok(v) = decode_protobuf_int32(&attr.raw_data) {
                    player_entity.attack_power = Some(v);
                }
            }
            attr_type::ATTR_DEFENSE_POWER => {
                if let Ok(v) = decode_protobuf_int32(&attr.raw_data) {
                    player_entity.defense_power = Some(v);
                }
            }
            attr_type::ATTR_ENDURANCE => {
                if let Ok(v) = decode_protobuf_int32(&attr.raw_data) {
                    player_entity.endurance = Some(v);
                }
            }
            attr_type::ATTR_DEXTERITY => {
                if let Ok(v) = decode_protobuf_int32(&attr.raw_data) {
                    player_entity.dexterity = Some(v);
                }
            }
            attr_type::ATTR_ATTACK_SPEED => {
                if let Ok(v) = decode_protobuf_int32(&attr.raw_data) {
                    player_entity.attack_speed = Some(v);
                }
            }
            attr_type::ATTR_HASTE => {
                if let Ok(v) = decode_protobuf_int32(&attr.raw_data) {
                    player_entity.haste = Some(v);
                }
            }
            attr_type::ATTR_LUCKY => {
                if let Ok(v) = decode_protobuf_int32(&attr.raw_data) {
                    player_entity.lucky = Some(v);
                }
            }
            _ => {}
        }
    }

    if cache_name.is_some()
        || cache_class.is_some()
        || cache_score.is_some()
        || cache_season_lv.is_some()
        || cache_season_str.is_some()
    {
        name_cache::update(
            uid,
            cache_name.as_deref(),
            cache_class,
            cache_score,
            cache_season_lv,
            cache_season_str,
        );
    }
}

fn process_monster_attrs(monster_entity: &mut Entity, attrs: &[pb::RawAttr]) {
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
