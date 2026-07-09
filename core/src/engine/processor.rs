use crate::capture::server::Server;
use crate::engine::class::{Class, ClassSpec, get_class_from_spec, get_class_spec_from_skill_id};
use crate::engine::combat_stats::process_stats;
use crate::engine::encounter::{Encounter, EncounterMutex};
use crate::engine::entity::{Entity, ImagineSlot, MAX_IMAGINE_NAMES, SkillMeta};
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
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// バトルイマジン検知の鮮度比較用シーケンス番号。wall-clock ではなく単調増加カウンタにする
/// のは、同一ミリ秒に複数検知が起きるとテスト・実戦とも鮮度が潰れて順序判定できなくなるため。
static IMAGINE_DETECTION_SEQ: AtomicU64 = AtomicU64::new(0);

fn next_imagine_seq() -> u64 {
    IMAGINE_DETECTION_SEQ.fetch_add(1, Ordering::Relaxed)
}

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
            if !cached.imagine_names.is_empty() {
                // 直前セッションで学習したイマジン名を復元（召喚検知が来るまでの間の即表示用）。
                // 挿入順に昇順の検知シーケンスを割り当てて ImagineSlot 化する
                // （先頭ほど古い＝last_seen が小さい。復元直後に鮮度の優劣を決めておく）。
                entity.imagines = cached
                    .imagine_names
                    .into_iter()
                    .map(|name| ImagineSlot { name, last_seen: next_imagine_seq() })
                    .collect();
                // 旧バージョンのキャッシュはセッションを跨いで累積し MAX_IMAGINE_NAMES を超え得るため、
                // 復元時にも最新 MAX 件へ丸める（cap_imagine_names はこのキャッシュ復元専用）。
                cap_imagine_names(&mut entity.imagines);
                // pending は復元しない（常に None スタート。保留状態はグループ境界を跨がない）。
            }
        }
    }
    entity
}

