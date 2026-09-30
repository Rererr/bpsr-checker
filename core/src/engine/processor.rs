use crate::capture::server::Server;
use crate::engine::class::{Class, ClassSpec, get_class_from_spec, get_class_spec_from_skill_id};
use crate::engine::combat_stats::{actual_value, process_stats};
use crate::engine::encounter::{Encounter, EncounterMutex, LevelMapChange};
use crate::engine::entity::{
    Entity, EntityKey, ImagineSlot, MAX_IMAGINE_NAMES, MAX_ROLE_SKILL_IMAGINES, SkillMeta,
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
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// バトルイマジン検知の鮮度比較用シーケンス番号。wall-clock ではなく単調増加カウンタにする
/// のは、同一ミリ秒に複数検知が起きるとテスト・実戦とも鮮度が潰れて順序判定できなくなるため。
static IMAGINE_DETECTION_SEQ: AtomicU64 = AtomicU64::new(0);

fn next_imagine_seq() -> u64 {
    IMAGINE_DETECTION_SEQ.fetch_add(1, Ordering::Relaxed)
}

/// SocialEnvelope の char_id 不一致（他キャラの通知が届いた）を、プロセス生存中1回だけ
/// info で記録済みかどうか。level_map_id 追跡は実機未確認の推定実装（pb.proto 参照）なので、
/// 実機で char_id の値そのものが観測できているかを切り分けるための最小限のログ。
/// 2回目以降は debug に留め、他キャラの通知が繰り返し届く場合でもログが連発しないようにする
/// （他キャラの通知が自クライアントに届くかは実機未確認）。
static SOCIAL_ENVELOPE_MISMATCH_LOGGED: AtomicBool = AtomicBool::new(false);

/// 0x17(SyncDungeonData) を「自キャラ確定済みで conn 未学習」を理由に破棄したことを、
/// プロセス生存中1回だけ info で記録済みかどうか（2回目以降は debug）。
static SYNC_DUNGEON_UNLEARNED_LOGGED: AtomicBool = AtomicBool::new(false);

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

/// 当プロセスの生存中にフル装備スキルリスト（attr116）で実イマジンを権威的に確定したことの
/// ある uid。`Entity::imagines_authoritative` の種。Encounter のリセット（`clear_combat_stats`）
/// でプレイヤー entity は破棄→name_cache から復元されるため、Entity のフラグだけでは
/// リセットごとに失われる。attr116 は他人なら AOI appear（ダンジョン読込/ボス部屋切替）でしか
/// 再送されないので、「このセッションで attr116 を見た」という事実をここで持ち越す。
/// 前セッションの name_cache（装備が変わっているかもしれない推定値）とはこれで区別できる。
static SKILL_LIST_CONFIRMED_UIDS: LazyLock<Mutex<HashSet<i64>>> =
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
fn get_or_create_entity(encounter: &mut Encounter, key: EntityKey) -> &mut Entity {
    let was_new = !encounter.entities.contains_key(&key);
    let entity = encounter.entities.entry(key).or_default();
    if was_new && key.is_player() {
        if let Some(cached) = name_cache::lookup(key.player_uid()) {
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
                // 当セッションで attr116 を見た uid のキャッシュは attr116 が書いた値なので権威的
                // （リセット跨ぎの持ち越し）。前セッション由来なら推定値のまま。
                entity.imagines_authoritative = SKILL_LIST_CONFIRMED_UIDS
                    .lock()
                    .is_ok_and(|set| set.contains(&key.player_uid()));
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
///
/// なお、確定スロットにある親イマジン（キングゴブリン等、`imagine_skills::NESTED_SUMMON_PARENTS`）
/// の召喚体が報告する子イマジン名は、そもそも pending に入れず「親の再検知」に読み替える（rule0）。
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
    let owner = get_or_create_entity(encounter, EntityKey::from_uuid(owner_uuid));
    let seq = next_imagine_seq();

    // 「確定スロットに一致するか」の判定はここ1箇所（rule0 の前提条件と rule1 の入口を兼ねる）。
    let mut matched_slot = owner.imagines.iter().position(|s| s.name == name);
    // ロールスキル枠に確定済みの名前（＝簡易版の発動エコー）は rule0 で親へ読み替えず、
    // 下のロールスキル吸収ブロックへ渡す（「role-skill 名は実イマジン枠へ混入させない」順序原則）。
    let is_role_skill_echo = owner.role_skill_imagines.iter().any(|s| s.name == name);

    // rule0: 入れ子召喚の吸収。キングゴブリン(3946)のように「他のバトルイマジンの召喚体」を呼ぶ
    // 奥義は自身の召喚報告IDを持たず、召喚体は子イマジン側の召喚スキル（例: 嵐の大斧ゴブリン=
    // 1008641 → 嵐のゴブリンウォーリアー）を AttrSkillId として報告する。中間 entity は無く
    // SummonerId/TopSummonerId ともプレイヤーなので attr では子イマジン装備者の召喚と区別できない。
    // 確定スロットに親がいて name がその子なら「親の再検知」に読み替える（pending/交換判定へ
    // 流さない＝親が子に追い出される誤表示を防ぐ。休眠イマジンだった親に現役の証拠も付く）。
    // 凸数は子召喚体の値なので親へは反映しない（0=未判明扱い）。name 自体が確定スロットに
    // ある場合（親子を両方装備）は通常の rule1 を優先する。
    // 「親は現役」を rule1 の交換証拠（pending の昇格）にまで使うのは、親スロットが当セッションの
    // attr116 で権威的に確定している場合（`imagines_authoritative`）だけ。前セッションの
    // name_cache だけで組まれた親（装備が変わっているかもしれない）では鮮度更新に留めて return
    // する＝子召喚を pending にも交換証拠にも使わない。こうすると、真の装備が [子, C] に
    // 変わっていた場合に C の pending が rule2 の自己修復で単独確定→親が外れ→以後の子召喚が
    // rule3 で正しく追加される（誤った確定を name_cache へ永続させない）。代償として、
    // attr116 未着のまま親装備者がダンジョン内で相方を付け替えた場合は次の attr116 まで
    // 追従しないが、他人の attr116 は appear（entity 生成時）に同梱されるので窓は極めて短い。
    let (name, tier) = if matched_slot.is_none() && !is_role_skill_echo {
        match owner
            .imagines
            .iter()
            .position(|s| crate::engine::imagine_skills::is_nested_summon_child(&s.name, &name))
        {
            Some(parent_idx) => {
                let parent = owner.imagines[parent_idx].name.clone();
                debug!(
                    "battle imagine nested summon absorbed: uid={owner_uid} {name} (summon skill {sk}) -> counted as {parent}"
                );
                if !owner.imagines_authoritative {
                    owner.imagines[parent_idx].last_seen = seq;
                    return;
                }
                matched_slot = Some(parent_idx);
                (parent, 0)
            }
            None => (name, tier),
        }
    } else {
        (name, tier)
    };

    // rule1: 既存の確定スロットと一致 → 再検知＝現役の証拠。並び順は変えず鮮度だけ更新する
    // （凸数はキャッシュ復元直後 0 の場合やレベル上げ後があるため、非0 が来たら追従する）。
    if let Some(idx) = matched_slot {
        let slot = &mut owner.imagines[idx];
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
    // （rule0 で親へ読み替えた場合は親が確定スロットにあるため rule1 で return 済み＝ここへは
    // 来ない。よって is_role_skill_echo は常に現在の name について評価した値になっている。）
    if is_role_skill_echo {
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
        // 空＝実イマジン未装備とは限らない（ImagineSkillNames 未収載の新イマジンは解決不能で
        // 空になる）ため、ここでは imagines_authoritative を立てず SKILL_LIST_CONFIRMED_UIDS
        // にも入れない（非空の場合は下の分岐でこれらを権威的に確定させている）。
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
    // フルリストに実イマジンが載っていた＝権威的な確定（内容が現状と同一でも、その事実は残す）。
    if !player_entity.imagines_authoritative {
        player_entity.imagines_authoritative = true;
        if let Ok(mut set) = SKILL_LIST_CONFIRMED_UIDS.lock() {
            set.insert(uid);
        }
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

/// `encounter.team` を更新し、メンバー構成 or team_id が変化したときだけ1行ログする
/// （NoticeUpdateTeamMemberInfo はダンジョン1周60回来る実測があり、毎通知ログすると
/// ログが連発するため）。
fn log_team_change(encounter: &mut Encounter, f: impl FnOnce(&mut crate::engine::team::TeamState)) {
    let before = encounter.team.clone();
    f(&mut encounter.team);
    if encounter.team.team_id != before.team_id || encounter.team.member_uids != before.member_uids
    {
        info!(
            "team: team_id={} leader={} members={:?}",
            encounter.team.team_id, encounter.team.leader_uid, encounter.team.member_uids
        );
    }
}

/// 0x2B(WorldSyncServerTime) / 0x17(WorldSyncDungeonData) / Team系5アーム / SocialEnvelope は should_accept を
/// 意図的に経由しないため、他クライアント由来のパケットを弾く判定をこの1箇所に集約する。
/// WorldSyncServerTime/Team系はキャラ選択と無関係にアプリ全体で使う値（is_paused でも
/// 止めない）。SocialEnvelope は自キャラのシーン追跡そのものなのでキャラ選択と関係するが、
/// conn_to_uid へ書き込まない（学習経路ではない）ため、conn 単体では自他を判定できず、
/// 呼び出し側で char_id による判定と組み合わせて絞り込む。
/// 0x17 は別の判定（`gate_sync_dungeon_data`）を使う。
/// conn が学習済み（`conn_to_uid` に載っている）で、その uid が追跡中キャラ
/// （`encounter.local_player_uid`）と異なるときだけ true。
/// 未学習 conn は常に false（通す）: 起動直後の精度と、Team 通知が別 TCP リンクで
/// 届く可能性を殺さないため。local_player_uid が未確定(0)のときも常に false
/// （まだ「追跡中キャラ」が無いので他クライアント判定のしようがない）。
fn conn_is_other_client(encounter: &Encounter, conn: Option<Server>) -> bool {
    if encounter.local_player_uid == 0 {
        return false;
    }
    let Some(conn) = conn else {
        return false;
    };
    match encounter.conn_to_uid.get(&conn) {
        Some(&uid) => uid != encounter.local_player_uid,
        None => false,
    }
}

/// 0x17(SyncDungeonData) の受理判定の結果。
#[derive(Debug, PartialEq, Eq)]
enum SyncDungeonGate {
    Accept,
    /// 学習済みで、自キャラ以外の conn。
    OtherClient,
    /// 自キャラ確定済みなのに、まだ学習していない conn。
    UnlearnedConn,
}

/// 0x17 は char_id を持たないので、conn だけで自他を判定する。自キャラの導出は
/// `self_player_uid()` の1か所に寄せる。受理するのは次のいずれか:
/// conn が None（取得元不明）／自キャラ未確定／conn が自キャラとして学習済み／一時停止中の未学習 conn
/// （学習パケットが停止中は処理されず、再開後も次のマップ移動まで段階が取れなくなるため）。
/// 学習済みの他キャラ conn は、停止中でも破棄する。
fn gate_sync_dungeon_data(encounter: &Encounter, conn: Option<Server>) -> SyncDungeonGate {
    let (Some(self_uid), Some(conn)) = (encounter.self_player_uid(), conn) else {
        return SyncDungeonGate::Accept;
    };
    match encounter.conn_to_uid.get(&conn) {
        Some(&uid) if uid == self_uid => SyncDungeonGate::Accept,
        Some(_) => SyncDungeonGate::OtherClient,
        None if encounter.is_paused => SyncDungeonGate::Accept,
        None => SyncDungeonGate::UnlearnedConn,
    }
}

/// 自キャラの level_map_id を更新し、変わったときだけシーン変更をログに残す
/// （SocialEnvelope と WorldEnterSnapshot の両経路がここを通る）。
fn apply_level_map_id(encounter: &mut Encounter, level_map_id: u32, source: &str) {
    let previous = encounter.current_level_map_id;
    let change = encounter.set_current_level_map_id(level_map_id);
    let stage = match change {
        LevelMapChange::Unchanged => return,
        LevelMapChange::StageKept => "kept",
        LevelMapChange::StageReset => "reset",
    };
    info!(
        "[{source}] scene changed: level_map_id {previous} -> {level_map_id} stage {stage} (now {})",
        encounter.current_dungeon_difficulty
    );
}

fn should_accept(encounter: &mut Encounter, conn: Option<Server>, op: &Pkt) -> bool {
    // ServerHandover と SocialEnvelope と WorldSyncDungeonData は process_opcode の専用アームで
    // 処理されるため、ここには到達しない（SocialEnvelope は conn_is_other_client と char_id 判定を
    // 自前で行う。ここを経由させると、自キャラ確定後の ServerHandover 直後に未学習 conn へ届く
    // 継続通知が落ちる。WorldSyncDungeonData は専用アームの `gate_sync_dungeon_data` で判定する。
    // 計測中（非停止）の1クライアントの実機ログでは学習済みの自キャラ conn から届いた）。
    // WorldEnterSnapshot/WorldEnterScene/LocalDeltaBatch は conn ↔ char_id の学習経路なので、
    // 身元不明の conn でも必ず通す（ここで弾くと学習が永久に起きず、UID 指定時に何も
    // 表示されなくなる）。他クライアント由来かどうかは各 process_* が learn_connection で
    // 判定し破棄する。LocalDeltaBatch は入場時以外にも継続的に届くため、戦闘途中でアプリを
    // 起動しても短時間で対象クライアントを特定できる主経路になる。
    if matches!(
        op,
        Pkt::ServerHandover
            | Pkt::SocialEnvelope
            | Pkt::WorldSyncDungeonData
            | Pkt::WorldEnterSnapshot
            | Pkt::WorldEnterScene
            | Pkt::LocalDeltaBatch
    ) {
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
            None => {
                // 先着 char_id に属する全コネクションを通す（1クライアント複数接続のため）
                if encounter.local_player_uid == 0 {
                    encounter.set_local_player_uid(uid_for_conn);
                }
                if uid_for_conn == encounter.local_player_uid {
                    encounter.active_connection = Some(conn);
                    true
                } else {
                    false
                }
            }
        };
    }

    // 未学習 conn: UID 指定時は「身元不明＝通さない」。複数クライアント起動中に
    // 他キャラのデータが混入するのを防ぐ。WorldEnterSnapshot は上で常時通すので、
    // 対象クライアントの入場/ゾーン移動を検出した時点で学習が進み accept に転じる。
    if sel.is_some() {
        return false;
    }

    // 対象キャラが確定済みなら身元不明の conn は通さない。
    // （active_connection は同一キャラの複数接続間で移り変わるため比較対象にしない）
    if encounter.local_player_uid != 0 {
        return false;
    }

    // 完全未確定 (起動直後・UID 未指定): 全 accept。学習経路の受信で確定する
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
            let Some(body) = notify.v_request.and_then(|r| r.data) else {
                return Ok(());
            };

            let mut encounter = enc.lock().map_err(|e| AppError::LockPoisoned(e.to_string()))?;

            // 他クライアントの conn 由来なら破棄（conn が未学習/自キャラ未確定なら通す）。
            if conn_is_other_client(&encounter, conn) {
                debug!(
                    "[SocialEnvelope] discarded: other client conn char_id={}",
                    body.char_id
                );
                return Ok(());
            }

            // char_id が判明していて自キャラも確定済みなら、一致する時だけ受理する
            // （自キャラ未確定時は暫定受理: SocialEnvelope はシーン遷移要求時にしか届かない
            // ため、ここで捨てると次の遷移が起きるまで level_map_id が不明なままになる）。
            if body.char_id != 0 {
                if let Some(self_uid) = encounter.self_player_uid() {
                    if body.char_id != self_uid {
                        if SOCIAL_ENVELOPE_MISMATCH_LOGGED.swap(true, Ordering::Relaxed) {
                            debug!(
                                "[SocialEnvelope] discarded: char_id={} self_uid={}",
                                body.char_id, self_uid
                            );
                        } else {
                            info!(
                                "[SocialEnvelope] discarded (first occurrence, logged once): char_id={} self_uid={}",
                                body.char_id, self_uid
                            );
                        }
                        return Ok(());
                    }
                }
            }

            // char_id はログに残す（自キャラ未確定時の暫定受理・char_id=0 の経路では、
            // どのキャラのシーンを取り込んだかを示す唯一の記録になるため）。
            let source = format!("SocialEnvelope char_id={}", body.char_id);
            if let Some(scene) = body.scene_data {
                apply_level_map_id(&mut encounter, scene.level_map_id, &source);
            }
        }

        // WorldNtf method 0x17: ダンジョン入場時のマスター難易度の段階。should_accept を経由しない。
        // SyncDungeonData に char_id は無く conn だけが自他の判定材料になる。2026-09-30 に観測した遷移
        // （町・フィールド・蝕ティナのノーマル/ハード/マスター）では届き、町・フィールド・ノーマル/ハードは
        // difficulty=0 だった。到着順は SocialEnvelope の後で、計測中（非停止）の1クライアントの実機ログでは
        // 学習済みの自キャラ conn から届いた。判定は `gate_sync_dungeon_data`（受理条件はそちら）。
        Pkt::WorldSyncDungeonData => {
            let Some(msg) = decode_packet::<pb::SyncDungeonData>(data, "SyncDungeonData") else {
                return Ok(());
            };
            let mut encounter = enc.lock().map_err(|e| AppError::LockPoisoned(e.to_string()))?;
            let conn_state = match conn {
                None => "conn=none".to_string(),
                Some(c) => match encounter.conn_to_uid.get(&c) {
                    Some(uid) => format!("conn={c} learned uid={uid}"),
                    None => format!("conn={c} unlearned"),
                },
            };
            match gate_sync_dungeon_data(&encounter, conn) {
                SyncDungeonGate::Accept => {}
                SyncDungeonGate::OtherClient => {
                    debug!("[SyncDungeonData] discarded: other client {conn_state}");
                    return Ok(());
                }
                SyncDungeonGate::UnlearnedConn => {
                    if SYNC_DUNGEON_UNLEARNED_LOGGED.swap(true, Ordering::Relaxed) {
                        debug!("[SyncDungeonData] discarded: unlearned conn {conn_state}");
                    } else {
                        info!(
                            "[SyncDungeonData] discarded (first occurrence, logged once): unlearned conn {conn_state}"
                        );
                    }
                    return Ok(());
                }
            }
            match msg.v_data.and_then(|d| d.dungeon_scene_info) {
                Some(info) => {
                    encounter.current_dungeon_difficulty =
                        crate::engine::content_names::master_stage(info.difficulty);
                    info!(
                        "[SyncDungeonData] scene_info=present difficulty={} -> stage={} current_level_map_id={} {conn_state}",
                        info.difficulty,
                        encounter.current_dungeon_difficulty,
                        encounter.current_level_map_id
                    );
                }
                None => {
                    info!(
                        "[SyncDungeonData] scene_info=absent (kept stage={}) {conn_state}",
                        encounter.current_dungeon_difficulty
                    );
                }
            }
        }

        // WorldNtf method 0x2B: ゲームクライアント（＝当アプリと同一PC）とサーバの
        // 正確な時刻同期。should_accept（他クライアント/未識別 conn のフィルタ）を
        // 意図的に経由しない: この値はキャラクター選択と無関係にアプリ全体の
        // buff_tracker オフセット推定へ効くべきで、conn が学習されるまで捨てられると
        // 起動直後の精度が落ちる。is_paused でも止めない（一時停止中もバフタイマーの
        // 精度は保ちたい）。imagine_only_mode の対象は process_world_entity_batch 側の
        // エンティティ集計であり、ここには関係しない。
        Pkt::WorldSyncServerTime => {
            let Some(msg) = decode_packet::<pb::SyncServerTime>(data, "SyncServerTime") else {
                return Ok(());
            };
            if crate::probe::enabled() {
                log::info!(
                    "PROBE sync-server-time client={} server={} delta={}",
                    msg.client_milliseconds,
                    msg.server_milliseconds,
                    msg.client_milliseconds - msg.server_milliseconds
                );
            }
            if msg.client_milliseconds > 0 && msg.server_milliseconds > 0 {
                let mut encounter = enc
                    .lock()
                    .map_err(|e| AppError::LockPoisoned(e.to_string()))?;
                if conn_is_other_client(&encounter, conn) {
                    return Ok(());
                }
                encounter.buff_tracker.observe_server_time_sync(
                    msg.client_milliseconds,
                    msg.server_milliseconds,
                    now_ms(),
                );
            }
        }

        // TeamNtf(service 0x399fca69): パーティ(PT)構成通知。0x2B と同じ理由で should_accept
        // を意図的に経由しない: PT構成はキャラ選択と無関係にアプリ全体で使う（compute.rs の
        // 「PTメンバーのみ食事/シロップ行を表示」フィルタの入力になる）。is_paused でも止めない
        // （一時停止中もPT構成の追従は止めたくない）。
        Pkt::TeamUpdateInfo => {
            let Some(msg) = decode_packet::<pb::NoticeUpdateTeamInfo>(data, "TeamUpdateInfo")
            else {
                return Ok(());
            };
            let Some(base) = msg.v_request.and_then(|r| r.base_info) else {
                return Ok(());
            };
            let mut encounter = enc.lock().map_err(|e| AppError::LockPoisoned(e.to_string()))?;
            if conn_is_other_client(&encounter, conn) {
                return Ok(());
            }
            log_team_change(&mut encounter, |t| t.update_info(base.team_id, base.leader_id));
        }

        // ダンジョン1周60回来る実測があるため、ログは log_team_change 内で変化時のみに絞る。
        Pkt::TeamUpdateMemberInfo => {
            let Some(msg) =
                decode_packet::<pb::NoticeUpdateTeamMemberInfo>(data, "TeamUpdateMemberInfo")
            else {
                return Ok(());
            };
            let Some(req) = msg.v_request else {
                return Ok(());
            };
            let uids: Vec<i64> = req
                .team_member_sync_datas
                .iter()
                .map(|d| d.char_id)
                .chain(req.team_member_social_datas.iter().map(|d| d.char_id))
                .collect();
            let mut encounter = enc.lock().map_err(|e| AppError::LockPoisoned(e.to_string()))?;
            if conn_is_other_client(&encounter, conn) {
                return Ok(());
            }
            log_team_change(&mut encounter, |t| t.update_members(uids));
        }

        Pkt::TeamJoin => {
            let Some(msg) = decode_packet::<pb::NotifyJoinTeam>(data, "TeamJoin") else {
                return Ok(());
            };
            let Some(req) = msg.v_request else {
                return Ok(());
            };
            let team_id = req.base_info.as_ref().map(|b| b.team_id).unwrap_or(0);
            let leader_uid = req.base_info.as_ref().map(|b| b.leader_id).unwrap_or(0);
            let uids: Vec<i64> = req
                .member_data
                .iter()
                .map(|d| d.char_id)
                .chain(req.member_sync_datas.keys().copied())
                .collect();
            let mut encounter = enc.lock().map_err(|e| AppError::LockPoisoned(e.to_string()))?;
            if conn_is_other_client(&encounter, conn) {
                return Ok(());
            }
            log_team_change(&mut encounter, |t| t.join(team_id, leader_uid, uids));
        }

        Pkt::TeamLeave => {
            let Some(msg) = decode_packet::<pb::NotifyLeaveTeam>(data, "TeamLeave") else {
                return Ok(());
            };
            let Some(req) = msg.v_request else {
                return Ok(());
            };
            let mut encounter = enc.lock().map_err(|e| AppError::LockPoisoned(e.to_string()))?;
            if conn_is_other_client(&encounter, conn) {
                return Ok(());
            }
            let local_uid = encounter.local_player_uid;
            log_team_change(&mut encounter, |t| t.leave(req.char_id, local_uid));
        }

        // 空メッセージ（NoticeTeamDissolveRequest）のため decode せず即クリアする。
        Pkt::TeamDissolve => {
            let mut encounter = enc.lock().map_err(|e| AppError::LockPoisoned(e.to_string()))?;
            if conn_is_other_client(&encounter, conn) {
                return Ok(());
            }
            log_team_change(&mut encounter, |t| t.dissolve());
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
                    process_enter_scene(&mut encounter, msg, conn);
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
                    process_local_delta_batch(&mut encounter, msg, conn);
                }

                Pkt::WorldDeltaBatch => {
                    let Some(msg) = decode_packet::<pb::WorldDeltaBatch>(data, "WorldDeltaBatch")
                    else {
                        return Ok(());
                    };
                    // M14計測: 範囲攻撃が対象ごとに別 SceneDelta で届くかを、同一バッチ内の
                    // distinct 対象数で確かめる（1つの SceneDelta は uuid ひとつしか持てない）。
                    // 消費前に形だけ数える。
                    crate::probe::record_delta_batch(&msg.delta_infos);
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

    let ts = now_ms();
    for pkt_entity in msg.appear {
        let target_uuid = pkt_entity.uuid;
        if target_uuid == 0 {
            continue;
        }
        let target_key = EntityKey::from_uuid(target_uuid);
        let target_uid = target_key.player_uid();
        let target_entity_type = target_key.kind();

        // 種別はキーに含まれるので、ここで Entity 側の種別を上書きする必要はない
        // （旧実装は `uuid >> 16` をキーにしていたため、同番号の非プレイヤーが appear すると
        //  PT メンバーの Entity 種別を Monster/Unknown へ書き換えて一覧から消していた）。
        let target_entity = get_or_create_entity(encounter, target_key);

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
        // target_entity の借用はここで終わる（以降未使用）。

        // AOI 出現時点で既に付与されている既存バフ一覧（食事/シロップ等）を同期する。
        // これが無いと、ダンジョンで先に食事していた他プレイヤーの食事/シロップを
        // 知る経路が無く、appear 時点でバッジが出ない（field 7 の到来自体は
        // 2026-08時点でグローバル版 probe 未確認。apply_appear_buff_sync のログで検証する）。
        if target_entity_type == EntityKind::Player {
            if let Some(bundle) = &pkt_entity.buff_infos {
                apply_appear_buff_sync(encounter, target_uid, bundle, ts);
            }
        }
    }
}

/// appear（AOI 出現）/ EnterScene（自キャラ入場）が運ぶ既存バフ一覧
/// （pb::EntityAppear.buff_infos、参照実装の BuffInfoSync 相当）を buff_tracker へ
/// 投入する。scene-add 経路（SceneDelta.buff_list の AddBuff＝平時の主経路）とは
/// 別の到達点だが、apply_buff_add は同一 buff_uuid を上書きするだけなので、
/// 同じバフが両経路から届いても二重計上にはならない。
fn apply_appear_buff_sync(encounter: &mut Encounter, target_uid: i64, bundle: &pb::BuffSnapshotBundle, ts: u128) {
    if bundle.buff_infos.is_empty() {
        return;
    }
    let mut consumables = 0;
    for b in &bundle.buff_infos {
        if crate::probe::enabled() {
            crate::probe::log_buff_snapshot("appear-sync", &b.encode_to_vec(), b);
        }
        if crate::engine::consumables::is_consumable(b.base_id) {
            consumables += 1;
        }
        encounter.buff_tracker.apply_buff_add(b.buff_uuid, b, ts, target_uid);
    }
    if crate::probe::enabled() {
        info!(
            "appear buff sync: uid={target_uid} n={} consumables={consumables}",
            bundle.buff_infos.len()
        );
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
            encounter.set_local_player_uid(player_uid);
        }
        Some(sel_uid) if sel_uid == player_uid => {
            // UID 一致: この connection を active に
            encounter.active_connection = Some(conn);
            encounter.set_local_player_uid(player_uid);
        }
        _ => {
            // 他クライアント由来: エンティティ作成・name_cache 更新をスキップ
            return;
        }
    }

    // ここまで到達すれば自キャラ確定済み（上の match で他クライアントは return 済み）なので、
    // conn/char_id によるフィルタは不要。
    if let Some(scene) = &v_data.scene_data {
        apply_level_map_id(encounter, scene.level_map_id, "WorldEnterSnapshot");
    }

    let target_entity = get_or_create_entity(encounter, EntityKey::player(player_uid));

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

/// conn ↔ char_id を学習し、この conn が対象クライアントかを返す。
/// false のとき呼び出し側は以降の処理を破棄する（他クライアント由来）。
///
/// 複数のゲームクライアントを同時起動していると、観測されるコネクションはキャラ数だけ
/// 存在する。UID 指定時は一致した conn だけを active にし、未指定時は先着に固定する。
fn learn_connection(encounter: &mut Encounter, conn: Server, player_uid: i64) -> bool {
    // 学習は毎パケット走るため、ログは「新規に判明した」瞬間だけに絞る
    let newly_learned = encounter.conn_to_uid.insert(conn, player_uid) != Some(player_uid);
    let sel = selected_uid::get();
    // 1つのゲームクライアントは複数の TCP コネクションを張る（scene 用の補助接続。
    // 実測: 同一 char_id が :10541 と :10408 の2本で学習された）。したがって判定は
    // conn 単位ではなく char_id 単位で行う。conn 単位にすると後着の接続が active を
    // 奪い、先の接続のデータが落ちる。
    let is_target = match sel {
        Some(sel_uid) => sel_uid == player_uid,
        // 自動検出: 先着の char_id に固定し、そのキャラの全コネクションを対象とする
        None => encounter.local_player_uid == 0 || encounter.local_player_uid == player_uid,
    };
    if newly_learned {
        info!(
            "[conn] learned char_id={player_uid} on {conn} (selected={sel:?}, target={is_target})"
        );
    }
    if !is_target {
        return false;
    }
    if encounter.active_connection != Some(conn) {
        info!("[conn] active connection -> {conn} (char_id={player_uid})");
    }
    encounter.active_connection = Some(conn);
    if encounter.local_player_uid == 0 {
        encounter.set_local_player_uid(player_uid);
    }
    true
}

fn process_local_delta_batch(
    encounter: &mut Encounter,
    msg: pb::LocalDeltaBatch,
    conn: Option<Server>,
) {
    let Some(delta_info) = msg.delta_info else {
        return;
    };

    // LocalDeltaBatch は自プレイヤー専用の差分。LocalSceneDelta.uuid(field 5) が自キャラ
    // なので、conn ↔ char_id の主要な学習経路になる。EnterScene / WorldEnterSnapshot が
    // 入場時にしか届かないのに対し、これは戦闘中・非戦闘中を問わず継続的に流れるため、
    // 戦闘途中でアプリを起動しても対象クライアントを短時間で特定できる。
    let self_uuid = if delta_info.uuid != 0 {
        delta_info.uuid
    } else {
        delta_info.base_delta.as_ref().map_or(0, |d| d.uuid)
    };
    if let (Some(conn), true) = (conn, self_uuid != 0) {
        let self_uid = entity::get_player_uid(self_uuid);
        if self_uid != 0 && !learn_connection(encounter, conn, self_uid) {
            return; // 他クライアント由来
        }
    }

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

/// ダメージレコードの攻撃者 UUID。召喚体のダメージは `top_summoner_id` で主人へ寄せる。
/// 0 は攻撃者不明（集計から落とすレコード）。
///
/// 3分計測の起点判定（デルタに自分の行動が含まれるか）と集計ループの両方がこの規則を使うため、
/// 判定を2箇所に書かずここへ集約する。
#[inline]
fn damage_attacker_uuid(damage: &pb::DamageRecord) -> i64 {
    if damage.top_summoner_id != 0 {
        damage.top_summoner_id
    } else {
        damage.attacker_uuid
    }
}

pub(crate) fn process_scene_delta(encounter: &mut Encounter, scene_delta: pb::SceneDelta) {
    let target_uuid = scene_delta.uuid;
    if target_uuid == 0 {
        return;
    }
    let target_key = EntityKey::from_uuid(target_uuid);
    let target_uid = target_key.player_uid();
    let target_entity_type = target_key.kind();
    let imagine_only = crate::engine::runtime_settings::imagine_only_mode();

    // Process attributes on the target entity（軽量モードではスキップ）
    if !imagine_only {
        let target_entity = get_or_create_entity(encounter, target_key);

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
    // M6計測用: このデルタが damages を含んでいたか（下の for ループが Vec を消費する前に控える）。
    let had_damages = !skill_effect.damages.is_empty();

    // 自キャラの識別。導出規則は Encounter::self_player_uid が唯一の定義で、表示側の射影
    // （compute::self_only_uid）と共有する。selected は「手動指定があるか」を区別する必要が
    // あるため別に読む（デルタ内で値は変わらないので1回だけ）。
    let selected = selected_uid::get();
    let self_key = encounter.self_player_key();

    // 計測スコープ（計測ボタンで始めた計測の絞り込み条件。通常モードは既定値＝絞り込み無し）。
    let scope = encounter.measure_scope();

    // このデルタが自分の戦闘か（自分が出したか、自分が受けたか）。
    // 3分計測の起点判定と M9計測の両方がこの1つの値から導かれる。
    //
    // 「自分が受けた」を含めるのは、被ダメージを測るために計測ボタンを押す使い方があるため。
    // 攻撃者側だけを見ると、一度も攻撃しないプレイヤーの計測が永久に始まらず、待っているあいだの
    // 被弾も丸ごと落ちる。実測で問題になったのは他プレイヤーどうしの戦闘で窓が開く経路であって、
    // 自分の被弾ではない（2026-08-26 の記録4件はいずれも自分が当事者でないデルタだった）。
    //
    // 待機中と戦闘時計の起点でしか使わない値なので、それ以外では走査を省く
    // （ダメージレコードの全走査は戦闘中に毎秒数千回走る）。
    let needs_self_action = encounter.time_fight_start_ms == 0
        || matches!(
            encounter.measure_mode,
            crate::engine::encounter::MeasureMode::Pending3Min { .. }
        );
    let delta_has_self_action = needs_self_action
        && self_key.is_some_and(|key| {
            key == target_key
                || skill_effect
                    .damages
                    .iter()
                    .any(|damage| EntityKey::from_uuid(damage_attacker_uuid(damage)) == key)
        });

    // 3分計測の待機中（Pending3Min）は、自分の行動が届くまで集計もタイムスタンプ更新も行わない。
    // 計測窓の起点を自分の初撃へ固定するため。以前は skill_effects を持つ任意のデルタで
    // Pending→Active が発火しており、他プレイヤーの与ダメージや自分の被弾で窓が回り始めていた
    // （2026-08-26 の実測では計測ボタンを押した4回すべてがこれに該当した）。
    //
    // 自キャラが未確定（self_key が None）のときは従来どおり最初のダメージで開始する。
    // ここで待ち続けると計測ボタンが何も起きないまま無反応になるため。
    if self_key.is_some()
        && !delta_has_self_action
        && matches!(
            encounter.measure_mode,
            crate::engine::encounter::MeasureMode::Pending3Min { .. }
        )
    {
        return;
    }

    if had_damages {
        let ts = now_ms();
        // 「戦闘中」判定は Encounter::is_combat_active に集約されている（同じ述語を
        // ここと別の場所に重複して書かない）。ロールオーバーは Normal モード限定
        // ＋既に一度は戦闘していた（time_last_combat_packet_ms != 0）ことが前提。
        // 閾値は旧実装の `diff > timeout_ms` と同じ境界（is_combat_active は `<=` で判定）。
        if matches!(
            encounter.measure_mode,
            crate::engine::encounter::MeasureMode::Normal
        ) && encounter.time_last_combat_packet_ms != 0
            && !encounter.is_combat_active(ts)
        {
            let snapshot = crate::compute::build_encounter_snapshot(encounter, ts);
            let selected = selected_uid::get();
            let should_push = !snapshot.player_rows.is_empty()
                && selected.map_or(true, |_| encounter.has_selected_participant);
            if should_push {
                crate::engine::history::push(snapshot);
            }
            // M2/M3/M5計測: 直前エンカウンターぶんのサマリーをログしカウンタをリセットする。
            crate::probe::log_and_reset_encounter_summary();
            // v0.8.3 以前と同様 clear 後もフォールスルーして当該フレームのダメージを集計する。
            // emit("encounter-reset") は廃止。フロントは次のポーリングで自然に更新される。
            encounter.clear_combat_stats();
        }
    }

    // 有効DPS（実働時間ベース）の分母に使う観測時刻。バッチ内の全ダメージイベントに共通の
    // 値を使う（time_last_combat_packet_ms と同じ「パケット単位」の粒度。1回だけ取得しループ内で
    // 使い回す＝O(1)）。通常DPSの分母（combat_elapsed_ms）には一切使わない、有効DPS専用の値。
    let active_ts = now_ms();

    // M9計測用（下の for ループが Vec を消費する前に控える）。
    let probe_damages_n = skill_effect.damages.len();

    // Process each damage event
    for damage in skill_effect.damages {
        // M5計測: value と lucky_value が両方非ゼロで同時出現するレコードの実態を調べる
        // （combat_stats.rs は lucky_value 優先で採用するため、両立時の基礎ダメージが
        // 捨てられていないか確認する）。
        if damage.value != 0 && damage.lucky_value != 0 {
            crate::probe::log_lucky_collision(damage.value, damage.lucky_value, damage.hp_lessen_value);
        }

        // target の monster_id は is_boss 判定と M8計測（対象別内訳）の両方で使う。
        // 同じ対象を判定する式を2つ書かないよう、1つの変数から導出する。
        let target_monster_id = encounter.entities.get(&target_key).and_then(|e| e.monster_id);
        let is_boss = target_monster_id.is_some_and(|id| MONSTER_NAMES_BOSS.contains_key(&id));

        // M13計測: is_dead はエンジンが一度も読んでいないフィールドで、実機で立つのか、
        // 継続ダメージのティックごとに多重に立つのかが未検証。attacker/skill が確定する前の
        // 生の値で記録する（この下の continue で捨てるレコードにも立ちうるため）。
        if damage.is_dead {
            crate::probe::record_is_dead(
                target_uuid,
                target_monster_id,
                damage.attacker_uuid,
                damage.owner_id,
                actual_value(&damage),
                damage.r#type == pb::DmgKind::Heal as i32,
            );
        }

        let attacker_uuid = damage_attacker_uuid(&damage);
        if attacker_uuid == 0 {
            // M2計測: attacker不明で捨てるレコード（DoT・バフ由来・設置物ダメージ等の
            // 疑いがある）を件数・実効値合計で計上する。実効値は combat_stats::actual_value
            // と同じ「lucky_value優先」（ラッキーヒットは value==0 で来る想定のため、生value
            // だと欠損量を過小評価してしまう）。
            crate::probe::record_skip_no_attacker(actual_value(&damage));
            continue; // no attacker — skip
        }
        let attacker_key = EntityKey::from_uuid(attacker_uuid);
        let attacker_uid = attacker_key.player_uid();
        let attacker_entity_type = attacker_key.kind();

        let skill_uid = damage.owner_id;
        if skill_uid == 0 {
            // M2計測: skill_uid(owner_id)不明で捨てるレコードを件数・実効値合計で計上する。
            crate::probe::record_skip_no_skill(actual_value(&damage));
            continue;
        }

        // selected_uid 参加判定（`selected` はデルタ先頭で1回だけ読んだ値）
        if let Some(sel) = selected {
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

        // M3計測: attacker が Player 以外に積まれたダメージを件数・実効値合計で計上する。
        // 内訳には「召喚の帰属漏れ（自分の火力が一覧から落ちる）」と「モンスターの与ダメージ
        // （自分とは無関係）」が混在するため、合計だけで結論を出さないこと。詳細は probe 側の doc。
        // 下の総ダメージ集計は attacker が Monster のときだけ除外するため、召喚（Unknown
        // attacker）が敵を殴った分は引き続き encounter.dmg_stats に残る。この計数自体は
        // encounter.dmg_stats と足し合わせる前提ではない（record_non_player_attacker の doc 参照）。
        if !is_heal && attacker_entity_type != EntityKind::Player {
            crate::probe::record_non_player_attacker(
                actual_value(&damage),
                attacker_uuid,
                damage.top_summoner_id,
            );
        }

        // M7計測: 召喚体が出し、top_summoner_id で主人へ寄せ**られた**ダメージをスキル別に記録する。
        // 上の record_non_player_attacker は逆に「寄せられなかった」ものを数える別観点なので、
        // 条件を共有させず独立に判定する（両者は排他）。ゲーム内の木人計測パネルが召喚体の
        // ダメージを数えていない疑いがあり、その差分を実測で確定させるための計測。
        if !is_heal && damage.top_summoner_id != 0 {
            crate::probe::record_summon_damage(skill_uid, actual_value(&damage));
        }

        // モンスターが出したダメージ/回復（＝自分たちの成果ではないもの）を総計から除外する
        // ための判定。dmg と heal で同じ述語（attacker が Monster かどうか）を共有する。
        // 以前は dmg=target基準（対象がPlayerか）・heal=attacker基準、と2通りに分かれており、
        // 「モンスター同士のダメージは total に残るのにモンスター同士の回復は落ちる」という
        // 非対称と、行(attacker_entity側は無条件加算)と分母(target基準)の不一致があった
        // （target=Player以外の非Healレコードが行にだけ乗り、シェア%が100%を超えうる不具合）。
        // top_summoner_id が付かない召喚（attacker_entity_type==Unknown）は Monster ではない
        // ためこの条件を通過し、引き続き自分の火力/回復として総計に残る（帰属漏れの分を落とさない）。
        let from_monster = attacker_entity_type == EntityKind::Monster;

        // 初撃対象ロック（MeasureScope::first_target_only）。ロックの確立も適用も、同じ
        // target_key からこの1箇所で導く。
        //
        // ロックを張るのは自分の与ダメージだけに限る。本アプリは AOI 全体を観測しているため、
        // 街や訓練場では他プレイヤーが別の敵を殴ったデルタが先に届くのが普通で、全体基準だと
        // 無関係な敵を掴んで以後の自分の火力が丸ごと落ちる。
        //
        // 対象は「プレイヤー以外」。木人は実測で Monster(種別コード64)だったが、EntityKind は
        // 64/640 以外をすべて Unknown に落とすため、Monster 限定にすると Unknown で来る対象で
        // 機能がエラーも出さずに無効化される。
        //
        // ロックは計測が終わるまで解除しない（clear_combat_stats が唯一の出口）。DamageRecord の
        // is_dead で解除する案は実測で棄却した。同じ対象へ別スキル・別攻撃者から繰り返し立つ
        // ＝「この一撃で死んだ」ではなく「対象が死亡状態」を意味するため、乗り換えが暴発する。
        //
        // 自キャラが未確定（self_key が None）のあいだは絞らない。ロックを張れないまま
        // 「ロック対象と一致するか」で判定すると全ダメージが落ちて計測が丸ごと0になる。
        // 待機解除（processor.rs 冒頭）と表示側の射影（compute::self_only_uid）も同じ場面で
        // 絞らない側へ倒しており、3箇所で方針を揃える。
        // ロックを張る候補はプレイヤー以外に限る（自分や味方を掴まないため）。一方で
        // 「ロック対象への与ダメージだけ通す」判定は非Healレコード全体に効かせる。対象が
        // プレイヤーの非Healレコード（反射・PvP等）を素通しにすると、ロック対象と別の相手への
        // ダメージが総計へ残る。被ダメ集計は admit_target を通らないので影響を受けない。
        let lock_candidate = !is_heal && !target_key.is_player();
        let admit_target = if let (true, false, Some(key)) =
            (scope.first_target_only, is_heal, self_key)
        {
            if lock_candidate && attacker_key == key && encounter.locked_target.is_none() {
                encounter.locked_target = Some(target_key);
                info!("first-target lock: target_uuid={target_uuid}");
            }
            // ロックが確定するまでは何も通さない。「最初に自分が攻撃した対象のみ」を素直に読めば、
            // 自分が殴る前に届いた他プレイヤーの与ダメージは計測に入らない。
            // 通す側に倒すと、確定までの短い窓に流れ込んだ無関係な火力が総計へ残る。
            encounter.locked_target == Some(target_key)
        } else {
            true
        };

        // M8/M10/M11計測: ゲートより**前**の母集団を記録する。ゲートが何を落としているかを
        // 測るのが目的なので、通した後を数えても答えにならない（distinct が構造上1になる）。
        // 条件は encounter.dmg_stats の加算から admit_target だけを外したもの。
        if !is_heal && !from_monster {
            crate::probe::record_damage_scope(
                target_uuid,
                target_monster_id,
                attacker_uuid,
                self_key.map_or(0, |k| k.player_uid()),
                actual_value(&damage),
            );
        }

        // Encounter-level totals first (avoids holding attacker_entity borrow across encounter.* mutations)
        if is_heal {
            if !from_monster {
                process_stats(&damage, &mut encounter.heal_stats);
            }
        } else if !from_monster && admit_target {
            process_stats(&damage, &mut encounter.dmg_stats);
            if is_boss {
                process_stats(&damage, &mut encounter.dmg_stats_boss_only);
            }
        }

        // Target-side damage-taken aggregation (player targets only)。
        // 「誰が受けたか」を見る別の述語（above の from_monster とは意味が異なるため統合しない）。
        let damage_hits_player = target_entity_type == EntityKind::Player;
        if !is_heal && damage_hits_player {
            process_stats(&damage, &mut encounter.dmg_taken_stats);
            let target_entity = get_or_create_entity(encounter, target_key);
            process_stats(&damage, &mut target_entity.dmg_taken_stats);
            target_entity.skill_meta.entry(skill_uid).or_insert(SkillMeta {
                property: damage.property as u8,
                damage_mode: damage.damage_mode as u8,
            });
            let by_attacker = target_entity
                .attacker_uid_to_dmg_taken_stats
                .entry(attacker_key)
                .or_default();
            process_stats(&damage, by_attacker);
            let by_attacker_skill = target_entity
                .attacker_skill_to_dmg_taken_stats
                .entry((attacker_key, skill_uid))
                .or_default();
            process_stats(&damage, by_attacker_skill);
        }

        let attacker_entity = get_or_create_entity(encounter, attacker_key);

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
        } else if admit_target {
            // ゲートする書き込み先は encounter.dmg_stats を守っている条件集合と完全に一致させる。
            // active_dmg_time は有効DPSの分母なので、ここを素通しにすると総ダメージ列と
            // 有効DPS列が矛盾する（ロック対象外を殴っている時間だけ分母が伸びる）。
            let dps_skill = attacker_entity
                .skill_uid_to_dps_stats
                .entry(skill_uid)
                .or_default();
            process_stats(&damage, dps_skill);
            process_stats(&damage, &mut attacker_entity.dmg_stats);
            attacker_entity.active_dmg_time.record_event(active_ts);
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
        encounter.fight_level_map_id = encounter.current_level_map_id;
        encounter.fight_dungeon_difficulty = encounter.current_dungeon_difficulty;
        info!(
            "[fight start] level_map_id={} dungeon_difficulty stage={}",
            encounter.fight_level_map_id, encounter.fight_dungeon_difficulty
        );
        // M6/M9計測: 戦闘時計の起点となったデルタが damages を含んでいたか（false なら
        // 自己バフ・詠唱等で分母が実ダメージ開始より早く進み始めている）と、そのダメージが
        // 自分のものだったか（false なら他人の与ダメージや自分の被弾で計測窓が回り始めている）。
        // 3分計測の Pending→Active 遷移も直後の同じブロックで起きるため、計測ボタンの起点は
        // この1行で決まる。
        crate::probe::log_fight_start(
            had_damages,
            delta_has_self_action,
            probe_damages_n,
            target_uuid,
            encounter.entities.get(&target_key).and_then(|e| e.monster_id),
            self_key.map_or(0, |k| k.player_uid()),
            matches!(
                encounter.measure_mode,
                crate::engine::encounter::MeasureMode::Pending3Min { .. }
            ),
        );
        if let crate::engine::encounter::MeasureMode::Pending3Min { duration_ms, scope } =
            encounter.measure_mode
        {
            encounter.measure_mode = crate::engine::encounter::MeasureMode::Active3Min {
                armed_at_ms: ts,
                duration_ms,
                scope,
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

    // Per-entity sampling (only for entities with activity on at least one of the 3 metrics)
    for (key, entity) in encounter.entities.iter_mut() {
        if !key.is_player() {
            continue;
        }
        let dmg_total = entity.dmg_stats.total;
        let heal_total = entity.heal_stats.total;
        let taken_total = entity.dmg_taken_stats.total;
        let no_activity = dmg_total == 0
            && entity.time_series.is_empty()
            && heal_total == 0
            && entity.heal_time_series.is_empty()
            && taken_total == 0
            && entity.dmg_taken_time_series.is_empty();
        if no_activity {
            continue;
        }
        // 与ダメ/回復/被ダメの3指標を同時刻に独立採取する（回復タブの推移グラフ・固定基準バーが
        // 与ダメ系列を誤って描く問題の根本解消。各指標は自身の total が非0か既存系列が非空の
        // ときのみ採取＝未活動の指標に無駄な系列を作らない）。
        sample_metric_series(
            &mut entity.time_series,
            &mut entity.last_sample_total_dmg,
            dmg_total,
            elapsed_since_start,
            interval_actual,
            cap,
        );
        sample_metric_series(
            &mut entity.heal_time_series,
            &mut entity.last_sample_total_heal,
            heal_total,
            elapsed_since_start,
            interval_actual,
            cap,
        );
        sample_metric_series(
            &mut entity.dmg_taken_time_series,
            &mut entity.last_sample_total_dmg_taken,
            taken_total,
            elapsed_since_start,
            interval_actual,
            cap,
        );

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

/// 1エンティティ・1指標ぶんの時系列サンプルを採取する（与ダメ/回復/被ダメの3系統で共通）。
/// `total` が0かつ既存 `series` も空なら何もしない（その指標で一度も活動が無いエンティティに
/// 系列を作らずメモリを節約する）。`take_time_series_sample` の per-entity ループから
/// 3回（各指標1回）呼ぶことで、判定・採取ロジックを1箇所に集約し3重コピペを避ける。
fn sample_metric_series(
    series: &mut VecDeque<crate::models::TimeSeriesPoint>,
    last_sample_total: &mut i64,
    total: i64,
    elapsed_since_start: u128,
    interval_actual: u128,
    cap: usize,
) {
    if total == 0 && series.is_empty() {
        return;
    }
    let delta = total - *last_sample_total;
    let dps = if interval_actual > 0 {
        (delta as f64) * 1000.0 / (interval_actual as f64)
    } else {
        0.0
    };
    series.push_back(crate::models::TimeSeriesPoint {
        t_ms: elapsed_since_start as f64,
        total_dmg: total as f64,
        total_dps: dps.max(0.0),
    });
    while series.len() > cap {
        series.pop_front();
    }
    *last_sample_total = total;
}

/// EnterScene (自キャラ入場) の PlayerEnt.attrs を処理する。
/// AOI 同期(SyncNearEntities)には含まれない詳細ステータス（会心/ファスト/万能/知力/敏捷/
/// 魔攻/魔防 等）がここに入る。PlayerEnt は自キャラなので、未確定なら local_player_uid も確定する。
///
/// PlayerEnt が自キャラである性質から、この経路は WorldEnterSnapshot に次ぐ
/// conn ↔ char_id の学習源でもある（ゾーン移動のたびに届くため、ログイン時にしか
/// 来ない WorldEnterSnapshot より早く対象クライアントを特定できる）。
fn process_enter_scene(encounter: &mut Encounter, msg: pb::EnterScene, conn: Option<Server>) {
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

    // connection ↔ char_id を学習し、他クライアント由来ならエンティティを作らず捨てる。
    // （learn_connection が local_player_uid の未確定時確定も担う）
    if let Some(conn) = conn {
        if !learn_connection(encounter, conn, player_uid) {
            return;
        }
    } else if encounter.local_player_uid == 0 {
        encounter.set_local_player_uid(player_uid);
    }
    let target_entity = get_or_create_entity(encounter, EntityKey::player(player_uid));
    process_player_attrs(player_uid, target_entity, &attrs.attrs, "enter_scene");

    // 自キャラの既存バフ一覧（食事/シロップ等）を同期。EntityAppear と同型の
    // player_ent に載る（apply_appear_buff_sync 参照。field 7 の到来自体は
    // 2026-08時点でグローバル版 probe 未確認。到来ログで確認する）。
    if let Some(bundle) = &player_ent.buff_infos {
        apply_appear_buff_sync(encounter, player_uid, bundle, now_ms());
    }
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
    use crate::engine::encounter::{MeasureMode, MeasureScope};
    use std::sync::atomic::Ordering;

    fn set_ts_config(samples: usize, interval_ms: u64) {
        crate::engine::runtime_settings::TS_SAMPLES.store(samples, Ordering::Relaxed);
        crate::engine::runtime_settings::TS_INTERVAL_MS.store(interval_ms, Ordering::Relaxed);
    }

    fn player() -> Entity {
        Entity::default()
    }

    // 3分計測中は TS_SAMPLES(=60) を超えても全ウィンドウ分のサンプルを保持し、
    // 折れ線が左端(t=0)から始まる。継続戦闘なら通常サンプルだけで右端(=window)に届く。
    #[test]
    fn three_min_series_spans_full_window() {
        set_ts_config(60, 1000); // 既定相当: 直近60サンプルだけだと先頭が切り捨てられる設定

        let window_ms: u128 = 90_000; // 90s 窓（91サンプル > 上限60）
        let interval: u128 = 1000;

        let mut enc = Encounter {
            measure_mode: MeasureMode::Active3Min { armed_at_ms: 0, duration_ms: window_ms, scope: MeasureScope::default() },
            ..Default::default()
        };
        enc.entities.insert(EntityKey::player(1), player());

        let mut ts: u128 = 0;
        while ts <= window_ms {
            enc.dmg_stats.total += 1000;
            enc.entities.get_mut(&EntityKey::player(1)).unwrap().dmg_stats.total += 1000;
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

        let p = &enc.entities[&EntityKey::player(1)];
        assert_eq!(p.time_series.front().unwrap().t_ms, 0.0);
        assert_eq!(p.time_series.back().unwrap().t_ms, window_ms as f64);
    }

    // 与ダメが0のプレイヤー（純ヒーラー相当）でも、回復が非0なら heal_time_series が
    // 独立して刻まれる（with dmg_time_series は空のまま）。旧実装は dmg_stats.total==0 の
    // エンティティを丸ごと skip していたため、この観測が失われていた。
    #[test]
    fn heal_only_entity_samples_heal_series_independently_of_dmg() {
        set_ts_config(60, 1000);
        let mut enc = Encounter::default();
        enc.entities.insert(EntityKey::player(1), player());

        for i in 0..3u128 {
            let ts = i * 1000;
            enc.entities.get_mut(&EntityKey::player(1)).unwrap().heal_stats.total += 500;
            enc.time_last_combat_packet_ms = ts;
            take_time_series_sample(&mut enc, ts, false);
        }

        let p = &enc.entities[&EntityKey::player(1)];
        assert_eq!(p.heal_time_series.len(), 3, "heal series should sample every tick");
        assert!(p.time_series.is_empty(), "dmg series should stay empty (dmg total is 0)");
        assert!(p.dmg_taken_time_series.is_empty(), "taken series should stay empty (taken total is 0)");
        // 累計値(total_dmgフィールドを回復累計として流用)が正しく積み上がっている(500*3=1500)。
        assert_eq!(p.heal_time_series.back().unwrap().total_dmg, 1500.0);
    }

    // 与ダメ/回復/被ダメが同時に発生するプレイヤー（タンク兼ヒーラー等）は3系統が
    // 同時刻(t_ms)で独立に刻まれ、互いの値が混線しない。
    #[test]
    fn all_three_metrics_sample_independently_at_same_tick() {
        set_ts_config(60, 1000);
        let mut enc = Encounter::default();
        enc.entities.insert(EntityKey::player(1), player());

        for i in 0..3u128 {
            let ts = i * 1000;
            {
                let e = enc.entities.get_mut(&EntityKey::player(1)).unwrap();
                e.dmg_stats.total += 100;
                e.heal_stats.total += 200;
                e.dmg_taken_stats.total += 300;
            }
            enc.time_last_combat_packet_ms = ts;
            take_time_series_sample(&mut enc, ts, false);
        }

        let p = &enc.entities[&EntityKey::player(1)];
        assert_eq!(p.time_series.len(), 3);
        assert_eq!(p.heal_time_series.len(), 3);
        assert_eq!(p.dmg_taken_time_series.len(), 3);
        // 同じ t_ms で3系統が刻まれている（採取タイミングが一致）。
        for i in 0..3 {
            assert_eq!(p.time_series[i].t_ms, p.heal_time_series[i].t_ms);
            assert_eq!(p.time_series[i].t_ms, p.dmg_taken_time_series[i].t_ms);
        }
        // 累計値が指標ごとに正しく独立している(100/200/300刻みが混線しない)。
        assert_eq!(p.time_series.back().unwrap().total_dmg, 300.0);
        assert_eq!(p.heal_time_series.back().unwrap().total_dmg, 600.0);
        assert_eq!(p.dmg_taken_time_series.back().unwrap().total_dmg, 900.0);
    }

    /// 有効DPS（実働時間ベース）の配線確認。process_scene_delta 経由で実際に
    /// Entity::active_dmg_time が更新されることを見る（間隔キャップ・初回猶予そのものの
    /// 計算は combat_stats::active_time_tests の単体テストで検証済み）。Encounter 側の
    /// 同名フィールドは production コードから読まれないため廃止済み（encounter.rs 参照）。
    /// 同一バッチ内の複数ダメージイベントは同じ観測時刻を共有するため、初回バッチは
    /// 初回イベント分の猶予(500ms)のみが積まれ(2件目は間隔0)、2バッチ目でさらに増える。
    #[test]
    fn active_dmg_time_updates_via_scene_delta_processing() {
        let mut enc = Encounter::default();
        let monster_uuid = 1_i64 << 16 | 64;
        let attacker_uid = 42_i64;

        let delta = pb::SceneDelta {
            uuid: monster_uuid,
            skill_effects: Some(pb::SkillImpact {
                damages: vec![
                    pb::DamageRecord {
                        value: 100,
                        hp_lessen_value: 100,
                        attacker_uuid: player_uuid(attacker_uid),
                        owner_id: 1001,
                        ..Default::default()
                    },
                    pb::DamageRecord {
                        value: 50,
                        hp_lessen_value: 50,
                        attacker_uuid: player_uuid(attacker_uid),
                        owner_id: 1001,
                        ..Default::default()
                    },
                ],
            }),
            ..Default::default()
        };

        process_scene_delta(&mut enc, delta.clone());
        let attacker = enc.entities.get(&EntityKey::player(attacker_uid)).expect("attacker entity created");
        assert_eq!(
            attacker.active_dmg_time.active_ms,
            crate::engine::combat_stats::ACTIVE_TIME_GAP_GRACE_MS,
            "初回バッチは初回イベント分の猶予(500ms)のみが積まれる(同一バッチ内2件目は間隔0)"
        );

        // 2バッチ目は now_ms() のミリ秒粒度に対して確実に間隔が付くよう少し待つ
        // （実時間ベースの配線確認なので、他のテストのように ts を直接注入できない）。
        std::thread::sleep(std::time::Duration::from_millis(5));
        process_scene_delta(&mut enc, delta);
        let attacker = enc.entities.get(&EntityKey::player(attacker_uid)).unwrap();
        assert!(
            attacker.active_dmg_time.active_ms > crate::engine::combat_stats::ACTIVE_TIME_GAP_GRACE_MS,
            "2バッチ目の処理でさらに実働時間が積算されるはず"
        );
    }

    // 最後の戦闘パケットが間隔ゲート未満で通常サンプルされない場合でも、
    // 確定時の force サンプルで右端=last_combat に届く。
    #[test]
    fn finalize_force_sample_closes_right_edge() {
        set_ts_config(200, 1000);

        let mut enc = Encounter {
            measure_mode: MeasureMode::Active3Min { armed_at_ms: 0, duration_ms: 90_000, scope: MeasureScope::default() },
            ..Default::default()
        };
        enc.entities.insert(EntityKey::player(1), player());

        let mut ts: u128 = 0;
        while ts <= 5000 {
            enc.dmg_stats.total += 1000;
            enc.entities.get_mut(&EntityKey::player(1)).unwrap().dmg_stats.total += 1000;
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

    // AOI appear（他プレイヤーが視界に入った瞬間）が運ぶ既存バフ一覧（食事等）を
    // buff_tracker へ同期する。ダンジョンで先に食事していた他プレイヤーが appear
    // した時点でバッジが出るようにするための配線（apply_appear_buff_sync 参照）。
    // trusted なオフセット既知の下で create_time+offset+duration-now を使うことを、
    // 受信基準フォールバック(duration そのまま=30分)との差で判別する。
    #[test]
    fn appear_buff_sync_registers_existing_food_buff_using_server_time() {
        let mut enc = Encounter::default();

        const UID: i64 = 42;
        const FOOD_ID: i32 = 700083; // ConsumableBuffIds.json food[0]
        const DURATION_MS: i32 = 1_800_000; // 30分
        const TWENTY_FIVE_MIN_MS: i64 = 25 * 60 * 1000;

        // オフセットを既知(0)に固定する。以降 appear buff 自身の観測（create_time が
        // 25分前＝候補が大きい）が来ても min により上書きされない。
        let now_real = now_ms();
        enc.buff_tracker.observe_server_time(now_real as i64, now_real);
        assert_eq!(enc.buff_tracker.server_clock_offset_ms(now_real), Some(0));

        // 25分前に付与された食事(create_time 過去・duration は総時間)が appear で届く。
        let create_time = now_real as i64 - TWENTY_FIVE_MIN_MS;
        let batch = pb::WorldEntityBatch {
            appear: vec![pb::EntityAppear {
                uuid: player_uuid_for(UID),
                buff_infos: Some(pb::BuffSnapshotBundle {
                    uuid: player_uuid_for(UID),
                    buff_infos: vec![pb::BuffSnapshot {
                        buff_uuid: 1,
                        base_id: FOOD_ID,
                        level: 1,
                        host_uuid: player_uuid_for(UID),
                        table_uuid: 0,
                        create_time,
                        fire_uuid: 0,
                        layer: 1,
                        part_id: 0,
                        count: 1,
                        duration: DURATION_MS,
                        fight_source_info: None,
                    }],
                }),
                ..Default::default()
            }],
            disappear: vec![],
        };

        process_world_entity_batch(&mut enc, batch);

        let snaps = enc.buff_tracker.snapshot_for(UID, now_real);
        assert_eq!(snaps.len(), 1, "appear の buff_infos が tracker に入っていない");
        assert_eq!(snaps[0].base_id, FOOD_ID);

        // 期限は create_time+offset(0)+duration。経過25分ぶん減った残り5分になる
        // （受信基準フォールバックなら30分のままになってしまう＝配線ミスの検出）。
        const FIVE_MIN_MS: i64 = 5 * 60 * 1000;
        assert_eq!(snaps[0].remaining_ms, FIVE_MIN_MS);
    }

    // Pkt::WorldSyncServerTime（WorldNtf method 0x2B）をデコードし、
    // buff_tracker.observe_server_time_sync 経由で server_clock_offset_ms が
    // client-server になることをエンドツーエンドで確認する。conn: None（未識別コネクション）
    // でも処理されること（should_accept を経由しない配線）も合わせて確認する。
    #[test]
    fn world_sync_server_time_updates_offset_via_process_opcode() {
        let enc = EncounterMutex::default();

        // server はローカル壁時計±24h の妥当性窓内でなければ捨てられる（observe_server_time_sync）
        // ので、実時刻から組み立てる。
        let server_ms = now_ms() as i64 - 1_000;
        let client_ms = server_ms + 100;
        let data = pb::SyncServerTime {
            client_milliseconds: client_ms,
            server_milliseconds: server_ms,
        }
        .encode_to_vec();

        process_opcode(&enc, PktEnvelope { op: Pkt::WorldSyncServerTime, data, conn: None }).unwrap();

        let now = now_ms();
        let offset = enc.lock().unwrap().buff_tracker.server_clock_offset_ms(now);
        assert_eq!(offset, Some(client_ms - server_ms));
    }

    // NotifyJoinTeam(TeamNtf method 0x3) を process_opcode 経由で処理すると、
    // encounter.team にPT構成（team_id/leader/メンバー）が反映される。
    // should_accept を通らない（conn: None でも受理される）ことも合わせて確認する。
    #[test]
    fn notify_join_team_populates_encounter_team_via_process_opcode() {
        let enc = EncounterMutex::default();

        let data = pb::NotifyJoinTeam {
            v_request: Some(pb::NotifyJoinTeamRequest {
                base_info: Some(pb::TeamBaseInfo { team_id: 100, leader_id: 1 }),
                member_data: vec![pb::TeamMemData { char_id: 2 }, pb::TeamMemData { char_id: 3 }],
                member_sync_datas: Default::default(),
            }),
        }
        .encode_to_vec();

        process_opcode(&enc, PktEnvelope { op: Pkt::TeamJoin, data, conn: None }).unwrap();

        let team = enc.lock().unwrap().team.clone();
        assert_eq!(team.team_id, 100);
        assert_eq!(team.leader_uid, 1);
        assert!(team.is_member(1), "leader もメンバーに含まれる");
        assert!(team.is_member(2));
        assert!(team.is_member(3));
        assert!(!team.is_member(99));
    }

    fn social_envelope_bytes(char_id: i64, level_map_id: u32) -> Vec<u8> {
        pb::SocialEnvelope {
            v_request: Some(pb::SocialRequest {
                data: Some(pb::SocialBody {
                    char_id,
                    scene_data: Some(pb::WorldLocation { level_map_id, line_id: 1 }),
                }),
            }),
        }
        .encode_to_vec()
    }

    /// char_id が自キャラ(local_player_uid)と一致すれば current_level_map_id を更新する。
    #[test]
    fn social_envelope_accepts_matching_char_id() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let my_uid = 100_i64;
        let enc = EncounterMutex::default();
        enc.lock().unwrap().local_player_uid = my_uid;

        process_opcode(
            &enc,
            envelope(Pkt::SocialEnvelope, social_envelope_bytes(my_uid, 6545), conn(40101)),
        )
        .unwrap();

        assert_eq!(enc.lock().unwrap().current_level_map_id, 6545);
    }

    /// char_id が自キャラと異なれば破棄する（current_level_map_id は変わらない）。
    #[test]
    fn social_envelope_rejects_other_char_id() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let my_uid = 100_i64;
        let other_uid = 200_i64;
        let enc = EncounterMutex::default();
        enc.lock().unwrap().local_player_uid = my_uid;

        process_opcode(
            &enc,
            envelope(Pkt::SocialEnvelope, social_envelope_bytes(other_uid, 6545), conn(40102)),
        )
        .unwrap();

        assert_eq!(enc.lock().unwrap().current_level_map_id, 0, "他キャラの char_id は破棄する");
    }

    /// self_player_uid() は selected_uid（手動指定）を優先する。local_player_uid が
    /// 未確定(0)でも、selected_uid と char_id が不一致なら「未確定」扱いにせず破棄する。
    #[test]
    fn social_envelope_rejects_other_char_id_when_selected_uid_overrides_unconfirmed_local() {
        let _guard = lock_selected_uid();
        let self_uid = 100_i64;
        let other_uid = 200_i64;
        selected_uid::set(Some(self_uid));
        let enc = EncounterMutex::default();
        assert_eq!(enc.lock().unwrap().local_player_uid, 0, "テスト前提: local_player_uid 未確定");

        process_opcode(
            &enc,
            envelope(Pkt::SocialEnvelope, social_envelope_bytes(other_uid, 6545), conn(40105)),
        )
        .unwrap();

        assert_eq!(
            enc.lock().unwrap().current_level_map_id,
            0,
            "selected_uid と不一致の char_id は破棄する"
        );

        selected_uid::set(None);
    }

    /// 自キャラ未確定(local_player_uid==0 かつ selected_uid なし)なら char_id 不一致でも
    /// 暫定受理する（SocialEnvelope はシーン遷移要求時にしか届かないため、ここで捨てると
    /// 次の遷移が起きるまで level_map_id が不明なままになってしまう）。
    #[test]
    fn social_envelope_provisionally_accepts_when_self_uid_unconfirmed() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let enc = EncounterMutex::default();
        assert_eq!(enc.lock().unwrap().local_player_uid, 0, "テスト前提: 自キャラ未確定");

        process_opcode(
            &enc,
            envelope(Pkt::SocialEnvelope, social_envelope_bytes(999, 6545), conn(40103)),
        )
        .unwrap();

        assert_eq!(enc.lock().unwrap().current_level_map_id, 6545);
    }

    /// char_id==0 のときは char_id 自体の一致判定を行わず、conn によるフィルタだけが効く
    /// （他クライアントの conn だと学習済みなら破棄する）。
    #[test]
    fn social_envelope_zero_char_id_still_filters_by_conn() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let my_uid = 100_i64;
        let other_uid = 200_i64;
        let other_conn = conn(40104);
        let enc = EncounterMutex::default();
        {
            let mut e = enc.lock().unwrap();
            e.local_player_uid = my_uid;
            // other_conn は他キャラのものと学習済み
            e.conn_to_uid.insert(other_conn, other_uid);
        }

        process_opcode(
            &enc,
            envelope(Pkt::SocialEnvelope, social_envelope_bytes(0, 6545), other_conn),
        )
        .unwrap();

        assert_eq!(
            enc.lock().unwrap().current_level_map_id,
            0,
            "char_id=0 でも他クライアントの conn は破棄する"
        );
    }

    fn sync_dungeon_data_bytes(difficulty: Option<i32>) -> Vec<u8> {
        pb::SyncDungeonData {
            v_data: Some(pb::DungeonSyncData {
                dungeon_scene_info: difficulty.map(|difficulty| pb::DungeonSceneInfo { difficulty }),
            }),
        }
        .encode_to_vec()
    }

    /// 自キャラ(local_player_uid=100)確定済みの Encounter で、SyncDungeonData を1通流して段階を返す。
    fn stage_after_sync_dungeon_data(
        enc: &EncounterMutex,
        difficulty: Option<i32>,
        c: Server,
    ) -> u32 {
        process_opcode(
            enc,
            PktEnvelope {
                op: Pkt::WorldSyncDungeonData,
                data: sync_dungeon_data_bytes(difficulty),
                conn: Some(c),
            },
        )
        .unwrap();
        let stage = enc.lock().unwrap().current_dungeon_difficulty;
        stage
    }

    fn enc_with_self(uid: i64) -> EncounterMutex {
        let enc = EncounterMutex::default();
        enc.lock().unwrap().local_player_uid = uid;
        enc
    }

    /// 自キャラ(uid=100)の conn として学習済みにした conn を返す（計測中の1クライアントの実機ログでは、0x17 はこの種の conn から届いた）。
    fn learned_conn(enc: &EncounterMutex, port: u16) -> Server {
        let c = conn(port);
        enc.lock().unwrap().conn_to_uid.insert(c, 100);
        c
    }

    /// 0x17 の difficulty が current_dungeon_difficulty になる。
    #[test]
    fn sync_dungeon_data_sets_current_stage() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let enc = enc_with_self(100);
        let c = learned_conn(&enc, 40201);
        assert_eq!(stage_after_sync_dungeon_data(&enc, Some(3), c), 3);
    }

    /// 学習済みの他キャラの conn 由来は破棄する。
    #[test]
    fn sync_dungeon_data_rejects_other_client_conn() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let enc = enc_with_self(100);
        let other_conn = conn(40202);
        enc.lock().unwrap().conn_to_uid.insert(other_conn, 200);
        assert_eq!(stage_after_sync_dungeon_data(&enc, Some(3), other_conn), 0);
    }

    /// 自キャラ確定済みなら、未学習の conn 由来は破棄する（計測中の実機ログでは学習済みの自キャラ conn から届いた）。
    #[test]
    fn sync_dungeon_data_rejects_unlearned_conn_when_self_known() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let enc = enc_with_self(100);
        enc.lock().unwrap().current_dungeon_difficulty = 4;
        assert_eq!(stage_after_sync_dungeon_data(&enc, Some(5), conn(40203)), 4);
    }

    /// 一時停止中は学習パケットが処理されず conn が未学習のまま残るので、未学習 conn でも受理する。
    /// 学習済みの他キャラ conn は停止中でも破棄する。
    #[test]
    fn sync_dungeon_data_while_paused_accepts_unlearned_but_not_other_client() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let enc = enc_with_self(100);
        enc.lock().unwrap().is_paused = true;
        assert_eq!(stage_after_sync_dungeon_data(&enc, Some(5), conn(40210)), 5);
        let other = conn(40211);
        enc.lock().unwrap().conn_to_uid.insert(other, 200);
        assert_eq!(stage_after_sync_dungeon_data(&enc, Some(7), other), 5);
    }

    /// 自キャラが未確定なら判定材料が無いので、未学習の conn でも受理する。
    #[test]
    fn sync_dungeon_data_accepts_unlearned_conn_when_self_unknown() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let enc = EncounterMutex::default();
        assert_eq!(stage_after_sync_dungeon_data(&enc, Some(5), conn(40209)), 5);
    }

    /// dungeon_scene_info が無いときは段階を変えない。
    #[test]
    fn sync_dungeon_data_without_scene_info_keeps_stage() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let enc = enc_with_self(100);
        let c = learned_conn(&enc, 40204);
        enc.lock().unwrap().current_dungeon_difficulty = 4;
        assert_eq!(stage_after_sync_dungeon_data(&enc, None, c), 4);
    }

    /// 負の difficulty は 0（不明）にする。
    #[test]
    fn sync_dungeon_data_negative_difficulty_is_zero() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let enc = enc_with_self(100);
        let c = learned_conn(&enc, 40205);
        enc.lock().unwrap().current_dungeon_difficulty = 4;
        assert_eq!(stage_after_sync_dungeon_data(&enc, Some(-1), c), 0);
    }

    /// SocialEnvelope で level_map_id が変わったときの段階の扱い（from → to, 前の段階 8）。
    fn stage_after_scene_change(from: u32, to: u32) -> u32 {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let enc = enc_with_self(100);
        {
            let mut e = enc.lock().unwrap();
            e.current_level_map_id = from;
            e.current_dungeon_difficulty = 8;
        }
        process_opcode(
            &enc,
            envelope(Pkt::SocialEnvelope, social_envelope_bytes(100, to), conn(40206)),
        )
        .unwrap();
        let stage = enc.lock().unwrap().current_dungeon_difficulty;
        stage
    }

    #[test]
    fn social_envelope_scene_change_resets_stage_unless_master() {
        assert_eq!(stage_after_scene_change(6545, 8), 0, "マスター→町はリセット");
        assert_eq!(stage_after_scene_change(8, 6545), 8, "→マスターは据え置き(0x17 が先に届いた場合)");
        assert_eq!(
            stage_after_scene_change(6545, 999_999_999),
            8,
            "表に無い id は判断できないので据え置き"
        );
        assert_eq!(stage_after_scene_change(6545, 6543), 0, "→ノーマルはリセット");
        assert_eq!(stage_after_scene_change(6545, 6544), 0, "→ハードはリセット");
    }

    /// SocialEnvelope（マスターへ移動）の後に 0x17 が届く順序。
    #[test]
    fn stage_is_set_when_sync_dungeon_data_follows_social_envelope() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let enc = enc_with_self(100);
        let c = learned_conn(&enc, 40207);
        process_opcode(&enc, envelope(Pkt::SocialEnvelope, social_envelope_bytes(100, 6545), c))
            .unwrap();
        assert_eq!(stage_after_sync_dungeon_data(&enc, Some(2), c), 2);
        assert_eq!(enc.lock().unwrap().current_level_map_id, 6545);
    }

    /// 0x17 が先、SocialEnvelope（マスターへ移動）が後に届く順序でも、段階は消えない。
    #[test]
    fn stage_survives_social_envelope_arriving_after_sync_dungeon_data() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let enc = enc_with_self(100);
        let c = learned_conn(&enc, 40208);
        assert_eq!(stage_after_sync_dungeon_data(&enc, Some(2), c), 2);
        process_opcode(&enc, envelope(Pkt::SocialEnvelope, social_envelope_bytes(100, 6545), c))
            .unwrap();
        let e = enc.lock().unwrap();
        assert_eq!((e.current_level_map_id, e.current_dungeon_difficulty), (6545, 2));
    }

    fn world_enter_snapshot_bytes(char_id: i64, level_map_id: u32) -> Vec<u8> {
        pb::WorldEnterSnapshot {
            v_data: Some(pb::PlayerSnapshot {
                char_id,
                scene_data: Some(pb::WorldLocation { level_map_id, line_id: 1 }),
                ..Default::default()
            }),
        }
        .encode_to_vec()
    }

    /// WorldEnterSnapshot 経由の level_map_id 更新でも、SocialEnvelope と同じ段階の扱いになる
    /// （マスター以外の既知シーンでリセット、マスターでは据え置き）。
    fn stage_after_snapshot_level(from: u32, to: u32) -> u32 {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let enc = enc_with_self(100);
        {
            let mut e = enc.lock().unwrap();
            e.current_level_map_id = from;
            e.current_dungeon_difficulty = 8;
        }
        process_opcode(
            &enc,
            envelope(Pkt::WorldEnterSnapshot, world_enter_snapshot_bytes(100, to), conn(40209)),
        )
        .unwrap();
        let e = enc.lock().unwrap();
        assert_eq!(e.current_level_map_id, to, "level_map_id は更新される");
        let stage = e.current_dungeon_difficulty;
        stage
    }

    #[test]
    fn world_enter_snapshot_scene_change_resets_stage_unless_master() {
        assert_eq!(stage_after_snapshot_level(6545, 8), 0, "マスター→町はリセット");
        assert_eq!(stage_after_snapshot_level(6545, 6545), 8, "同じマスターでは据え置き");
        assert_eq!(stage_after_snapshot_level(8, 6545), 8, "→マスターは据え置き");
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
        enc.entities.insert(EntityKey::player(1), player());

        // 1007740 = 奥义!毒爆 → ヴェノミーンの巣（分身/召喚スキル）
        process_scene_delta(&mut enc, summon_spawn_delta(1, 1_007_740));
        assert_eq!(
            enc.entities[&EntityKey::player(1)].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string()]
        );
    }

    // 無ダメージのイマジン(アルーナ=蘇生)も召喚 spawn 経路で検知できる（本機能の主目的）。
    #[test]
    fn no_damage_imagine_detected_via_summon() {
        let mut enc = Encounter::default();
        enc.entities.insert(EntityKey::player(1), player());

        // 2900240 = 奥义！生命祈愿 → アルーナ（蘇生・ダメージを出さない）
        process_scene_delta(&mut enc, summon_spawn_delta(1, 2_900_240));
        assert_eq!(enc.entities[&EntityKey::player(1)].imagine_display_names(), vec!["アルーナ".to_string()]);
    }

    // 複数の召喚は発見順に累積し、同一名は重複させない。
    #[test]
    fn summon_accumulates_and_dedups() {
        let mut enc = Encounter::default();
        enc.entities.insert(EntityKey::player(1), player());

        process_scene_delta(&mut enc, summon_spawn_delta(1, 1_007_740)); // ヴェノミーンの巣
        process_scene_delta(&mut enc, summon_spawn_delta(1, 2_900_240)); // アルーナ
        assert_eq!(
            enc.entities[&EntityKey::player(1)].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );

        // 同じイマジン(別ID 1007741=虚拟体 も同名解決)を再度 → 重複しない
        process_scene_delta(&mut enc, summon_spawn_delta(1, 1_007_741));
        assert_eq!(
            enc.entities[&EntityKey::player(1)].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );
    }

    // イマジン表に無い召喚スキル(=職業召喚など)はイマジンとして扱わない（誤名回避）。
    #[test]
    fn non_imagine_summon_ignored() {
        let mut enc = Encounter::default();
        enc.entities.insert(EntityKey::player(1), player());

        // 55404 は ImagineSkillNames.json に無い（実機で観測した非イマジン召喚）
        process_scene_delta(&mut enc, summon_spawn_delta(1, 55_404));
        assert!(enc.entities[&EntityKey::player(1)].imagine_display_names().is_empty());
    }

    // オーナー(AttrTopSummonerId)が欠けた召喚 attr は帰属できず無視される。
    #[test]
    fn summon_without_owner_ignored() {
        let mut enc = Encounter::default();
        enc.entities.insert(EntityKey::player(1), player());

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
        assert!(enc.entities[&EntityKey::player(1)].imagine_display_names().is_empty());
    }

    // ロローラは実ゲーム版の召喚ID(2900840=奥義！神霊依凭)で解決できる（版ズレで名前グルーピング不能な分の手動追記）。
    #[test]
    fn rorora_detected_via_game_summon_id() {
        let mut enc = Encounter::default();
        enc.entities.insert(EntityKey::player(1), player());

        process_scene_delta(&mut enc, summon_spawn_delta(1, 2_900_840));
        assert_eq!(enc.entities[&EntityKey::player(1)].imagine_display_names(), vec!["ロローラ".to_string()]);
    }

    // 装備枠は2つ。pending 方式では 3体目(新規)を検知しても confirmed を即座には書き換えない
    // （rule4: 定員一杯・pending 空 → まだ確証が無いのでいったん pending へ留め置くだけ）。
    // 「新規名を検知した瞬間に古い方を追い出して新旧混在ペアを作ってしまう」という前回ロジックの
    // 問題そのものを避けるのが pending 方式の意図であり、この挙動変化はその直接の反映。
    #[test]
    fn imagine_names_capped_to_two_keeping_latest() {
        let mut enc = Encounter::default();
        enc.entities.insert(EntityKey::player(1), player());

        process_scene_delta(&mut enc, summon_spawn_delta(1, 1_007_740)); // A: ヴェノミーンの巣
        process_scene_delta(&mut enc, summon_spawn_delta(1, 2_900_240)); // B: アルーナ
        process_scene_delta(&mut enc, summon_spawn_delta(1, 2_900_840)); // C: ロローラ（3体目・新規）

        // confirmed は [A,B] のまま（[B,C] へは即座に丸められない）。
        assert_eq!(
            enc.entities[&EntityKey::player(1)].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );
        // C はまだ確証が無いので pending へ留め置かれるだけ。
        assert_eq!(
            enc.entities[&EntityKey::player(1)].pending_imagine.as_ref().map(|s| s.name.as_str()),
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
        enc.entities.insert(EntityKey::player(2), player());

        process_scene_delta(&mut enc, summon_spawn_delta(2, 1_007_740)); // A: ヴェノミーンの巣
        process_scene_delta(&mut enc, summon_spawn_delta(2, 2_900_240)); // B: アルーナ
        assert_eq!(
            enc.entities[&EntityKey::player(2)].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );

        // C（新規）を検知 → 定員一杯・pending 空 → rule4: pending へ留め置くだけ
        process_scene_delta(&mut enc, summon_spawn_delta(2, 2_900_840)); // C: ロローラ
        assert_eq!(
            enc.entities[&EntityKey::player(2)].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()],
            "pending 設定だけでは confirmed が変化してはいけない"
        );
        assert_eq!(
            enc.entities[&EntityKey::player(2)].pending_imagine.as_ref().map(|s| s.name.as_str()),
            Some("ロローラ")
        );

        // A（現役）を再検知 → rule1: pending(C) が確定へ昇格し、放置された B を置き換える
        process_scene_delta(&mut enc, summon_spawn_delta(2, 1_007_740));
        assert_eq!(
            enc.entities[&EntityKey::player(2)].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "ロローラ".to_string()]
        );
        assert!(enc.entities[&EntityKey::player(2)].pending_imagine.is_none());
    }

    // ② 両枠同時交換で「新旧混在ペア」が一度も画面に出ないことの直接的な証明（pending 方式の本質）。
    // A,B(confirmed) → C(新規・定員一杯・pending 空→pending 化。confirmed は完全に不変)
    // → D(pending とは別の新規・rule5)で両枠同時交換が確定し [C,D] へ一気に切り替わる。
    // このテストを通じて観測可能な confirmed は常に [A,B] か [C,D] のいずれかのみであり、
    // [A,C]/[B,D]/[B,C] のような混在ペアが一瞬たりとも表示されないことがポイント。
    #[test]
    fn dual_slot_swap_confirmed_only_after_second_new_name() {
        let mut enc = Encounter::default();
        enc.entities.insert(EntityKey::player(3), player());

        process_scene_delta(&mut enc, summon_spawn_delta(3, 1_007_740)); // A: ヴェノミーンの巣
        process_scene_delta(&mut enc, summon_spawn_delta(3, 2_900_240)); // B: アルーナ
        assert_eq!(
            enc.entities[&EntityKey::player(3)].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );

        // C（新規）を検知 → rule4: pending へ留め置くだけ。confirmed は旧ペア [A,B] のまま不変。
        process_scene_delta(&mut enc, summon_spawn_delta(3, 2_900_840)); // C: ロローラ
        assert_eq!(
            enc.entities[&EntityKey::player(3)].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()],
            "混在ペア([A,C]等)を一瞬でも見せてはいけない"
        );

        // D（pending とは別の新規）を検知 → rule5: 両枠同時交換の確定。[C,D] へ一気に切り替わる。
        process_scene_delta(&mut enc, summon_spawn_delta(3, 1_002_830)); // D: フロストオーガ
        assert_eq!(
            enc.entities[&EntityKey::player(3)].imagine_display_names(),
            vec!["ロローラ".to_string(), "フロストオーガ".to_string()]
        );
        assert!(enc.entities[&EntityKey::player(3)].pending_imagine.is_none());
    }

    // ③ cap 不変条件: A,B,C,D,E を検知しても confirmed は常に len()<=2 に収まる
    // （pending の有無に関わらず imagines への push/置換は常に定員内で完結するため）。
    #[test]
    fn imagine_count_never_exceeds_cap() {
        let mut enc = Encounter::default();
        enc.entities.insert(EntityKey::player(4), player());

        let skills = [1_007_740, 2_900_240, 2_900_840, 1_002_830, 1_007_741_i32];
        for &sk in &skills {
            // 1007741 は 1007740 と同名（ヴェノミーンの巣）解決だが cap 確認の分母には影響しない。
            process_scene_delta(&mut enc, summon_spawn_delta(4, sk));
            assert!(
                enc.entities[&EntityKey::player(4)].imagine_display_names().len() <= MAX_IMAGINE_NAMES,
                "imagine count exceeded cap after skill {sk}"
            );
        }
    }

    // ④ 表示順の安定性: A,B 検知後に一方・両方を何度再検知しても並び順は反転しない([B,A] にならない)。
    // 名前は常に既存2枠のいずれかと一致する（第3の新規名は登場しない）ので pending は一切絡まない。
    #[test]
    fn display_order_stable_across_rechecks() {
        let mut enc = Encounter::default();
        enc.entities.insert(EntityKey::player(5), player());

        process_scene_delta(&mut enc, summon_spawn_delta(5, 1_007_740)); // A
        process_scene_delta(&mut enc, summon_spawn_delta(5, 2_900_240)); // B
        assert_eq!(
            enc.entities[&EntityKey::player(5)].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );

        // B を複数回、A も再検知 → 並び替えは起きない
        process_scene_delta(&mut enc, summon_spawn_delta(5, 2_900_240));
        process_scene_delta(&mut enc, summon_spawn_delta(5, 2_900_240));
        process_scene_delta(&mut enc, summon_spawn_delta(5, 1_007_740));
        process_scene_delta(&mut enc, summon_spawn_delta(5, 2_900_240));

        assert_eq!(
            enc.entities[&EntityKey::player(5)].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()],
            "display order must not reverse to [B, A]"
        );
        assert!(enc.entities[&EntityKey::player(5)].pending_imagine.is_none());
    }

    // ⑤ pending 自身の再検知だけでは確定に至らない回帰防止テスト。A,B(confirmed)→C(新規・pending
    // 化)の後、C を再検知しても rule2 が発火するだけで confirmed は不変・pending も C のまま
    // （＝ pending の再検知は「まだ現役の証拠」にはならず、昇格には既存スロットの再検知＝rule1、
    // または別の新規名＝rule5 のいずれかが必要）。
    #[test]
    fn pending_redetection_does_not_promote_alone() {
        let mut enc = Encounter::default();
        enc.entities.insert(EntityKey::player(6), player());

        process_scene_delta(&mut enc, summon_spawn_delta(6, 1_007_740)); // A
        process_scene_delta(&mut enc, summon_spawn_delta(6, 2_900_240)); // B
        process_scene_delta(&mut enc, summon_spawn_delta(6, 2_900_840)); // C（新規）→ pending

        process_scene_delta(&mut enc, summon_spawn_delta(6, 2_900_840)); // C を再検知（rule2）

        assert_eq!(
            enc.entities[&EntityKey::player(6)].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()],
            "pending 自身の再検知だけでは confirmed を書き換えてはいけない"
        );
        assert_eq!(
            enc.entities[&EntityKey::player(6)].pending_imagine.as_ref().map(|s| s.name.as_str()),
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
        enc.entities.insert(EntityKey::player(7), player());

        process_scene_delta(&mut enc, summon_spawn_delta(7, 1_007_740)); // A: ヴェノミーンの巣
        process_scene_delta(&mut enc, summon_spawn_delta(7, 2_900_240)); // B: アルーナ
        process_scene_delta(&mut enc, summon_spawn_delta(7, 2_900_840)); // C（新規）→ rule4: pending(hits=1)

        // hits=2（PENDING_PROMOTE_HITS=3 未満）→ まだ昇格しない
        process_scene_delta(&mut enc, summon_spawn_delta(7, 2_900_840));
        assert_eq!(
            enc.entities[&EntityKey::player(7)].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()],
            "閾値未満の再検知では自己修復してはいけない"
        );

        // hits=3（閾値到達）→ 自己修復: 旧確定ペア[A,B]を両方破棄し、C だけを単独確定にする
        process_scene_delta(&mut enc, summon_spawn_delta(7, 2_900_840));
        assert_eq!(
            enc.entities[&EntityKey::player(7)].imagine_display_names(),
            vec!["ロローラ".to_string()],
            "休眠相方のため C だけの単独確定へ自己修復するべき"
        );
        assert!(enc.entities[&EntityKey::player(7)].pending_imagine.is_none());

        // 自己修復後に別の新規名 D を検知 → rule3（定員未満）で 2 枠目へ直接追加され、
        // 混在ペアを経由せず [C, D] へ回復する。
        process_scene_delta(&mut enc, summon_spawn_delta(7, 1_002_830)); // D: フロストオーガ
        assert_eq!(
            enc.entities[&EntityKey::player(7)].imagine_display_names(),
            vec!["ロローラ".to_string(), "フロストオーガ".to_string()]
        );
    }

    /// キングゴブリン(3946・召喚報告ID無し)＋A を確定済みにしたプレイヤー。3946 は召喚経路では
    /// 検知できない（スキルリスト attr116 かキャッシュ復元でのみ確定する）ため直接構築する。
    fn player_with_goblin_king(uid: i64) -> Encounter {
        let mut enc = Encounter::default();
        enc.entities.insert(EntityKey::player(uid), player());
        let owner = enc.entities.get_mut(&EntityKey::player(uid)).unwrap();
        owner.imagines = vec![
            ImagineSlot { name: "キングゴブリン".to_string(), last_seen: 0, tier: 5, pending_hits: 0 },
            ImagineSlot { name: "ヴェノミーンの巣".to_string(), last_seen: 1, tier: 0, pending_hits: 0 },
        ];
        owner.imagines_authoritative = true; // attr116 で確定済みの想定
        enc
    }

    // ⑥d 親が前セッションの name_cache 由来（imagines_authoritative=false）のときは、子召喚を
    // 親の鮮度更新に留め、pending の昇格証拠には使わない。真の装備が [嵐のゴブリンウォーリアー, C]
    // へ変わっていた場合、C の pending が rule2 の自己修復で単独確定→親が外れ→以後の子召喚が
    // rule3 で正しく追加される（誤った [キングゴブリン, C] を確定・永続させない）。
    #[test]
    fn nested_summon_child_only_refreshes_parent_when_not_authoritative() {
        let mut enc = player_with_goblin_king(12);
        enc.entities.get_mut(&EntityKey::player(12)).unwrap().imagines_authoritative = false;

        process_scene_delta(&mut enc, summon_spawn_delta(12, 2_900_840)); // C: ロローラ → pending
        process_scene_delta(&mut enc, summon_spawn_delta(12, 1_008_641)); // 子召喚: 鮮度更新のみ
        let owner = &enc.entities[&EntityKey::player(12)];
        assert_eq!(
            owner.imagine_display_names(),
            vec!["キングゴブリン".to_string(), "ヴェノミーンの巣".to_string()],
            "推定だけの親では子召喚を交換証拠にしない"
        );
        assert_eq!(owner.pending_imagine.as_ref().map(|s| s.name.as_str()), Some("ロローラ"));

        // C の再検知が PENDING_PROMOTE_HITS 回に達すると自己修復で [C] 単独確定
        for _ in 1..PENDING_PROMOTE_HITS {
            process_scene_delta(&mut enc, summon_spawn_delta(12, 2_900_840));
        }
        assert_eq!(
            enc.entities[&EntityKey::player(12)].imagine_display_names(),
            vec!["ロローラ".to_string()]
        );
        // 親が外れたので、子召喚は通常の rule3 で真の装備として追加される
        process_scene_delta(&mut enc, summon_spawn_delta(12, 1_008_641));
        assert_eq!(
            enc.entities[&EntityKey::player(12)].imagine_display_names(),
            vec!["ロローラ".to_string(), "嵐のゴブリンウォーリアー".to_string()]
        );
    }

    // attr116 で確定した uid は、Encounter リセット後にキャッシュから復元された entity でも
    // imagines_authoritative を引き継ぐ（SKILL_LIST_CONFIRMED_UIDS による持ち越し）。
    #[test]
    fn skill_list_confirmation_survives_entity_recreation_from_cache() {
        let mut enc = Encounter::default();
        let uid = 990_012; // name_cache はプロセス共有のため専用 uid
        enc.entities.insert(EntityKey::player(uid), player());
        assert!(!enc.entities[&EntityKey::player(uid)].imagines_authoritative);

        process_scene_delta(&mut enc, skill_list_delta(uid, &[(3946, 5), (3942, 3)]));
        assert!(enc.entities[&EntityKey::player(uid)].imagines_authoritative);

        enc.clear_combat_stats(); // プレイヤー entity は破棄される
        assert!(!enc.entities.contains_key(&EntityKey::player(uid)));
        let restored = get_or_create_entity(&mut enc, EntityKey::player(uid));
        assert!(restored.imagines_authoritative, "当セッションで attr116 を見た uid は権威的のまま");
        assert_eq!(
            restored.imagine_display_names(),
            vec!["キングゴブリン".to_string(), "ヴェノミーンの巣".to_string()]
        );
    }

    // ⑥ 入れ子召喚（rule0）: キングゴブリンの召喚体は子イマジン側の召喚スキル
    // （1008641=嵐の大斧ゴブリン→嵐のゴブリンウォーリアー）を報告する。実機(2026-08-23)では
    // これが rule4 で pending → 相方の再検知(rule1)で「キングゴブリン→嵐のゴブリンウォーリアー」の
    // 単枠交換が確定し、装備していないイマジンが表示される誤りが繰り返し起きていた。
    // 修正後は親(キングゴブリン)の再検知として吸収され、confirmed も pending も変化しない。
    #[test]
    fn nested_summon_child_is_absorbed_as_goblin_king_reactivation() {
        let mut enc = player_with_goblin_king(8);
        let seq_before = enc.entities[&EntityKey::player(8)].imagines[0].last_seen;

        process_scene_delta(&mut enc, summon_spawn_delta_with_tier(8, 1_008_641, 3)); // 嵐の大斧ゴブリン
        let owner = &enc.entities[&EntityKey::player(8)];
        assert_eq!(
            owner.imagine_display_names(),
            vec!["キングゴブリン".to_string(), "ヴェノミーンの巣".to_string()]
        );
        assert!(owner.pending_imagine.is_none(), "子召喚は pending にしてはいけない");
        assert!(owner.imagines[0].last_seen > seq_before, "親の鮮度が更新される(現役の証拠)");
        assert_eq!(owner.imagines[0].tier, 5, "子召喚体の凸数で親の凸数を上書きしない");

        // 相方(A)の再検知でも、pending が無いので交換は起きない（旧バグの再現経路）。
        process_scene_delta(&mut enc, summon_spawn_delta(8, 1_007_740));
        assert_eq!(
            enc.entities[&EntityKey::player(8)].imagine_display_names(),
            vec!["キングゴブリン".to_string(), "ヴェノミーンの巣".to_string()]
        );
    }

    // ⑥b 親の再検知として扱われるので、pending がある状態で子召喚が来ると rule1 の単枠交換が
    // 「キングゴブリン＝現役」を根拠に進む＝相方(A)が pending へ置き換わり、親は残る。
    #[test]
    fn nested_summon_child_acts_as_parent_recheck_for_single_slot_swap() {
        let mut enc = player_with_goblin_king(9);
        process_scene_delta(&mut enc, summon_spawn_delta(9, 2_900_840)); // C: ロローラ → pending
        assert_eq!(
            enc.entities[&EntityKey::player(9)].pending_imagine.as_ref().map(|s| s.name.as_str()),
            Some("ロローラ")
        );

        process_scene_delta(&mut enc, summon_spawn_delta(9, 1_008_641)); // 子召喚 = 親の再検知
        let owner = &enc.entities[&EntityKey::player(9)];
        assert_eq!(
            owner.imagine_display_names(),
            vec!["キングゴブリン".to_string(), "ロローラ".to_string()]
        );
        assert!(owner.pending_imagine.is_none());
    }

    // ⑥c 親を装備していないプレイヤーでは従来どおり（嵐のゴブリンウォーリアー装備者の召喚を
    // そのまま帰属する）。親子を両方装備している場合も子の rule1 再検知が優先される。
    #[test]
    fn nested_summon_child_attributes_normally_without_goblin_king() {
        let mut enc = Encounter::default();
        enc.entities.insert(EntityKey::player(10), player());
        process_scene_delta(&mut enc, summon_spawn_delta(10, 1_008_641));
        assert_eq!(
            enc.entities[&EntityKey::player(10)].imagine_display_names(),
            vec!["嵐のゴブリンウォーリアー".to_string()]
        );

        let mut enc = player_with_goblin_king(11);
        enc.entities.get_mut(&EntityKey::player(11)).unwrap().imagines[1].name =
            "嵐のゴブリンウォーリアー".to_string();
        let parent_seq = enc.entities[&EntityKey::player(11)].imagines[0].last_seen;
        process_scene_delta(&mut enc, summon_spawn_delta_with_tier(11, 1_008_641, 4));
        let owner = &enc.entities[&EntityKey::player(11)];
        assert_eq!(owner.imagines[0].last_seen, parent_seq, "親ではなく子スロットの再検知");
        assert_eq!(owner.imagines[1].tier, 4, "子スロット自身の凸数は追従する");
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
        enc.entities.insert(EntityKey::player(uid), player());

        // 凸数付き検知 → ラベルに (5)。凸数無し検知 → 名前のみ。
        process_scene_delta(&mut enc, summon_spawn_delta_with_tier(uid, 1_007_740, 5));
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 2_900_240));
        assert_eq!(
            enc.entities[&EntityKey::player(uid)].imagine_display_labels(),
            vec!["ヴェノミーンの巣(5)".to_string(), "アルーナ".to_string()]
        );
        // 一致判定・永続化用の名前一覧は凸数を含まない（名前のみ）。
        assert_eq!(
            enc.entities[&EntityKey::player(uid)].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );

        // 再検知で凸数が判明したら追従する（0→3）。
        process_scene_delta(&mut enc, summon_spawn_delta_with_tier(uid, 2_900_240, 3));
        assert_eq!(
            enc.entities[&EntityKey::player(uid)].imagine_display_labels(),
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
        enc.entities.insert(EntityKey::player(uid), player());

        // 事前状態: 古い確定ペア[A,B]+pending(C) を召喚検知で作っておく。
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 1_007_740)); // A
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 2_900_240)); // B
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 2_900_840)); // C → pending
        assert!(enc.entities[&EntityKey::player(uid)].pending_imagine.is_some());

        // フルリスト（クラススキル10+イマジン2: 3902=サンダーオーガ凸0, 3906=フロストオーガ凸2）
        process_scene_delta(&mut enc, skill_list_delta(uid, &[(3902, 0), (3906, 2)]));
        assert_eq!(
            enc.entities[&EntityKey::player(uid)].imagine_display_labels(),
            vec!["サンダーオーガ".to_string(), "フロストオーガ(2)".to_string()]
        );
        assert!(enc.entities[&EntityKey::player(uid)].pending_imagine.is_none());

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
            enc.entities[&EntityKey::player(uid)].imagine_display_names(),
            vec!["サンダーオーガ".to_string(), "フロストオーガ".to_string()],
            "閾値未満の部分リストで確定表示を壊してはいけない"
        );

        // その後の装備替えは従来の召喚検知が追従する（新規名ロローラ→pending 止まり）。
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 2_900_840));
        assert!(enc.entities[&EntityKey::player(uid)].pending_imagine.is_some());
        assert_eq!(
            enc.entities[&EntityKey::player(uid)].imagine_display_names(),
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
        enc.entities.insert(EntityKey::player(uid), player());

        process_scene_delta(&mut enc, skill_list_delta(uid, &[(3906, 1), (3910, 0), (3021, 4)]));

        assert_eq!(
            enc.entities[&EntityKey::player(uid)].imagine_display_labels(),
            vec!["フロストオーガ(1)".to_string(), "虚蝕オーガ".to_string()],
            "role skill id must not appear in the real 2-slot imagines array"
        );
        assert_eq!(
            enc.entities[&EntityKey::player(uid)].role_skill_imagine_labels(),
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
        enc.entities.insert(EntityKey::player(uid), player());

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
            "キングムーク".to_string(),
            "鉄牙(3)".to_string(),
        ];
        assert_eq!(
            enc.entities[&EntityKey::player(uid)].role_skill_imagine_labels(),
            expected_labels,
            "all 4 simultaneous role skill slots must resolve without dropping any"
        );
        assert_eq!(
            enc.entities[&EntityKey::player(uid)].imagine_display_labels(),
            vec!["フロストオーガ(1)".to_string(), "虚蝕オーガ".to_string()],
            "role skill ids must never pollute the real 2-slot imagines array"
        );

        // 各ロールスキルの召喚エコー(canonical id)が来ても imagines/pending には一切触れない。
        for skill_id in [3902, 3901, 3908, 3943] {
            process_scene_delta(&mut enc, summon_spawn_delta(uid, skill_id));
        }

        assert_eq!(
            enc.entities[&EntityKey::player(uid)].imagine_display_labels(),
            vec!["フロストオーガ(1)".to_string(), "虚蝕オーガ".to_string()],
            "summon echoes of all 4 role skills must not disturb the confirmed real imagine pair"
        );
        assert!(enc.entities[&EntityKey::player(uid)].pending_imagine.is_none());
        assert_eq!(
            enc.entities[&EntityKey::player(uid)].role_skill_imagine_labels(),
            expected_labels,
            "role skill labels must remain intact after echo absorption for all 4 slots"
        );

        let cached = name_cache::lookup(uid).expect("cache entry should exist");
        assert_eq!(
            cached.role_skill_imagine_names,
            vec![
                "サンダーオーガ".to_string(),
                "フレイムオーガ".to_string(),
                "キングムーク".to_string(),
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
        enc.entities.insert(EntityKey::player(uid), player());

        // role_skill_imagine 未確定のため、召喚エコー(canonical id=3902→サンダーオーガ)が
        // rule3(定員未満)で誤って実イマジンとして確定してしまう。
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 3902));
        assert_eq!(
            enc.entities[&EntityKey::player(uid)].imagine_display_names(),
            vec!["サンダーオーガ".to_string()],
            "precondition: summon echo must be misattributed to imagines before role_skill_imagine is known"
        );

        // フルリスト到達（ロールスキルID(3021→サンダーオーガ)のみ・canonical idは0件）。
        process_scene_delta(&mut enc, skill_list_delta(uid, &[(3021, 4)]));

        assert!(
            enc.entities[&EntityKey::player(uid)].imagine_display_names().is_empty(),
            "stale misattributed imagines entry must be evicted once the full snapshot proves it's not a real slot"
        );
        assert_eq!(
            enc.entities[&EntityKey::player(uid)].role_skill_imagine_labels(),
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
        enc.entities.insert(EntityKey::player(uid), player());

        process_scene_delta(&mut enc, skill_list_delta(uid, &[(3906, 1), (3910, 0), (3021, 4)]));
        assert!(!enc.entities[&EntityKey::player(uid)].role_skill_imagines.is_empty());

        // 同じ実イマジン2枠のみでロールスキル対象IDを含まないフルリストが届く。
        process_scene_delta(&mut enc, skill_list_delta(uid, &[(3906, 1), (3910, 0)]));
        assert!(
            enc.entities[&EntityKey::player(uid)].role_skill_imagines.is_empty(),
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
        enc.entities.insert(EntityKey::player(uid), player());

        process_scene_delta(&mut enc, summon_spawn_delta(uid, 1_007_740)); // ヴェノミーンの巣
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 2_900_240)); // アルーナ
        assert_eq!(
            enc.entities[&EntityKey::player(uid)].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );

        // ロールスキル枠を「ロローラ」として既に確定済みにしておく（apply_skill_list_imagines相当）。
        enc.entities.get_mut(&EntityKey::player(uid)).unwrap().role_skill_imagines = vec![ImagineSlot {
            name: "ロローラ".to_string(),
            last_seen: 0,
            tier: 0,
            pending_hits: 0,
        }];

        // ロールスキルの簡易発動による召喚シグナル（実イマジンと同一の召喚報告ID経由=ロローラ）。
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 2_900_840));

        assert_eq!(
            enc.entities[&EntityKey::player(uid)].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()],
            "role skill echo must not disturb the confirmed real imagine pair"
        );
        assert!(
            enc.entities[&EntityKey::player(uid)].pending_imagine.is_none(),
            "role skill echo must not create a pending candidate"
        );
        assert_eq!(
            enc.entities[&EntityKey::player(uid)].role_skill_imagine_names(),
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
        enc.entities.insert(EntityKey::player(uid), player());

        {
            let owner = enc.entities.get_mut(&EntityKey::player(uid)).unwrap();
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
            enc.entities[&EntityKey::player(uid)].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "ロローラ".to_string()],
            "rule1 must still perform its normal single-slot swap even when role_skill_imagines shares A's name"
        );
        assert!(enc.entities[&EntityKey::player(uid)].pending_imagine.is_none());
        assert_eq!(
            enc.entities[&EntityKey::player(uid)].role_skill_imagine_names(),
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

        enc.entities.insert(EntityKey::player(uid), player());
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

        enc.entities.insert(EntityKey::player(uid), player());
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 1_007_740)); // A: ヴェノミーンの巣
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 2_900_240)); // B: アルーナ
        assert_eq!(
            enc.entities[&EntityKey::player(uid)].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );

        enc.clear_combat_stats(); // Player entity は破棄される（name_cache には残る）
        assert!(!enc.entities.contains_key(&EntityKey::player(uid)));

        // 次パケットで A を再検知 → name_cache から [A,B] を復元した上で A の鮮度を更新するのみ
        // （pending が無いので confirmed は不変）。
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 1_007_740));
        assert_eq!(
            enc.entities[&EntityKey::player(uid)].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()]
        );

        // 続けて C（ロローラ）を新規検知 → 定員一杯・pending 空 → rule4: pending へ留め置くだけ
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 2_900_840));
        assert_eq!(
            enc.entities[&EntityKey::player(uid)].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "アルーナ".to_string()],
            "pending 設定だけでは confirmed を書き換えない"
        );

        // もう一度 A を再検知 → rule1: pending(C) が確定へ昇格し、放置された B を置き換える
        process_scene_delta(&mut enc, summon_spawn_delta(uid, 1_007_740));
        assert_eq!(
            enc.entities[&EntityKey::player(uid)].imagine_display_names(),
            vec!["ヴェノミーンの巣".to_string(), "ロローラ".to_string()]
        );
        assert!(enc.entities[&EntityKey::player(uid)].pending_imagine.is_none());
    }

    fn conn(port: u16) -> Server {
        Server::new([10, 0, 0, 1], port, [192, 168, 0, 2], 5000)
    }

    fn lock_selected_uid() -> std::sync::MutexGuard<'static, ()> {
        selected_uid::lock_for_test()
    }

    /// 複数クライアント同時起動時の conn フィルタ。selected_uid はプロセス共有の
    /// グローバルなので、分岐を1テストに集約して直列に検証し、最後に必ず解除する。
    #[test]
    fn should_accept_filters_by_selected_uid_across_multiple_clients() {
        let _guard = lock_selected_uid();
        let mine = conn(40001); // 自分のキャラのクライアント
        let other = conn(40002); // 別キャラのクライアント
        let my_uid = 111_i64;
        let other_uid = 222_i64;

        // ── UID 未指定: 先着 char_id に固定される
        // （learn_connection は active_connection と local_player_uid を同時に確定させる）
        selected_uid::set(None);
        let mut enc = Encounter::default();
        assert!(
            should_accept(&mut enc, Some(mine), &Pkt::WorldDeltaBatch),
            "UID 未指定・完全未確定なら accept"
        );
        assert!(learn_connection(&mut enc, mine, my_uid));
        assert!(should_accept(&mut enc, Some(mine), &Pkt::WorldDeltaBatch));
        assert!(
            !should_accept(&mut enc, Some(other), &Pkt::WorldDeltaBatch),
            "対象キャラ確定後は身元不明の conn を弾く"
        );
        assert!(
            !learn_connection(&mut enc, other, other_uid),
            "別 char_id の conn は対象外"
        );
        assert!(
            !should_accept(&mut enc, Some(other), &Pkt::WorldDeltaBatch),
            "学習後も別キャラの conn は弾く"
        );

        // ── UID 指定・学習前: 身元不明の conn は両方とも弾く（混線防止の本丸）
        selected_uid::set(Some(my_uid));
        let mut enc = Encounter::default();
        assert!(
            !should_accept(&mut enc, Some(mine), &Pkt::WorldDeltaBatch),
            "UID 指定中は未学習 conn を通さない"
        );
        assert!(!should_accept(&mut enc, Some(other), &Pkt::WorldDeltaBatch));

        // ── ただし学習経路のパケットは常に通す（弾くと永久に特定できない）
        assert!(should_accept(&mut enc, Some(other), &Pkt::WorldEnterSnapshot));
        assert!(should_accept(&mut enc, Some(other), &Pkt::WorldEnterScene));
        assert!(should_accept(&mut enc, Some(other), &Pkt::ServerHandover));

        // ── 学習後: 一致 conn だけ通り、active_connection が確定する
        enc.conn_to_uid.insert(mine, my_uid);
        enc.conn_to_uid.insert(other, other_uid);
        assert!(
            !should_accept(&mut enc, Some(other), &Pkt::WorldDeltaBatch),
            "別キャラの conn は学習後も弾く"
        );
        assert!(should_accept(&mut enc, Some(mine), &Pkt::WorldDeltaBatch));
        assert_eq!(enc.active_connection, Some(mine));

        // ── 先に他 conn が active になっていても、UID 一致側へ矯正される
        let mut enc = Encounter::default();
        enc.active_connection = Some(other);
        enc.conn_to_uid.insert(mine, my_uid);
        enc.conn_to_uid.insert(other, other_uid);
        assert!(!should_accept(&mut enc, Some(other), &Pkt::WorldDeltaBatch));
        assert!(should_accept(&mut enc, Some(mine), &Pkt::WorldDeltaBatch));
        assert_eq!(enc.active_connection, Some(mine));

        selected_uid::set(None);
    }

    /// EnterScene は自キャラ入場通知なので conn ↔ char_id を学習する。
    /// 他クライアント由来なら学習だけ行い、エンティティは作らない。
    #[test]
    fn enter_scene_learns_conn_and_rejects_other_client() {
        let _guard = lock_selected_uid();
        let other = conn(40003);
        let other_uid = 333_i64;
        selected_uid::set(Some(444));

        let mut enc = Encounter::default();
        let msg = pb::EnterScene {
            enter_scene_info: Some(pb::EnterSceneInfo {
                player_ent: Some(pb::EntityAppear {
                    uuid: other_uid << 16 | 640,
                    attrs: Some(pb::EntityAttrs { attrs: vec![], ..Default::default() }),
                    ..Default::default()
                }),
            }),
        };
        process_enter_scene(&mut enc, msg, Some(other));

        assert_eq!(
            enc.conn_to_uid.get(&other),
            Some(&other_uid),
            "他クライアントでも conn↔uid は学習する（以降その conn を弾くため）"
        );
        assert_eq!(enc.active_connection, None, "他クライアントを active にしない");
        assert!(enc.entities.is_empty(), "他キャラのエンティティを作らない");
        assert_eq!(enc.local_player_uid, 0);

        selected_uid::set(None);
    }

    fn player_uuid(uid: i64) -> i64 {
        uid << 16 | 640
    }

    fn enter_scene_bytes(uid: i64) -> Vec<u8> {
        pb::EnterScene {
            enter_scene_info: Some(pb::EnterSceneInfo {
                player_ent: Some(pb::EntityAppear {
                    uuid: player_uuid(uid),
                    attrs: Some(pb::EntityAttrs { attrs: vec![], ..Default::default() }),
                    ..Default::default()
                }),
            }),
        }
        .encode_to_vec()
    }

    /// attacker_uid が target_uuid へ value ダメージを与える WorldDeltaBatch。
    fn delta_damage_bytes(target_uuid: i64, attacker_uid: i64, value: i64) -> Vec<u8> {
        pb::WorldDeltaBatch {
            delta_infos: vec![pb::SceneDelta {
                uuid: target_uuid,
                skill_effects: Some(pb::SkillImpact {
                    damages: vec![pb::DamageRecord {
                        value,
                        hp_lessen_value: value,
                        attacker_uuid: player_uuid(attacker_uid),
                        owner_id: 1001,
                        ..Default::default()
                    }],
                }),
                ..Default::default()
            }],
        }
        .encode_to_vec()
    }

    fn envelope(op: Pkt, data: Vec<u8>, conn: Server) -> PktEnvelope {
        PktEnvelope { op, data, conn: Some(conn) }
    }

    /// 2クライアント同時稼働のエンドツーエンド。実パケットを encode して
    /// process_opcode に流し、指定 UID のデータだけが集計されることを確認する。
    #[test]
    fn two_clients_only_selected_uid_is_aggregated_end_to_end() {
        let _guard = lock_selected_uid();
        let my_uid = 555_i64;
        let other_uid = 666_i64;
        let mine = conn(40011);
        let other = conn(40012);
        let monster_uuid = 7_i64 << 16 | 64;

        selected_uid::set(Some(my_uid));
        let enc = EncounterMutex::default();
        enc.lock().unwrap().local_player_uid = my_uid;

        // 両クライアントが入場（他クライアントが先着でも引きずられないこと）
        process_opcode(&enc, envelope(Pkt::WorldEnterScene, enter_scene_bytes(other_uid), other))
            .unwrap();
        process_opcode(&enc, envelope(Pkt::WorldEnterScene, enter_scene_bytes(my_uid), mine))
            .unwrap();

        // 両クライアントで戦闘が発生
        process_opcode(
            &enc,
            envelope(Pkt::WorldDeltaBatch, delta_damage_bytes(monster_uuid, other_uid, 9_999), other),
        )
        .unwrap();
        process_opcode(
            &enc,
            envelope(Pkt::WorldDeltaBatch, delta_damage_bytes(monster_uuid, my_uid, 100), mine),
        )
        .unwrap();

        let e = enc.lock().unwrap();
        assert_eq!(e.conn_to_uid.get(&mine), Some(&my_uid));
        assert_eq!(e.conn_to_uid.get(&other), Some(&other_uid));
        assert_eq!(e.active_connection, Some(mine), "指定 UID 側の conn が active");
        assert_eq!(e.dmg_stats.total, 100, "他クライアントの 9999 ダメージが混入していない");
        assert!(e.participant_player_uids.contains(&my_uid));
        assert!(
            !e.participant_player_uids.contains(&other_uid),
            "他キャラが参加者として計上されていない"
        );
        assert!(!e.entities.contains_key(&EntityKey::player(other_uid)), "他キャラのエンティティが作られていない");
        drop(e);

        selected_uid::set(None);
    }

    /// 戦闘中にアプリを後から起動したケース。EnterScene も WorldEnterSnapshot も
    /// 届かないため conn を一切学習できない。この状態で身元不明の conn を通すと
    /// 両クライアントのダメージが合算されてしまう（修正前の実際の症状）。
    /// UID 指定中は「特定できるまで何も採用しない」が正しい。
    #[test]
    fn two_clients_mid_session_start_rejects_unidentified_connections() {
        let _guard = lock_selected_uid();
        let my_uid = 555_i64;
        let other_uid = 666_i64;
        let mine = conn(40031);
        let other = conn(40032);
        let monster_uuid = 9_i64 << 16 | 64;

        selected_uid::set(Some(my_uid));
        let enc = EncounterMutex::default();
        enc.lock().unwrap().local_player_uid = my_uid;

        // 学習パケットなしで、いきなり双方の戦闘パケットが流れてくる
        process_opcode(
            &enc,
            envelope(Pkt::WorldDeltaBatch, delta_damage_bytes(monster_uuid, other_uid, 9_999), other),
        )
        .unwrap();
        process_opcode(
            &enc,
            envelope(Pkt::WorldDeltaBatch, delta_damage_bytes(monster_uuid, my_uid, 100), mine),
        )
        .unwrap();

        let e = enc.lock().unwrap();
        assert_eq!(
            e.dmg_stats.total, 0,
            "身元不明 conn を通すと他キャラの 9999 が混入する（合計 10099 になる）"
        );
        assert_eq!(e.active_connection, None, "特定できていない");
        drop(e);

        // ゾーン移動で EnterScene が届けば特定され、以降は自分の分だけ集計される
        process_opcode(&enc, envelope(Pkt::WorldEnterScene, enter_scene_bytes(my_uid), mine))
            .unwrap();
        process_opcode(
            &enc,
            envelope(Pkt::WorldDeltaBatch, delta_damage_bytes(monster_uuid, my_uid, 100), mine),
        )
        .unwrap();
        process_opcode(
            &enc,
            envelope(Pkt::WorldDeltaBatch, delta_damage_bytes(monster_uuid, other_uid, 9_999), other),
        )
        .unwrap();

        let e = enc.lock().unwrap();
        assert_eq!(e.active_connection, Some(mine), "EnterScene で特定が完了する");
        assert_eq!(e.dmg_stats.total, 100, "特定後も他クライアント分は入らない");
        drop(e);

        selected_uid::set(None);
    }

    /// 自プレイヤー専用デルタ（LocalSceneDelta.uuid = 自キャラ）。
    fn local_delta_bytes(self_uid: i64) -> Vec<u8> {
        pb::LocalDeltaBatch {
            delta_info: Some(pb::LocalSceneDelta {
                uuid: player_uuid(self_uid),
                base_delta: Some(pb::SceneDelta {
                    uuid: player_uuid(self_uid),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        }
        .encode_to_vec()
    }

    /// EnterScene / WorldEnterSnapshot が一切来なくても、LocalDeltaBatch だけで
    /// 対象クライアントを特定できる（入場通知への依存を断つための主経路）。
    #[test]
    fn local_delta_alone_identifies_target_client() {
        let _guard = lock_selected_uid();
        let my_uid = 555_i64;
        let other_uid = 666_i64;
        let mine = conn(40041);
        let other = conn(40042);
        let monster_uuid = 11_i64 << 16 | 64;

        selected_uid::set(Some(my_uid));
        let enc = EncounterMutex::default();
        enc.lock().unwrap().local_player_uid = my_uid;

        // 他クライアントの自分専用デルタが先に届いても引きずられない
        process_opcode(&enc, envelope(Pkt::LocalDeltaBatch, local_delta_bytes(other_uid), other))
            .unwrap();
        assert_eq!(enc.lock().unwrap().active_connection, None);

        // 対象クライアントの自分専用デルタで特定完了（入場通知は一度も来ていない）
        process_opcode(&enc, envelope(Pkt::LocalDeltaBatch, local_delta_bytes(my_uid), mine))
            .unwrap();
        assert_eq!(enc.lock().unwrap().active_connection, Some(mine));

        process_opcode(
            &enc,
            envelope(Pkt::WorldDeltaBatch, delta_damage_bytes(monster_uuid, other_uid, 9_999), other),
        )
        .unwrap();
        process_opcode(
            &enc,
            envelope(Pkt::WorldDeltaBatch, delta_damage_bytes(monster_uuid, my_uid, 100), mine),
        )
        .unwrap();

        let e = enc.lock().unwrap();
        assert_eq!(e.dmg_stats.total, 100, "他クライアント分が混入していない");
        assert!(!e.entities.contains_key(&EntityKey::player(other_uid)));
        drop(e);

        selected_uid::set(None);
    }

    /// 表示層まで通した検証。エンジン内部状態だけでなく、UI が実際に描画する
    /// `compute::get_dps_players()` の行に他キャラが現れないことを確認する。
    #[test]
    fn display_rows_contain_only_selected_character() {
        let _guard = lock_selected_uid();
        let my_uid = 555_i64;
        let other_uid = 666_i64;
        let mine = conn(40051);
        let other = conn(40052);
        let monster_uuid = 12_i64 << 16 | 64;

        selected_uid::set(Some(my_uid));
        let enc = EncounterMutex::default();
        enc.lock().unwrap().local_player_uid = my_uid;

        // 両クライアントが稼働し、両方で戦闘が起きている
        process_opcode(&enc, envelope(Pkt::LocalDeltaBatch, local_delta_bytes(other_uid), other))
            .unwrap();
        process_opcode(&enc, envelope(Pkt::LocalDeltaBatch, local_delta_bytes(my_uid), mine))
            .unwrap();
        process_opcode(
            &enc,
            envelope(Pkt::WorldDeltaBatch, delta_damage_bytes(monster_uuid, other_uid, 9_999), other),
        )
        .unwrap();
        process_opcode(
            &enc,
            envelope(Pkt::WorldDeltaBatch, delta_damage_bytes(monster_uuid, my_uid, 100), mine),
        )
        .unwrap();

        let window = crate::compute::get_dps_players(&enc);
        let shown: Vec<i64> = window.player_rows.iter().map(|r| r.uid as i64).collect();
        assert_eq!(shown, vec![my_uid], "表示行は指定キャラのみ");
        assert_eq!(window.player_rows[0].total_value, 100.0);
        assert_eq!(window.local_player_uid as i64, my_uid);

        // 接続を特定できている＝UI は「特定中」表示にならない
        assert!(crate::compute::selected_conn_resolved(&enc));

        selected_uid::set(None);
    }

    /// 特定前は表示を空にし、UI 側が「特定中」を出せる状態であること。
    /// （空表示のまま「戦闘していない」と誤認させないための契約）
    #[test]
    fn display_is_empty_and_unresolved_before_client_is_identified() {
        let _guard = lock_selected_uid();
        let my_uid = 555_i64;
        let other_uid = 666_i64;
        let other = conn(40062);
        let monster_uuid = 13_i64 << 16 | 64;

        selected_uid::set(Some(my_uid));
        let enc = EncounterMutex::default();
        enc.lock().unwrap().local_player_uid = my_uid;

        process_opcode(
            &enc,
            envelope(Pkt::WorldDeltaBatch, delta_damage_bytes(monster_uuid, other_uid, 9_999), other),
        )
        .unwrap();

        let window = crate::compute::get_dps_players(&enc);
        assert!(window.player_rows.is_empty(), "他キャラの行が出てはいけない");
        assert!(
            !crate::compute::selected_conn_resolved(&enc),
            "未特定を UI へ伝えられる"
        );

        selected_uid::set(None);
    }

    /// 実機で判明したトポロジ: 1つのゲームクライアントが複数の TCP コネクションを張る
    /// （scene 用の補助接続。実測で同一 char_id が :10541 と :10408 の2本で学習された）。
    /// 判定を conn 単位にすると後着の接続が active を奪い、先の接続のダメージが落ちる。
    #[test]
    fn one_client_with_multiple_connections_keeps_all_its_damage() {
        let my_uid = 3_485_705_i64;
        let primary = conn(50177); // 実機の主接続に相当
        let auxiliary = conn(58387); // 同一キャラの補助接続
        let monster_uuid = 14_i64 << 16 | 64;

        // ── UID 指定あり
        {
            let _guard = lock_selected_uid();
            selected_uid::set(Some(my_uid));
            let enc = EncounterMutex::default();
            enc.lock().unwrap().local_player_uid = my_uid;

            for c in [primary, auxiliary] {
                process_opcode(&enc, envelope(Pkt::LocalDeltaBatch, local_delta_bytes(my_uid), c))
                    .unwrap();
            }
            // 両方の接続からダメージが届く
            for c in [primary, auxiliary] {
                process_opcode(
                    &enc,
                    envelope(Pkt::WorldDeltaBatch, delta_damage_bytes(monster_uuid, my_uid, 100), c),
                )
                .unwrap();
            }
            assert_eq!(
                enc.lock().unwrap().dmg_stats.total,
                200,
                "補助接続のダメージが落ちていない"
            );
            selected_uid::set(None);
        }

        // ── UID 未指定（自動検出）でも同じであること
        {
            let _guard = lock_selected_uid();
            selected_uid::set(None);
            let enc = EncounterMutex::default();

            for c in [primary, auxiliary] {
                process_opcode(&enc, envelope(Pkt::LocalDeltaBatch, local_delta_bytes(my_uid), c))
                    .unwrap();
            }
            for c in [primary, auxiliary] {
                process_opcode(
                    &enc,
                    envelope(Pkt::WorldDeltaBatch, delta_damage_bytes(monster_uuid, my_uid, 100), c),
                )
                .unwrap();
            }
            assert_eq!(
                enc.lock().unwrap().dmg_stats.total,
                200,
                "自動検出でも補助接続を落とさない"
            );
        }
    }

    /// 上と同じ多接続構成で、別キャラのクライアントが混ざっても弾けること。
    #[test]
    fn multi_connection_client_still_excludes_other_character() {
        let _guard = lock_selected_uid();
        let my_uid = 3_485_705_i64;
        let other_uid = 9_999_999_i64;
        let primary = conn(50177);
        let auxiliary = conn(58387);
        let other = conn(60001);
        let monster_uuid = 15_i64 << 16 | 64;

        selected_uid::set(Some(my_uid));
        let enc = EncounterMutex::default();
        enc.lock().unwrap().local_player_uid = my_uid;

        for (c, uid) in [(primary, my_uid), (auxiliary, my_uid), (other, other_uid)] {
            process_opcode(&enc, envelope(Pkt::LocalDeltaBatch, local_delta_bytes(uid), c)).unwrap();
        }
        for (c, uid) in [(primary, my_uid), (auxiliary, my_uid), (other, other_uid)] {
            process_opcode(
                &enc,
                envelope(Pkt::WorldDeltaBatch, delta_damage_bytes(monster_uuid, uid, 100), c),
            )
            .unwrap();
        }

        let e = enc.lock().unwrap();
        assert_eq!(e.dmg_stats.total, 200, "自分の2接続分のみ");
        assert!(!e.entities.contains_key(&EntityKey::player(other_uid)));
        drop(e);

        selected_uid::set(None);
    }

    /// 対照実験: UID 未指定なら従来どおり先着 conn に固定され、
    /// 後着クライアントのダメージは入らない（回帰防止）。
    #[test]
    fn two_clients_without_selected_uid_locks_onto_first_connection() {
        let _guard = lock_selected_uid();
        let first_uid = 777_i64;
        let second_uid = 888_i64;
        let first = conn(40021);
        let second = conn(40022);
        let monster_uuid = 8_i64 << 16 | 64;

        selected_uid::set(None);
        let enc = EncounterMutex::default();

        process_opcode(&enc, envelope(Pkt::WorldEnterScene, enter_scene_bytes(first_uid), first))
            .unwrap();
        process_opcode(&enc, envelope(Pkt::WorldEnterScene, enter_scene_bytes(second_uid), second))
            .unwrap();
        process_opcode(
            &enc,
            envelope(Pkt::WorldDeltaBatch, delta_damage_bytes(monster_uuid, second_uid, 9_999), second),
        )
        .unwrap();
        process_opcode(
            &enc,
            envelope(Pkt::WorldDeltaBatch, delta_damage_bytes(monster_uuid, first_uid, 100), first),
        )
        .unwrap();

        let e = enc.lock().unwrap();
        assert_eq!(e.active_connection, Some(first));
        assert_eq!(e.dmg_stats.total, 100);
    }

    // --- 総ダメージへの攻撃者/対象フィルタ（反撃する木人でアプリがゲーム内表示より
    // 高く出ていた不具合の修正）--------------------------------------------------

    fn monster_uuid_for(uid: i64) -> i64 {
        (uid << 16) | 64
    }

    /// top_summoner_id 未付与の召喚（Unknown 種別 attacker）を模した uuid。
    /// 64(Monster)/640(Player) いずれとも一致しない下位ビットにする（summon_spawn_delta の
    /// 実測由来の型コード 0x0100 に合わせ、実データに近づける）。
    fn unknown_summon_uuid_for(uid: i64) -> i64 {
        (uid << 16) | 0x0100
    }

    fn damage_delta(target_uuid: i64, attacker_uuid: i64, value: i64) -> pb::SceneDelta {
        pb::SceneDelta {
            uuid: target_uuid,
            skill_effects: Some(pb::SkillImpact {
                damages: vec![pb::DamageRecord {
                    value,
                    hp_lessen_value: value,
                    attacker_uuid,
                    owner_id: 1001,
                    ..Default::default()
                }],
            }),
            ..Default::default()
        }
    }

    fn heal_delta(target_uuid: i64, attacker_uuid: i64, value: i64) -> pb::SceneDelta {
        pb::SceneDelta {
            uuid: target_uuid,
            skill_effects: Some(pb::SkillImpact {
                damages: vec![pb::DamageRecord {
                    value,
                    hp_lessen_value: value,
                    r#type: pb::DmgKind::Heal as i32,
                    attacker_uuid,
                    owner_id: 1001,
                    ..Default::default()
                }],
            }),
            ..Default::default()
        }
    }

    /// fight_level_map_id は戦闘開始の瞬間に current_level_map_id を写す。戦闘中にシーンが
    /// 変わっても遡って書き換えず、clear_combat_stats を挟んだ次の戦闘で新しい値になる。
    #[test]
    fn fight_level_map_id_captured_at_fight_start_and_reset_by_clear_combat_stats() {
        let my_uid = 555_i64;
        let boss_uuid = monster_uuid_for(9001);

        let mut enc = Encounter::default();
        enc.set_local_player_uid(my_uid);
        enc.current_level_map_id = 6545;

        process_scene_delta(&mut enc, damage_delta(boss_uuid, player_uuid_for(my_uid), 500));
        assert_eq!(enc.fight_level_map_id, 6545, "戦闘開始の瞬間のシーンを記録する");

        // 戦闘中にシーンが切り替わっても、進行中の計測の起点は遡って書き換えない。
        enc.current_level_map_id = 9999;
        process_scene_delta(&mut enc, damage_delta(boss_uuid, player_uuid_for(my_uid), 500));
        assert_eq!(enc.fight_level_map_id, 6545, "戦闘中のシーン変化で書き換わってはいけない");

        enc.clear_combat_stats();
        assert_eq!(enc.fight_level_map_id, 0, "リセットで戻る");

        // 次の戦闘は、その時点の current_level_map_id を新たに記録する。
        process_scene_delta(&mut enc, damage_delta(boss_uuid, player_uuid_for(my_uid), 500));
        assert_eq!(enc.fight_level_map_id, 9999, "次の戦闘は新しいシーンを記録する");
    }

    /// fight_dungeon_difficulty は戦闘開始の瞬間に current の段階を写し、clear_combat_stats では
    /// fight だけ 0 になる（current は保持）。
    #[test]
    fn fight_dungeon_difficulty_captured_at_fight_start_and_reset_by_clear_combat_stats() {
        let my_uid = 555_i64;
        let boss_uuid = monster_uuid_for(9001);

        let mut enc = Encounter::default();
        enc.set_local_player_uid(my_uid);
        enc.current_level_map_id = 6545;
        enc.current_dungeon_difficulty = 3;

        process_scene_delta(&mut enc, damage_delta(boss_uuid, player_uuid_for(my_uid), 500));
        assert_eq!(enc.fight_dungeon_difficulty, 3, "戦闘開始の瞬間の段階を記録する");

        enc.current_dungeon_difficulty = 7;
        process_scene_delta(&mut enc, damage_delta(boss_uuid, player_uuid_for(my_uid), 500));
        assert_eq!(enc.fight_dungeon_difficulty, 3, "戦闘中の変化で書き換わってはいけない");

        enc.clear_combat_stats();
        assert_eq!(enc.fight_dungeon_difficulty, 0, "fight だけリセットする");
        assert_eq!(enc.current_dungeon_difficulty, 7, "current は保持する");
    }

    /// モンスター→プレイヤーのダメージ（反撃）は総ダメージに積まれず、被ダメ側にのみ残る。
    #[test]
    fn monster_damage_to_player_excluded_from_total_dmg_but_kept_in_taken() {
        let mut enc = Encounter::default();
        let player_uid = 1_i64;
        let monster_uid = 2_i64;

        process_scene_delta(
            &mut enc,
            damage_delta(player_uuid_for(player_uid), monster_uuid_for(monster_uid), 9_999),
        );

        assert_eq!(enc.dmg_stats.total, 0, "モンスターの与ダメが総ダメージに混入している");
        assert_eq!(
            enc.dmg_taken_stats.total, 9_999,
            "被ダメ側は従来どおりモンスターの与ダメを計上する"
        );
    }

    /// モンスター→モンスターのダメージ（召喚モンスター同士の小競り合い等）は行にも総ダメージにも
    /// 現れない。旧 target 基準（target==Player を除外）は target=Monster のとき常に
    /// damage_hits_player=false で total に混入していた（ヘッダ「総ダメージ」＞各行の合計、という
    /// 不整合の温床。compute.rs の Player フィルタで行には出ないため気付きにくい）。
    #[test]
    fn monster_damage_to_monster_excluded_from_total_dmg() {
        let mut enc = Encounter::default();
        let attacker_monster_uid = 16_i64;
        let target_monster_uid = 17_i64;

        process_scene_delta(
            &mut enc,
            damage_delta(
                monster_uuid_for(target_monster_uid),
                monster_uuid_for(attacker_monster_uid),
                777,
            ),
        );

        assert_eq!(enc.dmg_stats.total, 0, "モンスター同士のダメージが総ダメージに混入している");
        assert_eq!(enc.dmg_taken_stats.total, 0, "対象がモンスターなので被ダメにも積まれない");
    }

    /// top_summoner_id が付かない召喚（attacker_entity_type == Unknown）のダメージは、
    /// 対象がモンスターである限り自分の火力として総ダメージに残る（最重要の回帰防止）。
    #[test]
    fn unattributed_summon_damage_to_monster_counts_toward_total_dmg() {
        let mut enc = Encounter::default();
        let summon_uid = 3_i64;
        let monster_uid = 4_i64;

        process_scene_delta(
            &mut enc,
            damage_delta(monster_uuid_for(monster_uid), unknown_summon_uuid_for(summon_uid), 500),
        );

        assert_eq!(enc.dmg_stats.total, 500, "帰属漏れの召喚ダメージが総ダメージから落ちている");
        assert_eq!(enc.dmg_taken_stats.total, 0, "対象がモンスターなので被ダメには積まれない");
    }

    /// プレイヤー→モンスターのダメージは従来どおり総ダメージに入る（回帰防止）。
    #[test]
    fn player_damage_to_monster_counts_toward_total_dmg_as_before() {
        let mut enc = Encounter::default();
        let player_uid = 5_i64;
        let monster_uid = 6_i64;

        process_scene_delta(
            &mut enc,
            damage_delta(monster_uuid_for(monster_uid), player_uuid_for(player_uid), 1_234),
        );

        assert_eq!(enc.dmg_stats.total, 1_234);
        assert_eq!(enc.dmg_taken_stats.total, 0);
    }

    /// プレイヤー→プレイヤーの非Healダメージ（誤射等）は行(attacker_entity.dmg_stats)と
    /// 総ダメージの両方に同じ述語(from_monster)で入る。旧 target 基準では target=Player の
    /// ため total からは除外される一方、attacker 側の行集計は無条件加算だったため、
    /// 分子(行)＞分母(total) となりシェア%が100%を超え得た（結果モーダルの行バーが
    /// 幅からはみ出る不具合の原因）。
    #[test]
    fn player_to_player_damage_counts_toward_total_dmg_matching_row() {
        let mut enc = Encounter::default();
        let attacker_uid = 18_i64;
        let target_uid = 19_i64;

        process_scene_delta(
            &mut enc,
            damage_delta(player_uuid_for(target_uid), player_uuid_for(attacker_uid), 321),
        );

        assert_eq!(
            enc.entities[&EntityKey::player(attacker_uid)].dmg_stats.total, 321,
            "attacker 行には従来どおり積まれるはず"
        );
        assert_eq!(
            enc.dmg_stats.total, 321,
            "行(321)と総ダメージが食い違うとシェア%が100%を超えうる"
        );
    }

    /// プレイヤーと同じエンティティ番号の召喚体が先に現れても、プレイヤー行が一覧から消えず、
    /// 行の合計とシェアの分母（`encounter.dmg_stats`）が一致する（最重要の回帰防止）。
    ///
    /// UUID の上位ビットは種別ごとに独立した連番で、プレイヤー・モンスター・召喚体の間で
    /// 同じ値が使い回される。旧実装は `uuid >> 16` を `entities` のキーにしていたため、
    /// 番号が衝突すると非プレイヤーが先にその Entity を作って種別を Unknown に固定し、
    /// プレイヤーの与ダメだけが `entity_type != Player` で行から落ちた。分母側は uuid から
    /// 都度導出した種別で判定していたので総ダメージには残り、症状は
    /// 「ダンジョンで PT メンバーが1人足りず、シェア合計が 100% 未満」として現れた。
    #[test]
    fn player_row_survives_entity_number_collision_with_summon() {
        let mut enc = Encounter::default();
        let shared_number = 2573_i64; // プレイヤーと召喚体が同じ上位ビットを持つ状況
        let monster_uid = 900_i64;

        // 先に召喚体（Unknown 種別）が同じ番号で登場する。
        process_scene_delta(
            &mut enc,
            damage_delta(
                monster_uuid_for(monster_uid),
                unknown_summon_uuid_for(shared_number),
                300,
            ),
        );
        // その後、同じ番号のプレイヤーが殴る。
        process_scene_delta(
            &mut enc,
            damage_delta(monster_uuid_for(monster_uid), player_uuid_for(shared_number), 700),
        );

        assert_eq!(
            enc.entities[&EntityKey::player(shared_number)].dmg_stats.total,
            700,
            "プレイヤーの与ダメが同番号の召喚体の Entity に吸われている"
        );
        assert_eq!(enc.dmg_stats.total, 1_000, "総ダメージには召喚体の分も入る");

        let mutex = EncounterMutex::new(enc);
        let window = crate::compute::get_dps_players(&mutex);
        let row = window
            .player_rows
            .iter()
            .find(|r| r.uid as i64 == shared_number)
            .expect("同番号の召喚体に乗っ取られてプレイヤー行が消えている");
        assert_eq!(row.total_value, 700.0);
    }

    /// 被ダメの「誰にやられたか」内訳も、同じ番号のモンスターと召喚体を1つにまとめない。
    /// キーを `uuid >> 16` に潰すと両者が同じ行へ合算され、内訳が読めなくなる。
    #[test]
    fn dmg_taken_breakdown_separates_same_number_attackers() {
        let mut enc = Encounter::default();
        let shared_number = 77_i64;
        let victim_uid = 5001_i64;

        process_scene_delta(
            &mut enc,
            damage_delta(
                player_uuid_for(victim_uid),
                monster_uuid_for(shared_number),
                100,
            ),
        );
        process_scene_delta(
            &mut enc,
            damage_delta(
                player_uuid_for(victim_uid),
                unknown_summon_uuid_for(shared_number),
                40,
            ),
        );

        let victim = &enc.entities[&EntityKey::player(victim_uid)];
        assert_eq!(victim.dmg_taken_stats.total, 140);
        assert_eq!(
            victim.attacker_uid_to_dmg_taken_stats
                [&EntityKey::from_uuid(monster_uuid_for(shared_number))]
                .total,
            100,
            "同番号の召喚体の分が混ざっている"
        );
        assert_eq!(
            victim.attacker_uid_to_dmg_taken_stats
                [&EntityKey::from_uuid(unknown_summon_uuid_for(shared_number))]
                .total,
            40,
            "同番号のモンスターの分が混ざっている"
        );
    }

    /// 被ダメの攻撃元一覧が返す行 id を、UI と同じ経路（f64 → 整数文字列 → i64）で
    /// `get_dmg_taken_skills` へ渡し直すと、その攻撃元の技別内訳が引ける。
    /// 行 id は UID ではなく UUID なので、`uuid >> 16` で潰していた頃より桁が大きい。
    /// f64 の可逆域（2^53）に収まっていることと、同番号の攻撃元が別行のまま辿れることを見る。
    #[test]
    fn dmg_taken_attacker_row_id_round_trips_into_skill_breakdown() {
        let mut enc = Encounter::default();
        let shared_number = 77_i64;
        let victim_uid = 5001_i64;

        process_scene_delta(
            &mut enc,
            damage_delta(
                player_uuid_for(victim_uid),
                monster_uuid_for(shared_number),
                100,
            ),
        );
        process_scene_delta(
            &mut enc,
            damage_delta(
                player_uuid_for(victim_uid),
                unknown_summon_uuid_for(shared_number),
                40,
            ),
        );

        let mutex = EncounterMutex::new(enc);
        let attackers = crate::compute::get_dmg_taken_attackers(&mutex, victim_uid)
            .expect("攻撃元一覧が引けない");
        assert_eq!(
            attackers.skill_rows.len(),
            2,
            "同番号のモンスターと召喚体が1行に潰れている"
        );

        for row in &attackers.skill_rows {
            let round_tripped: i64 = format!("{}", row.uid as i64)
                .parse()
                .expect("行 id が整数文字列として往復できない");
            assert_eq!(round_tripped as f64, row.uid, "f64 の可逆域を超えている");

            let skills = crate::compute::get_dmg_taken_skills(&mutex, victim_uid, round_tripped)
                .expect("技別内訳が引けない");
            let total: f64 = skills.skill_rows.iter().map(|s| s.total_value).sum();
            assert_eq!(
                total, row.total_value,
                "攻撃元行の合計と技別内訳の合計が食い違う（行 id の往復が壊れている）"
            );
        }
    }

    /// モンスターの自己回復/味方回復は heal_stats から除外される。dmg と同じ述語
    /// （attacker が Monster かどうか）を共有する。
    /// 注意: モンスター発の回復レコードが実機で実在するかは未確認（検証用の合成データ）。
    #[test]
    fn monster_self_heal_excluded_from_total_heal() {
        let mut enc = Encounter::default();
        let monster_uid = 7_i64;

        process_scene_delta(
            &mut enc,
            heal_delta(monster_uuid_for(monster_uid), monster_uuid_for(monster_uid), 8_000),
        );

        assert_eq!(enc.heal_stats.total, 0, "モンスターの自己回復が総回復に混入している");
    }

    /// プレイヤーの回復は従来どおり heal_stats に入る（回帰防止）。
    #[test]
    fn player_heal_counts_toward_total_heal_as_before() {
        let mut enc = Encounter::default();
        let healer_uid = 8_i64;
        let target_uid = 9_i64;

        process_scene_delta(
            &mut enc,
            heal_delta(player_uuid_for(target_uid), player_uuid_for(healer_uid), 300),
        );

        assert_eq!(enc.heal_stats.total, 300);
    }

    /// モンスター entity に ATTR_ID(=monster_id) 属性を反映させる合成 SceneDelta。
    /// is_boss 判定（MONSTER_NAMES_BOSS 収録かどうか）はこの属性由来なので、
    /// dmg_stats_boss_only 集計のテストで使う。
    fn monster_id_attr_delta(monster_uid: i64, monster_id: u32) -> pb::SceneDelta {
        let monster_uuid = monster_uuid_for(monster_uid);
        pb::SceneDelta {
            uuid: monster_uuid,
            attrs: Some(pb::EntityAttrs {
                uuid: monster_uuid,
                attrs: vec![pb::RawAttr {
                    id: attr_type::ATTR_ID,
                    raw_data: enc_varint(monster_id as u64),
                }],
            }),
            buff_list: None,
            skill_effects: None,
        }
    }

    // ─── 初撃対象ロック（MeasureScope::first_target_only） ────────────────────────

    fn armed_with(scope: MeasureScope) -> Encounter {
        Encounter {
            measure_mode: MeasureMode::Active3Min {
                armed_at_ms: 1_000,
                duration_ms: 180_000,
                scope,
            },
            ..Default::default()
        }
    }

    /// 自分が最初に殴った対象へロックが張られ、以後は別の対象への与ダメージが集計から落ちる。
    /// 落ちるのは総計・攻撃者側の合計・スキル別・有効DPSの分母のすべてで、同じ条件が効く。
    #[test]
    fn first_target_lock_admits_only_the_first_target_i_attacked() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let me = 555_i64;
        let first = monster_uuid_for(9001);
        let second = monster_uuid_for(9002);

        let mut enc = armed_with(MeasureScope { first_target_only: true, self_only: false });
        enc.set_local_player_uid(me);

        process_scene_delta(&mut enc, damage_delta(first, player_uuid_for(me), 100));
        process_scene_delta(&mut enc, damage_delta(second, player_uuid_for(me), 900));

        assert_eq!(enc.locked_target, Some(EntityKey::from_uuid(first)));
        assert_eq!(enc.dmg_stats.total, 100, "ロック対象外への与ダメージが総計へ入っている");

        let mine = enc.entities.get(&EntityKey::player(me)).expect("自分のEntity");
        assert_eq!(mine.dmg_stats.total, 100, "攻撃者側の合計にロック対象外が入っている");
        assert_eq!(
            mine.skill_uid_to_dps_stats.values().map(|s| s.total).sum::<i64>(),
            100,
            "スキル別内訳だけロック対象外が残っている（合計と食い違う）"
        );
    }

    /// ロックは自分の与ダメージでしか張らない。AOI には他プレイヤーが別の敵を殴ったデルタが
    /// 常時流れており、全体基準にすると無関係な敵を掴んで自分の火力が丸ごと落ちる。
    #[test]
    fn first_target_lock_is_not_established_by_another_players_damage() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let me = 555_i64;
        let other = 666_i64;
        let theirs = monster_uuid_for(9001);
        let mine_target = monster_uuid_for(9002);

        let mut enc = armed_with(MeasureScope { first_target_only: true, self_only: false });
        enc.set_local_player_uid(me);

        process_scene_delta(&mut enc, damage_delta(theirs, player_uuid_for(other), 700));
        process_scene_delta(&mut enc, damage_delta(mine_target, player_uuid_for(me), 100));

        assert_eq!(
            enc.locked_target,
            Some(EntityKey::from_uuid(mine_target)),
            "他プレイヤーの与ダメージでロックが確定している"
        );
        assert_eq!(enc.dmg_stats.total, 100, "ロック確定前の他人の与ダメージが残っている");
    }

    /// ロック対象が倒れても解除しない（計測終了まで固定）。DamageRecord.is_dead は実測で
    /// 「対象が死亡状態」を意味し同じ対象へ繰り返し立つため、乗り換えの判定には使えない。
    #[test]
    fn first_target_lock_survives_the_targets_death() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let me = 555_i64;
        let first = monster_uuid_for(9001);
        let second = monster_uuid_for(9002);

        let mut enc = armed_with(MeasureScope { first_target_only: true, self_only: false });
        enc.set_local_player_uid(me);

        process_scene_delta(&mut enc, damage_delta(first, player_uuid_for(me), 100));
        let mut killing_blow = damage_delta(first, player_uuid_for(me), 50);
        killing_blow.skill_effects.as_mut().unwrap().damages[0].is_dead = true;
        process_scene_delta(&mut enc, killing_blow);
        process_scene_delta(&mut enc, damage_delta(second, player_uuid_for(me), 900));

        assert_eq!(enc.locked_target, Some(EntityKey::from_uuid(first)));
        assert_eq!(enc.dmg_stats.total, 150, "撃破後に次の敵へ乗り換えている");
    }

    /// 設定 off（既定）では全対象が集計される。ロックも張らない。
    #[test]
    fn first_target_lock_off_keeps_every_target() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let me = 555_i64;

        let mut enc = armed_with(MeasureScope::default());
        enc.set_local_player_uid(me);

        process_scene_delta(&mut enc, damage_delta(monster_uuid_for(9001), player_uuid_for(me), 100));
        process_scene_delta(&mut enc, damage_delta(monster_uuid_for(9002), player_uuid_for(me), 900));

        assert_eq!(enc.locked_target, None);
        assert_eq!(enc.dmg_stats.total, 1_000);
    }

    /// 自分が受けたダメージでも計測窓は開く。被ダメージを測るために計測ボタンを押す使い方が
    /// あり、攻撃者側だけを見ると一度も攻撃しないプレイヤーの計測が永久に始まらないうえ、
    /// 待っているあいだの被弾も丸ごと落ちる。
    #[test]
    fn pending_3min_is_armed_by_damage_taken_by_me() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let my_uid = 555_i64;

        let mut enc = Encounter::default();
        enc.set_local_player_uid(my_uid);
        enc.measure_mode = MeasureMode::Pending3Min { duration_ms: 180_000, scope: MeasureScope::default() };

        // モンスター → 自分。攻撃者は自分ではないが、自分の戦闘である。
        process_scene_delta(
            &mut enc,
            damage_delta(player_uuid_for(my_uid), monster_uuid_for(9001), 400),
        );

        assert!(
            matches!(enc.measure_mode, MeasureMode::Active3Min { .. }),
            "自分の被弾で計測が始まらない（被ダメ計測ができない）"
        );
        assert_eq!(enc.dmg_taken_stats.total, 400, "待機解除と同じデルタの被ダメが落ちている");
    }

    /// 他プレイヤーどうしの戦闘では窓は開かない（実測で問題になったのはこの経路）。
    #[test]
    fn pending_3min_is_not_armed_by_combat_between_others() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let my_uid = 555_i64;

        let mut enc = Encounter::default();
        enc.set_local_player_uid(my_uid);
        enc.measure_mode = MeasureMode::Pending3Min { duration_ms: 180_000, scope: MeasureScope::default() };

        // 他プレイヤー → モンスター、モンスター → 他プレイヤー のどちらでも開かない。
        process_scene_delta(&mut enc, damage_delta(monster_uuid_for(9001), player_uuid_for(666), 700));
        process_scene_delta(&mut enc, damage_delta(player_uuid_for(666), monster_uuid_for(9001), 300));

        assert!(matches!(enc.measure_mode, MeasureMode::Pending3Min { .. }));
        assert_eq!(enc.dmg_stats.total, 0);
    }

    /// ロック中は、対象がプレイヤーの非Healレコード（反射・PvP等）も総計へ通さない。
    /// 素通しにするとロック対象と別の相手へのダメージが残る。
    #[test]
    fn first_target_lock_also_excludes_non_heal_damage_aimed_at_players() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let me = 555_i64;
        let victim = 777_i64;

        let mut enc = armed_with(MeasureScope { first_target_only: true, self_only: false });
        enc.set_local_player_uid(me);

        process_scene_delta(&mut enc, damage_delta(monster_uuid_for(9001), player_uuid_for(me), 100));
        process_scene_delta(&mut enc, damage_delta(player_uuid_for(victim), player_uuid_for(me), 900));

        assert_eq!(
            enc.dmg_stats.total, 100,
            "ロック対象以外（プレイヤー対象）の非Healダメージが総計へ入っている"
        );
        assert_eq!(
            enc.dmg_taken_stats.total, 900,
            "被ダメ集計までロックで落ちている（admit_target は被ダメを通らないはず）"
        );
    }

    /// 自キャラが未確定のあいだは絞らない。ロックを張れないまま「ロック対象と一致するか」で
    /// 判定すると全ダメージが落ち、計測が丸ごと0になる（待機解除・表示側の射影も同じ場面で
    /// 絞らない側へ倒しており、3箇所で方針が揃っていること自体をここで固定する）。
    #[test]
    fn first_target_lock_does_not_drop_everything_while_self_uid_is_unknown() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);

        let mut enc = armed_with(MeasureScope { first_target_only: true, self_only: false });
        assert_eq!(enc.local_player_uid, 0, "テスト前提: 自キャラ未確定");

        process_scene_delta(&mut enc, damage_delta(monster_uuid_for(9001), player_uuid_for(666), 100));
        process_scene_delta(&mut enc, damage_delta(monster_uuid_for(9002), player_uuid_for(666), 900));

        assert_eq!(enc.locked_target, None, "自キャラ未確定でロックが張られている");
        assert_eq!(
            enc.dmg_stats.total, 1_000,
            "自キャラ未確定のあいだに全ダメージが落ちている（計測が丸ごと0になる）"
        );
    }

    /// 回復はロックの対象外（対象がプレイヤーなので構造上排他）。
    /// 自分のみ計測と併用しても、ヒール出力が消えることはない。
    #[test]
    fn first_target_lock_does_not_touch_heals() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let me = 555_i64;
        let ally = 777_i64;

        let mut enc = armed_with(MeasureScope { first_target_only: true, self_only: false });
        enc.set_local_player_uid(me);

        process_scene_delta(&mut enc, damage_delta(monster_uuid_for(9001), player_uuid_for(me), 100));
        process_scene_delta(&mut enc, heal_delta(player_uuid_for(ally), player_uuid_for(me), 300));

        assert_eq!(enc.heal_stats.total, 300, "ロックが回復を落としている");
    }

    /// 通常モード（計測ボタンを押していない）ではロックは効かない。
    #[test]
    fn first_target_lock_is_inactive_outside_a_measurement() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let me = 555_i64;

        let mut enc = Encounter::default();
        enc.set_local_player_uid(me);

        process_scene_delta(&mut enc, damage_delta(monster_uuid_for(9001), player_uuid_for(me), 100));
        process_scene_delta(&mut enc, damage_delta(monster_uuid_for(9002), player_uuid_for(me), 900));

        assert_eq!(enc.locked_target, None);
        assert_eq!(enc.dmg_stats.total, 1_000);
    }

    // ─── 3分計測の起点（Pending3Min → Active3Min） ───────────────────────────────
    //
    // 計測窓の分母は armed_at_ms から回る（compute::combat_elapsed_ms）。以前は
    // skill_effects を持つ任意のデルタで遷移していたため、押した瞬間に周囲の誰かが殴っていれば
    // その時刻が起点になった（2026-08-26 の実測では計測ボタンを押した4回すべてが該当）。

    /// 待機中は他プレイヤーの与ダメージで計測窓が開かない。集計も戦闘時計も動かさない。
    #[test]
    fn pending_3min_is_not_armed_by_another_players_damage() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let my_uid = 555_i64;
        let other_uid = 666_i64;

        let mut enc = Encounter::default();
        enc.set_local_player_uid(my_uid);
        enc.measure_mode = MeasureMode::Pending3Min { duration_ms: 180_000, scope: MeasureScope::default() };

        process_scene_delta(
            &mut enc,
            damage_delta(monster_uuid_for(9001), player_uuid_for(other_uid), 1_000),
        );

        assert!(
            matches!(enc.measure_mode, MeasureMode::Pending3Min { .. }),
            "他プレイヤーの与ダメージで計測が始まってはいけない"
        );
        assert_eq!(enc.time_fight_start_ms, 0, "待機中は戦闘時計も動かさない");
        assert_eq!(enc.dmg_stats.total, 0, "待機中は集計もしない");
    }

    /// 自分の与ダメージで計測窓が開き、分母の起点(armed_at_ms)が戦闘時計と一致する。
    /// 起点より前に届いた他プレイヤーのダメージは集計に入らない。
    #[test]
    fn pending_3min_is_armed_by_own_damage_and_starts_the_denominator_there() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let my_uid = 555_i64;
        let other_uid = 666_i64;
        let boss_uuid = monster_uuid_for(9001);

        let mut enc = Encounter::default();
        enc.set_local_player_uid(my_uid);
        enc.measure_mode = MeasureMode::Pending3Min { duration_ms: 180_000, scope: MeasureScope::default() };

        process_scene_delta(&mut enc, damage_delta(boss_uuid, player_uuid_for(other_uid), 1_000));
        process_scene_delta(&mut enc, damage_delta(boss_uuid, player_uuid_for(my_uid), 500));

        match enc.measure_mode {
            MeasureMode::Active3Min { armed_at_ms, duration_ms, .. } => {
                assert_eq!(duration_ms, 180_000, "設定した計測長が失われている");
                assert_ne!(armed_at_ms, 0);
                assert_eq!(
                    armed_at_ms, enc.time_fight_start_ms,
                    "分母の起点は自分の初撃へ揃える"
                );
            }
            other => panic!("自分の与ダメージで Active3Min へ遷移するはず: {other:?}"),
        }
        assert_eq!(
            enc.dmg_stats.total, 500,
            "起点より前に届いた他プレイヤーのダメージが集計へ混ざっている"
        );
    }

    /// 自分の回復でも計測窓は開く（ヒーラーが計測ボタンを押しても始まらない、を防ぐ）。
    #[test]
    fn pending_3min_is_armed_by_own_heal() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);
        let my_uid = 555_i64;
        let ally_uid = 777_i64;

        let mut enc = Encounter::default();
        enc.set_local_player_uid(my_uid);
        enc.measure_mode = MeasureMode::Pending3Min { duration_ms: 180_000, scope: MeasureScope::default() };

        process_scene_delta(
            &mut enc,
            heal_delta(player_uuid_for(ally_uid), player_uuid_for(my_uid), 300),
        );

        assert!(
            matches!(enc.measure_mode, MeasureMode::Active3Min { .. }),
            "自分の回復でも計測は始まるべき"
        );
    }

    /// 自キャラが未確定(local_player_uid==0 かつ selected_uid なし)なら、従来どおり最初の
    /// ダメージで開始する。ここで待ち続けると計測ボタンが無反応のままになるため。
    #[test]
    fn pending_3min_falls_back_to_any_damage_when_self_uid_is_unknown() {
        let _guard = lock_selected_uid();
        selected_uid::set(None);

        let mut enc = Encounter::default();
        assert_eq!(enc.local_player_uid, 0, "テスト前提: 自キャラ未確定");
        enc.measure_mode = MeasureMode::Pending3Min { duration_ms: 180_000, scope: MeasureScope::default() };

        process_scene_delta(
            &mut enc,
            damage_delta(monster_uuid_for(9001), player_uuid_for(666), 1_000),
        );

        assert!(
            matches!(enc.measure_mode, MeasureMode::Active3Min { .. }),
            "自キャラ未確定では待たずに開始する"
        );
    }

    /// player→ボス(MONSTER_NAMES_BOSS 収録の monster_id)のダメージは dmg_stats_boss_only にも
    /// 積まれる。旧 `assert_eq!(enc.dmg_stats_boss_only.total, 0)`（target=Player固定のテスト）は
    /// is_boss が常に false になる無効アサーションだったため、実際に boss 判定が成立する
    /// ケースをここで検証する。
    #[test]
    fn player_damage_to_boss_monster_counts_toward_boss_only_total() {
        let mut enc = Encounter::default();
        let player_uid = 20_i64;
        let boss_uid = 21_i64;
        let boss_monster_id: u32 = 103; // MonsterNameBoss.json 収録（イグニソル）
        assert!(
            MONSTER_NAMES_BOSS.contains_key(&boss_monster_id),
            "テスト前提のboss idがMonsterNameBoss.jsonから外れている"
        );

        process_scene_delta(&mut enc, monster_id_attr_delta(boss_uid, boss_monster_id));
        process_scene_delta(
            &mut enc,
            damage_delta(monster_uuid_for(boss_uid), player_uuid_for(player_uid), 777),
        );

        assert_eq!(enc.dmg_stats.total, 777);
        assert_eq!(
            enc.dmg_stats_boss_only.total, 777,
            "boss収録のmonster_idなのにboss_only集計に積まれていない"
        );
    }

    /// 対照実験: MONSTER_NAMES_BOSS 未収録の monster_id では boss_only 集計に積まれない
    /// （通常の dmg_stats のみ）。
    #[test]
    fn player_damage_to_non_boss_monster_excluded_from_boss_only_total() {
        let mut enc = Encounter::default();
        let player_uid = 22_i64;
        let monster_uid = 23_i64;
        let non_boss_monster_id: u32 = 999_999;
        assert!(
            !MONSTER_NAMES_BOSS.contains_key(&non_boss_monster_id),
            "テスト前提の非boss idがMonsterNameBoss.jsonに含まれてしまっている"
        );

        process_scene_delta(&mut enc, monster_id_attr_delta(monster_uid, non_boss_monster_id));
        process_scene_delta(
            &mut enc,
            damage_delta(monster_uuid_for(monster_uid), player_uuid_for(player_uid), 555),
        );

        assert_eq!(enc.dmg_stats.total, 555);
        assert_eq!(enc.dmg_stats_boss_only.total, 0);
    }
}
