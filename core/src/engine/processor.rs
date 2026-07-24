use crate::capture::server::Server;
use crate::engine::class::{Class, ClassSpec, get_class_from_spec, get_class_spec_from_skill_id};
use crate::engine::combat_stats::process_stats;
use crate::engine::encounter::{Encounter, EncounterMutex};
use crate::engine::entity::{
    Entity, ImagineSlot, MAX_IMAGINE_NAMES, MAX_ROLE_SKILL_IMAGINES, SkillMeta,
};
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
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// バトルイマジン検知の鮮度比較用シーケンス番号。wall-clock ではなく単調増加カウンタにする
/// のは、同一ミリ秒に複数検知が起きるとテスト・実戦とも鮮度が潰れて順序判定できなくなるため。
static IMAGINE_DETECTION_SEQ: AtomicU64 = AtomicU64::new(0);

fn next_imagine_seq() -> u64 {
    IMAGINE_DETECTION_SEQ.fetch_add(1, Ordering::Relaxed)
}

/// `pending_imagine` が単独昇格（自己修復）するまでに要求する再検知回数
/// （rule4 の初回検知=1 を含む）。休眠イマジン（召喚報告ID未登録で相方が rule5 を
/// 満たせない）が絡む装備替えでも、有限回の再検知で確定表示が自己修復するようにする。
/// 閾値を小さくしすぎると単発の誤読で誤昇格しやすくなり、大きくしすぎると自己修復が遅れる。
const PENDING_PROMOTE_HITS: u32 = 3;