/// 召喚エンティティ（非 Player/非 Monster）の attr から、オーナー（`AttrTopSummonerId`、無ければ
/// `AttrSummonerId`）と召喚元スキル（`AttrSkillId`）を読む。スキルがバトルイマジン名に解決できたら
/// オーナー（プレイヤー）の確定イマジン `imagines` / 保留候補 `pending_imagine` を更新する。
///
/// バトルイマジンは装備枠の「奥義」発動スキルだが、戦闘中は召喚エンティティとして現れ、その
/// `AttrSkillId` が召喚元スキル（分身/召喚スキル。NameDesign が親イマジンと同名なので
/// ImagineSkillNames.json で解決できる）を指す。ダメージを出さないイマジン（例: アルーナ＝蘇生）も
/// 召喚は spawn するため、ダメージ列ではなくこの経路が唯一の確実な検知信号になる。
/// 名前解決できないスキル（＝非イマジン召喚）は無視する（誤名回避の安全側デフォルト）。
///
/// # pending（保留）方式
/// 定員 [`MAX_IMAGINE_NAMES`] が埋まっている状態で新規名を検知しても `imagines`（確定・表示用）は
/// 即座に書き換えない。確証（＝ゲーム上あり得ない事象＝既に外れた枠の再 spawn）が得られるまで
/// `pending_imagine`（最大1件）へ留め置く。これにより画面に「新旧混在ペア」が一瞬でも表示される
/// ことを原理的に防ぐ。確定へ昇格するのは次のいずれか:
/// - 既存スロットの再検知（＝そのスロットは現役の確定証拠）と同時に pending があれば、
///   一致しなかった方のスロットを pending の内容へ差し替える（単枠交換の確定）。
/// - pending とは別の新規名がもう1件検知される（＝両枠同時交換の確定）。
/// pending 自身が再検知されても確定へは至らない（現役の証拠にはならない。単に鮮度だけ更新）。
fn try_attribute_summon_imagine(encounter: &mut Encounter, attrs: &[pb::RawAttr]) {
    let mut top_owner: Option<i64> = None;
    let mut direct_owner: Option<i64> = None;
    let mut skill_id: Option<i32> = None;
    for attr in attrs {
        match attr.id {
            attr_type::ATTR_TOP_SUMMONER_ID => {
                if let Ok(v) = decode_protobuf_int64(&attr.raw_data) {
                    if v != 0 {
                        top_owner = Some(v);
                    }
                }
            }
            attr_type::ATTR_SUMMONER_ID => {
                if let Ok(v) = decode_protobuf_int64(&attr.raw_data) {
                    if v != 0 {
                        direct_owner = Some(v);
                    }
                }
            }
            attr_type::ATTR_SKILL_ID => {
                if let Ok(v) = decode_protobuf_int32(&attr.raw_data) {
                    if v != 0 {
                        skill_id = Some(v);
                    }
                }
            }
            _ => {}
        }
    }

    // オーナーと召喚スキルが同一 attr バッチ（＝spawn）で揃ったときのみ確定させる。
    let (Some(owner_uuid), Some(sk)) = (top_owner.or(direct_owner), skill_id) else {
        return;
    };
    if EntityKind::from(owner_uuid) != EntityKind::Player {
        return;
    }
    let Some(name) = crate::engine::imagine_skills::imagine_name(sk) else {
        return;
    };
    let owner_uid = entity::get_player_uid(owner_uuid);
    let owner = get_or_create_entity(encounter, owner_uid, EntityKind::Player);
    let seq = next_imagine_seq();

    // rule1: 既存の確定スロットと一致 → 再検知＝現役の証拠。並び順は変えず鮮度だけ更新する。
    if let Some(slot) = owner.imagines.iter_mut().find(|s| s.name == name) {
        slot.last_seen = seq;
        if let Some(pending) = owner.pending_imagine.take() {
            // 一致したスロット(reactivate した方)が現役と確定したので、一致しなかった方を
            // pending の内容で置き換える（単枠交換の確定）。
            if let Some(other) = owner.imagines.iter_mut().find(|s| s.name != name) {
                info!(
                    "battle imagine confirmed (single-slot swap): uid={owner_uid} {} -> {} (reactivated: {name})",
                    other.name, pending.name
                );
                *other = pending;
            }
            name_cache::update_imagine(owner_uid, &owner.imagine_display_names());
        }
        return;
    }

    // rule2: pending 自身の再検知 → まだ現役の証拠にはならない。鮮度だけ更新し confirmed は触らない。
    if let Some(pending) = owner.pending_imagine.as_mut() {
        if pending.name == name {
            pending.last_seen = seq;
            debug!("battle imagine pending re-detected (not yet confirmed): uid={owner_uid} name={name}");
            return;
        }
    }

    // ここまで来た name は imagines にも pending にも一致しない新規名。
    if owner.imagines.len() < MAX_IMAGINE_NAMES {
        // rule3: 定員未満なので曖昧さが無く、即座に確定へ追加してよい。
        info!("battle imagine detected: uid={owner_uid} name={name} (summon skill {sk})");
        owner.imagines.push(ImagineSlot { name, last_seen: seq });
        name_cache::update_imagine(owner_uid, &owner.imagine_display_names());
        return;
    }

    match owner.pending_imagine.take() {
        None => {
            // rule4: 定員一杯かつ pending 空 → まだ確証が無いので pending へ留め置く。
            // confirmed（imagines）は一切変更しない。未確定情報は name_cache にも書かない。
            info!("battle imagine pending (awaiting confirmation): uid={owner_uid} name={name}");
            owner.pending_imagine = Some(ImagineSlot { name, last_seen: seq });
        }
        Some(old_pending) => {
            // rule5: pending とは別の新規名がもう1件 → 両枠同時交換が確定。
            info!(
                "battle imagine confirmed (dual-slot swap): uid={owner_uid} {} , {name}",
                old_pending.name
            );
            owner.imagines = vec![old_pending, ImagineSlot { name, last_seen: seq }];
            name_cache::update_imagine(owner_uid, &owner.imagine_display_names());
        }
    }
}

/// バトルイマジンの装備枠は2つ（SlotPositionId 7/8）。**この関数はライブ検知経路からは呼ばれない**
/// （ライブ検知の追い出し判断は `try_attribute_summon_imagine` の rule1/5 が pending 方式で明示的に
/// 行うため、鮮度最小を機械的に追い出す処理は不要になった）。`get_or_create_entity` のキャッシュ
/// 復元専用: 旧バージョンでセッションを跨いで累積し `MAX_IMAGINE_NAMES` を超えた古いキャッシュを、
/// 復元時にも最新 MAX 件へ丸める（最も長く再検知されていない＝`last_seen` が最小のものから落とす）。
fn cap_imagine_names(names: &mut Vec<ImagineSlot>) {
    while names.len() > MAX_IMAGINE_NAMES {
        let Some((idx, _)) = names
            .iter()
            .enumerate()
            .min_by_key(|(idx, slot)| (slot.last_seen, *idx))
        else {
            break;
        };
        names.remove(idx);
    }
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

/// 自キャラ戦闘ステータス attr のデコード。ZDPS の isNoValue 準拠で**空 raw_data は値 0**を意味する
/// （クラス変更等でステータスが 0 になると空 raw_data で届くため、空を 0 として反映しないと
/// 古い値が残る）。デコード不能時も 0 を返す。
fn decode_stat_i32(data: &[u8]) -> i32 {
    if data.is_empty() {
        return 0;
    }
    decode_protobuf_int32(data).unwrap_or(0)
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
                Pkt::WorldEnterScene => {
                    let Some(msg) = decode_packet::<pb::EnterScene>(data, "EnterScene") else {
                        return Ok(());
                    };
                    process_enter_scene(&mut encounter, msg);
                }

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
                _ => {
                    // 召喚エンティティの spawn（AttrSkillId=召喚元スキルが載る本命経路）。
                    // 親プレイヤーへイマジン名を帰属させる。
                    try_attribute_summon_imagine(encounter, &attrs.attrs);
                }
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

pub(crate) fn process_scene_delta(encounter: &mut Encounter, scene_delta: pb::SceneDelta) {
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
                _ => {
                    // 召喚エンティティの更新経路（spawn 側で取り逃した場合の保険）。
                    try_attribute_summon_imagine(encounter, &attrs_collection.attrs);
                }
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

    // Time-series sampling（間隔ゲート付き。実体は take_time_series_sample に集約）
    take_time_series_sample(encounter, ts, false);
}

/// 時系列サンプルを1点採取する。通常は間隔ゲート（`TS_INTERVAL_MS`）で間引くが、
/// `force=true` のときはゲートを無視して採取する（3分計測の確定時に終端を計測末尾へ
/// 揃え、結果グラフの折れ線を右端まで届かせるため）。
///
/// `ts` は now_ms() ドメインの時刻。サンプルの `t_ms` は `ts - time_fight_start_ms`。
pub(crate) fn take_time_series_sample(encounter: &mut Encounter, ts: u128, force: bool) {
    let interval_ms = u128::from(
        crate::engine::runtime_settings::TS_INTERVAL_MS.load(std::sync::atomic::Ordering::Relaxed),
    );
    if interval_ms == 0 {
        return;
    }
    let gap = ts.saturating_sub(encounter.last_sample_ms);
    let due = encounter.last_sample_ms == 0 || gap >= interval_ms;
    if !due && !force {
        return;
    }
    // 確定時の終端サンプル: 直近サンプルと同時刻なら既に末尾が採れているので二重採取しない
    // （同一 x への dps=0 点が右端で下向きのヒゲになるのを防ぐ）。
    if force && !due && gap == 0 {
        return;
    }

    // 最初のサンプルは間隔ぶんを窓とみなして DPS を過大計上しない
    let interval_actual = if encounter.last_sample_ms == 0 {
        interval_ms
    } else {
        gap
    };
    let elapsed_since_start = ts.saturating_sub(encounter.time_fight_start_ms);

    // 3分計測中はウィンドウ全体ぶんを保持する。直近 TS_SAMPLES 窓だと計測開始直後の
    // サンプルが pop_front で捨てられ、結果グラフの折れ線が左端(0:00)から始まらないため。
    // 通常時は従来どおり TS_SAMPLES（ライブのローリング窓）を使う。
    let cap = {
        let base = crate::engine::runtime_settings::TS_SAMPLES
            .load(std::sync::atomic::Ordering::Relaxed);
        if let crate::engine::encounter::MeasureMode::Active3Min { duration_ms, .. } =
            encounter.measure_mode
        {
            base.max((duration_ms / interval_ms) as usize + 2)
        } else {
            base
        }
    };

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

/// EnterScene (自キャラ入場) の PlayerEnt.attrs を処理する。
/// AOI 同期(SyncNearEntities)には含まれない詳細ステータス（会心/ファスト/万能/知力/敏捷/
/// 魔攻/魔防 等）がここに入る。PlayerEnt は自キャラなので、未確定なら local_player_uid も確定する。
fn process_enter_scene(encounter: &mut Encounter, msg: pb::EnterScene) {
    let Some(info) = msg.enter_scene_info else {
        return;
    };
    let Some(player_ent) = info.player_ent else {
        return;
    };
    let Some(attrs) = player_ent.attrs else {
        return;
    };
    let player_uid = entity::get_player_uid(player_ent.uuid);
    if player_uid == 0 {
        return;
    }
    // EnterScene は自キャラの入場通知。local_player_uid 未確定ならここで確定させる
    // （初回フル同期をこの経路で確実に取得するため）。
    if encounter.local_player_uid == 0 {
        encounter.local_player_uid = player_uid;
    }
    let target_entity = get_or_create_entity(encounter, player_uid, EntityKind::Player);
    target_entity.entity_type = EntityKind::Player;
    process_player_attrs(player_uid, target_entity, &attrs.attrs);
}

fn process_player_attrs(uid: i64, player_entity: &mut Entity, attrs: &[pb::RawAttr]) {
    use crate::capture::binary_reader::BinaryReader;

    let mut cache_name: Option<String> = None;
    let mut cache_class: Option<i32> = None;
    let mut cache_score: Option<i32> = None;
    let mut cache_season_lv: Option<i32> = None;
    let mut cache_season_str: Option<i32> = None;

    for attr in attrs {
        // 空 raw_data はスキップしない: ステータスが 0 になった通知（ZDPS の isNoValue=空）を
        // 取りこぼすと古い値が残るため、ステータス系アームで空を 0 として反映する。
        if attr.id == 0 {
            continue;
        }

        match attr.id {
            attr_type::ATTR_NAME => {
                // 空（名前なし）は先頭バイトのスライスで panic するため早期スキップ。
                if !attr.raw_data.is_empty() {
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
            // 戦闘ステータスは空 raw_data を 0 として反映（クラス変更で 0 化した値の取りこぼし防止）。
            attr_type::ATTR_ATTACK_POWER => {
                player_entity.attack_power = Some(decode_stat_i32(&attr.raw_data));
            }
            attr_type::ATTR_DEFENSE_POWER => {
                player_entity.defense_power = Some(decode_stat_i32(&attr.raw_data));
            }
            attr_type::ATTR_ENDURANCE => {
                player_entity.endurance = Some(decode_stat_i32(&attr.raw_data));
            }
            attr_type::ATTR_STRENGTH => {
                player_entity.strength = Some(decode_stat_i32(&attr.raw_data));
            }
            attr_type::ATTR_INTELLIGENCE => {
                player_entity.intelligence = Some(decode_stat_i32(&attr.raw_data));
            }
            attr_type::ATTR_AGILITY => {
                player_entity.agility = Some(decode_stat_i32(&attr.raw_data));
            }
            attr_type::ATTR_MAGIC_ATTACK => {
                player_entity.magic_attack = Some(decode_stat_i32(&attr.raw_data));
            }
            attr_type::ATTR_MAGIC_DEFENSE => {
                player_entity.magic_defense = Some(decode_stat_i32(&attr.raw_data));
            }
            attr_type::ATTR_CRIT => {
                player_entity.crit_stat = Some(decode_stat_i32(&attr.raw_data));
            }
            attr_type::ATTR_CRIT_DMG => {
                player_entity.crit_dmg = Some(decode_stat_i32(&attr.raw_data));
            }
            attr_type::ATTR_RESIST => {
                player_entity.resist = Some(decode_stat_i32(&attr.raw_data));
            }
            attr_type::ATTR_CAST_SPEED => {
                player_entity.cast_speed = Some(decode_stat_i32(&attr.raw_data));
            }
            attr_type::ATTR_VERSATILITY => {
                player_entity.versatility = Some(decode_stat_i32(&attr.raw_data));
            }
            attr_type::ATTR_DEXTERITY => {
                player_entity.dexterity = Some(decode_stat_i32(&attr.raw_data));
            }
            attr_type::ATTR_ATTACK_SPEED => {
                player_entity.attack_speed = Some(decode_stat_i32(&attr.raw_data));
            }
            attr_type::ATTR_HASTE => {
                player_entity.haste = Some(decode_stat_i32(&attr.raw_data));
            }
            attr_type::ATTR_LUCKY => {
                player_entity.lucky = Some(decode_stat_i32(&attr.raw_data));
            }
            attr_type::ATTR_LUCKY_DMG => {
                player_entity.lucky_dmg = Some(decode_stat_i32(&attr.raw_data));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::encounter::MeasureMode;
    use std::sync::atomic::Ordering;

    fn set_ts_config(samples: usize, interval_ms: u64) {
        crate::engine::runtime_settings::TS_SAMPLES.store(samples, Ordering::Relaxed);
        crate::engine::runtime_settings::TS_INTERVAL_MS.store(interval_ms, Ordering::Relaxed);
    }

    fn player() -> Entity {
        Entity { entity_type: EntityKind::Player, ..Default::default() }
    }

    // 3分計測中は TS_SAMPLES(=60) を超えても全ウィンドウ分のサンプルを保持し、
    // 折れ線が左端(t=0)から始まる。継続戦闘なら通常サンプルだけで右端(=window)に届く。
    #[test]
    fn three_min_series_spans_full_window() {
        set_ts_config(60, 1000); // 既定相当: 直近60サンプルだけだと先頭が切り捨てられる設定

        let window_ms: u128 = 90_000; // 90s 窓（91サンプル > 上限60）
        let interval: u128 = 1000;

        let mut enc = Encounter {
            measure_mode: MeasureMode::Active3Min { armed_at_ms: 0, duration_ms: window_ms },
            ..Default::default()
        };
        enc.entities.insert(1, player());

        let mut ts: u128 = 0;
        while ts <= window_ms {
            enc.dmg_stats.total += 1000;
            enc.entities.get_mut(&1).unwrap().dmg_stats.total += 1000;
            enc.time_last_combat_packet_ms = ts;
            take_time_series_sample(&mut enc, ts, false);
            ts += interval;
        }

        // 上限60を超えて全サンプル保持（左端=0 / 右端=window）。
        assert!(
            enc.time_series.len() > 60,
            "series truncated to cap: {}",
            enc.time_series.len()
        );
        assert_eq!(enc.time_series.front().unwrap().t_ms, 0.0, "left edge not at 0");
        assert_eq!(
            enc.time_series.back().unwrap().t_ms,
            window_ms as f64,
            "right edge not at window"
        );

        let p = &enc.entities[&1];
        assert_eq!(p.time_series.front().unwrap().t_ms, 0.0);
        assert_eq!(p.time_series.back().unwrap().t_ms, window_ms as f64);
    }

    // 最後の戦闘パケットが間隔ゲート未満で通常サンプルされない場合でも、
    // 確定時の force サンプルで右端=last_combat に届く。
    #[test]
    fn finalize_force_sample_closes_right_edge() {
        set_ts_config(200, 1000);

        let mut enc = Encounter {
            measure_mode: MeasureMode::Active3Min { armed_at_ms: 0, duration_ms: 90_000 },
            ..Default::default()
        };
        enc.entities.insert(1, player());

        let mut ts: u128 = 0;
        while ts <= 5000 {
            enc.dmg_stats.total += 1000;
            enc.entities.get_mut(&1).unwrap().dmg_stats.total += 1000;
            enc.time_last_combat_packet_ms = ts;
            take_time_series_sample(&mut enc, ts, false);
            ts += 1000;
        }
        // 最後の戦闘パケットは 5300ms（間隔未満なので通常サンプルでは採れない）
        enc.time_last_combat_packet_ms = 5300;
        assert_eq!(enc.time_series.back().unwrap().t_ms, 5000.0);

        let end = enc.time_last_combat_packet_ms;
        take_time_series_sample(&mut enc, end, true);
        assert_eq!(
            enc.time_series.back().unwrap().t_ms,
            5300.0,
            "force sample didn't extend to last_combat"
        );
    }

    fn player_uuid_for(uid: i64) -> i64 {
        (uid << 16) | 640
    }

    /// 値を bare varint(LEB128) で符号化する（attr raw_data の形式）。
    fn enc_varint(mut v: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(byte);
                break;
            }
            out.push(byte | 0x80);
        }
        out
    }

    /// 召喚エンティティの spawn を模した合成 SceneDelta。オーナー(AttrTopSummonerId)と
    /// 召喚元スキル(AttrSkillId)を載せる。uuid は Player/Monster 以外の型コードで Unknown 判定。
    fn summon_spawn_delta(owner_uid: i64, skill_id: i32) -> pb::SceneDelta {
        let summon_uuid = (skill_id as i64) << 16 | 0x0100; // &0xFFFF=0x100 → Unknown
        pb::SceneDelta {
            uuid: summon_uuid,
            attrs: Some(pb::EntityAttrs {
                uuid: summon_uuid,
                attrs: vec![
                    pb::RawAttr {
                        id: attr_type::ATTR_TOP_SUMMONER_ID,
                        raw_data: enc_varint(player_uuid_for(owner_uid) as u64),
                    },
                    pb::RawAttr {
                        id: attr_type::ATTR_SKILL_ID,
                        raw_data: enc_varint(skill_id as u64),
                    },
                ],
            }),
            buff_list: None,
            skill_effects: None,
        }
    }

    // 召喚の AttrSkillId(分身/召喚スキル)がオーナー(プレイヤー)へイマジン名として帰属する。
    #[test]
    fn summon_attributes_imagine_to_owner() {
        let mut enc = Encounter::default();
        enc.entities.insert(1, player());

        // 1007740 = 奥义!毒爆 → ヴェノミーンの巣（分身/召喚スキル）
        process_scene_delta(&mut enc, summon_spawn_delta(1, 1_007_740));
        assert_eq!(
            enc.entities[&1].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string()]
        );
    }

    // 無ダメージのイマジン(アルーナ=蘇生)も召喚 spawn 経路で検知できる（本機能の主目的）。
    #[test]
    fn no_damage_imagine_detected_via_summon() {
        let mut enc = Encounter::default();
        enc.entities.insert(1, player());

        // 2900240 = 奥义！生命祈愿 → アルーナ（蘇生・ダメージを出さない）
        process_scene_delta(&mut enc, summon_spawn_delta(1, 2_900_240));
        assert_eq!(enc.entities[&1].imagine_display_names(), vec!["アルーナ".to_string()]);
    }

    // 複数の召喚は発見順に累積し、同一名は重複させない。
    #[test]
    fn summon_accumulates_and_dedups() {
        let mut enc = Encounter::default();
        enc.entities.insert(1, player());

        process_scene_delta(&mut enc, summon_spawn_delta(1, 1_007_740)); // ヴェノミーンの巣
        process_scene_delta(&mut enc, summon_spawn_delta(1, 2_900_240)); // アルーナ
        assert_eq!(
            enc.entities[&1].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );

        // 同じイマジン(別ID 1007741=虚拟体 も同名解決)を再度 → 重複しない
        process_scene_delta(&mut enc, summon_spawn_delta(1, 1_007_741));
        assert_eq!(
            enc.entities[&1].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );
    }

    // イマジン表に無い召喚スキル(=職業召喚など)はイマジンとして扱わない（誤名回避）。
    #[test]
    fn non_imagine_summon_ignored() {
        let mut enc = Encounter::default();
        enc.entities.insert(1, player());

        // 55404 は ImagineSkillNames.json に無い（実機で観測した非イマジン召喚）
        process_scene_delta(&mut enc, summon_spawn_delta(1, 55_404));
        assert!(enc.entities[&1].imagine_display_names().is_empty());
    }

    // オーナー(AttrTopSummonerId)が欠けた召喚 attr は帰属できず無視される。
    #[test]
    fn summon_without_owner_ignored() {
        let mut enc = Encounter::default();
        enc.entities.insert(1, player());

        let delta = pb::SceneDelta {
            uuid: (1_007_740i64 << 16) | 0x0100,
            attrs: Some(pb::EntityAttrs {
                uuid: (1_007_740i64 << 16) | 0x0100,
                attrs: vec![pb::RawAttr {
                    id: attr_type::ATTR_SKILL_ID,
                    raw_data: enc_varint(1_007_740),
                }],
            }),
            buff_list: None,
            skill_effects: None,
        };
        process_scene_delta(&mut enc, delta);
        assert!(enc.entities[&1].imagine_display_names().is_empty());
    }

    // ロローラは実ゲーム版の召喚ID(2900840=奥義！神霊依凭)で解決できる（版ズレで名前グルーピング不能な分の手動追記）。
    #[test]
    fn rorora_detected_via_game_summon_id() {
        let mut enc = Encounter::default();
        enc.entities.insert(1, player());

        process_scene_delta(&mut enc, summon_spawn_delta(1, 2_900_840));
        assert_eq!(enc.entities[&1].imagine_display_names(), vec!["ロローラ".to_string()]);
    }

    // 装備枠は2つ。pending 方式では 3体目(新規)を検知しても confirmed を即座には書き換えない
    // （rule4: 定員一杯・pending 空 → まだ確証が無いのでいったん pending へ留め置くだけ）。
    // 「新規名を検知した瞬間に古い方を追い出して新旧混在ペアを作ってしまう」という前回ロジックの
    // 問題そのものを避けるのが pending 方式の意図であり、この挙動変化はその直接の反映。
    #[test]
    fn imagine_names_capped_to_two_keeping_latest() {
        let mut enc = Encounter::default();
        enc.entities.insert(1, player());

        process_scene_delta(&mut enc, summon_spawn_delta(1, 1_007_740)); // A: ヴェノミーンの巣
        process_scene_delta(&mut enc, summon_spawn_delta(1, 2_900_240)); // B: アルーナ
        process_scene_delta(&mut enc, summon_spawn_delta(1, 2_900_840)); // C: ロローラ（3体目・新規）

        // confirmed は [A,B] のまま（[B,C] へは即座に丸められない）。
        assert_eq!(
            enc.entities[&1].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );
        // C はまだ確証が無いので pending へ留め置かれるだけ。
        assert_eq!(
            enc.entities[&1].pending_imagine.as_ref().map(|s| s.name.as_str()),
            Some("ロローラ")
        );
    }

    // ① 単枠交換の pending→confirmed 昇格（本バグ修正の核心）: A,B を検知後、新規 C は
    // 定員一杯のため即座に confirmed へは反映されず pending に留まる（confirmed は [A,B] のまま
    // 変化しない＝新旧混在ペアが一切表示されない）。その後 A（現役）を再検知すると、それが
    // 「B は既に外された」ことの確証になり、pending の C が確定へ昇格して B を置き換える。
    #[test]
    fn single_slot_swap_pending_then_confirmed_on_recheck() {
        let mut enc = Encounter::default();
        enc.entities.insert(2, player());

        process_scene_delta(&mut enc, summon_spawn_delta(2, 1_007_740)); // A: ヴェノミーンの巣
        process_scene_delta(&mut enc, summon_spawn_delta(2, 2_900_240)); // B: アルーナ
        assert_eq!(
            enc.entities[&2].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );

        // C（新規）を検知 → 定員一杯・pending 空 → rule4: pending へ留め置くだけ
        process_scene_delta(&mut enc, summon_spawn_delta(2, 2_900_840)); // C: ロローラ
        assert_eq!(
            enc.entities[&2].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()],
            "pending 設定だけでは confirmed が変化してはいけない"
        );
        assert_eq!(
            enc.entities[&2].pending_imagine.as_ref().map(|s| s.name.as_str()),
            Some("ロローラ")
        );

        // A（現役）を再検知 → rule1: pending(C) が確定へ昇格し、放置された B を置き換える
        process_scene_delta(&mut enc, summon_spawn_delta(2, 1_007_740));
        assert_eq!(
            enc.entities[&2].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "ロローラ".to_string()]
        );
        assert!(enc.entities[&2].pending_imagine.is_none());
    }

    // ② 両枠同時交換で「新旧混在ペア」が一度も画面に出ないことの直接的な証明（pending 方式の本質）。
    // A,B(confirmed) → C(新規・定員一杯・pending 空→pending 化。confirmed は完全に不変)
    // → D(pending とは別の新規・rule5)で両枠同時交換が確定し [C,D] へ一気に切り替わる。
    // このテストを通じて観測可能な confirmed は常に [A,B] か [C,D] のいずれかのみであり、
    // [A,C]/[B,D]/[B,C] のような混在ペアが一瞬たりとも表示されないことがポイント。
    #[test]
    fn dual_slot_swap_confirmed_only_after_second_new_name() {
        let mut enc = Encounter::default();
        enc.entities.insert(3, player());

        process_scene_delta(&mut enc, summon_spawn_delta(3, 1_007_740)); // A: ヴェノミーンの巣
        process_scene_delta(&mut enc, summon_spawn_delta(3, 2_900_240)); // B: アルーナ
        assert_eq!(
            enc.entities[&3].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );

        // C（新規）を検知 → rule4: pending へ留め置くだけ。confirmed は旧ペア [A,B] のまま不変。
        process_scene_delta(&mut enc, summon_spawn_delta(3, 2_900_840)); // C: ロローラ
        assert_eq!(
            enc.entities[&3].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()],
            "混在ペア([A,C]等)を一瞬でも見せてはいけない"
        );

        // D（pending とは別の新規）を検知 → rule5: 両枠同時交換の確定。[C,D] へ一気に切り替わる。
        process_scene_delta(&mut enc, summon_spawn_delta(3, 1_002_830)); // D: フロストオーガ
        assert_eq!(
            enc.entities[&3].imagine_display_names(),
            vec!["ロローラ".to_string(), "フロストオーガ".to_string()]
        );
        assert!(enc.entities[&3].pending_imagine.is_none());
    }

    // ③ cap 不変条件: A,B,C,D,E を検知しても confirmed は常に len()<=2 に収まる
    // （pending の有無に関わらず imagines への push/置換は常に定員内で完結するため）。
    #[test]
    fn imagine_count_never_exceeds_cap() {
        let mut enc = Encounter::default();
        enc.entities.insert(4, player());

        let skills = [1_007_740, 2_900_240, 2_900_840, 1_002_830, 1_007_741_i32];
        for &sk in &skills {
            // 1007741 は 1007740 と同名（ヴェノミーンの巣）解決だが cap 確認の分母には影響しない。
            process_scene_delta(&mut enc, summon_spawn_delta(4, sk));
            assert!(
                enc.entities[&4].imagine_display_names().len() <= MAX_IMAGINE_NAMES,
                "imagine count exceeded cap after skill {sk}"
            );
        }
    }

    // ④ 表示順の安定性: A,B 検知後に一方・両方を何度再検知しても並び順は反転しない([B,A] にならない)。
    // 名前は常に既存2枠のいずれかと一致する（第3の新規名は登場しない）ので pending は一切絡まない。
    #[test]
    fn display_order_stable_across_rechecks() {
        let mut enc = Encounter::default();
        enc.entities.insert(5, player());

        process_scene_delta(&mut enc, summon_spawn_delta(5, 1_007_740)); // A
        process_scene_delta(&mut enc, summon_spawn_delta(5, 2_900_240)); // B
        assert_eq!(
            enc.entities[&5].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );

        // B を複数回、A も再検知 → 並び替えは起きない
        process_scene_delta(&mut enc, summon_spawn_delta(5, 2_900_240));
        process_scene_delta(&mut enc, summon_spawn_delta(5, 2_900_240));
        process_scene_delta(&mut enc, summon_spawn_delta(5, 1_007_740));
        process_scene_delta(&mut enc, summon_spawn_delta(5, 2_900_240));

        assert_eq!(
            enc.entities[&5].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()],
            "display order must not reverse to [B, A]"
        );
        assert!(enc.entities[&5].pending_imagine.is_none());
    }

    // ⑤ pending 自身の再検知だけでは確定に至らない回帰防止テスト。A,B(confirmed)→C(新規・pending
    // 化)の後、C を再検知しても rule2 が発火するだけで confirmed は不変・pending も C のまま
    // （＝ pending の再検知は「まだ現役の証拠」にはならず、昇格には既存スロットの再検知＝rule1、
    // または別の新規名＝rule5 のいずれかが必要）。
    #[test]
    fn pending_redetection_does_not_promote_alone() {
        let mut enc = Encounter::default();
        enc.entities.insert(6, player());

        process_scene_delta(&mut enc, summon_spawn_delta(6, 1_007_740)); // A
        process_scene_delta(&mut enc, summon_spawn_delta(6, 2_900_240)); // B
        process_scene_delta(&mut enc, summon_spawn_delta(6, 2_900_840)); // C（新規）→ pending

        process_scene_delta(&mut enc, summon_spawn_delta(6, 2_900_840)); // C を再検知（rule2）

        assert_eq!(
            enc.entities[&6].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()],
            "pending 自身の再検知だけでは confirmed を書き換えてはいけない"
        );
        assert_eq!(
            enc.entities[&6].pending_imagine.as_ref().map(|s| s.name.as_str()),
            Some("ロローラ"),
            "pending の再検知は pending のまま(昇格しない)"
        );
    }

    // ⑥ pending 設定だけでは name_cache へ永続化されない回帰防止テスト（未確定情報をディスクへ
    // 書かない、という設計の直接検証）。専用 uid を使い他テストの name_cache と衝突しないこと。
    #[test]
    fn pending_never_persisted_to_name_cache() {
        let mut enc = Encounter::default();
        let uid = 990_002; // name_cache はプロセス共有のため専用 uid を使う

        enc.entities.insert(uid, player());
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 1_007_740)); // A
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 2_900_240)); // B

        let cached = name_cache::lookup(uid).expect("cache entry should exist after B confirmed");
        assert_eq!(
            cached.imagine_names,
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );

        // C（新規）を検知 → rule4: pending へ留め置くだけ（confirmed は不変・name_cache も未変更）
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 2_900_840));

        let cached_after_pending =
            name_cache::lookup(uid).expect("cache entry should still exist");
        assert_eq!(
            cached_after_pending.imagine_names,
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()],
            "pending 設定だけで name_cache へ永続化してはいけない"
        );
    }

    // ⑦ clear_combat_stats 跨ぎのシナリオ: Player entity 破棄 → name_cache 復元後も pending 方式が
    // 破綻しないことを確認する。復元直後に A を再検知しても pending が無いので confirmed は不変。
    // 続けて C（新規）を検知すると定員一杯のため pending へ留め置かれるだけで confirmed は
    // [A,B] のまま（pending 方式のおかげでこの中間状態も断定できる＝旧 LRU 方式からの改善点）。
    // もう一度 A を再検知すると rule1 が発火し、pending(C) が確定へ昇格して B を置き換える。
    #[test]
    fn imagine_survives_across_clear_combat_stats_with_recheck() {
        let mut enc = Encounter::default();
        let uid = 990_001; // name_cache はプロセス共有のため他テストと衝突しない専用 uid を使う

        enc.entities.insert(uid, player());
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 1_007_740)); // A: ヴェノミーンの巣
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 2_900_240)); // B: アルーナ
        assert_eq!(
            enc.entities[&uid].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );

        enc.clear_combat_stats(); // Player entity は破棄される（name_cache には残る）
        assert!(!enc.entities.contains_key(&uid));

        // 次パケットで A を再検知 → name_cache から [A,B] を復元した上で A の鮮度を更新するのみ
        // （pending が無いので confirmed は不変）。
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 1_007_740));
        assert_eq!(
            enc.entities[&uid].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );

        // 続けて C（ロローラ）を新規検知 → 定員一杯・pending 空 → rule4: pending へ留め置くだけ
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 2_900_840));
        assert_eq!(
            enc.entities[&uid].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()],
            "pending 設定だけでは confirmed を書き換えない"
        );

        // もう一度 A を再検知 → rule1: pending(C) が確定へ昇格し、放置された B を置き換える
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 1_007_740));
        assert_eq!(
            enc.entities[&uid].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "ロローラ".to_string()]
        );
        assert!(enc.entities[&uid].pending_imagine.is_none());
    }
}