/// `ImagineSkillNames.json` 未登録の召喚元スキルIDを、プロセス生存中1回だけログへ記録する
/// ための既知集合（`BPSR_PROBE=1` の開発時調査専用。配布ビルドではログ出力せずディスクを
/// 圧迫しない）。新規イマジンの召喚報告ID発見のため、人が多い場所での長時間観測でログに
/// 未知IDを残す目的。既知イマジンの検知ロジック自体には影響しない。
static UNRESOLVED_SUMMON_SKILLS: LazyLock<Mutex<HashSet<i32>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// 装備スキルリスト(ATTR_SKILL_LEVEL_ID_LIST=116)の調査用ログ（`BPSR_PROBE=1` の開発時のみ）。
/// uid ごとに最後にログした skill_id 列を覚え、内容が変わった時だけ info! する
/// （人が多い場所でも appear の度に同内容を繰り返さないための抑制）。表示への反映自体は
/// `apply_skill_list_imagines` が probe と無関係に常時行う。このログは調査目的のみ。
static LAST_SKILL_LIST: LazyLock<Mutex<HashMap<i64, Vec<i32>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 装備データ(ATTR_EQUIP_DATA=200)の調査用ログ（`BPSR_PROBE=1` の開発時のみ）。
/// 抑制は LAST_SKILL_LIST と同様。イマジンのアイテム構成IDがプレイヤー attr として
/// 観測できるかを確認する目的。
static LAST_EQUIP_LIST: LazyLock<Mutex<HashMap<i64, Vec<(i32, i32)>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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
                // 凸数は並列配列 imagine_tiers から復元（旧キャッシュで不足する分は 0=未判明）。
                let tiers = cached.imagine_tiers;
                entity.imagines = cached
                    .imagine_names
                    .into_iter()
                    .enumerate()
                    .map(|(i, name)| ImagineSlot {
                        name,
                        last_seen: next_imagine_seq(),
                        tier: tiers.get(i).copied().unwrap_or(0),
                        pending_hits: 0,
                    })
                    .collect();
                // 旧バージョンのキャッシュはセッションを跨いで累積し MAX_IMAGINE_NAMES を超え得るため、
                // 復元時にも最新 MAX 件へ丸める（cap_imagine_names はこのキャッシュ復元専用）。
                cap_imagine_names(&mut entity.imagines, MAX_IMAGINE_NAMES);
                // pending は復元しない（常に None スタート。保留状態はグループ境界を跨がない）。
            }
            if !cached.role_skill_imagine_names.is_empty() {
                // ロールスキル(簡易版バトルイマジン、最大4枠)も同様に直前セッションの検知結果を
                // 即表示用に復元（imagines の復元処理と同じパターン）。
                let tiers = cached.role_skill_imagine_tiers;
                entity.role_skill_imagines = cached
                    .role_skill_imagine_names
                    .into_iter()
                    .enumerate()
                    .map(|(i, name)| ImagineSlot {
                        name,
                        last_seen: next_imagine_seq(),
                        tier: tiers.get(i).copied().unwrap_or(0),
                        pending_hits: 0,
                    })
                    .collect();
                cap_imagine_names(&mut entity.role_skill_imagines, MAX_ROLE_SKILL_IMAGINES);
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
/// - pending 自身が [`PENDING_PROMOTE_HITS`] 回再検知される（＝相方が休眠イマジン等で
///   rule5 の条件を満たせない場合の自己修復。旧確定ペアを両方破棄し、確証のある pending 名
///   だけを単独確定にする。2枠目は次に新規検知が来るまで「未知（空）」表示のまま）。
/// 通常の再検知だけでは確定へは至らない（現役の証拠にはならない。単に鮮度だけ更新）。
fn try_attribute_summon_imagine(encounter: &mut Encounter, attrs: &[pb::RawAttr]) {
    let mut top_owner: Option<i64> = None;
    let mut direct_owner: Option<i64> = None;
    let mut skill_id: Option<i32> = None;
    let mut tier: i32 = 0; // イマジンレベル（凸数）。0=未判明のまま扱う
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
            attr_type::ATTR_SKILL_REMODEL_LEVEL => {
                if let Ok(v) = decode_protobuf_int32(&attr.raw_data) {
                    if v > 0 {
                        tier = v;
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
        if crate::probe::enabled() {
            if let Ok(mut seen) = UNRESOLVED_SUMMON_SKILLS.lock() {
                if seen.insert(sk) {
                    info!("summon attr: unresolved skill (not a known imagine): owner={owner_uuid} skill_id={sk}");
                }
            }
        }
        return;
    };
    let owner_uid = entity::get_player_uid(owner_uuid);
    let owner = get_or_create_entity(encounter, owner_uid, EntityKind::Player);
    let seq = next_imagine_seq();

    // rule1: 既存の確定スロットと一致 → 再検知＝現役の証拠。並び順は変えず鮮度だけ更新する
    // （凸数はキャッシュ復元直後 0 の場合やレベル上げ後があるため、非0 が来たら追従する）。
    if let Some(slot) = owner.imagines.iter_mut().find(|s| s.name == name) {
        slot.last_seen = seq;
        let tier_changed = tier > 0 && slot.tier != tier;
        if tier_changed {
            slot.tier = tier;
        }
        if let Some(mut pending) = owner.pending_imagine.take() {
            // 一致したスロット(reactivate した方)が現役と確定したので、一致しなかった方を
            // pending の内容で置き換える（単枠交換の確定）。確定スロットの pending_hits は常に0。
            pending.pending_hits = 0;
            if let Some(other) = owner.imagines.iter_mut().find(|s| s.name != name) {
                info!(
                    "battle imagine confirmed (single-slot swap): uid={owner_uid} {} -> {} (reactivated: {name})",
                    other.name, pending.name
                );
                *other = pending;
            }
            name_cache::update_imagine(
                owner_uid,
                &owner.imagine_display_names(),
                &owner.imagine_tiers(),
            );
        } else if tier_changed {
            // 名前構成は不変でも凸数が確定/変化したら永続化する（表示の (N) を次回起動へ引き継ぐ）。
            name_cache::update_imagine(
                owner_uid,
                &owner.imagine_display_names(),
                &owner.imagine_tiers(),
            );
        }
        return;
    }

    // ロールスキル(簡易版バトルイマジン、最大4枠)のエコー吸収。ロールスキルの簡易発動は実イマジンの
    // 召喚シグナル(AttrSkillId)と protocol レベルで同一の形になり得るため、
    // apply_skill_list_imagines（attr116 の権威的スナップショット）が既にロールスキル枠として
    // この名前を確定させている場合、以後この召喚検知経路からは imagines/pending_imagine に
    // 一切触れさせず、ここで吸収する。これをしないと、短いクールタイムで連発されるロールスキルの
    // 発動ノイズが定員一杯の pending/確定スワップ判定（rule1〜5）へ繰り返し流れ込み、実イマジン
    // 2枠との間で確定表示がフラッピングする（本バグの直接原因）。
    if owner.role_skill_imagines.iter().any(|s| s.name == name) {
        if let Some(slot) = owner.role_skill_imagines.iter_mut().find(|s| s.name == name) {
            slot.last_seen = seq;
            let tier_changed = tier > 0 && slot.tier != tier;
            if tier_changed {
                slot.tier = tier;
                name_cache::update_role_skill_imagines(
                    owner_uid,
                    &owner.role_skill_imagine_names(),
                    &owner.role_skill_imagine_tiers(),
                );
            }
        }
        return;
    }

    // rule2: pending 自身の再検知 → 通常は鮮度だけ更新し confirmed は触らない。ただし
    // PENDING_PROMOTE_HITS 回に達したら自己修復（旧確定ペアを両方破棄し、確証のある pending
    // 名だけを単独確定にする）。相方が休眠イマジンで rule5 の条件を満たせない場合の救済。
    let mut promoted: Option<ImagineSlot> = None;
    if let Some(pending) = owner.pending_imagine.as_mut() {
        if pending.name == name {
            pending.last_seen = seq;
            if tier > 0 {
                pending.tier = tier;
            }
            pending.pending_hits += 1;
            if pending.pending_hits >= PENDING_PROMOTE_HITS {
                promoted = owner.pending_imagine.take();
            } else {
                debug!(
                    "battle imagine pending re-detected (not yet confirmed): uid={owner_uid} name={name} hits={}",
                    pending.pending_hits
                );
                return;
            }
        }
    }
    if let Some(mut promoted) = promoted {
        promoted.pending_hits = 0;
        info!(
            "battle imagine self-heal: uid={owner_uid} promoted {} to sole confirmed (stale pair evicted after {PENDING_PROMOTE_HITS} re-detections; dormant partner suspected)",
            promoted.name
        );
        owner.imagines = vec![promoted];
        name_cache::update_imagine(
            owner_uid,
            &owner.imagine_display_names(),
            &owner.imagine_tiers(),
        );
        return;
    }

    // ここまで来た name は imagines にも pending にも一致しない新規名。
    if owner.imagines.len() < MAX_IMAGINE_NAMES {
        // rule3: 定員未満なので曖昧さが無く、即座に確定へ追加してよい。
        info!("battle imagine detected: uid={owner_uid} name={name} (summon skill {sk})");
        owner.imagines.push(ImagineSlot { name, last_seen: seq, tier, pending_hits: 0 });
        name_cache::update_imagine(
            owner_uid,
            &owner.imagine_display_names(),
            &owner.imagine_tiers(),
        );
        return;
    }

    match owner.pending_imagine.take() {
        None => {
            // rule4: 定員一杯かつ pending 空 → まだ確証が無いので pending へ留め置く。
            // confirmed（imagines）は一切変更しない。未確定情報は name_cache にも書かない。
            info!("battle imagine pending (awaiting confirmation): uid={owner_uid} name={name}");
            owner.pending_imagine =
                Some(ImagineSlot { name, last_seen: seq, tier, pending_hits: 1 });
        }
        Some(mut old_pending) => {
            // rule5: pending とは別の新規名がもう1件 → 両枠同時交換が確定。
            info!(
                "battle imagine confirmed (dual-slot swap): uid={owner_uid} {} , {name}",
                old_pending.name
            );
            old_pending.pending_hits = 0;
            owner.imagines =
                vec![old_pending, ImagineSlot { name, last_seen: seq, tier, pending_hits: 0 }];
            name_cache::update_imagine(
                owner_uid,
                &owner.imagine_display_names(),
                &owner.imagine_tiers(),
            );
        }
    }
}

/// バトルイマジン(定員 `MAX_IMAGINE_NAMES`=2)・ロールスキル(定員 `MAX_ROLE_SKILL_IMAGINES`=4)の
/// いずれも `get_or_create_entity` のキャッシュ復元専用に使う共通処理。**ライブ検知経路からは
/// 呼ばれない**（ライブ検知の追い出し判断は `try_attribute_summon_imagine` の rule1/5 が pending
/// 方式で明示的に行うため、鮮度最小を機械的に追い出す処理は不要になった）。旧バージョンで
/// セッションを跨いで累積し `cap` を超えた古いキャッシュを、復元時にも最新 `cap` 件へ丸める
/// （最も長く再検知されていない＝`last_seen` が最小のものから落とす）。
fn cap_imagine_names(names: &mut Vec<ImagineSlot>, cap: usize) {
    while names.len() > cap {
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

/// 自キャラ戦闘ステータス attr のデコード。**空 raw_data は値 0**（no-value 通知）を意味する
/// （クラス変更等でステータスが 0 になると空 raw_data で届くため、空を 0 として反映しないと
/// 古い値が残る）。デコード不能時も 0 を返す。
fn decode_stat_i32(data: &[u8]) -> i32 {
    if data.is_empty() {
        return 0;
    }
    decode_protobuf_int32(data).unwrap_or(0)
}

/// raw_data を SkillLevelList（repeated SkillLevelInfo=1 のタグ付き形式）として decode する
/// （ATTR_SKILL_LEVEL_ID_LIST=116 の中身。2026-07-10 ダンジョン実測でタグ付き形式と確定）。
fn decode_skill_level_info_list(data: &[u8]) -> Vec<pb::SkillLevelInfo> {
    pb::SkillLevelList::decode(data).map(|list| list.skills).unwrap_or_default()
}

/// raw_data を EquipNineList（repeated EquipNine=1 のタグ付き形式）として decode する
/// （ATTR_EQUIP_DATA=200 の中身）。
fn decode_equip_nine_list(data: &[u8]) -> Vec<pb::EquipNine> {
    pb::EquipNineList::decode(data).map(|list| list.equips).unwrap_or_default()
}

/// 装備スキルリストがフルスナップショット（全習得スキル+装備中イマジン）とみなせる最小件数。
/// フルリストは実測40件超（クラススキルブック一式）で、差分更新は数件のため、この閾値で
/// 「部分リストを装備イマジンの全量と誤認して確定表示を壊す」事故を防ぐ。
const MIN_FULL_SKILL_LIST_LEN: usize = 10;

/// フルの装備スキルリスト（attr 116）から装備中バトルイマジンを権威的に確定する。
/// ダンジョン実測(2026-07-10)で、フルリストには**装備中イマジンの canonical スキルID
/// （39xx）がちょうど装備分（≤2件）だけ**凸数付きで載ることを確認済み（休眠イマジン含む。
/// 自分=EnterScene(マップ移動ごと)、他人=AOI appear(ダンジョン読込/ボス切替)で届く）。
/// 部分リスト（差分）や、イマジンを1件も解決できないリストでは何もしない（安全側）。
/// 以降の装備替えの即時追従は従来の召喚検知（pending 方式）が補完する。
fn apply_skill_list_imagines(
    uid: i64,
    player_entity: &mut Entity,
    infos: &[pb::SkillLevelInfo],
    src: &'static str,
) {
    if infos.len() < MIN_FULL_SKILL_LIST_LEN {
        return;
    }
    let mut slots: Vec<ImagineSlot> = Vec::new();
    let mut role_skill_candidates: Vec<ImagineSlot> = Vec::new();
    for info in infos {
        // ロールスキル(簡易版バトルイマジン、最大 MAX_ROLE_SKILL_IMAGINES 枠)は実イマジンとは
        // 別枠のIDを持つ。先にこちらを判定して continue することで、role-skill id が万一
        // imagine_name() 側でも解決できてしまっても実イマジンの2枠(slots)へは絶対に混入させない
        // （多重防御）。dedup/上限cap は下の実イマジン側(slots)と同じ方針。
        if let Some(name) = crate::engine::imagine_skills::role_skill_imagine_name(info.skill_id) {
            if role_skill_candidates.iter().any(|s| s.name == name) {
                continue;
            }
            if role_skill_candidates.len() >= MAX_ROLE_SKILL_IMAGINES {
                warn!(
                    "skill list imagines: uid={uid} more than {MAX_ROLE_SKILL_IMAGINES} role skill entries; extra id={} name={name} ignored",
                    info.skill_id
                );
                continue;
            }
            role_skill_candidates.push(ImagineSlot {
                name,
                last_seen: next_imagine_seq(),
                tier: info.remodel_level.max(0),
                pending_hits: 0,
            });
            continue;
        }
        let Some(name) = crate::engine::imagine_skills::imagine_name(info.skill_id) else {
            continue;
        };
        if slots.iter().any(|s| s.name == name) {
            continue;
        }
        if slots.len() >= MAX_IMAGINE_NAMES {
            warn!(
                "skill list imagines: uid={uid} more than {MAX_IMAGINE_NAMES} arcane entries; extra id={} name={name} ignored",
                info.skill_id
            );
            continue;
        }
        slots.push(ImagineSlot {
            name,
            last_seen: next_imagine_seq(),
            tier: info.remodel_level.max(0),
            pending_hits: 0,
        });
    }

    // ロールスキル枠の権威的更新（実イマジンの slots 判定とは独立。slots が空でも行う）。
    // 両辺とも同一のフルスナップショットの決定的な走査順から作られるため、順序込みの比較でよい
    // （imagines/slots の全置換比較と同じ前提）。
    let role_skill_changed = player_entity.role_skill_imagines.len() != role_skill_candidates.len()
        || player_entity
            .role_skill_imagines
            .iter()
            .zip(&role_skill_candidates)
            .any(|(a, b)| a.name != b.name || a.tier != b.tier);
    if role_skill_changed {
        if role_skill_candidates.is_empty() {
            info!("role skill imagine cleared (skill list [{src}]): uid={uid}");
        } else {
            info!(
                "role skill imagine confirmed (skill list [{src}]): uid={uid} {}",
                role_skill_candidates
                    .iter()
                    .map(|s| format!("{}({})", s.name, s.tier))
                    .collect::<Vec<_>>()
                    .join("/")
            );
        }
        player_entity.role_skill_imagines = role_skill_candidates;
        name_cache::update_role_skill_imagines(
            uid,
            &player_entity.role_skill_imagine_names(),
            &player_entity.role_skill_imagine_tiers(),
        );
    }

    if slots.is_empty() {
        // 今回のフルリストに実イマジンの canonical id が1件も無かった場合、確定済みの
        // role-skill 候補（複数件）と同名の陳腐化した imagines エントリがあれば全て除去する。
        // 実イマジン（SlotPositionId 7/8）が装備されていれば、このフルリスト（attr116）に必ず
        // canonical id として現れるはず（apply_skill_list_imagines のトップコメント参照）なので、
        // ここに現れないことは「未確定」ではなく「実イマジンではない」ことの確証になる。
        // role_skill_imagines が空だった間（初回スナップショット到達前）に summon ヒューリスティックが
        // 誤って実イマジンとして確定させてしまった残骸（rule3）を、この確証で救済する。
        if !player_entity.role_skill_imagines.is_empty() {
            let role_skill_names = player_entity.role_skill_imagine_names();
            let before_len = player_entity.imagines.len();
            player_entity.imagines.retain(|s| !role_skill_names.contains(&s.name));
            if player_entity
                .pending_imagine
                .as_ref()
                .is_some_and(|p| role_skill_names.contains(&p.name))
            {
                player_entity.pending_imagine = None;
            }
            if player_entity.imagines.len() != before_len {
                info!(
                    "battle imagine evicted (misattributed role skill echo, skill list [{src}]): uid={uid} names=[{}]",
                    role_skill_names.join("/")
                );
                name_cache::update_imagine(
                    uid,
                    &player_entity.imagine_display_names(),
                    &player_entity.imagine_tiers(),
                );
            }
        }
        return;
    }
    // 名前と凸数が現状と同一なら何もしない（鮮度・キャッシュの無駄な更新を避ける）。
    let same = player_entity.imagines.len() == slots.len()
        && player_entity
            .imagines
            .iter()
            .zip(&slots)
            .all(|(a, b)| a.name == b.name && a.tier == b.tier);
    if same {
        return;
    }
    info!(
        "battle imagine confirmed (skill list [{src}]): uid={uid} {}",
        slots
            .iter()
            .map(|s| format!("{}({})", s.name, s.tier))
            .collect::<Vec<_>>()
            .join("/")
    );
    player_entity.imagines = slots;
    player_entity.pending_imagine = None;
    name_cache::update_imagine(
        uid,
        &player_entity.imagine_display_names(),
        &player_entity.imagine_tiers(),
    );
}

/// 装備スキルリスト attr の調査用ログ（`BPSR_PROBE=1` の開発時のみ呼ばれる）。内容が
/// 前回ログ時から変わった時だけ全 skill_id を記録し、イマジン奥義候補（canonical 解決 or
/// 奥義！/絶技！接頭辞）を明示行で残す。src はどの経路で届いたか（enter_scene/appear/delta）。
fn log_skill_level_id_list(uid: i64, src: &'static str, infos: &[pb::SkillLevelInfo]) {
    let ids: Vec<i32> = infos.iter().map(|i| i.skill_id).collect();
    let Ok(mut last) = LAST_SKILL_LIST.lock() else {
        return;
    };
    if last.get(&uid) == Some(&ids) {
        return;
    }
    let detail: Vec<String> = infos
        .iter()
        .map(|i| format!("{}(lv{},t{})", i.skill_id, i.current_level, i.remodel_level))
        .collect();
    info!("skill list attr [{src}]: uid={uid} n={} ids=[{}]", ids.len(), detail.join(", "));
    for i in infos {
        // イマジン候補の判定は2系統: ①ImagineSkillNames の canonical 解決（確実）
        // ②日本語スキル名の「奥義！」「絶技！」接頭辞（未登録の新イマジン発見用）。
        let canonical = crate::engine::imagine_skills::imagine_name(i.skill_id);
        let ja_name = crate::engine::skill_names::skill_name_ja(i.skill_id);
        let prefix_hit =
            ja_name.is_some_and(|n| n.starts_with("奥義！") || n.starts_with("絶技！"));
        if canonical.is_some() || prefix_hit {
            info!(
                "skill list attr [{src}]: uid={uid} imagine arcane: id={} imagine_name={} skill_name={} tier={}",
                i.skill_id,
                canonical.as_deref().unwrap_or("-"),
                ja_name.unwrap_or("-"),
                i.remodel_level
            );
        }
    }
    last.insert(uid, ids);
}

/// 装備データ attr の調査用ログ（`BPSR_PROBE=1` の開発時のみ呼ばれる）。抑制方式は
/// log_skill_level_id_list と同様。
fn log_equip_data(uid: i64, src: &'static str, raw_data: &[u8]) {
    let equips = decode_equip_nine_list(raw_data);
    let pairs: Vec<(i32, i32)> = equips.iter().map(|e| (e.slot, e.equip_id)).collect();
    let Ok(mut last) = LAST_EQUIP_LIST.lock() else {
        return;
    };
    if last.get(&uid) == Some(&pairs) {
        return;
    }
    let detail: Vec<String> =
        pairs.iter().map(|(slot, id)| format!("slot{slot}={id}")).collect();
    info!("equip data attr [{src}]: uid={uid} n={} [{}]", pairs.len(), detail.join(", "));
    last.insert(uid, pairs);
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
                    // probe: EnterScene の実フィールド構造（decode 型に無いものも含む）を棚卸し
                    crate::probe::scan_message("EnterScene(0x3)", &data, Some(1));
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
                    // probe: SyncContainerData(v_data=CharSerialize) の実フィールド構造を棚卸し
                    crate::probe::scan_message("WorldEnterSnapshot(0x15)", &data, Some(1));
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

                    if crate::probe::enabled() {
                        log::info!(
                            "PROBE buff-opcode [0x3003]: raw=[{}B]{}",
                            data.len(),
                            data.iter().map(|b| format!("{b:02x}")).collect::<String>()
                        );
                    }

                    // BuffSnapshot と BuffTick は同一 op で届き、フィールドが全て varint で
                    // 番号も重なるため protobuf 上どちらの decode も常に成功してしまう。
                    // よって型では判別できず、両方を試す必要がある。誤った型で decode された
                    // 側は host_uuid が Player にならず apply_* 内で無視されるため害はない。
                    // ここを else-if にすると BuffTick 形式のデバフ更新が落ちる（regression 注意）。
                    if let Ok(msg) = pb::BuffSnapshot::decode(data.as_slice()) {
                        crate::probe::log_buff_snapshot("opcode-0x3003", &data, &msg);
                        encounter.buff_tracker.apply_full_info(&msg, ts);
                    }
                    if let Ok(msg) = pb::BuffTick::decode(data.as_slice()) {
                        crate::probe::log_buff_tick("opcode-0x3003", &data, &msg);
                        encounter.buff_tracker.apply_change(&msg, ts);
                    }
                }

                Pkt::BuffSnapshotBundle => {
                    let ts = now_ms();
                    if let Ok(msg) = pb::BuffSnapshotBundle::decode(data.as_slice()) {
                        for buff in &msg.buff_infos {
                            if crate::probe::enabled() {
                                crate::probe::log_buff_snapshot("bundle-0x3005", &buff.encode_to_vec(), buff);
                            }
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
            if crate::probe::enabled() {
                crate::probe::log_attrs(
                    &format!("appear {target_entity_type:?}"),
                    target_uuid,
                    &attrs.attrs,
                );
            }
            match target_entity_type {
                EntityKind::Player => {
                    process_player_attrs(target_uid, target_entity, &attrs.attrs, "appear");
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

    if crate::probe::enabled() {
        log_container_equips(player_uid, v_data);
        log_aoyi_slots(player_uid, v_data);
    }
    // 補足: 装備中イマジンの権威的確定は attr 116（フル装備スキルリスト）経路で行う
    // （apply_skill_list_imagines）。0x15 の profession_list.slot_skill_info_map には
    // クラススキルしか載らず、イマジン装備枠は無いことを実機確認済み（2026-07-10）。
}

/// SyncContainerData(0x15) の奥義（バトルイマジン）装備スロットの調査用ログ
/// （`BPSR_PROBE=1` の開発時のみ呼ばれる）。装備中イマジンの特定は attr 116 経路
/// （`apply_skill_list_imagines`）で確定済みのため、これは 0x15 側の構造調査専用の記録。
fn log_aoyi_slots(player_uid: i64, v_data: &pb::PlayerSnapshot) {
    let Some(profession) = v_data.profession_list.as_ref() else {
        return;
    };
    // 装備中判定のデバッグ: 職業マップのキー一覧と、現在職業の装備スロット→スキルIDを記録する
    // （apply_container_imagines が何も確定しない場合の原因切り分け用）。
    let mut prof_ids: Vec<i32> = profession.profession_list.keys().copied().collect();
    prof_ids.sort_unstable();
    info!(
        "professions: uid={player_uid} cur={} available={prof_ids:?}",
        profession.cur_profession_id
    );
    if let Some(cur) = profession.profession_list.get(&profession.cur_profession_id) {
        let mut slots: Vec<(i32, i32)> =
            cur.slot_skill_info_map.iter().map(|(s, k)| (*s, *k)).collect();
        slots.sort_unstable();
        info!("profession slots: uid={player_uid} {slots:?}");
    } else {
        info!("profession slots: uid={player_uid} (cur profession not in map)");
    }
    if profession.aoyi_skill_info_map.is_empty() {
        info!("aoyi slots: uid={player_uid} (empty)");
        return;
    }
    let mut slots: Vec<_> = profession.aoyi_skill_info_map.iter().collect();
    slots.sort_by_key(|(slot, _)| **slot);
    for (slot, info) in slots {
        let via_imagine = crate::engine::imagine_skills::imagine_name(info.skill_id)
            .unwrap_or_else(|| "-".to_string());
        let via_skill =
            crate::engine::skill_names::skill_name_ja(info.skill_id).unwrap_or("-");
        info!(
            "aoyi slot: uid={player_uid} slot={slot} skill_id={} lv={} tier={} imagine_name={via_imagine} skill_name={via_skill}",
            info.skill_id, info.level, info.remodel_level
        );
    }
}

/// SyncContainerData(0x15) の装備リスト×アイテムパッケージ突合の調査用ログ
/// （`BPSR_PROBE=1` の開発時のみ呼ばれる）。装備スロットの item_uuid をパッケージ内アイテムと
/// 突合して config_id を引き、どのパッケージ（type 6=バトルイマジンの想定）に居たかを記録する。
fn log_container_equips(player_uid: i64, v_data: &pb::PlayerSnapshot) {
    let packages = v_data.item_package.as_ref().map(|p| &p.packages);
    if let Some(packages) = packages {
        let mut summary: Vec<String> = packages
            .iter()
            .map(|(pkg_id, bag)| format!("pkg{}:{}items", pkg_id, bag.items.len()))
            .collect();
        summary.sort();
        info!("container packages: uid={player_uid} [{}]", summary.join(", "));
    }
    let Some(equip) = v_data.equip.as_ref() else {
        info!("container equip: uid={player_uid} (no equip list)");
        return;
    };
    let mut slots: Vec<_> = equip.equip_list.iter().collect();
    slots.sort_by_key(|(slot, _)| **slot);
    for (slot, info) in slots {
        // item_uuid を全パッケージから探し、パッケージIDと config_id を特定する。
        let mut found: Option<(i32, i32)> = None; // (pkg_id, config_id)
        if let Some(packages) = packages {
            for (pkg_id, bag) in packages {
                if let Some(item) = bag.items.get(&(info.item_uuid as i64)) {
                    found = Some((*pkg_id, item.config_id));
                    break;
                }
            }
        }
        match found {
            Some((pkg_id, config_id)) => info!(
                "container equip: uid={player_uid} slot={slot} refine={} pkg={pkg_id} config_id={config_id}",
                info.equip_slot_refine_level
            ),
            None => info!(
                "container equip: uid={player_uid} slot={slot} refine={} item_uuid={} (item not found in packages)",
                info.equip_slot_refine_level, info.item_uuid
            ),
        }
    }
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
            if crate::probe::enabled() {
                crate::probe::log_attrs(
                    &format!("delta {target_entity_type:?}"),
                    target_uuid,
                    &attrs_collection.attrs,
                );
            }
            match target_entity_type {
                EntityKind::Player => {
                    process_player_attrs(target_uid, target_entity, &attrs_collection.attrs, "delta");
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

                if crate::probe::enabled() {
                    let decoded_payload = if buff.body_raw.is_empty() {
                        None
                    } else {
                        pb::BuffPayload::decode(buff.body_raw.as_slice()).ok()
                    };
                    crate::probe::log_buff_event(buff, decoded_payload.as_ref());
                }

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
                        crate::probe::log_buff_snapshot("scene-add", &body.detail_raw, &info);
                        encounter
                            .buff_tracker
                            .apply_buff_add(buff_uuid, &info, ts, target_uid);
                    }
                    LOGIC_EFFECT_BUFF_CHANGE => {
                        let Ok(change) = pb::BuffChange::decode(body.detail_raw.as_slice()) else {
                            continue;
                        };
                        crate::probe::log_buff_change("scene-change", &body.detail_raw, &change);
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
    if crate::probe::enabled() {
        crate::probe::log_attrs("enter_scene Player", player_ent.uuid, &attrs.attrs);
    }
    // EnterScene は自キャラの入場通知。local_player_uid 未確定ならここで確定させる
    // （初回フル同期をこの経路で確実に取得するため）。
    if encounter.local_player_uid == 0 {
        encounter.local_player_uid = player_uid;
    }
    let target_entity = get_or_create_entity(encounter, player_uid, EntityKind::Player);
    target_entity.entity_type = EntityKind::Player;
    process_player_attrs(player_uid, target_entity, &attrs.attrs, "enter_scene");
}

fn process_player_attrs(
    uid: i64,
    player_entity: &mut Entity,
    attrs: &[pb::RawAttr],
    src: &'static str,
) {
    use crate::capture::binary_reader::BinaryReader;

    let mut cache_name: Option<String> = None;
    let mut cache_class: Option<i32> = None;
    let mut cache_score: Option<i32> = None;
    let mut cache_season_lv: Option<i32> = None;
    let mut cache_season_str: Option<i32> = None;

    for attr in attrs {
        // 空 raw_data はスキップしない: ステータスが 0 になった通知（no-value=空 raw_data）を
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
            // 装備スキルリスト（フルの場合は装備イマジンの一次情報。診断ログ+権威的更新）。
            attr_type::ATTR_SKILL_LEVEL_ID_LIST => {
                if !attr.raw_data.is_empty() {
                    let infos = decode_skill_level_info_list(&attr.raw_data);
                    if crate::probe::enabled() {
                        log_skill_level_id_list(uid, src, &infos);
                    }
                    apply_skill_list_imagines(uid, player_entity, &infos, src);
                }
            }
            attr_type::ATTR_EQUIP_DATA => {
                if crate::probe::enabled() && !attr.raw_data.is_empty() {
                    log_equip_data(uid, src, &attr.raw_data);
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

    /// summon_spawn_delta の凸数(AttrSkillRemodelLevel)付き版。
    fn summon_spawn_delta_with_tier(owner_uid: i64, skill_id: i32, tier: i32) -> pb::SceneDelta {
        let mut delta = summon_spawn_delta(owner_uid, skill_id);
        if let Some(attrs) = delta.attrs.as_mut() {
            attrs.attrs.push(pb::RawAttr {
                id: attr_type::ATTR_SKILL_REMODEL_LEVEL,
                raw_data: enc_varint(tier as u64),
            });
        }
        delta
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

    // ⑤b 休眠相方（召喚報告ID未登録で二度と検知されない相方）による pending 永久スタックの
    // 自己修復。相方 B' が rule5 を満たす新規名を出さない限り、pending(C) は PENDING_PROMOTE_HITS
    // 回の再検知で単独確定へ昇格し、旧確定ペア[A,B]を両方破棄する（2枠目は「未知」へ縮小）。
    // 閾値未満では確定は不変であることも境界値として確認する。
    #[test]
    fn pending_self_heals_after_threshold_hits_when_partner_stays_dormant() {
        let mut enc = Encounter::default();
        enc.entities.insert(7, player());

        process_scene_delta(&mut enc, summon_spawn_delta(7, 1_007_740)); // A: ヴェノミーンの巣
        process_scene_delta(&mut enc, summon_spawn_delta(7, 2_900_240)); // B: アルーナ
        process_scene_delta(&mut enc, summon_spawn_delta(7, 2_900_840)); // C（新規）→ rule4: pending(hits=1)

        // hits=2（PENDING_PROMOTE_HITS=3 未満）→ まだ昇格しない
        process_scene_delta(&mut enc, summon_spawn_delta(7, 2_900_840));
        assert_eq!(
            enc.entities[&7].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()],
            "閾値未満の再検知では自己修復してはいけない"
        );

        // hits=3（閾値到達）→ 自己修復: 旧確定ペア[A,B]を両方破棄し、C だけを単独確定にする
        process_scene_delta(&mut enc, summon_spawn_delta(7, 2_900_840));
        assert_eq!(
            enc.entities[&7].imagine_display_names(),
            vec!["ロローラ".to_string()],
            "休眠相方のため C だけの単独確定へ自己修復するべき"
        );
        assert!(enc.entities[&7].pending_imagine.is_none());

        // 自己修復後に別の新規名 D を検知 → rule3（定員未満）で 2 枠目へ直接追加され、
        // 混在ペアを経由せず [C, D] へ回復する。
        process_scene_delta(&mut enc, summon_spawn_delta(7, 1_002_830)); // D: フロストオーガ
        assert_eq!(
            enc.entities[&7].imagine_display_names(),
            vec!["ロローラ".to_string(), "フロストオーガ".to_string()]
        );
    }

    // 装備スキルリスト/装備データ attr の decode 検証（タグ付き repeated 形式。
    // 2026-07-10 ダンジョン実測 hex と同じワイヤ形式で roundtrip する）。
    #[test]
    fn decode_skill_level_info_list_roundtrip() {
        let list = pb::SkillLevelList {
            skills: vec![
                pb::SkillLevelInfo { skill_id: 3926, current_level: 1, remodel_level: 5 },
                pb::SkillLevelInfo { skill_id: 2424, current_level: 4, remodel_level: 0 },
                pb::SkillLevelInfo { skill_id: 3910, current_level: 1, remodel_level: 3 },
            ],
        };
        let decoded = decode_skill_level_info_list(&list.encode_to_vec());
        assert_eq!(
            decoded.iter().map(|i| (i.skill_id, i.current_level, i.remodel_level)).collect::<Vec<_>>(),
            vec![(3926, 1, 5), (2424, 4, 0), (3910, 1, 3)]
        );
    }

    // 実機ダンジョンで観測した attr116 の生バイト列（先頭部分）がそのまま decode できること。
    #[test]
    fn decode_skill_level_info_list_real_dungeon_bytes() {
        let raw: Vec<u8> = vec![
            0x0a, 0x05, 0x08, 0xd9, 0x36, 0x10, 0x01, // {skill_id=7001, lv=1}
            0x0a, 0x07, 0x08, 0x8d, 0x12, 0x10, 0x1e, 0x18, 0x03, // {2317, lv30, t3}
            0x0a, 0x05, 0x08, 0xf2, 0x19, 0x10, 0x01, // {3314, lv=1}
        ];
        let decoded = decode_skill_level_info_list(&raw);
        assert_eq!(
            decoded.iter().map(|i| (i.skill_id, i.current_level, i.remodel_level)).collect::<Vec<_>>(),
            vec![(7001, 1, 0), (2317, 30, 3), (3314, 1, 0)]
        );
    }

    #[test]
    fn decode_equip_nine_list_roundtrip() {
        let list = pb::EquipNineList {
            equips: vec![
                pb::EquipNine { slot: 200, equip_id: 2_001_032 },
                pb::EquipNine { slot: 207, equip_id: 2_071_011 },
            ],
        };
        let decoded = decode_equip_nine_list(&list.encode_to_vec());
        assert_eq!(
            decoded.iter().map(|e| (e.slot, e.equip_id)).collect::<Vec<_>>(),
            vec![(200, 2_001_032), (207, 2_071_011)]
        );
    }

    // 凸数(AttrSkillRemodelLevel)が表示ラベル「名前(N)」へ反映されること、凸数無し検知では
    // (N) が付かないこと、再検知で凸数が判明したら追従し name_cache へ並列永続化されることを検証。
    #[test]
    fn imagine_tier_shown_in_labels_and_updates_on_redetection() {
        let mut enc = Encounter::default();
        let uid = 990_003; // name_cache はプロセス共有のため専用 uid を使う
        enc.entities.insert(uid, player());

        // 凸数付き検知 → ラベルに (5)。凸数無し検知 → 名前のみ。
        process_scene_delta(&mut enc, summon_spawn_delta_with_tier(uid, 1_007_740, 5));
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 2_900_240));
        assert_eq!(
            enc.entities[&uid].imagine_display_labels(),
            vec!["ヴェノミーンの巣(5)".to_string(), "アルーナ".to_string()]
        );
        // 一致判定・永続化用の名前一覧は凸数を含まない（名前のみ）。
        assert_eq!(
            enc.entities[&uid].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );

        // 再検知で凸数が判明したら追従する（0→3）。
        process_scene_delta(&mut enc, summon_spawn_delta_with_tier(uid, 2_900_240, 3));
        assert_eq!(
            enc.entities[&uid].imagine_display_labels(),
            vec!["ヴェノミーンの巣(5)".to_string(), "アルーナ(3)".to_string()]
        );

        // name_cache へ凸数が並列配列として永続化される。
        let cached = name_cache::lookup(uid).expect("cache entry should exist");
        assert_eq!(
            cached.imagine_names,
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );
        assert_eq!(cached.imagine_tiers, vec![5, 3]);
    }

    /// クラススキル多数+イマジン奥義（canonical 39xx）を混ぜたフル装備スキルリストの
    /// SceneDelta（attr 116）を作る。実機のフルリスト（40件超）を模して閾値を満たす。
    fn skill_list_delta(uid: i64, arcane: &[(i32, i32)]) -> pb::SceneDelta {
        let mut skills: Vec<pb::SkillLevelInfo> = (0..MIN_FULL_SKILL_LIST_LEN as i32)
            .map(|i| pb::SkillLevelInfo {
                skill_id: 2400 + i, // クラススキル帯（イマジンとして解決されない）
                current_level: 30,
                remodel_level: 6,
            })
            .collect();
        for &(id, tier) in arcane {
            skills.push(pb::SkillLevelInfo { skill_id: id, current_level: 1, remodel_level: tier });
        }
        let raw = pb::SkillLevelList { skills }.encode_to_vec();
        let player_uuid = (uid << 16) | 640; // Player 型コード
        pb::SceneDelta {
            uuid: player_uuid,
            attrs: Some(pb::EntityAttrs {
                uuid: player_uuid,
                attrs: vec![pb::RawAttr {
                    id: attr_type::ATTR_SKILL_LEVEL_ID_LIST,
                    raw_data: raw,
                }],
            }),
            buff_list: None,
            skill_effects: None,
        }
    }

    // フル装備スキルリスト(attr 116)由来の装備イマジン確定: canonical 39xx（凸数付き）を
    // 抽出して古い確定ペア・pending を権威的に置き換えること、クラススキルは無視されること、
    // 部分リスト（閾値未満）では何もしないことを検証。他プレイヤーの appear/delta と自分の
    // enter_scene が同じ経路を通る（process_player_attrs の 116 アーム）。
    #[test]
    fn full_skill_list_sets_imagines_authoritatively() {
        let mut enc = Encounter::default();
        let uid = 990_004; // name_cache はプロセス共有のため専用 uid を使う
        enc.entities.insert(uid, player());

        // 事前状態: 古い確定ペア[A,B]+pending(C) を召喚検知で作っておく。
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 1_007_740)); // A
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 2_900_240)); // B
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 2_900_840)); // C → pending
        assert!(enc.entities[&uid].pending_imagine.is_some());

        // フルリスト（クラススキル10+イマジン2: 3902=サンダーオーガ凸0, 3906=フロストオーガ凸2）
        process_scene_delta(&mut enc, skill_list_delta(uid, &[(3902, 0), (3906, 2)]));
        assert_eq!(
            enc.entities[&uid].imagine_display_labels(),
            vec!["サンダーオーガ".to_string(), "フロストオーガ(2)".to_string()]
        );
        assert!(enc.entities[&uid].pending_imagine.is_none());

        // name_cache にも名前+凸数が永続化される。
        let cached = name_cache::lookup(uid).expect("cache entry should exist");
        assert_eq!(
            cached.imagine_names,
            vec!["サンダーオーガ".to_string(), "フロストオーガ".to_string()]
        );
        assert_eq!(cached.imagine_tiers, vec![0, 2]);

        // 部分リスト（閾値未満）は無視され、確定表示は変わらない。
        let mut partial = skill_list_delta(uid, &[(3942, 5)]);
        if let Some(attrs) = partial.attrs.as_mut() {
            let raw = pb::SkillLevelList {
                skills: vec![pb::SkillLevelInfo {
                    skill_id: 3942,
                    current_level: 1,
                    remodel_level: 5,
                }],
            }
            .encode_to_vec();
            attrs.attrs[0].raw_data = raw;
        }
        process_scene_delta(&mut enc, partial);
        assert_eq!(
            enc.entities[&uid].imagine_display_names(),
            vec!["サンダーオーガ".to_string(), "フロストオーガ".to_string()],
            "閾値未満の部分リストで確定表示を壊してはいけない"
        );

        // その後の装備替えは従来の召喚検知が追従する（新規名ロローラ→pending 止まり）。
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 2_900_840));
        assert!(enc.entities[&uid].pending_imagine.is_some());
        assert_eq!(
            enc.entities[&uid].imagine_display_names(),
            vec!["サンダーオーガ".to_string(), "フロストオーガ".to_string()]
        );
    }

    // ロールスキル(簡易版バトルイマジン)は実イマジンの2枠(slots)とは別枠として確定し、対応する
    // canonicalの表示名を role_skill_imagines に反映する。imagines には一切混入しない
    // （3021→サンダーオーガのロールスキルIDを、実イマジン2件(3906/3910)と混ぜたフルリストで確認）。
    #[test]
    fn full_skill_list_sets_role_skill_imagine_without_polluting_real_slots() {
        let mut enc = Encounter::default();
        let uid = 990_005; // name_cache はプロセス共有のため専用 uid を使う
        enc.entities.insert(uid, player());

        process_scene_delta(&mut enc, skill_list_delta(uid, &[(3906, 1), (3910, 0), (3021, 4)]));

        assert_eq!(
            enc.entities[&uid].imagine_display_labels(),
            vec!["フロストオーガ(1)".to_string(), "虚蝕オーガ".to_string()],
            "role skill id must not appear in the real 2-slot imagines array"
        );
        assert_eq!(
            enc.entities[&uid].role_skill_imagine_labels(),
            vec!["サンダーオーガ(4)".to_string()]
        );

        let cached = name_cache::lookup(uid).expect("cache entry should exist");
        assert_eq!(cached.role_skill_imagine_names, vec!["サンダーオーガ".to_string()]);
        assert_eq!(cached.role_skill_imagine_tiers, vec![4]);
    }

    // エンドツーエンド回帰テスト: ロールスキルの4枠(SlotPositionId 21-24)を同時装備しているケース。
    // フル attr116 スナップショットで4件を検出→role_skill_imagines へ確定、続けてそれぞれの
    // canonical id(3902/3901/3908/3943)で召喚エコーが来ても imagines/pending_imagine には一切
    // 触れず吸収され、最終的に role_skill_imagine_labels() が4件とも欠落なく表示されることを確認する
    // （ユーザー指摘の「4枠同時装備で3件目以降が黙って消える/フラッピングが再発する」ケースの直接検証）。
    #[test]
    fn full_skill_list_and_summon_echoes_handle_four_simultaneous_role_skills() {
        let mut enc = Encounter::default();
        let uid = 990_012; // name_cache 専用 uid
        enc.entities.insert(uid, player());

        // フルリスト: 実イマジン2枠(3906/3910) + ロールスキル4枠(3021/3022/3023/3024)。
        process_scene_delta(
            &mut enc,
            skill_list_delta(
                uid,
                &[(3906, 1), (3910, 0), (3021, 4), (3022, 2), (3023, 0), (3024, 3)],
            ),
        );

        let expected_labels = vec![
            "サンダーオーガ(4)".to_string(),
            "フレイムオーガ(2)".to_string(),
            "ムークボス".to_string(),
            "鉄牙(3)".to_string(),
        ];
        assert_eq!(
            enc.entities[&uid].role_skill_imagine_labels(),
            expected_labels,
            "all 4 simultaneous role skill slots must resolve without dropping any"
        );
        assert_eq!(
            enc.entities[&uid].imagine_display_labels(),
            vec!["フロストオーガ(1)".to_string(), "虚蝕オーガ".to_string()],
            "role skill ids must never pollute the real 2-slot imagines array"
        );

        // 各ロールスキルの召喚エコー(canonical id)が来ても imagines/pending には一切触れない。
        for skill_id in [3902, 3901, 3908, 3943] {
            process_scene_delta(&mut enc, summon_spawn_delta(uid, skill_id));
        }

        assert_eq!(
            enc.entities[&uid].imagine_display_labels(),
            vec!["フロストオーガ(1)".to_string(), "虚蝕オーガ".to_string()],
            "summon echoes of all 4 role skills must not disturb the confirmed real imagine pair"
        );
        assert!(enc.entities[&uid].pending_imagine.is_none());
        assert_eq!(
            enc.entities[&uid].role_skill_imagine_labels(),
            expected_labels,
            "role skill labels must remain intact after echo absorption for all 4 slots"
        );

        let cached = name_cache::lookup(uid).expect("cache entry should exist");
        assert_eq!(
            cached.role_skill_imagine_names,
            vec![
                "サンダーオーガ".to_string(),
                "フレイムオーガ".to_string(),
                "ムークボス".to_string(),
                "鉄牙".to_string(),
            ]
        );
        assert_eq!(cached.role_skill_imagine_tiers, vec![4, 2, 0, 3]);
    }

    // 回帰テスト: ロールスキルのみを持つプレイヤー（実イマジン0枠）。role_skill_imagine が
    // まだ未確定（None）の間に、ロールスキルの簡易発動がプロトコル上は実イマジンと同一の
    // 召喚シグナル(AttrSkillId=canonical id)を出すため、summon ヒューリスティック(rule3)が
    // それを誤って imagines へ確定させてしまう。後続のフル attr116 スナップショットに
    // ロールスキルIDのみ（canonical idは0件）が載っていれば、この陳腐化した imagines
    // エントリを除去し role_skill_imagine を正しく設定できることを確認する。
    #[test]
    fn full_skill_list_evicts_stale_imagine_misattributed_before_role_skill_known() {
        let mut enc = Encounter::default();
        let uid = 990_010; // name_cache 専用 uid
        enc.entities.insert(uid, player());

        // role_skill_imagine 未確定のため、召喚エコー(canonical id=3902→サンダーオーガ)が
        // rule3(定員未満)で誤って実イマジンとして確定してしまう。
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 3902));
        assert_eq!(
            enc.entities[&uid].imagine_display_names(),
            vec!["サンダーオーガ".to_string()],
            "precondition: summon echo must be misattributed to imagines before role_skill_imagine is known"
        );

        // フルリスト到達（ロールスキルID(3021→サンダーオーガ)のみ・canonical idは0件）。
        process_scene_delta(&mut enc, skill_list_delta(uid, &[(3021, 4)]));

        assert!(
            enc.entities[&uid].imagine_display_names().is_empty(),
            "stale misattributed imagines entry must be evicted once the full snapshot proves it's not a real slot"
        );
        assert_eq!(
            enc.entities[&uid].role_skill_imagine_labels(),
            vec!["サンダーオーガ(4)".to_string()]
        );

        let cached = name_cache::lookup(uid).expect("cache entry should exist");
        assert!(cached.imagine_names.is_empty());
        assert_eq!(cached.role_skill_imagine_names, vec!["サンダーオーガ".to_string()]);
        assert_eq!(cached.role_skill_imagine_tiers, vec![4]);
    }

    // 直前にロールスキルが確定していた状態で、次のフルリストに対象IDが含まれなければ
    // role_skill_imagine をクリアする（ロールスキル未装備化・対象変更等の反映）。
    #[test]
    fn full_skill_list_clears_role_skill_imagine_when_absent() {
        let mut enc = Encounter::default();
        let uid = 990_006; // name_cache 専用 uid
        enc.entities.insert(uid, player());

        process_scene_delta(&mut enc, skill_list_delta(uid, &[(3906, 1), (3910, 0), (3021, 4)]));
        assert!(!enc.entities[&uid].role_skill_imagines.is_empty());

        // 同じ実イマジン2枠のみでロールスキル対象IDを含まないフルリストが届く。
        process_scene_delta(&mut enc, skill_list_delta(uid, &[(3906, 1), (3910, 0)]));
        assert!(
            enc.entities[&uid].role_skill_imagines.is_empty(),
            "role skill imagine must be cleared when absent from a full snapshot"
        );

        let cached = name_cache::lookup(uid).expect("cache entry should exist");
        assert!(cached.role_skill_imagine_names.is_empty());
        assert!(cached.role_skill_imagine_tiers.is_empty());
    }

    // 回帰テスト(本バグの修正確認): ロールスキル枠に確定済みの名前と同名の召喚シグナルが
    // 来ても、定員一杯の confirmed pair / pending には一切触れない。
    #[test]
    fn role_skill_echo_does_not_disturb_confirmed_imagines_or_pending() {
        let mut enc = Encounter::default();
        let uid = 990_007; // name_cache 専用 uid
        enc.entities.insert(uid, player());

        process_scene_delta(&mut enc, summon_spawn_delta(uid, 1_007_740)); // ヴェノミーンの巣
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 2_900_240)); // アルーナ
        assert_eq!(
            enc.entities[&uid].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );

        // ロールスキル枠を「ロローラ」として既に確定済みにしておく（apply_skill_list_imagines相当）。
        enc.entities.get_mut(&uid).unwrap().role_skill_imagines = vec![ImagineSlot {
            name: "ロローラ".to_string(),
            last_seen: 0,
            tier: 0,
            pending_hits: 0,
        }];

        // ロールスキルの簡易発動による召喚シグナル（実イマジンと同一の召喚報告ID経由=ロローラ）。
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 2_900_840));

        assert_eq!(
            enc.entities[&uid].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()],
            "role skill echo must not disturb the confirmed real imagine pair"
        );
        assert!(
            enc.entities[&uid].pending_imagine.is_none(),
            "role skill echo must not create a pending candidate"
        );
        assert_eq!(
            enc.entities[&uid].role_skill_imagine_names(),
            vec!["ロローラ".to_string()]
        );
    }

    // 回帰テスト: role_skill_imagine と確定済み実イマジン枠(imagines)が偶然同じ名前を共有していても、
    // rule1(既存確定スロットの再検知)はロールスキル短絡ブロックより先に評価されるため、通常どおり
    // pending の昇格（単枠交換）が機能し、role_skill_imagine には一切影響しないことを確認する。
    #[test]
    fn shared_name_between_confirmed_imagine_and_role_skill_still_follows_rule1_single_slot_swap() {
        let mut enc = Encounter::default();
        let uid = 990_011; // name_cache 専用 uid
        enc.entities.insert(uid, player());

        {
            let owner = enc.entities.get_mut(&uid).unwrap();
            owner.imagines = vec![
                ImagineSlot {
                    name: "ヴェノミーンの巣".to_string(), // A
                    last_seen: 0,
                    tier: 0,
                    pending_hits: 0,
                },
                ImagineSlot {
                    name: "アルーナ".to_string(), // B
                    last_seen: 1,
                    tier: 0,
                    pending_hits: 0,
                },
            ];
            owner.pending_imagine = Some(ImagineSlot {
                name: "ロローラ".to_string(), // P
                last_seen: 2,
                tier: 0,
                pending_hits: 1,
            });
            owner.role_skill_imagines = vec![ImagineSlot {
                name: "ヴェノミーンの巣".to_string(), // A と同名を role skill 側も指す
                last_seen: 3,
                tier: 0,
                pending_hits: 0,
            }];
        }

        // A(ヴェノミーンの巣)の再検知 → rule1 がロールスキル短絡ブロックより先に評価され、
        // 通常どおり pending(P) が確定へ昇格して B を置き換える（role_skill_imagines が同名でも無関係）。
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 1_007_740));

        assert_eq!(
            enc.entities[&uid].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "ロローラ".to_string()],
            "rule1 must still perform its normal single-slot swap even when role_skill_imagines shares A's name"
        );
        assert!(enc.entities[&uid].pending_imagine.is_none());
        assert_eq!(
            enc.entities[&uid].role_skill_imagine_names(),
            vec!["ヴェノミーンの巣".to_string()],
            "role_skill_imagines must be untouched by rule1"
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
